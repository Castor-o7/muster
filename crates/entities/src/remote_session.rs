//! Sessions on a linked daemon, and the link side of a message (MUS-66).
//!
//! A link joins two daemons (see `docs/daemon-links-design.md`). Each side
//! advertises its own local sessions; the other keeps them here, apart from
//! `Session`, so the gate's frame filter, session bindings, the sweeper and the
//! roster's own bookkeeping never mistake a far session for a local one.
//!
//! Both entities are kept by the daemon only. Clients may read
//! `RemoteSession` (the shim lists it on the roster) but never write either.

use myko::myko_item;
#[allow(unused_imports)]
use myko::prelude::{EventPublishing as _, Querying as _, RegistryScoped as _, RequestScoped as _};

/// The separator in `nickname@link` addressing and in a `RemoteSession` id.
pub const LINK_SEPARATOR: char = '@';

/// One session on a linked daemon, as that daemon last advertised it.
/// The entity id is `<session_id>@<link>`, so two links can't share a row.
#[myko_item]
pub struct RemoteSession {
    /// The link this session came over, named as this daemon knows it.
    pub link: String,
    /// The session's id on its own daemon.
    pub session_id: String,
    pub nickname: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operator: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_task: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activity: Option<String>,
    /// `waiting`, `channel`, `busy` or `none`, as the far daemon computed it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wake: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_activity_at: Option<i64>,
    /// Whether the link is up right now. While it's down the row is kept, and
    /// mail to the session queues.
    pub link_up: bool,
    /// When this daemon last heard about the session from its link.
    pub seen_at: i64,
}

impl RemoteSession {
    pub fn make_id(session_id: &str, link: &str) -> String {
        format!("{session_id}{LINK_SEPARATOR}{link}")
    }

    /// How a recipient names this session explicitly: `nickname@link`.
    pub fn address(&self) -> String {
        format!("{}{LINK_SEPARATOR}{}", self.nickname, self.link)
    }
}

/// The local id inbound mail is stored under: derived from the link and the
/// sender's id in a namespace of its own, so it's stable across a resend (for
/// dedup) but can't equal an id a local send derives.
pub fn inbound_message_id(link: &str, remote_id: &str) -> String {
    // A fixed, arbitrary namespace for link mail.
    const LINK_NAMESPACE: uuid::Uuid =
        uuid::Uuid::from_u128(0x6d75_7374_6572_2d6c_696e_6b2d_6d61_696c);
    uuid::Uuid::new_v5(&LINK_NAMESPACE, format!("{link}\0{remote_id}").as_bytes()).to_string()
}

/// Bumped whenever mail is put in an outbox, so the link tasks can look for it
/// at once instead of scanning on a timer.
pub static OUTBOX_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inbound_ids_are_stable_and_per_link() {
        let a = inbound_message_id("murph", "m1");
        assert_eq!(a, inbound_message_id("murph", "m1"));
        assert_ne!(a, inbound_message_id("a1", "m1"));
        assert_ne!(a, inbound_message_id("murph", "m2"));
        // Not the peer's id, whatever it sends.
        assert_ne!(a, "m1");
    }

    #[test]
    fn ids_and_addresses_use_the_link_name() {
        assert_eq!(RemoteSession::make_id("s1", "murph"), "s1@murph");
    }
}
