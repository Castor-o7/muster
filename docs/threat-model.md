# Muster threat model

Muster carries messages and wake-ups between coding-agent sessions on the
operator's machines. This page says what it protects, from whom, how, and where
the protection stops. Decided scope is in the roadmap's Decisions; the stories
named here are in [roadmap.md](roadmap.md).

## What's protected

- **Acting as a session.** Sending, reading or acking mail as a session, or
  changing its roster row. A forged message can steer an agent that trusts it.
- **Waking a session.** A message that wakes an idle agent spends the operator's
  tokens and starts work nobody asked for.
- **Credentials.** Pairing tokens, the local token, the gate's TLS key.
- **Mail.** Message bodies, which can hold code, paths and plans.
- **The daemon's availability.** One misbehaving client shouldn't stop the rest.

## Who it defends against

In scope, per the roadmap's Decisions:

1. **A malicious website** open in a browser on any of the operator's machines.
   It can make the browser send requests to `127.0.0.1` and LAN addresses.
2. **One compromised session**, such as an agent that has been prompt-injected
   into misusing its tools. It runs as the operator's account.
3. **One compromised or stolen paired machine**, holding that machine's token.
4. **Other devices on the LAN**, which can reach the gate's port.

Out of scope for now: other people with accounts on the operator's machines,
the open internet (the gate stays on the LAN or a Tailscale/WireGuard network),
and anyone with root on the daemon's host.

## The surfaces

| Port | Listener | Who can reach it | What guards it |
|---|---|---|---|
| 6255 | TLS gate | the LAN (Murph's firewall, MUS-56) | TLS with a pinned certificate; a per-machine token; the frame filter; per-IP and per-machine connection caps |
| 6256 | hook listener | the daemon's host | the local token; refuses any `Origin` or upgrade; a connection cap |
| 6257 | local gate | the daemon's host | the local token; refuses any `Origin`; only myko's WebSocket route; the frame filter as `(local)`; a connection cap |
| 6258 | myko's own listener | the daemon's host | admits only requests a gate forwarded (the local token), refuses any `Origin`, via the patched myko-server |

## How each attacker is handled

**A malicious website.** Every port on the daemon's host refuses a request
naming a browser `Origin` (MUS-7, MUS-59). A page can't read the local token,
and the gate needs a token a page doesn't have. Pairing codes are one-time and
expire (MUS-8), so a code a page somehow sees is soon useless.

**A compromised session.** The gate's frame filter lets a paired machine act
only as sessions it owns, and the hook listener checks the same bindings
(MUS-5, MUS-6, MUS-33 to MUS-35). Message rates are limited per session and per
machine (MUS-9, MUS-60). Pairing codes and tokens are masked in logs and message
bodies (MUS-57). The operator can pause every wake-up and limit which machines
may send (MUS-10). What this does *not* stop: a session runs as the operator's
account, so it can read the local token, the stored pairing and the daemon's
state directory, and so can change the pause and allowlist, or claim to relay
the operator's word. Approvals a session can't forge are MUS-39.

**A compromised paired machine.** Its token reaches only the gate, where it can
act only as its own sessions, and only up to its connection and message caps.
`muster-daemon unpair` revokes it within seconds. It can read messages its own
sessions receive and the roster, which the one-operator scope makes low value.

**Other LAN devices.** They reach the gate's port but get nowhere without a
token: TLS comes first, and a connection without a valid token gets 401.
MUS-56 narrows the firewall to listed machines.

## Known gaps

- **The operator's account is one trust zone.** Anything running as the operator
  on the daemon's host can act as any local session and change the controls.
  MUS-39 is the fix.
- **The frame filter parses separately from myko.** That is where the earlier
  bypasses came from. MUS-41 moves authorization into myko.
- **Claimed fields.** A session through the local gate keeps the host name it
  claims, which the per-machine message limit and the sender allowlist trust.
  Both need the local token, so this is part of the first gap.
- **myko is patched locally** (`vendor/myko-server`, MUS-59) until upstream has
  an equivalent hook; the request to myko's maintainers is pending.
- **Dependencies.** CI runs `cargo deny` (MUS-11, `deny.toml`). Its ignored
  advisories are listed there with reasons. Four of the web UI's git
  dependencies declare no licence and are excluded from the check (MUS-61).
