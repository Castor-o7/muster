//! MUS-9 and MUS-60: a session that floods SendMessage is throttled, a machine
//! can't dodge that by spreading a flood across new sessions, and other
//! sessions and machines keep sending.

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

#[allow(dead_code)]
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
fn a_flooding_session_is_throttled_and_others_keep_sending() {
    let ctx = setup();
    set_session(&ctx, &session("rate-flood", Some("c-rate-flood")));
    set_session(&ctx, &session("rate-calm", Some("c-rate-calm")));
    set_session(&ctx, &session("rate-recipient", None));
    let send = |from: &str, n: usize| {
        SendMessage {
            response_required: false,
            to_session_id: SessionId(Arc::from("rate-recipient")),
            body: format!("message {n}"),
            as_session: None,
            idempotency_key: None,
        }
        .execute(cmd_ctx(&ctx, Some(from)))
    };
    let burst = muster_entities::rate::BURST as usize;
    for n in 0..burst {
        send("c-rate-flood", n).expect("within the burst");
    }
    let refused = send("c-rate-flood", burst).expect_err("past the burst");
    assert!(
        format!("{refused:?}").contains("too many messages"),
        "{refused:?}"
    );
    send("c-rate-calm", 0).expect("another session keeps sending");
}

fn on_host(mut s: Session, host: &str) -> Session {
    s.host = Some(muster_entities::HostInfo {
        name: host.into(),
        os: "linux".into(),
        arch: "x86_64".into(),
    });
    s
}

#[test]
fn a_flood_spread_across_new_sessions_is_throttled_per_machine() {
    let ctx = setup();
    set_session(&ctx, &session("spread-recipient", None));
    set_session(
        &ctx,
        &on_host(
            session("spread-calm", Some("c-spread-calm")),
            "spread-calm-host",
        ),
    );
    let send = |client: &str| {
        SendMessage {
            response_required: false,
            to_session_id: SessionId(Arc::from("spread-recipient")),
            body: "hi".into(),
            as_session: None,
            idempotency_key: None,
        }
        .execute(cmd_ctx(&ctx, Some(client)))
    };
    let per_session = muster_entities::rate::BURST as usize / 2;
    let machine = muster_entities::rate::MACHINE_BURST as usize;
    let mut sent = 0;
    let mut n = 0;
    let refused = loop {
        let (id, client) = (format!("spread-{n}"), format!("c-spread-{n}"));
        set_session(&ctx, &on_host(session(&id, Some(&client)), "spread-host"));
        let mut refused = None;
        for _ in 0..per_session {
            match send(&client) {
                Ok(_) => sent += 1,
                Err(e) => {
                    refused = Some(e);
                    break;
                }
            }
        }
        if let Some(e) = refused {
            break e;
        }
        n += 1;
        assert!(sent <= machine, "the machine total didn't hold");
    };
    assert_eq!(sent, machine, "every session stayed under its own limit");
    assert!(
        format!("{refused:?}").contains("this machine"),
        "{refused:?}"
    );
    send("c-spread-calm").expect("another machine keeps sending");
}
