//! What every shim integration test shares (MUS-95): the shim binary, run
//! with a home of its own.
//!
//! A shim started with the developer's real HOME finds their real pairing in
//! `~/.config/muster` and connects to their live daemon, so a test run could
//! read and write a real roster. Every test starts the shim with [`shim`],
//! which points HOME and the config dirs at a throwaway directory. A test that
//! needs its own (a daemon's config, say) sets HOME again after it, to another
//! temporary directory. `tests/isolated_home.rs` fails any test file that
//! starts the shim another way.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::LazyLock;

pub const SHIM: &str = env!("CARGO_BIN_EXE_muster-shim");

/// One empty home for the whole test binary. Shared, as the real one was, but
/// holding nothing: no pairing, no daemon address, no local token.
static HOME: LazyLock<tempfile::TempDir> =
    LazyLock::new(|| tempfile::tempdir().expect("isolated home for shim tests"));

/// The isolated home every [`shim`] command gets.
pub fn isolated_home() -> &'static Path {
    let home = HOME.path();
    assert_ne!(
        Some(home.to_path_buf()),
        real_home(),
        "the isolated home is the real one"
    );
    home
}

/// The HOME this test process was started with, which a shim must never see.
pub fn real_home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
}

/// `muster-shim`, with HOME, `XDG_CONFIG_HOME` and their Windows equivalents
/// in the isolated home, and no daemon address, pairing, or Claude or Codex
/// config dir from the environment: `setup`, `doctor` and `statusline` honour
/// `CLAUDE_CONFIG_DIR`, and `setup --apply` writes there. A test adds what it
/// needs.
pub fn shim() -> Command {
    let home = isolated_home();
    let mut cmd = Command::new(SHIM);
    cmd.env("HOME", home)
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("USERPROFILE", home)
        .env("APPDATA", home.join("AppData").join("Roaming"))
        .env_remove("MUSTER_DAEMON_ADDRESS")
        .env_remove("MUSTER_BASE_URL")
        .env_remove("MYKO_ADDRESS")
        .env_remove("CLAUDE_CONFIG_DIR")
        .env_remove("CODEX_HOME");
    cmd
}
