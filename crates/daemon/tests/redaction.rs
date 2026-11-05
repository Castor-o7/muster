//! MUS-57: a pairing code or bearer token in a message body is masked before
//! the message is stored, so it reaches no transcript.

use std::sync::Arc;

use hyphae::{Gettable, Materialize};
use muster_entities::{Message, SendMessage, Session, SessionId};
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

fn setup() -> MykoServerContext {
    muster_entities::link();
    daemon::link();
    let blackhole: Arc<dyn Persister> = Arc::new(BlackholePersister);
    let server = MykoServer::builder()
        .with_default_persister(blackhole)
        .build();
    let ctx = server.ctx();
    let _server: &'static MykoServer = Box::leak(Box::new(server));
    ctx
}

fn session(id: &str, client_id: Option<&str>) -> Session {
    Session {
        id: SessionId(Arc::from(id)),
        client_id: client_id.map(|c| ClientId(Arc::from(c))),
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
    }
}

fn set_session(ctx: &MykoServerContext, s: &Session) {
    let event = MEvent::from_item(s, MEventType::SET, &Uuid::new_v4().to_string());
    ctx.apply_event_batch(vec![event])
        .expect("apply Session SET");
}

fn cmd_ctx(ctx: &MykoServerContext, caller_client_id: Option<&str>) -> CommandContext {
    let req = RequestContext::new(
        Arc::<str>::from(Uuid::new_v4().to_string().as_str()),
        caller_client_id.map(Arc::<str>::from),
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

fn only_message(ctx: &MykoServerContext) -> Message {
    let store = ctx
        .registry
        .get(Message::ENTITY_NAME_STATIC)
        .expect("Message store exists");
    let entries = store.entries().materialize().get();
    assert_eq!(entries.len(), 1, "expected exactly one Message");
    myko::utils::downcast_item::<Message>(&entries.into_iter().next().unwrap().1)
        .expect("entry is a Message")
}

#[test]
fn a_pairing_code_in_a_message_is_stored_redacted() {
    let ctx = setup();
    set_session(&ctx, &session("sender", Some("c-sender")));
    set_session(&ctx, &session("recipient", None));
    let cmd = SendMessage {
        response_required: false,
        to_session_id: SessionId(Arc::from("recipient")),
        body: "Murph is at muster-pair:v1:eyJhZGRyZXNzIjoibXVycGgifQ and the header was Bearer tok_123".into(),
        as_session: None,
        idempotency_key: None,
    };
    cmd.execute(cmd_ctx(&ctx, Some("c-sender")))
        .expect("send succeeds");
    let stored = only_message(&ctx);
    assert_eq!(
        stored.body,
        "Murph is at muster-pair:v1:<redacted> and the header was Bearer <redacted>"
    );
}
