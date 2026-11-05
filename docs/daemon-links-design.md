# Design: messaging between daemons (G5, MUS-66)

Status: **draft for the watchman (grand-photon).** Nothing is built until this
is approved. Written by modest-roach, 2026-11-04, against Muster main at
`d9c35d3` with the optional TLS gate from #89 (MUS-64). Names are Muster's;
`scripts/rename-to-marshal.sh` maps them for an eventual upstream PR.

## What it's for

Today every session talks to one daemon. a1's sessions reach Murph's daemon by
pairing a1 as a *machine*. A **link** joins two daemons instead: each keeps its
own roster, store and operator controls, and a session on one can message a
session on the other, both ways. The far side's sessions show up on the roster,
marked with the link they came over.

Out of scope for the first version: rooms and broadcasts across a link,
messages to an operator's email across a link, and forwarding further than one
hop (A–B–C does **not** let A reach C).

## Shape

```
 a1 daemon (dialer)                               Murph daemon (listener)
 ┌──────────────────┐   one TLS WebSocket, /link   ┌──────────────────┐
 │ link task        │ ───────────────────────────► │ gate → link task │
 │ outbox, roster   │ ◄─────────────────────────── │ outbox, roster   │
 └──────────────────┘   frames go both ways        └──────────────────┘
```

- One side **dials**, the other **listens**. The dialer opens a WebSocket to
  the listener's gate at a new route, `/link`, over TLS with the listener's
  certificate pinned, presenting a **link token**. Both directions ride that one
  connection, so only the listener has to be reachable. That fits a1 and Murph:
  a1 can reach Murph's gate, and Murph can't count on reaching a1.
- The link speaks a small JSON protocol of its own (below), **not** myko's
  frames. A linked daemon can only do what those few frames allow. It can never
  run myko commands on the other side.
- The receiving daemon stores what arrives as an ordinary `Message`, so
  delivery, wake-ups, pause, acks and the hooks work unchanged on that side.

## (1) What links two daemons

No new ports and no new environment variables. A link reuses pairing:

On the listener (Murph):

```bash
muster-daemon pair --name a1 --link [--address murph.example:6255] [--expires 15m]
```

This prints `muster-daemon link <code>`. The code is the same one-time,
expiring invite as a machine pairing (MUS-8), marked `kind: link`. On the
dialer (a1):

```bash
muster-daemon link <code>       # redeems the invite at Murph's gate, stores the link
muster-daemon links             # lists links and whether each is up
muster-daemon unlink murph      # forgets it; on Murph, `muster-daemon unpair a1` revokes it
```

- **Peer address and certificate:** carried in the code, exactly as for a
  machine: the gate's `host:port` and the SHA-256 fingerprint of its
  certificate, which the dialer pins.
- **Peer token:** what redeeming the invite returns. The dialer stores the
  address, fingerprint and token owner-only in its state dir (`links`), one line
  per link, as the shim stores `pairing`. The listener keeps only the token's
  hash, in `tokens` like a machine's, with the kind recorded beside it.
- **Token kinds are separate.** A link token opens only `/link`. A machine
  token can't open `/link`, and a link token can't open `/myko` or `/hook/*`.
  Stealing one doesn't buy the other.
- **Both sides' names:** the listener names the link at `pair` time (`a1`). The
  dialer learns the listener's name from the code (`murph`). That name labels
  the far side's sessions on its roster.
- The running dialer picks up `link` and `unlink` within a few seconds, the same
  way the controls are picked up (MUS-10). Neither daemon needs a restart.

**For upstream, which has no CLI.** Offer both: (a) the `link`, `links` and
`unlink` subcommands, landing with the Gate PR's `pair` family, or (b) a
`MARSHAL_LINKS` variable naming a file of link codes, read at start. The
operator asks upstream which it wants, together with `--tls` against
`MARSHAL_TLS`.

## The link protocol

Text frames, JSON, one object each, at most 128 KiB. A larger frame closes the
link.

