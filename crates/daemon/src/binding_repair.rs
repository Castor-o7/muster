//! MUS-50: put back a session's client binding when a daemon write drops it.
//!
//! myko fills a `Session`'s `client_id` from the connection only when the row
//! arrives over a client's WebSocket. The daemon's own writes, the hooks'
//! `register_hook_session` above all, write a whole row built from a copy they
//! read moments before. If the shim's registration lands between that read and
//! that write, the copy has no `client_id`, the write clears the binding, and
//! every send from the session is refused as unregistered until the shim
//! notices and registers again. That was seen on 2026-11-02.
//!
//! Narrowing the window wouldn't close it, since the read can lag the shim's
//! write by however long the store takes to apply it. So this repairs the
//! effect instead, whatever caused it. Each `Session` SET that carries a
//! client is remembered. When a later SET leaves the stored row with no
//! client, and the client it was last bound to is still connected, the binding
//! is put back. A row whose client has really gone (the sweeper's demotion,
//! MUS-54) is left alone, because that client isn't connected.

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

/// Session id → the client it was last bound to over its WebSocket.
static BOUND: LazyLock<Mutex<HashMap<Arc<str>, ClientId>>> = LazyLock::new(Default::default);

#[myko_saga]
pub struct KeepClientBindingSaga;

impl SagaHandler for KeepClientBindingSaga {
    type EventItem = Session;
    type Command = KeepClientBinding;
    const EVENT_TYPE: MEventType = MEventType::SET;

    fn handle(session: Session, _event: MEvent, _ctx: Arc<SagaContext>) -> Option<Self::Command> {
        let mut bound = BOUND.lock().unwrap_or_else(|e| e.into_inner());
        match &session.client_id {
            Some(cid) => {
                bound.insert(session.id.0.clone(), cid.clone());
                None
            }
            // Only a session that had a binding can have lost one.
            None => bound
                .contains_key(&session.id.0)
                .then(|| KeepClientBinding {
                    session_id: session.id.0.to_string(),
                }),
        }
    }
}

/// Server-internal: if `session_id`'s row has no client but the client it was
/// last bound to is still connected, bind it again.
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
    let Some(cid) = BOUND
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(session_id)
        .cloned()
    else {
        return Ok(());
    };
    // Decide on the row as stored now, not the event that triggered this: a
    // later write may already have put the binding back.
    let sessions: Vec<Arc<Session>> = ctx.exec_query(GetAllSessions {})?;
    let Some(row) = sessions.iter().find(|s| s.id.0.as_ref() == session_id) else {
        return Ok(());
    };
    if row.client_id.is_some() {
        return Ok(());
    }
    let connected = ctx
        .exec_query(GetAllClients {})?
        .iter()
        .any(|c| c.id == cid);
    if !connected {
        // Its client has gone: the row is a pull row now, rightly unbound.
        BOUND
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(session_id);
        return Ok(());
    }
    log::info!(
        "[binding] session {session_id} lost its binding to live client {} in a daemon write; restoring it",
        cid.0
    );
    let mut restored = (**row).clone();
    restored.client_id = Some(cid);
    ctx.emit_set(&restored)
}
