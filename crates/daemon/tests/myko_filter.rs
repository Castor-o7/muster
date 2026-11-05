//! Myko's own listener admits only requests a gate forwarded, which
//! carry the local token, and refuses any browser origin, on every route.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener as StdListener, TcpStream};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use myko::server::Persister;
use myko_server::{BlackholePersister, MykoServer};

const TOKEN: &str = "myko-filter-test-token";

fn start() -> SocketAddr {
    marshal_entities::link();
    daemon::link();
    let port = StdListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let addr: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
    thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async move {
            let blackhole: Arc<dyn Persister> = Arc::new(BlackholePersister);
            let server = MykoServer::builder()
                .with_bind_addr(addr)
                .with_default_persister(blackhole)
                .with_request_filter(daemon::gate::myko_request_filter(TOKEN.to_string()))
                .build();
            let _ = server.run().await;
        });
    });
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if TcpStream::connect(addr).is_ok() {
            return addr;
        }
        thread::sleep(Duration::from_millis(50));
    }
    panic!("myko never listened on {addr}");
}

fn status_of(addr: SocketAddr, request: &str) -> String {
    let mut s = TcpStream::connect(addr).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    s.write_all(request.as_bytes()).unwrap();
    let mut buf = [0u8; 256];
    let n = s.read(&mut buf).unwrap_or(0);
    String::from_utf8_lossy(&buf[..n])
        .lines()
        .next()
        .unwrap_or("")
        .to_string()
}

fn upgrade(extra: &str) -> String {
    format!(
        "GET /myko HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: Upgrade\r\nUpgrade: websocket\r\n\
         Sec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n{extra}\r\n"
    )
}

fn mcp(extra: &str) -> String {
    let body = r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#;
    format!(
        "POST /myko/mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\n{extra}Connection: close\r\n\r\n{body}",
        body.len()
    )
}

#[test]
fn myko_admits_only_gate_forwarded_requests() {
    let addr = start();
    let bearer = format!("Authorization: Bearer {TOKEN}\r\n");
    let origin = "Origin: https://evil.example\r\n";

    // A web page: refused on the WebSocket and on MCP, even holding the token.
    assert!(status_of(addr, &upgrade(origin)).contains("403"));
    assert!(status_of(addr, &upgrade(&format!("{origin}{bearer}"))).contains("403"));
    assert!(status_of(addr, &mcp(origin)).contains("403"));

    // A local process dialing myko directly, without the token: refused.
    assert!(status_of(addr, &upgrade("")).contains("401"));
    assert!(status_of(addr, &upgrade("Authorization: Bearer wrong\r\n")).contains("401"));
    assert!(status_of(addr, &mcp("")).contains("401"));

    // What a gate forwards: admitted.
    assert!(status_of(addr, &upgrade(&bearer)).contains("101"));
}
