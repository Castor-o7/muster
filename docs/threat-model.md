# Marshal threat model

Marshal carries messages and wake-ups between coding-agent sessions on the
operator's machines. This page says what the TLS gate protects, from whom, how,
and where the protection stops.

## Which mode this covers

The daemon runs plain by default, and with the gate only with `marshal-daemon
--tls`. **Everything below is about TLS mode.** In plain mode it doesn't apply:
myko serves `MARSHAL_BIND` with no authentication, `asSession` is
trust-on-assert, there are no limits, and the daemon is protected only by where
it's bound. The startup line names the mode. So that a gated daemon can't become
an open one by losing a flag, it refuses to start plain while its state
directory has paired machines, unless `--plain` says so explicitly.

## What's protected

- **Acting as a session.** Sending, reading or acking mail as a session, or
  changing its roster row. A forged message can steer an agent that trusts it.
- **Waking a session.** A message that wakes an idle agent spends the operator's
  tokens and starts work nobody asked for.
- **Credentials.** Pairing tokens, the local token, the gate's TLS key.
- **Mail.** Message bodies, which can hold code, paths and plans.
- **The daemon's availability.** One misbehaving client shouldn't stop the rest.

## Who it defends against

1. **A malicious website** open in a browser on any of the operator's machines.
   It can make the browser send requests to `127.0.0.1` and LAN addresses.
2. **One compromised session**, such as an agent that has been prompt-injected
   into misusing its tools. It runs as the operator's account.
3. **One compromised or stolen paired machine**, holding that machine's token.
4. **Other devices on the LAN**, which can reach the gate's port.

Out of scope: other people with accounts on the operator's machines, the open
internet (the gate stays on the LAN or a VPN such as Tailscale or WireGuard),
and anyone with root on the daemon's host.

## The surfaces

| Port | Listener | Who can reach it | What guards it |
|---|---|---|---|
| 6155 | the gate | the LAN, if `MARSHAL_BIND` allows | TLS with a pinned certificate; a per-machine token; the frame filter; per-IP and per-machine connection caps. Plain HTTP from loopback takes the local path (below); plain HTTP from anywhere else gets `426` |
| 6155, plain from loopback | the gate's local path | the daemon's host | the local token; refuses any `Origin`; only myko's WebSocket route; the frame filter as `(local)`; a connection cap |
| 6156 | hook listener | the daemon's host | the local token; refuses any `Origin` or upgrade; a connection cap |
| 6158 | myko's own listener | the daemon's host | admits only requests the gate forwarded (the local token), refuses any `Origin`, via the patched myko-server |

**Loopback is not a trust boundary.** The gate sends plain HTTP from a loopback
peer to the local path. A reverse proxy on the daemon's host, such as an HTTPS
front end for the web UI, forwards remote requests from `127.0.0.1`, so the gate
can't tell them from a local tool by address. That's still safe, because the
local path requires the local token, which a remote client doesn't have, and
refuses any `Origin`, which a browser behind the proxy sends. The token is the
boundary, not the address.

## How sessions are tied to machines

The first paired machine to use a session id owns it, and the daemon keeps that
binding in its state directory, so it survives a restart. Paired machines'
session ids must be UUIDs, so they can't pose as a nickname or an operator.

- **Hooks.** The gate strips any machine mark the client sent and adds the
  verified machine name. The hook listener refuses a session bound to another
  machine.
- **WebSocket.** The gate reads each frame a paired machine sends and allows
  only queries and subscriptions it knows, commands whose `asSession` is absent
  or the machine's own, and events that set a `Session` the machine owns.
  Anything else, including an unparseable frame, is refused and logged. Message
  bodies are readable only through a session's own `ReadMessages`. myko's other
  routes are closed to paired machines.
- **Attestation.** For a paired machine, the daemon records the pairing name as
  the session's host, and `MARSHAL_OPERATOR` as its operator, whatever the
  session claims.

## How each attacker is handled

**A malicious website.** Every port on the daemon's host refuses a request
naming a browser `Origin`. A page can't read the local token, and the gate needs
a token a page doesn't have. Pairing codes are one-time and expire, so a code a
page somehow sees is soon useless.

**A compromised session.** A paired machine's sessions can act only as
themselves, as above. Message rates are limited per session and per machine.
Pairing codes and tokens are masked in logs and message bodies. The operator can
pause every wake-up and limit which machines may send. What this does *not*
stop: a session runs as the operator's account, so it can read the local token,
the stored pairing and the daemon's state directory, and so can change the pause
and allowlist, or claim to relay the operator's word.

**A compromised paired machine.** Its token reaches only the gate, where it can
act only as its own sessions, and only up to its connection and message caps.
`marshal-daemon unpair` revokes it within seconds. It can read messages its own
sessions receive and the roster.

**Other LAN devices.** They reach the gate's port but get nowhere without a
token: TLS comes first, and a connection without a valid token gets `401`.

## Known gaps

- **The operator's account is one trust zone.** Anything running as the operator
  on the daemon's host can act as any local session and change the controls.
- **The frame filter parses separately from myko.** It's an allowlist, but a
  frame myko reads differently from the filter could still get through. An
  authorization hook in myko itself would let the filter retire.
- **First use wins.** A session id nobody has used through a hook or the gate can
  be claimed by whichever machine names it first, and roster ids are visible to
  paired machines.
- **Reads are only partly limited.** The roster, rooms and memberships stay
  readable by every paired machine.
- **Claimed fields.** A session on the daemon's own host keeps the host and
  operator it claims, which the per-machine message limit and the sender
  allowlist trust. Acting there needs the local token, so this is part of the
  first gap.
- **myko is patched locally** (`vendor/myko-server`) until myko-server has an
  equivalent hook.
- **Codex, Pi, opencode and Prime Agent** clients don't support pairing yet, and
  the web UI needs TLS off.
