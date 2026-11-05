//! Delivery conformance (MUS-69): one scenario table, run against every
//! harness, with the real `muster-daemon` and `muster-shim` binaries and no TUI
//! or model.
//!
//! Each harness is a fake that drives the real binaries the way its TUI does
//! (`FakeHarness`). What each fake assumes about its TUI, and the harness
//! version it's pinned to, is in `docs/delivery-conformance.md`. Messages are
//! sent by a real `muster-shim` speaking MCP, so the send result is checked as
//! well as what the recipient sees.
//!
//! The suite needs the daemon binary, so it doesn't run in plain
//! `cargo test`. Build first, then run it:
//!
//! ```sh
//! cargo build --workspace
//! cargo test -p muster-shim --test conformance -- --ignored --nocapture
//! ```
//!
//! It prints a harness × scenario grid. A cell in `KNOWN_FAILURES` is expected
//! to fail and is reported as such; if one starts passing, the suite fails so
//! the list gets updated.

use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader, Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

use serde_json::{Value, json};
use tempfile::TempDir;

const SHIM: &str = env!("CARGO_BIN_EXE_muster-shim");
/// Short, so a dropped ack is redelivered within a test (default 60 s).
const ACK_LEASE_SECS: u64 = 2;
/// How long an idle session gets to be woken.
const WAKE_WITHIN: Duration = Duration::from_secs(10);
/// How long to watch for a wake that must not happen.
const QUIET_FOR: Duration = Duration::from_secs(4);

/// Cells expected to fail today, each with the story that fixes it. The
/// suite reports them as known failures, and fails if one passes.
const KNOWN_FAILURES: &[(&str, &str, &str)] = &[
    ("codex", "daemon-restart-mid-wait", "MUS-76"),
    ("pi", "resume-wakes-held-mail", "MUS-82"),
    ("opencode", "dropped-ack-redelivers", "MUS-81"),
    ("opencode", "resume-wakes-held-mail", "MUS-82"),
    ("opencode", "mail-waiting-at-reconnect", "MUS-81"),
    ("opencode", "hung-handover-released", "MUS-81"),
    ("prime-agent", "dropped-ack-redelivers", "MUS-81"),
    ("prime-agent", "resume-wakes-held-mail", "MUS-82"),
    ("prime-agent", "hung-handover-released", "MUS-81"),
];

/// Harnesses in the grid that have no fake yet, with the stage that adds one.
const NOT_YET: &[(&str, &str)] = &[];

// ---------------------------------------------------------------------------
// The daemon

#[derive(Clone, Copy, Debug, PartialEq)]
enum Mode {
    Plain,
    Tls,
}

