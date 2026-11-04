//! The pairing code that connects a machine to a Muster daemon.
//!
//! `muster-daemon pair` prints one; `muster-shim pair <code>` stores it. The
//! code carries everything a client needs to reach the daemon's TLS gate:
//! where it is, which certificate to expect (the daemon's certificate is
//! self-signed, so the client pins its SHA-256 fingerprint instead of
//! trusting a CA), and the access token the gate checks.
//!
//! Format: `muster-pair:v1:` followed by the base64url (no padding) of a JSON
//! object. Shared by the daemon and the shim so the two can't drift.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const PREFIX: &str = "muster-pair:v1:";

/// One machine's credentials for one daemon.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pairing {
    /// The gate's `host:port`, as this machine should dial it.
    pub address: String,
    /// Lowercase hex SHA-256 of the daemon certificate's DER encoding.
    pub fingerprint: String,
    /// The access token, presented to the gate on every connection.
    pub token: String,
    /// The name this machine was paired under (what `unpair` takes).
    pub name: String,
}

#[derive(Debug, PartialEq, Eq)]
pub enum PairingError {
    NotAPairingCode,
    Malformed,
}

impl std::fmt::Display for PairingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PairingError::NotAPairingCode => {
                write!(f, "not a Muster pairing code (expected `{PREFIX}…`)")
            }
            PairingError::Malformed => write!(f, "the pairing code is damaged or incomplete"),
        }
    }
}

impl std::error::Error for PairingError {}

impl Pairing {
    pub fn to_code(&self) -> String {
        let json = serde_json::to_vec(self).expect("Pairing serializes");
        format!("{PREFIX}{}", URL_SAFE_NO_PAD.encode(json))
    }

    pub fn from_code(code: &str) -> Result<Self, PairingError> {
        let payload = code
            .trim()
            .strip_prefix(PREFIX)
            .ok_or(PairingError::NotAPairingCode)?;
        let json = URL_SAFE_NO_PAD
            .decode(payload)
            .map_err(|_| PairingError::Malformed)?;
        let pairing: Pairing =
            serde_json::from_slice(&json).map_err(|_| PairingError::Malformed)?;
        let valid = !pairing.address.is_empty()
            && !pairing.token.is_empty()
            && pairing.fingerprint.len() == 64
            && pairing.fingerprint.bytes().all(|b| b.is_ascii_hexdigit());
        if !valid {
            return Err(PairingError::Malformed);
        }
        Ok(Pairing {
            fingerprint: pairing.fingerprint.to_ascii_lowercase(),
            ..pairing
        })
    }
}

/// Lowercase hex SHA-256 of a certificate's DER bytes — what a pairing pins.
pub fn cert_fingerprint(der: &[u8]) -> String {
    hex(&Sha256::digest(der))
}

