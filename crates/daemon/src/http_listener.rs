//! muster's own minimal plain-HTTP listener for the Claude Code
//! `/hook/*` endpoints.
//!
//! Deliberately separate from myko's listener. Stock myko serves
//! `/myko/mcp` (WS + HTTP MCP) on its own port and we do **not** fork it
//! to bolt on extra routes. The hooks need a dumb plain-HTTP endpoint a
//! curl one-liner POSTs raw hook JSON to, so muster binds a second port
//! and shares the server's `Arc<MykoServerContext>` — same registry, same
//! event log, no second source of truth.
//!
//! The HTTP handling is intentionally tiny: HTTP/1.1, `Content-Length`
//! bodies only (curl `--data-binary @-` always sends one), one request
//! per connection, `Connection: close`. Hook bodies are a few hundred
//! bytes; anything fancier would be a dependency we don't need.

use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use myko::server::MykoServerContext;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

use crate::{credentials, hooks};

/// Hard cap on a hook request body. Claude Code hook payloads are small;
/// this only exists so a malformed/hostile `Content-Length` can't make us
/// allocate unboundedly.
const MAX_BODY: usize = 256 * 1024;

/// `/hook/wait` bounds: how long one long-poll is held when the caller names
/// no `timeout`, the most it may ask for, and how often the inbox is checked.
const WAIT_DEFAULT_SECS: u64 = 240;
const WAIT_MAX_SECS: u64 = 600;
const WAIT_POLL: Duration = Duration::from_millis(500);

/// Run the hook listener forever. Spawn on a tokio task.
pub async fn run(bind: SocketAddr, ctx: Arc<MykoServerContext>) -> std::io::Result<()> {
    run_bound(bind, ctx, None).await
}

/// [`run`], refusing requests the gate marked as coming from a machine that
/// doesn't own the session they name. The session bindings live in `state_dir`.
/// Requests from the daemon's own host carry no machine and aren't checked.
pub async fn run_bound(
    bind: SocketAddr,
    ctx: Arc<MykoServerContext>,
    state_dir: Option<Arc<PathBuf>>,
) -> std::io::Result<()> {
    let listener = TcpListener::bind(bind).await?;
    log::info!("muster hook listener on http://{bind} (/hook/*)");
    loop {
        let (stream, _peer) = match listener.accept().await {
            Ok(pair) => pair,
            Err(e) => {
                log::warn!("[hook] accept error: {e}");
                continue;
            }
        };
        let ctx = ctx.clone();
        let state_dir = state_dir.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_conn(stream, ctx, state_dir).await {
                log::debug!("[hook] connection error: {e}");
            }
        });
    }
}

async fn handle_conn(
    mut stream: TcpStream,
    ctx: Arc<MykoServerContext>,
    state_dir: Option<Arc<PathBuf>>,
) -> std::io::Result<()> {
    let mut buf: Vec<u8> = Vec::with_capacity(2048);
    let mut tmp = [0u8; 2048];

    // Read until the end of headers.
    let header_end = loop {
        if let Some(pos) = find(&buf, b"\r\n\r\n") {
            break pos;
        }
        let n = stream.read(&mut tmp).await?;
        if n == 0 {
            // Connection closed before a complete header block.
            return Ok(());
        }
        buf.extend_from_slice(&tmp[..n]);
        if buf.len() > MAX_BODY + 8192 {
            return write_response(&mut stream, 413, "payload too large").await;
        }
    };

    let head = String::from_utf8_lossy(&buf[..header_end]).into_owned();
    let mut lines = head.split("\r\n");
    let request_line = lines.next().unwrap_or("");
    let mut rl = request_line.split_whitespace();
    let method = rl.next().unwrap_or("");
    let target = rl.next().unwrap_or("");

    let content_length = lines
        .filter_map(|l| l.split_once(':'))
        .find(|(k, _)| k.trim().eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.trim().parse::<usize>().ok())
        .unwrap_or(0)
        .min(MAX_BODY);

    // Body bytes already buffered past the header terminator, plus any
    // remainder up to Content-Length.
    let body_start = header_end + 4;
    let mut body: Vec<u8> = buf
        .get(body_start..)
        .map(|s| s.to_vec())
        .unwrap_or_default();
    while body.len() < content_length {
        let n = stream.read(&mut tmp).await?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&tmp[..n]);
    }
    body.truncate(content_length);

    let (path, query) = match target.split_once('?') {
        Some((p, q)) => (p, q),
        None => (target, ""),
    };

    if method != "POST" {
        return write_response(&mut stream, 405, "method not allowed").await;
    }

    if let Some(dir) = &state_dir
        && !machine_may_act(dir, query, &body)
    {
        return write_response(&mut stream, 403, "forbidden").await;
    }

    if path == "/hook/wait" {
        return handle_wait(stream, query, &body, &ctx).await;
    }

    match hooks::dispatch(path, query, &body, &ctx) {
        Some(outcome) => {
            write_response(&mut stream, 200, &outcome.body).await?;
            // At-least-once inbox: ack the surfaced messages ONLY after the
            // response was successfully written + flushed above. If that write
            // failed (curl gone / `--max-time` elapsed), the `?` bailed and we
            // never reach here, so the messages stay unread and re-surface next
            // turn — never acked-but-unseen.
            if let Some((session, ids)) = outcome.deferred_ack {
                hooks::ack_surfaced(&ctx, &session, ids);
            }
            Ok(())
        }
        None => write_response(&mut stream, 404, "not found").await,
    }
}

