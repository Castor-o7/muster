//! Links between daemons (MUS-66). See `docs/daemon-links-design.md`.
//!
//! One daemon (the dialer) opens a WebSocket to the other's gate at `/link`,
//! over TLS with that gate's certificate pinned, presenting a link token. Both
//! directions ride that connection. The two daemons speak the small protocol in
//! [`Frame`], never myko's frames, so a linked daemon can do only what those
//! frames allow.
//!
//! - Each side sends `hello`, then a `roster` of its own local sessions, again
//!   whenever it changes. The other side keeps them as `RemoteSession` rows.
//! - Mail for a remote session is stored locally with an outbound
//!   `MessageLink`, the outbox. The link task sends it as `deliver` and drops
//!   the outbox entry on `stored`; `deferred` holds it for `retry_after`;
//!   `refused` drops it and tells the sender.
//! - A `deliver` is accepted only from a session in that link's own last
//!   roster, for a local session, and is stored under a local id derived from
//!   the link and the sender's id, which also deduplicates a resend.

use std::collections::HashMap;
use std::io;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_util::{SinkExt as _, StreamExt as _};
use myko::prelude::EventPublishing as _;
use myko::{command::CommandContext, request::RequestContext, server::MykoServerContext};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;

use muster_entities::{
    GetAllMessageLinks, GetAllRemoteSessions, GetAllSessionNicknames, GetAllSessions,
    GetAllWakeWatchers, GetMessagesByIds, Message, MessageId, MessageLink, MessageLinkId,
    RemoteSession, RemoteSessionId, SessionId,
};

/// The protocol version in `hello`. A mismatch closes the link.
pub const PROTOCOL_VERSION: u32 = 1;
/// How long a link may be down before its sessions are dropped and the mail
/// queued for it is refused back to its senders.
pub const LINK_GIVE_UP: Duration = Duration::from_secs(24 * 60 * 60);
/// The gate route a dialer opens.
pub const LINK_PATH: &str = "/link";
/// Largest frame either side accepts. A bigger one closes the link.
pub const MAX_FRAME: usize = 128 * 1024;
/// Most sessions one link may advertise.
pub const MAX_ROSTER: usize = 512;
/// The dialer's stored links, one per line, owner-only.
const LINKS_FILE: &str = "links";

const HELLO_TIMEOUT: Duration = Duration::from_secs(10);
/// After a link comes up, mail for a session this daemon doesn't know yet is
/// deferred rather than refused for this long: after a restart, sessions take
/// a moment to register again.
const RECIPIENT_GRACE: Duration = Duration::from_secs(60);
const TICK: Duration = Duration::from_millis(250);
const ROSTER_EVERY: Duration = Duration::from_secs(1);
/// Resend a `deliver` that got no answer in this long.
const UNANSWERED: Duration = Duration::from_secs(30);
const BACKOFF_MAX: Duration = Duration::from_secs(60);
/// Longest text field kept from a peer's roster.
const MAX_FIELD: usize = 256;

/// One frame of the link protocol.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Frame {
    Hello {
        name: String,
        version: u32,
    },
    Roster {
        sessions: Vec<RosterEntry>,
    },
    Deliver {
        id: String,
        from: String,
        to: String,
        body: String,
        #[serde(default)]
        response_required: bool,
        sent_at: i64,
    },
    Stored {
        id: String,
    },
    Deferred {
        id: String,
        retry_after: u64,
    },
    Refused {
        id: String,
        why: String,
    },
}

/// What a link shares about one session: enough to choose a recipient. No
/// cwd, pid or branch.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RosterEntry {
    pub session_id: String,
    pub nickname: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operator: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_task: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wake: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_activity_at: Option<i64>,
}

/// What both ends of a link need from the daemon.
pub struct Hub {
    pub ctx: Arc<MykoServerContext>,
    /// This daemon's name, sent in `hello`.
    pub name: String,
    pub state_dir: PathBuf,
}

// ── Stored links (the dialer's side) ─────────────────────────────────────

/// A link this daemon dials.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredLink {
    /// The far daemon's name: it labels that daemon's sessions here.
    pub name: String,
    pub address: String,
    pub fingerprint: String,
    pub token: String,
}

