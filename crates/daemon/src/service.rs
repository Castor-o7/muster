//! `muster-daemon service`: the unit that keeps the daemon running across
//! reboots (MUS-15). A systemd user unit on Linux, a launchd agent on macOS.
//!
//! By default it only prints the file it would write and where. `--apply` writes
//! it, and refuses if a unit is already there unless `--force` is given, in which
//! case the old file is backed up next to it first. It never starts, stops or
//! restarts anything: it prints the command for the operator to run, since a
//! restart of a running daemon needs the operator's own word.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

/// The systemd unit name `scripts/rollout.sh` already expects.
pub const SYSTEMD_UNIT: &str = "muster-daemon.service";
/// The launchd label.
pub const LAUNCHD_LABEL: &str = "dev.muster.daemon";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Systemd,
    Launchd,
}

impl Kind {
    pub fn for_this_os() -> Option<Kind> {
        if cfg!(target_os = "linux") {
            Some(Kind::Systemd)
        } else if cfg!(target_os = "macos") {
            Some(Kind::Launchd)
        } else {
            None
        }
    }

    /// Where the unit goes, under `home`.
    pub fn path(self, home: &Path) -> PathBuf {
        match self {
            Kind::Systemd => home.join(".config/systemd/user").join(SYSTEMD_UNIT),
            Kind::Launchd => home
                .join("Library/LaunchAgents")
                .join(format!("{LAUNCHD_LABEL}.plist")),
        }
    }

    /// What the operator runs to start it, once written.
    pub fn start_commands(self, path: &Path) -> Vec<String> {
        match self {
            Kind::Systemd => vec![
                "systemctl --user daemon-reload".into(),
                format!("systemctl --user enable --now {SYSTEMD_UNIT}"),
                "loginctl enable-linger \"$USER\"   # so it runs at boot, before you log in".into(),
            ],
            Kind::Launchd => vec![format!(
                "launchctl bootstrap gui/$(id -u) {}",
                path.display()
            )],
        }
    }

    /// What the operator runs to apply a changed unit to a running daemon.
    pub fn restart_commands(self, path: &Path) -> Vec<String> {
        match self {
            Kind::Systemd => vec![
                "systemctl --user daemon-reload".into(),
                format!("systemctl --user restart {SYSTEMD_UNIT}"),
            ],
            Kind::Launchd => vec![
                format!("launchctl bootout gui/$(id -u)/{LAUNCHD_LABEL}"),
                format!("launchctl bootstrap gui/$(id -u) {}", path.display()),
            ],
        }
    }
}

/// The unit's text. `exe` is the daemon binary, `tls` adds `MUSTER_TLS=1`,
/// and `log_dir` is where launchd writes the daemon's output.
pub fn render(kind: Kind, exe: &Path, tls: bool, log_dir: &Path) -> String {
    let exe = exe.display();
    match kind {
        Kind::Systemd => {
            let env = if tls {
                "Environment=MUSTER_TLS=1\n"
            } else {
                ""
            };
            let exe = systemd_quote(&exe.to_string());
            format!(
                "# Written by `muster-daemon service --apply`.\n\
                 [Unit]\n\
                 Description=Muster daemon\n\
                 After=network-online.target\n\
                 Wants=network-online.target\n\
                 \n\
                 [Service]\n\
                 ExecStart={exe}\n\
                 {env}\
                 Restart=on-failure\n\
                 RestartSec=5\n\
                 \n\
                 [Install]\n\
                 WantedBy=default.target\n"
            )
        }
        Kind::Launchd => {
            let env = if tls {
                "  <key>EnvironmentVariables</key>\n  <dict>\n    <key>MUSTER_TLS</key>\n    \
                 <string>1</string>\n  </dict>\n"
            } else {
                ""
            };
            let log = xml_escape(&log_dir.join("muster-daemon.log").display().to_string());
            let exe = xml_escape(&exe.to_string());
            format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
                 <!-- Written by `muster-daemon service --apply`. -->\n\
                 <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \
                 \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
                 <plist version=\"1.0\">\n\
                 <dict>\n  \
                 <key>Label</key>\n  <string>{LAUNCHD_LABEL}</string>\n  \
                 <key>ProgramArguments</key>\n  <array>\n    <string>{exe}</string>\n  </array>\n\
                 {env}  \
                 <key>RunAtLoad</key>\n  <true/>\n  \
                 <key>KeepAlive</key>\n  <dict>\n    <key>SuccessfulExit</key>\n    <false/>\n  </dict>\n  \
                 <key>ThrottleInterval</key>\n  <integer>5</integer>\n  \
                 <key>StandardOutPath</key>\n  <string>{log}</string>\n  \
                 <key>StandardErrorPath</key>\n  <string>{log}</string>\n\
                 </dict>\n\
                 </plist>\n"
            )
        }
    }
}

