//! `muster-shim doctor`: report what's wrong with this machine's Muster setup
//! (MUS-15). It only reads. It registers no session, changes no file, and
//! prints one line per check, with a fix for each problem it finds. The exit
//! status is non-zero if any check fails.
//!
//! Checks, in order: this binary is the `muster-shim` on `PATH`; there's a
//! credential (a pairing, or the daemon's local token); the daemon accepts a
//! WebSocket handshake with it; and Claude Code is wired to Muster, with the MCP
//! server and every hook in `plugins/muster-shim/hooks/hooks.json`, once each,
//! and no leftover Marshal entry (MUS-20).

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Result;
use serde_json::Value;

/// How long the daemon may take to answer the handshake.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Ok,
    Warn,
    Fail,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Finding {
    pub level: Level,
    pub check: &'static str,
    pub detail: String,
    pub fix: Option<String>,
}

impl Finding {
    fn ok(check: &'static str, detail: impl Into<String>) -> Self {
        Finding {
            level: Level::Ok,
            check,
            detail: detail.into(),
            fix: None,
        }
    }
    fn warn(check: &'static str, detail: impl Into<String>, fix: impl Into<String>) -> Self {
        Finding {
            level: Level::Warn,
            check,
            detail: detail.into(),
            fix: Some(fix.into()),
        }
    }
    fn fail(check: &'static str, detail: impl Into<String>, fix: impl Into<String>) -> Self {
        Finding {
            level: Level::Fail,
            check,
            detail: detail.into(),
            fix: Some(fix.into()),
        }
    }
}

/// The Claude Code hooks Muster needs: (event, command, whether it must be
/// `asyncRewake`). The same set as `plugins/muster-shim/hooks/hooks.json`.
const CLAUDE_HOOKS: &[(&str, &str, bool)] = &[
    (
        "SessionStart",
        "muster-shim claude-hook session-start",
        false,
    ),
    ("SessionStart", "muster-shim claude-hook wait", true),
    ("Stop", "muster-shim claude-hook wait", true),
    (
        "UserPromptSubmit",
        "muster-shim claude-hook prompt-submit",
        false,
    ),
    (
        "PostToolUse",
        "muster-shim claude-hook post-tool-use",
        false,
    ),
];

/// The plugin id that supplies the hooks and MCP server when installed.
const PLUGIN_ID: &str = "muster-shim@muster";

/// Every `(command, asyncRewake)` under `event` in a settings file's `hooks`.
fn hook_commands<'a>(settings: &'a Value, event: &str) -> Vec<(&'a str, bool)> {
    settings["hooks"][event]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|group| group["hooks"].as_array().into_iter().flatten())
        .filter_map(|h| {
            let cmd = h["command"].as_str()?;
            Some((cmd.trim(), h["asyncRewake"].as_bool().unwrap_or(false)))
        })
        .collect()
}

fn plugin_enabled(settings: Option<&Value>) -> bool {
    settings.is_some_and(|s| s["enabledPlugins"][PLUGIN_ID].as_bool() == Some(true))
}

/// Claude Code's wiring, from `settings.json` and `.claude.json` (either may be
/// missing). Pure, so it's tested without a home directory.
pub fn claude_wiring(settings: Option<&Value>, claude_json: Option<&Value>) -> Vec<Finding> {
    let mut out = Vec::new();
    let plugin = plugin_enabled(settings);

    // The MCP server: the plugin, or an `mcpServers` entry.
    let servers = claude_json.and_then(|c| c["mcpServers"].as_object());
    let has_server = servers.is_some_and(|m| {
        m.values().any(|v| {
            v["command"]
                .as_str()
                .is_some_and(|c| c.ends_with("muster-shim"))
        })
    });
    out.push(match (plugin, has_server) {
        (true, true) => Finding::warn(
            "claude mcp",
            "the muster MCP server is configured twice: by the plugin and in ~/.claude.json",
            "keep one: `claude mcp remove -s user muster`, or uninstall the plugin",
        ),
        (true, false) => Finding::ok("claude mcp", format!("from the {PLUGIN_ID} plugin")),
        (false, true) => Finding::ok("claude mcp", "muster-shim in ~/.claude.json"),
        (false, false) => Finding::fail(
            "claude mcp",
            "no muster MCP server for Claude Code",
            "`claude mcp add -s user muster muster-shim`, or install the plugin",
        ),
    });

    // Leftover Marshal wiring (MUS-20): its tools sit beside Muster's.
    let marshal: Vec<&str> = servers
        .into_iter()
        .flatten()
        .filter(|(_, v)| {
            v["command"]
                .as_str()
                .is_some_and(|c| c.ends_with("marshal-shim"))
        })
        .map(|(k, _)| k.as_str())
        .collect();
    if !marshal.is_empty() {
        out.push(Finding::warn(
            "claude mcp",
            format!("Marshal is still configured: {}", marshal.join(", ")),
            format!(
                "`claude mcp remove -s user {}` once nothing needs Marshal (MUS-20)",
                marshal[0]
            ),
        ));
    }

    // The hooks: the plugin supplies them, so with the plugin on, any in
    // settings.json run twice.
    for &(event, command, rewake) in CLAUDE_HOOKS {
        let found: Vec<bool> = settings
            .map(|s| hook_commands(s, event))
            .unwrap_or_default()
            .into_iter()
            .filter(|(c, _)| *c == command)
            .map(|(_, r)| r)
            .collect();
        let check = "claude hook";
        let name = format!("{event}: {command}");
        out.push(match (plugin, found.as_slice()) {
            (true, []) => continue,
            (true, _) => Finding::warn(
                check,
                format!("{name} is in settings.json and also comes from the plugin"),
                "remove it from ~/.claude/settings.json; the plugin runs it",
            ),
            (false, []) => Finding::fail(
                check,
                format!("{name} is missing"),
                "add it to ~/.claude/settings.json as plugins/muster-shim/hooks/hooks.json \
                 has it, or install the plugin",
            ),
            (false, [r]) if *r != rewake => Finding::fail(
                check,
                format!("{name} has asyncRewake {r}, needs {rewake}"),
                "fix it to match plugins/muster-shim/hooks/hooks.json",
            ),
            (false, [_]) => continue,
            (false, more) => Finding::warn(
                check,
                format!("{name} is there {} times", more.len()),
                "keep one in ~/.claude/settings.json",
            ),
        });
    }
    if plugin || out.iter().all(|f| f.check != "claude hook") {
        out.push(Finding::ok(
            "claude hook",
            if plugin {
                "from the plugin"
            } else {
                "all five in ~/.claude/settings.json"
            },
        ));
    }
    out
}

