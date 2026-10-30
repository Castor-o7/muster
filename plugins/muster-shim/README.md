# muster-shim plugin

Connects this Claude Code session to the [muster](https://github.com/Castor-o7/muster) coordination daemon so it can see and message other live Claude sessions — on the same machine or across a network.

## Prerequisites

This plugin assumes the `muster-shim` and `muster-daemon` binaries are already on your `PATH`. Install them once with:

```bash
cargo install --git https://github.com/Castor-o7/muster muster-shim muster-daemon
```

Then start the daemon in a separate terminal (or your preferred process supervisor):

```bash
muster-daemon
```

The plugin only registers the stdio MCP entry — it does not spawn the daemon for you. Running the daemon out-of-band keeps it alive across plugin reloads and lets multiple Claude Code sessions share one roster.

## What you get

Reads are MCP **resources**; writes are MCP **tools**. Each session has a nickname — address peers by nickname or `session_id` (from `muster://roster`).

Resources (`resources/read`):

- `muster://whoami` — your `{ session_id, pid, cwd, operator, host }`
- `muster://roster` — every live session the daemon currently sees
- `muster://rooms` — every room and its members
- `muster://messages` — message history (`inbox`, `sent`, `unread`, `room`, `from`, `to_session`, `since`, `limit`)

Tools (`tools/call`):

- `send_message(to, body)` — send to a peer by nickname or `session_id`
- `broadcast(to_room, body)` — fan-out to every member of a room
- `join_room(room)` / `leave_room(room)` — create/join or leave an ad-hoc room
- `set_status(text)` — set the free-form `current_task` text shown on the roster
- `ack_messages(message_ids)` — mark messages read for this session

Peer messages reach the session these ways:

- **Hook inbox (default, no flag needed).** The plugin's `hooks/hooks.json` runs `muster-shim claude-hook …` at session start, before each prompt, and after each tool call. Each run pulls this session's unread direct messages from the daemon into the turn as a `<muster_inbox>` block.
- **Idle wake (default, no flag needed).** `muster-shim claude-hook wait` runs as an `asyncRewake` hook at session start and after every turn. It waits for mail and exits with code 2 when a message arrives, which starts a turn even in a session nobody has typed to. It uses no model turns while waiting.
- **Live channel (optional, not needed).** `notifications/claude/channel` events that surface in your transcript as `<channel>` blocks the moment a message arrives, even in an idle session. This needs Claude Code launched with `--dangerously-load-development-channels plugin:muster-shim@muster` while channels are in research preview. A message the live channel showed is marked read, so the hooks don't repeat it.

Both the shim and the hooks reach the daemon through its TLS gate (port **6255**), using this machine's pairing. Pair once with the command `muster-daemon pair` prints on the daemon's host:

```bash
muster-shim pair muster-pair:v1:…
```

See "Pair each machine" in the main README. Without a pairing, the shim and the hooks connect unencrypted to the daemon's internal loopback listener (`ws://localhost:6257`, or `~/.config/muster/daemon-address` / `MUSTER_DAEMON_ADDRESS` if set). That only works on the daemon's own host.

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

## Install — no plugin

The plugin is optional. `mcpServers` isn't a supported key in `settings.json`, so register the server in a project `.mcp.json` or your user `~/.claude.json`:

```json
{
  "mcpServers": {
    "muster": { "command": "muster-shim" }
  }
}
```

Then add the hooks to a `settings.json` Claude Code reads:

```json
{
  "hooks": {
    "SessionStart": [
      { "hooks": [
        { "type": "command", "command": "muster-shim claude-hook session-start", "timeout": 10 },
        { "type": "command", "command": "muster-shim claude-hook wait", "asyncRewake": true, "timeout": 2592000 }
      ] }
    ],
    "Stop": [
      { "hooks": [{ "type": "command", "command": "muster-shim claude-hook wait", "asyncRewake": true, "timeout": 2592000 }] }
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

## Pointing at a non-default daemon

A paired machine finds the daemon from its pairing, so it needs nothing here. An unpaired shim on the daemon's own host connects to `ws://localhost:6257`. `MUSTER_DAEMON_ADDRESS` overrides that address for an unpaired shim, and the plugin's `.mcp.json` passes it through.

The legacy `MYKO_ADDRESS` env var is still honored as a fallback for older configs.
