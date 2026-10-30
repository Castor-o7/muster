//! End-to-end test of `/hook/wait`, the long-poll `muster-shim watch` holds
//! open under Claude Code's Monitor tool so an idle session hears a direct
//! message without the live-channel flag.
//!
//! The contract: a message sent while the wait is held is returned promptly
//! and acked; a waiter that went away never has a message acked on its
//! behalf; and an idle wait ends empty at its timeout.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use muster_entities::{SendMessage, Session, SessionId};
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

fn pick_free_port() -> u16 {
    let l = std::net::TcpListener::bind("127.0.0.1:0").expect("pick free port");
    let p = l.local_addr().unwrap().port();
    drop(l);
    p
}

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
        to_session_id: SessionId(Arc::from("recipient")),
        body: body.into(),
        as_session: None,
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

    let addr: SocketAddr = format!("127.0.0.1:{}", pick_free_port()).parse().unwrap();
    let listener_ctx = ctx.clone();
    thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("rt");
        rt.block_on(async move {
            let _ = daemon::http_listener::run(addr, Arc::new(listener_ctx)).await;
        });
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    while TcpStream::connect(addr).is_err() {
        assert!(Instant::now() < deadline, "hook listener never came up");
        thread::sleep(Duration::from_millis(25));
    }
    // Register the recipient so sends to it are routable.
    http_post(addr, "/hook/session-register?harness=claude", RECIPIENT);
    (ctx, addr)
}

#[test]
fn held_wait_returns_a_message_sent_during_it_and_acks_it() {
    let (ctx, addr) = start();

    let waiter = thread::spawn(move || {
        let started = Instant::now();
        let body = http_post(addr, "/hook/wait?harness=claude&timeout=30", RECIPIENT);
        (body, started.elapsed())
    });
    thread::sleep(Duration::from_millis(300));
    send(&ctx, "WAIT-E2E-live-marker");

    let (body, elapsed) = waiter.join().expect("waiter thread");
    assert!(
        body.contains("WAIT-E2E-live-marker") && body.contains("<muster_inbox"),
        "held wait did not return the message; got: {body:?}"
    );
    assert!(
        elapsed < Duration::from_secs(5),
        "wait returned only after {elapsed:?}; it must return when the message lands"
    );

    let again = http_post(addr, "/hook/prompt-submit", RECIPIENT);
    assert!(
        !again.contains("WAIT-E2E-live-marker"),
        "a message the wait delivered must be acked; got: {again:?}"
    );
}

#[test]
fn abandoned_wait_does_not_ack_a_later_message() {
    let (ctx, addr) = start();

    // A watcher killed mid-wait (the Monitor expiring) closes its socket.
    let mut s = TcpStream::connect(addr).expect("connect hook listener");
    s.write_all(request("/hook/wait?harness=claude&timeout=30", RECIPIENT).as_bytes())
        .expect("write request");
    thread::sleep(Duration::from_millis(300));
    drop(s);
    thread::sleep(Duration::from_millis(300));

    send(&ctx, "WAIT-E2E-orphan-marker");
    thread::sleep(Duration::from_millis(1500));

    let body = http_post(addr, "/hook/prompt-submit", RECIPIENT);
    assert!(
        body.contains("WAIT-E2E-orphan-marker"),
        "a wait whose caller left must not consume the message; got: {body:?}"
    );
}

#[test]
fn idle_wait_ends_empty_at_its_timeout() {
    let (_ctx, addr) = start();

    let started = Instant::now();
    let body = http_post(addr, "/hook/wait?harness=claude&timeout=1", RECIPIENT);
    assert_eq!(body, "", "an idle wait must end with no context");
    assert!(
        started.elapsed() >= Duration::from_millis(900),
        "wait ended early"
    );
}
