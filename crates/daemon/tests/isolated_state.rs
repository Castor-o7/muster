//! MUS-95 (follow-up): no daemon test touches the real home or state dir.
//! The daemon reads HOME in two places, `credentials::state_dir()` (falling
//! back to `~/.local/state/muster`) and `credentials::ensure_local_token()`
//! (the config dir). The daemon's tests run everything in-process and pass
//! each listener a temporary state dir, and these checks keep it that way.

use std::path::{Path, PathBuf};

/// Every `.rs` file under `dir`, at any depth (`tests/common/mod.rs`, say).
fn rust_files(dir: &Path) -> Vec<(PathBuf, String)> {
    let mut files = Vec::new();
    let mut dirs = vec![dir.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(&dir).expect("read dir") {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                dirs.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let text = std::fs::read_to_string(&path).expect("read file");
                files.push((path, text));
            }
        }
    }
    files
}

/// Where each of `patterns` appears in `files`, as `file:line: pattern`.
fn find(files: &[(PathBuf, String)], patterns: &[&str]) -> Vec<String> {
    let mut found = Vec::new();
    for (path, text) in files {
        for (n, line) in text.lines().enumerate() {
            for pattern in patterns {
                if line.contains(pattern) {
                    found.push(format!("{}:{}: {pattern}", path.display(), n + 1));
                }
            }
        }
    }
    found
}

#[test]
fn no_daemon_test_starts_a_process_or_reads_the_real_state() {
    let tests = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    // This file names the patterns it looks for.
    let files: Vec<_> = rust_files(&tests)
        .into_iter()
        .filter(|(p, _)| !p.ends_with("isolated_state.rs"))
        .collect();
    let found = find(
        &files,
        &["Command::new(", "state_dir()", "ensure_local_token()"],
    );
    assert!(
        found.is_empty(),
        "a daemon test would start a process with the real HOME, or read the real state or \
         config dir; pass a tempfile::TempDir instead: {found:?}"
    );
}

/// `credentials::tests::the_local_token_is_owner_only_and_reused` sets
/// `XDG_CONFIG_HOME` for its whole test process. That's safe only while it's
/// the one place in the library that reads or sets the config dir, outside
/// the daemon's `main.rs`.
#[test]
fn the_config_dir_is_read_and_set_only_in_credentials() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let files: Vec<_> = rust_files(&src)
        .into_iter()
        .filter(|(p, _)| !p.ends_with("main.rs") && !p.ends_with("credentials.rs"))
        .collect();
    let found = find(
        &files,
        &[
            "ensure_local_token(",
            "read_local_token(",
            "pairing::config_dir(",
            "set_var(\"XDG_CONFIG_HOME\"",
            "set_var(\"HOME\"",
        ],
    );
    assert!(
        found.is_empty(),
        "the config dir is read or set outside credentials.rs, so the local-token test's \
         process-wide XDG_CONFIG_HOME could redirect it: {found:?}"
    );
}
