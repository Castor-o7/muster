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
        atomic::{AtomicBool, AtomicUsize, Ordering},
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
    ("claude", "wake-none-not-expected", "MUS-71"),
    ("claude-tls", "wake-none-not-expected", "MUS-71"),
];

/// Harnesses in the grid that have no fake yet, with the stage that adds one.
const NOT_YET: &[(&str, &str)] = &[
    ("codex", "MUS-69 stage 2"),
    ("opencode", "MUS-69 stage 3"),
    ("pi", "MUS-69 stage 3"),
    ("prime-agent", "MUS-69 stage 3"),
];

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

/// Forwards hook requests to the daemon, and can drop `/hook/ack` so a
/// delivery's ack never arrives. Every shim request is `Connection: close`.
struct HookProxy {
    base: String,
    drop_acks: Arc<AtomicBool>,
    dropped: Arc<AtomicUsize>,
}

impl HookProxy {
    fn start(upstream: SocketAddr) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let drop_acks = Arc::new(AtomicBool::new(false));
        let dropped = Arc::new(AtomicUsize::new(0));
        let (drop_flag, drop_count) = (drop_acks.clone(), dropped.clone());
        thread::spawn(move || {
            for client in listener.incoming().flatten() {
                let (drop_flag, drop_count) = (drop_flag.clone(), drop_count.clone());
                thread::spawn(move || {
                    let _ = forward(client, upstream, &drop_flag, &drop_count);
                });
            }
        });
        Self {
            base,
            drop_acks,
            dropped,
        }
    }

    fn drop_acks(&self, on: bool) {
        self.drop_acks.store(on, Ordering::SeqCst);
    }

    fn acks_dropped(&self) -> usize {
        self.dropped.load(Ordering::SeqCst)
    }
}