fn daemon_bin() -> PathBuf {
    let bin = Path::new(SHIM).with_file_name("muster-daemon");
    assert!(
        bin.exists(),
        "{} is missing: run `cargo build --workspace` before the conformance suite",
        bin.display()
    );
    bin
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn wait_until(within: Duration, mut ok: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + within;
    while Instant::now() < deadline {
        if ok() {
            return true;
        }
        thread::sleep(Duration::from_millis(100));
    }
    ok()
}

/// A real `muster-daemon` in throwaway dirs on free loopback ports.
struct Daemon {
    dir: TempDir,
    mode: Mode,
    bind: u16,
    hook: u16,
    internal: u16,
    myko: u16,
    child: Option<Child>,
}

impl Daemon {
    fn start(mode: Mode) -> Self {
        let mut d = Self {
            dir: tempfile::tempdir().unwrap(),
            mode,
            bind: free_port(),
            hook: free_port(),
            internal: free_port(),
            myko: free_port(),
            child: None,
        };
        d.spawn();
        d
    }

    fn config_dir(&self) -> PathBuf {
        self.dir.path().join("config")
    }

    fn env(&self) -> Vec<(String, String)> {
        let p = |name: &str| self.dir.path().join(name).display().to_string();
        let addr = |port: u16| format!("127.0.0.1:{port}");
        vec![
            ("XDG_CONFIG_HOME".into(), p("config")),
            ("HOME".into(), p("home")),
            ("MUSTER_STATE_DIR".into(), p("state")),
            ("MUSTER_HOST".into(), "conformance".into()),
            (
                "MUSTER_TLS".into(),
                if self.mode == Mode::Tls { "1" } else { "0" }.into(),
            ),
            ("MUSTER_ACK_LEASE_SECS".into(), ACK_LEASE_SECS.to_string()),
            ("MUSTER_BIND".into(), addr(self.bind)),
            ("MUSTER_HOOK_BIND".into(), addr(self.hook)),
            ("MUSTER_INTERNAL_BIND".into(), addr(self.internal)),
            ("MUSTER_MYKO_BIND".into(), addr(self.myko)),
        ]
    }

    fn command(&self) -> Command {
        let mut cmd = Command::new(daemon_bin());
        for var in [
            "MUSTER_BASE_URL",
            "MUSTER_DAEMON_ADDRESS",
            "MYKO_ADDRESS",
            "MYKO_POSTGRES_URL",
        ] {
            cmd.env_remove(var);
        }
        cmd.envs(self.env());
        cmd
    }

    fn spawn(&mut self) {
        let log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.dir.path().join("daemon.log"))
            .unwrap();
        let child = self
            .command()
            .env("RUST_LOG", "info")
            .stdout(log.try_clone().unwrap())
            .stderr(log)
            .spawn()
            .expect("spawn muster-daemon");
        self.child = Some(child);
        let up = |port| TcpStream::connect(("127.0.0.1", port)).is_ok();
        assert!(
            wait_until(Duration::from_secs(20), || up(self.hook) && up(self.bind)),
            "muster-daemon didn't come up; log:\n{}",
            self.log()
        );
    }

    /// Stop it the way systemd does (SIGTERM, which ends held waits first), and
    /// start it again on the same ports and dirs.
    fn restart(&mut self) {
        self.stop();
        self.spawn();
    }

    fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = Command::new("kill")
                .args(["-TERM", &child.id().to_string()])
                .status();
            let exited = wait_until(Duration::from_secs(10), || {
                child.try_wait().ok().flatten().is_some()
            });
            if !exited {
                let _ = child.kill();
            }
            let _ = child.wait();
        }
    }

    /// Run a `muster-daemon` subcommand (`pause`, `resume`, …) against it.
    fn cli(&self, args: &[&str]) {
        let out = self.command().args(args).output().unwrap();
        assert!(
            out.status.success(),
            "muster-daemon {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn hook_addr(&self) -> SocketAddr {
        format!("127.0.0.1:{}", self.hook).parse().unwrap()
    }

    fn ws_address(&self) -> String {
        format!("ws://127.0.0.1:{}", self.bind)
    }

    fn log(&self) -> String {
        std::fs::read_to_string(self.dir.path().join("daemon.log")).unwrap_or_default()
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

// ---------------------------------------------------------------------------
// A fault-injecting proxy in front of the hook listener

/// Where a delivery's acknowledgement gets lost, which depends on how the
/// harness acks.
#[derive(Clone, Copy, Debug, PartialEq)]
enum AckFault {
    /// The harness acks with its own `/hook/ack` request (Claude's waiter):
    /// that request never reaches the daemon.
    DropAckRequests,
    /// The daemon acks once it has flushed a hook's reply (`deferred_ack`):
    /// the reply to a request starting with this path is read from the
    /// daemon, then lost before the harness gets it.
    DropReplies(&'static str),
}

/// Forwards hook requests to the daemon, and can lose a delivery's ack on the
/// way (`AckFault`). Every shim request is `Connection: close`.
struct HookProxy {
    base: String,
    fault: Arc<std::sync::Mutex<Option<AckFault>>>,
    faulted: Arc<AtomicUsize>,
}

impl HookProxy {
    fn start(upstream: SocketAddr) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let fault = Arc::new(std::sync::Mutex::new(None));
        let faulted = Arc::new(AtomicUsize::new(0));
        let (f, n) = (fault.clone(), faulted.clone());
        thread::spawn(move || {
            for client in listener.incoming().flatten() {
                let (f, n) = (f.clone(), n.clone());
                thread::spawn(move || {
                    let fault = *f.lock().unwrap();
                    let _ = forward(client, upstream, fault, &n);
                });
            }
        });
        Self {
            base,
            fault,
            faulted,
        }
    }

    fn set_fault(&self, fault: Option<AckFault>) {
        *self.fault.lock().unwrap() = fault;
    }

    /// How many acks the fault has lost so far.
    fn faulted(&self) -> usize {
        self.faulted.load(Ordering::SeqCst)
    }
}

fn forward(
    mut client: TcpStream,
    upstream: SocketAddr,
    fault: Option<AckFault>,
    faulted: &AtomicUsize,
) -> std::io::Result<()> {
    let mut reader = BufReader::new(client.try_clone()?);
    let mut head = String::new();
    let mut content_length = 0usize;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            return Ok(());
        }
        if let Some((k, v)) = line.split_once(':')
            && k.eq_ignore_ascii_case("content-length")
        {
            content_length = v.trim().parse().unwrap_or(0);
        }
        let end = line == "\r\n";
        head.push_str(&line);
        if end {
            break;
        }
    }
    let mut body = vec![0; content_length];
    reader.read_exact(&mut body)?;
    if fault == Some(AckFault::DropAckRequests) && head.starts_with("POST /hook/ack") {
        // The ack is lost on the way: the daemon never hears it.
        faulted.fetch_add(1, Ordering::SeqCst);
        return Ok(());
    }
    let mut up = TcpStream::connect(upstream)?;
    up.write_all(head.as_bytes())?;
    up.write_all(&body)?;
    if let Some(AckFault::DropReplies(prefix)) = fault
        && head.starts_with(&format!("POST {prefix}"))
    {
        // Take the whole reply, so the daemon has flushed it, then lose it.
        let mut reply = Vec::new();
        up.read_to_end(&mut reply)?;
        if reply.windows(13).any(|w| w == b"<muster_inbox") {
            faulted.fetch_add(1, Ordering::SeqCst);
        }
        return Ok(());
    }
    std::io::copy(&mut up, &mut client)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// A real muster-shim speaking MCP

/// `muster-shim` as an MCP server, as a harness spawns it. Used for the sender,
/// and for a Claude recipient, whose TUI runs one too.
struct Mcp {
    child: Child,
    stdin: ChildStdin,
    lines: mpsc::Receiver<String>,
    next_id: u64,
}

impl Mcp {
    fn start(daemon: &Daemon, session_id: &str) -> Self {
        let mut child = Command::new(SHIM)
            .env_remove("MUSTER_BASE_URL")
            .env_remove("MYKO_ADDRESS")
            .env("XDG_CONFIG_HOME", daemon.config_dir())
            .env("HOME", daemon.dir.path().join("home"))
            .env("MUSTER_HOST", "conformance")
            .env("MUSTER_DAEMON_ADDRESS", daemon.ws_address())
            .env("CLAUDE_CODE_SESSION_ID", session_id)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn muster-shim");
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, lines) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        let mut mcp = Self {
            child,
            stdin,
            lines,
            next_id: 0,
        };
        mcp.rpc(
            "initialize",
            json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "conformance", "version": "0"}
            }),
        )
        .expect("MCP initialize");
        mcp
    }

    fn rpc(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.next_id += 1;
        let id = self.next_id;
        let req = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        writeln!(self.stdin, "{req}").map_err(|e| e.to_string())?;
        self.stdin.flush().map_err(|e| e.to_string())?;
        let deadline = Instant::now() + Duration::from_secs(20);
        while let Some(left) = deadline.checked_duration_since(Instant::now()) {
            let line = self
                .lines
                .recv_timeout(left)
                .map_err(|_| format!("no reply to {method}"))?;
            let Ok(msg) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if msg["id"] == json!(id) {
                if let Some(err) = msg.get("error") {
                    return Err(format!("{method}: {err}"));
                }
                return Ok(msg["result"].clone());
            }
        }
        Err(format!("no reply to {method}"))
    }

    /// Call a tool and return its JSON result.
    fn tool(&mut self, name: &str, args: Value) -> Result<Value, String> {
        let r = self.rpc("tools/call", json!({"name": name, "arguments": args}))?;
        let text = r["content"][0]["text"].as_str().unwrap_or_default();
        if r["isError"] == json!(true) {
            return Err(format!("{name}: {text}"));
        }
        serde_json::from_str(text).map_err(|e| format!("{name} result isn't JSON ({e}): {text}"))
    }

    fn send(&mut self, to: &str, body: &str, key: Option<&str>) -> Result<Value, String> {
        let mut args = json!({"to": to, "body": body});
        if let Some(key) = key {
            args["idempotency_key"] = json!(key);
        }
        self.tool("send_message", args)
    }

    fn broadcast(&mut self, room: &str, body: &str) -> Result<Value, String> {
        self.tool("broadcast", json!({"to_room": room, "body": body}))
    }

    fn roster(&mut self) -> Result<Value, String> {
        let r = self.rpc("resources/read", json!({"uri": "muster://roster"}))?;
        let text = r["contents"][0]["text"].as_str().unwrap_or_default();
        serde_json::from_str(text).map_err(|e| format!("roster isn't JSON: {e}"))
    }

    /// The roster row for `session_id`, once it's there.
    fn row(&mut self, session_id: &str, within: Duration) -> Option<Value> {
        let deadline = Instant::now() + within;
        loop {
            if let Ok(roster) = self.roster()
                && let Some(row) = roster["sessions"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|s| s["session_id"] == json!(session_id))
            {
                return Some(row.clone());
            }
            if Instant::now() >= deadline {
                return None;
            }
            thread::sleep(Duration::from_millis(250));
        }
    }
}

impl Drop for Mcp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

// ---------------------------------------------------------------------------
// Fake harnesses

/// What a harness's TUI does, as far as delivery goes, driven against the real
/// binaries. Text returned is what the model would see.
trait FakeHarness {
    /// The session starts: whatever the TUI runs at session start.
    fn start(&mut self, daemon: &Daemon, proxy: &HookProxy);
    /// The session goes idle: arm whatever wakes it.
    fn arm_idle(&mut self);
    /// What an idle session was woken with, if it was woken within `within`.
    fn wait_for_wake(&mut self, within: Duration) -> Option<String>;
    /// Whether the armed wake path has fired yet, without consuming it.
    fn woken_yet(&mut self) -> bool;
    /// A busy session reaches its next delivery point (a hook or a turn
    /// boundary): what was injected, if anything.
    fn busy_boundary(&mut self) -> Option<String>;
    fn session_id(&self) -> &str;
    /// Lose this harness's delivery acks, or stop losing them, wherever it
    /// acks: a hook harness through the proxy, a push harness in the harness.
    fn lose_acks(&mut self, proxy: &HookProxy, on: bool);
    /// How many acks have been lost so far.
    fn acks_lost(&mut self, proxy: &HookProxy) -> usize;
    /// Whether the harness's MCP shim attaches to the session's roster row
    /// (Claude's does; Codex's registers no session, `MUSTER_HARNESS=codex`).
    fn mcp_attaches(&self) -> bool;
    /// Cut the harness's daemon connection, or restore it, and say whether
    /// it's open afterwards. Only a push plugin runs the scenarios that call it.
    fn set_online(&mut self, _on: bool) -> bool {
        unreachable!("only a push plugin cuts its connection")
    }
    /// Make the harness's API hang taking a push, or let the hung ones go
    /// (they're taken after all). Only a push plugin runs it.
    fn hang_handovers(&mut self, _on: bool) {
        unreachable!("only a push plugin hangs a handover")
    }
    /// How many handovers have hung so far.
    fn handovers_hung(&mut self) -> usize {
        unreachable!("only a push plugin hangs a handover")
    }
}

