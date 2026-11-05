//! MUS-50: a daemon write built from a stale copy of a session's row drops the
//! client binding the shim had just registered. These drive the effect
//! directly, with a real WebSocket client: the window it needs is inside the
//! daemon, between the hook's read of the roster and its write, so an HTTP
//! hook can't be made to hit it on cue.

use std::{
    net::SocketAddr,
    sync::Arc,
    time::{Duration, Instant},
};

use hyphae::Gettable;
use muster_entities::{GetAllSessions, Session, SessionId};
use myko::{
    client::{ConnectionStatus, MykoClient, MykoProtocol},
    entities::client::ClientId,
    server::{MykoServerContext, Persister},
    wire::{MEvent, MEventType},
};
use myko_server::{BlackholePersister, MykoServer};
use uuid::Uuid;

/// A running server: sagas run only under `MykoServer::run`.
fn setup() -> (MykoServerContext, String) {
    muster_entities::link();
    daemon::link();
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let bind: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
    let (tx, rx) = std::sync::mpsc::sync_channel::<MykoServerContext>(1);
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("runtime");
        rt.block_on(async move {
            let blackhole: Arc<dyn Persister> = Arc::new(BlackholePersister);
            let server = Arc::new(
                MykoServer::builder()
                    .with_bind_addr(bind)
                    .with_default_persister(blackhole)
                    .build(),
            );
            tx.send(server.ctx()).expect("ctx");
            let _ = server.run().await;
        });
    });
    let ctx = rx.recv_timeout(Duration::from_secs(5)).expect("server up");
    (ctx, format!("ws://{bind}"))
}

fn connect(addr: &str) -> MykoClient {
    let client = MykoClient::new();
    client.set_protocol(MykoProtocol::JSON);
    client.set_address(Some(addr.to_string()));
    let status = client.connection_status();
    assert!(
        eventually(Duration::from_secs(8), || matches!(
            status.get(),
            ConnectionStatus::Connected(_)
        )),
        "client connects"
    );
    client
}

fn row(id: &str) -> Session {
    Session {
        id: SessionId(Arc::from(id)),
        client_id: None,
        pid: 1,
        cwd: "/repo".into(),
        git_branch: None,
        current_task: None,
        session_name: None,
        activity: None,
        kind: None,
        connected_at: 0,
        last_activity_at: Some(1),
        last_tool: None,
        last_tool_at: None,
        operator: None,
        host: None,
        project: None,
        channels_enabled: None,
        acks_pushes: None,
        harness: Some("claude".into()),
        identity: None,
    }
}

fn client_of(ctx: &MykoServerContext, id: &str) -> Option<ClientId> {
    daemon::succession::internal_ctx(ctx)
        .exec_query(GetAllSessions {})
        .unwrap()
        .iter()
        .find(|s| s.id.0.as_ref() == id)
        .and_then(|s| s.client_id.clone())
}

fn eventually(within: Duration, mut f: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + within;
    while Instant::now() < deadline {
        if f() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    f()
}

/// The hook's write: the whole row, from a copy read before the shim's
/// registration landed, so with no client.
fn stale_daemon_write(ctx: &MykoServerContext, id: &str) {
    stale_daemon_write_with(ctx, id, None);
}

/// The same, from a copy that still names an older connection.
fn stale_daemon_write_with(ctx: &MykoServerContext, id: &str, client: Option<ClientId>) {
    let mut r = row(id);
    r.client_id = client;
    let ev = MEvent::from_item(&r, MEventType::SET, &Uuid::new_v4().to_string());
    ctx.apply_event_batch(vec![ev]).expect("apply SET");
}

fn register(shim: &MykoClient, ctx: &MykoServerContext, sid: &str) -> ClientId {
    let before = client_of(ctx, sid);
    shim.send_event(MEvent::from_item(&row(sid), MEventType::SET, "shim"))
        .expect("send");
    assert!(
        eventually(Duration::from_secs(5), || {
            let now = client_of(ctx, sid);
            now.is_some() && now != before
        }),
        "the registration binds the row"
    );
    client_of(ctx, sid).unwrap()
}

#[test]
fn a_stale_daemon_write_doesnt_unbind_a_live_session() {
    let (ctx, addr) = setup();
    let shim = connect(&addr);
    let sid = Uuid::new_v4().to_string();
    // The shim registers over its WebSocket: myko binds the row to it.
    shim.send_event(MEvent::from_item(&row(&sid), MEventType::SET, "shim"))
        .expect("send");
    assert!(
        eventually(Duration::from_secs(5), || client_of(&ctx, &sid).is_some()),
        "the shim's registration binds the row"
    );
    let bound = client_of(&ctx, &sid);
    stale_daemon_write(&ctx, &sid);
    assert!(
        eventually(Duration::from_secs(5), || client_of(&ctx, &sid) == bound),
        "the binding to the still-connected shim is put back; got {:?}",
        client_of(&ctx, &sid)
    );
}

/// The repair's own SET triggers the saga again, which must do nothing: one
/// repair write, not a loop.
#[test]
fn a_repair_is_written_once() {
    let (ctx, addr) = setup();
    let shim = connect(&addr);
    let sid = Uuid::new_v4().to_string();
    let bound = register(&shim, &ctx, &sid);
    stale_daemon_write(&ctx, &sid);
    assert!(eventually(Duration::from_secs(5), || client_of(&ctx, &sid)
        .as_ref()
        == Some(&bound)));
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(daemon::binding_repair::repairs_for(&sid), 1);
    assert_eq!(client_of(&ctx, &sid).as_ref(), Some(&bound));
}

/// A shim that reconnects gets a new client id, while its old connection may
/// linger in the Client store for a while. A stale write, naming no client or
/// the old one, is bound to the newest connection, never back to the old.
#[test]
fn a_moved_session_follows_its_newest_connection() {
    let (ctx, addr) = setup();
    let old_shim = connect(&addr);
    let new_shim = connect(&addr);
    let sid = Uuid::new_v4().to_string();
    let old = register(&old_shim, &ctx, &sid);
    let new = register(&new_shim, &ctx, &sid);
    assert_ne!(old, new);
    for stale in [None, Some(old.clone())] {
        stale_daemon_write_with(&ctx, &sid, stale.clone());
        assert!(
            eventually(Duration::from_secs(5), || client_of(&ctx, &sid).as_ref()
                == Some(&new)),
            "a stale write naming {stale:?} is bound to the newest connection; got {:?}",
            client_of(&ctx, &sid)
        );
    }
    std::thread::sleep(Duration::from_millis(1000));
    assert_eq!(
        client_of(&ctx, &sid).as_ref(),
        Some(&new),
        "and it stays there"
    );
    drop(old_shim);
}

#[test]
fn a_session_whose_client_has_gone_stays_unbound() {
    let (ctx, addr) = setup();
    let shim = connect(&addr);
    let sid = Uuid::new_v4().to_string();
    shim.send_event(MEvent::from_item(&row(&sid), MEventType::SET, "shim"))
        .expect("send");
    assert!(eventually(Duration::from_secs(5), || client_of(&ctx, &sid).is_some()));
    // The shim goes away, and then the row is written without a client, as
    // when the sweeper demotes it to a pull row (MUS-54).
    shim.set_address(None);
    drop(shim);
    std::thread::sleep(Duration::from_millis(1500));
    stale_daemon_write(&ctx, &sid);
    std::thread::sleep(Duration::from_millis(1000));
    assert_eq!(
        client_of(&ctx, &sid),
        None,
        "an unbinding after the client left stands"
    );
}
