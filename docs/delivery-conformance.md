# Delivery conformance

MUS-69. One scenario table, run against every harness Muster delivers to, with
the real `muster-daemon` and `muster-shim` binaries and no TUI or model. Each
harness is a fake that drives the real binaries the way its TUI does. The suite
is `crates/shim/tests/conformance.rs`.

```sh
cargo build --workspace
cargo test -p muster-shim --test conformance -- --ignored --nocapture
```

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
| 4 | `duplicate-shown-at-most-once` | A send retried with the same idempotency key is one message, shown once. Mail the waiter delivered isn't injected again by the next hook. |
| 5 | `daemon-restart-mid-wait` | The daemon restarts (SIGTERM, as systemd stops it) while the session waits. The session still wakes on mail sent after the restart. |
| 6 | `operator-pause-stops-wakes` | With `muster-daemon pause`, mail doesn't wake an idle session but still arrives at the next hook. |
| 7 | `broadcast-never-wakes` | A room broadcast neither wakes the session nor is injected. |
| 8 | `wake-none-not-expected` | A session the roster calls `wake: none` isn't reported to the sender as `wake: expected`. |

Some cells fail today. Each is listed in `KNOWN_FAILURES` against the story
that fixes it, and shows as `known (MUS-nn)` in the grid. If one starts
passing, the suite fails until it's taken off the list.

| Cells | Story | What fails |
|---|---|---|
| `codex` × 6 | MUS-74 | The Codex bridge wakes a session while the operator has paused wakes. It never checks the pause. |
| all × 3b, `codex` × 3 | MUS-75 | A prompt hook's mail is marked read once the daemon has flushed the reply. If the reply is then lost, the mail is gone, though the hook-delivery doc says a dropped hook leaves it unread. |
| `codex` × 5 | MUS-76 | After a restart without Postgres, an idle Codex session is off the roster until its next hook: the bridge registers a thread only on `thread/started`. |

## The harnesses

| Column | Harness | Mode | Status |
|---|---|---|---|
| `claude` | Claude Code | plain | built |
| `claude-tls` | Claude Code | `MUSTER_TLS=1`, as Murph runs: the hooks carry the local token, and the shim connects through the gate's port | built |
| `codex` | Codex, through `muster-shim codex-bridge` and the app-server | plain | built |
| `opencode` | opencode plugin (`plugins/muster-opencode`) | plain | stage 3 |
| `pi` | Pi plugin (`plugins/muster-pi`) | plain | stage 3 |
| `prime-agent` | Prime Agent plugin (`plugins/muster-prime-agent`) | plain | stage 3 |

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

## Gaps

These are listed so nobody reads the grid as more than it is.

- **opencode, Pi and Prime Agent have no fake yet** (stage 3). Their columns
  read `not yet`. The TypeScript plugins' own tests don't run in
  CI either. Stage 3 adds bun to CI, downloaded from bun's GitHub release at a
  pinned version and checked against a SHA-256 committed in the workflow.
- **The fixtures are constructed, not recorded.** The Claude fixtures follow
  Claude Code's documented hook input. The Codex fixtures follow the frames the
  bridge's own tests use, because Codex isn't installed where they were
  written. Recording both is below. The shim reads only a few fields from each
  (the ids and cwd), so the risk is small.
- **Codex runs in plain mode only.** It has no `codex-tls` column yet.
- **Restarts are tested without Postgres** (MUS-76). Mail sent before a
  restart and not yet delivered lives only in memory, and is lost with it.
  Scenario 5 sends after the restart. The durable case needs a database in CI.
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
