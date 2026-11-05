//! The shim's side of the daemon gate: the stored pairing, TLS with the
//! daemon's certificate pinned, and the local tunnel myko's client rides.
//!
//! `muster-shim pair <code>` stores the pairing (the gate address, the
//! certificate fingerprint and the access token) in the config dir. When
//! one is present:
//!
//! - The MCP server starts [`start_tunnel`]: a loopback listener whose every
//!   connection is carried over pinned TLS to the gate. myko's client only
//!   trusts certificates the OS trusts, so it dials the tunnel with plain
//!   `ws://127.0.0.1:<port>/myko?token=…` and the tunnel adds the TLS. The
//!   token never leaves the machine unencrypted.
//! - The hook subcommands use [`https_post_why`]: the same pinned TLS, blocking,
//!   with the token in an `Authorization: Bearer` header.

use std::io::{self, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use muster_entities::pairing::{Pairing, cert_fingerprint};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, verify_tls12_signature, verify_tls13_signature};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};

const PAIRING_FILE: &str = "pairing";

/// SNI sent to the gate. The certificate is pinned by fingerprint, so the
/// name is never checked; it only has to be a valid DNS name.
const SERVER_NAME: &str = "muster-daemon";

/// The stored pairing, if this machine has one.
pub fn load_pairing() -> Option<Pairing> {
    crate::config_file_candidates(PAIRING_FILE)
        .into_iter()
        .find_map(|path| {
            let code = std::fs::read_to_string(&path).ok()?;
            match Pairing::from_code(&code) {
                Ok(p) => Some(p),
                Err(e) => {
                    log::warn!("[muster-shim] ignoring {}: {e}", path.display());
                    None
                }
            }
        })
}

/// Store `pairing` (owner-only) and return where it went.
pub fn save_pairing(pairing: &Pairing) -> io::Result<PathBuf> {
    let path = crate::config_file_candidates(PAIRING_FILE)
        .into_iter()
        .next()
        .ok_or_else(|| io::Error::other("no config directory (is HOME set?)"))?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    options
        .open(&path)?
        .write_all(format!("{}\n", pairing.to_code()).as_bytes())?;
    Ok(path)
}

/// Delete every stored pairing. Returns whether there was one.
pub fn remove_pairing() -> io::Result<bool> {
    let mut removed = false;
    for path in crate::config_file_candidates(PAIRING_FILE) {
        match std::fs::remove_file(&path) {
            Ok(()) => removed = true,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
    }
    Ok(removed)
}

/// A rustls client config that accepts exactly the certificate whose
/// SHA-256 is `fingerprint`, and still checks the handshake signatures.
pub fn client_config(fingerprint: &str) -> io::Result<Arc<rustls::ClientConfig>> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let verifier = Arc::new(PinnedCert {
        fingerprint: fingerprint.to_ascii_lowercase(),
        provider: provider.clone(),
    });
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(io::Error::other)?
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth();
    Ok(Arc::new(config))
}

