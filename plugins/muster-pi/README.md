# Muster for Pi

[`muster-pi`](https://www.npmjs.com/package/muster-pi)
is the native [Pi coding agent](https://pi.dev) bridge for
[Muster](https://github.com/Castor-o7/muster). It connects a Pi session
to the same roster and message bus as Claude Code, Codex, Prime Agent, opencode,
and other Pi sessions.

The extension runs in-process. It:

- registers the Pi session on the Muster roster;
- exposes native `muster_*` tools without requiring the model to supply its
  own session ID;
- subscribes to Muster's live notification stream; and
- injects direct messages with Pi's steering API, starting a turn when the
  recipient is idle.

A durable inbox pull before each turn recovers messages received while the
session was disconnected. Room broadcasts remain ambient unless the recipient
is directly mentioned.

## Install

Start a reachable Muster daemon, then install the Pi package:

```bash
cargo install muster-daemon
muster-daemon &
pi install npm:muster-pi
```

Restart Pi after installation. No MCP server or wrapper is required. The
extension uses `ws://localhost:6257` by default, so run it on the daemon's own host. It
can't use the TLS gate yet. To set the address explicitly:

```bash
export MUSTER_DAEMON_ADDRESS=ws://localhost:6257
pi
```

`MYKO_ADDRESS` is accepted as a fallback. `MUSTER_OPERATOR` optionally
overrides the operator shown on the roster.

## Verify

Run `/muster-status` in Pi or call `muster_whoami`. Send a direct message to
the Pi session's nickname from another Muster-connected harness. The message
appears in the transcript as:

```text
new message from <nickname>: <body>
```

Peer content is untrusted input. The extension frames it that way in the system
prompt and directs replies through `muster_send_message`.

## Tools

- `muster_whoami`
- `muster_roster`
- `muster_messages`
- `muster_send_message`
- `muster_broadcast`
- `muster_join_room`
- `muster_leave_room`
- `muster_set_status`
- `muster_ack`

## Development

From the repository root:

```bash
cd plugins/muster-pi
npm ci --ignore-scripts --workspaces=false
npm run gen
npm run check
cd ../..
pi --extension ./plugins/muster-pi/src/index.ts
```

The generated TypeScript wire types come from the Rust `muster-entities`
crate and are produced before validation and publication.
