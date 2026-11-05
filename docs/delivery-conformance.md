# Delivery conformance

MUS-69. One scenario table, run against every harness Muster delivers to, with
the real `muster-daemon` and `muster-shim` binaries and no TUI or model. Each
harness is a fake that drives the real binaries the way its TUI does. The suite
is `crates/shim/tests/conformance.rs`.

```sh
cargo build --workspace
cargo test -p muster-shim --test conformance -- --ignored --nocapture
```

The plugin columns also need bun and each plugin's dependencies; see the gaps
below.

`CONFORMANCE_ONLY=<text>` runs only the cells whose harness or scenario name
contains it. `CONFORMANCE_GRID=<file>` also writes the grid to a file. CI runs
the whole suite after `cargo build --workspace`.

## How a cell runs

Each cell gets a fresh daemon in throwaway dirs, on free loopback ports, with
no Postgres and a 2 s ack lease (`MUSTER_ACK_LEASE_SECS`). Its hook traffic goes
through a small proxy that can drop `/hook/ack`. A second session sends, through
a real `muster-shim` speaking MCP, so the send result (`live_push`, `wake`) is
checked as well as what the recipient sees.

## The scenarios

| # | Scenario | Passes when |
|---|---|---|
| 1 | `idle-wakes-on-dm` | An idle session's wake path fires with the message. |
| 2 | `busy-gets-it-at-next-hook` | A busy session's next hook or turn carries the message, and the one after doesn't. |
| 3 | `dropped-ack-redelivers` | When a delivery's ack is lost, the message comes again at the session's next delivery point once the lease lapses, and not after an ack that arrives. Claude's waiter acks with its own `/hook/ack`, which the proxy drops. Codex has no ack request: the daemon acks a hook's reply once it has flushed it, so the proxy loses the reply instead. |
| 3b | `lost-hook-reply-redelivers` | When the reply to a busy session's prompt hook is lost after the daemon sent it, as when a harness kills a hook at its timeout, the message comes again at the next hook. |
| 4 | `duplicate-shown-at-most-once` | A send retried with the same idempotency key is one message, shown once. A wake shows its message once, not once from the push and again from the woken turn's drain. Mail a wake delivered isn't injected again by the next hook. |
| 4b | `burst-both-delivered` | Two messages sent back to back to an idle session both reach the model, between the wake and the next delivery point. A harness that drains the inbox while handling the first must show what it drains. |
| 5 | `daemon-restart-mid-wait` | The daemon restarts (SIGTERM, as systemd stops it) while the session waits, in memory, so the roster empties. The session still wakes on mail sent after the restart. |
| 5 (pg) | `daemon-restart-mid-wait-pg` | The same, with the daemon persisting to Postgres. |
| 5b (pg) | `restart-keeps-unread-mail` | Mail sent while the session is busy, then a restart against Postgres: the mail is kept, and wakes the session once it goes idle. n/a for the push plugins, which take the push into their running turn. |
| 6 | `operator-pause-stops-wakes` | With `muster-daemon pause`, mail doesn't wake an idle session but still arrives at the next hook. |
| 6b | `resume-wakes-held-mail` | Mail sent while wakes are paused doesn't wake the session, and resuming alone wakes it with that mail, with no new message. |
| 7 | `broadcast-never-wakes` | A room broadcast neither wakes the session nor is injected. |
| 8 | `wake-none-not-expected` | A session the roster calls `wake: none` isn't reported to the sender as `wake: expected`. |
| 9 | `mail-waiting-at-reconnect` | Mail sent while a push plugin's connection is down is shown once at the next turn after it reconnects, and not again after that, even when another message is pushed live first. A plugin that drains the inbox after a push acks the waiting mail there without showing it (upstream #133). |
| 10 | `hung-handover-released` | When the harness's API hangs taking a push, a turn's pull doesn't show the message while the plugin holds it, the next turn shows it once the hold's lease lapses, and no pull shows it again after the hung handover goes through. That late handover can show it a second time, which the design accepts ([push-ack-design.md](push-ack-design.md)). |

| 11 | `restart-keeps-identity` | A session that ends as its TUI ends it, followed by a new session in the same working directory, hands the new one its nickname (MUS-3). Mail to the old nickname and the old id reaches the new one. If the new one first got a fresh nickname, mail to that still reaches it, and its next hook says its name changed. |
| 12 | `live-sibling-keeps-its-name` | A second session in the same working directory, started while the first is live and idle past the quiet time, gets a nickname of its own, and the first keeps its own. For Codex the first is a hook-owned row with no client, which mustn't be taken over until MUS-77. |
| 13 | `unread-mail-follows-a-restart` | Mail sent to a session while no hook runs, so it's still unread when the session ends, is shown once by the session that takes over its nickname, and not at the hook after (MUS-47). |

