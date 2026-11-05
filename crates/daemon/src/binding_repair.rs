//! MUS-50: put back a session's client binding when a daemon write drops it.
//!
//! myko fills a `Session`'s `client_id` from the connection only when the row
//! arrives over a client's WebSocket. The daemon's own writes, the hooks'
//! `register_hook_session` above all, write a whole row built from a copy they
//! read moments before. If the shim's registration lands between that read and
//! that write, the copy has the wrong `client_id`: none, or an older
//! connection's. The write clears or stales the binding, and every send from
//! the session is refused as unregistered until the shim registers again. That
//! was seen on 2026-11-02.
//!
//! Narrowing the window wouldn't close it, since the read can lag the shim's
//! write by however long the store takes to apply it. So this repairs the
//! effect instead, whatever caused it:
//!
//! - Every `Session` SET that names a client is noted, in the order each
//!   client was first seen for that session. Client ids are per connection, so
//!   the newest one seen is the shim's current connection.
//! - When a SET leaves the stored row with no client, or with an older client
//!   than the newest one still connected, the row is bound to that newest
//!   connected client again.
//! - A row whose clients have all gone (the sweeper's demotion, MUS-54) is left
//!   unbound, because none of them is connected.
//!
//! The repair's own SET names the newest client, so it triggers nothing more.

use std::{
    collections::HashMap,
    sync::{Arc, LazyLock, Mutex},
};

use muster_entities::{GetAllSessions, Session};
use myko::entities::client::{ClientId, GetAllClients};
use myko::prelude::EventPublishing as _;
use myko::{
    command::{CommandContext, CommandError, CommandHandler},
    myko_command,
    prelude::myko_saga,
    saga::{SagaContext, SagaHandler},
    wire::{MEvent, MEventType},
};

/// Force-link the saga and command registrations.
pub fn link() {}

/// Session id → the clients seen bound to it, oldest first.
static SEEN: LazyLock<Mutex<HashMap<Arc<str>, Vec<ClientId>>>> = LazyLock::new(Default::default);

/// Session id → how many times its binding has been put back.
static REPAIRS: LazyLock<Mutex<HashMap<Arc<str>, usize>>> = LazyLock::new(Default::default);

/// How many times this daemon has put `session_id`'s binding back.
pub fn repairs_for(session_id: &str) -> usize {
    REPAIRS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(session_id)
        .copied()
        .unwrap_or(0)
}

#[myko_saga]
pub struct KeepClientBindingSaga;

impl SagaHandler for KeepClientBindingSaga {
    type EventItem = Session;
    type Command = KeepClientBinding;
    const EVENT_TYPE: MEventType = MEventType::SET;

    fn handle(session: Session, _event: MEvent, _ctx: Arc<SagaContext>) -> Option<Self::Command> {
        let mut seen = SEEN.lock().unwrap_or_else(|e| e.into_inner());
        let clients = seen.entry(session.id.0.clone()).or_default();
        if let Some(cid) = &session.client_id
            && !clients.contains(cid)
        {
            clients.push(cid.clone());
        }
        // A SET naming the newest client seen is the shim's own, or a repair:
        // nothing to do. Anything else may have dropped or staled the binding.
        let current = clients
            .last()
            .is_some_and(|newest| session.client_id.as_ref() == Some(newest));
        if clients.is_empty() || current {
            return None;
        }
        Some(KeepClientBinding {
            session_id: session.id.0.to_string(),
        })
    }
}

/// Server-internal: bind `session_id`'s row to the newest of its clients that's
/// still connected, if the stored row isn't bound to it.
#[myko_command]
pub struct KeepClientBinding {
    pub session_id: String,
}

impl CommandHandler for KeepClientBinding {
    fn execute(self, ctx: CommandContext) -> Result<(), CommandError> {
        restore_if_lost(&ctx, &self.session_id)
    }
}

fn restore_if_lost(ctx: &CommandContext, session_id: &str) -> Result<(), CommandError> {
    let clients: Vec<ClientId> = SEEN
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(session_id)
        .cloned()
        .unwrap_or_default();
    if clients.is_empty() {
        return Ok(());
    }
    let connected: Vec<ClientId> = ctx
        .exec_query(GetAllClients {})?
        .iter()
        .map(|c| c.id.clone())
        .collect();
    let Some(newest_live) = clients
        .iter()
        .rev()
        .find(|c| connected.contains(c))
        .cloned()
    else {
        // Every client it was bound to has gone: a pull row now, rightly unbound.
        SEEN.lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(session_id);
        return Ok(());
    };
    // Decide on the row as stored now, not the event that triggered this: a
    // later write may already have put the binding right.
    let sessions: Vec<Arc<Session>> = ctx.exec_query(GetAllSessions {})?;
    let Some(row) = sessions.iter().find(|s| s.id.0.as_ref() == session_id) else {
        return Ok(());
    };
    if row.client_id.as_ref() == Some(&newest_live) {
        return Ok(());
    }
    log::info!(
        "[binding] session {session_id} was left bound to {:?} by a daemon write; binding it to its live client {} again",
        row.client_id.as_ref().map(|c| c.0.as_ref()),
        newest_live.0
    );
    let mut restored = (**row).clone();
    restored.client_id = Some(newest_live);
    ctx.emit_set(&restored)?;
    *REPAIRS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entry(Arc::from(session_id))
        .or_default() += 1;
    Ok(())
}
