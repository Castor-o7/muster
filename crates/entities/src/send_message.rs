//! Server-handled `SendMessage` command.
//!
//! Persistence is primary; live push is a best-effort overlay. The pull
//! model decouples acceptance from delivery — the recipient reads on its
//! next turn via the hook, online or not. The handler:
//!   1. resolves the sender — `asSession` for connectionless HTTP-MCP
//!      callers, or the WS `client_id` for shim callers (`resolve_caller`),
//!   2. validates the recipient `Session` exists on the roster (it need
//!      NOT be online),
//!   3. persists the `Message` SET unconditionally,
//!   4. if the recipient has a live WS client, best-effort dispatches a
//!      `NotifyChannel` to surface it immediately (the Track-2 live path);
//!      otherwise the recipient pulls it next turn,
//!   5. returns separate status for the synchronous live-channel push and
//!      asynchronous wake boundary. The daemon cannot observe a later Codex
//!      bridge `turn/start`, so it never claims that wake succeeded.
//!
//! Only an unresolvable sender or unknown recipient returns a
//! `CommandError`; an offline recipient is success.

use std::sync::Arc;

use chrono::Utc;
use myko::prelude::{EventPublishing as _, RequestScoped as _};
use myko::{
    command::{CommandContext, CommandError, CommandHandler},
    myko_command,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    message::{
        CONTEXT_BODY_MAX_CHARS, GetMessagesByIds, Message, MessageId, body_too_large,
        context_preview,
    },
    message_read::{MessageRead, MessageReadId},
    notify::NotifyChannel,
    session::{GetAllSessions, Session, SessionId, resolve_caller},
    wake_watcher::{GetAllWakeWatchers, WakeWatcher},
};

#[myko_command(SendMessageResult)]
pub struct SendMessage {
    pub to_session_id: SessionId,
    #[serde(default)]
    pub response_required: bool,

    pub body: String,

    /// Self-identified sender, for callers with no connection identity —
    /// an agent calling this tool over stock myko's HTTP-MCP transport
    /// (no WS `client_id`). The id is the agent's own `session_id`, which
    /// it learned from its SessionStart hook output. WS shim callers omit
    /// this; the server derives the sender from the live connection.
    #[serde(default, rename = "asSession", skip_serializing_if = "Option::is_none")]
    pub as_session: Option<SessionId>,

    /// Makes a retry safe. Sending again with the key of a send whose outcome
    /// wasn't seen, such as one that timed out, stores and delivers nothing new.
    #[serde(
        default,
        rename = "idempotencyKey",
        skip_serializing_if = "Option::is_none"
    )]
    pub idempotency_key: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts-export", derive(myko::TS), ts(export))]
pub struct SendMessageResult {
    pub message_id: MessageId,
    /// Recipient's session id, echoed back so callers can confirm what
    /// was actually addressed (the daemon's caller resolution can
    /// disambiguate by host+cwd in edge cases the client couldn't see).
    pub to_session_id: SessionId,
    pub sent_at: i64,
    /// Outcome of the synchronous `NotifyChannel` path. This says nothing
    /// about a later wake bridge attempt.
    #[serde(default)]
    pub live_push: LivePushStatus,
    /// Whether this synchronous response observed an asynchronous wake.
    ///
    /// A durable unread message may be noticed by a Codex bridge after this
    /// command returns. Muster currently has no bridge acknowledgement path,
    /// so an inbox result is `Unobserved`, never a false claim of wake failure.
    #[serde(default)]
    pub wake: WakeStatus,
    /// Compatibility alias for clients predating `live_push`.
    ///
    /// This is `true` exactly when `live_push == Delivered`. It describes only
    /// the synchronous channel push; it does not describe asynchronous wake.
    #[serde(default)]
    pub delivered_live: bool,
    /// Set when the recipient is on a linked daemon (MUS-66): the message waits
    /// in this daemon's outbox for that link, and `link_up` says whether the
    /// link is up now. Absent for local recipients.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via_link: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link_up: Option<bool>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts-export", derive(myko::TS), ts(export))]
pub enum LivePushStatus {
    /// Compatibility default when a newer client reads an older daemon result.
    #[default]
    Unknown,
    /// The recipient's live channel accepted the push.
    Delivered,
    /// The recipient had no render-capable live channel, so no push was tried.
    Unavailable,
    /// A render-capable live channel was advertised, but dispatch failed.
    Failed,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts-export", derive(myko::TS), ts(export))]
pub enum WakeStatus {
    /// The live push landed. The durable inbox copy is marked read then, or,
    /// for a recipient that acks its own pushes, when it acks.
    NotNeeded,
    /// No live push landed, but a waiter holds `/hook/wait` for the recipient
    /// and wakes aren't paused, so the waiter will hand it the message and
    /// start a turn: the roster's `wake: waiting`. States that a path EXISTS,
    /// never that a wake happened.
    ///
    /// It exists because `live_push: unavailable` alone can't tell a sender
    /// whether the recipient will wake or will only see the message at its next
    /// turn. A live client alone doesn't count (MUS-71).
    Expected,
    /// Wake handling, if configured, happens after this response and is not
    /// observed by the sender.
    #[default]
    Unobserved,
}

impl SendMessageResult {
    /// Normalize a response from an older daemon that only populated
    /// `delivered_live`.
    pub fn effective_live_push(&self) -> LivePushStatus {
        match (self.live_push, self.delivered_live) {
            (LivePushStatus::Unknown, true) => LivePushStatus::Delivered,
            (status, _) => status,
        }
    }

