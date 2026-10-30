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

## Status

Built: hook binding, and a gate filter that reads each WebSocket frame from a paired
machine (`crates/daemon/src/gate_filter.rs`). Unknown commands, queries, events and
unparseable frames are refused, and myko's other routes are closed to paired machines.
The allowlist comes from what the shim sends, and the gate script checks that a real
shim's traffic isn't refused.

An adversarial review on 2026-10-30 broke the first version: a NUL-suffixed `id` key passed
the filter and became the real `id` when myko stripped NUL afterwards, and a tab in the
request line lost the machine name. A recheck then found that a command body sent as a JSON
array skipped the `asSession` check, because serde fills fields by position. All are fixed
with regression tests. Paired machines' session ids must now be UUIDs, so they can't pose as
a nickname or operator. Further findings are in MUS-33 and MUS-34.

A third round found free-text room names reaching mention banners, also fixed. That round
wasn't a clean pass: its hook-surface reviewer stopped early, so the hook surface was traced
by reading the code instead, with no bypass found, and it wasn't tested with attacks.

Still open:

- **The filter is an allowlist over a parser myko also interprets.** Three bypasses came from
  myko reading a frame differently from the filter. Treat more as possible, and prefer an
  authorization hook in myko in the long term.
- **First use wins.** A session nobody has used through a hook or the gate can still be
  claimed by whichever machine names it first. Roster ids are visible to paired machines,
  and this matters most for sessions already running during a rollout.
- **Reads are only partly limited.** The roster, rooms and memberships stay readable by
  every paired machine. Message bodies no longer are, except through a session's own
  `ReadMessages`.
- **Local sessions' `operator` and `host`** are still taken as given, since the daemon's own
  host is trusted (MUS-7 protects its ports). Paired machines' are attested (MUS-6).
- **Codex over the gate** would need `GetAllMessages` and `GetAllMessageReads` to be
  handled differently before MUS-18.

## Alternatives

- **Add an authorization hook to myko upstream.** The cleanest long-term answer, and worth
  proposing, but it depends on another project and needs a fork until it lands. It would
  let the gate's filter retire later.
- **Replace remote WebSocket access with a narrow Muster API.** The strongest option, but
  it stops remote shims being myko clients, which goes against the architecture decision.

## Open questions

- Who owns a session that moves between machines? Probably nobody: it stays refused until
  the operator releases it.

## Done when

A test shows that one paired machine can't read, clear or send as another machine's
session, including by raw event. The gate script now covers this, but it isn't in effect
until the daemon is rebuilt and restarted. Until then, treat every paired machine as fully
trusted.