| Frame | Sent by | Meaning |
|---|---|---|
| `hello {name, version}` | both, first | who this is; the protocol version (1) |
| `roster {sessions: [...]}` | both | a full snapshot of this daemon's **local** sessions, sent after `hello` and again on change (at most once a second) |
| `deliver {id, from, to, body, response_required, sent_at}` | both | a direct message from one of the sender's local sessions to one of the receiver's local sessions |
| `stored {id}` / `refused {id, why}` | both | the answer to one `deliver` |

A roster entry carries what peers need to choose a recipient and nothing more:
session id, nickname, host name, operator, project, `current_task`, activity,
`wake` and last activity. It carries **no cwd, pid or git branch**. Those stay on
the daemon that owns the session.

## Where things live

- **`RemoteSession`**, a new entity separate from `Session`: `{id, link, nickname, host,
  operator, project, current_task, activity, wake, last_activity_at, link_up}`.
  It's rebuilt from each `roster` frame and never persisted (blackholed like
  `Client`). Keeping it out of `Session` means the frame filter, session
  bindings, the sweeper and the roster's own bookkeeping never mistake a far
  session for a local one.
- **Resolving a recipient.** `SendMessage`'s resolver tries local sessions first,
  then remote ones (by id, prefix or nickname). A remote recipient makes the
  `Message` carry `via_link: Some(link)` and `forwarded_at: None`. The sender's
  result says `delivery: "remote"` (or `"queued_remote"` while the link is
  down), so the sending agent knows it isn't a local inbox.
- **Outbox.** The link task sends every `Message` with its `via_link` and
  `forwarded_at: None`, in `sent_at` order, and sets `forwarded_at` on
  `stored`. With Postgres, the outbox survives a restart, because it's just
  messages. `refused` turns into a notice to the sender, as MUS-48 does for
  mail unread at reap.
- **Roster for agents.** `muster://roster` and `muster-shim roster` list remote
  sessions after local ones, each with `"via": "murph"` and `"link_up"`. Inbox
  lines name the sender as `fuzzy-otter (via murph)`.

## (2) Does a test session need its own pairing file?

**No.** A session only ever talks to its own daemon. A session on the daemon's
host uses the local token, not a pairing. A session for a second daemon on the
same host needs:

- `XDG_CONFIG_HOME` pointing at that daemon's config dir, so the shim reads
  *that* daemon's local token, and with no `pairing` file in it;
- `MUSTER_DAEMON_ADDRESS=ws://127.0.0.1:6265` and
  `MUSTER_BASE_URL=http://127.0.0.1:6266`, so the shim and the global
  `claude-hook` commands reach it rather than the main daemon.

