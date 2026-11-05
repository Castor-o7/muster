# Live delivery without the channels flag

Goal G4. This explains how Muster delivers direct messages to Claude Code,
and wakes an idle session, when Claude is launched without
`--dangerously-load-development-channels`. It also compares that with
Marshal's channel push. Every claim was checked against the code at
`ea472f6`; file references are to that tree.

## The two paths at a glance

| | Marshal channel push | Muster hook delivery |
|---|---|---|
| Needs a launch flag | yes: `claude --dangerously-load-development-channels plugin:marshal-shim@marshal` (or `server:marshal`) | no |
| Transport to Claude | MCP notification `notifications/claude/channel` from the shim over stdio | Claude Code hooks: command hooks that print `additionalContext`, and an `asyncRewake` hook that exits 2 |
| Who moves the message | daemon → shim's WebSocket → MCP notification, at send time | the recipient pulls: each hook asks the daemon's hook listener for unread mail |
| Idle session wakes | yes, the notification starts a turn | yes, through the `wait` hook (below) |
| Busy session sees it | yes, as a channel event | at the next tool call (`PostToolUse`) or prompt (`UserPromptSubmit`) |
| Marked read when | the push is handed to the shim's WebSocket client (`send_message.rs`, the dedupe after `push_to_client`) | the hook output has been written back (prompt hooks), or the waiter has printed it and acked (`/hook/wait`) |
| Message body in context | a bounded preview in `meta.body` (2,000 chars) | a bounded preview in the `<muster_inbox>` block: at most 20 messages, an 8,000-char budget split between them (min 256, max 2,000 each) |
| Room broadcasts | not pushed; ambient | not surfaced; ambient. Only direct mail is injected |

Both paths share the daemon, the durable `Message` store and the read
records. Only the last leg to the model differs. A session can have both: a
flagged session gets the push, and its hooks then find nothing unread,
because the push marked the message read.

## How Marshal's push works (and where it fails without the flag)

`SendMessage` (`crates/entities/src/send_message.rs`) always persists the
message first. Then, if the recipient's roster row has a live WebSocket
`client_id`, it sends a `NotifyChannel` command to that shim
(`push_to_client`). The shim turns it into a `notifications/claude/channel`
MCP notification (`crates/shim/src/mcp.rs`). If the push reached the client,
the daemon writes a `MessageRead`, so the per-turn inbox doesn't show it a
second time.

Without the flag, Claude Code receives the notification and drops it. Before
Muster's first commit (`37731f2`) that lost mail in two ways:

1. **Mail marked read but never seen.** On macOS the shim couldn't read its
   parent's command line, so it reported `channels_enabled = None`. The daemon
   treats `None` as "can render", so it pushed, got `Delivered`, and marked the
   message read. The model never saw it.