pub fn read_links(dir: &Path) -> io::Result<Vec<StoredLink>> {
    let text = match std::fs::read_to_string(dir.join(LINKS_FILE)) {
        Ok(t) => t,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    Ok(text
        .lines()
        .filter_map(|line| {
            let mut p = line.split_whitespace();
            Some(StoredLink {
                name: p.next()?.to_string(),
                address: p.next()?.to_string(),
                fingerprint: p.next()?.to_string(),
                token: p.next()?.to_string(),
            })
        })
        .collect())
}

fn write_links(dir: &Path, links: &[StoredLink]) -> io::Result<()> {
    let text: String = links
        .iter()
        .map(|l| format!("{} {} {} {}\n", l.name, l.address, l.fingerprint, l.token))
        .collect();
    crate::credentials::write_private_file(dir, LINKS_FILE, text.as_bytes())
}

/// Store `link`, replacing one of the same name.
pub fn save_link(dir: &Path, link: StoredLink) -> io::Result<()> {
    let mut links: Vec<_> = read_links(dir)?
        .into_iter()
        .filter(|l| l.name != link.name)
        .collect();
    links.push(link);
    write_links(dir, &links)
}

/// Forget the link `name`. Returns whether there was one.
pub fn remove_link(dir: &Path, name: &str) -> io::Result<bool> {
    let links = read_links(dir)?;
    let kept: Vec<_> = links.iter().filter(|l| l.name != name).cloned().collect();
    if kept.len() == links.len() {
        return Ok(false);
    }
    write_links(dir, &kept)?;
    Ok(true)
}

// ── One connection at a time per link ────────────────────────────────────

static GENERATIONS: Mutex<Option<HashMap<String, u64>>> = Mutex::new(None);

/// Claim `label` for a new connection; an older one for it stops at its next tick.
fn claim(label: &str) -> u64 {
    let mut g = GENERATIONS.lock().unwrap_or_else(|e| e.into_inner());
    let map = g.get_or_insert_with(HashMap::new);
    let next = map.get(label).copied().unwrap_or(0) + 1;
    map.insert(label.to_string(), next);
    next
}

fn is_current(label: &str, generation: u64) -> bool {
    let g = GENERATIONS.lock().unwrap_or_else(|e| e.into_inner());
    g.as_ref().and_then(|m| m.get(label)).copied() == Some(generation)
}

/// Whether a link named `label` has a connection open now.
static UP: Mutex<Option<HashMap<String, bool>>> = Mutex::new(None);

pub fn is_up(label: &str) -> bool {
    UP.lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .and_then(|m| m.get(label))
        .copied()
        .unwrap_or(false)
}

fn set_up(label: &str, up: bool) {
    UP.lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_or_insert_with(HashMap::new)
        .insert(label.to_string(), up);
}

// ── The protocol, shared by both ends ────────────────────────────────────

fn ws_config() -> WebSocketConfig {
    WebSocketConfig::default()
        .max_message_size(Some(MAX_FRAME))
        .max_frame_size(Some(MAX_FRAME))
}

fn cmd_ctx(ctx: &Arc<MykoServerContext>) -> CommandContext {
    let tx: Arc<str> = uuid::Uuid::new_v4().to_string().into();
    let req = RequestContext::internal(tx, ctx.host_id, "link");
    CommandContext::new(Arc::from("link"), Arc::new(req), ctx.clone())
}

fn encode(frame: &Frame) -> WsMessage {
    WsMessage::text(serde_json::to_string(frame).unwrap_or_default())
}

/// Run one connection of the link `label` until it closes, is replaced, or
/// (with `still_wanted`) the link is removed.
pub async fn run_session<S>(
    ws: WebSocketStream<S>,
    label: &str,
    hub: &Hub,
    still_wanted: impl Fn() -> bool,
) -> io::Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let generation = claim(label);
    let (mut tx, mut rx) = ws.split();
    let send = |frame: Frame| encode(&frame);
    tx.send(send(Frame::Hello {
        name: hub.name.clone(),
        version: PROTOCOL_VERSION,
    }))
    .await
    .map_err(io::Error::other)?;

    let first = tokio::time::timeout(HELLO_TIMEOUT, rx.next())
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "no hello"))?;
    match first.and_then(|m| m.ok()).and_then(parse) {
        Some(Frame::Hello { version, name }) if version == PROTOCOL_VERSION => {
            log::info!("[link] {label}: up (far side calls itself {name})");
        }
        Some(Frame::Hello { version, .. }) => {
            log::warn!(
                "[link] {label}: closing: it speaks link protocol {version}, this daemon {PROTOCOL_VERSION}"
            );
            let _ = tx.close().await;
            return Ok(());
        }
        _ => {
            log::warn!("[link] {label}: closing: the first frame wasn't hello");
            let _ = tx.close().await;
            return Ok(());
        }
    }
    set_up(label, true);
    mark_link(hub, label, true);
    let up_since = Instant::now();

    let mut in_flight: HashMap<String, Instant> = HashMap::new();
    let mut last_roster = String::new();
    let mut roster_at = Instant::now() - ROSTER_EVERY;
    let mut outbox_seen = u64::MAX;
    let mut outbox_at = Instant::now() - Duration::from_secs(5);
    let mut tick = tokio::time::interval(TICK);
    let result = loop {
        tokio::select! {
            incoming = rx.next() => {
                let Some(Ok(msg)) = incoming else { break Ok(()) };
                if msg.is_close() { break Ok(()) }
                if !msg.is_text() { continue }
                let Some(frame) = parse(msg) else {
                    log::warn!("[link] {label}: closing: a frame that isn't the link protocol");
                    break Ok(());
                };
                if let Some(answer) = handle_frame(hub, label, frame, &mut in_flight, up_since)
                    && tx.send(send(answer)).await.is_err()
                {
                    break Ok(());
                }
            }
            _ = tick.tick() => {
                if !is_current(label, generation) {
                    log::info!("[link] {label}: replaced by a newer connection");
                    break Ok(());
                }
                if !still_wanted() {
                    log::info!("[link] {label}: closing: no longer linked");
                    break Ok(());
                }
                if roster_at.elapsed() >= ROSTER_EVERY {
                    roster_at = Instant::now();
                    let snapshot = local_roster(hub);
                    let text = serde_json::to_string(&snapshot).unwrap_or_default();
                    if text != last_roster {
                        last_roster = text;
                        if tx.send(send(Frame::Roster { sessions: snapshot })).await.is_err() {
                            break Ok(());
                        }
                    }
                }
                let generation_now =
                    muster_entities::OUTBOX_GENERATION.load(std::sync::atomic::Ordering::SeqCst);
                if generation_now != outbox_seen || outbox_at.elapsed() >= Duration::from_secs(2) {
                    outbox_seen = generation_now;
                    outbox_at = Instant::now();
                    let mut failed = false;
                    for frame in due_outbox(hub, label, &mut in_flight) {
                        if tx.send(send(frame)).await.is_err() {
                            failed = true;
                            break;
                        }
                    }
                    if failed { break Ok(()) }
                }
            }
        }
    };
    // A newer connection for this link owns its state now.
    if is_current(label, generation) {
        set_up(label, false);
        mark_link(hub, label, false);
        log::info!("[link] {label}: down");
    }
    let _ = tx.close().await;
    result
}