/// Lowercase hex SHA-256 of a token — what the daemon stores instead of the
/// token itself, so its token file can't be used to connect.
pub fn token_hash(token: &str) -> String {
    hex(&Sha256::digest(token.as_bytes()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Where this user's muster config files live: `$XDG_CONFIG_HOME/muster`, else
/// `~/.config/muster` (`%APPDATA%\muster` on Windows). The shim's `pairing`
/// file and the daemon's `local-token` are both here.
pub fn config_dir() -> Option<std::path::PathBuf> {
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME").filter(|s| !s.is_empty()) {
        return Some(std::path::PathBuf::from(xdg).join("muster"));
    }
    if cfg!(windows)
        && let Some(appdata) = std::env::var_os("APPDATA").filter(|s| !s.is_empty())
    {
        return Some(std::path::PathBuf::from(appdata).join("muster"));
    }
    let home = std::env::var_os("HOME").filter(|s| !s.is_empty())?;
    Some(
        std::path::PathBuf::from(home)
            .join(".config")
            .join("muster"),
    )
}

/// The file the daemon writes its local token to, owner-only (MUS-7).
pub const LOCAL_TOKEN_FILE: &str = "local-token";

/// The local token, if a daemon on this host has written one. Processes on the
/// daemon's host present it to the hook listener and the local gate; a process
/// that can't read the operator's config directory has nothing to present.
pub fn read_local_token() -> Option<String> {
    let path = config_dir()?.join(LOCAL_TOKEN_FILE);
    let text = std::fs::read_to_string(path).ok()?;
    let token = text.lines().next()?.trim();
    (!token.is_empty()).then(|| token.to_string())
}

/// Mask pairing codes and bearer tokens in `text` (MUS-57): everything after
/// `muster-pair:v1:` or `Bearer ` up to the next character that can't be part
/// of a code or token becomes `<redacted>`. Both binaries run their log lines
/// through this, and the daemon runs message bodies through it, so a secret
/// pasted by mistake never reaches a transcript or the model API.
pub fn redact(text: &str) -> std::borrow::Cow<'_, str> {
    const MARKERS: [&str; 2] = [PREFIX, "Bearer "];
    if !MARKERS.iter().any(|m| text.contains(m)) {
        return std::borrow::Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    loop {
        let next = MARKERS
            .iter()
            .filter_map(|m| rest.find(m).map(|i| (i, *m)))
            .min_by_key(|(i, _)| *i);
        let Some((idx, marker)) = next else {
            out.push_str(rest);
            return std::borrow::Cow::Owned(out);
        };
        out.push_str(&rest[..idx + marker.len()]);
        let after = &rest[idx + marker.len()..];
        let secret = after
            .bytes()
            .take_while(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'=' | b'.'))
            .count();
        if secret > 0 {
            out.push_str("<redacted>");
        }
        rest = &after[secret..];
    }
}

impl Pairing {
    /// The pairing with its token masked, for showing to a person or a session.
    pub fn masked(&self) -> String {
        format!(
            "address: {}\nname: {}\nfingerprint: {}\ntoken: **** (masked; see ~/.config/muster/pairing)",
            self.address, self.name, self.fingerprint
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Pairing {
        Pairing {
            address: "192.0.2.10:6255".into(),
            fingerprint: "ab".repeat(32),
            token: "tok_abc-DEF_123".into(),
            name: "a1".into(),
        }
    }

    #[test]
    fn round_trips() {
        let p = sample();
        assert_eq!(Pairing::from_code(&p.to_code()), Ok(p));
    }

    #[test]
    fn tolerates_surrounding_whitespace_and_uppercase_fingerprints() {
        let mut p = sample();
        p.fingerprint = "AB".repeat(32);
        let parsed = Pairing::from_code(&format!("  {}\n", p.to_code())).unwrap();
        assert_eq!(parsed.fingerprint, "ab".repeat(32));
    }

    #[test]
    fn rejects_other_strings() {
        assert_eq!(
            Pairing::from_code("ws://host:6255"),
            Err(PairingError::NotAPairingCode)
        );
        assert_eq!(
            Pairing::from_code("muster-pair:v1:not-base64!"),
            Err(PairingError::Malformed)
        );
        let mut p = sample();
        p.fingerprint = "short".into();
        assert_eq!(
            Pairing::from_code(&p.to_code()),
            Err(PairingError::Malformed)
        );
    }

    #[test]
    fn hashes_are_hex_sha256() {
        assert_eq!(
            token_hash("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(cert_fingerprint(b"abc").len(), 64);
    }

    #[test]
    fn redacts_a_pairing_code_in_a_log_line() {
        let line = "[muster-shim] ignoring /x/pairing: muster-pair:v1:eyJhIjoiYiJ9 (bad)";
        assert_eq!(
            redact(line),
            "[muster-shim] ignoring /x/pairing: muster-pair:v1:<redacted> (bad)"
        );
    }

    #[test]
    fn redacts_a_bearer_token_in_a_message_body() {
        let body = "try Authorization: Bearer abc-DEF_123= then retry; Bearer alone";
        assert_eq!(
            redact(body),
            "try Authorization: Bearer <redacted> then retry; Bearer <redacted>"
        );
    }

    #[test]
    fn leaves_ordinary_text_untouched() {
        let text = "nothing secret here, muster-pair is just a word";
        assert!(matches!(redact(text), std::borrow::Cow::Borrowed(_)));
    }

    #[test]
    fn masks_the_token_in_the_shown_pairing() {
        let p = Pairing {
            address: "murph:6255".into(),
            fingerprint: "ab".repeat(32),
            token: "secret-token".into(),
            name: "a1".into(),
        };
        let shown = p.masked();
        assert!(shown.contains("murph:6255") && shown.contains("a1"));
        assert!(!shown.contains("secret-token"));
    }
}
