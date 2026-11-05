//! Integration test (MUS-83): a shim whose local token the daemon refuses says
//! so, names the token file, and doesn't wait out the command timeout.
//!
//! Drives the REAL `muster-shim` over stdio MCP against a fake gate that
//! answers every WebSocket handshake `401 Unauthorized`, as the daemon's local
//! gate does for a missing or wrong token (`gate.rs`, "missing or wrong local
//! token").

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

const SHIM: &str = env!("CARGO_BIN_EXE_muster-shim");

/// A gate that refuses every handshake for its token. Returns its address.
fn refusing_gate() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            thread::spawn(move || {
                let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                let mut head = Vec::new();
                let mut byte = [0u8; 1];
                while !head.ends_with(b"\r\n\r\n") {
                    match stream.read(&mut byte) {
                        Ok(1) => head.push(byte[0]),
                        _ => return,
                    }
                }
                let _ = stream.write_all(
                    b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                );
            });
        }
    });
    format!("ws://{address}")
}

struct Shim {
    child: Child,
    lines: mpsc::Receiver<String>,
}

impl Shim {
    fn start(gate: &str, home: &std::path::Path) -> Shim {
        let mut child = Command::new(SHIM)
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("HOME", home)
            .env("XDG_CONFIG_HOME", home.join("xdg"))
            .env("MUSTER_DAEMON_ADDRESS", gate)
            .env(
                "CLAUDE_CODE_SESSION_ID",
                "e5000000-0000-4000-8000-000000000083",
            )
            .env("RUST_LOG", "warn")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn muster-shim");
        let stdout = child.stdout.take().unwrap();
        let (tx, lines) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        Shim { child, lines }
    }

    fn send(&mut self, id: u64, method: &str, params: serde_json::Value) {
        let frame =
            serde_json::json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        let stdin = self.child.stdin.as_mut().unwrap();
        writeln!(stdin, "{frame}").unwrap();
        stdin.flush().unwrap();
    }

    /// The reply to request `id`, as text, or `None` after `limit`.
    fn reply(&self, id: u64, limit: Duration) -> Option<String> {
        let deadline = Instant::now() + limit;
        while let Some(left) = deadline.checked_duration_since(Instant::now()) {
            let line = self.lines.recv_timeout(left).ok()?;
            let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
                continue;
            };
            if value["id"] == id {
                return Some(line);
            }
        }
        None
    }
}

impl Drop for Shim {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Start a shim against a refusing gate, make its first write, and return the
/// reply text and how long it took.
fn first_write_against_refusing_gate(home: &std::path::Path) -> (String, Duration) {
    let gate = refusing_gate();
    let mut shim = Shim::start(&gate, home);
    shim.send(
        1,
        "initialize",
        serde_json::json!({"protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": {"name": "itest", "version": "0"}}),
    );
    shim.reply(1, Duration::from_secs(10))
        .expect("initialize reply");
    let started = Instant::now();
    shim.send(
        2,
        "tools/call",
        serde_json::json!({"name": "send_message", "arguments": {"to": "anyone", "body": "hello"}}),
    );
    let reply = shim
        .reply(2, Duration::from_secs(30))
        .expect("send_message reply");
    (reply, started.elapsed())
}

#[test]
fn a_wrong_local_token_is_reported_with_its_file() {
    let home = tempfile::tempdir().unwrap();
    let token = home.path().join("xdg").join("muster").join("local-token");
    std::fs::create_dir_all(token.parent().unwrap()).unwrap();
    std::fs::write(&token, "not-the-daemons-token\n").unwrap();

    let (reply, took) = first_write_against_refusing_gate(home.path());

    assert!(
        reply.contains("refused this shim's local token (401)"),
        "{reply}"
    );
    assert!(
        reply.contains(&token.display().to_string()),
        "names the file: {reply}"
    );
    assert!(
        reply.contains("doesn't match the running daemon's"),
        "{reply}"
    );
    assert!(!reply.contains("daemon unresponsive"), "{reply}");
    assert!(
        !reply.contains("not-the-daemons-token"),
        "never shows the token: {reply}"
    );
    // Before MUS-83 this waited out the registration hold and the command
    // timeout (about 15 s) and then blamed an unresponsive daemon.
    assert!(took < Duration::from_secs(5), "answered in {took:?}");
}

#[test]
fn a_missing_local_token_is_reported_with_its_file() {
    let home = tempfile::tempdir().unwrap();
    let token = home.path().join("xdg").join("muster").join("local-token");

    let (reply, took) = first_write_against_refusing_gate(home.path());

    assert!(
        reply.contains("refused this shim's local token (401)"),
        "{reply}"
    );
    assert!(
        reply.contains(&token.display().to_string()),
        "names the file: {reply}"
    );
    assert!(reply.contains("is missing"), "{reply}");
    assert!(took < Duration::from_secs(5), "answered in {took:?}");
}