fn forward(
    mut client: TcpStream,
    upstream: SocketAddr,
    drop_acks: &AtomicBool,
    dropped: &AtomicUsize,
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
    if head.starts_with("POST /hook/ack") && drop_acks.load(Ordering::SeqCst) {
        // The ack is lost on the way: the daemon never hears it.
        dropped.fetch_add(1, Ordering::SeqCst);
        return Ok(());
    }
    let mut up = TcpStream::connect(upstream)?;
    up.write_all(head.as_bytes())?;
    up.write_all(&body)?;
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
        // Every send carries a key, so retrying the refusal below can't make a
        // second message.
        let key = key.map_or_else(|| uuid::Uuid::new_v4().to_string(), str::to_string);
        args["idempotency_key"] = json!(key);
        self.retrying_unbound(|mcp| mcp.tool("send_message", args.clone()))
    }

    /// Right after it connects, the shim's connection may not be bound to its
    /// session yet, and the daemon refuses a send with "has no session on the
    /// roster"; the shim re-SETs its session, so retry briefly.
    fn retrying_unbound(
        &mut self,
        mut call: impl FnMut(&mut Self) -> Result<Value, String>,
    ) -> Result<Value, String> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match call(self) {
                Err(e)
                    if e.contains("has no session on the roster") && Instant::now() < deadline =>
                {
                    thread::sleep(Duration::from_millis(250));
                }
                other => return other,
            }
        }
    }

    fn broadcast(&mut self, room: &str, body: &str) -> Result<Value, String> {
        self.retrying_unbound(|mcp| mcp.tool("broadcast", json!({"to_room": room, "body": body})))
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
    /// A busy session reaches its next delivery point (a hook or a turn
    /// boundary): what was injected, if anything.
    fn busy_boundary(&mut self) -> Option<String>;
    fn session_id(&self) -> &str;
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

    fn busy_boundary(&mut self) -> Option<String> {
        let out = self.run_hook("post-tool-use", "post-tool-use");
        let v: Value = serde_json::from_str(out.trim()).ok()?;
        let text = v["hookSpecificOutput"]["additionalContext"].as_str()?;
        (!text.is_empty()).then(|| text.to_string())
    }

    fn session_id(&self) -> &str {
        &self.session_id
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
        let mut mcp = Mcp::start(daemon, sender_id);
        // Don't hand it out until its own row is on the roster.
        let _ = mcp.row(sender_id, Duration::from_secs(15));
        mcp
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
    ("duplicate-shown-at-most-once", duplicate_shown_at_most_once),
    ("daemon-restart-mid-wait", daemon_restart_mid_wait),
    ("operator-pause-stops-wakes", operator_pause_stops_wakes),
    ("broadcast-never-wakes", broadcast_never_wakes),
    ("wake-none-not-expected", wake_none_not_expected),
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

/// Scenario 3: No loss: when the ack of a delivery is lost, the message comes again
/// once its lease lapses, and not after a successful ack.
fn dropped_ack_redelivers(c: &mut Cell) -> Result<(), String> {
    c.proxy.drop_acks(true);
    c.harness.arm_idle();
    let t = token("ack");
    c.send(&t)?;
    let first = c.harness.wait_for_wake(WAKE_WITHIN);
    if !contains(&first, &t) {
        return Err(format!("first delivery missing; got {first:?}"));
    }
    if c.proxy.acks_dropped() == 0 {
        return Err("the harness never acked, so nothing was dropped".into());
    }
    c.proxy.drop_acks(false);
    thread::sleep(Duration::from_secs(ACK_LEASE_SECS + 1));
    c.harness.arm_idle();
    let second = c.harness.wait_for_wake(WAKE_WITHIN);
    if !contains(&second, &t) {
        return Err(format!(
            "not redelivered after the lost ack; got {second:?}"
        ));
    }
    thread::sleep(Duration::from_secs(ACK_LEASE_SECS + 1));
    let third = c.harness.busy_boundary();
    if contains(&third, &t) {
        return Err("delivered a third time after a successful ack".into());
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
    if !contains(&c.harness.wait_for_wake(WAKE_WITHIN), &t2) {
        return Err("the waiter didn't deliver the second message".into());
    }
    if contains(&c.harness.busy_boundary(), &t2) {
        return Err("mail the waiter delivered was injected again by the next hook".into());
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
    // A hook-registered row has pid 0; the session's MCP shim fills it in when
    // it attaches. Until then the roster says `none` for a different reason,
    // and the bug can't show.
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

type Harness = (&'static str, Mode, fn() -> Box<dyn FakeHarness>);

const HARNESSES: &[Harness] = &[
    ("claude", Mode::Plain, || Box::new(FakeClaude::new())),
    ("claude-tls", Mode::Tls, || Box::new(FakeClaude::new())),
];

enum Outcome {
    Pass,
    Fail(String),
    KnownFailure(&'static str),
    UnexpectedPass(&'static str),
}

fn run_cell(harness: &Harness, scenario: Scenario, name: &str) -> Outcome {
    let known = KNOWN_FAILURES
        .iter()
        .find(|(h, s, _)| *h == harness.0 && *s == name)
        .map(|(_, _, story)| *story);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut cell = Cell::new(harness.1, (harness.2)());
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
                .is_some_and(|o| !name.contains(o) && !harness.0.contains(o))
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
            };
            eprintln!(
                "{:<12} {:<30} {:<14} {:.1}s",
                harness.0,
                name,
                cell,
                started.elapsed().as_secs_f32()
            );
            match outcome {
                Outcome::Fail(e) => failures.push(format!("{} / {name}: {e}", harness.0)),
                Outcome::UnexpectedPass(story) => failures.push(format!(
                    "{} / {name} passes, but KNOWN_FAILURES lists it for {story}: remove it",
                    harness.0
                )),
                _ => {}
            }
            grid.insert((harness.0, name), cell);
        }
    }

    // The grid: one row per scenario, one column per harness.
    let mut columns: Vec<&str> = HARNESSES.iter().map(|h| h.0).collect();
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
    eprintln!("{table}");
    if let Ok(path) = std::env::var("CONFORMANCE_GRID") {
        let _ = std::fs::write(path, &table);
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}