/// Whether the machine the gate named in `query` may act for the session in
/// `body`. Without a session id the handlers have nothing to act on. Without a
/// machine the request came from the daemon's own host, which is always allowed
/// and claims the session, so a paired machine can't take it by getting there
/// first.
fn machine_may_act(state_dir: &Path, query: &str, body: &[u8]) -> bool {
    let session = serde_json::from_slice::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v.get("session_id")?.as_str().map(str::to_string));
    let Some(session) = session else {
        return true;
    };
    let Some(machine) = query.split('&').find_map(|kv| kv.strip_prefix("machine=")) else {
        if let Err(e) = credentials::claim_session(state_dir, credentials::LOCAL_MACHINE, &session)
        {
            log::warn!("[hook] can't record local session {session}: {e}");
        }
        return true;
    };
    match credentials::claim_session(state_dir, machine, &session) {
        Ok(true) => true,
        Ok(false) => {
            log::warn!("[hook] refused {machine}: session {session} isn't its own");
            false
        }
        Err(e) => {
            log::warn!("[hook] refused {machine}: can't read session bindings: {e}");
            false
        }
    }
}

/// `/hook/wait`: hold the request until the session has an unread direct
/// message (respond with it, then ack) or `timeout` seconds pass (respond
/// empty). The caller sends nothing after its request, so the socket turning
/// readable means it went away — stop then, before a message is written into
/// a dead connection and acked as delivered.
async fn handle_wait(
    mut stream: TcpStream,
    query: &str,
    body: &[u8],
    ctx: &Arc<MykoServerContext>,
) -> std::io::Result<()> {
    let Some((sid, cmd_ctx)) = hooks::begin_wait(query, body, ctx) else {
        return write_response(&mut stream, 200, "").await;
    };
    let secs = query
        .split('&')
        .find_map(|kv| kv.strip_prefix("timeout="))
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(WAIT_DEFAULT_SECS)
        .min(WAIT_MAX_SECS);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(secs);
    let mut probe = [0u8; 64];
    loop {
        if let Some(outcome) = hooks::poll_wait(&cmd_ctx, &sid) {
            write_response(&mut stream, 200, &outcome.body).await?;
            if let Some((session, ids)) = outcome.deferred_ack {
                hooks::ack_surfaced(ctx, &session, ids);
            }
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            return write_response(&mut stream, 200, "").await;
        }
        tokio::select! {
            _ = tokio::time::sleep(WAIT_POLL) => {}
            _ = stream.read(&mut probe) => return Ok(()),
        }
    }
}

async fn write_response(stream: &mut TcpStream, status: u16, body: &str) -> std::io::Result<()> {
    let reason = match status {
        200 => "OK",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        413 => "Payload Too Large",
        _ => "OK",
    };
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\n\
         Content-Type: text/plain; charset=utf-8\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\r\n",
        body.len(),
    );
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(body.as_bytes()).await?;
    stream.flush().await?;
    let _ = stream.shutdown().await;
    Ok(())
}

/// First index of `needle` in `haystack`, or `None`.
fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}