Scenarios 9 and 10 are about a live push, so they're `n/a` for the hook
harnesses (Claude and Codex). Scenarios 11 to 13 are `n/a` for the plugins,
which don't report a harness yet. The suite's daemon runs with a 1 s
succession grace and a 2 s quiet time.

Some cells fail today. Each is listed in `KNOWN_FAILURES` against the story
that fixes it, and shows as `known (MUS-nn)` in the grid. If one starts
passing, the suite fails until it's taken off the list.

| Cells | Story | What fails |
|---|---|---|
| `opencode`, `prime-agent` × 3 | MUS-81 | A pushed message is marked read when the push reaches the plugin's connection. If the harness then loses it, the next turn's inbox drain finds nothing unread, and the message is gone: the push-side twin of MUS-75. Pi acks its own pushes and passes. |
| `opencode`, `prime-agent` × 10 | MUS-81 | The push is marked read before the harness takes it, so a handover that never resolves loses it. |
| `pi`, `opencode`, `prime-agent` × 6b | MUS-82 | While wakes are paused, the push is skipped and the mail stays unread, but resuming pushes nothing again, so a push session isn't woken until its next turn. |

## The harnesses

| Column | Harness | Mode | Status |
|---|---|---|---|
| `claude` | Claude Code | plain | built |
| `claude-tls` | Claude Code | `MUSTER_TLS=1`, as Murph runs: the hooks carry the local token, and the shim connects through the gate's port | built |
| `codex` | Codex, through `muster-shim codex-bridge` and the app-server | plain | built |
| `codex-tls` | Codex | `MUSTER_TLS=1`: the bridge and the hooks carry the local token through the gate's port (MUS-18) | built |
| `opencode` | opencode plugin (`plugins/muster-opencode`) | plain | built |
| `pi` | Pi plugin (`plugins/muster-pi`) | plain | built |
| `prime-agent` | Prime Agent plugin (`plugins/muster-prime-agent`) | plain | built |

### What the fake Claude assumes

Pinned to **Claude Code 2.1.318**. Its hook input comes from
`crates/shim/tests/fixtures/claude-code-2.1.318/`. It drives the hooks exactly
as `plugins/muster-shim/hooks/hooks.json` wires them, and it assumes these TUI
behaviours, which the suite can't check:

- **Command hooks get the hook JSON on stdin**, and Claude adds a
  `PostToolUse` or `UserPromptSubmit` hook's
  `hookSpecificOutput.additionalContext` to the turn.
- **An `asyncRewake` hook that exits 2 starts a turn** with its stderr, even
  in a session nobody has typed in. Any other exit starts nothing. The fake
  reads the waiter's exit code and stderr and treats exit 2 as the wake.
- **`SessionStart` runs `session-start`, then starts a waiter. `Stop` starts a
  waiter after every turn.** The fake arms a waiter for each idle period: with
  the SessionStart input the first time and the Stop input after that.
- **Claude runs the MCP shim for the session** with `CLAUDE_CODE_SESSION_ID`
  set, and without `--dangerously-load-development-channels`. So the roster
  row has a live client with channels off. Scenario 8 depends on this.
- **A busy session's next delivery point is `PostToolUse`.** The fake doesn't
  model a waiter that's still running while a turn is busy: it treats busy as
  no waiter, as when the waiter has just woken the session and `Stop` hasn't
  fired yet.

### What the fake Codex assumes

Pinned to **Codex 0.163.0**, the version `codex_bridge.rs` cites. Its
app-server frames and hook input come from
`crates/shim/tests/fixtures/codex-0.163.0/`. It runs the real
`muster-shim codex-bridge` against a fake shared app-server that hosts one
thread, and the real `muster-shim codex-hook` commands, as `codex-run` and
`codex_setup.rs` wire them. It assumes these behaviours, which the suite can't
check:

- **The app-server announces an attached TUI's thread** with
  `thread/started` to every client that initializes, and the bridge registers
  it from that.
- **`turn/start` is accepted on an idle thread and refused while a turn
  runs.** The fake models busy as "refused". The exact busy error is a guess.
  The bridge treats any error as "retry later".
- **A turn started by `turn/start` runs Codex's `UserPromptSubmit` hook**
  before the model, and that hook injects the inbox. The fake treats an
  accepted `turn/start` as the wake and then runs that hook.