/// Claude Code, as `plugins/muster-shim/hooks/hooks.json` wires it: the
/// `claude-hook` commands with the hook JSON on stdin, an `asyncRewake` waiter
/// whose exit 2 starts a turn with its stderr, and the MCP shim Claude runs.
struct FakeClaude {
    session_id: String,
    cwd: String,
    env: Vec<(String, String)>,
    mcp: Option<Mcp>,
    waiter: Option<Child>,
    /// The first waiter comes from SessionStart, later ones from Stop.
    armed_before: bool,
}

/// The Claude Code version the fake's hook input is pinned to.
const CLAUDE_CODE_VERSION: &str = "2.1.318";

/// Claude Code's hook input for `event`, from the pinned fixtures.
fn claude_fixture(event: &str, session_id: &str, cwd: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(format!("claude-code-{CLAUDE_CODE_VERSION}"))
        .join(format!("{event}.json"));
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("fixture {}: {e}", path.display()))
        .replace("{{session_id}}", session_id)
        .replace("{{cwd}}", cwd)
        .trim()
        .to_string()
}

impl FakeClaude {
    fn new() -> Self {
        Self {
            session_id: uuid::Uuid::new_v4().to_string(),
            cwd: "/tmp/conformance".into(),
            env: Vec::new(),
            mcp: None,
            waiter: None,
            armed_before: false,
        }
    }

    fn hook_input(&self, fixture: &str) -> String {
        claude_fixture(fixture, &self.session_id, &self.cwd)
    }

    fn hook(&self, event: &str) -> Command {
        let mut cmd = Command::new(SHIM);
        cmd.args(["claude-hook", event])
            .env_remove("MUSTER_DAEMON_ADDRESS")
            .env_remove("MYKO_ADDRESS")
            .envs(self.env.iter().cloned());
        cmd
    }

    /// Run a command hook to completion, as Claude does, and return stdout.
    fn run_hook(&self, event: &str, fixture: &str) -> String {
        let mut child = self
            .hook(event)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn claude-hook");
        child
            .stdin
            .take()
            .unwrap()
            .write_all(self.hook_input(fixture).as_bytes())
            .unwrap();
        let out = child.wait_with_output().unwrap();
        String::from_utf8_lossy(&out.stdout).into_owned()
    }
}

impl FakeHarness for FakeClaude {
    fn start(&mut self, daemon: &Daemon, proxy: &HookProxy) {
        self.env = vec![
            (
                "XDG_CONFIG_HOME".into(),
                daemon.config_dir().display().to_string(),
            ),
            (
                "HOME".into(),
                daemon.dir.path().join("home").display().to_string(),
            ),
            ("MUSTER_HOST".into(), "conformance".into()),
            ("MUSTER_BASE_URL".into(), proxy.base.clone()),
            ("MUSTER_WAITER_LOCK_WAIT_SECS".into(), "2".into()),
        ];
        // SessionStart: register and inject identity; Claude also spawns the
        // MCP shim. The SessionStart waiter is `arm_idle`.
        self.run_hook("session-start", "session-start");
        self.mcp = Some(Mcp::start(daemon, &self.session_id));
    }

    fn arm_idle(&mut self) {
        // The SessionStart or Stop hook's `asyncRewake` waiter.
        let fixture = if self.armed_before {
            "stop"
        } else {
            "session-start"
        };
        self.armed_before = true;
        let input = self.hook_input(fixture);
        let mut child = self
            .hook("wait")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn claude-hook wait");
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
        self.waiter = Some(child);
        // Let it reach the daemon and register its WakeWatcher.
        thread::sleep(Duration::from_millis(1500));
    }

    fn wait_for_wake(&mut self, within: Duration) -> Option<String> {
        let mut child = self.waiter.take()?;
        let exited = wait_until(within, || child.try_wait().ok().flatten().is_some());
        if !exited {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        let status = child.wait().unwrap();
        let mut stderr = String::new();
        let _ = child.stderr.take().unwrap().read_to_string(&mut stderr);
        // asyncRewake: exit 2 starts a turn with stderr; anything else doesn't.
        (status.code() == Some(2)).then_some(stderr)
    }

    fn woken_yet(&mut self) -> bool {
        self.waiter
            .as_mut()
            .is_some_and(|w| w.try_wait().ok().flatten().is_some())
    }

    fn busy_boundary(&mut self) -> Option<String> {
        let out = self.run_hook("post-tool-use", "post-tool-use");
        let v: Value = serde_json::from_str(out.trim()).ok()?;
        let text = v["hookSpecificOutput"]["additionalContext"].as_str()?;
        (!text.is_empty()).then(|| text.to_string())
    }

    fn session_id(&self) -> &str {
        &self.session_id
    }

    fn lose_acks(&mut self, proxy: &HookProxy, on: bool) {
        proxy.set_fault(on.then_some(AckFault::DropAckRequests));
    }

    fn acks_lost(&mut self, proxy: &HookProxy) -> usize {
        proxy.faulted()
    }

    fn mcp_attaches(&self) -> bool {
        true
    }
}

impl Drop for FakeClaude {
    fn drop(&mut self) {
        if let Some(mut child) = self.waiter.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// The Codex version the fake's fixtures are pinned to.
const CODEX_VERSION: &str = "0.163.0";

/// A Codex fixture, with the thread id and cwd filled in.
fn codex_fixture(name: &str, thread_id: &str, cwd: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(format!("codex-{CODEX_VERSION}"))
        .join(format!("{name}.json"));
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("fixture {}: {e}", path.display()))
        .replace("{{thread_id}}", thread_id)
        .replace("{{cwd}}", cwd);
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("fixture {}: {e}", path.display()))
}

/// The one thread a fake app-server hosts.
struct AppThread {
    /// A turn is running, so `turn/start` is refused.
    busy: bool,
    /// `turn/start` requests accepted so far.
    turns: usize,
}

/// A Codex app-server with one thread, attached to a TUI: every connection
/// gets `thread/started` for it, and `turn/start` is accepted while the thread
/// is idle (and then a turn runs, so it's busy) and refused while it's busy.
struct FakeAppServer {
    endpoint: String,
    thread: Arc<std::sync::Mutex<AppThread>>,
    stop: Option<mpsc::Sender<()>>,
    join: Option<thread::JoinHandle<()>>,
}

impl FakeAppServer {
    fn start(thread_id: String, cwd: String) -> Self {
        let state = Arc::new(std::sync::Mutex::new(AppThread {
            busy: true,
            turns: 0,
        }));
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = format!("ws://{}", listener.local_addr().unwrap());
        let (stop, stopped) = mpsc::channel::<()>();
        let shared = state.clone();
        let join = thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(async move {
                let listener = tokio::net::TcpListener::from_std(listener).unwrap();
                let accept = async {
                    loop {
                        let Ok((stream, _)) = listener.accept().await else {
                            continue;
                        };
                        let (state, tid, cwd) = (shared.clone(), thread_id.clone(), cwd.clone());
                        tokio::spawn(async move {
                            // A bridge that's killed resets without a close
                            // handshake; that's cleanup, not a finding.
                            let _ = serve_app_server(stream, state, tid, cwd).await;
                        });
                    }
                };
                tokio::select! {
                    () = accept => {}
                    _ = tokio::task::spawn_blocking(move || stopped.recv()) => {}
                }
            });
        });
        Self {
            endpoint,
            thread: state,
            stop: Some(stop),
            join: Some(join),
        }
    }

    fn turns(&self) -> usize {
        self.thread.lock().unwrap().turns
    }

    fn set_busy(&self, busy: bool) {
        self.thread.lock().unwrap().busy = busy;
    }
}

