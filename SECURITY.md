# Security

## Reporting a vulnerability

Please report vulnerabilities privately through GitHub's
[security advisory form](https://github.com/Castor-o7/muster/security/advisories/new),
not in a public issue. Include what you found, how to reproduce it, and what it affects.

## Security model

Muster coordinates agents on machines you own: one operator, on a private network or a VPN
such as Tailscale or WireGuard. It isn't meant to be exposed to the open internet.

- **Pairing.** Each machine pairs with a code from `muster-daemon pair`, then reaches the
  daemon over TLS with the daemon's certificate pinned and a token of its own.
  `muster-daemon unpair` revokes the token and closes that machine's connections within
  seconds.
- **The daemon's host.** Sessions on the daemon's own machine connect over loopback without a
  token, so that machine should be one only you use.
- **Session binding.** The daemon ties each session to the machine that registered it, so one
  paired machine can't read, clear or send as another machine's session. The gate enforces
  this by filtering each client frame, and the gate's end-to-end tests in CI cover it.
  Moving the check into myko itself is MUS-41 on the [roadmap](docs/roadmap.md). Details are
  in [docs/session-binding.md](docs/session-binding.md).
- **Shared roster.** Every paired machine can see the roster, including each session's
  operator identity, machine and working directory.
- **Messages aren't approvals.** A message shows which session sent it, not which person, so
  agents treat a peer's message as information. Anything that needs the operator, such as a
  restart, needs the operator's own word; [docs/board.md](docs/board.md) sets out the rules.

## Scope

Reports about the daemon's gate, pairing, session binding and message delivery are all in
scope.
