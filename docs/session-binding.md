# Tying sessions to machines (MUS-5)

Goal: a compromised paired machine can't read, clear or send as another machine's
session. See the threat model in [roadmap.md](roadmap.md).

## What's wrong today

The gate checks a machine's token, logs its name, then discards it. Past the gate, the
daemon doesn't know which machine is connected. That leaves three holes, read from the
code but not yet exploited in a test:

1. **Hooks trust `session_id`.** `/hook/wait` and `/hook/prompt-submit` take it from the
   request body, then hand over that session's unread mail and mark it read.
2. **Commands trust `asSession`.** `resolve_caller` accepts any session on the roster, so
   a client can send, read or acknowledge as anyone.
3. **Clients can write raw events.** myko's WebSocket handler passes a client's `Event`
   and `EventBatch` messages straight to `apply_event`
   (`myko-server-6.9.0/src/ws_handler.rs:1417`). A paired client can set or delete any
   entity directly: sessions, messages, rooms, and later the ledger.

The third hole means that checking identity in commands alone is not enough. myko 6.9.0
has no authorization hook, and the builder's per-entity persister overrides don't see
which client wrote an event.

## Design

Muster stays a stock myko service. Enforcement goes in the gate, which is already ours
and already sits in front of everything a paired machine can reach.

**1. Bind sessions to machines.** The first paired machine to use a session id owns it.
The daemon keeps that binding in its state directory, and it has to survive a restart.
Tools on the daemon's own host use the loopback ports, which MUS-7 protects separately.

**2. Hooks.** The gate removes any `X-Muster-Machine` header the client sent and adds the
verified machine name. Hook handlers refuse a session bound to another machine. This
closes hole 1 and is the smallest piece, so it comes first.

**3. WebSocket.** The gate stops copying WebSocket bytes blindly for paired machines. It
reads each client frame and allows only:

- queries and subscriptions,
- commands whose `asSession` is absent or belongs to this machine,
- events that set a `Session` bound to this machine.

Anything else is dropped and logged. This closes holes 2 and 3, and it applies to myko's
HTTP-MCP route too, unless that route is simply closed to paired machines.

## Alternatives

- **Add an authorization hook to myko upstream.** The cleanest long-term answer, and worth
  proposing, but it depends on another project and needs a fork until it lands. It would
  let the gate's filter retire later.
- **Replace remote WebSocket access with a narrow Muster API.** The strongest option, but
  it stops remote shims being myko clients, which goes against the architecture decision.

## Open questions

- Which frame encodings does the shim's myko client use? The filter must parse them all,
  and refuse any it can't parse.
- Which events does the shim really send? The allowlist should come from observing them.
- Does a filter stop queries leaking other sessions' message bodies? Filtering writes
  doesn't protect reads, so queries may need limits too.
- Who owns a session that moves between machines? Probably nobody: it stays refused until
  the operator releases it.

## Done when

A test shows that one paired machine can't read, clear or send as another machine's
session, including by raw event. Until then, treat every paired machine as fully trusted.