- **Codex's MCP shim registers no session** (`MUSTER_HARNESS=codex`), so the
  thread's roster row is hook-owned with no live client. The fake runs no MCP
  shim for it.
- **After a wake, the bridge won't wake the same thread again for 30 s.** So
  scenario 3 checks redelivery at the next hook, not with a second wake.

### What the fake Pi assumes

Pinned to **`@earendil-works/pi-coding-agent` 0.83.0**, the version in
`plugins/muster-pi/package-lock.json`. The real muster-pi extension runs
under bun, loaded by `plugins/muster-pi/test/conformance-driver.ts` with a
stand-in for pi's `ExtensionAPI`. It connects to the daemon itself and takes
the live push, as it does inside pi. It assumes these behaviours of pi, which
the suite can't check:

- **`pi.sendMessage(…, { triggerTurn: true })` on an idle session starts a
  turn** with that message, and on a busy one steers it into the running
  turn. The fake records the first as the wake and the second as busy
  delivery.
- **A turn starts with `before_agent_start`**, and whatever message the
  handler returns is shown to the model. The fake fires it at each busy
  boundary and after a wake, which is where muster-pi drains the inbox.
- **`session_start`, `agent_settled` and `session_shutdown` fire as their
  names say.** The session id comes from `ctx.sessionManager.getSessionFile()`.
- **A lost delivery** means `pi.sendMessage` rejects after the extension has
  received the push, as when pi fails to take the message. That's where the
  fake loses the ack in scenario 3.
- **A hung handover** (scenario 10) means `pi.sendMessage` doesn't resolve
  until the suite lets it go, and then pi takes the message after all.
- **A dropped connection** (scenario 9) is the extension's WebSocket closing.
  The driver wraps `WebSocket` before it loads the extension, closes the open
  socket, and points new ones at a closed port until the suite restores it.
  myko reconnects on its own, as it does after a real drop.

Two scenarios are `n/a` for Pi. A plugin has no prompt hooks, so a hook reply
can't be lost (3b). And a connected plugin renders pushes, so its roster row
says `channel`, never `none` (8).

### What the fake opencode assumes

Pinned to **`@opencode-ai/plugin` and `@opencode-ai/sdk` 1.17.11**, the
versions in `plugins/muster-opencode/package-lock.json`. The real
muster-opencode plugin runs under bun, loaded by
`plugins/muster-opencode/test/conformance-driver.ts` with a stand-in for
opencode's plugin input: the SDK client (`session.prompt`, `tui.showToast`,
`app.log`) and bun's own `$`. It connects to the daemon itself and takes the
live push. It assumes these behaviours of opencode, which the suite can't
check:

- **`client.session.prompt` on an idle session starts a turn** with that
  text. On a busy one, opencode queues it behind the running turn, so the model
  sees it next. The fake records the first as the wake and the second as busy
  delivery.
- **A user turn runs the `chat.message` hook**, and parts the hook adds are
  shown to the model. muster-opencode drains the inbox there, so the fake runs
  it at each busy boundary, and after a wake, because the plugin's
  `session.prompt` is itself a user turn.
- **The `event` hook gets `session.created` and `session.idle`** with the
  session id where `eventSessionId` looks for it.
- **A lost delivery** means `session.prompt` rejects after the plugin has
  received the push. A hung handover and a dropped connection work as for Pi.

Scenarios 3b and 8 are `n/a`, as for Pi.

### What the fake Prime Agent assumes

Prime Agent runs the pi extension API, so its fake is the fake Pi with Prime
Agent's session manager, in `plugins/muster-prime-agent/test/conformance-driver.ts`.
It's pinned to **`@earendil-works/pi-coding-agent` 0.84.0**, the version in
`plugins/muster-prime-agent/package-lock.json`, and makes Pi's assumptions,
plus these:

- **The session id comes from `ctx.sessionManager.getSessionId()`**, and
  `getHeader()` describes the conversation. The fake's header has no
  `rlmDepth`, so it's a top-level session, which the extension registers. An
  RLM sub-agent, which it skips, isn't covered.
- **`ctx.ui.setWidget`** exists alongside `notify` and `setStatus`, for the
  statusline widget.

Scenarios 3b and 8 are `n/a`, as for Pi.

## Gaps

These are listed so nobody reads the grid as more than it is.

- **The plugin columns need bun and the plugin's `node_modules`.** CI downloads
  bun 1.3.14 from its GitHub release, checks it against a SHA-256 committed in
  the workflow, and runs `npm ci` in each plugin. Locally, put bun on `PATH` or
  set `MUSTER_BUN`, and run `npm ci --workspaces=false` in the plugin's
  directory. Without bun the plugin cells read `no bun`, and CI fails them
  (`CONFORMANCE_REQUIRE_BUN`).
