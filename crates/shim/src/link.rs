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
//! - The hook subcommands use [`https_post`]: the same pinned TLS, blocking,
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
            Err(rustls::Error::General(
                "the daemon's certificate doesn't match this machine's pairing; \
                 re-pair if the daemon was reinstalled"
                    .into(),
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

/// POST `body` to `path` on the paired gate over pinned TLS and return the
/// response body. `None` on any failure, a non-2xx status included.
pub fn https_post(pairing: &Pairing, path: &str, body: &str, timeout: Duration) -> Option<String> {
    let addr = pairing.address.to_socket_addrs().ok()?.next()?;
    let tcp = TcpStream::connect_timeout(&addr, timeout).ok()?;
    tcp.set_read_timeout(Some(timeout)).ok()?;
    tcp.set_write_timeout(Some(timeout)).ok()?;
    let conn =
        rustls::ClientConnection::new(client_config(&pairing.fingerprint).ok()?, server_name())
            .ok()?;
    let mut tls = rustls::StreamOwned::new(conn, tcp);
    let request = format!(
        "POST {path} HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\n\
         Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        pairing.address,
        pairing.token,
        body.len()
    );
    tls.write_all(request.as_bytes()).ok()?;
    let mut buf = Vec::new();
    // A peer that closes without TLS close_notify ends the read with
    // UnexpectedEof; what arrived before it is still the whole response.
    match tls.read_to_end(&mut buf) {
        Ok(_) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof && !buf.is_empty() => {}
        Err(_) => return None,
    }
    let text = String::from_utf8_lossy(&buf);
    let idx = text.find("\r\n\r\n")?;
    let status = text[..idx].lines().next()?.split_whitespace().nth(1)?;
    if !status.starts_with('2') {
        return None;
    }
    Some(text[idx + 4..].to_string())
}
