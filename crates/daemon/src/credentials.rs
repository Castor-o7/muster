//! The gate's TLS identity and the per-machine access tokens.
//!
//! Both live in the daemon's state directory (`MUSTER_STATE_DIR`, else
//! `$XDG_STATE_HOME/muster`, else `~/.local/state/muster`):
//!
//! - `gate-cert.der` / `gate-key.der` — a self-signed certificate made on
//!   first use. Clients pin its SHA-256 fingerprint (carried in the pairing
//!   code), so no CA is involved and the names in it don't matter.
//! - `tokens` — one `name sha256(token)` line per paired machine. Only
//!   hashes are kept, so the file can't be used to connect; `unpair` deletes a
//!   line to revoke that machine.
//!
//! Files are written owner-only (0600 on unix).

use std::io;
use std::path::{Path, PathBuf};

use muster_entities::pairing::{cert_fingerprint, token_hash};

const CERT_FILE: &str = "gate-cert.der";
const KEY_FILE: &str = "gate-key.der";
const TOKENS_FILE: &str = "tokens";
/// One `name sha256(secret) expires-unix-seconds` line per outstanding invite
/// (MUS-8). Only hashes are kept, so the file can't be used to pair.
const INVITES_FILE: &str = "invites";

/// Serializes invite redemption within the daemon, so two machines racing
/// with the same code can't both get a token.
static INVITES_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The daemon's state directory, resolved from the environment.
pub fn state_dir() -> io::Result<PathBuf> {
    if let Some(dir) = std::env::var_os("MUSTER_STATE_DIR").filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(dir));
    }
    if let Some(dir) = std::env::var_os("XDG_STATE_HOME").filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(dir).join("muster"));
    }
    let home = std::env::var_os("HOME")
        .filter(|v| !v.is_empty())
        .ok_or_else(|| io::Error::other("HOME is not set; set MUSTER_STATE_DIR"))?;
    Ok(PathBuf::from(home).join(".local/state/muster"))
}

/// The gate's certificate and private key, both DER.
pub struct Identity {
    pub cert_der: Vec<u8>,
    pub key_der: Vec<u8>,
}

impl Identity {
    pub fn fingerprint(&self) -> String {
        cert_fingerprint(&self.cert_der)
    }
}

/// Load the gate identity from `dir`, creating it on first use.
pub fn load_or_create_identity(dir: &Path) -> io::Result<Identity> {
    let (cert_path, key_path) = (dir.join(CERT_FILE), dir.join(KEY_FILE));
    if cert_path.exists() && key_path.exists() {
        return Ok(Identity {
            cert_der: std::fs::read(cert_path)?,
            key_der: std::fs::read(key_path)?,
        });
    }
    let generated = rcgen::generate_simple_self_signed(vec!["muster-daemon".to_string()])
        .map_err(io::Error::other)?;
    let identity = Identity {
        cert_der: generated.cert.der().to_vec(),
        key_der: generated.signing_key.serialize_der(),
    };
    std::fs::create_dir_all(dir)?;
    write_private(&key_path, &identity.key_der)?;
    write_private(&cert_path, &identity.cert_der)?;
    log::info!(
        "[gate] created TLS identity in {} (fingerprint {})",
        dir.display(),
        identity.fingerprint()
    );
    Ok(identity)
}

/// A token's name is what `unpair` and the logs refer to.
/// The prefix a link's token name carries in `tokens` and `invites` (MUS-66).
/// `valid_name` refuses `:`, so no machine can be named like a link.
pub const LINK_PREFIX: &str = "link:";

/// `tokens`' name for the link `name`.
pub fn link_token_name(name: &str) -> String {
    format!("{LINK_PREFIX}{name}")
}

/// Whether a token name from `verify_token` is a link's, and which.
pub fn as_link(token_name: &str) -> Option<&str> {
    token_name.strip_prefix(LINK_PREFIX)
}

/// A name `tokens` may hold: a valid machine name, or `link:` and one.
fn valid_token_name(name: &str) -> bool {
    valid_name(as_link(name).unwrap_or(name))
}

pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

/// Mint a token for `name`, replacing any earlier one, and return it.
pub fn add_token(dir: &Path, name: &str) -> io::Result<String> {
    if !valid_token_name(name) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "names are 1-64 characters of letters, digits, '-', '_' or '.'",
        ));
    }
    let mut raw = [0u8; 32];
    getrandom::fill(&mut raw).map_err(io::Error::other)?;
    let token = {
        use base64::Engine as _;
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(raw)
    };
    let mut entries: Vec<(String, String)> = read_tokens(dir)?
        .into_iter()
        .filter(|(n, _)| n != name)
        .collect();
    entries.push((name.to_string(), token_hash(&token)));
    write_tokens(dir, &entries)?;
    Ok(token)
}

