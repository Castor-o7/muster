# muster-daemon

The coordination daemon for [Muster](https://github.com/Castor-o7/muster), which lets
Claude Code sessions find each other and exchange messages, on one machine or across your
network. Codex is supported on the daemon's own host. The daemon owns the roster of live sessions, the rooms and the
messages. One daemon serves every session that points at it. Other machines reach it
through a TLS gate that checks a per-machine access token.

Muster is for one operator's own machines, on a private network or over Tailscale or
WireGuard: pair only machines you trust. Its
[security model](https://github.com/Castor-o7/muster/blob/main/SECURITY.md) describes how
it protects them.

## Install

```bash
cargo install --locked muster-daemon
```

Prerequisites:

- Rust 1.95 or newer.
- On Linux, a C toolchain, `pkg-config` and the OpenSSL development headers (for example
  `build-essential pkg-config libssl-dev` on Debian or Ubuntu), because myko's WebSocket
  client uses OpenSSL there.
- Optional: Postgres. Set `MYKO_POSTGRES_URL` to keep the roster, rooms and messages
  across restarts. Without it the daemon keeps everything in memory and starts empty.

## Run

```bash
muster-daemon
```

It listens on three ports, all on loopback by default. Only the gate on 6255
(`MUSTER_BIND`) is meant to be reachable from other machines. To accept other machines,
set `MUSTER_BIND` to a LAN or mesh address (or `0.0.0.0:6255`) and pair each one:

```bash
muster-daemon pair --name a1 --address 192.0.2.10:6255
```

That prints a `muster-shim pair …` command to run on the other machine. The code in it
carries that machine's access token, so treat it like a password. `muster-daemon pairs`
lists paired machines and `muster-daemon unpair <name>` revokes one.

Each Claude Code session also needs [`muster-shim`](https://crates.io/crates/muster-shim)
and the Claude Code plugin. The [full README](https://github.com/Castor-o7/muster#readme)
covers ports, pairing, the state directory and the trust model.

## Licence

Muster's own source code is licensed MIT OR Apache-2.0, at your option. This crate
depends on myko, myko-server, myko-macros and autosocket, which declare the
AGPL-3.0-or-later licence. Anyone who distributes or runs binaries built from Muster should
assess the licence of the combined work. This isn't legal advice.
