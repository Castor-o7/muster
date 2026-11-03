# muster-shim

The per-session client for [Muster](https://github.com/Castor-o7/muster), which lets
Claude Code sessions find each other and exchange messages, on one machine or across your
network. `muster-shim` is the stdio MCP server Claude Code starts for each session, and
the command the plugin's hooks run to deliver messages and wake an idle session. It
connects to a running [`muster-daemon`](https://crates.io/crates/muster-daemon). It also
sets up Codex on the daemon's own host (`muster-shim codex-setup`).

Muster is for one operator's own machines, on a private network or over Tailscale or
WireGuard: pair only machines you trust. Its
[security model](https://github.com/Castor-o7/muster/blob/main/SECURITY.md) describes how
it protects them.

**What the shim shares.** The shim puts each session on a roster that every paired machine
can read, with its operator identity: your Claude account email, or failing that the
workspace's git `user.email`, then your login name. Set `MUSTER_OPERATOR` to use a
different identity. On a paired machine, the daemon's `MUSTER_OPERATOR`, if set, decides
the operator shown instead. The roster also shows the session's title (the name you gave
it in Claude Code, which can reveal what you're working on), process id, host name, OS and
architecture, working directory, project, git branch, status, activity (busy, idle or
shell), kind (interactive or background), and the name of its last MCP tool call and when
it ran (names only, never arguments).

## Install

```bash
cargo install --locked muster-shim
```

Prerequisites:

- Rust 1.95 or newer.
- On Linux, a C toolchain, `pkg-config` and the OpenSSL development headers (for example
  `build-essential pkg-config libssl-dev` on Debian or Ubuntu).
- A reachable `muster-daemon`. On a machine other than the daemon's host, pair it first
  with the `muster-shim pair …` command that `muster-daemon pair` prints.

## Connect Claude Code

Installing the binary alone doesn't connect Claude Code. Add the plugin, which registers
the MCP server and the hooks that deliver messages:

```text
/plugin marketplace add Castor-o7/muster
/plugin install muster-shim@muster
```

Then restart the session. To wire the MCP server and hooks by hand instead, and for Codex,
see the [full README](https://github.com/Castor-o7/muster#readme).

From a terminal on a paired machine or the daemon's host, `muster-shim all "<message>"`
messages every session and `muster-shim tell <name> "<message>"` messages one.

## Licence

Muster's own source code is licensed MIT OR Apache-2.0, at your option. This crate
depends on myko, myko-macros and autosocket, which declare the AGPL-3.0-or-later licence.
Anyone who distributes or runs binaries built from Muster should assess the licence of the
combined work. This isn't legal advice.