/// Make a one-time invite for `name`, good for `ttl` (MUS-8). Returns the
/// secret, which `redeem_invite` exchanges for a token once. A new invite for
/// the same name replaces any outstanding one.
pub fn add_invite(dir: &Path, name: &str, ttl: std::time::Duration) -> io::Result<String> {
    if !valid_token_name(name) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "names are 1-64 characters of letters, digits, '-', '_' or '.'",
        ));
    }
    let mut raw = [0u8; 32];
    getrandom::fill(&mut raw).map_err(io::Error::other)?;
    let secret = {
        use base64::Engine as _;
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(raw)
    };
    let expires = unix_now().saturating_add(ttl.as_secs());
    let _guard = INVITES_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _file_lock = lock_file(dir, INVITES_FILE)?;
    let now = unix_now();
    let mut entries: Vec<(String, String, u64)> = read_invites(dir)?
        .into_iter()
        .filter(|(n, _, exp)| n != name && *exp > now)
        .collect();
    entries.push((name.to_string(), token_hash(&secret), expires));
    write_invites(dir, &entries)?;
    Ok(secret)
}

/// Exchange an invite for the name it was made for and a fresh token (MUS-8).
/// The invite is deleted whether or not anything else succeeds, so it works
/// once. An unknown, used or expired invite gets `None`.
pub fn redeem_invite(dir: &Path, secret: &str) -> io::Result<Option<(String, String)>> {
    let presented = token_hash(secret);
    let name = {
        let _guard = INVITES_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _file_lock = lock_file(dir, INVITES_FILE)?;
        let now = unix_now();
        let entries = read_invites(dir)?;
        let before = entries.len();
        let mut found = None;
        let mut kept = Vec::with_capacity(entries.len());
        for (name, hash, expires) in entries {
            if constant_time_eq(hash.as_bytes(), presented.as_bytes()) {
                if expires > now {
                    found = Some(name);
                }
            } else if expires > now {
                kept.push((name, hash, expires));
            }
        }
        // A wrong guess with nothing expired changes nothing, so it writes nothing.
        if kept.len() != before {
            write_invites(dir, &kept)?;
        }
        found
    };
    let Some(name) = name else {
        return Ok(None);
    };
    let token = add_token(dir, &name)?;
    Ok(Some((name, token)))
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn read_invites(dir: &Path) -> io::Result<Vec<(String, String, u64)>> {
    let text = match std::fs::read_to_string(dir.join(INVITES_FILE)) {
        Ok(t) => t,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    Ok(text
        .lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let (name, hash, expires) = (parts.next()?, parts.next()?, parts.next()?);
            Some((name.to_string(), hash.to_string(), expires.parse().ok()?))
        })
        .collect())
}

fn write_invites(dir: &Path, entries: &[(String, String, u64)]) -> io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let text: String = entries
        .iter()
        .map(|(n, h, e)| format!("{n} {h} {e}\n"))
        .collect();
    let tmp = dir.join(format!("{INVITES_FILE}.tmp"));
    write_private(&tmp, text.as_bytes())?;
    std::fs::rename(tmp, dir.join(INVITES_FILE))
}

/// Hold `<name>.lock` in `dir` across processes, so `muster-daemon pair`
/// and the running daemon don't rewrite the same file at once.
fn lock_file(dir: &Path, name: &str) -> io::Result<std::fs::File> {
    std::fs::create_dir_all(dir)?;
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join(format!("{name}.lock")))?;
    file.lock()?;
    Ok(file)
}

/// Revoke `name`. Returns whether it was paired.
pub fn remove_token(dir: &Path, name: &str) -> io::Result<bool> {
    let entries = read_tokens(dir)?;
    let before = entries.len();
    let kept: Vec<_> = entries.into_iter().filter(|(n, _)| n != name).collect();
    if kept.len() == before {
        return Ok(false);
    }
    write_tokens(dir, &kept)?;
    release_sessions(dir, name)?;
    Ok(true)
}

/// Which token `unpair` revokes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnpairKind {
    /// `--machine`: only a machine pairing.
    Machine,
    /// `--link`: only a linked daemon's token (`link:<name>`).
    Link,
    /// A bare name: whichever one exists, refused if both do.
    Either,
}

