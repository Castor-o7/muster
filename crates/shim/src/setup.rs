//! `muster-shim setup [--pair-file PATH] [--apply]`: set this machine up as a
//! Muster client in one command (MUS-15).
//!
//! It pairs with the daemon if given a code and not yet paired, and wires
//! (the code comes from a file, or stdin with `--pair-file -`, never the command
//! line, where shell history, `ps` and agent transcripts would keep it),
//! Claude Code to Muster by installing the `muster-shim` plugin through Claude
//! Code's own `claude plugin` commands, so Muster never rewrites Claude's
//! config files itself. Without `--apply` it only prints what it would do.
//! With it, it backs `settings.json` up before Claude changes it, runs each
//! step, and ends with `muster-shim doctor`'s checks.
//!
//! It leaves wiring made by hand alone, complete or not: a plugin added on top
//! would run every hook twice. Partial hand wiring is reported with the
//! doctor's fixes instead.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use serde_json::Value;

use crate::doctor::{self, Level};

const MARKETPLACE: &str = "Castor-o7/muster";
const MARKETPLACE_NAME: &str = "muster";
const PLUGIN: &str = "muster-shim@muster";

/// One thing setup would do.
#[derive(Debug, PartialEq, Eq)]
pub enum Step {
    /// Pair in this process, with the code read from the file.
    Pair,
    /// Run `claude` with these arguments.
    Claude(Vec<&'static str>),
    /// Add `statusLine` to `settings.json`, so the session's nickname is
    /// always on screen (MUS-38). No `claude` command sets it.
    StatusLine,
}

/// The statusLine command setup adds.
pub const STATUSLINE_COMMAND: &str = "muster-shim statusline";

/// How to run an existing statusline and Muster's side by side: both read
/// Claude's JSON on stdin, so it's read once and given to each.
pub fn chain_hint(existing: &str) -> String {
    format!(
        "input=$(cat); printf '%s' \"$input\" | {existing}; printf ' '; printf '%s' \"$input\" \
         | {STATUSLINE_COMMAND}"
    )
}

/// `settings.json`'s text with `statusLine` added, everything else byte for
/// byte as it was. It's inserted as the object's first key rather than
/// re-serialized, which would reorder the operator's keys. Refuses a file that
/// isn't a JSON object, or that already has a `statusLine`.
pub fn add_statusline(text: &str) -> Result<String> {
    let entry = format!(
        "\"statusLine\": {{ \"type\": \"command\", \"command\": \"{STATUSLINE_COMMAND}\" }}"
    );
    if text.trim().is_empty() {
        return Ok(format!("{{\n  {entry}\n}}\n"));
    }
    let before: Value = serde_json::from_str(text).context("settings.json isn't valid JSON")?;
    let Some(obj) = before.as_object() else {
        bail!("settings.json isn't a JSON object");
    };
    if obj.contains_key("statusLine") {
        bail!("settings.json already has a statusLine");
    }
    let open = text.find('{').context("settings.json has no object")?;
    let sep = if obj.is_empty() { "" } else { "," };
    let out = format!("{}{{\n  {entry}{sep}{}", &text[..open], &text[open + 1..]);
    // Check the result is the same settings plus exactly this key.
    let mut after: Value =
        serde_json::from_str(&out).context("adding statusLine broke the JSON")?;
    after.as_object_mut().map(|o| o.remove("statusLine"));
    if after != before {
        bail!("adding statusLine would change other settings; add it by hand");
    }
    Ok(out)
}

/// What setup found and will do. Pure, so it's tested without a home.
#[derive(Debug, PartialEq, Eq)]
pub struct Plan {
    pub steps: Vec<Step>,
    /// Lines to tell the operator: what's already done, or what to fix by hand.
    pub notes: Vec<String>,
}

pub fn plan(
    paired: bool,
    have_code: bool,
    settings: Option<&Value>,
    claude_json: Option<&Value>,
    marketplace_known: bool,
) -> Plan {
    let mut steps = Vec::new();
    let mut notes = Vec::new();

    match (paired, have_code) {
        (true, true) => notes.push(
            "already paired, so the code isn't used; `muster-shim unpair` first to pair again"
                .into(),
        ),
        (true, false) => notes.push("already paired".into()),
        (false, true) => steps.push(Step::Pair),
        (false, false) => notes.push(
            "not paired: fine on the daemon's own host. Elsewhere, run `muster-daemon pair \
             --name <this machine>` on the daemon's host, save the code to a file and \
             pass it with --pair-file"
                .into(),
        ),
    }

    let plugin_on = settings.is_some_and(|s| s["enabledPlugins"][PLUGIN].as_bool() == Some(true));
    let has_hand_wiring = settings.is_some_and(|s| {
        s["hooks"].as_object().is_some_and(|h| {
            h.values()
                .any(|v| v.to_string().contains("muster-shim claude-hook"))
        })
    }) || claude_json.is_some_and(|c| {
        c["mcpServers"].as_object().is_some_and(|m| {
            m.values().any(|v| {
                v["command"]
                    .as_str()
                    .is_some_and(|c| c.ends_with("muster-shim"))
            })
        })
    });
    if plugin_on {
        notes.push(format!("Claude Code already has the {PLUGIN} plugin"));
    } else if has_hand_wiring {
        let problems: Vec<String> = doctor::claude_wiring(settings, claude_json)
            .into_iter()
            .filter(|f| f.level == Level::Fail)
            .map(|f| format!("{}; fix: {}", f.detail, f.fix.unwrap_or_default()))
            .collect();
        if problems.is_empty() {
            notes.push("Claude Code is already wired to Muster by hand".into());
        } else {
            notes.push(
                "Claude Code is partly wired to Muster by hand, so setup leaves it alone \
                 (the plugin on top would run each hook twice). Fix it by hand:"
                    .into(),
            );
            notes.extend(problems.into_iter().map(|p| format!("  {p}")));
        }
    } else {
        if !marketplace_known {
            steps.push(Step::Claude(vec![
                "plugin",
                "marketplace",
                "add",
                MARKETPLACE,
            ]));
        }
        steps.push(Step::Claude(vec!["plugin", "install", PLUGIN]));
    }

    // MUS-38: the statusline is where the operator sees which session is which.
    match settings.and_then(|s| s.get("statusLine")) {
        None => steps.push(Step::StatusLine),
        Some(line) => {
            let cmd = line["command"].as_str().unwrap_or("").trim();
            if cmd.contains(STATUSLINE_COMMAND) {
                notes.push("the statusline already shows Muster's nickname".into());
            } else {
                notes.push(format!(
                    "settings.json already has a statusLine, so setup leaves it alone. To show \
                     the session's nickname beside it, set its command to: {}",
                    chain_hint(if cmd.is_empty() {
                        "<your command>"
                    } else {
                        cmd
                    })
                ));
            }
        }
    }
    Plan { steps, notes }
}

fn claude_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("CLAUDE_CONFIG_DIR").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".claude"))
}

