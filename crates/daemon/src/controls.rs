//! The operator's controls over peer messages (MUS-10), read from files in
//! the daemon's state directory, which `muster-daemon pause`, `resume`,
//! `allow` and `disallow` write:
//!
//! - `wakes-paused`: present while wake-ups are paused; holds when, in ms.
//! - `allowed-senders`: present while an allowlist is set; one machine per line.
//! - `forget-sessions`: sessions the operator asked to remove (MUS-105), one
//!   `<session id, id prefix or nickname>\t<ms>` per line, kept for
//!   [`FORGET_HOLD`].
//!
//! The running daemon reads them every [`REFRESH`], applies them to
//! `muster_entities::controls`, and publishes the pause as a `WakePause` record
//! for rosters. The daemon's own host name is always allowed, so an allowlist
//! never cuts off the sessions running beside the daemon.

use std::io;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use muster_entities::{WAKE_PAUSE_ID, WakePause, WakePauseId};
use myko::server::MykoServerContext;
use myko::wire::{MEvent, MEventType};

pub const PAUSED_FILE: &str = "wakes-paused";
pub const ALLOWED_FILE: &str = "allowed-senders";
pub const FORGET_FILE: &str = "forget-sessions";
/// How long a forgotten session's hooks are refused, so a waiter still
/// polling under its id (MUS-104) can't put the row straight back.
pub const FORGET_HOLD: Duration = Duration::from_secs(10 * 60);
/// How soon the running daemon sees a change.
pub const REFRESH: Duration = Duration::from_secs(2);

/// This host's short name, as the shim reports it for sessions here.
pub fn this_host() -> Option<String> {
    std::env::var("MUSTER_HOST")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| {
            gethostname::gethostname()
                .into_string()
                .ok()
                .and_then(|h| h.split('.').next().map(str::to_string))
        })
        .filter(|s| !s.is_empty())
}

/// When wake-ups were paused, or `None` if they aren't.
pub fn paused_since(dir: &Path) -> Option<i64> {
    let text = std::fs::read_to_string(dir.join(PAUSED_FILE)).ok()?;
    Some(text.trim().parse().unwrap_or(0))
}

pub fn set_paused(dir: &Path, paused: bool) -> io::Result<()> {
    let path = dir.join(PAUSED_FILE);
    if paused {
        std::fs::create_dir_all(dir)?;
        if !path.exists() {
            std::fs::write(&path, chrono::Utc::now().timestamp_millis().to_string())?;
        }
        Ok(())
    } else {
        match std::fs::remove_file(&path) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }
}

/// The machines the operator listed, or `None` when no allowlist is set.
pub fn allowed(dir: &Path) -> Option<Vec<String>> {
    let text = std::fs::read_to_string(dir.join(ALLOWED_FILE)).ok()?;
    Some(
        text.lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(str::to_string)
            .collect(),
    )
}

/// Add or remove a machine. Removing the last one keeps an empty allowlist,
/// which lets only this host send; `clear_allowed` removes the list.
pub fn set_allowed(dir: &Path, machine: &str, allow: bool) -> io::Result<Vec<String>> {
    let mut list = allowed(dir).unwrap_or_default();
    list.retain(|m| !m.eq_ignore_ascii_case(machine));
    if allow {
        list.push(machine.to_string());
    }
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join(ALLOWED_FILE), list.join("\n") + "\n")?;
    Ok(list)
}

pub fn clear_allowed(dir: &Path) -> io::Result<()> {
    match std::fs::remove_file(dir.join(ALLOWED_FILE)) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// Ask the running daemon to forget a session (MUS-105): `target` is its id,
/// an id prefix of at least 8 characters, or its nickname.
pub fn forget(dir: &Path, target: &str) -> io::Result<()> {
    use std::io::Write as _;
    std::fs::create_dir_all(dir)?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join(FORGET_FILE))?;
    writeln!(
        file,
        "{}\t{}",
        target.trim(),
        chrono::Utc::now().timestamp_millis()
    )
}

fn forget_requests(dir: &Path) -> Vec<(String, i64)> {
    std::fs::read_to_string(dir.join(FORGET_FILE))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            let (target, at) = l.split_once('\t')?;
            Some((target.trim().to_string(), at.trim().parse().ok()?))
        })
        .filter(|(t, _)| !t.is_empty())
        .collect()
}

/// Session ids whose hooks are refused while their forget holds.
static HELD: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// Whether the operator forgot this session within [`FORGET_HOLD`].
pub fn is_forgotten(session_id: &str) -> bool {
    HELD.lock()
        .map(|h| h.iter().any(|id| id == session_id))
        .unwrap_or(false)
}

