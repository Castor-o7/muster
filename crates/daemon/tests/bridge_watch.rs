//! `/hook/bridge-watch` (MUS-77): a Codex app-server bridge saying it can wake a thread, so the
//! roster shows the wake path (`waiting`) and a send to it says `expected`.
//!
//! One test, because the operator's pause is process-wide. It also covers MUS-94: each
//! bridge, and a held `/hook/wait`, has a wake-path record of its own.

use std::sync::Arc;

use muster_entities::{
    GetAllWakeWatchers, SendMessage, Session, SessionId, WakeStatus, WakeWatcher,
};
use myko::{
    command::{CommandContext, CommandHandler},
    core::item::Eventable,
    entities::client::ClientId,
    request::RequestContext,
    server::{MykoServerContext, Persister},
    wire::{MEvent, MEventType},
};
use myko_server::{BlackholePersister, MykoServer};
use uuid::Uuid;

fn setup() -> Arc<MykoServerContext> {
    muster_entities::link();
    daemon::link();
    let blackhole: Arc<dyn Persister> = Arc::new(BlackholePersister);
    let server = MykoServer::builder()
        .with_default_persister(blackhole)
        .build();
    let ctx = server.ctx();
    Box::leak(Box::new(server));
    Arc::new(ctx)
}

fn session(id: &str, client: Option<&str>) -> Session {
    Session {
        id: SessionId(Arc::from(id)),
        client_id: client.map(|c| ClientId(Arc::from(c))),
        pid: 0,
        cwd: "/repo".into(),
        git_branch: None,
        current_task: None,
        session_name: None,
        activity: None,
        kind: None,
        connected_at: 100,
        last_activity_at: None,
        last_tool: None,
        last_tool_at: None,
        operator: None,
        host: None,
        project: None,
        channels_enabled: None,
        acks_pushes: None,
        harness: None,
        identity: None,
    }
}

fn set<T: Eventable>(ctx: &MykoServerContext, item: &T) {
    let event = MEvent::from_item(item, MEventType::SET, &Uuid::new_v4().to_string());
    ctx.apply_event_batch(vec![event]).expect("apply SET");
}

fn cmd_ctx(ctx: &MykoServerContext, client: &str) -> CommandContext {
    let req = RequestContext::new(
        Arc::<str>::from(Uuid::new_v4().to_string().as_str()),
        Some(Arc::<str>::from(client)),
        vec![Arc::<str>::from("test")],
        Uuid::new_v4(),
        chrono::Utc::now().to_rfc3339(),
    );
    CommandContext::new(
        Arc::<str>::from("SendMessage"),
        Arc::new(req),
        Arc::new(ctx.clone()),
    )
}