    /// Normalize wake status across mixed daemon/client versions.
    pub fn effective_wake(&self) -> WakeStatus {
        if self.effective_live_push() == LivePushStatus::Delivered {
            WakeStatus::NotNeeded
        } else {
            self.wake
        }
    }
}

impl CommandHandler for SendMessage {
    #[cfg(target_arch = "wasm32")]
    fn execute(self, _ctx: CommandContext) -> Result<Self::Result, CommandError> {
        // SendMessage requires the live `client_registry`, which only the
        // server build has. Wasm clients never reach this — the macro skips
        // `register_command_handler!` on wasm — so the body is inert.
        unreachable!("SendMessage::execute is server-only");
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn execute(mut self, ctx: CommandContext) -> Result<Self::Result, CommandError> {
        // A pasted pairing code or token is masked before anything is stored
        // or delivered (MUS-57).
        self.body = crate::pairing::redact(&self.body).into_owned();
        if let Some(refusal) = body_too_large(&self.body) {
            return Err(err(&ctx, &refusal));
        }
        let sessions: Vec<Arc<Session>> = ctx.exec_query(GetAllSessions {})?;

        let sender = resolve_caller(&ctx, &sessions, self.as_session.as_ref())?;

        // Resolve the recipient token server-side so every harness (Claude via
        // the shim, the opencode plugin, raw HTTP-MCP) shares ONE resolution
        // policy — resolution used to live only in the shim, so plugin/HTTP
        // callers could address by exact id alone. Accepts id / nickname /
        // operator (human-via-agent) / id-prefix; see `resolve_recipient`.
        let token = self.to_session_id.0.clone();
        // An empty recipient would match every id by prefix and could land on
        // the sender itself.
        if token.trim().is_empty() {
            return Err(err(
                &ctx,
                "no recipient: `to` is empty; name a session by nickname, id or operator email (see muster://roster)",
            ));
        }
        // `nickname@link` names a session on a linked daemon (MUS-66).
        if let Some(remote) = resolve_remote(&ctx, &token, true)? {
            return self.send_remote(&ctx, &sender, &remote);
        }
        let (recipient, to_operator) = match resolve_recipient(&ctx, &sessions, &token) {
            Ok(found) => found,
            // A bare name prefers local; only when nothing local matches is a
            // linked daemon's session tried.
            Err(e) if e.message.starts_with("no session matches") => {
                match resolve_remote(&ctx, &token, false)? {
                    Some(remote) => return self.send_remote(&ctx, &sender, &remote),
                    None => return Err(e),
                }
            }
            Err(e) => return Err(e),
        };

        // The id comes from the sender and key, so a retry finds the first send.
        let id = match &self.idempotency_key {
            Some(key) => Uuid::new_v5(
                &Uuid::NAMESPACE_OID,
                format!("{}\0{key}", sender.id.0).as_bytes(),
            ),
            None => Uuid::now_v7(),
        };
        let id = MessageId(Arc::from(id.to_string()));
        if self.idempotency_key.is_some() {
            let found: Vec<Arc<Message>> = ctx.exec_query(GetMessagesByIds {
                ids: vec![id.clone()],
            })?;
            if let Some(first) = found.first() {
                if first.body != self.body || first.to_session_id.as_ref() != Some(&recipient.id) {
                    return Err(err(
                        &ctx,
                        "this idempotency key was already used for a different message",
                    ));
                }
                // Already stored, and delivered or waiting in the inbox. What
                // happened to its live push wasn't recorded, so don't claim any.
                return Ok(SendMessageResult {
                    message_id: first.id.clone(),
                    to_session_id: recipient.id.clone(),
                    sent_at: first.sent_at,
                    live_push: LivePushStatus::Unknown,
                    wake: WakeStatus::Unobserved,
                    delivered_live: false,
                    via_link: None,
                    link_up: None,
                });
            }
        }
        // A retry of a message already stored returned above, so only a new
        // message spends the sender's allowance (MUS-9).
        crate::controls::sender_allowed(sender.host.as_ref().map(|h| h.name.as_str()))
            .map_err(|why| err(&ctx, &why))?;
        crate::rate::take(
            sender.id.0.as_ref(),
            sender.host.as_ref().map(|h| h.name.as_str()),
        )
        .map_err(|why| err(&ctx, &why))?;
        let now = Utc::now().timestamp_millis();
        let msg = Message {
            response_required: self.response_required,
            id,
            from_session_id: sender.id.clone(),
            to_session_id: Some(recipient.id.clone()),
            to_room_id: None,
            to_operator: to_operator.clone(),
            body: self.body.clone(),
            sent_at: now,
        };

        // Persist unconditionally: in the pull model the recipient reads
        // this on its next turn via the hook, whether or not it has a live
        // connection right now. Delivery is decoupled from acceptance.
        ctx.emit_set(&msg)?;

        // Best-effort live push: only when the recipient has a live WS client
        // AND its session can actually render channel pushes. A flag-off
        // recipient (channels_enabled == Some(false)) has a live client but
        // Claude silently drops `notifications/claude/channel`, so a push is a
        // no-op the daemon would otherwise mis-report as delivered_live=true.
        // Treat it as offline: skip the push, report delivered_live=false, and
        // let it pull from the inbox next turn. `None` (legacy shim that
        // doesn't report channel state) keeps prior best-effort behavior.
        let recipient_can_render = recipient.channels_enabled != Some(false);
        // Surface the sender by its memorable nickname so the recipient sees who
        // it's from at a glance. Read the daemon-assigned, frozen handle
        // (`SessionNickname`); fall back to the computed candidate for the brief
        // window before the assignment saga has run. The full id stays in `meta`
        // for exact reply routing; `from_nickname` lets an agent reply by name.
        let from_nickname = crate::nickname_for(&ctx, sender.id.0.as_ref())?;
        let (context_body, body_truncated) = context_preview(&msg.body, CONTEXT_BODY_MAX_CHARS);
        let live_push = match recipient.client_id.as_ref() {
            Some(cid) if recipient_can_render => {
                if push_to_client(
                    cid.0.as_ref(),
                    // Concise origin ping only — no leading "muster:" (Claude
                    // already prefixes the channel name, else it reads "muster:
                    // muster:") and no body (it would just be a truncated
                    // banner). `meta.body` is a bounded model-context preview;
                    // the persisted Message retains the complete body.
                    direct_push_content(&from_nickname, to_operator.as_deref()),
                    serde_json::json!({
                        "source": "muster",
                        "kind": "new_message",
                        "message_id": msg.id.0.as_ref(),
                        "from_session": sender.id.0.as_ref(),
                        "from_nickname": from_nickname,
                        "to_session": recipient.id.0.as_ref(),
                        // Present when addressed to a human via their operator
                        // identity: the receiving agent should surface it to that
                        // operator, not treat it as ordinary peer chatter.
                        "to_operator": to_operator,
                        "response_required": self.response_required,
                        "body": context_body,
                        "body_truncated": body_truncated,
                        "sent_at": now,
                    }),
                ) {
                    LivePushStatus::Delivered
                } else {
                    LivePushStatus::Failed
                }
            }
            _ => LivePushStatus::Unavailable,
        };
        let delivered_live = live_push == LivePushStatus::Delivered;
        // Report a wake path only when one exists: the same rule as the
        // roster's `wake_state` (MUS-71). A live push that landed needs no
        // wake. Otherwise the message wakes the recipient only through a waiter
        // holding `/hook/wait` (a `WakeWatcher`), and only while the operator
        // hasn't paused wakes, since a paused `/hook/wait` hands out nothing.
        //
        // A live client alone isn't a wake path. A Claude session's MCP shim
        // without the channels flag is connected but can't wake it, and the
        // roster calls that session `none`; reporting `expected` there told
        // senders a session would wake when it wouldn't. Codex's MCP shim
        // registers no session, so its rows are hook-owned and its bridge's
        // wake leaves no trace here: those report `unobserved`, not
        // `expected`, as the roster reports `none`.
        let waiting = || {
            ctx.exec_query(GetAllWakeWatchers {})
                .unwrap_or_default()
                .iter()
                .any(|w: &Arc<WakeWatcher>| {
                    w.id.0.as_ref() == recipient.id.0.as_ref() && w.until > now
                })
        };
        let wake = match live_push {
            LivePushStatus::Delivered => WakeStatus::NotNeeded,
            _ if !crate::controls::wakes_paused() && waiting() => WakeStatus::Expected,
            _ => WakeStatus::Unobserved,
        };

        // Dedup the pull inbox against a landed live push. A channels-ON
        // recipient already rendered this message via the NotifyChannel, so
        // mark it read for them — the per-turn `<muster_inbox>` reads only
        // UNREAD direct messages, so it won't re-surface it. Without this the
        // recipient sees every direct message TWICE (once live, once in the
        // inbox). Best-effort: a failed write just falls back to that harmless
        // double-surface.
        //
        // "Delivered" here only means the push was queued on the connection,
        // so a client that then loses it would lose it already marked read. A
        // recipient that acks its own pushes (`acks_pushes`) is left unread
        // until that ack instead (MUS-81, upstream #140).
        if delivered_live && recipient.acks_pushes != Some(true) {
            let read_id = MessageRead::make_id(msg.id.0.as_ref(), recipient.id.0.as_ref());
            if let Err(e) = ctx.emit_set(&MessageRead {
                id: MessageReadId(Arc::from(read_id.as_str())),
                message_id: msg.id.clone(),
                session_id: recipient.id.clone(),
                read_at: now,
            }) {
                log::warn!("[send_message] live-push inbox dedup write failed: {e:?}");
            }
        }

        Ok(SendMessageResult {
            message_id: msg.id,
            to_session_id: recipient.id.clone(),
            sent_at: now,
            live_push,
            wake,
            delivered_live,
            via_link: None,
            link_up: None,
        })
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl SendMessage {
    /// A send to a session on a linked daemon (MUS-66): stored here with an
    /// outbox entry, which the link task forwards. No live push or wake from
    /// here; the far daemon does that when it stores the message.
    fn send_remote(
        self,
        ctx: &CommandContext,
        sender: &Session,
        remote: &crate::RemoteSession,
    ) -> Result<SendMessageResult, CommandError> {
        let to = SessionId(Arc::from(remote.session_id.as_str()));
        let id = match &self.idempotency_key {
            Some(key) => Uuid::new_v5(
                &Uuid::NAMESPACE_OID,
                format!("{}\0{key}", sender.id.0).as_bytes(),
            ),
            None => Uuid::now_v7(),
        };
        let id = MessageId(Arc::from(id.to_string()));
        if self.idempotency_key.is_some() {
            let found: Vec<Arc<Message>> = ctx.exec_query(GetMessagesByIds {
                ids: vec![id.clone()],
            })?;
            if let Some(first) = found.first() {
                if first.body != self.body || first.to_session_id.as_ref() != Some(&to) {
                    return Err(err(
                        ctx,
                        "this idempotency key was already used for a different message",
                    ));
                }
                return Ok(SendMessageResult {
                    message_id: first.id.clone(),
                    to_session_id: to,
                    sent_at: first.sent_at,
                    live_push: LivePushStatus::Unavailable,
                    wake: WakeStatus::Unobserved,
                    delivered_live: false,
                    via_link: Some(remote.link.clone()),
                    link_up: Some(remote.link_up),
                });
            }
        }
        crate::controls::sender_allowed(sender.host.as_ref().map(|h| h.name.as_str()))
            .map_err(|why| err(ctx, &why))?;
        crate::rate::take(
            sender.id.0.as_ref(),
            sender.host.as_ref().map(|h| h.name.as_str()),
        )
        .map_err(|why| err(ctx, &why))?;
        let now = Utc::now().timestamp_millis();
        let msg = Message {
            response_required: self.response_required,
            id: id.clone(),
            from_session_id: sender.id.clone(),
            to_session_id: Some(to.clone()),
            to_room_id: None,
            to_operator: None,
            body: self.body,
            sent_at: now,
        };
        ctx.emit_set(&msg)?;
        ctx.emit_set(&crate::MessageLink {
            id: crate::MessageLinkId(id.0.clone()),
            link: remote.link.clone(),
            outbound: true,
            remote_id: None,
            retry_at: None,
            note: None,
        })?;
        crate::OUTBOX_GENERATION.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(SendMessageResult {
            message_id: id,
            to_session_id: to,
            sent_at: now,
            live_push: LivePushStatus::Unavailable,
            wake: WakeStatus::Unobserved,
            delivered_live: false,
            via_link: Some(remote.link.clone()),
            link_up: Some(remote.link_up),
        })
    }
}

/// A session on a linked daemon named by `token` (MUS-66). `nickname@link` (or
/// `session-id@link`) names one explicitly. A bare token is tried only when no
/// local session matched it: a bare name prefers local. Exact id, then a unique
/// nickname, then a unique id prefix. `Ok(None)` when nothing remote matches.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn resolve_remote(
    ctx: &CommandContext,
    token: &str,
    explicit_only: bool,
) -> Result<Option<Arc<crate::RemoteSession>>, CommandError> {
    let remotes: Vec<Arc<crate::RemoteSession>> = ctx.exec_query(crate::GetAllRemoteSessions {})?;
    if remotes.is_empty() {
        return Ok(None);
    }
    if let Some((name, link)) = token.rsplit_once(crate::LINK_SEPARATOR)
        && remotes.iter().any(|r| r.link == link)
    {
        let on_link: Vec<_> = remotes.iter().filter(|r| r.link == link).collect();
        if let Some(r) = on_link
            .iter()
            .find(|r| r.nickname == name || r.session_id == name)
        {
            return Ok(Some((*r).clone()));
        }
        return Err(err(
            ctx,
            &format!("no session '{name}' on the link {link} — check muster://roster"),
        ));
    }
    if explicit_only {
        return Ok(None);
    }
    if let Some(r) = remotes.iter().find(|r| r.session_id == token) {
        return Ok(Some(r.clone()));
    }
    let pick = |matches: Vec<&Arc<crate::RemoteSession>>, how: &str| match matches.len() {
        0 => Ok(None),
        1 => Ok(Some(matches[0].clone())),
        _ => Err(err(
            ctx,
            &format!(
                "{how} '{token}' matches sessions on more than one link: {}; name one as nickname@link",
                matches
                    .iter()
                    .map(|r| r.address())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        )),
    };
    if let Some(r) = pick(
        remotes.iter().filter(|r| r.nickname == token).collect(),
        "nickname",
    )? {
        return Ok(Some(r));
    }
    pick(
        remotes
            .iter()
            .filter(|r| r.session_id.starts_with(token))
            .collect(),
        "id-prefix",
    )
}

fn err(ctx: &CommandContext, message: &str) -> CommandError {
    CommandError {
        tx: ctx.tx().to_string(),
        command_id: ctx.command_id.to_string(),
        message: message.to_string(),
    }
}

/// Resolve a caller-supplied recipient token against the live roster. Runs
/// server-side (inside the command) rather than in each client, so the shim,
/// the opencode plugin, and raw HTTP-MCP callers all share one policy.
///
/// Priority order:
///   1. exact session id;
///   2. a unique nickname (`swift-falcon`, what the statusline shows);
///   3. an OPERATOR identity — the human, addressed by their resolved operator
///      string (an email like `operator@example.com`, optionally `op:`/`human:`
///      prefixed). This is human-via-agent routing: it delivers to that
///      operator's most-recently-active session, preferring one with a live,
///      render-capable client, so the sender addresses the *person* and the
///      daemon picks which of their agents currently has the floor. If none of
///      the operator's sessions is live the freshest still takes it and reads
///      from its inbox next turn — delivery is decoupled from liveness;
///   4. a unique session-id prefix.
///
/// Ambiguous or unknown tokens error with the candidates listed, so a caller
/// can never silently mis-route.
///
/// A resolved recipient session plus the operator identity it was addressed as:
/// `Some` only when resolution went through the operator (human-via-agent) tier,
/// which marks the message human-addressed (`Message::to_operator`); `None` for
/// id / nickname / prefix resolution (agent-to-agent).
#[cfg(not(target_arch = "wasm32"))]
type ResolvedRecipient = (Arc<Session>, Option<String>);

/// Returns the resolved session plus, when resolution went through the operator
/// tier (3), the operator identity that was addressed — so the caller can mark
/// the message human-addressed (`Message::to_operator`). `None` for the id /
/// nickname / prefix tiers (agent-addressed).
#[cfg(not(target_arch = "wasm32"))]
fn resolve_recipient(
    ctx: &CommandContext,
    sessions: &[Arc<Session>],
    token: &str,
) -> Result<ResolvedRecipient, CommandError> {
    // 1. Unique nickname. The daemon assigns these, whereas a client chooses its
    // session id, so a nickname must win: otherwise a client could register an id
    // equal to a peer's nickname and receive that peer's mail.
    let mut by_nick: Vec<Arc<Session>> = Vec::new();
    for s in sessions {
        if crate::nickname_for(ctx, s.id.0.as_ref())? == token {
            by_nick.push(s.clone());
        }
    }
    if by_nick.len() == 1 {
        return Ok((by_nick.remove(0), None));
    }
    if by_nick.len() > 1 {
        return Err(err(ctx, &ambiguous(ctx, "nickname", token, &by_nick)));
    }
    // 2. Exact session id.
    if let Some(s) = sessions.iter().find(|s| s.id.0.as_ref() == token) {
        return Ok((s.clone(), None));
    }
    // 3. Operator identity — human-via-agent routing. Record the operator we
    // addressed (the resolved session's canonical operator string, falling back
    // to the bare token) so the message is marked human-addressed downstream.
    let op = token
        .strip_prefix("op:")
        .or_else(|| token.strip_prefix("human:"))
        .unwrap_or(token);
    if let Some(best) = pick_operator_session(sessions, op) {
        let addressed = best.operator.clone().unwrap_or_else(|| op.to_string());
        return Ok((best, Some(addressed)));
    }
    // 4. Unique session-id prefix.
    let by_prefix: Vec<Arc<Session>> = sessions
        .iter()
        .filter(|s| s.id.0.as_ref().starts_with(token))
        .cloned()
        .collect();
    match by_prefix.len() {
        1 => Ok((by_prefix.into_iter().next().unwrap(), None)),
        0 => Err(err(
            ctx,
            &format!(
                "no session matches '{token}' (by id, nickname, operator, or id-prefix) — check muster://roster"
            ),
        )),
        _ => Err(err(ctx, &ambiguous(ctx, "id-prefix", token, &by_prefix))),
    }
}

/// Best-effort recipient resolution for `@mentions` in a broadcast body:
/// exact id, unique nickname, or operator identity (human-via-agent,
/// most-active). Returns `None` for unknown / ambiguous tokens — a mention is
/// an explicit handle, so we never guess or error on it; an unresolvable
/// `@token` is just prose. Shared with `BroadcastMessage`'s @mention escape
/// hatch. Mirrors tiers 1–3 of `resolve_recipient` minus the prefix tier and
/// the hard errors.
///
/// Like `resolve_recipient`, the second tuple element is the addressed operator
/// identity when resolution went through the operator tier (so a human
/// `@mention` produces a human-addressed DM), else `None`.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn resolve_mention(
    ctx: &CommandContext,
    sessions: &[Arc<Session>],
    token: &str,
) -> Result<Option<ResolvedRecipient>, CommandError> {
    // 1. Unique nickname (ambiguous → treat as prose, not a hard error). As in
    // `resolve_recipient`, a daemon-assigned nickname wins over a chosen id.
    let mut by_nick: Vec<Arc<Session>> = Vec::new();
    for s in sessions {
        if crate::nickname_for(ctx, s.id.0.as_ref())? == token {
            by_nick.push(s.clone());
        }
    }
    if by_nick.len() == 1 {
        return Ok(Some((by_nick.remove(0), None)));
    }
    if by_nick.len() > 1 {
        return Ok(None);
    }
    // 2. Exact session id.
    if let Some(s) = sessions.iter().find(|s| s.id.0.as_ref() == token) {
        return Ok(Some((s.clone(), None)));
    }
    // 3. Operator identity — human-via-agent.
    let op = token
        .strip_prefix("op:")
        .or_else(|| token.strip_prefix("human:"))
        .unwrap_or(token);
    Ok(pick_operator_session(sessions, op).map(|best| {
        let addressed = best.operator.clone().unwrap_or_else(|| op.to_string());
        (best, Some(addressed))
    }))
}

/// Choose which of an operator's sessions receives a human-via-agent message:
/// the most-recently-active one, with live + render-capable sessions ranked
/// above idle/disconnected ones (the human's current focus). `None` when the
/// operator has no session on the roster. Pure over the roster so the routing
/// policy is unit-testable without a live command context.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn pick_operator_session(sessions: &[Arc<Session>], op: &str) -> Option<Arc<Session>> {
    let now = Utc::now().timestamp_millis();
    sessions
        .iter()
        .filter(|s| {
            s.operator
                .as_deref()
                .is_some_and(|o| o.eq_ignore_ascii_case(op))
        })
        .cloned()
        .max_by_key(|s| {
            let live = s.client_id.is_some() && s.channels_enabled != Some(false);
            // Clients report this time. One from the future isn't activity, and
            // would otherwise win this session the operator's mail indefinitely.
            let active = s.last_activity_at.unwrap_or(s.connected_at);
            (live as i64, if active > now { 0 } else { active })
        })
}

#[cfg(not(target_arch = "wasm32"))]
fn ambiguous(ctx: &CommandContext, kind: &str, token: &str, matches: &[Arc<Session>]) -> String {
    let list = matches
        .iter()
        .map(|s| {
            let nick = crate::nickname_for(ctx, s.id.0.as_ref()).unwrap_or_default();
            format!("{nick} ({})", s.id.0.as_ref())
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "'{token}' is an ambiguous {kind} matching {} sessions [{list}] — address by full session_id",
        matches.len()
    )
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn push_to_client(client_id: &str, content: String, meta: serde_json::Value) -> bool {
    use myko::{command::CommandRequest, server::try_client_registry};
    // A live push wakes its session, so none goes out while the operator has
    // paused wake-ups (MUS-10). The message is stored either way.
    if crate::controls::wakes_paused() {
        return false;
    }
    let Some(registry) = try_client_registry() else {
        log::warn!("[send_message] client_registry not initialized; cannot deliver");
        return false;
    };
    let cmd = NotifyChannel { content, meta };
    let request = CommandRequest::new(cmd);
    registry.send_command_request_to(client_id, &request)
}

/// The channel banner for a direct message. Agent-addressed mail stays a
/// concise origin ping; human-addressed mail opens with the relay-to-operator
/// contract so the recipient agent puts it in front of the person instead of
/// acting on it. Pure so it can be asserted without a live client registry.
pub(crate) fn direct_push_content(from_nickname: &str, to_operator: Option<&str>) -> String {
    match to_operator {
        Some(op) => format!(
            "{} New message from {from_nickname}.",
            crate::message::operator_relay_notice(op)
        ),
        None => format!("new message from {from_nickname}"),
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod operator_routing_tests {
    use super::*;

    fn session(id: &str, last_activity_at: i64) -> Arc<Session> {
        Arc::new(Session {
            id: SessionId(Arc::from(id)),
            client_id: None,
            pid: 0,
            cwd: String::new(),
            git_branch: None,
            current_task: None,
            session_name: None,
            activity: None,
            kind: None,
            connected_at: 0,
            last_activity_at: Some(last_activity_at),
            last_tool: None,
            last_tool_at: None,
            operator: Some("op@example.test".to_string()),
            host: None,
            project: None,
            channels_enabled: None,
            acks_pushes: None,
        })
    }

    #[test]
    fn a_future_activity_time_does_not_win_operator_mail() {
        let now = Utc::now().timestamp_millis();
        let sessions = [
            session("honest", now - 1_000),
            session("liar", now + 86_400_000),
        ];
        let picked = pick_operator_session(&sessions, "op@example.test").unwrap();
        assert_eq!(picked.id.0.as_ref(), "honest");
    }
}

#[cfg(test)]
mod push_content_tests {
    use super::direct_push_content;

    #[test]
    fn agent_addressed_banner_is_unchanged() {
        assert_eq!(
            direct_push_content("fleet-wolf", None),
            "new message from fleet-wolf"
        );
    }

    #[test]
    fn human_addressed_banner_opens_with_the_relay_contract() {
        let content = direct_push_content("fleet-wolf", Some("operator@example.com"));
        assert!(content.starts_with("For operator (operator@example.com): relay to them;"));
        assert!(content.ends_with("New message from fleet-wolf."));
    }
}
