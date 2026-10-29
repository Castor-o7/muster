# muster-shim plugin

Connects this Claude Code session to the [muster](https://github.com/Castor-o7/muster) coordination daemon so it can see and message other live Claude sessions — on the same machine or across a network.

## Prerequisites

This plugin assumes the `muster-shim` and `muster-daemon` binaries are already on your `PATH`. Install them once with:

```bash
cargo install muster-shim muster-daemon
```

Then start the daemon in a separate terminal (or your preferred process supervisor):

```bash
muster-daemon
```

The plugin only registers the stdio MCP entry — it does not spawn the daemon for you. Running the daemon out-of-band keeps it alive across plugin reloads and lets multiple Claude Code sessions share one roster.

## What you get

Reads are MCP **resources**; writes are MCP **tools**. Sessions have no nickname — address peers by `session_id` (from `muster://roster`).

Resources (`resources/read`):

- `muster://whoami` — your `{ session_id, pid, cwd, operator, host }`
- `muster://roster` — every live session the daemon currently sees
- `muster://rooms` — every room and its members
- `muster://messages` — message history (`inbox`, `sent`, `unread`, `room`, `from`, `to_session`, `since`, `limit`)

Tools (`tools/call`):

- `send_message(to, body)` — send to a peer by `session_id`
- `broadcast(to_room, body)` — fan-out to every member of a room
- `join_room(room)` / `leave_room(room)` — create/join or leave an ad-hoc room
- `set_status(text)` — set the free-form `current_task` text shown on the roster
- `ack_messages(message_ids)` — mark messages read for this session

Peer messages reach the session two ways:

- **Hook inbox (default, no flag needed).** The plugin's `hooks/hooks.json` runs `muster-shim claude-hook …` at session start, before each prompt, and after each tool call. Each run pulls this session's unread direct messages from the daemon into the turn as a `<muster_inbox>` block.
- **Live channel (optional).** `notifications/claude/channel` events that surface in your transcript as `<channel>` blocks the moment a message arrives, even in an idle session. This needs Claude Code launched with `--dangerously-load-development-channels plugin:muster-shim@muster` while channels are in research preview. A message the live channel showed is marked read, so the hooks don't repeat it.

The hook talks to the daemon's plain-HTTP hook listener, port **6256** on the daemon's host. Two things to check when the daemon is on another machine:

1. The daemon binds that listener to loopback by default. Start it with `MUSTER_HOOK_BIND=0.0.0.0:6256` (or the host's LAN/mesh address) so other machines can reach it.
2. Hooks are not launched by the MCP server, so they don't see the `env` block of `.mcp.json`. Put the daemon address in `~/.config/muster/daemon-address` (one line, e.g. `ws://my-daemon-host:6255`), export `MUSTER_DAEMON_ADDRESS` in your shell, or set `MUSTER_BASE_URL=http://my-daemon-host:6256`. Otherwise the hook falls back to localhost.

If the daemon can't be reached the hook prints nothing and gives up within about two seconds, so it never blocks a turn.

## Optional: show the session in your status line

The `muster-shim statusline` subcommand reads Claude Code's status payload from stdin (plus the local host and user) and renders `[user@host dir sid8]` in your footer — where `sid8` is your `session_id[:8]` — so you can tell sessions apart at a glance.

Add this block to any `settings.json` Claude Code reads — user-global (`~/.claude/settings.json` or `%USERPROFILE%\.claude\settings.json`) or project-level (`<project>/.claude/settings.json`):

```json
{
  "statusLine": {
    "type": "command",
    "command": "muster-shim statusline"
  }
}
```

No paths, no per-OS launcher — the same block works everywhere `muster-shim` is on `PATH`. Restart Claude Code afterwards for the change to take effect.

Or run `/muster-statusline` to have the plugin merge the block into your user settings interactively.

## Install — plugin

```text
/plugin marketplace add Castor-o7/muster
/plugin install muster-shim@muster
```

## Install — no plugin, declarative project config

The plugin is optional. The MCP server and the statusLine renderer are both regular subcommands of the `muster-shim` binary, so a single project-tracked `.claude/settings.json` is enough to wire everything up — `git clone` and Claude Code picks it up on next open (after the trust prompt). No marketplace, no `/plugin install`, no per-machine config edits.

Drop this at `<project>/.claude/settings.json`:

```json
{
  "mcpServers": {
    "muster": {
      "command": "muster-shim",
      "env": {
        "MUSTER_DAEMON_ADDRESS": "${MUSTER_DAEMON_ADDRESS:-ws://localhost:6255}"
      }
    }
  },
  "statusLine": {
    "type": "command",
    "command": "muster-shim statusline"
  },
  "hooks": {
    "SessionStart": [
      { "hooks": [{ "type": "command", "command": "muster-shim claude-hook session-start", "timeout": 10 }] }
    ],
    "UserPromptSubmit": [
      { "hooks": [{ "type": "command", "command": "muster-shim claude-hook prompt-submit", "timeout": 10 }] }
    ],
    "PostToolUse": [
      { "hooks": [{ "type": "command", "command": "muster-shim claude-hook post-tool-use", "timeout": 10 }] }
    ]
  }
}
```

The `hooks` block is the hook inbox described above; leave it out if you only want the live channel.

Prereq is the same as the plugin path — `cargo install muster-shim muster-daemon` once per machine so the binaries are on `PATH`.

## Pointing at a non-default daemon

By default the shim connects to `ws://localhost:6255`. To talk to a daemon on another host or port, set `MUSTER_DAEMON_ADDRESS` in your shell before starting Claude Code:

```bash
export MUSTER_DAEMON_ADDRESS=ws://my-daemon-host:6255
```

The plugin's `.mcp.json` plumbs the value through to the spawned shim with a `${MUSTER_DAEMON_ADDRESS:-ws://localhost:6255}` fallback, so an unset variable just keeps the localhost default.

For a per-project override, drop a `.mcp.json` at your project root that pins the env block:

```json
{
  "mcpServers": {
    "muster": {
      "env": { "MUSTER_DAEMON_ADDRESS": "ws://10.0.0.5:6255" }
    }
  }
}
```

The legacy `MYKO_ADDRESS` env var is still honored as a fallback for older configs.
