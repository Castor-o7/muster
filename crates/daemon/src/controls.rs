//! The operator's controls over peer messages (MUS-10), read from files in
//! the daemon's state directory, which `muster-daemon pause`, `resume`,
//! `allow` and `disallow` write:
//!
//! - `wakes-paused`: present while wake-ups are paused; holds when, in ms.
//! - `allowed-senders`: present while an allowlist is set; one machine per line.
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