fn parse(msg: WsMessage) -> Option<Frame> {
    let text = msg.into_text().ok()?;
    serde_json::from_str(text.as_str()).ok()
}

/// Act on one frame from the far side, returning the answer to send, if any.
fn handle_frame(
    hub: &Hub,
    label: &str,
    frame: Frame,
    in_flight: &mut HashMap<String, Instant>,
    up_since: Instant,
) -> Option<Frame> {
    let ctx = cmd_ctx(&hub.ctx);
    match frame {
        Frame::Hello { .. } => None,
        Frame::Roster { sessions } => {
            ingest_roster(&ctx, label, sessions);
            None
        }
        Frame::Deliver {
            id,
            from,
            to,
            body,
            response_required,
            ..
        } => Some(receive(
            &ctx,
            label,
            Delivery {
                id,
                from: &from,
                to: &to,
                body,
                response_required,
            },
            up_since.elapsed() < RECIPIENT_GRACE,
        )),
        Frame::Stored { id } => {
            in_flight.remove(&id);
            drop_outbox(&ctx, &id);
            None
        }
        Frame::Deferred { id, retry_after } => {
            in_flight.remove(&id);
            let retry_at =
                chrono::Utc::now().timestamp_millis() + (retry_after.clamp(1, 3600) * 1000) as i64;
            let _ = ctx.emit_set(&MessageLink {
                id: MessageLinkId(Arc::from(id.as_str())),
                link: label.to_string(),
                outbound: true,
                remote_id: None,
                retry_at: Some(retry_at),
                note: Some("the far daemon's rate limit".into()),
            });
            None
        }
        Frame::Refused { id, why } => {
            in_flight.remove(&id);
            refuse_back(&ctx, label, &id, &why);
            None
        }
    }
}

/// This daemon's own sessions, as a link shares them.
fn local_roster(hub: &Hub) -> Vec<RosterEntry> {
    let ctx = cmd_ctx(&hub.ctx);
    let sessions: Vec<Arc<muster_entities::Session>> =
        ctx.exec_query(GetAllSessions {}).unwrap_or_default();
    let nicknames: Vec<Arc<muster_entities::SessionNickname>> = ctx
        .exec_query(GetAllSessionNicknames {})
        .unwrap_or_default();
    let watchers = ctx.exec_query(GetAllWakeWatchers {}).unwrap_or_default();
    let now = chrono::Utc::now().timestamp_millis();
    let mut out: Vec<RosterEntry> = sessions
        .iter()
        .take(MAX_ROSTER)
        .map(|s| {
            let id = s.id.0.as_ref();
            let nickname = nicknames
                .iter()
                .find(|n| n.id.0.as_ref() == id)
                .map(|n| n.nickname.clone())
                .unwrap_or_else(|| muster_entities::nickname(id));
            RosterEntry {
                session_id: id.to_string(),
                nickname,
                host: s.host.as_ref().map(|h| h.name.clone()),
                operator: s.operator.clone(),
                project: s.project.clone(),
                current_task: s.current_task.clone(),
                activity: s.activity.clone(),
                wake: Some(
                    muster_entities::wake_state(s, &watchers, now)
                        .as_str()
                        .to_string(),
                ),
                last_activity_at: s.last_activity_at,
            }
        })
        .collect();
    out.sort_by(|a, b| a.session_id.cmp(&b.session_id));
    out
}