/// What `unpair` did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unpaired {
    Machine,
    Link,
    Nothing,
    /// A bare name matched a machine and a link, so neither was revoked
    /// (MUS-84): revoking a test link mustn't cut off a whole machine.
    Ambiguous,
}

/// Revoke a machine's pairing or a daemon's link here. A bare `link:<name>`
/// names the link exactly.
pub fn unpair(dir: &Path, name: &str, kind: UnpairKind) -> io::Result<Unpaired> {
    let (name, kind) = match (as_link(name), kind) {
        (Some(bare), UnpairKind::Either | UnpairKind::Link) => (bare, UnpairKind::Link),
        _ => (name, kind),
    };
    let link = link_token_name(name);
    let revoke = |token: &str, done: Unpaired| -> io::Result<Unpaired> {
        Ok(if remove_token(dir, token)? {
            done
        } else {
            Unpaired::Nothing
        })
    };
    match kind {
        UnpairKind::Machine => revoke(name, Unpaired::Machine),
        UnpairKind::Link => revoke(&link, Unpaired::Link),
        UnpairKind::Either => {
            let names = token_names(dir)?;
            let has = |n: &str| names.iter().any(|t| t == n);
            match (has(name), has(&link)) {
                (true, true) => Ok(Unpaired::Ambiguous),
                (true, false) => revoke(name, Unpaired::Machine),
                (false, true) => revoke(&link, Unpaired::Link),
                (false, false) => Ok(Unpaired::Nothing),
            }
        }
    }
}

/// The paired machine names, in pairing order.
pub fn token_names(dir: &Path) -> io::Result<Vec<String>> {
    Ok(read_tokens(dir)?.into_iter().map(|(n, _)| n).collect())
}

/// The name a presented token belongs to, or `None` if it's not valid.
/// Reads the file each time, so `pair`/`unpair` take effect without a restart.
pub fn verify_token(dir: &Path, token: &str) -> Option<String> {
    let presented = token_hash(token);
    let entries = read_tokens(dir).ok()?;
    entries
        .into_iter()
        .find(|(_, hash)| constant_time_eq(hash.as_bytes(), presented.as_bytes()))
        .map(|(name, _)| name)
}

fn read_tokens(dir: &Path) -> io::Result<Vec<(String, String)>> {
    let text = match std::fs::read_to_string(dir.join(TOKENS_FILE)) {
        Ok(t) => t,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    Ok(text
        .lines()
        .filter_map(|line| {
            let (name, hash) = line.trim().split_once(' ')?;
            Some((name.to_string(), hash.trim().to_string()))
        })
        .collect())
}

fn write_tokens(dir: &Path, entries: &[(String, String)]) -> io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let text: String = entries.iter().map(|(n, h)| format!("{n} {h}\n")).collect();
    // Write a sibling and rename, so a running gate never reads half a file.
    let tmp = dir.join(format!("{TOKENS_FILE}.tmp"));
    write_private(&tmp, text.as_bytes())?;
    std::fs::rename(tmp, dir.join(TOKENS_FILE))
}

/// Write `dir/name` owner-only, through a sibling and a rename, so a reader
/// never sees half a file.
pub fn write_private_file(dir: &Path, name: &str, bytes: &[u8]) -> io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!("{name}.tmp"));
    write_private(&tmp, bytes)?;
    std::fs::rename(tmp, dir.join(name))
}

fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::io::Write as _;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    options.open(path)?.write_all(bytes)
}

/// The token local processes present to the hook listener and the local gate
/// (MUS-7). Written owner-only to the operator's muster config directory on
/// first start, so anything that can't read that directory can't act as a
/// session. Read back on later starts, so sessions keep working across a
/// restart.
pub fn ensure_local_token() -> io::Result<String> {
    let dir = muster_entities::pairing::config_dir().ok_or_else(|| {
        io::Error::other("no config directory for the local token (is HOME set?)")
    })?;
    let path = dir.join(muster_entities::pairing::LOCAL_TOKEN_FILE);
    if let Ok(text) = std::fs::read_to_string(&path) {
        let token = text.lines().next().unwrap_or("").trim();
        if !token.is_empty() {
            return Ok(token.to_string());
        }
    }
    std::fs::create_dir_all(&dir)?;
    let mut raw = [0u8; 32];
    getrandom::fill(&mut raw).map_err(io::Error::other)?;
    let token = {
        use base64::Engine as _;
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(raw)
    };
    write_private(&path, format!("{token}\n").as_bytes())?;
    log::info!("[local] wrote a new local token to {}", path.display());
    Ok(token)
}