/// Which session a forget names: an exact id, else the one session on the
/// roster with that nickname, else the one id that starts with it (at least 8
/// characters). `Err` holds the candidates when it names several, which
/// removes nothing: nicknames repeat across restarts (a ghost and its live
/// successor were both `mellow-weevil` on 2026-11-06), so a guess could remove
/// the live one.
pub fn resolve_target(
    target: &str,
    ids: &[String],
    nicknames: &[(String, String)],
) -> Result<Option<String>, Vec<String>> {
    if ids.iter().any(|id| id == target) {
        return Ok(Some(target.to_string()));
    }
    let one = |mut found: Vec<String>| match found.len() {
        0 => Ok(None),
        1 => Ok(found.pop()),
        _ => Err(found),
    };
    let named: Vec<String> = nicknames
        .iter()
        .filter(|(id, nick)| nick == target && ids.contains(id))
        .map(|(id, _)| id.clone())
        .collect();
    if !named.is_empty() || target.len() < 8 {
        return one(named);
    }
    one(ids
        .iter()
        .filter(|id| id.starts_with(target))
        .cloned()
        .collect())
}

/// Remove the sessions the operator forgot, and hold their ids off.
fn apply_forgets(dir: &Path, ctx: &MykoServerContext) {
    use hyphae::{Gettable as _, Materialize as _};
    use muster_entities::{Session, SessionNickname};
    use myko::{core::item::Eventable as _, utils::downcast_item};

    let now = chrono::Utc::now().timestamp_millis();
    let requests = forget_requests(dir);
    let live: Vec<(String, i64)> = requests
        .iter()
        .filter(|(_, at)| now - at < FORGET_HOLD.as_millis() as i64)
        .cloned()
        .collect();
    if live.len() != requests.len() {
        let text: String = live.iter().map(|(t, at)| format!("{t}\t{at}\n")).collect();
        let _ = if text.is_empty() {
            std::fs::remove_file(dir.join(FORGET_FILE))
        } else {
            std::fs::write(dir.join(FORGET_FILE), text)
        };
    }
    if live.is_empty() {
        if let Ok(mut held) = HELD.lock() {
            held.clear();
        }
        return;
    }
    let ids: Vec<String> = ctx
        .registry
        .get(Session::ENTITY_NAME_STATIC)
        .map(|s| {
            s.entries()
                .materialize()
                .get()
                .into_iter()
                .map(|(id, _)| id.to_string())
                .collect()
        })
        .unwrap_or_default();
    let nicknames: Vec<(String, String)> = ctx
        .registry
        .get(SessionNickname::ENTITY_NAME_STATIC)
        .map(|s| {
            s.entries()
                .materialize()
                .get()
                .into_iter()
                .filter_map(|(id, item)| {
                    downcast_item::<SessionNickname>(&item)
                        .map(|n| (id.to_string(), n.nickname.clone()))
                })
                .collect()
        })
        .unwrap_or_default();
    let mut held = Vec::new();
    for (target, _) in &live {
        let id = match resolve_target(target, &ids, &nicknames) {
            Ok(Some(id)) => id,
            // A full id stays held after its row is gone, when it no longer resolves.
            Ok(None) => target.clone(),
            Err(candidates) => {
                log::warn!(
                    "[controls] not forgetting {target}: it names {} sessions ({}); forget one by its id",
                    candidates.len(),
                    candidates.join(", ")
                );
                continue;
            }
        };
        // Deleted again on every refresh while the forget holds, should
        // anything have put the row back.
        if ids.contains(&id) {
            match ctx.del_by_id(Session::ENTITY_NAME_STATIC, &id) {
                Ok(()) => log::warn!(
                    "[controls] forgot session {id} ({target}) at the operator's request"
                ),
                Err(e) => log::warn!("[controls] couldn't forget session {id}: {e}"),
            }
        }
        held.push(id);
    }
    if let Ok(mut h) = HELD.lock() {
        *h = held;
    }
}

/// Apply the files once, and publish the pause if it changed since `last`.
/// Without `allowlist` (plain mode, MUS-64) the sender list is ignored: with no
/// pairing there are no machine names to check senders against.
pub fn apply(dir: &Path, ctx: &MykoServerContext, last: &mut Option<(bool, i64)>, allowlist: bool) {
    let since = paused_since(dir);
    muster_entities::controls::set_wakes_paused(since.is_some());
    let senders = allowed(dir).filter(|_| allowlist).map(|mut list| {
        if let Some(host) = this_host()
            && !list.iter().any(|m| m.eq_ignore_ascii_case(&host))
        {
            list.push(host);
        }
        list
    });
    muster_entities::controls::set_allowed_senders(senders);
    apply_forgets(dir, ctx);
    let now = (since.is_some(), since.unwrap_or(0));
    if *last != Some(now) {
        let record = WakePause {
            id: WakePauseId(Arc::from(WAKE_PAUSE_ID)),
            paused: now.0,
            since: now.1,
        };
        let event = MEvent::from_item(&record, MEventType::SET, &uuid::Uuid::new_v4().to_string());
        match ctx.apply_event_batch(vec![event]) {
            Ok(_) => *last = Some(now),
            Err(e) => log::warn!("[controls] couldn't publish the wake pause: {e:?}"),
        }
        if now.0 {
            log::warn!("[controls] wake-ups are paused");
        } else if last.is_some() {
            log::info!("[controls] wake-ups are on");
        }
    }
}