fn clip(s: Option<String>) -> Option<String> {
    s.map(|v| v.chars().take(MAX_FIELD).collect())
}

/// A session id a peer may use: short, printable, and with no `@`.
fn acceptable_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_graphic() && b != b'@' && b != b'/')
}

/// Replace `label`'s remote sessions with `sessions`, refusing any that would
/// collide with a local session or another link's.
fn ingest_roster(ctx: &CommandContext, label: &str, sessions: Vec<RosterEntry>) {
    if sessions.len() > MAX_ROSTER {
        log::warn!(
            "[link] {label}: refused a roster of {} sessions (at most {MAX_ROSTER})",
            sessions.len()
        );
        return;
    }
    let local: Vec<Arc<muster_entities::Session>> =
        ctx.exec_query(GetAllSessions {}).unwrap_or_default();
    let remotes: Vec<Arc<RemoteSession>> =
        ctx.exec_query(GetAllRemoteSessions {}).unwrap_or_default();
    let now = chrono::Utc::now().timestamp_millis();
    let mut keep = std::collections::HashSet::new();
    for entry in sessions {
        let id = entry.session_id.as_str();
        if !acceptable_id(id) {
            log::warn!("[link] {label}: refused a roster entry with an unusable id");
            continue;
        }
        if local.iter().any(|s| s.id.0.as_ref() == id) {
            log::warn!("[link] {label}: refused {id}: that's a local session's id");
            continue;
        }
        if remotes
            .iter()
            .any(|r| r.session_id == id && r.link != label)
        {
            log::warn!("[link] {label}: refused {id}: another link has that session");
            continue;
        }
        let nickname: String = entry.nickname.chars().take(64).collect();
        let row = RemoteSession {
            id: RemoteSessionId(Arc::from(RemoteSession::make_id(id, label).as_str())),
            link: label.to_string(),
            session_id: id.to_string(),
            nickname: if nickname.is_empty() {
                muster_entities::nickname(id)
            } else {
                nickname
            },
            host: clip(entry.host),
            operator: clip(entry.operator),
            project: clip(entry.project),
            current_task: clip(entry.current_task),
            activity: clip(entry.activity),
            wake: clip(entry.wake),
            last_activity_at: entry.last_activity_at,
            link_up: true,
            seen_at: now,
        };
        keep.insert(row.id.0.to_string());
        if let Err(e) = ctx.emit_set(&row) {
            log::warn!("[link] {label}: couldn't store a remote session: {e:?}");
        }
    }
    for gone in remotes
        .iter()
        .filter(|r| r.link == label && !keep.contains(r.id.0.as_ref()))
    {
        let _ = ctx.emit_del(&**gone);
    }
}

/// Mark every remote session of `label` up or down.
fn mark_link(hub: &Hub, label: &str, up: bool) {
    let ctx = cmd_ctx(&hub.ctx);
    let remotes: Vec<Arc<RemoteSession>> =
        ctx.exec_query(GetAllRemoteSessions {}).unwrap_or_default();
    for r in remotes
        .iter()
        .filter(|r| r.link == label && r.link_up != up)
    {
        let mut row = (**r).clone();
        row.link_up = up;
        let _ = ctx.emit_set(&row);
    }
}

/// One `deliver`'s contents.
struct Delivery<'a> {
    id: String,
    from: &'a str,
    to: &'a str,
    body: String,
    response_required: bool,
}