/// Whether a presented local token is the daemon's, compared in constant time.
pub fn local_token_matches(expected: &str, presented: &str) -> bool {
    constant_time_eq(
        token_hash(expected).as_bytes(),
        token_hash(presented).as_bytes(),
    )
}

/// The operator this daemon serves, from `MUSTER_OPERATOR`. When set, it's what
/// sessions from paired machines show, whatever they claim. Without it their
/// own claim stands.
pub fn daemon_operator() -> Option<String> {
    std::env::var("MUSTER_OPERATOR")
        .ok()
        .filter(|s| !s.is_empty())
}

/// The owner of a session on the daemon's own host. `valid_name` can't produce
/// it, so no paired machine can share it.
pub const LOCAL_MACHINE: &str = "(local)";

/// Session ids bound to the paired machine that first used them.
const BINDINGS_FILE: &str = "session-bindings";

/// Serializes the read-then-append in [`claim_session`], so two machines racing
/// for a new session can't both win.
static BINDINGS_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Whether `machine` may act for `session`. The first paired machine to use a
/// session owns it from then on. An id that can't be stored safely on one line
/// is refused.
pub fn claim_session(dir: &Path, machine: &str, session: &str) -> io::Result<bool> {
    let safe = !session.is_empty()
        && session.len() <= 128
        && session
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'));
    if !safe {
        return Ok(false);
    }
    let _guard = BINDINGS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _file_lock = lock_bindings(dir)?;
    let path = dir.join(BINDINGS_FILE);
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e),
    };
    for line in text.lines() {
        if let Some((owner, id)) = line.split_once(' ')
            && id == session
        {
            return Ok(owner == machine);
        }
    }
    write_bindings(&path, &format!("{text}{machine} {session}\n"))?;
    Ok(true)
}

/// [`claim_session`] for a paired machine, whose session ids must be UUIDs, as
/// Claude Code's are. Recipients are also addressed by nickname, operator and id
/// prefix, so a machine free to choose any id could register one of those and
/// receive mail meant for someone else.
pub fn claim_machine_session(dir: &Path, machine: &str, session: &str) -> io::Result<bool> {
    let is_uuid = session.len() == 36 && uuid::Uuid::try_parse(session).is_ok();
    if !is_uuid {
        return Ok(false);
    }
    claim_session(dir, machine, session)
}

/// Forget every session bound to `machine`, when it's unpaired.
/// Hold the bindings across processes: `muster-daemon unpair` edits them from
/// outside the running daemon, and each side rewrites the whole file. Released
/// when the returned file closes.
fn lock_bindings(dir: &Path) -> io::Result<std::fs::File> {
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join(format!("{BINDINGS_FILE}.lock")))?;
    file.lock()?;
    Ok(file)
}

fn release_sessions(dir: &Path, machine: &str) -> io::Result<()> {
    let _guard = BINDINGS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _file_lock = lock_bindings(dir)?;
    let path = dir.join(BINDINGS_FILE);
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    let kept: String = text
        .lines()
        .filter(|line| {
            line.split_once(' ')
                .is_none_or(|(owner, _)| owner != machine)
        })
        .map(|line| format!("{line}\n"))
        .collect();
    write_bindings(&path, &kept)
}