fn watch(ctx: &Arc<MykoServerContext>, sid: &str, watching: bool) -> String {
    let body = format!(r#"{{"session_id":"{sid}","watching":{watching}}}"#);
    daemon::hooks::dispatch("/hook/bridge-watch", "harness=codex", body.as_bytes(), ctx)
        .expect("a known route")
        .body
}

/// A watch from the bridge process named `bridge`.
fn watch_as(ctx: &Arc<MykoServerContext>, bridge: &str, sid: &str, watching: bool) -> String {
    let body = format!(r#"{{"session_id":"{sid}","watching":{watching},"bridge":"{bridge}"}}"#);
    daemon::hooks::dispatch("/hook/bridge-watch", "harness=codex", body.as_bytes(), ctx)
        .expect("a known route")
        .body
}

fn watched(ctx: &MykoServerContext, sid: &str) -> bool {
    let now = chrono::Utc::now().timestamp_millis();
    cmd_ctx(ctx, "c-sender")
        .exec_query(GetAllWakeWatchers {})
        .unwrap_or_default()
        .iter()
        .any(|w: &Arc<WakeWatcher>| w.session() == sid && w.until > now)
}

fn send_wake(ctx: &MykoServerContext, to: &str) -> WakeStatus {
    SendMessage {
        to_session_id: SessionId(Arc::from(to)),
        response_required: false,
        body: "hi".into(),
        as_session: None,
        idempotency_key: None,
    }
    .execute(cmd_ctx(ctx, "c-sender"))
    .expect("sent")
    .wake
}

#[test]
fn a_bridge_watched_thread_shows_its_wake_path_until_the_bridge_stops() {
    let ctx = setup();
    set(&ctx, &session("sender", Some("c-sender")));
    // A Codex thread: registered by its hooks, with no MCP client and no waiter.
    set(&ctx, &session("thread-1", None));

    assert!(!watched(&ctx, "thread-1"));
    assert_ne!(send_wake(&ctx, "thread-1"), WakeStatus::Expected);

    assert_eq!(watch(&ctx, "thread-1", true), "watching");
    assert!(
        watched(&ctx, "thread-1"),
        "the roster now has a WakeWatcher"
    );
    assert_eq!(send_wake(&ctx, "thread-1"), WakeStatus::Expected);

    // A session that isn't on the roster gets no record.
    assert_eq!(watch(&ctx, "nobody", true), "not watching");
    assert!(!watched(&ctx, "nobody"));

    // The bridge stops: the record goes.
    assert_eq!(watch(&ctx, "thread-1", false), "not watching");
    assert!(!watched(&ctx, "thread-1"));
    assert_ne!(send_wake(&ctx, "thread-1"), WakeStatus::Expected);

    // While wakes are paused, none is kept, even if the bridge asks.
    muster_entities::controls::set_wakes_paused(true);
    assert_eq!(watch(&ctx, "thread-1", true), "not watching");
    assert!(!watched(&ctx, "thread-1"));
    muster_entities::controls::set_wakes_paused(false);
    assert_eq!(watch(&ctx, "thread-1", true), "watching");
    assert!(watched(&ctx, "thread-1"));
    assert_eq!(watch(&ctx, "thread-1", false), "not watching");

    // MUS-94: wake leadership passes from bridge A to bridge B. B watches first, then A's
    // late unwatch arrives. B's record stays.
    set(&ctx, &session("thread-2", None));
    assert_eq!(watch_as(&ctx, "bridge-a", "thread-2", true), "watching");
    assert_eq!(watch_as(&ctx, "bridge-b", "thread-2", true), "watching");
    assert_eq!(
        watch_as(&ctx, "bridge-a", "thread-2", false),
        "not watching"
    );
    assert!(
        watched(&ctx, "thread-2"),
        "a stale leader's unwatch must leave the new leader's record"
    );
    assert_eq!(send_wake(&ctx, "thread-2"), WakeStatus::Expected);
    assert_eq!(
        watch_as(&ctx, "bridge-b", "thread-2", false),
        "not watching"
    );
    assert!(!watched(&ctx, "thread-2"));

    // MUS-94: a held /hook/wait and a bridge for the same session. The bridge's
    // `watching:false` leaves the held wait's record, and the wait's end leaves the bridge's.
    set(&ctx, &session("both", None));
    let internal = cmd_ctx(&ctx, "c-daemon");
    daemon::hooks::mark_waiting_now(&internal, "both");
    assert_eq!(watch_as(&ctx, "bridge-a", "both", true), "watching");
    assert_eq!(watch_as(&ctx, "bridge-a", "both", false), "not watching");
    assert!(
        watched(&ctx, "both"),
        "a bridge's watching:false must leave a held wait's record"
    );
    assert_eq!(watch_as(&ctx, "bridge-a", "both", true), "watching");
    daemon::hooks::end_wait(&internal, "both");
    assert!(
        watched(&ctx, "both"),
        "a held wait's end must leave the bridge's record"
    );

    // When the session ends, every wake path to it goes.
    daemon::hooks::mark_waiting_now(&internal, "both");
    let ended = r#"{"session_id":"both"}"#;
    daemon::hooks::dispatch("/hook/session-end", "", ended.as_bytes(), &ctx)
        .expect("a known route");
    let left: Vec<Arc<WakeWatcher>> = internal
        .exec_query(GetAllWakeWatchers {})
        .unwrap_or_default()
        .into_iter()
        .filter(|w| w.session() == "both")
        .collect();
    assert!(
        left.is_empty(),
        "an ended session keeps no wake path: {left:?}"
    );
}
