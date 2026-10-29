# muster-opencode

The [opencode](https://opencode.ai) counterpart of the Claude Code `muster-shim`. Connects an opencode session to the [muster](../../README.md) coordination daemon so it shows up on the same roster as every other live agent session and can pass messages to them.

Where the Claude integration needs a shim **binary** (an MCP child process), a proprietary `notifications/claude/channel` push, curl **hooks**, and `~/.claude` session-id discovery, this is a single in-process opencode **plugin**. opencode's plugin hooks supply everything the shim had to bolt on for Claude:

| Concern | Claude `muster-shim` | this plugin |
|---|---|---|
| Tools (send/broadcast/…) | stdio MCP server | native opencode tools (`muster_*`), acting session auto-filled |
| Roster registration | shim startup + `/hook/session-start` | `event` hook → `SET Session` over the myko client |
| Session id | `CLAUDE_CODE_SESSION_ID` / `~/.claude` heuristics | handed to the plugin by opencode |
| Live inbound | `notifications/claude/channel` (proprietary, flag-gated) | `client.tui.showToast` on the daemon's `NotifyChannel` push |
| In-context delivery | `/hook/prompt-submit` → `<muster_inbox>` | `experimental.chat.system.transform` → `<muster_inbox>` |
| Liveness / cleanup | 5 s shim loop + `/hook/session-end` | 5 s timer + `session.deleted` |

It talks to the daemon with [`@myko/core`](https://www.npmjs.com/package/@myko/core) — the same myko client the rest of the stack uses — so there is no second source of truth for the wire protocol (the muster command/query/event shapes live in [`src/entities.ts`](src/entities.ts), hand-mirrored from the Rust `muster-entities` crate).

## Requirements

- A reachable `muster-daemon` (see the [top-level README](../../README.md)). The plugin defaults to `ws://localhost:6255`.
- opencode (the plugin runs under opencode's Bun runtime).

## Install

### Option A — npm package (once published)

> **Not published from CI yet.** `muster-opencode` 0.52.0 was
> published by hand and has no npm trusted publisher, so `npm publish` from the
> release workflow is rejected with `E403 OIDC permission denied`. The package
> is deliberately left out of the flux `publish` task until a maintainer adds a
> Trusted Publisher (GitHub Actions, `Castor-o7/muster`, `release.yml`) on
> npmjs.com; otherwise its failure blocks muster-pi and muster-prime-agent,
> which publish after it. The fleet installs this plugin from the git tree, not
> npm (pulse-deploy `pulse_agents_execution`), so nothing depends on the gap.

Add it to your opencode config (`opencode.json` / `~/.config/opencode/opencode.json`):

```jsonc
{
  "$schema": "https://opencode.ai/config.json",
  "plugin": ["muster-opencode"]
}
```

opencode installs the plugin and its `@myko/core` dependency into `~/.cache/opencode/node_modules` on next start.

### Option B — local checkout (development)

From this directory:

```bash
bun install
```

then point opencode at the local plugin. Either symlink the package into opencode's global plugin dir:

```bash
ln -s "$PWD" ~/.config/opencode/plugin/muster-opencode
```

or reference it from `opencode.json` by path (see `opencode.example.jsonc`).

## Configure

Set these in the environment opencode launches under:

| Var | Default | Meaning |
|---|---|---|
| `MUSTER_DAEMON_ADDRESS` | `ws://localhost:6255` | daemon WebSocket URL (`MYKO_ADDRESS` honored as a fallback) |
| `MUSTER_OPERATOR` | `$USER` / `$USERNAME` | who this session belongs to (anchors the `op:*` room) |

## What you get

Tools (call them like any other opencode tool):

- `muster_roster` — list every live session (host, cwd, branch, operator, session id)
- `muster_send_message` — direct-message a peer by session id
- `muster_broadcast` — message a room (`everyone`, `host:*`, `op:*`, `project:*`, ad-hoc)
- `muster_join_room` / `muster_leave_room`
- `muster_set_status` — set the status peers see on the roster

Inbound peer messages arrive two ways: an instant TUI toast (real-time) and, authoritatively, as a `<muster_inbox>` block injected at the start of your next turn (untrusted-input framed, then acked).

Muster is optional enrichment, not a turn dependency. If the daemon is
disconnected or an inbox command does not complete promptly, the hook skips
inbox injection and lets the OpenCode turn continue; unread messages remain in
Muster for a later connected turn.

## Testing

```bash
bun test                              # unit + integration
bun test test/entities.test.ts test/daemon.test.ts  # unit only
```

- **Unit** (`test/entities.test.ts`, `test/daemon.test.ts`) pins the muster wire shapes and proves a disconnected coordination daemon cannot block an OpenCode turn.
- **Integration** (`test/integration.test.ts`) spins up a **real `muster-daemon` binary** and round-trips the plugin's actual `MusterDaemon` (the `@myko/core` client) against it over the real myko WS wire: roster registration + entity fields, send → inbox-pull + ack, the real-time `NotifyChannel` push, `join_room` + `broadcast` delivery, and `set_status`. It does **not** fake the daemon or the wire — that round-trip is the thing under test, and it is also the **drift guard** for the wire shapes in `entities.ts`: rename a field in the Rust `muster-entities` source and the round-trip fails. The suite skips (loudly) if no daemon binary is found; enable it with:

  ```bash
  (cd ../.. && cargo build -p muster-daemon)   # builds target/debug/muster-daemon
  bun test test/integration.test.ts             # or set MUSTER_DAEMON_BIN=/path/to/muster-daemon
  ```

## Status / follow-ups

What's verified: typecheck against the real `@opencode-ai/plugin` + `@myko/core` types, and the **full daemon wire round-trip** (the 4 integration tests above). Not yet exercised: the opencode-hook glue inside a *live opencode session* (the hooks are typechecked against opencode's real types but unrun end-to-end in opencode itself).

- `src/entities.ts` is a small hand-mirror of the Rust `muster-entities` wire shapes, kept honest by the integration test above (the drift guard). `muster-entities` *can* emit these as TypeScript via `cargo run -p muster-entities --features codegen --bin typegen -- <dir>` (the `typegen` bin is wired and works), but ts-rs dumps the whole linked type graph — muster's types plus all of myko's base types — so consuming the full static surface wasn't worth the build pipeline for this 6-verb plugin. The hand-mirror + live round-trip is the deliberate trade.
- A pulse-deploy Ansible role to deploy + wire this on the fleet is a separate follow-up (parallel to `roles/muster_shim*`).
- A couple of opencode-surface details (the exact field carrying the session id into `experimental.chat.system.transform`, and toast variant names) are handled defensively; verify against the opencode version you run.

> Note: the daemon serves its WebSocket at the `/myko` path. The plugin accepts a bare `ws://host:6255` (the fleet convention) and appends `/myko` automatically — see `withMykoPath` in `src/daemon.ts`.