#[derive(Debug)]
struct PinnedCert {
    fingerprint: String,
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for PinnedCert {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        if cert_fingerprint(end_entity) == self.fingerprint {
            Ok(ServerCertVerified::assertion())
        } else {
            // Named by `PostFailure::Certificate` when it reaches a caller.
            Err(rustls::Error::InvalidCertificate(
                rustls::CertificateError::ApplicationVerificationFailure,
            ))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

fn server_name() -> ServerName<'static> {
    ServerName::try_from(SERVER_NAME).expect("valid DNS name")
}

/// Start the loopback tunnel to `pairing`'s gate and return its local port.
/// Each local connection gets its own TLS connection to the gate.
pub async fn start_tunnel(pairing: &Pairing) -> io::Result<u16> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    let connector = tokio_rustls::TlsConnector::from(client_config(&pairing.fingerprint)?);
    let gate = pairing.address.clone();
    tokio::spawn(async move {
        loop {
            let (mut local, _) = match listener.accept().await {
                Ok(pair) => pair,
                Err(e) => {
                    log::warn!("[muster-shim] tunnel accept error: {e}");
                    continue;
                }
            };
            let (connector, gate) = (connector.clone(), gate.clone());
            tokio::spawn(async move {
                let tcp = match tokio::net::TcpStream::connect(&gate).await {
                    Ok(tcp) => tcp,
                    Err(e) => {
                        log::warn!("[muster-shim] can't reach the gate at {gate}: {e}");
                        return;
                    }
                };
                let _ = tcp.set_nodelay(true);
                let mut remote = match connector.connect(server_name(), tcp).await {
                    Ok(tls) => tls,
                    Err(e) => {
                        log::warn!("[muster-shim] TLS to {gate} failed: {e}");
                        return;
                    }
                };
                let _ = tokio::io::copy_bidirectional(&mut local, &mut remote).await;
            });
        }
    });
    Ok(port)
}

/// Connect to the first address of `target` that accepts. A name like
/// `localhost` can resolve to `::1` ahead of the `127.0.0.1` the daemon binds,
/// so stopping at the first address would be refused forever. Each attempt is
/// capped at [`CONNECT_ATTEMPT`], however long the caller's request may run.
pub fn connect_any(target: impl ToSocketAddrs, timeout: Duration) -> Option<TcpStream> {
    let attempt = timeout.min(CONNECT_ATTEMPT);
    target
        .to_socket_addrs()
        .ok()?
        .find_map(|addr| TcpStream::connect_timeout(&addr, attempt).ok())
}

const CONNECT_ATTEMPT: Duration = Duration::from_secs(5);

/// Why a POST to the daemon got no usable answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PostFailure {
    /// Nothing answered: the daemon is down, or this machine is asleep or off
    /// the network.
    Unreachable,
    /// The daemon's certificate isn't the one this machine paired with.
    Certificate,
    /// The paired address answered, but not with TLS: the daemon there runs
    /// plain (MUS-64), or the address now belongs to something else.
    NotTls,
    /// The daemon answered with this non-2xx status.
    Status(u16),
}

impl PostFailure {
    /// Whether the daemon, not the network, said no, so retrying won't help
    /// until someone changes something.
    pub fn is_refusal(self) -> bool {
        matches!(
            self,
            Self::Certificate | Self::NotTls | Self::Status(401 | 403 | 426)
        )
    }
}

impl std::fmt::Display for PostFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unreachable => write!(f, "can't reach the daemon (down, or no network)"),
            Self::Certificate => write!(
                f,
                "the daemon's certificate doesn't match this machine's pairing; pair again if the daemon was reset"
            ),
            Self::Status(401) => write!(
                f,
                "the daemon refused this machine's token (401); it may need pairing again"
            ),
            Self::Status(403) => write!(
                f,
                "the daemon refused this request (403); this machine doesn't own that session"
            ),
            Self::NotTls => write!(
                f,
                "this machine is paired, but the daemon at the paired address isn't serving TLS \
                 (it accepted the connection and never answered TLS): turn TLS back on there \
                 (MUSTER_TLS=1), or remove the pairing with `muster-shim unpair`"
            ),
            Self::Status(426) => write!(
                f,
                "the daemon requires pairing (426): run `muster-daemon pair --name <machine>` on its \
                 host, then `muster-shim pair <code>` here"
            ),
            Self::Status(code) => write!(f, "the daemon answered with an error ({code})"),
        }
    }
}

/// A successful response: its body, and the message ids the daemon wants acked
/// once they've been handed on.
#[derive(Debug, PartialEq, Eq)]
pub struct Reply {
    pub body: String,
    pub ack_ids: Vec<String>,
    /// The daemon ended this wait because it's stopping, so a retry will
    /// briefly find it unreachable.
    pub restarting: bool,
}

