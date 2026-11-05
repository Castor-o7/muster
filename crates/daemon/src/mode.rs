//! Which mode the daemon runs in (MUS-64): plain, like upstream Marshal, or
//! with the TLS gate.
//!
//! - **Plain** (the default): myko serves WebSocket and MCP directly on
//!   `MUSTER_BIND`, and the hook listener takes requests without a token.
//!   Security comes from where the daemon is bound, as in upstream Marshal.
//! - **TLS** (`--tls` or `MUSTER_TLS=1`): the gate on `MUSTER_BIND` adds TLS,
//!   pairing, tokens, the frame filter and the limits; see `gate`.
//!
//! When the flag and the variable disagree the daemon refuses to start. It also
//! refuses to run plain with paired machines in its state dir unless
//! `MUSTER_TLS=0` says so, so that losing the setting from a unit file can't
//! quietly turn a gated daemon into an open one.

/// The environment variable that turns the gate on.
pub const ENV: &str = "MUSTER_TLS";
/// The command-line flag that turns the gate on.
pub const FLAG: &str = "--tls";

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

/// `MUSTER_TLS`'s value as a switch: `Some(true)` for on, `Some(false)` for an
/// explicit off, `None` when unset or empty.
pub fn parse_env(value: Option<&str>) -> Result<Option<bool>, String> {
    let Some(raw) = value else {
        return Ok(None);
    };
    match raw.trim().to_ascii_lowercase().as_str() {
        "" => Ok(None),
        "1" | "true" | "on" | "yes" => Ok(Some(true)),
        "0" | "false" | "off" | "no" => Ok(Some(false)),
        _ => Err(format!(
            "{ENV}={raw} isn't understood; use {ENV}=1 for the TLS gate or {ENV}=0 for plain"
        )),
    }
}

/// Decide the mode from the `--tls` flag, `MUSTER_TLS`, and whether the state
/// dir holds any pairings.
pub fn resolve(flag: bool, env: Option<&str>, has_pairings: bool) -> Result<Mode, String> {
    let from_env = parse_env(env)?;
    if flag && from_env == Some(false) {
        return Err(format!(
            "{FLAG} and {ENV}={} disagree; set one",
            env.unwrap_or_default().trim()
        ));
    }
    if flag || from_env == Some(true) {
        return Ok(Mode::Tls);
    }
    if has_pairings && from_env.is_none() {
        return Err(format!(
            "this daemon has paired machines, but TLS isn't on: set {ENV}=1 (or pass {FLAG}) \
             to keep the gate, or {ENV}=0 to run plain anyway, unauthenticated"
        ));
    }
    Ok(Mode::Plain)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_is_the_default() {
        assert_eq!(resolve(false, None, false), Ok(Mode::Plain));
        assert_eq!(resolve(false, Some(""), false), Ok(Mode::Plain));
    }

    #[test]
    fn the_flag_or_the_variable_turns_tls_on() {
        assert_eq!(resolve(true, None, false), Ok(Mode::Tls));
        for on in ["1", "true", "ON", " yes "] {
            assert_eq!(resolve(false, Some(on), false), Ok(Mode::Tls), "{on}");
            assert_eq!(resolve(true, Some(on), false), Ok(Mode::Tls), "{on}");
        }
    }

    #[test]
    fn an_explicit_off_runs_plain() {
        for off in ["0", "false", "off", "No"] {
            assert_eq!(resolve(false, Some(off), false), Ok(Mode::Plain), "{off}");
            assert_eq!(resolve(false, Some(off), true), Ok(Mode::Plain), "{off}");
        }
    }

    #[test]
    fn a_conflict_refuses_to_start_and_names_both() {
        let err = resolve(true, Some("0"), false).unwrap_err();
        assert_eq!(err, "--tls and MUSTER_TLS=0 disagree; set one");
    }

    #[test]
    fn an_unknown_value_is_refused() {
        let err = resolve(false, Some("maybe"), false).unwrap_err();
        assert!(err.contains("MUSTER_TLS=maybe"), "{err}");
    }

    #[test]
    fn paired_machines_need_tls_or_an_explicit_off() {
        let err = resolve(false, None, true).unwrap_err();
        assert!(err.contains("paired machines"), "{err}");
        assert_eq!(resolve(true, None, true), Ok(Mode::Tls));
        assert_eq!(resolve(false, Some("1"), true), Ok(Mode::Tls));
    }
}