impl Drop for FakeAppServer {
    fn drop(&mut self) {
        drop(self.stop.take());
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

async fn serve_app_server(
    stream: tokio::net::TcpStream,
    state: Arc<std::sync::Mutex<AppThread>>,
    thread_id: String,
    cwd: String,
) -> Result<(), String> {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message as Frame;

    let mut ws = tokio_tungstenite::accept_async(stream)
        .await
        .map_err(|e| e.to_string())?;
    let fixture = |name: &str| codex_fixture(name, &thread_id, &cwd);
    let mut announced = false;
    while let Some(frame) = ws.next().await {
        let Ok(Frame::Text(text)) = frame else {
            continue;
        };
        let Ok(req) = serde_json::from_str::<Value>(text.as_ref()) else {
            continue;
        };
        let method = req["method"].as_str().unwrap_or_default();
        let reply = match method {
            "initialize" => Some(json!({"id": req["id"], "result": fixture("initialize-result")})),
            "initialized" => {
                // An attached TUI's thread: every client hears about it.
                if !announced {
                    announced = true;
                    ws.send(Frame::Text(fixture("thread-started").to_string().into()))
                        .await
                        .map_err(|e| e.to_string())?;
                }
                None
            }
            "turn/start" if req["params"]["threadId"] == json!(thread_id) => {
                let mut t = state.lock().unwrap();
                if t.busy {
                    Some(json!({"id": req["id"], "error": fixture("turn-start-busy-error")}))
                } else {
                    t.busy = true;
                    t.turns += 1;
                    Some(json!({"id": req["id"], "result": fixture("turn-start-result")}))
                }
            }
            "turn/start" => Some(json!({
                "id": req["id"],
                "error": {"code": -32602, "message": "no such thread"}
            })),
            // Anything else the bridge asks gets an empty result, so it
            // never hangs waiting on the fake.
            _ if req.get("id").is_some() => Some(json!({"id": req["id"], "result": {}})),
            _ => None,
        };
        if let Some(reply) = reply {
            ws.send(Frame::Text(reply.to_string().into()))
                .await
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// Codex, as `muster-shim codex-run` sets it up: the real `codex-bridge`
/// watching the daemon and a (fake) shared app-server, and the `codex-hook`
/// commands Codex's hooks run. An idle thread is woken by the bridge's
/// `turn/start`; the turn's `UserPromptSubmit` hook then injects the inbox,
/// and the daemon acks it once that reply is flushed.
struct FakeCodex {
    thread_id: String,
    cwd: String,
    hook_base: String,
    env: Vec<(String, String)>,
    app: Option<FakeAppServer>,
    bridge: Option<Child>,
    turns_before: usize,
}

impl FakeCodex {
    fn new() -> Self {
        Self {
            thread_id: uuid::Uuid::new_v4().to_string(),
            cwd: "/tmp/conformance-codex".into(),
            hook_base: String::new(),
            env: Vec::new(),
            app: None,
            bridge: None,
            turns_before: 0,
        }
    }

    fn app(&self) -> &FakeAppServer {
        self.app.as_ref().expect("started")
    }

    /// Run a Codex command hook and return what it injected.
    fn run_hook(&self, event: &str, fixture: &str) -> Option<String> {
        let input = codex_fixture(fixture, &self.thread_id, &self.cwd).to_string();
        let mut child = Command::new(SHIM)
            .args(["codex-hook", event, &self.hook_base])
            .env_remove("MUSTER_DAEMON_ADDRESS")
            .env_remove("MYKO_ADDRESS")
            .envs(self.env.iter().cloned())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn codex-hook");
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
        let out = child.wait_with_output().unwrap();
        let v: Value = serde_json::from_slice(&out.stdout).ok()?;
        let text = v["hookSpecificOutput"]["additionalContext"].as_str()?;
        (!text.is_empty()).then(|| text.to_string())
    }
}

impl FakeHarness for FakeCodex {
    fn start(&mut self, daemon: &Daemon, proxy: &HookProxy) {
        let tmp = daemon.dir.path().join("tmp");
        std::fs::create_dir_all(&tmp).unwrap();
        self.hook_base = proxy.base.clone();
        self.env = vec![
            (
                "XDG_CONFIG_HOME".into(),
                daemon.config_dir().display().to_string(),
            ),
            (
                "HOME".into(),
                daemon.dir.path().join("home").display().to_string(),
            ),
            ("MUSTER_HOST".into(), "conformance".into()),
            ("MUSTER_BASE_URL".into(), proxy.base.clone()),
            ("TMPDIR".into(), tmp.display().to_string()),
        ];
        let app = FakeAppServer::start(self.thread_id.clone(), self.cwd.clone());
        let ready = daemon.dir.path().join("bridge.ready");
        let log = std::fs::File::create(daemon.dir.path().join("bridge.log")).unwrap();
        let bridge = Command::new(SHIM)
            .args([
                "codex-bridge",
                "--endpoint",
                &app.endpoint,
                "--daemon",
                &daemon.ws_address(),
                "--poll-ms",
                "100",
                "--ready-file",
            ])
            .arg(&ready)
            .env_remove("MUSTER_DAEMON_ADDRESS")
            .env_remove("MYKO_ADDRESS")
            .envs(self.env.iter().cloned())
            .env("RUST_LOG", "muster_shim=debug")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(log)
            .spawn()
            .expect("spawn codex-bridge");
        self.app = Some(app);
        self.bridge = Some(bridge);
        assert!(
            wait_until(Duration::from_secs(20), || ready.is_file()),
            "codex-bridge never became ready; log:\n{}",
            std::fs::read_to_string(daemon.dir.path().join("bridge.log")).unwrap_or_default()
        );
    }

    fn arm_idle(&mut self) {
        // The turn ends and the thread sits idle in the TUI.
        self.turns_before = self.app().turns();
        self.app().set_busy(false);
    }

    fn wait_for_wake(&mut self, within: Duration) -> Option<String> {
        let before = self.turns_before;
        if !wait_until(within, || self.app().turns() > before) {
            return None;
        }
        // The bridge started a turn: Codex runs its UserPromptSubmit hook.
        Some(
            self.run_hook("prompt-submit", "user-prompt-submit")
                .unwrap_or_default(),
        )
    }

    fn woken_yet(&mut self) -> bool {
        self.app().turns() > self.turns_before
    }

    fn busy_boundary(&mut self) -> Option<String> {
        self.app().set_busy(true);
        self.run_hook("post-tool-use", "post-tool-use")
    }

    fn session_id(&self) -> &str {
        &self.thread_id
    }

    fn lose_acks(&mut self, proxy: &HookProxy, on: bool) {
        proxy.set_fault(on.then_some(AckFault::DropReplies("/hook/prompt-submit")));
    }

    fn acks_lost(&mut self, proxy: &HookProxy) -> usize {
        proxy.faulted()
    }

    fn mcp_attaches(&self) -> bool {
        false
    }
}

impl Drop for FakeCodex {
    fn drop(&mut self) {
        if let Some(mut child) = self.bridge.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// bun, for the TypeScript plugins' fakes: `MUSTER_BUN`, else `bun` on PATH.
fn bun_bin() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("MUSTER_BUN").filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(path));
    }
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|dir| dir.join("bun"))
            .find(|bun| bun.is_file())
    })
}

/// A TypeScript plugin, loaded by its `test/conformance-driver.ts` with a fake
/// host that stands in for the harness's API, and driven over the driver's
/// line protocol. The plugin connects to the daemon itself, as it does inside
/// its harness, and takes the live push. Its `node_modules` must be installed.
struct FakePlugin {
    plugin: &'static str,
    session_id: String,
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    replies: Option<mpsc::Receiver<String>>,
}

/// The prefix on the driver's replies, which the plugin's own logging lacks.
const DRIVER_REPLY: &str = "@@conformance ";

impl FakePlugin {
    fn new(plugin: &'static str) -> Self {
        Self {
            plugin,
            session_id: String::new(),
            child: None,
            stdin: None,
            replies: None,
        }
    }