/// A complete HTTP response, or why there isn't a usable one.
pub fn parse_response(buf: &[u8]) -> Result<Reply, PostFailure> {
    let text = String::from_utf8_lossy(buf);
    let idx = text.find("\r\n\r\n").ok_or(PostFailure::Unreachable)?;
    let status = text[..idx]
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse::<u16>().ok())
        .ok_or(PostFailure::Unreachable)?;
    if !(200..300).contains(&status) {
        return Err(PostFailure::Status(status));
    }
    let ack_ids = text[..idx]
        .lines()
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.eq_ignore_ascii_case("x-muster-ack"))
        .map(|(_, ids)| {
            ids.split(',')
                .map(|id| id.trim().to_string())
                .filter(|id| !id.is_empty())
                .collect()
        })
        .unwrap_or_default();
    let restarting = text[..idx]
        .lines()
        .filter_map(|line| line.split_once(':'))
        .any(|(name, _)| name.eq_ignore_ascii_case("x-muster-restarting"));
    Ok(Reply {
        body: text[idx + 4..].to_string(),
        ack_ids,
        restarting,
    })
}

/// How long the gate gets to answer a TLS handshake.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// Why a handshake failed. A connection that was accepted and then never
/// answered the ClientHello is not a gate.
fn handshake_failure(e: &io::Error) -> PostFailure {
    match e.kind() {
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => PostFailure::NotTls,
        _ => tls_failure(e),
    }
}

/// rustls reports a certificate it rejected, or a reply that isn't TLS at all,
/// as an I/O error wrapping its own.
fn tls_failure(e: &io::Error) -> PostFailure {
    match e
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<rustls::Error>())
    {
        Some(rustls::Error::InvalidCertificate(_)) => PostFailure::Certificate,
        Some(rustls::Error::InvalidMessage(_)) => PostFailure::NotTls,
        _ => PostFailure::Unreachable,
    }
}

/// Exchange a one-time invite at the gate for this machine's pairing (MUS-8).
/// The gate pins the same certificate the pairing will.
pub fn redeem_invite(invite: muster_entities::pairing::Invite) -> anyhow::Result<Pairing> {
    let probe = invite.clone().into_pairing(invite.invite.clone());
    let reply = match https_post_why(&probe, "/pair", "{}", Duration::from_secs(15)) {
        Ok(reply) => reply,
        Err(PostFailure::Status(401)) => anyhow::bail!(
            "the daemon refused this code: it was already used or has expired. \
             Ask for a new one (`muster-daemon pair --name {}`)",
            invite.name
        ),
        Err(failure) => anyhow::bail!("couldn't pair with {}: {failure}", invite.address),
    };
    #[derive(serde::Deserialize)]
    struct Paired {
        name: String,
        token: String,
    }
    let paired: Paired = serde_json::from_str(&reply.body)
        .map_err(|e| anyhow::anyhow!("the daemon's reply to pairing wasn't understood: {e}"))?;
    let mut pairing = invite.into_pairing(paired.token);
    pairing.name = paired.name;
    Ok(pairing)
}

