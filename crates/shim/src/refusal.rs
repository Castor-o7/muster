//! Say when the daemon refuses this shim's credential.
//!
//! myko's client reconnects quietly: a gate that answers the WebSocket
//! handshake with `401` only leaves the connection `Disconnected`, so a tool
//! call used to wait out its whole timeout and report "daemon unresponsive?".
//! While the shim is disconnected, [`probe`] repeats the handshake once. If the
//! daemon answers `401`, the refusal is recorded, naming the file the
//! credential comes from, and tool calls fail at once with that message
//! instead. A successful connection clears it.

use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// Where this shim's credential for the daemon comes from.
#[derive(Clone, Debug)]
pub enum Credential {
    /// The host's local token, as the daemon on this host wrote it.
    LocalToken(Option<PathBuf>),
    /// A pairing (`marshal-shim pair`), whose token the daemon holds.
    Pairing(Option<PathBuf>),
}

static REFUSED: Mutex<Option<String>> = Mutex::new(None);
static PROBING: AtomicBool = AtomicBool::new(false);

/// How long a probe's handshake may take.
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// The recorded refusal, if the daemon's last answer was a refusal.
pub fn current() -> Option<String> {
    REFUSED.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

fn set(message: Option<String>) {
    *REFUSED.lock().unwrap_or_else(|e| e.into_inner()) = message;
}

/// The connection is up, so whatever was refused before isn't any more.
pub fn clear() {
    set(None);
}

/// The message for a `401` from the daemon at `address` (shown without its
/// query, which carries the token).
pub fn message(credential: &Credential, address: &str) -> String {
    let address = address.split('?').next().unwrap_or_default();
    let shown = |path: &Option<PathBuf>, fallback: &str| {
        path.as_ref()
            .map_or_else(|| fallback.to_string(), |p| p.display().to_string())
    };
    match credential {
        Credential::LocalToken(path) => {
            let file = shown(
                path,
                "the local token file (no config directory: is HOME set?)",
            );
            let state = match path {
                Some(p) if p.exists() => "doesn't match the running daemon's",
                _ => "is missing",
            };
            format!(
                "the daemon at {address} refused this shim's local token (401): {file} {state}. \
                 The daemon writes that file when it starts; check that this session's \
                 XDG_CONFIG_HOME/HOME point at the same config directory as the daemon's, \
                 then restart the session"
            )
        }
        Credential::Pairing(path) => {
            let file = shown(path, "this machine's pairing file");
            format!(
                "the paired daemon refused this machine's token (401): the pairing in {file} \
                 was revoked or belongs to another daemon. Pair again with a fresh code \
                 (`marshal-daemon pair` on the daemon's host, then `marshal-shim pair <code>`), \
                 then restart the session"
            )
        }
    }
}

/// Repeat the WebSocket handshake to `url` once, and record a refusal if the
/// daemon answers `401`. Only one probe runs at a time; any other outcome
/// (connection refused, a timeout, success) leaves things as they were, and
/// the old timeout message stands for a daemon that's simply down.
pub async fn probe(url: String, credential: Credential) {
    if PROBING.swap(true, Ordering::SeqCst) {
        return;
    }
    let outcome = tokio::time::timeout(
        PROBE_TIMEOUT,
        tokio_tungstenite::connect_async(url.as_str()),
    )
    .await;
    match outcome {
        Ok(Err(tokio_tungstenite::tungstenite::Error::Http(response)))
            if response.status().as_u16() == 401 =>
        {
            let text = message(&credential, &url);
            log::warn!("[marshal-shim] {text}");
            set(Some(text));
        }
        Ok(Ok((mut socket, _))) => {
            let _ = socket.close(None).await;
        }
        _ => {}
    }
    PROBING.store(false, Ordering::SeqCst);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_local_token_message_names_the_file_and_hides_the_token() {
        let dir = std::env::temp_dir().join(format!("marshal-refusal-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("local-token");
        let missing = message(
            &Credential::LocalToken(Some(path.clone())),
            "ws://127.0.0.1:6155?token=s3cret",
        );
        assert!(
            missing.contains("refused this shim's local token (401)"),
            "{missing}"
        );
        assert!(missing.contains(&path.display().to_string()), "{missing}");
        assert!(missing.contains("is missing"), "{missing}");
        assert!(!missing.contains("s3cret"), "{missing}");
        std::fs::write(&path, "old\n").unwrap();
        let stale = message(
            &Credential::LocalToken(Some(path.clone())),
            "ws://127.0.0.1:6155",
        );
        assert!(
            stale.contains("doesn't match the running daemon's"),
            "{stale}"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_pairing_message_says_to_pair_again() {
        let text = message(
            &Credential::Pairing(Some(PathBuf::from("/x/marshal/pairing"))),
            "ws://127.0.0.1:40000/myko?token=s3cret",
        );
        assert!(
            text.contains("/x/marshal/pairing") && text.contains("marshal-shim pair"),
            "{text}"
        );
        assert!(!text.contains("s3cret"), "{text}");
    }
}
