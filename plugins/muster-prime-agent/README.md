# Muster for Prime Agent

`muster-prime-agent` is the native Prime Agent bridge for
[muster](https://github.com/Castor-o7/muster). It is separate from the
`muster-pi` package and targets Prime Agent's conversation lifecycle and canonical
session identities.

The extension runs inside each top-level Prime Agent conversation. Prime Agent
RLM sub-agents stay part of their parent's Muster session and are not added to the
roster independently. It:

- registers the top-level conversation on the Muster roster using Prime Agent's
  session ID;
- exposes native `muster_*` tools to the model;
- subscribes to Muster's live notification stream; and
- injects incoming direct messages with
  `pi.sendMessage(..., { deliverAs: "steer", triggerTurn: true })`.

A direct message therefore appears in the transcript immediately and starts a turn
when the recipient is idle. While the agent is working, it steers the active run at
the next tool boundary. A durable inbox pull before each turn recovers messages
received while disconnected.

## Install

Start a reachable daemon, then install the Prime Agent package:

```bash
cargo install muster-daemon
muster-daemon &
prime-agent package install npm:muster-prime-agent
```

Restart Prime Agent after installation. No MCP server or wrapper is required.
Run it on the daemon's own host, because it can't use the TLS gate yet. To set the address
explicitly:

```bash
export MUSTER_DAEMON_ADDRESS=ws://localhost:6257
prime-agent
```

The default is `ws://localhost:6257`. `MUSTER_OPERATOR` optionally overrides the
operator name shown on the roster.

For development:

```bash
cargo run -p muster-entities --no-default-features --features codegen \
  --bin typegen -- plugins/muster-prime-agent/src/generated
prime-agent --extension ./plugins/muster-prime-agent/src/index.ts
```

## Verify

Run `/muster-status` inside Prime Agent or use `muster_whoami`. Send a direct
message from another Muster session to its nickname or session ID. It appears as:

```text
new message from <nickname>: <body>
```

Peer content is untrusted input. The extension adds that warning to the system
prompt and directs replies through `muster_send_message`. Room broadcasts remain
ambient unless the recipient is directly mentioned.

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