/// POST `body` to `path` on the paired gate over pinned TLS, saying why it
/// failed if it got no usable answer.
pub fn https_post_why(
    pairing: &Pairing,
    path: &str,
    body: &str,
    timeout: Duration,
) -> Result<Reply, PostFailure> {
    let unreachable = |_| PostFailure::Unreachable;
    let mut tcp = connect_any(pairing.address.as_str(), timeout).ok_or(PostFailure::Unreachable)?;
    // The handshake gets its own short limit: a held wait may run for minutes,
    // but a gate answers a ClientHello at once. A daemon running plain (MUS-64)
    // never answers it at all; it waits for an HTTP head that never comes.
    let handshake = timeout.min(HANDSHAKE_TIMEOUT);
    tcp.set_read_timeout(Some(handshake)).map_err(unreachable)?;
    tcp.set_write_timeout(Some(handshake))
        .map_err(unreachable)?;
    let config = client_config(&pairing.fingerprint).map_err(unreachable)?;
    let mut conn = rustls::ClientConnection::new(config, server_name())
        .map_err(|_| PostFailure::Unreachable)?;
    while conn.is_handshaking() {
        conn.complete_io(&mut tcp)
            .map_err(|e| handshake_failure(&e))?;
    }
    tcp.set_read_timeout(Some(timeout)).map_err(unreachable)?;
    tcp.set_write_timeout(Some(timeout)).map_err(unreachable)?;
    let mut tls = rustls::StreamOwned::new(conn, tcp);
    let request = format!(
        "POST {path} HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\n\
         Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        pairing.address,
        pairing.token,
        body.len()
    );
    tls.write_all(request.as_bytes())
        .map_err(|e| tls_failure(&e))?;
    let mut buf = Vec::new();
    // A peer that closes without TLS close_notify ends the read with
    // UnexpectedEof; what arrived before it is still the whole response.
    match tls.read_to_end(&mut buf) {
        Ok(_) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof && !buf.is_empty() => {}
        Err(e) => return Err(tls_failure(&e)),
    }
    parse_response(&buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{SocketAddr, TcpListener};

    /// A loopback address that refuses connections, held for as long as the
    /// returned socket lives. It's bound but not listening, so the port can't be
    /// handed to another test running in parallel, as a dropped listener's can.
    fn refusing_address() -> (socket2::Socket, SocketAddr) {
        use socket2::{Domain, Socket, Type};
        let socket = Socket::new(Domain::IPV4, Type::STREAM, None).unwrap();
        socket
            .bind(&SocketAddr::from(([127, 0, 0, 1], 0)).into())
            .unwrap();
        let addr = socket.local_addr().unwrap().as_socket().unwrap();
        (socket, addr)
    }

    #[test]
    fn a_response_says_why_it_failed() {
        assert_eq!(
            parse_response(b"HTTP/1.1 200 OK\r\nX-Muster-Ack: a, b\r\n\r\nbody"),
            Ok(Reply {
                body: "body".to_string(),
                ack_ids: vec!["a".to_string(), "b".to_string()],
                restarting: false,
            })
        );
        let stopping =
            parse_response(b"HTTP/1.1 200 OK\r\nX-Muster-Restarting: 1\r\n\r\n").unwrap();
        assert!(stopping.restarting && stopping.body.is_empty());
        for code in [401, 403] {
            let raw = format!("HTTP/1.1 {code} No\r\n\r\n");
            let failure = parse_response(raw.as_bytes()).unwrap_err();
            assert_eq!(failure, PostFailure::Status(code));
            assert!(failure.is_refusal());
        }
        let outage = parse_response(b"HTTP/1.1 502 Bad Gateway\r\n\r\n").unwrap_err();
        assert!(!outage.is_refusal());
        assert_eq!(parse_response(b""), Err(PostFailure::Unreachable));
    }

    #[test]
    fn connect_any_skips_an_address_that_refuses() {
        let (_held, refused) = refusing_address();
        let live = TcpListener::bind("127.0.0.1:0").unwrap();
        let live_addr = live.local_addr().unwrap();

        let stream = connect_any(&[refused, live_addr][..], Duration::from_secs(1))
            .expect("should fall through to the address that accepts");
        assert_eq!(stream.peer_addr().unwrap(), live_addr);
    }

    #[test]
    fn a_listener_that_never_answers_tls_is_not_a_gate() {
        let plain = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = plain.local_addr().unwrap();
        // Accepts, reads, and sends nothing back, like myko waiting for a head.
        let held = std::thread::spawn(move || {
            let (mut s, _) = plain.accept().unwrap();
            let mut buf = [0u8; 4096];
            while matches!(s.read(&mut buf), Ok(n) if n > 0) {}
        });
        let pairing = Pairing {
            address: addr.to_string(),
            fingerprint: "0".repeat(64),
            token: "t".into(),
            name: "x".into(),
        };
        let failure = https_post_why(&pairing, "/hook/wait", "{}", Duration::from_secs(1));
        assert_eq!(failure, Err(PostFailure::NotTls));
        assert!(PostFailure::NotTls.is_refusal());
        drop(held);
    }

    #[test]
    fn connect_any_is_none_when_every_address_refuses() {
        let (_held, refused) = refusing_address();
        assert!(connect_any(&[refused][..], Duration::from_secs(1)).is_none());
    }
}