/// Whether this binary is the `muster-shim` that Claude Code will run.
pub fn path_check(exe: Option<&Path>, on_path: Option<&Path>) -> Finding {
    let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    match (exe, on_path) {
        (_, None) => Finding::fail(
            "binary",
            "no muster-shim on PATH, and Claude Code runs it by name",
            "`cargo install --git https://github.com/Castor-o7/muster muster-shim`, \
             and put ~/.cargo/bin on PATH",
        ),
        (Some(exe), Some(found)) if canon(exe) != canon(found) => Finding::warn(
            "binary",
            format!(
                "this is {}, but PATH finds {} first",
                exe.display(),
                found.display()
            ),
            "Claude Code runs the one on PATH; check that's the one you meant",
        ),
        (_, Some(found)) => Finding::ok("binary", found.display().to_string()),
    }
}

fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join(name))
        .find(|p| p.is_file())
}

fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

/// Claude Code's config files, honouring `CLAUDE_CONFIG_DIR`.
fn claude_files() -> (Option<PathBuf>, Option<PathBuf>) {
    if let Some(dir) = std::env::var_os("CLAUDE_CONFIG_DIR").filter(|d| !d.is_empty()) {
        let dir = PathBuf::from(dir);
        return (
            Some(dir.join("settings.json")),
            Some(dir.join(".claude.json")),
        );
    }
    let home = std::env::var_os("HOME").map(PathBuf::from);
    (
        home.as_ref()
            .map(|h| h.join(".claude").join("settings.json")),
        home.map(|h| h.join(".claude.json")),
    )
}

fn credential_check() -> Finding {
    if let Some(p) = crate::link::load_pairing() {
        return Finding::ok(
            "credential",
            format!("paired as {} with {}", p.name, p.address),
        );
    }
    match muster_entities::pairing::read_local_token() {
        Some(_) => Finding::ok(
            "credential",
            "not paired; the daemon's local token is readable",
        ),
        None => Finding::warn(
            "credential",
            "not paired, and no local token: fine for a plain-mode daemon on this host only",
            "on another machine, pair: `muster-daemon pair` on the daemon's host, then \
             `muster-shim pair <code>` here",
        ),
    }
}

async fn daemon_check() -> Finding {
    let (url, credential) = match crate::daemon_address().await {
        Ok(v) => v,
        Err(e) => {
            return Finding::fail(
                "daemon",
                format!("couldn't build the daemon address: {e:#}"),
                "check the pairing with `muster-shim pairing`",
            );
        }
    };
    // A paired connection goes through a loopback tunnel, so name the gate.
    let shown = match crate::link::load_pairing() {
        Some(p) => format!("the TLS gate at {}", p.address),
        None => url.split('?').next().unwrap_or_default().to_string(),
    };
    let outcome = tokio::time::timeout(
        HANDSHAKE_TIMEOUT,
        tokio_tungstenite::connect_async(url.as_str()),
    )
    .await;
    match outcome {
        Ok(Ok((mut socket, _))) => {
            let _ = socket.close(None).await;
            Finding::ok("daemon", format!("accepted a handshake at {shown}"))
        }
        Ok(Err(tokio_tungstenite::tungstenite::Error::Http(r))) if r.status().as_u16() == 401 => {
            Finding::fail(
                "daemon",
                crate::refusal::message(&credential, &url),
                "see the message above",
            )
        }
        Ok(Err(e)) => Finding::fail(
            "daemon",
            format!("no handshake at {shown}: {e}"),
            "start the daemon (`muster-daemon`), or check the address and the firewall",
        ),
        Err(_) => Finding::fail(
            "daemon",
            format!("no answer from {shown} within {HANDSHAKE_TIMEOUT:?}"),
            "check the daemon is running and reachable from here",
        ),
    }
}

