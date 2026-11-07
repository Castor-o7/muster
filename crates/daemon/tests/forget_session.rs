//! MUS-105: `muster-daemon forget` removes a session's roster row, and holds its
//! hooks off so a stale waiter can't put the row straight back.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use hyphae::{Gettable, Materialize};
use muster_entities::{Session, SessionId, SessionNickname, SessionNicknameId};
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

    a_forget_by_nickname_holds_the_old_id_and_spares_a_newcomer(&ctx, addr);
}

fn nickname_of(ctx: &MykoServerContext, id: &str) -> Option<String> {
    ctx.registry
        .get(SessionNickname::ENTITY_NAME_STATIC)?
        .entries()
        .materialize()
        .get()
        .into_iter()
        .find(|(k, _)| k.as_ref() == id)
        .and_then(|(_, item)| {
            myko::utils::downcast_item::<SessionNickname>(&item).map(|n| n.nickname.clone())
        })
}

/// Run by the test above: the hold list is process-wide, so the cases share one
/// test rather than race. Forgetting by nickname resolves once: after the row is gone, a new session
/// given the freed nickname survives, and the old id's hooks stay refused.
fn a_forget_by_nickname_holds_the_old_id_and_spares_a_newcomer(
    ctx: &MykoServerContext,
    addr: SocketAddr,
) {
    let ghost = "ghost-0000-0000-0000-000000000000";
    let ghost_body = format!(r#"{{"session_id":"{ghost}","cwd":"/work/repo"}}"#);
    http_post(addr, "/hook/session-register?harness=claude", &ghost_body);
    let deadline = Instant::now() + Duration::from_secs(5);
    let nick = loop {
        if let Some(n) = nickname_of(ctx, ghost) {
            break n;
        }
        assert!(Instant::now() < deadline, "the ghost never got a nickname");
        thread::sleep(Duration::from_millis(25));
    };

    let dir = tempfile::tempdir().unwrap();
    daemon::controls::forget(dir.path(), &nick).unwrap();
    daemon::controls::apply(dir.path(), ctx, &mut None, false);
    assert!(
        !ids(ctx).contains(&ghost.to_string()),
        "the ghost's row is gone"
    );

    // A new session takes the freed nickname.
    let mut newcomer = sender();
    newcomer.id = SessionId(Arc::from("newcomer"));
    newcomer.client_id = None;
    let named = SessionNickname {
        id: SessionNicknameId(Arc::from("newcomer")),
        nickname: nick.clone(),
    };
    ctx.apply_event_batch(vec![
        MEvent::from_item(&newcomer, MEventType::SET, &Uuid::new_v4().to_string()),
        MEvent::from_item(&named, MEventType::SET, &Uuid::new_v4().to_string()),
    ])
    .expect("seed the newcomer");

    daemon::controls::apply(dir.path(), ctx, &mut None, false);
    assert!(
        ids(ctx).contains(&"newcomer".to_string()),
        "the newcomer survives"
    );
    assert!(
        daemon::controls::is_forgotten(ghost),
        "the old id is still held"
    );
    http_post(addr, "/hook/prompt-submit?harness=claude", &ghost_body);
    assert!(
        !ids(ctx).contains(&ghost.to_string()),
        "its hook can't bring it back"
    );
}