    fn call(&mut self, cmd: Value) -> Value {
        let stdin = self.stdin.as_mut().expect("started");
        writeln!(stdin, "{cmd}").expect("write to the plugin driver");
        stdin.flush().expect("flush the plugin driver");
        let wait = cmd["ms"].as_u64().map_or(0, |ms| ms / 1000) + 20;
        let line = self
            .replies
            .as_ref()
            .expect("started")
            .recv_timeout(Duration::from_secs(wait))
            .unwrap_or_else(|_| panic!("{} driver didn't answer {cmd}", self.plugin));
        let reply: Value = serde_json::from_str(&line).expect("driver reply is JSON");
        if let Some(e) = reply.get("error") {
            panic!("{} driver: {e}", self.plugin);
        }
        reply
    }

    fn text(reply: &Value) -> Option<String> {
        reply["text"].as_str().map(str::to_string)
    }
}

impl FakeHarness for FakePlugin {
    fn start(&mut self, daemon: &Daemon, _proxy: &HookProxy) {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../plugins")
            .join(self.plugin);
        assert!(
            dir.join("node_modules").is_dir(),
            "{}/node_modules is missing: install it first (npm ci --workspaces=false)",
            dir.display()
        );
        let log =
            std::fs::File::create(daemon.dir.path().join(format!("{}.log", self.plugin))).unwrap();
        let mut child = Command::new(bun_bin().expect("bun"))
            .arg("run")
            .arg(dir.join("test/conformance-driver.ts"))
            .current_dir(daemon.dir.path())
            .env_remove("MYKO_ADDRESS")
            .env("MUSTER_DAEMON_ADDRESS", daemon.ws_address())
            .env("MUSTER_HOST", "conformance")
            .env("MUSTER_OPERATOR", "conformance")
            // The plugin's hold on a push lapses after the same lease (MUS-81).
            .env("MUSTER_ACK_LEASE_SECS", ACK_LEASE_SECS.to_string())
            .env("HOME", daemon.dir.path().join("home"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(log)
            .spawn()
            .expect("spawn the plugin driver under bun");
        let stdout = child.stdout.take().unwrap();
        let (tx, replies) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Some(reply) = line.strip_prefix(DRIVER_REPLY)
                    && tx.send(reply.to_string()).is_err()
                {
                    break;
                }
            }
        });
        self.stdin = child.stdin.take();
        self.child = Some(child);
        self.replies = Some(replies);
        let session_file = daemon
            .dir
            .path()
            .join(format!("{}-session.jsonl", self.plugin));
        let reply = self.call(json!({"cmd": "start", "session_file": session_file}));
        self.session_id = reply["session_id"]
            .as_str()
            .expect("session id")
            .to_string();
    }

    fn arm_idle(&mut self) {
        self.call(json!({"cmd": "arm_idle"}));
    }

    fn wait_for_wake(&mut self, within: Duration) -> Option<String> {
        let ms = within.as_millis() as u64;
        Self::text(&self.call(json!({"cmd": "wait_for_wake", "ms": ms})))
    }

    fn woken_yet(&mut self) -> bool {
        self.call(json!({"cmd": "woken_yet"}))["woken"] == json!(true)
    }

    fn busy_boundary(&mut self) -> Option<String> {
        Self::text(&self.call(json!({"cmd": "busy_boundary"})))
    }

    fn session_id(&self) -> &str {
        &self.session_id
    }

    fn lose_acks(&mut self, _proxy: &HookProxy, on: bool) {
        // The daemon marks a pushed message read when the push reaches the
        // plugin's connection. Lose it after that: the harness's API throws.
        self.call(json!({"cmd": "lose_deliveries", "on": on}));
    }

    fn acks_lost(&mut self, _proxy: &HookProxy) -> usize {
        self.call(json!({"cmd": "lost"}))["lost"]
            .as_u64()
            .unwrap_or(0) as usize
    }

    fn mcp_attaches(&self) -> bool {
        false
    }

    fn set_online(&mut self, on: bool) -> bool {
        self.call(json!({"cmd": "set_online", "on": on}))["open"] == json!(true)
    }

    fn hang_handovers(&mut self, on: bool) {
        self.call(json!({"cmd": "hang_deliveries", "on": on}));
    }

    fn handovers_hung(&mut self) -> usize {
        self.call(json!({"cmd": "hung"}))["hung"]
            .as_u64()
            .unwrap_or(0) as usize
    }
}

