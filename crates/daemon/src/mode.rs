//! Which mode the daemon runs in: plain, as Marshal ran before the gate, or
//! with the TLS gate.
//!
//! - **Plain** (the default): myko serves WebSocket and MCP directly on
//!   `MARSHAL_BIND`, and the hook listener takes requests without a token.
//!   Security comes from where the daemon is bound.
//! - **TLS** (`--tls`): the gate on `MARSHAL_BIND` adds TLS, pairing, tokens,
//!   the frame filter and the limits; see `gate`.
//!
//! The daemon refuses to run plain with paired machines in its state dir
//! unless `--plain` says so, so that losing `--tls` from a unit file can't
//! quietly turn a gated daemon into an open one. Given both flags, it refuses
//! to start.
//!
//! Only these two flags are looked for. Any other argument is left alone, as
//! the daemon has always done.

/// The flag that turns the gate on.
pub const TLS_FLAG: &str = "--tls";
/// The flag that runs plain even though machines are paired.
pub const PLAIN_FLAG: &str = "--plain";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Plain,
    Tls,
}

impl Mode {
    pub fn is_tls(self) -> bool {
        self == Self::Tls
    }
}

/// Whether the state dir holds any pairings. A tokens file that exists but
/// can't be read is an error, not "none", and a line that doesn't parse counts
/// as a pairing: reading either as none would let a daemon that lost `--tls`
/// start plain without the refusal below.
pub fn has_pairings(state_dir: &std::path::Path) -> Result<bool, String> {
    crate::credentials::any_token_entries(state_dir).map_err(|e| {
        format!(
            "can't read the paired machines in {}: {e}; fix or remove the file before starting",
            state_dir.display()
        )
    })
}

/// Decide the mode from the daemon's arguments and whether the state dir holds
/// any pairings.
pub fn resolve<S: AsRef<str>>(args: &[S], has_pairings: bool) -> Result<Mode, String> {
    let given = |flag: &str| args.iter().any(|a| a.as_ref() == flag);
    match (given(TLS_FLAG), given(PLAIN_FLAG)) {
        (true, true) => Err(format!("{TLS_FLAG} and {PLAIN_FLAG} disagree; pass one")),
        (true, false) => Ok(Mode::Tls),
        (false, true) => Ok(Mode::Plain),
        (false, false) if has_pairings => Err(format!(
            "this daemon has paired machines, but TLS isn't on: pass {TLS_FLAG} to keep the \
             gate, or {PLAIN_FLAG} to run plain anyway, unauthenticated"
        )),
        (false, false) => Ok(Mode::Plain),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: [&str; 0] = [];

    #[test]
    fn plain_is_the_default() {
        assert_eq!(resolve(&NONE, false), Ok(Mode::Plain));
    }

    #[test]
    fn the_flag_turns_tls_on() {
        assert_eq!(resolve(&["--tls"], false), Ok(Mode::Tls));
        assert_eq!(resolve(&["--tls"], true), Ok(Mode::Tls));
    }

    #[test]
    fn other_arguments_are_left_alone() {
        assert_eq!(resolve(&["--verbose"], false), Ok(Mode::Plain));
        assert_eq!(resolve(&["--verbose", "--tls"], false), Ok(Mode::Tls));
    }

    #[test]
    fn both_flags_refuse_to_start_and_name_both() {
        let err = resolve(&["--tls", "--plain"], false).unwrap_err();
        assert_eq!(err, "--tls and --plain disagree; pass one");
    }

    #[test]
    fn paired_machines_need_tls_or_an_explicit_plain() {
        let err = resolve(&NONE, true).unwrap_err();
        assert!(err.contains("paired machines"), "{err}");
        assert!(err.contains("--plain"), "{err}");
        assert_eq!(resolve(&["--plain"], true), Ok(Mode::Plain));
    }

    #[test]
    fn no_tokens_file_means_no_pairings() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(has_pairings(dir.path()), Ok(false));
        crate::credentials::add_token(dir.path(), "host-a").unwrap();
        assert_eq!(has_pairings(dir.path()), Ok(true));
    }

    #[test]
    fn an_unreadable_tokens_file_refuses_rather_than_reading_as_none() {
        let dir = tempfile::tempdir().unwrap();
        // A directory where the file should be: it exists, but can't be read.
        std::fs::create_dir(dir.path().join("tokens")).unwrap();
        let err = has_pairings(dir.path()).unwrap_err();
        assert!(err.contains("can't read the paired machines"), "{err}");
    }

    #[test]
    fn a_line_that_does_not_parse_still_counts_as_paired() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("tokens"), "garbled\n").unwrap();
        assert_eq!(has_pairings(dir.path()), Ok(true));
        std::fs::write(dir.path().join("tokens"), "\n  \n").unwrap();
        assert_eq!(has_pairings(dir.path()), Ok(false));
    }
}