/// The answer to a `deliver` from `label`. In `grace`, an unknown recipient
/// is deferred, not refused (see [`RECIPIENT_GRACE`]).
fn receive(ctx: &CommandContext, label: &str, d: Delivery<'_>, grace: bool) -> Frame {
    let Delivery {
        id,
        from,
        to,
        body,
        response_required,
    } = d;
    let refuse = |why: &str| {
        log::warn!("[link] {label}: refused a message: {why}");
        Frame::Refused {
            id: id.clone(),
            why: why.to_string(),
        }
    };
    if id.is_empty() || id.len() > 128 {
        return refuse("an unusable message id");
    }
    let remotes: Vec<Arc<RemoteSession>> =
        ctx.exec_query(GetAllRemoteSessions {}).unwrap_or_default();
    if !remotes
        .iter()
        .any(|r| r.link == label && r.session_id == from)
    {
        return refuse("the sender isn't one of that daemon's sessions");
    }
    let local: Vec<Arc<muster_entities::Session>> =
        ctx.exec_query(GetAllSessions {}).unwrap_or_default();
    if !local.iter().any(|s| s.id.0.as_ref() == to) {
        if grace {
            return Frame::Deferred { id, retry_after: 5 };
        }
        return refuse("no such session here");
    }
    let body = muster_entities::pairing::redact(&body).into_owned();
    if let Some(why) = muster_entities::body_too_large(&body) {
        return refuse(&why);
    }
    let local_id = MessageId(Arc::from(
        muster_entities::inbound_message_id(label, &id).as_str(),
    ));
    let held: Vec<Arc<Message>> = ctx
        .exec_query(GetMessagesByIds {
            ids: vec![local_id.clone()],
        })
        .unwrap_or_default();
    if !held.is_empty() {
        return Frame::Stored { id };
    }
    // The operator's allowlist names a link by the link's name (MUS-10).
    if let Err(why) = muster_entities::controls::sender_allowed(Some(label)) {
        return refuse(&why);
    }
    let link_machine = crate::credentials::link_token_name(label);
    if let Err(why) = muster_entities::rate::take(
        &RemoteSession::make_id(from, label),
        Some(link_machine.as_str()),
    ) {
        return Frame::Deferred {
            id,
            retry_after: retry_after_secs(&why),
        };
    }
    let msg = Message {
        response_required,
        id: local_id.clone(),
        from_session_id: SessionId(Arc::from(from)),
        to_session_id: Some(SessionId(Arc::from(to))),
        to_room_id: None,
        to_operator: None,
        body,
        sent_at: chrono::Utc::now().timestamp_millis(),
    };
    if let Err(e) = ctx.emit_set(&msg) {
        log::warn!("[link] {label}: couldn't store a message: {e:?}");
        return Frame::Deferred { id, retry_after: 5 };
    }
    let _ = ctx.emit_set(&MessageLink {
        id: MessageLinkId(local_id.0.clone()),
        link: label.to_string(),
        outbound: false,
        remote_id: Some(id.clone()),
        retry_at: None,
        note: None,
    });
    Frame::Stored { id }
}

/// The wait a rate-limit refusal names ("try again in N s"), or 5 s.
fn retry_after_secs(why: &str) -> u64 {
    why.split("try again in ")
        .nth(1)
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|n| n.parse().ok())
        .unwrap_or(5)
}

/// Outbox entries for `label` that are due, as `deliver` frames, oldest first.
/// One held back by `deferred` holds back the ones after it, so order holds.
fn due_outbox(hub: &Hub, label: &str, in_flight: &mut HashMap<String, Instant>) -> Vec<Frame> {
    let ctx = cmd_ctx(&hub.ctx);
    let entries: Vec<Arc<MessageLink>> = ctx.exec_query(GetAllMessageLinks {}).unwrap_or_default();
    let ids: Vec<MessageId> = entries
        .iter()
        .filter(|e| e.outbound && e.link == label)
        .map(|e| MessageId(e.id.0.clone()))
        .collect();
    if ids.is_empty() {
        return Vec::new();
    }
    let mut messages: Vec<Arc<Message>> =
        ctx.exec_query(GetMessagesByIds { ids }).unwrap_or_default();
    messages.sort_by_key(|m| m.sent_at);
    let now_ms = chrono::Utc::now().timestamp_millis();
    let mut out = Vec::new();
    for m in messages {
        let id = m.id.0.to_string();
        let entry = entries.iter().find(|e| e.id.0.as_ref() == id.as_str());
        if entry.and_then(|e| e.retry_at).is_some_and(|at| at > now_ms) {
            break;
        }
        if in_flight
            .get(&id)
            .is_some_and(|sent| sent.elapsed() < UNANSWERED)
        {
            continue;
        }
        let Some(to) = m.to_session_id.as_ref() else {
            continue;
        };
        in_flight.insert(id.clone(), Instant::now());
        out.push(Frame::Deliver {
            id,
            from: m.from_session_id.0.to_string(),
            to: to.0.to_string(),
            body: m.body.clone(),
            response_required: m.response_required,
            sent_at: m.sent_at,
        });
        if out.len() >= 64 {
            break;
        }
    }
    out
}