impl Drop for FakePlugin {
    fn drop(&mut self) {
        drop(self.stdin.take());
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

// ---------------------------------------------------------------------------
// Scenarios

/// Everything one cell needs: a fresh daemon, the proxy, the harness under
/// test, and a sender.
struct Cell {
    daemon: Daemon,
    proxy: HookProxy,
    harness: Box<dyn FakeHarness>,
    sender: Mcp,
    sender_id: String,
}

impl Cell {
    fn new(mode: Mode, mut harness: Box<dyn FakeHarness>) -> Self {
        let daemon = Daemon::start(mode);
        let proxy = HookProxy::start(daemon.hook_addr());
        harness.start(&daemon, &proxy);
        let sender_id = uuid::Uuid::new_v4().to_string();
        let sender = Self::sender(&daemon, &sender_id);
        Self {
            daemon,
            proxy,
            harness,
            sender,
            sender_id,
        }
    }

    /// A sender session, registered as a Claude session's hook would, with
    /// its MCP shim.
    fn sender(daemon: &Daemon, sender_id: &str) -> Mcp {
        let mut hook = Command::new(SHIM)
            .args(["claude-hook", "session-start"])
            .env("XDG_CONFIG_HOME", daemon.config_dir())
            .env("HOME", daemon.dir.path().join("home"))
            .env("MUSTER_HOST", "conformance")
            .env("MUSTER_BASE_URL", format!("http://{}", daemon.hook_addr()))
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        hook.stdin
            .take()
            .unwrap()
            .write_all(
                json!({"session_id": sender_id, "cwd": "/tmp/sender"})
                    .to_string()
                    .as_bytes(),
            )
            .unwrap();
        hook.wait().unwrap();
        Mcp::start(daemon, sender_id)
    }

    fn recipient(&self) -> String {
        self.harness.session_id().to_string()
    }

    /// Wait until the sender can see the recipient on the roster.
    fn recipient_on_roster(&mut self) -> Result<Value, String> {
        let id = self.recipient();
        self.sender
            .row(&id, Duration::from_secs(15))
            .ok_or_else(|| format!("recipient {id} never appeared on the roster"))
    }

    fn send(&mut self, body: &str) -> Result<Value, String> {
        self.recipient_on_roster()?;
        let to = self.recipient();
        self.sender.send(&to, body, None)
    }
}

fn token(label: &str) -> String {
    format!(
        "{label}-{}",
        &uuid::Uuid::new_v4().simple().to_string()[..12]
    )
}

fn contains(seen: &Option<String>, token: &str) -> bool {
    seen.as_deref().is_some_and(|s| s.contains(token))
}

type Scenario = fn(&mut Cell) -> Result<(), String>;

const SCENARIOS: &[(&str, Scenario)] = &[
    ("idle-wakes-on-dm", idle_wakes_on_dm),
    ("busy-gets-it-at-next-hook", busy_gets_it_at_next_hook),
    ("dropped-ack-redelivers", dropped_ack_redelivers),
    ("lost-hook-reply-redelivers", lost_hook_reply_redelivers),
    ("duplicate-shown-at-most-once", duplicate_shown_at_most_once),
    ("burst-both-delivered", burst_both_delivered),
    ("daemon-restart-mid-wait", daemon_restart_mid_wait),
    ("operator-pause-stops-wakes", operator_pause_stops_wakes),
    ("resume-wakes-held-mail", resume_wakes_held_mail),
    ("broadcast-never-wakes", broadcast_never_wakes),
    ("wake-none-not-expected", wake_none_not_expected),
    ("mail-waiting-at-reconnect", mail_waiting_at_reconnect),
    ("hung-handover-released", hung_handover_released),
];

/// Scenario 1: An idle session wakes on a direct message.
fn idle_wakes_on_dm(c: &mut Cell) -> Result<(), String> {
    c.harness.arm_idle();
    let t = token("idle");
    c.send(&t)?;
    let woke = c.harness.wait_for_wake(WAKE_WITHIN);
    if !contains(&woke, &t) {
        return Err(format!("not woken with the message; woke with {woke:?}"));
    }
    Ok(())
}

/// Scenario 2: A busy session gets the message at its next hook or turn, once.
fn busy_gets_it_at_next_hook(c: &mut Cell) -> Result<(), String> {
    let t = token("busy");
    c.send(&t)?;
    let seen = c.harness.busy_boundary();
    if !contains(&seen, &t) {
        return Err(format!("next hook didn't carry it; got {seen:?}"));
    }
    let again = c.harness.busy_boundary();
    if contains(&again, &t) {
        return Err("the hook after that showed it again".into());
    }
    Ok(())
}

/// Scenario 3: No loss: when a delivery's ack is lost (Claude: the waiter's
/// `/hook/ack`; Codex: the hook's reply, which the daemon acks on flush), the
/// message comes again at the session's next delivery point once the lease
/// lapses, and not after an ack that arrives.
fn dropped_ack_redelivers(c: &mut Cell) -> Result<(), String> {
    c.harness.lose_acks(&c.proxy, true);
    c.harness.arm_idle();
    let t = token("ack");
    c.send(&t)?;
    let _first = c.harness.wait_for_wake(WAKE_WITHIN);
    if c.harness.acks_lost(&c.proxy) == 0 {
        return Err("no delivery's ack was lost, so the scenario didn't run".into());
    }
    c.harness.lose_acks(&c.proxy, false);
    thread::sleep(Duration::from_secs(ACK_LEASE_SECS + 1));
    let again = c.harness.busy_boundary();
    if !contains(&again, &t) {
        return Err(format!(
            "not delivered again after its ack was lost (message lost); got {again:?}"
        ));
    }
    thread::sleep(Duration::from_secs(ACK_LEASE_SECS + 1));
    if contains(&c.harness.busy_boundary(), &t) {
        return Err("delivered again after a successful ack".into());
    }
    Ok(())
}

/// Scenario 3b: No loss on a busy session's hook either: when the reply to a
/// prompt hook is lost after the daemon sent it (as when the harness kills a
/// hook at its timeout), the message comes again at the next hook.
fn lost_hook_reply_redelivers(c: &mut Cell) -> Result<(), String> {
    c.proxy
        .set_fault(Some(AckFault::DropReplies("/hook/prompt-submit")));
    let t = token("reply");
    c.send(&t)?;
    let _lost = c.harness.busy_boundary();
    if c.proxy.faulted() == 0 {
        return Err("no hook reply carrying mail was lost, so the scenario didn't run".into());
    }
    c.proxy.set_fault(None);
    thread::sleep(Duration::from_secs(ACK_LEASE_SECS + 1));
    let again = c.harness.busy_boundary();
    if !contains(&again, &t) {
        return Err(format!(
            "not delivered again after the hook's reply was lost (message lost); got {again:?}"
        ));
    }
    Ok(())
}

/// Scenario 4: A duplicate is shown at most once: a send retried with the same
/// idempotency key is one message, and mail delivered by the waiter isn't
/// injected again by the next hook.
fn duplicate_shown_at_most_once(c: &mut Cell) -> Result<(), String> {
    c.recipient_on_roster()?;
    let to = c.recipient();
    let t = token("dup");
    let key = uuid::Uuid::new_v4().to_string();
    let a = c.sender.send(&to, &t, Some(&key))?;
    let b = c.sender.send(&to, &t, Some(&key))?;
    if a["message_id"] != b["message_id"] {
        return Err(format!(
            "a retried send made a second message: {} and {}",
            a["message_id"], b["message_id"]
        ));
    }
    let seen = c.harness.busy_boundary().unwrap_or_default();
    if seen.matches(&t).count() != 1 {
        return Err(format!(
            "shown {} times in one hook, not once",
            seen.matches(&t).count()
        ));
    }
    if contains(&c.harness.busy_boundary(), &t) {
        return Err("shown again at the next hook".into());
    }
    c.harness.arm_idle();
    let t2 = token("dup-wake");
    c.send(&t2)?;
    let woke = c.harness.wait_for_wake(WAKE_WITHIN).unwrap_or_default();
    match woke.matches(&t2).count() {
        0 => return Err("the wake didn't deliver the second message".into()),
        1 => {}
        // As when a push's text and the woken turn's inbox drain both carry it.
        n => {
            return Err(format!(
                "the wake showed the second message {n} times, not once"
            ));
        }
    }
    if contains(&c.harness.busy_boundary(), &t2) {
        return Err("mail the waiter delivered was injected again by the next hook".into());
    }
    Ok(())
}

/// Scenario 4b: Two direct messages sent back to back to an idle session both
/// reach the model, between the wake and the next delivery point. A harness
/// that drains the inbox while handling the first must show what it drains,
/// not just ack it.
fn burst_both_delivered(c: &mut Cell) -> Result<(), String> {
    c.harness.arm_idle();
    let (t1, t2) = (token("burst-a"), token("burst-b"));
    c.send(&t1)?;
    c.send(&t2)?;
    let woke = c.harness.wait_for_wake(WAKE_WITHIN).unwrap_or_default();
    // Whatever the wake didn't carry must come at the next delivery point.
    thread::sleep(Duration::from_millis(500));
    let next = c.harness.busy_boundary().unwrap_or_default();
    let seen = format!("{woke}\n{next}");
    let missing: Vec<&str> = [&t1, &t2]
        .into_iter()
        .filter(|t| !seen.contains(t.as_str()))
        .map(String::as_str)
        .collect();
    if !missing.is_empty() {
        return Err(format!(
            "never shown: {missing:?} (the wake showed {:?}, the next delivery point {:?})",
            woke, next
        ));
    }
    Ok(())
}

/// Scenario 5: The daemon restarts while a session waits; the session still wakes on
/// mail sent after the restart.
fn daemon_restart_mid_wait(c: &mut Cell) -> Result<(), String> {
    c.harness.arm_idle();
    c.daemon.restart();
    // The old sender's connection went with the old daemon, and without
    // Postgres so did the roster; register the sender again.
    c.sender = Cell::sender(&c.daemon, &c.sender_id);
    let t = token("restart");
    c.send(&t)?;
    let woke = c.harness.wait_for_wake(Duration::from_secs(25));
    if !contains(&woke, &t) {
        return Err(format!(
            "not woken after the restart; got {woke:?}; daemon log:\n{}",
            c.daemon.log()
        ));
    }
    Ok(())
}

/// Scenario 6: The operator's pause stops wakes. Mail still arrives at the next hook,
/// which doesn't wake anyone.
fn operator_pause_stops_wakes(c: &mut Cell) -> Result<(), String> {
    c.daemon.cli(&["pause"]);
    // The daemon reads its controls every 2 s.
    thread::sleep(Duration::from_millis(2500));
    c.harness.arm_idle();
    let t = token("paused");
    c.send(&t)?;
    let woke = c.harness.wait_for_wake(QUIET_FOR);
    if woke.is_some() {
        return Err(format!("woken while paused: {woke:?}"));
    }
    let seen = c.harness.busy_boundary();
    if !contains(&seen, &t) {
        return Err(format!(
            "the next hook didn't deliver it while paused; got {seen:?}"
        ));
    }
    Ok(())
}

/// Scenario 6b: Resuming alone wakes a session whose mail arrived while wakes
/// were paused. No new message is sent after the resume: the held, still
/// unread mail is enough.
fn resume_wakes_held_mail(c: &mut Cell) -> Result<(), String> {
    c.daemon.cli(&["pause"]);
    // The daemon reads its controls every 2 s.
    thread::sleep(Duration::from_millis(2500));
    c.harness.arm_idle();
    let t = token("held");
    c.send(&t)?;
    // Held through a pause long enough for any wake to have fired.
    thread::sleep(QUIET_FOR);
    if c.harness.woken_yet() {
        return Err("woken while paused, so resuming wasn't what woke it".into());
    }
    c.daemon.cli(&["resume"]);
    let woke = c.harness.wait_for_wake(WAKE_WITHIN);
    if !contains(&woke, &t) {
        return Err(format!(
            "resuming didn't wake the session with the mail held during the pause; got {woke:?}"
        ));
    }
    Ok(())
}

/// Scenario 7: Broadcasts never wake anyone, and aren't injected either.
fn broadcast_never_wakes(c: &mut Cell) -> Result<(), String> {
    c.recipient_on_roster()?;
    c.harness.arm_idle();
    let t = token("broadcast");
    c.sender.broadcast("everyone", &t)?;
    let woke = c.harness.wait_for_wake(QUIET_FOR);
    if woke.is_some() {
        return Err(format!("woken by a broadcast: {woke:?}"));
    }
    if contains(&c.harness.busy_boundary(), &t) {
        return Err("a broadcast was injected at the next hook".into());
    }
    Ok(())
}

/// Scenario 8: A session the roster calls `wake: none` must not be reported to the
/// sender as `wake: expected` (MUS-71). Here the session's MCP shim is
/// connected, but no waiter runs and it isn't busy, so nothing can wake it.
fn wake_none_not_expected(c: &mut Cell) -> Result<(), String> {
    let mut row = c.recipient_on_roster()?;
    if c.harness.mcp_attaches() {
        // A hook-registered row has pid 0; the session's MCP shim fills it in
        // when it attaches. Until then the roster says `none` for a different
        // reason, and the bug can't show.
        let id = c.recipient();
        let deadline = Instant::now() + Duration::from_secs(15);
        while row["pid"].as_u64().unwrap_or(0) == 0 {
            if Instant::now() >= deadline {
                return Err("precondition: the session's MCP shim never attached".into());
            }
            thread::sleep(Duration::from_millis(250));
            row = c
                .sender
                .row(&id, Duration::from_secs(5))
                .unwrap_or_default();
        }
    }
    if row["wake"] != json!("none") {
        return Err(format!(
            "precondition: the roster should say wake none, says {}",
            row["wake"]
        ));
    }
    let r = c.send(&token("none"))?;
    if r["wake"] == json!("expected") {
        return Err(format!(
            "send said wake expected for a session the roster says can't wake (live_push {})",
            r["live_push"]
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// The grid

/// One column of the grid.
struct Harness {
    name: &'static str,
    mode: Mode,
    make: fn() -> Box<dyn FakeHarness>,
    /// Scenarios that don't apply to this harness, and why. They show as
    /// `n/a` in the grid.
    not_applicable: &'static [(&'static str, &'static str)],
    /// A TypeScript plugin's fake, which needs bun (`bun_bin`).
    needs_bun: bool,
}

/// Scenario 9 (MUS-81, upstream #134's case): Mail that waited out a
/// dropped connection is shown once, at the next turn, and a push after the
/// reconnect doesn't take it with it. While the plugin is offline the daemon
/// can't push, so the message stays unread. Then a second message is pushed
/// live: a plugin that drains the inbox after a push would ack the first one
/// there without showing it.
fn mail_waiting_at_reconnect(c: &mut Cell) -> Result<(), String> {
    c.recipient_on_roster()?;
    let to = c.recipient();
    if c.harness.set_online(false) {
        return Err("the plugin's connection never dropped, so the scenario didn't run".into());
    }
    let waiting = token("waiting");
    let sent = c.sender.send(&to, &waiting, None)?;
    if sent["live_push"] == json!("delivered") {
        return Err(format!(
            "pushed while the plugin was offline, so the scenario didn't run: {sent}"
        ));
    }
    if !c.harness.set_online(true) {
        return Err("the plugin never reconnected".into());
    }
    let pushed = token("pushed");
    let sent = c.sender.send(&to, &pushed, None)?;
    if sent["live_push"] != json!("delivered") {
        return Err(format!(
            "the message after the reconnect wasn't pushed, so the scenario didn't run: {sent}"
        ));
    }
    let seen = c.harness.busy_boundary().unwrap_or_default();
    for (what, t) in [("the mail that waited", &waiting), ("the push", &pushed)] {
        let n = seen.matches(t.as_str()).count();
        if n != 1 {
            return Err(format!(
                "{what} was shown {n} times at the next turn, not once; got {seen:?}"
            ));
        }
    }
    thread::sleep(Duration::from_secs(ACK_LEASE_SECS + 1));
    let again = c.harness.busy_boundary();
    if contains(&again, &waiting) || contains(&again, &pushed) {
        return Err(format!("shown again at the turn after; got {again:?}"));
    }
    Ok(())
}

/// Scenario 10 (MUS-81): A push whose handover hangs isn't lost behind it.
/// The harness's API never resolves taking the push. While the plugin holds
/// it, a turn's pull doesn't show it too; once the hold's lease lapses, the
/// next turn shows it. When the hung handover finally goes through, the
/// harness has it a second time (the design's accepted duplicate), but no
/// later pull shows it again.
fn hung_handover_released(c: &mut Cell) -> Result<(), String> {
    c.harness.hang_handovers(true);
    let t = token("hung");
    c.send(&t)?;
    let deadline = Instant::now() + WAKE_WITHIN;
    while c.harness.handovers_hung() == 0 {
        if Instant::now() >= deadline {
            return Err("no handover hung, so the scenario didn't run".into());
        }
        thread::sleep(Duration::from_millis(100));
    }
    let early = c.harness.busy_boundary();
    if contains(&early, &t) {
        return Err(format!(
            "a pull showed it while its push was still being handed over; got {early:?}"
        ));
    }
    thread::sleep(Duration::from_secs(ACK_LEASE_SECS + 1));
    let late = c.harness.busy_boundary();
    if !contains(&late, &t) {
        return Err(format!(
            "not shown once the hung handover's hold lapsed (message lost); got {late:?}"
        ));
    }
    c.harness.hang_handovers(false);
    // The late handover goes through now, and may show it a second time.
    let _late_handover = c.harness.busy_boundary();
    thread::sleep(Duration::from_secs(ACK_LEASE_SECS + 1));
    if contains(&c.harness.busy_boundary(), &t) {
        return Err("a later pull showed it again after it was acked".into());
    }
    Ok(())
}

const HARNESSES: &[Harness] = &[
    Harness {
        name: "claude",
        mode: Mode::Plain,
        make: || Box::new(FakeClaude::new()),
        not_applicable: HOOK_HARNESS_NOT_APPLICABLE,
        needs_bun: false,
    },
    Harness {
        name: "claude-tls",
        mode: Mode::Tls,
        make: || Box::new(FakeClaude::new()),
        not_applicable: HOOK_HARNESS_NOT_APPLICABLE,
        needs_bun: false,
    },
    Harness {
        name: "codex",
        mode: Mode::Plain,
        make: || Box::new(FakeCodex::new()),
        not_applicable: HOOK_HARNESS_NOT_APPLICABLE,
        needs_bun: false,
    },
    Harness {
        name: "pi",
        mode: Mode::Plain,
        make: || Box::new(FakePlugin::new("muster-pi")),
        not_applicable: PUSH_PLUGIN_NOT_APPLICABLE,
        needs_bun: true,
    },
    Harness {
        name: "opencode",
        mode: Mode::Plain,
        make: || Box::new(FakePlugin::new("muster-opencode")),
        not_applicable: PUSH_PLUGIN_NOT_APPLICABLE,
        needs_bun: true,
    },
    Harness {
        name: "prime-agent",
        mode: Mode::Plain,
        make: || Box::new(FakePlugin::new("muster-prime-agent")),
        not_applicable: PUSH_PLUGIN_NOT_APPLICABLE,
        needs_bun: true,
    },
];

/// Scenarios that don't apply to a harness delivered by hooks, with no live
/// push to the session.
const HOOK_HARNESS_NOT_APPLICABLE: &[(&str, &str)] = &[
    (
        "mail-waiting-at-reconnect",
        "no live push: mail waits for the next hook, which pulls it",
    ),
    (
        "hung-handover-released",
        "no live push to hand over: the hook prints the inbox and then acks",
    ),
];

/// Scenarios that don't apply to a plugin that takes the live push.
const PUSH_PLUGIN_NOT_APPLICABLE: &[(&str, &str)] = &[
    (
        "lost-hook-reply-redelivers",
        "no prompt hooks: delivery is the live push and a per-turn inbox drain",
    ),
    (
        "wake-none-not-expected",
        "a connected plugin renders pushes, so the roster says channel, never none",
    ),
];

enum Outcome {
    Pass,
    Fail(String),
    KnownFailure(&'static str),
    UnexpectedPass(&'static str),
    NotApplicable,
    NoBun,
}

fn run_cell(harness: &Harness, scenario: Scenario, name: &str) -> Outcome {
    if harness.not_applicable.iter().any(|(s, _)| *s == name) {
        return Outcome::NotApplicable;
    }
    if harness.needs_bun && bun_bin().is_none() {
        return Outcome::NoBun;
    }
    let known = KNOWN_FAILURES
        .iter()
        .find(|(h, s, _)| *h == harness.name && *s == name)
        .map(|(_, _, story)| *story);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut cell = Cell::new(harness.mode, (harness.make)());
        scenario(&mut cell)
    }))
    .unwrap_or_else(|panic| {
        Err(panic
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| panic.downcast_ref::<&str>().map(|s| s.to_string()))
            .unwrap_or_else(|| "panicked".into()))
    });
    match (result, known) {
        (Ok(()), None) => Outcome::Pass,
        (Ok(()), Some(story)) => Outcome::UnexpectedPass(story),
        (Err(_), Some(story)) => Outcome::KnownFailure(story),
        (Err(e), None) => Outcome::Fail(e),
    }
}

#[test]
#[ignore = "needs `cargo build --workspace` first; run with --ignored"]
fn delivery_conformance() {
    let only = std::env::var("CONFORMANCE_ONLY").ok();
    let mut grid: BTreeMap<(&str, &str), String> = BTreeMap::new();
    let mut failures = Vec::new();
    for harness in HARNESSES {
        for (name, scenario) in SCENARIOS {
            if only
                .as_deref()
                .is_some_and(|o| !name.contains(o) && !harness.name.contains(o))
            {
                continue;
            }
            let started = Instant::now();
            let outcome = run_cell(harness, *scenario, name);
            let cell = match &outcome {
                Outcome::Pass => "pass".to_string(),
                Outcome::KnownFailure(story) => format!("known ({story})"),
                Outcome::Fail(_) => "FAIL".to_string(),
                Outcome::UnexpectedPass(story) => format!("PASS? ({story})"),
                Outcome::NotApplicable => "n/a".to_string(),
                Outcome::NoBun => "no bun".to_string(),
            };
            eprintln!(
                "{:<12} {:<30} {:<14} {:.1}s",
                harness.name,
                name,
                cell,
                started.elapsed().as_secs_f32()
            );
            if let Outcome::Fail(e) = &outcome {
                eprintln!("    why: {e}");
            }
            match outcome {
                Outcome::Fail(e) => failures.push(format!("{} / {name}: {e}", harness.name)),
                Outcome::UnexpectedPass(story) => failures.push(format!(
                    "{} / {name} passes, but KNOWN_FAILURES lists it for {story}: remove it",
                    harness.name
                )),
                // CI sets CONFORMANCE_REQUIRE_BUN, so a missing bun there is a
                // failure, not a quiet gap.
                Outcome::NoBun if std::env::var_os("CONFORMANCE_REQUIRE_BUN").is_some() => {
                    failures.push(format!(
                        "{} / {name}: no bun (set MUSTER_BUN or put bun on PATH)",
                        harness.name
                    ));
                }
                _ => {}
            }
            grid.insert((harness.name, name), cell);
        }
    }

    // The grid: one row per scenario, one column per harness.
    let mut columns: Vec<&str> = HARNESSES.iter().map(|h| h.name).collect();
    columns.extend(NOT_YET.iter().map(|(h, _)| *h));
    let mut table = format!("\n{:<30}", "scenario");
    for col in &columns {
        table.push_str(&format!(" | {col:<14}"));
    }
    table.push('\n');
    for (name, _) in SCENARIOS {
        table.push_str(&format!("{name:<30}"));
        for col in &columns {
            let cell = grid.get(&(*col, *name)).cloned().unwrap_or_else(|| {
                NOT_YET
                    .iter()
                    .find(|(h, _)| h == col)
                    .map_or("skipped".into(), |_| "not yet".into())
            });
            table.push_str(&format!(" | {cell:<14}"));
        }
        table.push('\n');
    }
    for (harness, stage) in NOT_YET {
        table.push_str(&format!("not yet: {harness} ({stage})\n"));
    }
    for harness in HARNESSES {
        for (scenario, why) in harness.not_applicable {
            table.push_str(&format!("n/a: {} × {scenario}: {why}\n", harness.name));
        }
    }
    eprintln!("{table}");
    if let Ok(path) = std::env::var("CONFORMANCE_GRID") {
        let _ = std::fs::write(path, &table);
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}