/// Replace the bindings file in one step. A crash or full disk partway through
/// must not leave it cut short, because a missing binding lets any machine claim
/// that session.
fn write_bindings(path: &Path, text: &str) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    write_private(&tmp, text.as_bytes())?;
    std::fs::rename(&tmp, path)
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {

    fn invite_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("muster-invites-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn unpair_never_revokes_a_machine_and_a_link_together() {
        let dir = invite_dir("unpair");
        add_token(&dir, "a1").unwrap();
        add_token(&dir, &link_token_name("a1")).unwrap();
        add_token(&dir, "b2").unwrap();
        add_token(&dir, &link_token_name("c3")).unwrap();
        let names = |d: &Path| token_names(d).unwrap();

        // A bare name that matches both is refused, and nothing is revoked.
        assert_eq!(
            unpair(&dir, "a1", UnpairKind::Either).unwrap(),
            Unpaired::Ambiguous
        );
        assert_eq!(names(&dir).len(), 4);
        // --link takes only the link, --machine only the machine.
        assert_eq!(
            unpair(&dir, "a1", UnpairKind::Link).unwrap(),
            Unpaired::Link
        );
        assert!(names(&dir).contains(&"a1".to_string()));
        assert_eq!(
            unpair(&dir, "a1", UnpairKind::Link).unwrap(),
            Unpaired::Nothing
        );
        assert_eq!(
            unpair(&dir, "a1", UnpairKind::Machine).unwrap(),
            Unpaired::Machine
        );
        // A bare name that matches one revokes that one.
        assert_eq!(
            unpair(&dir, "b2", UnpairKind::Either).unwrap(),
            Unpaired::Machine
        );
        assert_eq!(
            unpair(&dir, "b2", UnpairKind::Link).unwrap(),
            Unpaired::Nothing
        );
        assert_eq!(
            unpair(&dir, "link:c3", UnpairKind::Either).unwrap(),
            Unpaired::Link
        );
        assert_eq!(
            unpair(&dir, "nobody", UnpairKind::Either).unwrap(),
            Unpaired::Nothing
        );
        assert!(names(&dir).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_invite_works_once() {
        let dir = invite_dir("once");
        let secret = add_invite(&dir, "a1", std::time::Duration::from_secs(600)).unwrap();
        let (name, token) = redeem_invite(&dir, &secret)
            .unwrap()
            .expect("first use works");
        assert_eq!(name, "a1");
        assert_eq!(verify_token(&dir, &token).as_deref(), Some("a1"));
        assert!(
            redeem_invite(&dir, &secret).unwrap().is_none(),
            "a used invite is refused"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_expired_or_unknown_invite_is_refused() {
        let dir = invite_dir("expired");
        let secret = add_invite(&dir, "a1", std::time::Duration::ZERO).unwrap();
        assert!(redeem_invite(&dir, &secret).unwrap().is_none(), "expired");
        assert!(
            redeem_invite(&dir, "never-issued").unwrap().is_none(),
            "unknown"
        );
        assert!(token_names(&dir).unwrap().is_empty(), "no token was made");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_new_invite_replaces_the_old_one_for_that_name() {
        let dir = invite_dir("replace");
        let old = add_invite(&dir, "a1", std::time::Duration::from_secs(600)).unwrap();
        let new = add_invite(&dir, "a1", std::time::Duration::from_secs(600)).unwrap();
        assert!(redeem_invite(&dir, &old).unwrap().is_none());
        assert!(redeem_invite(&dir, &new).unwrap().is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[cfg(unix)]
    fn the_local_token_is_owner_only_and_reused() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = std::env::temp_dir().join(format!("muster-local-token-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // A 755 config dir with a permissive umask must still get a 0600 file.
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        // SAFETY: this is the only test in the crate that reads or sets the
        // config directory, and it runs in its own process when run alone.
        unsafe { std::env::set_var("XDG_CONFIG_HOME", &dir) };
        let first = ensure_local_token().unwrap();
        let path = dir
            .join("muster")
            .join(muster_entities::pairing::LOCAL_TOKEN_FILE);
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "local-token mode");
        assert!(first.len() >= 40, "token is 32 random bytes, base64url");
        assert_eq!(
            ensure_local_token().unwrap(),
            first,
            "a second start reuses it"
        );
        assert!(local_token_matches(&first, &first));
        assert!(!local_token_matches(&first, "something-else"));
        let _ = std::fs::remove_dir_all(&dir);
    }
    use super::*;

    #[test]
    fn identity_is_created_once_and_reused() {
        let dir = tempfile::tempdir().unwrap();
        let first = load_or_create_identity(dir.path()).unwrap();
        let second = load_or_create_identity(dir.path()).unwrap();
        assert_eq!(first.fingerprint(), second.fingerprint());
        assert_eq!(first.fingerprint().len(), 64);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(dir.path().join(KEY_FILE))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600, "private key must be owner-only");
        }
    }

    #[test]
    fn first_machine_to_use_a_session_owns_it() {
        let dir = tempfile::tempdir().unwrap();
        assert!(claim_session(dir.path(), "a1", "s-1").unwrap());
        assert!(claim_session(dir.path(), "a1", "s-1").unwrap());
        assert!(!claim_session(dir.path(), "murph", "s-1").unwrap());
        assert!(claim_session(dir.path(), "murph", "s-2").unwrap());
    }

    #[test]
    fn no_pairing_can_be_named_like_the_local_machine() {
        assert!(!valid_name(LOCAL_MACHINE));
    }

    #[test]
    fn failing_to_write_keeps_the_existing_bindings() {
        let dir = tempfile::tempdir().unwrap();
        assert!(claim_session(dir.path(), "a1", "s-1").unwrap());
        // A directory where the temp file goes makes the write fail.
        std::fs::create_dir(dir.path().join("session-bindings.tmp")).unwrap();
        assert!(claim_session(dir.path(), "a1", "s-2").is_err());
        assert!(!claim_session(dir.path(), "murph", "s-1").unwrap());
    }

    #[test]
    fn a_machine_cannot_claim_an_id_that_could_pose_as_someone() {
        let dir = tempfile::tempdir().unwrap();
        for id in ["copper-komodo", "josh", "josh@example.com", "4a212405"] {
            assert!(
                !claim_machine_session(dir.path(), "a1", id).unwrap(),
                "{id}"
            );
        }
        let id = "4a212405-0246-4c12-98f6-2359ca28c533";
        assert!(claim_machine_session(dir.path(), "a1", id).unwrap());
    }

    #[test]
    fn a_session_id_cannot_inject_a_binding() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!claim_session(dir.path(), "attacker", "x\nattacker victim").unwrap());
        assert!(!claim_session(dir.path(), "attacker", "x y").unwrap());
        assert!(!claim_session(dir.path(), "attacker", "").unwrap());
        assert!(claim_session(dir.path(), "a1", "victim").unwrap());
    }

    #[test]
    fn a_claim_waits_for_another_process_holding_the_bindings() {
        let dir = tempfile::tempdir().unwrap();
        let held = lock_bindings(dir.path()).unwrap();
        let path = dir.path().to_path_buf();
        let claim = std::thread::spawn(move || claim_session(&path, "a1", "s1").unwrap());
        std::thread::sleep(std::time::Duration::from_millis(200));
        assert!(!claim.is_finished());
        drop(held);
        assert!(claim.join().unwrap());
    }

    #[test]
    fn unpairing_releases_a_machines_sessions() {
        let dir = tempfile::tempdir().unwrap();
        add_token(dir.path(), "old").unwrap();
        assert!(claim_session(dir.path(), "old", "s-1").unwrap());
        assert!(claim_session(dir.path(), "a1", "s-2").unwrap());
        assert!(remove_token(dir.path(), "old").unwrap());
        assert!(claim_session(dir.path(), "murph", "s-1").unwrap());
        assert!(!claim_session(dir.path(), "murph", "s-2").unwrap());
    }

    #[test]
    fn tokens_verify_until_revoked() {
        let dir = tempfile::tempdir().unwrap();
        let a1 = add_token(dir.path(), "a1").unwrap();
        let murph = add_token(dir.path(), "murph").unwrap();
        assert_eq!(verify_token(dir.path(), &a1).as_deref(), Some("a1"));
        assert_eq!(verify_token(dir.path(), &murph).as_deref(), Some("murph"));
        assert_eq!(verify_token(dir.path(), "guess"), None);

        assert!(remove_token(dir.path(), "a1").unwrap());
        assert_eq!(verify_token(dir.path(), &a1), None);
        assert_eq!(verify_token(dir.path(), &murph).as_deref(), Some("murph"));
        assert!(!remove_token(dir.path(), "a1").unwrap());
    }

    #[test]
    fn repairing_a_name_replaces_its_token() {
        let dir = tempfile::tempdir().unwrap();
        let old = add_token(dir.path(), "a1").unwrap();
        let new = add_token(dir.path(), "a1").unwrap();
        assert_eq!(verify_token(dir.path(), &old), None);
        assert_eq!(verify_token(dir.path(), &new).as_deref(), Some("a1"));
        assert_eq!(token_names(dir.path()).unwrap(), vec!["a1"]);
    }

    #[test]
    fn only_hashes_are_stored() {
        let dir = tempfile::tempdir().unwrap();
        let token = add_token(dir.path(), "a1").unwrap();
        let file = std::fs::read_to_string(dir.path().join(TOKENS_FILE)).unwrap();
        assert!(!file.contains(&token));
    }

    #[test]
    fn names_are_restricted() {
        let dir = tempfile::tempdir().unwrap();
        assert!(add_token(dir.path(), "a b").is_err());
        assert!(add_token(dir.path(), "").is_err());
        assert!(add_token(dir.path(), "a1.local").is_ok());
    }
}