fn drop_outbox(ctx: &CommandContext, id: &str) {
    let entries: Vec<Arc<MessageLink>> = ctx.exec_query(GetAllMessageLinks {}).unwrap_or_default();
    if let Some(e) = entries.iter().find(|e| e.outbound && e.id.0.as_ref() == id) {
        let _ = ctx.emit_del(&**e);
    }
}

/// Drop the outbox entry `id` and tell its sender why the far side won't take it.
fn refuse_back(ctx: &CommandContext, label: &str, id: &str, why: &str) {
    drop_outbox(ctx, id);
    let found: Vec<Arc<Message>> = ctx
        .exec_query(GetMessagesByIds {
            ids: vec![MessageId(Arc::from(id))],
        })
        .unwrap_or_default();
    let Some(original) = found.first() else {
        return;
    };
    let Some(to) = original.to_session_id.clone() else {
        return;
    };
    let notice_id = MessageId(Arc::from(
        muster_entities::inbound_message_id(label, &format!("refused:{id}")).as_str(),
    ));
    let notice = Message {
        response_required: false,
        id: notice_id.clone(),
        from_session_id: to,
        to_session_id: Some(original.from_session_id.clone()),
        to_room_id: None,
        to_operator: None,
        body: format!(
            "(automatic, from the link {label}) your message {id} wasn't delivered: {why}"
        ),
        sent_at: chrono::Utc::now().timestamp_millis(),
    };
    let _ = ctx.emit_set(&notice);
    let _ = ctx.emit_set(&MessageLink {
        id: MessageLinkId(notice_id.0.clone()),
        link: label.to_string(),
        outbound: false,
        remote_id: None,
        retry_at: None,
        note: None,
    });
}

/// Give up on links down longer than [`LINK_GIVE_UP`]: drop their remote
/// sessions, and refuse queued mail back to its senders. Spawn on a tokio task.
pub async fn run_sweeper(hub: Arc<Hub>) {
    let mut interval = tokio::time::interval(Duration::from_secs(60));
    loop {
        interval.tick().await;
        sweep(&hub, chrono::Utc::now().timestamp_millis());
    }
}

pub fn sweep(hub: &Hub, now_ms: i64) {
    let ctx = cmd_ctx(&hub.ctx);
    let give_up = LINK_GIVE_UP.as_millis() as i64;
    let remotes: Vec<Arc<RemoteSession>> =
        ctx.exec_query(GetAllRemoteSessions {}).unwrap_or_default();
    for r in remotes
        .iter()
        .filter(|r| !is_up(&r.link) && now_ms - r.seen_at > give_up)
    {
        let _ = ctx.emit_del(&**r);
    }
    let entries: Vec<Arc<MessageLink>> = ctx.exec_query(GetAllMessageLinks {}).unwrap_or_default();
    let outbound: Vec<_> = entries.iter().filter(|e| e.outbound).collect();
    if outbound.is_empty() {
        return;
    }
    let messages: Vec<Arc<Message>> = ctx
        .exec_query(GetMessagesByIds {
            ids: outbound.iter().map(|e| MessageId(e.id.0.clone())).collect(),
        })
        .unwrap_or_default();
    for e in outbound {
        let sent_at = messages
            .iter()
            .find(|m| m.id.0.as_ref() == e.id.0.as_ref())
            .map(|m| m.sent_at);
        match sent_at {
            Some(at) if now_ms - at > give_up && !is_up(&e.link) => refuse_back(
                &ctx,
                &e.link,
                e.id.0.as_ref(),
                "the link was down for more than a day",
            ),
            // The message itself was swept: nothing left to send.
            None => {
                let _ = ctx.emit_del(&**e);
            }
            _ => {}
        }
    }
}

// ── The listener's side: a gate connection on `/link` ───────────────────

/// Finish the WebSocket handshake the gate began (it has read the request head)
/// and run the link. `key` is the client's `Sec-WebSocket-Key`.
pub async fn accept<S>(
    mut stream: S,
    key: &str,
    label: &str,
    hub: &Hub,
    peer: SocketAddr,
) -> io::Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    use tokio::io::AsyncWriteExt as _;
    let accept = tokio_tungstenite::tungstenite::handshake::derive_accept_key(key.as_bytes());
    let response = format!(
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
         Sec-WebSocket-Accept: {accept}\r\n\r\n"
    );
    stream.write_all(response.as_bytes()).await?;
    stream.flush().await?;
    log::info!("[link] {label}: accepted from {peer}");
    let ws = WebSocketStream::from_raw_socket(
        stream,
        tokio_tungstenite::tungstenite::protocol::Role::Server,
        Some(ws_config()),
    )
    .await;
    run_session(ws, label, hub, || true).await
}

// ── The dialer's side ────────────────────────────────────────────────────

