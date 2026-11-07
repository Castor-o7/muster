//! MUS-105: `muster-daemon forget` removes a session's roster row, and holds its
//! hooks off so a stale waiter can't put the row straight back.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use hyphae::{Gettable, Materialize};
use muster_entities::{Session, SessionId};
use myko::{
    core::item::Eventable,
    entities::client::ClientId,
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
        acks_pushes: None,
        harness: None,
        identity: None,
    }
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

    let addr: SocketAddr = format!("127.0.0.1:{}", pick_free_port()).parse().unwrap();
    let listener_ctx = ctx.clone();
    let (stop, stop_rx) = tokio::sync::watch::channel(false);
    thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("rt");
        rt.block_on(async move {
            let _ =
                daemon::http_listener::run_until(addr, Arc::new(listener_ctx), None, stop_rx).await;
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

fn ids(ctx: &MykoServerContext) -> Vec<String> {
    ctx.registry
        .get(Session::ENTITY_NAME_STATIC)
        .map(|s| {
            s.entries()
                .materialize()
                .get()
                .into_iter()
                .map(|(id, _)| id.to_string())
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn forgetting_removes_the_row_and_holds_its_hooks_off() {
    let (ctx, addr) = start();
    assert!(ids(&ctx).contains(&"recipient".to_string()), "registered");

    let dir = tempfile::tempdir().unwrap();
    daemon::controls::forget(dir.path(), "recipient").unwrap();
    daemon::controls::apply(dir.path(), &ctx, &mut None, false);
    assert!(
        !ids(&ctx).contains(&"recipient".to_string()),
        "the row is gone"
    );
    assert!(ids(&ctx).contains(&"sender".to_string()), "nothing else is");
    assert!(daemon::controls::is_forgotten("recipient"));

    // A stale waiter polling as it doesn't bring it back while the forget holds.
    http_post(addr, "/hook/session-register?harness=claude", RECIPIENT);
    http_post(addr, "/hook/prompt-submit?harness=claude", RECIPIENT);
    assert!(!ids(&ctx).contains(&"recipient".to_string()), "held off");
    assert!(!daemon::controls::is_forgotten("sender"));
}
