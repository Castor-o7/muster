# Security

## Reporting a vulnerability

Please report vulnerabilities privately through GitHub's
[security advisory form](https://github.com/ignition-is-go/marshal/security/advisories/new),
not in a public issue. Include what you found, how to reproduce it, and what it affects.

## Security model

Marshal coordinates agents on machines you own: one operator, on a private network or a VPN
such as Tailscale or WireGuard. It isn't meant to be exposed to the open internet.

- **Plain mode (the default).** The daemon has no authentication, and any client that can
  reach it can act as any session. It's protected only by where it's bound, so keep
  `MARSHAL_BIND` on loopback or a network you trust.
- **TLS mode (`marshal-daemon --tls`).** Each machine pairs with a one-time code from
  `marshal-daemon pair`, then reaches the daemon over TLS with the daemon's certificate
  pinned and a token of its own. `marshal-daemon unpair` revokes the token and closes that
  machine's connections within seconds. Tools on the daemon's own host present a local token
  the daemon writes owner-only, so that machine should be one only you use.
- **Session binding.** With TLS on, the daemon ties each session to the machine that
  registered it, so one paired machine can't read, clear or send as another machine's
  session. The gate enforces this by filtering each client frame, and
  `scripts/test-gate-e2e.sh` covers it.
- **Shared roster.** Every paired machine can see the roster, including each session's
  operator identity, machine and working directory.
- **Messages aren't approvals.** A message shows which session sent it, not which person, so
  agents treat a peer's message as information. Anything that needs the operator, such as a
  restart, needs the operator's own word.

[docs/threat-model.md](docs/threat-model.md) has the details, including the known gaps.

## Scope

Reports about the daemon's gate, pairing, session binding and message delivery are all in
scope.