/// Keep every stored link connected, picking up `link` and `unlink` within a
/// few seconds. Spawn on a tokio task.
pub async fn run_dialers(hub: Arc<Hub>) {
    let mut running: HashMap<String, tokio::task::JoinHandle<()>> = HashMap::new();
    let mut interval = tokio::time::interval(Duration::from_secs(2));
    loop {
        interval.tick().await;
        let links = read_links(&hub.state_dir).unwrap_or_else(|e| {
            log::warn!("[link] can't read the stored links: {e}");
            Vec::new()
        });
        running.retain(|name, task| !task.is_finished() && links.iter().any(|l| &l.name == name));
        for link in links {
            if running.contains_key(&link.name) {
                continue;
            }
            let hub = hub.clone();
            let name = link.name.clone();
            running.insert(name, tokio::spawn(dial_forever(hub, link)));
        }
    }
}

fn still_linked(dir: &Path, link: &StoredLink) -> bool {
    read_links(dir)
        .map(|links| links.iter().any(|l| l == link))
        .unwrap_or(true)
}

async fn dial_forever(hub: Arc<Hub>, link: StoredLink) {
    let mut backoff = Duration::from_secs(1);
    loop {
        if !still_linked(&hub.state_dir, &link) {
            return;
        }
        let started = Instant::now();
        match dial(&link).await {
            Ok(ws) => {
                let dir = hub.state_dir.clone();
                let wanted = link.clone();
                if let Err(e) =
                    run_session(ws, &link.name, &hub, move || still_linked(&dir, &wanted)).await
                {
                    log::warn!("[link] {}: {e}", link.name);
                }
            }
            Err(e) => log::warn!(
                "[link] {}: can't connect to {}: {e}",
                link.name,
                link.address
            ),
        }
        // A connection that stayed up a while starts the backoff again.
        if started.elapsed() > BACKOFF_MAX {
            backoff = Duration::from_secs(1);
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(BACKOFF_MAX);
    }
}

/// Open the link to `link`'s gate: TLS with its certificate pinned, then the
/// WebSocket upgrade with the link token.
pub async fn dial(
    link: &StoredLink,
) -> io::Result<WebSocketStream<tokio_rustls::client::TlsStream<tokio::net::TcpStream>>> {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;
    let tcp = tokio::time::timeout(
        HELLO_TIMEOUT,
        tokio::net::TcpStream::connect(link.address.as_str()),
    )
    .await
    .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "connect timed out"))??;
    let _ = tcp.set_nodelay(true);
    let connector = tokio_rustls::TlsConnector::from(pinned_client_config(&link.fingerprint)?);
    let name = rustls::pki_types::ServerName::try_from("muster-daemon")
        .map_err(io::Error::other)?
        .to_owned();
    let tls = tokio::time::timeout(HELLO_TIMEOUT, connector.connect(name, tcp))
        .await
        .map_err(|_| {
            io::Error::new(
                io::ErrorKind::TimedOut,
                "no TLS answer: is TLS on at the far daemon?",
            )
        })??;
    let mut request = format!("ws://{}{LINK_PATH}", link.address)
        .into_client_request()
        .map_err(io::Error::other)?;
    request.headers_mut().insert(
        "Authorization",
        format!("Bearer {}", link.token)
            .parse()
            .map_err(io::Error::other)?,
    );
    let (ws, _) = tokio_tungstenite::client_async_with_config(request, tls, Some(ws_config()))
        .await
        .map_err(|e| match e {
            tokio_tungstenite::tungstenite::Error::Http(r) if r.status() == 401 => io::Error::new(
                io::ErrorKind::PermissionDenied,
                "the far daemon refused this link's token (401): it may have been unpaired there; \
                 link again with a new code",
            ),
            other => io::Error::other(other),
        })?;
    Ok(ws)
}

