//! The gate: Muster's only network-facing listener.
//!
//! myko has no TLS or authentication, so the daemon binds myko and the hook
//! listener to loopback and puts this in front of them on `MUSTER_BIND`:
//!
//! 1. TLS, with the daemon's self-signed certificate (clients pin it — see
//!    `credentials`).
//! 2. One HTTP request head is read and its token checked: the `token` query
//!    parameter (the shim's WebSocket URL) or `Authorization: Bearer` (the
//!    hooks). A bad or missing token gets `401` and the connection closes.
//! 3. The token is stripped, `/hook/*` is routed to the hook listener and
//!    everything else to myko, and from then on bytes are copied both ways.
//!    WebSocket upgrades and plain request/response work the same way.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsAcceptor;

use futures_util::{SinkExt as _, StreamExt as _};
use tokio_tungstenite::{WebSocketStream, tungstenite::protocol::Role};

use crate::credentials;
use crate::gate_filter::{self, Refused};

/// The WebSocket route the shim uses. It's the only myko route open to paired
/// machines.
const MYKO_WS_PATH: &str = "/myko";

/// Cap on the request head (request line + headers).
const MAX_HEAD: usize = 16 * 1024;
/// Time a client gets to finish the TLS handshake and send its request head.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

pub struct GateConfig {
    /// Public bind (`MUSTER_BIND`).
    pub bind: SocketAddr,
    /// myko's loopback listener (WebSocket + MCP).
    pub myko: SocketAddr,
    /// The hook listener's loopback address.
    pub hooks: SocketAddr,
    /// Where the TLS identity and token hashes live.
    pub state_dir: PathBuf,
}

/// Bind and serve forever. Spawn on a tokio task.
pub async fn run(config: GateConfig) -> std::io::Result<()> {
    let listener = TcpListener::bind(config.bind).await?;
    serve(listener, config).await
}

/// Serve on an already-bound listener (lets tests use an ephemeral port).
pub async fn serve(listener: TcpListener, config: GateConfig) -> std::io::Result<()> {
    let identity = credentials::load_or_create_identity(&config.state_dir)?;
    let acceptor = acceptor(&identity)?;
    log::info!(
        "[gate] TLS on {} (fingerprint {}); paired machines: {}",
        listener.local_addr()?,
        identity.fingerprint(),
        credentials::token_names(&config.state_dir)
            .map(|n| if n.is_empty() {
                "none yet".to_string()
            } else {
                n.join(", ")
            })
            .unwrap_or_else(|e| format!("unreadable ({e})"))
    );
    let config = Arc::new(config);
    loop {
        let (tcp, peer) = match listener.accept().await {
            Ok(pair) => pair,
            Err(e) => {
                log::warn!("[gate] accept error: {e}");
                continue;
            }
        };
        let (acceptor, config) = (acceptor.clone(), config.clone());
        tokio::spawn(async move {
            if let Err(e) = handle(tcp, peer, acceptor, &config).await {
                log::debug!("[gate] {peer}: {e}");
            }
        });
    }
}

fn acceptor(identity: &credentials::Identity) -> std::io::Result<TlsAcceptor> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let server = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(std::io::Error::other)?
        .with_no_client_auth()
        .with_single_cert(
            vec![CertificateDer::from(identity.cert_der.clone())],
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(identity.key_der.clone())),
        )
        .map_err(std::io::Error::other)?;
    Ok(TlsAcceptor::from(Arc::new(server)))
}

async fn handle(
    tcp: TcpStream,
    peer: SocketAddr,
    acceptor: TlsAcceptor,
    config: &GateConfig,
) -> std::io::Result<()> {
    let (mut tls, head, rest) = tokio::time::timeout(HANDSHAKE_TIMEOUT, async {
        let mut tls = acceptor.accept(tcp).await?;
        let (head, rest) = read_head(&mut tls).await?;
        Ok::<_, std::io::Error>((tls, head, rest))
    })
    .await
    .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "handshake timed out"))??;

    let Some(request) = Request::parse(&head) else {
        return respond(&mut tls, "400 Bad Request").await;
    };
    let Some(name) = request
        .token
        .as_deref()
        .and_then(|t| credentials::verify_token(&config.state_dir, t))
    else {
        log::warn!("[gate] rejected {peer}: missing or unknown token");
        return respond(&mut tls, "401 Unauthorized").await;
    };

    let is_hook = request.path().starts_with("/hook/");
    if !is_hook && !(request.path() == MYKO_WS_PATH && request.is_websocket_upgrade()) {
        // myko's other routes (MCP, HTTP) would bypass the frame filter.
        log::warn!("[gate] refused {name}: {}", request.path());
        return respond(&mut tls, "403 Forbidden").await;
    }
    let upstream_addr = if is_hook { config.hooks } else { config.myko };
    log::debug!(
        "[gate] {peer} ({name}) -> {} {}",
        upstream_addr,
        request.path()
    );
    let mut upstream = TcpStream::connect(upstream_addr).await?;
    if is_hook {
        upstream
            .write_all(request.forwarded_head_as(&name).as_bytes())
            .await?;
        upstream.write_all(&rest).await?;
        tokio::io::copy_bidirectional(&mut tls, &mut upstream).await?;
        return Ok(());
    }
    upstream
        .write_all(request.forwarded_head().as_bytes())
        .await?;
    filter_websocket(tls, upstream, &name, &config.state_dir).await
}