fn claude_json_path() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("CLAUDE_CONFIG_DIR").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(dir).join(".claude.json"));
    }
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".claude.json"))
}

fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

fn marketplace_known(dir: &Path) -> bool {
    read_json(&dir.join("plugins/known_marketplaces.json"))
        .is_some_and(|v| v.get(MARKETPLACE_NAME).is_some())
}

/// Copy `path` to `<name>.bak.<stamp>` beside it, if it exists.
pub fn backup(path: &Path, stamp: &str) -> Result<Option<PathBuf>> {
    if !path.exists() {
        return Ok(None);
    }
    let name = path
        .file_name()
        .context("no file name")?
        .to_string_lossy()
        .into_owned();
    let to = path.with_file_name(format!("{name}.bak.{stamp}"));
    std::fs::copy(path, &to).with_context(|| format!("backing up {}", path.display()))?;
    Ok(Some(to))
}

fn describe(step: &Step) -> String {
    match step {
        Step::Pair => "pair with the code from --pair-file".into(),
        Step::Claude(args) => format!("claude {}", args.join(" ")),
        Step::StatusLine => format!(
            "add \"statusLine\": {STATUSLINE_COMMAND} to settings.json, so the nickname shows"
        ),
    }
}

pub async fn run(args: &[String]) -> Result<()> {
    let mut apply = false;
    let mut code_file = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--apply" => apply = true,
            "--pair-file" => {
                code_file = Some(
                    it.next()
                        .context("--pair-file needs a path, or - for stdin")?,
                )
            }
            a if a == "--pair" || a.starts_with("--pair=") => bail!(
                "a pairing code on the command line stays in shell history and `ps`: save it \
                 to a file and pass --pair-file PATH (delete the file afterwards), or pipe it \
                 in with --pair-file -"
            ),
            _ => bail!("usage: muster-shim setup [--pair-file PATH|-] [--apply]"),
        }
    }
    // Read now, so a bad path fails before anything changes.
    let code = match code_file {
        None => None,
        Some(p) if p == "-" => Some(crate::read_code(std::io::stdin())?),
        Some(p) => Some(crate::read_code(
            std::fs::File::open(p).with_context(|| format!("opening {p}"))?,
        )?),
    };
    let dir = claude_dir().context("HOME isn't set")?;
    let settings_path = dir.join("settings.json");
    let settings = read_json(&settings_path);
    let claude_json = claude_json_path().as_deref().and_then(read_json);
    let plan = plan(
        crate::link::load_pairing().is_some(),
        code.is_some(),
        settings.as_ref(),
        claude_json.as_ref(),
        marketplace_known(&dir),
    );

    for n in &plan.notes {
        println!("{n}");
    }
    if plan.steps.is_empty() {
        println!("nothing to do");
    } else if !apply {
        println!("would run (dry run: nothing changed; add --apply to run these):");
        for s in &plan.steps {
            println!("  {}", describe(s));
        }
        return Ok(());
    }

    if apply && !plan.steps.is_empty() {
        let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
        if plan
            .steps
            .iter()
            .any(|s| matches!(s, Step::Claude(_) | Step::StatusLine))
            && let Some(b) = backup(&settings_path, &stamp)?
        {
            println!("backed up {} to {}", settings_path.display(), b.display());
        }
        for step in &plan.steps {
            println!("running: {}", describe(step));
            match step {
                Step::Pair => {
                    let said = crate::pair_with_code(code.as_deref().unwrap_or_default())
                        .context("pairing failed; stopped there")?;
                    println!("{said}");
                    if let Some(p) = code_file.filter(|p| *p != "-") {
                        println!("the code is used up now; you can delete {p}");
                    }
                }
                Step::StatusLine => {
                    // Read now: a plugin install just before may have changed it.
                    let text = match std::fs::read_to_string(&settings_path) {
                        Ok(t) => t,
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
                        Err(e) => return Err(e).context("reading settings.json"),
                    };
                    let out = add_statusline(&text).context("adding the statusline")?;
                    if let Some(dir) = settings_path.parent() {
                        std::fs::create_dir_all(dir)?;
                    }
                    let tmp = settings_path.with_extension("json.tmp");
                    std::fs::write(&tmp, out)?;
                    std::fs::rename(&tmp, &settings_path)?;
                }
                Step::Claude(args) => {
                    let status = Command::new("claude")
                        .args(args)
                        .status()
                        .with_context(|| format!("starting `{}`", describe(step)))?;
                    if !status.success() {
                        bail!("`{}` failed ({status}); stopped there", describe(step));
                    }
                }
            }
        }
        println!("done. Restart Claude Code sessions to pick it up.");
    }
    println!("\nchecking with `muster-shim doctor`:");
    doctor::run().await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// No pairing or plugin step (the statusline step is tested on its own).
    fn no_wiring(p: &Plan) -> bool {
        !p.steps
            .iter()
            .any(|s| matches!(s, Step::Pair | Step::Claude(_)))
    }

    fn hooks_json() -> Value {
        serde_json::from_str(include_str!(
            "../../../plugins/muster-shim/hooks/hooks.json"
        ))
        .unwrap()
    }

    #[test]
    fn a_clean_machine_pairs_and_installs_the_plugin() {
        let p = plan(false, true, None, None, false);
        assert_eq!(
            p.steps,
            vec![
                Step::Pair,
                Step::Claude(vec!["plugin", "marketplace", "add", MARKETPLACE]),
                Step::Claude(vec!["plugin", "install", PLUGIN]),
                Step::StatusLine,
            ]
        );
    }

    #[test]
    fn a_known_marketplace_isnt_added_again() {
        let p = plan(true, false, Some(&json!({})), None, true);
        assert_eq!(
            p.steps,
            vec![
                Step::Claude(vec!["plugin", "install", PLUGIN]),
                Step::StatusLine
            ]
        );
    }

    #[test]
    fn complete_hand_wiring_is_left_alone() {
        let claude = json!({ "mcpServers": { "muster": { "command": "muster-shim" } } });
        let p = plan(true, false, Some(&hooks_json()), Some(&claude), false);
        assert!(no_wiring(&p), "{p:?}");
        assert!(
            p.notes
                .iter()
                .any(|n| n.contains("already wired to Muster by hand"))
        );
    }

    #[test]
    fn partial_hand_wiring_is_reported_not_changed() {
        let mut settings = hooks_json();
        settings["hooks"].as_object_mut().unwrap().remove("Stop");
        let p = plan(true, false, Some(&settings), None, false);
        assert!(no_wiring(&p), "{p:?}");
        assert!(p.notes.iter().any(|n| n.contains("partly wired")));
        assert!(
            p.notes
                .iter()
                .any(|n| n.contains("Stop: muster-shim claude-hook wait"))
        );
        assert!(p.notes.iter().any(|n| n.contains("no muster MCP server")));
    }

    #[test]
    fn the_plugin_already_on_and_a_second_code_do_nothing() {
        let settings = json!({ "enabledPlugins": { PLUGIN: true } });
        let p = plan(true, true, Some(&settings), None, true);
        assert!(no_wiring(&p), "{p:?}");
        assert!(p.notes.iter().any(|n| n.contains("code isn't used")));
    }

    #[test]
    fn statusline_is_added_with_the_other_keys_kept_as_they_were() {
        let text = "{\n  \"theme\": \"dark\",\n  \"z\": [1, 2]\n}\n";
        let out = add_statusline(text).unwrap();
        assert!(
            out.ends_with("  \"theme\": \"dark\",\n  \"z\": [1, 2]\n}\n"),
            "{out}"
        );
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["statusLine"]["command"], STATUSLINE_COMMAND);
        assert_eq!(v["theme"], "dark");
        // Empty object and missing file.
        let v: Value = serde_json::from_str(&add_statusline("{}").unwrap()).unwrap();
        assert_eq!(v["statusLine"]["type"], "command");
        let v: Value = serde_json::from_str(&add_statusline("").unwrap()).unwrap();
        assert_eq!(v["statusLine"]["command"], STATUSLINE_COMMAND);
    }

    #[test]
    fn statusline_refuses_to_replace_one_or_touch_bad_json() {
        let existing = r#"{"statusLine": {"type": "command", "command": "mine"}}"#;
        assert!(add_statusline(existing).is_err());
        assert!(add_statusline("[1]").is_err());
        assert!(add_statusline("{ not json").is_err());
    }

    #[test]
    fn the_plan_adds_the_statusline_only_where_there_is_none() {
        let p = plan(true, false, Some(&json!({})), None, true);
        assert!(p.steps.contains(&Step::StatusLine), "{p:?}");

        let mine = json!({ "statusLine": { "type": "command", "command": "~/bin/line.sh" } });
        let p = plan(true, false, Some(&mine), None, true);
        assert!(!p.steps.contains(&Step::StatusLine), "{p:?}");
        assert!(
            p.notes.iter().any(|n| n.contains("leaves it alone")
                && n.contains("| ~/bin/line.sh;")
                && n.contains("| muster-shim statusline")),
            "{p:?}"
        );

        let ours =
            json!({ "statusLine": { "type": "command", "command": "muster-shim statusline" } });
        let p = plan(true, false, Some(&ours), None, true);
        assert!(!p.steps.contains(&Step::StatusLine));
        assert!(p.notes.iter().any(|n| n.contains("already shows")));
    }

    #[test]
    fn a_code_is_read_from_its_first_non_empty_line() {
        let code = crate::read_code("\n  muster-pair:v2:abc  \n".as_bytes()).unwrap();
        assert_eq!(code, "muster-pair:v2:abc");
        assert!(crate::read_code("\n \n".as_bytes()).is_err());
    }

    #[test]
    fn backup_copies_beside_and_skips_a_missing_file() {
        let dir = std::env::temp_dir().join(format!("muster-setup-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("settings.json");
        assert_eq!(backup(&f, "1").unwrap(), None);
        std::fs::write(&f, "{}").unwrap();
        let b = backup(&f, "1").unwrap().unwrap();
        assert_eq!(b, dir.join("settings.json.bak.1"));
        assert_eq!(std::fs::read_to_string(b).unwrap(), "{}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
