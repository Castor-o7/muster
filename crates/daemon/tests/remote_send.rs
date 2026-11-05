//! Sending to a session on a linked daemon (MUS-66): `nickname@link` names
//! one explicitly, a bare name prefers a local session, and the message goes
//! into the outbox rather than any local inbox.

use std::sync::Arc;

use hyphae::{Gettable, Materialize};
use muster_entities::{
    GetAllMessageLinks, Message, MessageLink, RemoteSession, RemoteSessionId, SendMessage, Session,
    SessionId,
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

fn session(id: &str, client: &str) -> Session {
    Session {
        id: SessionId(Arc::from(id)),
        client_id: Some(ClientId(Arc::from(client))),
        agent_id: None,
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

fn remote(session_id: &str, nickname: &str, link: &str) -> RemoteSession {
    RemoteSession {
        id: RemoteSessionId(Arc::from(RemoteSession::make_id(session_id, link).as_str())),
        link: link.into(),
        session_id: session_id.into(),
        nickname: nickname.into(),
        host: None,
        operator: None,
        project: None,
        current_task: None,
        activity: None,
        wake: None,
        last_activity_at: None,
        link_up: true,
        seen_at: 0,
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

fn send(ctx: &MykoServerContext, to: &str) -> Result<muster_entities::SendMessageResult, String> {
    SendMessage {
        to_session_id: SessionId(Arc::from(to)),
        response_required: false,
        body: format!("to {to}"),
        as_session: None,
        idempotency_key: None,
    }
    .execute(cmd_ctx(ctx, "c-sender"))
    .map_err(|e| e.message)
}

fn message_count(ctx: &MykoServerContext) -> usize {
    ctx.registry
        .get(Message::ENTITY_NAME_STATIC)
        .map(|store| store.entries().materialize().get().len())
        .unwrap_or(0)
}

fn outbox(ctx: &MykoServerContext) -> Vec<Arc<MessageLink>> {
    cmd_ctx(ctx, "c-sender")
        .exec_query(GetAllMessageLinks {})
        .unwrap_or_default()
}

#[test]
fn nickname_at_link_reaches_the_remote_session_through_the_outbox() {
    let ctx = setup();
    set(&ctx, &session("sender", "c-sender"));
    set(&ctx, &remote("far-1", "fuzzy-otter", "murph"));

    let result = send(&ctx, "fuzzy-otter@murph").expect("sent");
    assert_eq!(result.via_link.as_deref(), Some("murph"));
    assert_eq!(result.to_session_id.0.as_ref(), "far-1");
    let queued = outbox(&ctx);
    assert_eq!(queued.len(), 1);
    assert!(queued[0].outbound && queued[0].link == "murph");
    assert_eq!(queued[0].id.0.as_ref(), result.message_id.0.as_ref());
}

#[test]
fn a_bare_name_prefers_a_local_session() {
    let ctx = setup();
    set(&ctx, &session("sender", "c-sender"));
    set(&ctx, &session("local-1", "c-local"));
    // A remote session whose session id equals the local one's.
    set(&ctx, &remote("local-1", "fuzzy-otter", "murph"));

    let result = send(&ctx, "local-1").expect("sent");
    assert_eq!(result.via_link, None, "a bare id goes to the local session");
    assert!(outbox(&ctx).is_empty());

    // Only the explicit form reaches the remote one.
    let result = send(&ctx, "local-1@murph").expect("sent");
    assert_eq!(result.via_link.as_deref(), Some("murph"));
}

#[test]
fn a_bare_name_falls_back_to_a_remote_session() {
    let ctx = setup();
    set(&ctx, &session("sender", "c-sender"));
    set(&ctx, &remote("far-1", "fuzzy-otter", "murph"));
    let result = send(&ctx, "fuzzy-otter").expect("sent");
    assert_eq!(result.via_link.as_deref(), Some("murph"));
}

#[test]
fn a_name_on_two_links_is_ambiguous() {
    let ctx = setup();
    set(&ctx, &session("sender", "c-sender"));
    set(&ctx, &remote("far-1", "fuzzy-otter", "murph"));
    set(&ctx, &remote("far-2", "fuzzy-otter", "b2"));
    let err = send(&ctx, "fuzzy-otter").unwrap_err();
    assert!(
        err.contains("fuzzy-otter@murph") && err.contains("fuzzy-otter@b2"),
        "{err}"
    );
    assert!(send(&ctx, "fuzzy-otter@b2").is_ok());
}

#[test]
fn an_unknown_name_on_a_link_says_so() {
    let ctx = setup();
    set(&ctx, &session("sender", "c-sender"));
    set(&ctx, &remote("far-1", "fuzzy-otter", "murph"));
    let err = send(&ctx, "nobody@murph").unwrap_err();
    assert!(
        err.contains("no session 'nobody' on the link murph"),
        "{err}"
    );
    // An operator's email isn't mistaken for a link address.
    let err = send(&ctx, "operator@example.com").unwrap_err();
    assert!(err.contains("no session matches"), "{err}");
    assert_eq!(message_count(&ctx), 0);
}