/// Finish the WebSocket handshake with myko, then relay frames: everything from
/// myko, but from the client only what `gate_filter` allows.
async fn filter_websocket<S>(
    mut client: S,
    mut upstream: TcpStream,
    machine: &str,
    state_dir: &Path,
) -> std::io::Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let (head, after_head) = read_head(&mut upstream).await?;
    // `read_head` leaves off the blank line that ends the head.
    client
        .write_all(format!("{head}\r\n\r\n").as_bytes())
        .await?;
    client.flush().await?;
    if !head.starts_with("HTTP/1.1 101") {
        return Ok(());
    }

    let client_ws = WebSocketStream::from_raw_socket(client, Role::Server, None).await;
    let upstream_ws =
        WebSocketStream::from_partially_read(upstream, after_head, Role::Client, None).await;
    let (mut client_tx, mut client_rx) = client_ws.split();
    let (mut upstream_tx, mut upstream_rx) = upstream_ws.split();

    let operator = credentials::daemon_operator();
    let from_client = async {
        while let Some(Ok(frame)) = client_rx.next().await {
            match gate_filter::check_frame(state_dir, machine, operator.as_deref(), frame) {
                Ok(frame) => {
                    if upstream_tx.send(frame).await.is_err() {
                        break;
                    }
                }
                Err(Refused(why)) => log::warn!("[gate] refused {machine}: {why}"),
            }
        }
    };
    let from_myko = async {
        while let Some(Ok(frame)) = upstream_rx.next().await {
            if client_tx.send(frame).await.is_err() {
                break;
            }
        }
    };
    tokio::select! {
        () = from_client => {}
        () = from_myko => {}
    }
    Ok(())
}

/// Read up to the end of the request head. Returns the head (without the
/// blank line) and any bytes read past it.
async fn read_head<S: AsyncRead + Unpin>(stream: &mut S) -> std::io::Result<(String, Vec<u8>)> {
    let mut buf = Vec::with_capacity(1024);
    let mut chunk = [0u8; 2048];
    loop {
        if let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let rest = buf.split_off(end + 4);
            buf.truncate(end);
            let head = String::from_utf8(buf).map_err(|_| {
                std::io::Error::new(std::io::ErrorKind::InvalidData, "request head is not UTF-8")
            })?;
            return Ok((head, rest));
        }
        if buf.len() > MAX_HEAD {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "request head too large",
            ));
        }
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        buf.extend_from_slice(&chunk[..n]);
    }
}

async fn respond<S: AsyncWrite + Unpin>(stream: &mut S, status: &str) -> std::io::Result<()> {
    let response = format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    stream.write_all(response.as_bytes()).await?;
    stream.shutdown().await
}

/// The parts of a request head the gate needs.
#[derive(Debug, PartialEq)]
struct Request {
    method: String,
    /// Request target with the `token` parameter removed.
    target: String,
    version: String,
    /// Header lines other than `Authorization`.
    headers: Vec<String>,
    token: Option<String>,
}

impl Request {
    fn parse(head: &str) -> Option<Self> {
        let mut lines = head.split("\r\n");
        let mut parts = lines.next()?.split(' ');
        let (method, raw_target, version) = (parts.next()?, parts.next()?, parts.next()?);
        if parts.next().is_some() || !version.starts_with("HTTP/") {
            return None;
        }
        // The hook listener and myko split the request line on any whitespace.
        // Anything but a plain token here would make them read a different
        // method or target from the one checked below.
        if !method.bytes().all(|b| b.is_ascii_uppercase())
            || !raw_target.bytes().all(|b| b.is_ascii_graphic())
            || !version.bytes().all(|b| b.is_ascii_graphic())
        {
            return None;
        }

        let (path, query) = raw_target.split_once('?').unwrap_or((raw_target, ""));
        let mut token = None;
        let kept: Vec<&str> = query
            .split('&')
            .filter(|p| !p.is_empty())
            .filter(|p| match p.strip_prefix("token=") {
                Some(t) => {
                    token = Some(t.to_string());
                    false
                }
                None => true,
            })
            .collect();
        let target = if kept.is_empty() {
            path.to_string()
        } else {
            format!("{path}?{}", kept.join("&"))
        };

        let mut headers = Vec::new();
        for line in lines {
            match line.split_once(':') {
                Some((k, v)) if k.trim().eq_ignore_ascii_case("authorization") => {
                    if let Some(t) = v.trim().strip_prefix("Bearer ") {
                        token.get_or_insert_with(|| t.trim().to_string());
                    }
                }
                _ => headers.push(line.to_string()),
            }
        }
        Some(Request {
            method: method.to_string(),
            target,
            version: version.to_string(),
            headers,
            token,
        })
    }