2. **Nothing pulled it in.** The plugin shipped no hooks, so a session without
   the flag had no other way to receive mail (upstream issue #139).

Muster fixes the first in `crates/shim/src/channels.rs`. `parent_cmdline`
reads the parent with `ps` on macOS, and `channels_enabled_in` reports
`Some(false)` unless the command line contains both the flag and `muster`.
`SendMessage` then skips the push for `Some(false)`, reports
`live_push: unavailable`, and leaves the message unread. Hooks fix the
second.

## How Muster's hooks work

### Wiring

`plugins/muster-shim/hooks/hooks.json` installs four hook events with the
plugin, so nothing needs to be set at launch:

| Event | Command | Purpose |
|---|---|---|
| `SessionStart` | `muster-shim claude-hook session-start` (10 s) | register the session and inject the `<muster_session>` identity block |
| `SessionStart` | `muster-shim claude-hook wait`, `asyncRewake: true` (timeout 30 days) | start the first waiter |
| `Stop` | `muster-shim claude-hook wait`, `asyncRewake: true` | start a waiter again after every turn |
| `UserPromptSubmit` | `muster-shim claude-hook prompt-submit` | inject unread mail before the prompt |
| `PostToolUse` | `muster-shim claude-hook post-tool-use` | inject unread mail between tool calls |

`claude-hook` dispatches before the tokio runtime starts
(`crates/shim/src/main.rs`, the `Some("claude-hook")` arm). It is a single
blocking HTTP call with no WebSocket.

### Pull hooks: delivery while the session is working

`codex_hook::run_for(Harness::Claude, …)` (`crates/shim/src/codex_hook.rs`)
is the same bridge Codex uses. It reads the hook JSON (`session_id`, `cwd`)
from stdin and POSTs to the daemon's hook listener. `prompt-submit` and
`post-tool-use` both go to `/hook/prompt-submit`. The bridge prints
`{"hookSpecificOutput":{"hookEventName":…,"additionalContext":<inbox>}}`,
which Claude adds to the turn. The Claude harness has a 2 s timeout because
`PostToolUse` runs after every tool call; Codex gets 5 s. Any failure prints
nothing, so a hook can never break a turn.

On the daemon side (`crates/daemon/src/hooks.rs`), `handle_prompt_submit`
re-runs the session registration, which repairs a missing roster row. Then
`surface_unread` reads up to 20 unread **direct** messages, leases them, and
renders the `<muster_inbox>` block. The block puts mail for the operator
first, then peer mail framed as untrusted. The shim's hooks ask to ack the mail
themselves (`ack=client`, MUS-75), as the waiter does. The listener returns the
ids in an `X-Muster-Ack` header and writes no `MessageRead`; the hook prints
the block, and only then POSTs `/hook/ack` with the ids. A hook killed at its
timeout, or a reply dropped on a tunnel hop, therefore leaves the mail unread:
the lease keeps it from being offered twice for 60 s, and then the next hook
gets it again. Delivery is at-least-once.

Until MUS-75 the listener acked once it had written and flushed the reply
(`deferred_ack`), which it still does for a caller that doesn't send
`ack=client`. A flush doesn't prove the reply arrived, so a reply lost after
it lost the mail.

### The waiter: waking an idle session

A pull hook only runs when the session is already doing something. Waking an
idle session is the job of `muster-shim claude-hook wait`
(`codex_hook::wait_hook`):

1. It takes a per-session file lock (`waiter-<session_id>` in the config dir),
   so only one waiter runs per session even though `Stop` fires every turn. A
   new waiter waits up to `MUSTER_WAITER_LOCK_WAIT_SECS` (default 35 s) for
   the lock, which covers a resumed session whose old waiter hasn't yet
   noticed its parent is gone.
2. It long-polls `POST /hook/wait?…&ack=client&timeout=30`. The daemon
   (`http_listener::handle_wait`) registers the session and then, every
   500 ms until the timeout, refreshes a `WakeWatcher` record and checks the
   inbox with `poll_wait`. It returns early if the daemon is stopping
   (`X-Muster-Restarting`) or the client hangs up.
3. When mail arrives, the daemon answers with the inbox block and an
   `X-Muster-Ack: <ids>` header, and does **not** ack it itself
   (`ack=client`).
4. The waiter prints the block to **stderr** and exits **2**. For an
   `asyncRewake` hook, Claude Code treats that as a blocking result and starts
   a turn with the stderr text, even in a session nobody has typed in. Only
   after printing does the waiter POST `/hook/ack` with the ids. If that ack
   is lost, the mail comes again rather than going missing.
5. The turn ends, `Stop` fires, and a new waiter starts. Nothing needs to be
   kept running between turns.

The waiter stops without delivering if its parent process changes (the
session closed), so it doesn't take mail for a dead session. Mail it has
already leased stays unread, and the 60 s lease (`MUSTER_ACK_LEASE_SECS`)
lapses so another reader can take it. If the daemon refuses the waiter (gate
or pairing errors), the waiter tells the session once per cause, through the
same path, and keeps a marker so later waiters don't repeat it.

`muster-shim watch <session_id> [--once]` is the older, opt-in form of the
same loop. It runs under Claude's Monitor tool (each stdout line is an event)
or as a background Bash command. It uses a 240 s poll and splits output to
fit Monitor's line and event limits. The `wait` hook replaced it as the
default, because the hook needs no agent to start it.

### What the roster says

While a waiter holds `/hook/wait`, the session has a `WakeWatcher` record.
The record lasts 2 minutes, is refreshed every minute, and is not persisted
across daemon restarts (#43). `SendMessage` uses it in its result: a flagless
session gets `live_push: unavailable` and `wake: expected` when a waiter is
present and wakes aren't paused, the same rule as the roster's `wake: waiting`.
Otherwise it gets `wake: unobserved`. A live client alone doesn't count: the
MCP shim of a session without the flag can't wake it (MUS-71).
`wake: expected` means a wake path exists, not that a turn started.

## Differences that matter

- **Who acts.** A push is sent once by the daemon, at send time. Hook
  delivery is pulled by the recipient, at the recipient's next hook. The
  waiter keeps that delay to about 500 ms on an idle session.
- **What "read" means.** With a push, "read" means the daemon reached the
  shim's WebSocket. It doesn't confirm Claude rendered it, which is how the
  macOS case went wrong. With hooks, "read" means the hook or the waiter
  printed the text for Claude Code, then acked it.
- **Duplicates.** Both paths prefer a duplicate to a loss. Hooks can show a
  message twice if an ack is lost or a lease lapses. A push is not retried.
- **Framing.** Both wrap peer text as untrusted. Hook mail arrives as a
  `<muster_inbox>` block in `additionalContext`, or as the waiter's stderr in
  a "Stop hook" result. A push arrives as a `<channel>` event.
- **Scope.** Neither path injects room broadcasts. Only direct mail, including
  mail to an operator routed to their most active session, can interrupt or
  wake.
- **Operator pause.** `controls::wakes_paused()` (MUS-10) stops both. The
  push is skipped (`push_to_client` returns false, so the result reads
  `failed`), and `/hook/wait` hands out nothing. The Codex bridge also starts
  no turns while the daemon's `WakePause` record says paused (MUS-74). The
  prompt hooks still deliver, because they don't wake a session.
- **Codex.** Codex can't take a push or an `asyncRewake`. It uses the same
  pull hooks plus the app-server bridge's `turn/start`
  ([codex-live-delivery.md](codex-live-delivery.md)).

## Porting notes for upstream

This is the "Hooks" PR in [upstream-inventory.md](upstream-inventory.md).
Upstream already has the hook listener and `/hook/prompt-submit` for Codex.
The new parts are `claude-hook`, `hooks.json`, `/hook/wait` with
`ack=client` and `/hook/ack`, `WakeWatcher`, and the macOS parent-cmdline
read. On Marshal names these become `marshal-shim claude-hook …`, port 6156,
`MARSHAL_WAITER_LOCK_WAIT_SECS` and `MARSHAL_ACK_LEASE_SECS`. Upstream's
`response_required` metadata (#161) changes the inbox line to
`- from X [response_required=…]:`, so the waiter's output picks it up with
no further change.
