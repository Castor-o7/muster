//! MUS-7: the hook listener and the local gate refuse a browser and a local
//! process without the local token, and admit a local process with it.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener as StdListener, TcpStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use myko::server::Persister;
use myko_server::{BlackholePersister, MykoServer};

const TOKEN: &str = "local-test-token";

fn wait_listening(addr: SocketAddr) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if TcpStream::connect(addr).is_ok() {
            return;
        }
        thread::sleep(Duration::from_millis(25));
    }
    panic!("nothing listening on {addr}");
}

/// Send a raw request and return the status line.
fn status_of(addr: SocketAddr, request: &str) -> String {
    let mut s = TcpStream::connect(addr).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    s.write_all(request.as_bytes()).unwrap();
    let mut buf = Vec::new();
    let _ = s.read_to_end(&mut buf);
    let text = String::from_utf8_lossy(&buf);
    text.lines().next().unwrap_or("").to_string()
}

fn post(target: &str, extra_headers: &str) -> String {
    let body = r#"{"session_id":"local-access-test"}"#;
    format!(
        "POST {target} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\n{extra_headers}Connection: close\r\n\r\n{body}",
        body.len()
    )
}

fn start_hook_listener() -> SocketAddr {
    muster_entities::link();
    daemon::link();
    let blackhole: Arc<dyn Persister> = Arc::new(BlackholePersister);
    let server = MykoServer::builder()
        .with_default_persister(blackhole)
        .build();
    let ctx = server.ctx();
    let _server: &'static MykoServer = Box::leak(Box::new(server));
    // Bound here and handed to the listener, so no other process can take
    // the port between picking and binding it (MUS-88).
    let std_listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind hook listener");
    std_listener.set_nonblocking(true).unwrap();
    let addr: SocketAddr = std_listener.local_addr().unwrap();
    let state_dir = std::env::temp_dir().join(format!("muster-local-access-{}", addr.port()));
    std::fs::create_dir_all(&state_dir).unwrap();
    let access = Arc::new(daemon::http_listener::Access {
        state_dir,
        local_token: TOKEN.to_string(),
    });
    thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async move {
            let listener = tokio::net::TcpListener::from_std(std_listener).expect("hook listener");
            let (_running, stopping) = tokio::sync::watch::channel(false);
            let _ =
                daemon::http_listener::serve_until(listener, Arc::new(ctx), Some(access), stopping)
                    .await;
        });
    });
    wait_listening(addr);
    addr
}

#[test]
fn hook_listener_refuses_browsers_and_admits_the_local_token() {
    let addr = start_hook_listener();
    // A page's fetch or form post names its origin.
    assert!(
        status_of(
            addr,
            &post("/hook/prompt-submit", "Origin: https://evil.example\r\n")
        )
        .contains("403")
    );
    // A page's WebSocket is an upgrade, with or without an Origin.
    let ws = "GET /hook/wait HTTP/1.1\r\nHost: 127.0.0.1\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
              Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n";
    assert!(status_of(addr, ws).contains("403"));
    // A local process without the token can't act, with or without a session.
    assert!(status_of(addr, &post("/hook/prompt-submit", "")).contains("401"));
    assert!(
        status_of(
            addr,
            &post("/hook/prompt-submit", "Authorization: Bearer wrong\r\n")
        )
        .contains("401")
    );
    // With it, the request reaches the handlers: a route that doesn't exist
    // answers 404, which only the dispatcher says.
    assert!(
        status_of(
            addr,
            &post(
                "/hook/no-such-route",
                &format!("Authorization: Bearer {TOKEN}\r\n")
            )
        )
        .contains("404")
    );
    assert!(
        status_of(
            addr,
            &post(&format!("/hook/no-such-route?token={TOKEN}"), "")
        )
        .contains("404")
    );
}

fn start_local_gate() -> SocketAddr {
    // A stand-in for myko: answers any upgrade with 101.
    let upstream = StdListener::bind("127.0.0.1:0").unwrap();
    let upstream_addr = upstream.local_addr().unwrap();
    thread::spawn(move || {
        for conn in upstream.incoming().flatten() {
            let mut conn = conn;
            let mut buf = [0u8; 4096];
            let _ = conn.read(&mut buf);
            let _ = conn.write_all(b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\r\n");
            thread::sleep(Duration::from_millis(200));
        }
    });
    // Bound here and handed to the gate (MUS-88).
    let std_listener = StdListener::bind("127.0.0.1:0").unwrap();
    std_listener.set_nonblocking(true).unwrap();
    let addr: SocketAddr = std_listener.local_addr().unwrap();
    let state_dir: PathBuf =
        std::env::temp_dir().join(format!("muster-local-gate-{}", addr.port()));
    std::fs::create_dir_all(&state_dir).unwrap();
    thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async move {
            let listener = tokio::net::TcpListener::from_std(std_listener).unwrap();
            let _ = daemon::gate::serve_local(
                listener,
                daemon::gate::LocalGateConfig {
                    bind: addr,
                    myko: upstream_addr,
                    state_dir,
                    local_token: TOKEN.to_string(),
                },
            )
            .await;
        });
    });
    wait_listening(addr);
    addr
}

fn upgrade(target: &str, extra_headers: &str) -> String {
    format!(
        "GET {target} HTTP/1.1\r\nHost: 127.0.0.1\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
         Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n{extra_headers}\r\n"
    )
}

#[test]
fn local_gate_refuses_browsers_and_admits_the_local_token() {
    let addr = start_local_gate();
    // A page's WebSocket carries its origin, token or not.
    assert!(
        status_of(
            addr,
            &upgrade(
                &format!("/myko?token={TOKEN}"),
                "Origin: https://evil.example\r\n"
            )
        )
        .contains("403")
    );
    // A local process without the token is turned away.
    assert!(status_of(addr, &upgrade("/myko", "")).contains("401"));
    assert!(status_of(addr, &upgrade("/myko?token=wrong", "")).contains("401"));
    // With it, only myko's WebSocket route is open.
    assert!(
        status_of(
            addr,
            &format!("GET /myko/mcp?token={TOKEN} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
        )
        .contains("403")
    );
    assert!(status_of(addr, &upgrade(&format!("/myko?token={TOKEN}"), "")).contains("101"));
}
