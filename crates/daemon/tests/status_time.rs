//! MUS-113: `SetSessionCurrentTask` stamps `current_task_at` itself, so a client that sets a
//! status without a time (the TypeScript plugins) can't leave an older time beside it.

use std::sync::Arc;

use hyphae::{Gettable, Materialize};
use muster_entities::{Session, SessionId, SetSessionCurrentTask};
use myko::{
    command::{CommandContext, CommandHandler},
    core::item::Eventable,
    request::RequestContext,
    server::{MykoServerContext, Persister},
    wire::{MEvent, MEventType},
};
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
    let _: &'static MykoServer = Box::leak(Box::new(server));
    ctx
}

fn session(id: &str) -> Session {
    Session {
        id: SessionId(Arc::from(id)),
        client_id: None,
        pid: 0,
        cwd: "/repo".into(),
        git_branch: None,
        current_task: Some("day 1".into()),
        session_name: None,
        activity: None,
        kind: None,
        connected_at: 100,
        last_activity_at: None,
        last_tool: None,
        last_tool_at: None,
        // Set hours ago, with the status it described.
        current_task_at: Some(1_000),
        operator: None,
        host: None,
        project: None,
        channels_enabled: None,
        acks_pushes: None,
        harness: None,
        identity: None,
    }
}

fn cmd_ctx(ctx: &MykoServerContext) -> CommandContext {
    let req = RequestContext::new(
        Arc::<str>::from(Uuid::new_v4().to_string().as_str()),
        None,
        vec![Arc::<str>::from("test")],
        Uuid::new_v4(),
        chrono::Utc::now().to_rfc3339(),
    );
    CommandContext::new(
        Arc::<str>::from("SetSessionCurrentTask"),
        Arc::new(req),
        Arc::new(ctx.clone()),
    )
}

fn only_session(ctx: &MykoServerContext) -> Session {
    let store = ctx
        .registry
        .get(Session::ENTITY_NAME_STATIC)
        .expect("Session store exists");
    let entries = store.entries().materialize().get();
    assert_eq!(entries.len(), 1);
    myko::utils::downcast_item::<Session>(&entries.into_iter().next().unwrap().1)
        .expect("entry is a Session")
}

#[test]
fn a_status_set_without_a_time_is_stamped_now_and_a_cleared_one_has_none() {
    let ctx = setup();
    let event = MEvent::from_item(&session("s1"), MEventType::SET, &Uuid::new_v4().to_string());
    ctx.apply_event_batch(vec![event])
        .expect("apply Session SET");

    // What a TypeScript plugin sends: the status, and no time.
    let wire = serde_json::json!({"id": "s1", "currentTask": "MUS-113"});
    let cmd: SetSessionCurrentTask = serde_json::from_value(wire).expect("the plugins' shape");
    let before = chrono::Utc::now().timestamp_millis();
    cmd.execute(cmd_ctx(&ctx)).expect("set the status");
    let after = chrono::Utc::now().timestamp_millis();
    let s = only_session(&ctx);
    assert_eq!(s.current_task.as_deref(), Some("MUS-113"));
    let at = s.current_task_at.expect("stamped");
    assert!(
        (before..=after).contains(&at),
        "the old time {at} was left beside the new status"
    );

    SetSessionCurrentTask {
        id: SessionId(Arc::from("s1")),
        current_task: None,
    }
    .execute(cmd_ctx(&ctx))
    .expect("clear the status");
    let s = only_session(&ctx);
    assert_eq!(s.current_task, None);
    assert_eq!(s.current_task_at, None, "no status, no time");
    assert_eq!(s.cwd, "/repo", "the rest of the row is kept");
}

#[test]
fn an_unknown_session_is_refused() {
    let ctx = setup();
    let err = SetSessionCurrentTask {
        id: SessionId(Arc::from("nobody")),
        current_task: Some("x".into()),
    }
    .execute(cmd_ctx(&ctx));
    assert!(err.is_err());
}