/// A path as one `ExecStart=` word. `%` starts a systemd specifier, so it's
/// doubled. A path with whitespace, quotes or backslashes is double-quoted,
/// with `"` and `\` escaped inside.
fn systemd_quote(s: &str) -> String {
    let s = s.replace('%', "%%");
    if s.chars()
        .any(|c| c.is_whitespace() || matches!(c, '"' | '\'' | '\\'))
    {
        format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        s
    }
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// What `--apply` did.
#[derive(Debug, PartialEq, Eq)]
pub enum Applied {
    /// A new unit, with nothing there before.
    Written,
    /// Already exactly this text: nothing written, nothing backed up.
    Unchanged,
    /// Replaced under `--force`, with the old file kept at this path.
    Replaced { backup: PathBuf },
}

/// Write `text` to `path`. An existing different file is refused unless
/// `force`, and then backed up as `<name>.bak.<stamp>` before it's replaced.
pub fn apply(path: &Path, text: &str, force: bool, stamp: &str) -> Result<Applied> {
    match std::fs::read_to_string(path) {
        Ok(old) if old == text => Ok(Applied::Unchanged),
        Ok(_) if !force => bail!(
            "{} already exists and differs. It may be running a daemon with settings of its \
             own (Postgres, binds). Compare it with `muster-daemon service` and re-run with \
             --force to replace it; the old file is backed up first",
            path.display()
        ),
        Ok(_) => {
            let name = path
                .file_name()
                .context("unit path has no file name")?
                .to_string_lossy();
            let backup = path.with_file_name(format!("{name}.bak.{stamp}"));
            std::fs::copy(path, &backup)
                .with_context(|| format!("backing up {} first", path.display()))?;
            write(path, text)?;
            Ok(Applied::Replaced { backup })
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            write(path, text)?;
            Ok(Applied::Written)
        }
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

fn write(path: &Path, text: &str) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    // Write beside it and rename, so a crash never leaves half a unit.
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, text).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("installing {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("muster-service-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn the_systemd_unit_runs_the_binary_and_restarts_on_failure() {
        let text = render(
            Kind::Systemd,
            Path::new("/opt/m/muster-daemon"),
            true,
            Path::new("/l"),
        );
        assert!(text.contains("\nExecStart=/opt/m/muster-daemon\n"));
        assert!(text.contains("\nEnvironment=MUSTER_TLS=1\n"));
        assert!(text.contains("\nRestart=on-failure\n"));
        assert!(text.contains("\nWantedBy=default.target\n"));
        let plain = render(Kind::Systemd, Path::new("/x"), false, Path::new("/l"));
        assert!(!plain.contains("MUSTER_TLS"));
    }

    #[test]
    fn an_awkward_path_stays_one_word_in_exec_start() {
        let text = render(
            Kind::Systemd,
            Path::new("/home/j d/a&b/100%/muster-daemon"),
            false,
            Path::new("/l"),
        );
        assert!(
            text.contains("\nExecStart=\"/home/j d/a&b/100%%/muster-daemon\"\n"),
            "{text}"
        );
        assert_eq!(systemd_quote(r#"/a"b\c"#), r#""/a\"b\\c""#);
        assert_eq!(systemd_quote("/plain/path"), "/plain/path");
    }

    #[test]
    fn the_launchd_agent_keeps_the_daemon_alive() {
        let text = render(
            Kind::Launchd,
            Path::new("/Users/a b/x&<y>/muster-daemon"),
            true,
            Path::new("/Users/a/Library/Logs"),
        );
        assert!(text.contains("<string>dev.muster.daemon</string>"));
        assert!(text.contains("<string>/Users/a b/x&amp;&lt;y&gt;/muster-daemon</string>"));
        assert!(text.contains("<key>RunAtLoad</key>\n  <true/>"));
        assert!(text.contains("<key>SuccessfulExit</key>\n    <false/>"));
        assert!(text.contains("<key>MUSTER_TLS</key>"));
        assert!(text.contains("/Users/a/Library/Logs/muster-daemon.log"));
    }

    #[test]
    fn units_go_where_each_service_manager_looks() {
        let home = Path::new("/h");
        assert_eq!(
            Kind::Systemd.path(home),
            Path::new("/h/.config/systemd/user/muster-daemon.service")
        );
        assert_eq!(
            Kind::Launchd.path(home),
            Path::new("/h/Library/LaunchAgents/dev.muster.daemon.plist")
        );
    }

    #[test]
    fn apply_writes_a_new_unit_and_leaves_an_identical_one() {
        let dir = tmp("new");
        let path = dir.join("u/muster-daemon.service");
        assert_eq!(apply(&path, "a\n", false, "1").unwrap(), Applied::Written);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "a\n");
        assert_eq!(apply(&path, "a\n", false, "2").unwrap(), Applied::Unchanged);
        assert_eq!(
            std::fs::read_dir(path.parent().unwrap()).unwrap().count(),
            1
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn apply_refuses_a_different_unit_without_force_and_backs_it_up_with_it() {
        let dir = tmp("force");
        let path = dir.join("muster-daemon.service");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&path, "old, with MYKO_POSTGRES_URL\n").unwrap();

        let err = apply(&path, "new\n", false, "7").unwrap_err().to_string();
        assert!(err.contains("already exists"), "{err}");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "old, with MYKO_POSTGRES_URL\n"
        );

        let applied = apply(&path, "new\n", true, "7").unwrap();
        let backup = dir.join("muster-daemon.service.bak.7");
        assert_eq!(
            applied,
            Applied::Replaced {
                backup: backup.clone()
            }
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new\n");
        assert_eq!(
            std::fs::read_to_string(&backup).unwrap(),
            "old, with MYKO_POSTGRES_URL\n"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