/// A rustls client config accepting exactly the certificate with SHA-256
/// `fingerprint`, still checking handshake signatures.
fn pinned_client_config(fingerprint: &str) -> io::Result<Arc<rustls::ClientConfig>> {
    use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
    use rustls::crypto::{CryptoProvider, verify_tls12_signature, verify_tls13_signature};
    use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
    use rustls::{DigitallySignedStruct, SignatureScheme};

    #[derive(Debug)]
    struct Pinned {
        fingerprint: String,
        provider: Arc<CryptoProvider>,
    }
    impl ServerCertVerifier for Pinned {
        fn verify_server_cert(
            &self,
            end_entity: &CertificateDer<'_>,
            _: &[CertificateDer<'_>],
            _: &ServerName<'_>,
            _: &[u8],
            _: UnixTime,
        ) -> Result<ServerCertVerified, rustls::Error> {
            if muster_entities::pairing::cert_fingerprint(end_entity) == self.fingerprint {
                Ok(ServerCertVerified::assertion())
            } else {
                Err(rustls::Error::InvalidCertificate(
                    rustls::CertificateError::ApplicationVerificationFailure,
                ))
            }
        }
        fn verify_tls12_signature(
            &self,
            message: &[u8],
            cert: &CertificateDer<'_>,
            dss: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, rustls::Error> {
            verify_tls12_signature(
                message,
                cert,
                dss,
                &self.provider.signature_verification_algorithms,
            )
        }
        fn verify_tls13_signature(
            &self,
            message: &[u8],
            cert: &CertificateDer<'_>,
            dss: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, rustls::Error> {
            verify_tls13_signature(
                message,
                cert,
                dss,
                &self.provider.signature_verification_algorithms,
            )
        }
        fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
            self.provider
                .signature_verification_algorithms
                .supported_schemes()
        }
    }

    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let verifier = Arc::new(Pinned {
        fingerprint: fingerprint.to_ascii_lowercase(),
        provider: provider.clone(),
    });
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(io::Error::other)?
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth();
    Ok(Arc::new(config))
}

/// Exchange a link invite at the far gate for a token (MUS-8's `/pair`).
pub async fn redeem(invite: &muster_entities::pairing::Invite) -> anyhow::Result<String> {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    let tcp = tokio::net::TcpStream::connect(invite.address.as_str())
        .await
        .map_err(|e| anyhow::anyhow!("can't reach {}: {e}", invite.address))?;
    let connector = tokio_rustls::TlsConnector::from(pinned_client_config(&invite.fingerprint)?);
    let name = rustls::pki_types::ServerName::try_from("muster-daemon")?.to_owned();
    let mut tls = tokio::time::timeout(HELLO_TIMEOUT, connector.connect(name, tcp))
        .await
        .map_err(|_| anyhow::anyhow!("{} didn't answer TLS: is TLS on there?", invite.address))?
        .map_err(|e| anyhow::anyhow!("TLS to {} failed: {e}", invite.address))?;
    let request = format!(
        "POST /pair HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nContent-Length: 2\r\n\
         Connection: close\r\n\r\n{{}}",
        invite.address, invite.invite
    );
    tls.write_all(request.as_bytes()).await?;
    let mut buf = Vec::new();
    let _ = tls.read_to_end(&mut buf).await;
    let text = String::from_utf8_lossy(&buf);
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    if !head.starts_with("HTTP/1.1 200") {
        anyhow::bail!(
            "the far daemon refused this code: it was already used or has expired. \
             Ask for a new one (`muster-daemon pair --name <this daemon> --link` there)"
        );
    }
    #[derive(Deserialize)]
    struct Paired {
        token: String,
    }
    let paired: Paired = serde_json::from_str(body)
        .map_err(|e| anyhow::anyhow!("the far daemon's reply wasn't understood: {e}"))?;
    Ok(paired.token)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_round_trip_as_tagged_json() {
        let f = Frame::Deferred {
            id: "m1".into(),
            retry_after: 3,
        };
        let text = serde_json::to_string(&f).unwrap();
        assert_eq!(text, r#"{"type":"deferred","id":"m1","retry_after":3}"#);
        assert_eq!(serde_json::from_str::<Frame>(&text).unwrap(), f);
        assert!(serde_json::from_str::<Frame>(r#"{"type":"run_command"}"#).is_err());
    }

    #[test]
    fn a_rate_limit_answer_names_its_wait() {
        assert_eq!(
            retry_after_secs("too many messages from this session: ...; try again in 7 s"),
            7
        );
        assert_eq!(retry_after_secs("something else"), 5);
    }

    #[test]
    fn peer_ids_cant_carry_separators() {
        assert!(acceptable_id("a0000000-0000-4000-8000-000000000001"));
        assert!(!acceptable_id("victim@murph"));
        assert!(!acceptable_id("a/b"));
        assert!(!acceptable_id(""));
        assert!(!acceptable_id(&"x".repeat(129)));
        assert!(!acceptable_id("has space"));
    }

    #[test]
    fn stored_links_round_trip() {
        let dir = std::env::temp_dir().join(format!("muster-links-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let link = StoredLink {
            name: "murph".into(),
            address: "192.0.2.10:6255".into(),
            fingerprint: "ab".repeat(32),
            token: "tok".into(),
        };
        save_link(&dir, link.clone()).unwrap();
        assert_eq!(read_links(&dir).unwrap(), vec![link]);
        assert!(remove_link(&dir, "murph").unwrap());
        assert!(!remove_link(&dir, "murph").unwrap());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn one_connection_per_link() {
        let first = claim("t-link");
        assert!(is_current("t-link", first));
        let second = claim("t-link");
        assert!(!is_current("t-link", first));
        assert!(is_current("t-link", second));
    }
}
