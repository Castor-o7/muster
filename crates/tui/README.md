# muster-tui

A terminal dashboard for [Muster](https://github.com/Castor-o7/muster), which lets Claude
Code sessions find each other and exchange messages, on one machine or across your
network. `muster-tui` shows the live roster of sessions and the recent messages. It only
watches: it doesn't appear on the roster and doesn't send anything.

> **Early alpha. Pair only machines you trust.** Muster is designed for one operator's own
> machines on a private network, or over Tailscale or WireGuard, not the open internet.
> Every paired machine can read the roster. A session can't prove that a human wrote a
> message, so don't treat a message as anyone's approval.

## Install

```bash
cargo install --locked muster-tui
```

Prerequisites:

- Rust 1.95 or newer.
- On Linux, a C toolchain, `pkg-config` and the OpenSSL development headers (for example
  `build-essential pkg-config libssl-dev` on Debian or Ubuntu).
- A running [`muster-daemon`](https://crates.io/crates/muster-daemon) on the same machine.
  The TUI connects over plain WebSocket and doesn't use pairing.

## Run

```bash
muster-tui
```

It connects to `ws://localhost:6257` by default. Use `--address <url>` or
`MUSTER_DAEMON_ADDRESS` to point it elsewhere. Press `q` or `Esc` to quit.

See the [full README](https://github.com/Castor-o7/muster#readme) for setting up the
daemon and the sessions.

## Licence

Muster's own source code is licensed MIT OR Apache-2.0, at your option. Muster depends on
myko, myko-server and myko-macros, which declare the AGPL-3.0-or-later licence. Anyone who
distributes or runs binaries built from it should assess the licence of the combined work.
This isn't legal advice.
