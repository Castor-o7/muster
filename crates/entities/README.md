# muster-entities

Shared types for [Muster](https://github.com/Castor-o7/muster), which lets Claude Code
sessions find each other and exchange messages, on one machine or across your network.
This library holds the myko entities (sessions, messages, rooms and room members), the
server-handled commands such as `SendMessage` and `BroadcastMessage`, and the pairing-code
format shared by the daemon and the shim. It's published because `muster-daemon`,
`muster-shim` and `muster-tui` depend on it. Its API isn't stable.

> **Early alpha. Pair only machines you trust.** Muster is designed for one operator's own
> machines on a private network, or over Tailscale or WireGuard, not the open internet.
> Every paired machine can read the roster. A session can't prove that a human wrote a
> message, so don't treat a message as anyone's approval.

## Install

You don't install this crate on its own. To install Muster:

```bash
cargo install --locked muster-daemon muster-shim
```

Prerequisites: Rust 1.95 or newer, and on Linux a C toolchain, `pkg-config` and the
OpenSSL development headers (for example `build-essential pkg-config libssl-dev` on Debian
or Ubuntu).

See the [full README](https://github.com/Castor-o7/muster#readme) for setup.

## Licence

Muster's own source code is licensed MIT OR Apache-2.0, at your option. Muster depends on
myko, myko-server and myko-macros, which declare the AGPL-3.0-or-later licence. Anyone who
distributes or runs binaries built from it should assess the licence of the combined work.
This isn't legal advice.
