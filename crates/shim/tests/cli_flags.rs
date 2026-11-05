//! `--help` and `--version` answer and exit before the shim connects to
//! anything, and an unknown argument points at `--help`.
//!
//! Each run has a deadline, so a shim that starts serving when it should have
//! exited fails the test instead of hanging it.

use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

const DEADLINE: Duration = Duration::from_secs(20);

fn shim(args: &[&str]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_muster-shim"))
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn muster-shim");
    let started = Instant::now();
    while child.try_wait().expect("poll muster-shim").is_none() {
        if started.elapsed() > DEADLINE {
            let _ = child.kill();
            panic!("muster-shim {args:?} kept running instead of exiting");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    child
        .wait_with_output()
        .expect("collect muster-shim output")
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn version_prints_the_name_and_version_and_exits() {
    for flag in ["--version", "-V"] {
        let out = shim(&[flag]);
        assert!(
            out.status.success(),
            "{flag}: {:?} {}",
            out.status,
            stderr(&out)
        );
        // Trim the line ending so the check holds on Windows too.
        assert_eq!(
            stdout(&out).trim_end(),
            format!("muster-shim {}", env!("CARGO_PKG_VERSION"))
        );
        assert_eq!(stderr(&out), "");
    }
}

#[test]
fn help_names_every_subcommand_and_exits() {
    for flag in ["--help", "-h"] {
        let out = shim(&[flag]);
        assert!(
            out.status.success(),
            "{flag}: {:?} {}",
            out.status,
            stderr(&out)
        );
        let help = stdout(&out);
        for expected in [
            "Usage: muster-shim",
            "--check",
            "--version",
            "statusline",
            "codex-hook",
            "codex-setup",
            "codex-run",
            "codex-bridge",
            "claude-hook",
            "watch",
            "tell",
            "all",
            "roster",
            "doctor",
            "setup",
            "pair",
            "pairing",
            "unpair",
            "MUSTER_DAEMON_ADDRESS",
            "MYKO_ADDRESS",
        ] {
            assert!(
                help.contains(expected),
                "{flag} output lacks {expected:?}:\n{help}"
            );
        }
        assert_eq!(stderr(&out), "");
    }
}

#[test]
fn an_unknown_argument_points_at_help() {
    let out = shim(&["--no-such-flag"]);
    assert!(!out.status.success());
    assert_eq!(stdout(&out), "");
    let err = stderr(&out);
    assert!(err.contains("unknown argument: --no-such-flag"), "{err}");
    assert!(err.contains("muster-shim --help"), "{err}");
}
