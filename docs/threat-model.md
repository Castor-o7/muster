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
- **Mail.** Message bodies, which can hold code, paths and plans. In memory only,
  unless the operator keeps the store: in Postgres, or with `MUSTER_PERSIST=file` in
  `store.jsonl` in the state directory (MUS-17), a `0600` file the daemon refuses to
  open if it's readable by others. Pairing codes and tokens are redacted from bodies
  before they're stored (MUS-57).
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

## Which mode this covers

The daemon runs plain by default, as upstream Marshal does, and with the gate
only with `--tls` or `MUSTER_TLS=1` (MUS-64). **Everything in this document is
about TLS mode.** In plain mode it doesn't apply: myko serves `MUSTER_BIND`
with no authentication, `asSession` is trust-on-assert, there are no limits, and
the daemon is protected only by where it's bound. The startup line names the
mode. So that a gated daemon can't become an open one by losing a setting, it
refuses to start plain while its state directory has paired machines, unless
`MUSTER_TLS=0` says so explicitly.

## The surfaces

| Port | Listener | Who can reach it | What guards it |
|---|---|---|---|
| 6255 | TLS gate | the LAN (Murph's firewall, MUS-56) | TLS with a pinned certificate; a per-machine token; the frame filter; per-IP and per-machine connection caps. Plain HTTP from loopback takes the local gate's path (below); plain HTTP from anywhere else gets `426` |
| 6256 | hook listener | the daemon's host | the local token; refuses any `Origin` or upgrade; a connection cap |
| 6255 (plain, loopback) | local gate | the daemon's host | the local token; refuses any `Origin`; only myko's WebSocket route; the frame filter as `(local)`; a connection cap. The old 6257 port was retired in MUS-70 |
| 6258 | myko's own listener | the daemon's host | admits only requests a gate forwarded (the local token), refuses any `Origin`, via the patched myko-server |

**Loopback is not a trust boundary.** The gate sends plain HTTP from a loopback
peer to the local path. A reverse proxy on the daemon's host, such as an HTTPS
front end for the web UI, forwards remote requests from `127.0.0.1`, so the gate
can't tell them from a local tool by address. That's still safe, because the
local path requires the local token, which a remote client doesn't have, and
refuses any `Origin`, which a browser behind the proxy sends. The token is the
boundary, not the address.

**Links between daemons (MUS-66).** A linked daemon reaches only the gate's
`/link` route, with a link token that opens nothing else. It speaks a small
protocol of its own, never myko's frames. It may send mail only from sessions in
its own last roster, only to local sessions, and never as a local session or
another link's (colliding ids are refused). Its mail is rate-limited per link and
per remote sender, with `deferred` answers. It's matched against the sender
allowlist by the link's name. Nothing is forwarded on to a third daemon. What a
link gives the far operator: every message sent to their sessions, and our
sessions' nicknames, hosts, operators, projects, `current_task` text and
activity. The full table is in [daemon-links-design.md](daemon-links-design.md).

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

- **A kept store holds mail at rest, unencrypted** (MUS-17). With `MUSTER_PERSIST=file`,
  up to 14 days of mail sit in plain text in the state directory, beside the tokens:
  no new attacker reaches it, but a lost disk or a backup now loses mail too. It's
  excluded from Time Machine on macOS; other backup and sync tools aren't handled.
  Mail the sweep deleted stays in the file until the next compaction, at most a day.
  Lines in it are loaded at boot without the gate's checks, so anyone who can write it
  can plant mail, which that account could already do through the local token.
- **The operator's account is one trust zone.** Anything running as the operator
  on the daemon's host can act as any local session and change the controls.
  MUS-39 is the fix.
- **The frame filter parses separately from myko.** That is where the earlier
  bypasses came from. MUS-41 moves authorization into myko.
- **Claimed fields.** A session through the local gate keeps the host name it
  claims, which the per-machine message limit and the sender allowlist trust.
  Both need the local token, so this is part of the first gap.
- **Roles are claims, and a nickname follows its identity (MUS-3).** A session
  can declare a role with `MUSTER_IDENTITY` (`watchman`, say), and mail to that
  name reaches it. Nothing attests the claim: on a machine where every session
  shares one credential, as on a1, any session there can declare any role. So
  the roster shows a role as declared, and a role is never handed to a new
  session while any session holding it is live. The same goes for the
  inferred identity: a new session in a dead session's working directory takes
  over its nickname and its mail. Keys include the attested machine and
  operator, so a paired machine can only take over its own sessions'
  nicknames. Role *addressing* is wider: a send to a role name reaches the
  one session claiming it, on any machine, so a session on a paired machine
  that declares `watchman` takes mail addressed to `watchman` (and two claims
  make the send fail as ambiguous). Nobody addresses a role yet; address the
  watchman by nickname until roles are attested. A wrong
  takeover hands mail to the wrong session, so "dead" is strict (an observed
  client drop with no hook since, or a removal this daemon saw), Codex rows on
  the roster never count as dead until MUS-77, and every takeover leaves a
  `Succession` record saying why. The record is the daemon's to write; the
  gate refuses a client's events on it. Same caveat as Rally's RAL-53.
- **myko is patched locally** (`vendor/myko-server`, MUS-59) until upstream has
  an equivalent hook; the request to myko's maintainers is pending.
- **Dependencies.** CI runs `cargo deny` (MUS-11, `deny.toml`). Its ignored
  advisories are listed there with reasons. Four of the web UI's git
  dependencies declare no licence and are excluded from the check (MUS-61).
