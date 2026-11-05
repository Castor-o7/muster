//! MUS-10: while the operator has paused wake-ups, a held wait hands out
//! nothing and the mail waits for the next prompt; with a sender allowlist,
//! only listed machines may send. These share the process-wide controls, so
//! they run as one test.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use muster_entities::{HostInfo, SendMessage, Session, SessionId};
use myko::{
    command::{CommandContext, CommandHandler},
    entities::client::ClientId,
    request::RequestContext,
    server::{MykoServerContext, Persister},
    wire::{MEvent, MEventType},
};
use myko_server::{BlackholePersister, MykoServer};
use uuid::Uuid;

const RECIPIENT: &str = r#"{"session_id":"recipient","cwd":"/work/repo"}"#;

fn sender() -> Session {
    Session {
        id: SessionId(Arc::from("sender")),
        client_id: Some(ClientId(Arc::from("c-sender"))),
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

fn cmd_ctx(ctx: &MykoServerContext) -> CommandContext {
    let req = RequestContext::new(
        Arc::<str>::from(Uuid::new_v4().to_string().as_str()),
        Some(Arc::<str>::from("c-sender")),
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

fn send(ctx: &MykoServerContext, body: &str) {
    SendMessage {
        response_required: false,
        to_session_id: SessionId(Arc::from("recipient")),
        body: body.into(),
        as_session: None,
        idempotency_key: None,
    }
    .execute(cmd_ctx(ctx))
    .expect("send to recipient");
}

fn request(target: &str, body: &str) -> String {
    format!(
        "POST {target} HTTP/1.1\r\nHost: x\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

fn http_post(addr: SocketAddr, target: &str, body: &str) -> String {
    let mut s = TcpStream::connect(addr).expect("connect hook listener");
    s.write_all(request(target, body).as_bytes())
        .expect("write request");
    let mut resp = String::new();
    s.read_to_string(&mut resp).expect("read response");
    resp.split_once("\r\n\r\n")
        .map(|(_, b)| b)
        .unwrap_or("")
        .to_string()
}

/// Bring up a daemon context with the real hook listener and a seeded sender.
fn start() -> (MykoServerContext, SocketAddr) {
    let (ctx, addr, stop) = start_stoppable();
    // Never stopped: leak the sender so the listener keeps running.
    Box::leak(Box::new(stop));
    (ctx, addr)
}

fn start_stoppable() -> (
    MykoServerContext,
    SocketAddr,
    tokio::sync::watch::Sender<bool>,
) {
    muster_entities::link();
    daemon::link();
    let blackhole: Arc<dyn Persister> = Arc::new(BlackholePersister);
    let server = MykoServer::builder()
        .with_default_persister(blackhole)
        .build();
    let ctx = server.ctx();
    Box::leak(Box::new(server));
    let event = MEvent::from_item(&sender(), MEventType::SET, &Uuid::new_v4().to_string());
    ctx.apply_event_batch(vec![event]).expect("seed sender");

    // Bound here and handed to the listener, so no other process can take
    // the port between picking and binding it (MUS-88).
    let std_listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind hook listener");
    std_listener.set_nonblocking(true).unwrap();
    let addr: SocketAddr = std_listener.local_addr().unwrap();
    let listener_ctx = ctx.clone();
    let (stop, stop_rx) = tokio::sync::watch::channel(false);
    thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("rt");
        rt.block_on(async move {
            let listener = tokio::net::TcpListener::from_std(std_listener).expect("hook listener");
            let _ =
                daemon::http_listener::serve_until(listener, Arc::new(listener_ctx), None, stop_rx)
                    .await;
        });
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    while TcpStream::connect(addr).is_err() {
        assert!(Instant::now() < deadline, "hook listener never came up");
        thread::sleep(Duration::from_millis(25));
    }
    // Register the recipient so sends to it are routable.
    http_post(addr, "/hook/session-register?harness=claude", RECIPIENT);
    (ctx, addr, stop)
}

#[test]
fn pausing_holds_wakes_and_the_allowlist_refuses_unlisted_machines() {
    let (ctx, addr) = start();

    // Paused: a wait sees the message land but hands out nothing.
    muster_entities::controls::set_wakes_paused(true);
    send(&ctx, "PAUSE-E2E-held");
    let started = Instant::now();
    let body = http_post(addr, "/hook/wait?harness=claude&timeout=2", RECIPIENT);
    assert!(body.is_empty(), "a paused wait handed out mail: {body:?}");
    assert!(
        started.elapsed() >= Duration::from_secs(2),
        "it should hold to its timeout"
    );

    // Resumed: the next wait gets the mail that arrived while paused.
    muster_entities::controls::set_wakes_paused(false);
    let body = http_post(addr, "/hook/wait?harness=claude&timeout=5", RECIPIENT);
    assert!(
        body.contains("PAUSE-E2E-held"),
        "after resume the mail should arrive: {body:?}"
    );

    // An allowlist that doesn't name the sender's machine refuses its sends.
    let mut on_a1 = sender();
    on_a1.host = Some(HostInfo {
        name: "a1".into(),
        os: "macos".into(),
        arch: "aarch64".into(),
    });
    let event = MEvent::from_item(&on_a1, MEventType::SET, &Uuid::new_v4().to_string());
    ctx.apply_event_batch(vec![event])
        .expect("move the sender to a1");
    muster_entities::controls::set_allowed_senders(Some(vec!["murph".into()]));
    let refused = try_send(&ctx, "ALLOW-E2E-refused").expect_err("a1 isn't listed");
    assert!(
        format!("{refused:?}").contains("allows messages only from murph"),
        "{refused:?}"
    );
    muster_entities::controls::set_allowed_senders(Some(vec!["murph".into(), "A1".into()]));
    try_send(&ctx, "ALLOW-E2E-listed").expect("a listed machine sends, in any case");
    muster_entities::controls::set_allowed_senders(None);
    try_send(&ctx, "ALLOW-E2E-open").expect("with no list, any machine sends");
}

fn try_send(
    ctx: &MykoServerContext,
    body: &str,
) -> Result<muster_entities::SendMessageResult, myko::command::CommandError> {
    SendMessage {
        response_required: false,
        to_session_id: SessionId(Arc::from("recipient")),
        body: body.into(),
        as_session: None,
        idempotency_key: None,
    }
    .execute(cmd_ctx(ctx))
}
