//! The operator's controls over peer messages (MUS-10).
//!
//! Two settings, both set by the operator with `muster-daemon` commands and
//! read from the daemon's state directory every few seconds:
//!
//! - **Pause.** While wake-ups are paused, no held wait hands out mail and no
//!   live push is sent, so a message can't wake a session. Mail is still
//!   stored, and a session still gets it when someone types to it.
//! - **Sender allowlist.** When set, only sessions on a listed machine may send
//!   or broadcast; anyone else is refused. When unset, any machine may.

use std::sync::RwLock;
use std::sync::atomic::{AtomicBool, Ordering};

static WAKES_PAUSED: AtomicBool = AtomicBool::new(false);
static ALLOWED_SENDERS: RwLock<Option<Vec<String>>> = RwLock::new(None);

pub fn wakes_paused() -> bool {
    WAKES_PAUSED.load(Ordering::SeqCst)
}

pub fn set_wakes_paused(paused: bool) {
    WAKES_PAUSED.store(paused, Ordering::SeqCst);
}

/// `None` lets every machine send; `Some(list)` only the machines named.
pub fn set_allowed_senders(machines: Option<Vec<String>>) {
    *ALLOWED_SENDERS.write().unwrap_or_else(|e| e.into_inner()) = machines;
}

/// Whether a session on `machine` may send. A session with no machine is
/// refused whenever a list is set, since it can't be matched against it.
pub fn sender_allowed(machine: Option<&str>) -> Result<(), String> {
    let allowed = ALLOWED_SENDERS.read().unwrap_or_else(|e| e.into_inner());
    let Some(list) = allowed.as_ref() else {
        return Ok(());
    };
    match machine {
        Some(m) if list.iter().any(|a| a.eq_ignore_ascii_case(m)) => Ok(()),
        Some(m) => Err(format!(
            "the operator allows messages only from {}; this session is on {m}",
            list.join(", ")
        )),
        None => Err(format!(
            "the operator allows messages only from {}; this session names no machine",
            list.join(", ")
        )),
    }
}