    fn is_websocket_upgrade(&self) -> bool {
        self.headers.iter().any(|h| {
            h.split_once(':').is_some_and(|(k, v)| {
                k.trim().eq_ignore_ascii_case("upgrade")
                    && v.trim().eq_ignore_ascii_case("websocket")
            })
        })
    }

    fn path(&self) -> &str {
        self.target.split('?').next().unwrap_or("")
    }

    /// The head for a hook request, with the verified machine name in place of
    /// any `machine` parameter the client sent. The name is restricted by
    /// `credentials::valid_name`, so it needs no escaping.
    fn forwarded_head_as(&self, machine: &str) -> String {
        let (path, query) = self.target.split_once('?').unwrap_or((&self.target, ""));
        let mut kept: Vec<&str> = query
            .split('&')
            .filter(|p| !p.is_empty() && !p.starts_with("machine="))
            .collect();
        let verified = format!("machine={machine}");
        kept.push(&verified);
        let target = format!("{path}?{}", kept.join("&"));
        Request {
            method: self.method.clone(),
            target,
            version: self.version.clone(),
            headers: self.headers.clone(),
            token: None,
        }
        .forwarded_head()
    }

    fn forwarded_head(&self) -> String {
        let mut out = format!("{} {} {}\r\n", self.method, self.target, self.version);
        for h in &self.headers {
            out.push_str(h);
            out.push_str("\r\n");
        }
        out.push_str("\r\n");
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn takes_the_token_from_the_query_and_strips_it() {
        let r = Request::parse(
            "GET /myko?token=abc&tenant=x HTTP/1.1\r\nHost: 127.0.0.1:1\r\nUpgrade: websocket",
        )
        .unwrap();
        assert_eq!(r.token.as_deref(), Some("abc"));
        assert_eq!(r.target, "/myko?tenant=x");
        assert_eq!(
            r.forwarded_head(),
            "GET /myko?tenant=x HTTP/1.1\r\nHost: 127.0.0.1:1\r\nUpgrade: websocket\r\n\r\n"
        );
    }

    #[test]
    fn takes_the_token_from_a_bearer_header_and_drops_the_header() {
        let r = Request::parse(
            "POST /hook/prompt-submit?host=a1 HTTP/1.1\r\nauthorization: Bearer xyz\r\nContent-Length: 2",
        )
        .unwrap();
        assert_eq!(r.token.as_deref(), Some("xyz"));
        assert_eq!(r.path(), "/hook/prompt-submit");
        assert!(
            !r.forwarded_head()
                .to_ascii_lowercase()
                .contains("authorization")
        );
    }

    #[test]
    fn replaces_a_client_supplied_machine_name() {
        let r =
            Request::parse("POST /hook/wait?machine=victim&timeout=1 HTTP/1.1\r\nHost: h").unwrap();
        let head = r.forwarded_head_as("attacker");
        assert!(
            head.starts_with("POST /hook/wait?timeout=1&machine=attacker HTTP/1.1\r\n"),
            "{head}"
        );
    }

    #[test]
    fn whitespace_inside_the_request_line_is_rejected() {
        // Found by review: the listener would end the target at the tab, losing
        // the `machine` the gate appends, or take the target from the method.
        for line in [
            "POST /hook/prompt-submit\tx HTTP/1.1",
            "POST /hook/prompt-submit?a=1\u{0b}x HTTP/1.1",
            "POST /hook/prompt-submit\u{a0}x HTTP/1.1",
            "POST\t/hook/session-register?machine=victim /hook/x HTTP/1.1",
            "post /hook/x HTTP/1.1",
            "POST /hook/x HTTP/1.1\tjunk",
        ] {
            assert!(
                Request::parse(&format!("{line}\r\nHost: h")).is_none(),
                "{line:?}"
            );
        }
    }

    #[test]
    fn no_token_is_none() {
        let r = Request::parse("GET /myko HTTP/1.1\r\nHost: h").unwrap();
        assert_eq!(r.token, None);
    }

    #[test]
    fn rejects_garbage() {
        assert_eq!(Request::parse("hello"), None);
        assert_eq!(Request::parse("GET / SMTP"), None);
    }
}
