//! The link side of a message (MUS-66). See `remote_session`.

use myko::myko_item;
#[allow(unused_imports)]
use myko::prelude::{EventPublishing as _, Querying as _, RegistryScoped as _, RequestScoped as _};

/// The link side of one `Message`, keyed by that message's id. An outbound row
/// is the outbox entry and is deleted once the far daemon has the message; an
/// inbound row records where the message came from, for labels and dedup.
#[myko_item]
pub struct MessageLink {
    pub link: String,
    /// `true`: stored here for a session on `link`, waiting in the outbox until
    /// the far daemon answers `stored`. `false`: arrived over `link`.
    pub outbound: bool,
    /// For inbound mail, the id the sending daemon gave the message. Never
    /// used as a local id, so a peer can't squat one (MUS-66).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_id: Option<String>,
    /// For outbound mail held back by a `deferred` answer: not before this
    /// (wall-clock millis).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_at: Option<i64>,
    /// For outbound mail: why the far daemon couldn't take it yet, if it said.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}