fn render(f: &Finding) -> String {
    let mark = match f.level {
        Level::Ok => "ok  ",
        Level::Warn => "warn",
        Level::Fail => "FAIL",
    };
    match &f.fix {
        Some(fix) => format!(
            "{mark} {:<12} {}\n     {:<12} fix: {fix}",
            f.check, f.detail, ""
        ),
        None => format!("{mark} {:<12} {}", f.check, f.detail),
    }
}

pub async fn run() -> Result<()> {
    muster_entities::link();
    let mut findings = vec![path_check(
        std::env::current_exe().ok().as_deref(),
        which("muster-shim").as_deref(),
    )];
    findings.push(credential_check());
    findings.push(daemon_check().await);
    let (settings, claude_json) = claude_files();
    findings.extend(claude_wiring(
        settings.as_deref().and_then(read_json).as_ref(),
        claude_json.as_deref().and_then(read_json).as_ref(),
    ));
    for f in &findings {
        println!("{}", render(f));
    }
    let failed = findings.iter().filter(|f| f.level == Level::Fail).count();
    if failed > 0 {
        anyhow::bail!("{failed} check(s) failed");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn hooks_json() -> Value {
        let text = include_str!("../../../plugins/muster-shim/hooks/hooks.json");
        serde_json::from_str(text).unwrap()
    }

    fn levels(f: &[Finding]) -> Vec<(Level, &str)> {
        f.iter().map(|f| (f.level, f.detail.as_str())).collect()
    }

    #[test]
    fn the_plugins_hooks_are_exactly_what_doctor_wants() {
        // If hooks.json changes, CLAUDE_HOOKS must change with it.
        let plugin = hooks_json();
        let mut found = Vec::new();
        for event in plugin["hooks"].as_object().unwrap().keys() {
            for (cmd, rewake) in hook_commands(&plugin, event) {
                found.push((event.clone(), cmd.to_string(), rewake));
            }
        }
        let mut want: Vec<_> = CLAUDE_HOOKS
            .iter()
            .map(|(e, c, r)| (e.to_string(), c.to_string(), *r))
            .collect();
        found.sort();
        want.sort();
        assert_eq!(found, want);
    }

    #[test]
    fn hooks_copied_from_the_plugin_pass() {
        let claude = json!({ "mcpServers": { "muster": { "command": "muster-shim" } } });
        let f = claude_wiring(Some(&hooks_json()), Some(&claude));
        assert!(f.iter().all(|f| f.level == Level::Ok), "{:?}", levels(&f));
    }

    #[test]
    fn a_missing_hook_and_server_fail() {
        let f = claude_wiring(None, None);
        let fails = f.iter().filter(|f| f.level == Level::Fail).count();
        assert_eq!(fails, 1 + CLAUDE_HOOKS.len(), "{:?}", levels(&f));
    }

    #[test]
    fn a_wait_hook_without_async_rewake_fails() {
        let mut settings = hooks_json();
        settings["hooks"]["Stop"][0]["hooks"][0]["asyncRewake"] = json!(false);
        let claude = json!({ "mcpServers": { "muster": { "command": "muster-shim" } } });
        let f = claude_wiring(Some(&settings), Some(&claude));
        assert!(
            f.iter()
                .any(|f| f.level == Level::Fail && f.detail.starts_with("Stop:")),
            "{:?}",
            levels(&f)
        );
    }

    #[test]
    fn leftover_marshal_and_doubled_hooks_warn() {
        let mut settings = hooks_json();
        settings["enabledPlugins"] = json!({ PLUGIN_ID: true });
        let claude = json!({ "mcpServers": {
            "muster": { "command": "muster-shim" },
            "marshal": { "command": "marshal-shim" }
        } });
        let f = claude_wiring(Some(&settings), Some(&claude));
        assert!(f.iter().all(|f| f.level != Level::Fail), "{:?}", levels(&f));
        assert!(f.iter().any(|f| f.detail.contains("configured twice")));
        assert!(
            f.iter()
                .any(|f| f.detail.contains("Marshal is still configured: marshal"))
        );
        assert!(
            f.iter()
                .any(|f| f.detail.contains("also comes from the plugin"))
        );
    }

    #[test]
    fn the_plugin_alone_passes() {
        let settings = json!({ "enabledPlugins": { PLUGIN_ID: true } });
        let f = claude_wiring(Some(&settings), Some(&json!({})));
        assert!(f.iter().all(|f| f.level == Level::Ok), "{:?}", levels(&f));
    }

    #[test]
    fn path_check_needs_muster_shim_on_path() {
        assert_eq!(path_check(None, None).level, Level::Fail);
        let a = Path::new("/nonexistent/a/muster-shim");
        let b = Path::new("/nonexistent/b/muster-shim");
        assert_eq!(path_check(Some(a), Some(b)).level, Level::Warn);
        assert_eq!(path_check(Some(a), Some(a)).level, Level::Ok);
    }
}
