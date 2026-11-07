//! MUS-93: a restart whose predecessor never had a client, and was active
//! inside the quiet time when the new session registered, as a Claude session
//! restarted before its MCP shim bound. In its own test binary, because the
//! sweeper's observations (`succession::observe`) are process-wide.

use std::sync::Arc;

use daemon::{nickname_assign::ensure_assigned, succession};
use muster_entities::{HostInfo, Session, SessionId};
use myko::command::CommandContext;
use myko::server::{MykoServerContext, Persister};
use myko_server::{BlackholePersister, MykoServer};
use uuid::Uuid;

fn setup() -> MykoServerContext {
    muster_entities::link();
    daemon::link();
    let blackhole: Arc<dyn Persister> = Arc::new(BlackholePersister);
    let server = MykoServer::builder()
        .with_default_persister(blackhole)
        .build();
    let ctx = server.ctx();
    Box::leak(Box::new(server));
    ctx
}

/// A Claude session with no client bound, last active at `at`.
fn session(cwd: &str, at: i64) -> Session {
    Session {
        id: SessionId(Arc::from(Uuid::new_v4().to_string().as_str())),
        client_id: None,
        pid: 1,
        cwd: cwd.into(),
        git_branch: None,
        current_task: None,
        session_name: None,
        activity: None,
        kind: None,
        connected_at: at,
        last_activity_at: Some(at),
        last_tool: None,
        last_tool_at: None,
        operator: Some("op@example.com".into()),
        host: Some(HostInfo {
            name: "a1".into(),
            os: String::new(),
            arch: String::new(),
        }),
        project: None,
        channels_enabled: None,
        acks_pushes: None,
        harness: Some("claude".into()),
        identity: None,
    }
}

fn cmd(ctx: &MykoServerContext) -> CommandContext {
    succession::internal_ctx(ctx)
}

/// Register a session and name it, as the session-start hook does.
fn start(ctx: &MykoServerContext, s: &Session) -> String {
    ctx.set(s).expect("session SET");
    ensure_assigned(&cmd(ctx), s.id.0.as_ref()).expect("named")
}

fn unique_cwd() -> String {
    format!("/work/{}", Uuid::new_v4())
}

/// The quiet time and the swap window, at their defaults (600 s each).
const QUIET_MS: i64 = 600_000;
const WINDOW_MS: i64 = 600_000;

#[test]
fn a_predecessor_live_only_for_its_quiet_time_is_still_taken_over() {
    let ctx = setup();
    let cwd = unique_cwd();
    let t0 = chrono::Utc::now().timestamp_millis();
    // The old session hooked just now, and its MCP shim never bound before it
    // ended: no client, so only the quiet time can tell it's gone.
    let old = session(&cwd, t0);
    let nick = start(&ctx, &old);
    let new = session(&cwd, t0);
    let fresh = start(&ctx, &new);
    assert_ne!(
        fresh, nick,
        "a predecessor active inside the quiet time isn't taken over at registration"
    );
    // Before the quiet time has passed, a late check still finds it live.
    succession::recheck_pending(&cmd(&ctx), t0 + QUIET_MS - 1_000);
    assert_eq!(
        ensure_assigned(&cmd(&ctx), new.id.0.as_ref()).unwrap(),
        fresh
    );
    // Once it has, the late check takes it over: the same rule, applied later.
    succession::recheck_pending(&cmd(&ctx), t0 + QUIET_MS);
    assert_eq!(
        ensure_assigned(&cmd(&ctx), new.id.0.as_ref()).unwrap(),
        nick,
        "once its quiet time has passed, the new session takes its nickname"
    );

    // The other side, in the same test because the sweeper's state is shared:
    // a sibling that keeps signalling is never taken over, even after the swap
    // window, when nothing checks any more.
    let cwd = unique_cwd();
    let t0 = chrono::Utc::now().timestamp_millis();
    let sibling = session(&cwd, t0);
    let sibling_nick = start(&ctx, &sibling);
    let later = session(&cwd, t0);
    let later_nick = start(&ctx, &later);
    assert_ne!(later_nick, sibling_nick);
    let mut signalled = sibling.clone();
    for at in [t0 + 300_000, t0 + 590_000] {
        signalled.last_activity_at = Some(at);
        ctx.set(&signalled).expect("sibling signals");
        succession::recheck_pending(&cmd(&ctx), at + 1_000);
    }
    succession::recheck_pending(&cmd(&ctx), t0 + WINDOW_MS);
    // Past the window it would count as dead, but the check has ended.
    succession::recheck_pending(&cmd(&ctx), t0 + 590_000 + QUIET_MS);
    assert_eq!(
        ensure_assigned(&cmd(&ctx), later.id.0.as_ref()).unwrap(),
        later_nick,
        "a sibling that kept signalling through the swap window keeps its name"
    );
}
