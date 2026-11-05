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
| 3 | `dropped-ack-redelivers` | When the ack of a delivery is lost, the message comes again once the lease lapses, and not after an ack that arrives. |
| 4 | `duplicate-shown-at-most-once` | A send retried with the same idempotency key is one message, shown once. Mail the waiter delivered isn't injected again by the next hook. |
| 5 | `daemon-restart-mid-wait` | The daemon restarts (SIGTERM, as systemd stops it) while the session waits. The session still wakes on mail sent after the restart. |
| 6 | `operator-pause-stops-wakes` | With `muster-daemon pause`, mail doesn't wake an idle session but still arrives at the next hook. |
| 7 | `broadcast-never-wakes` | A room broadcast neither wakes the session nor is injected. |
| 8 | `wake-none-not-expected` | A session the roster calls `wake: none` isn't reported to the sender as `wake: expected`. |

Scenario 8 fails today and is listed in `KNOWN_FAILURES` against MUS-71. A
known failure shows as `known (MUS-71)` in the grid. If one starts passing, the
suite fails until it's taken off the list.

## The harnesses

| Column | Harness | Mode | Status |
|---|---|---|---|
| `claude` | Claude Code | plain | built |
| `claude-tls` | Claude Code | `MUSTER_TLS=1`, as Murph runs: the hooks carry the local token, and the shim connects through the gate's port | built |
| `codex` | Codex, through `muster-shim codex-bridge` and the app-server | plain | stage 2 |
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

## Gaps

These are listed so nobody reads the grid as more than it is.

- **Codex, opencode, Pi and Prime Agent have no fake yet** (stages 2 and 3).
  Their columns read `not yet`. The TypeScript plugins' own tests don't run in
  CI either. Stage 3 adds bun to CI, downloaded from bun's GitHub release at a
  pinned version and checked against a SHA-256 committed in the workflow.
- **The Claude fixtures are constructed, not recorded.** They follow Claude
  Code's documented hook input. Recording them from a live run is below. The
  shim reads only `session_id` and `cwd` today, so the risk is small.
- **Restarts are tested without Postgres.** Mail sent before a restart and not
  yet delivered lives only in memory, and is lost with it. Scenario 5 sends
  after the restart. The durable case needs a database in CI.
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

Stage 2 adds the same for the Codex app-server's JSON-RPC.
