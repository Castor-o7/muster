//! `WakePause`: whether the operator has paused wake-ups.
//!
//! The daemon writes one record, id `wakes`, from its state directory; the gate
//! refuses a client's events on it, so only the daemon can change it. Rosters
//! read it so a session can tell why a message didn't wake it.

use myko::myko_item;
#[allow(unused_imports)]
use myko::prelude::{EventPublishing as _, Querying as _, RegistryScoped as _, RequestScoped as _};

#[myko_item]
pub struct WakePause {
    /// Whether wake-ups are paused.
    pub paused: bool,
    /// When they were paused, in ms since the epoch; 0 when not paused.
    pub since: i64,
}

/// The id of the one `WakePause` record.
pub const WAKE_PAUSE_ID: &str = "wakes";