/// Keep applying the files, every [`REFRESH`]. Spawn on a tokio task.
pub async fn run(dir: std::path::PathBuf, ctx: MykoServerContext, allowlist: bool) {
    let mut last = None;
    let mut interval = tokio::time::interval(REFRESH);
    loop {
        interval.tick().await;
        apply(&dir, &ctx, &mut last, allowlist);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("muster-controls-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn a_forget_names_one_session_or_removes_nothing() {
        let ids = vec![
            "64a05eda-eab4-46f8-9aa9-0ee0ec43eea5".to_string(),
            "64a05eda-0000-0000-0000-000000000000".to_string(),
            "02f2a191-6522-43d9-9a5d-7e1713f90121".to_string(),
            "7195eda2-fd79-4bc1-a71c-4ffe6c6b1533".to_string(),
        ];
        let nick = |id: &str, n: &str| (id.to_string(), n.to_string());
        let one = |t: &str, n: &[(String, String)]| resolve_target(t, &ids, n);
        let nicks = vec![nick(&ids[0], "mellow-weevil")];

        assert_eq!(one(&ids[2], &nicks), Ok(Some(ids[2].clone())), "an id");
        assert_eq!(one("mellow-weevil", &nicks), Ok(Some(ids[0].clone())));
        assert_eq!(
            one("02f2a191", &nicks),
            Ok(Some(ids[2].clone())),
            "a prefix"
        );
        assert_eq!(one("02f2a19", &nicks), Ok(None), "too short a prefix");
        assert_eq!(one("99999999", &nicks), Ok(None), "no match");
        assert_eq!(
            one("64a05eda", &nicks),
            Err(vec![ids[0].clone(), ids[1].clone()]),
            "a prefix two ids share removes nothing"
        );
        // Nicknames repeat across restarts: the ghost and its live successor.
        let twice = vec![
            nick(&ids[0], "mellow-weevil"),
            nick(&ids[3], "mellow-weevil"),
        ];
        assert_eq!(
            one("mellow-weevil", &twice),
            Err(vec![ids[0].clone(), ids[3].clone()]),
            "a repeated nickname removes nothing"
        );
        // A nickname row whose session has left the roster doesn't count.
        let gone = vec![
            nick(&ids[0], "mellow-weevil"),
            nick("left", "mellow-weevil"),
        ];
        assert_eq!(one("mellow-weevil", &gone), Ok(Some(ids[0].clone())));
    }

    #[test]
    fn forget_requests_round_trip() {
        let d = dir("forget");
        forget(&d, " mellow-weevil ").unwrap();
        forget(&d, "02f2a191").unwrap();
        let got: Vec<String> = forget_requests(&d).into_iter().map(|(t, _)| t).collect();
        assert_eq!(got, vec!["mellow-weevil", "02f2a191"]);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn pause_and_resume_round_trip() {
        let d = dir("pause");
        assert!(paused_since(&d).is_none());
        set_paused(&d, true).unwrap();
        let since = paused_since(&d).expect("paused");
        set_paused(&d, true).unwrap();
        assert_eq!(
            paused_since(&d),
            Some(since),
            "pausing again keeps the first time"
        );
        set_paused(&d, false).unwrap();
        assert!(paused_since(&d).is_none());
        set_paused(&d, false).unwrap();
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_allowlist_adds_removes_and_clears() {
        let d = dir("allow");
        assert_eq!(allowed(&d), None, "no list: anyone may send");
        assert_eq!(set_allowed(&d, "a1", true).unwrap(), vec!["a1"]);
        assert_eq!(
            set_allowed(&d, "A1", true).unwrap(),
            vec!["A1"],
            "no duplicate, any case"
        );
        assert_eq!(set_allowed(&d, "murph", true).unwrap(), vec!["A1", "murph"]);
        assert_eq!(set_allowed(&d, "a1", false).unwrap(), vec!["murph"]);
        assert_eq!(
            set_allowed(&d, "murph", false).unwrap(),
            Vec::<String>::new()
        );
        assert_eq!(
            allowed(&d),
            Some(vec![]),
            "an empty list still limits senders"
        );
        clear_allowed(&d).unwrap();
        assert_eq!(allowed(&d), None);
        let _ = std::fs::remove_dir_all(&d);
    }
}