**Lamplighter's test on Murph.** Run two throwaway daemons, neither of them the
production one: X on 6265–6268 (`MUSTER_BIND`, `MUSTER_HOOK_BIND`,
`MUSTER_INTERNAL_BIND`, `MUSTER_MYKO_BIND`) and Y on 6275–6278, each with its own
`MUSTER_STATE_DIR`, `XDG_CONFIG_HOME` and Postgres database, both with
`MUSTER_TLS=1`. Pair Y as a link on X, link Y to X, and start one session on
each. The production daemon is untouched. The **real a1↔Murph run** comes
after: a daemon on a1 dials Murph's production daemon. That means a `pair
--link` on Murph and a Murph daemon running this code, so it needs a restart,
which is the operator's word through Lamplighter.

## (3) Restarts and messages in flight

Delivery is **at least once, deduplicated by message id**:

- **Either side restarts.** The link drops. The dialer reconnects with backoff
  (1 s doubling to 60 s), and both sides exchange `hello` and full `roster`
  snapshots. Then each side resends every outbox message that never got
  `stored`, in order. The receiver drops a `deliver` whose id it already holds
  and answers `stored`, so a resend after a lost `stored` isn't duplicated.
- **The receiver had stored it but crashed before answering:** it's resent,
  found by id and acknowledged. With Postgres the id survives the crash.
  Without it the first copy died with the daemon, so storing the resend is
  right.
- **The sender restarts with messages still in its outbox:** with Postgres
  they're kept and sent when the link is back. Without Postgres they're lost,
  as all its mail is. The send result says `queued_remote` while the link is
  down, so the sending agent isn't told "delivered".
- **While the link is down,** remote sessions stay on the roster with
  `link_up: false` and their last-seen time, and sends to them queue. After 24
  hours down, the rows are dropped, queued messages are refused back to their
  senders as a notice, and sends answer "no session" again.
- **A recipient reaped on the far side** gets the existing MUS-46 and MUS-48
  treatment there. The sender's daemon only learns `stored`.
- A held `/hook/wait` on the receiving side is woken by the new `Message` as
  usual. Pause there holds the wake-up, not the delivery.

## (4) TLS only

Yes, on **both** daemons. `link` refuses unless the shell or the unit turns TLS
on, and a plain daemon has no gate, so it has no `/link` route. A dialer whose
own daemon runs plain refuses to start the link: its sessions would be
unattested (`asSession` is trust-on-assert in plain mode), so a link would let
anything on its host speak as any of its sessions to the far side.

## Threat model

New in `docs/threat-model.md` when the code lands. A link adds one attacker:
**a compromised or malicious linked daemon**, or a stolen link token.

| It might try | Answer |
|---|---|
| Send as one of *our* sessions, or as a third daemon's | The receiver accepts a `deliver` only if `from` is in that link's **own last roster** and isn't a local session id or another link's. A `roster` naming an id that's local, or held by another link, is refused and logged. Remote rows are keyed by `(link, id)`. |
| Read mail that isn't for it | It receives only `deliver` frames for sessions it advertised. Nothing else crosses: no myko frames and no subscriptions. |
| Use the link as a relay to a third daemon | No transitive forwarding. A daemon advertises only local sessions, and a `deliver` whose `to` is remote is refused. |
| Flood | Each link gets the machine-level rate limit (MUS-60) as one machine named after the link, and each remote sender gets the session limit (MUS-9). One `/link` connection per link: a new one replaces the old. Roster at most 512 sessions, frames at most 128 KiB, bodies at most 64 KiB, as today. |
| Pass the sender allowlist | The allowlist (MUS-10) matches a remote sender by its **link name**, not its self-reported host. `allow murph` admits the link; a remote session calling itself `a1` doesn't become `a1`. |
| Spoof host or operator on its sessions | Shown as reported, but always beside `via <link>`, and never used for bindings, the allowlist or operator routing. |
| Replay a captured `deliver` | TLS stops capture on the wire. A replay from the peer itself is deduplicated by id. |
| Keep a revoked link open | `unpair` on the listener ends the open link within 5 s (the gate's existing revocation check), and `unlink` on the dialer closes it. |
| Prompt injection in bodies | Unchanged: bodies are untrusted peer text, quoted as today. A peer is not the operator, and that includes a peer on another daemon. |

What a link does **not** protect: the far daemon's operator can read every
message sent to their sessions, and see our sessions' nicknames, hosts,
projects, tasks and activity. Linking is trusting that operator with that much.

## Tests (the "done when")

- Unit: frame parsing and size caps; the `from` and id-collision checks; token
  kinds kept apart at the gate; outbox ordering and dedup.
- `scripts/test-gate-e2e.sh`, a new links section with two TLS daemons on
  loopback: link by code (one-time, expiring); send both ways and see each wait
  wake; roster rows `via` the link; kill the listener mid-flight and restart it;
  queued mail arrives once; a forged `from`, a local-id collision and a
  machine token on `/link` are each refused; `unpair` closes the link; a plain
  daemon refuses `link`.
- A Zoo run (G8): two agents on two linked daemons coordinating a handoff.
- The real a1↔Murph run, with the operator's OK for Murph's part.

## Questions for the watchman

1. **One dialer, both directions** (above), or both sides dialing? One is simpler
   and fits NAT. Its cost is that only the dialer can re-establish the link.
2. **Roster fields across the link:** no cwd, pid or branch (above). Is
   `current_task` acceptable to share?
3. **Operator-addressed messages across a link** (to `joshd@…` reaching the
   operator's most active agent on either daemon): left out of v1. Do you want
   it in?
4. **24 hours** before a down link's rows and queue give up: is that right?