- **Only the pi plugin's own `bun test` suite runs in CI.** opencode's and
  Prime Agent's don't yet; their conformance drivers do.
- **The fixtures are constructed, not recorded.** The Claude fixtures follow
  Claude Code's documented hook input. The Codex fixtures follow the frames the
  bridge's own tests use, because Codex isn't installed where they were
  written. Recording both is below. The shim reads only a few fields from each
  (the ids and cwd), so the risk is small.
- **A paired Codex machine isn't a column.** `codex-tls` runs on the daemon's
  host, as `claude-tls` does. A bridge on a paired machine, over the pairing's
  tunnel, is checked in `scripts/test-gate-e2e.sh` instead (MUS-18): it
  registers its thread, is woken through the gate, and says why once unpaired.
- **The Postgres cells need a database** (MUS-76): `CONFORMANCE_POSTGRES_URL`,
  in which each daemon gets its own events table (`conformance_…`) and leaves it
  behind. Without one they show `no postgres`. A local run therefore
  accumulates `conformance_*` tables; drop them with
  `psql "$CONFORMANCE_POSTGRES_URL" -Atc "select 'drop table ' || tablename || ';' from pg_tables where tablename like 'conformance\_%'" | psql "$CONFORMANCE_POSTGRES_URL"`. CI runs a `postgres:16` service
  and sets `CONFORMANCE_REQUIRE_POSTGRES`, so there a missing database fails.
- **Pause is only checked on the waiter.** With channels on, `SendMessage`'s
  live push also checks the pause. No harness here uses channels, because the
  fakes run without the flag, as Muster's sessions do.
- **The two TUI behaviours above are assumed, not checked.** If Claude Code
  changes what exit 2 from an `asyncRewake` hook does, this suite still
  passes. Only a real-agent run (MUS-68) would show it.

## Re-recording fixtures

Do this when Claude Code changes its hook input, or to replace the constructed
fixtures with recorded ones.

1. In a scratch directory, not a real project, write
   `.claude/settings.json` with one command hook per event the plugin uses
   (`SessionStart`, `Stop`, `UserPromptSubmit`, `PostToolUse`), each running
   `cat > /tmp/hook-<event>.json`.
2. Run `claude --version` and note it. Start Claude there, send one prompt that
   makes one tool call (for example "run `true`"), and let the turn end. This
   runs a model once.
3. Make `crates/shim/tests/fixtures/claude-code-<version>/`. Copy each file in,
   replacing the session id with `{{session_id}}` and the working directory
   with `{{cwd}}`, and remove anything personal, like paths under your home.
4. Set `CLAUDE_CODE_VERSION` in `conformance.rs` to the new version, update the
   fixture README to say the files are recorded, and update the version above.
5. Run the suite. If a cell changes, the change is a finding: report it.

For Pi, the pinned thing is the extension API, not recorded data. When
`@earendil-works/pi-coding-agent` changes in `plugins/muster-pi/package-lock.json`,
check its `ExtensionAPI` types against the stand-in in `conformance-driver.ts`
(`on`, `sendMessage`, `registerTool`, `registerCommand`, and the events the
extension handles), update the version above, and run the suite.

For opencode, the pinned thing is the plugin API, likewise. When
`@opencode-ai/plugin` changes in `plugins/muster-opencode/package-lock.json`,
check its `Plugin` input and `Hooks` types against that plugin's
`conformance-driver.ts`, update the version above, and run the suite.

For Codex, with Codex installed:

1. Run `codex --version` and note it. In a scratch directory, start a Codex
   app-server listening on a loopback WebSocket and attach a TUI to it with
   `codex --remote`, as `muster-shim codex-run` does. Check
   `codex app-server --help` for the listen flag in that version.
2. Connect a WebSocket client to the same port, for example
   `websocat ws://127.0.0.1:<port>`. Send `initialize` and `initialized`, and
   save the `initialize` result and the `thread/started` notification. Send a
   `turn/start` for the thread while a turn is running, and save the error.
   That turn runs a model.
3. For the hook input, add `[[hooks.UserPromptSubmit]]` and
   `[[hooks.PostToolUse]]` hooks that run `cat > /tmp/codex-<event>.json` to
   the scratch directory's Codex config, and run one turn that makes one tool
   call.
4. Make `crates/shim/tests/fixtures/codex-<version>/`, replace the thread id
   with `{{thread_id}}` and the cwd with `{{cwd}}`, set `CODEX_VERSION` in
   `conformance.rs`, and run the suite.
