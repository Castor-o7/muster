# Design: messaging between daemons (G5, MUS-66)

Status: **built** (MUS-66, `crates/daemon/src/links.rs`; tests in
`scripts/test-links-e2e.sh` and `crates/daemon/tests/remote_send.rs`).
Approved by the watchman (grand-photon), 2026-11-04, with the
changes folded in below (message ids, `deferred`, `nickname@link`, version
mismatch) and the answers under "Decisions". Written by modest-roach, 2026-11-04, against Muster main at
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
muster-daemon unlink murph      # forgets it; on Murph, `muster-daemon unlink a1` revokes it (MUS-84)
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
operator asks upstream which it wants. (For TLS itself the operator chose the
`--tls` flag, with `--plain` as the escape, on 2026-11-05.)

## The link protocol

Text frames, JSON, one object each, at most 128 KiB. A larger frame closes the
link.

| Frame | Sent by | Meaning |
|---|---|---|
| `hello {name, version}` | both, first | who this is; the protocol version (1) |
| `roster {sessions: [...]}` | both | a full snapshot of this daemon's **local** sessions, sent after `hello` and again on change (at most once a second) |
| `deliver {id, from, to, body, response_required, sent_at}` | both | a direct message from one of the sender's local sessions to one of the receiver's local sessions |
| `stored {id}` | both | the `deliver` with this id is stored (or was already) |
| `deferred {id, retry_after}` | both | not now: a rate limit was hit; send it again after `retry_after` seconds |
| `refused {id, why}` | both | never: unknown recipient, a forged `from`, a body too large, or a `to` that isn't local |

`id` in these frames is always the **sender's** message id. `deferred` is
retryable, so a long outbox flush after an outage that runs into the receiver's
rate limits slows down rather than losing mail. `refused` is permanent.

**Versions.** `hello` carries the protocol version. On a mismatch the side that
notices logs both versions and closes the link. There is no partial protocol
and no downgrade. The dialer keeps retrying with backoff, so the link comes back
once both sides run the same version.

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
  then remote ones (by id, prefix or nickname): **a bare name prefers local.**
  `nickname@link` (e.g. `fuzzy-otter@murph`) names a remote session
  explicitly, so a sender can still reach it if a local session later gets that
  nickname. A remote recipient makes the
  `Message` carry `via_link: Some(link)` and `forwarded_at: None`. The sender's
  result says `delivery: "remote"` (or `"queued_remote"` while the link is
  down), so the sending agent knows it isn't a local inbox.
- **Remote mail gets a local id.** A `deliver` is stored under a fresh local
  message id, never the sender's: Muster's idempotent sends derive ids, so
  storing a peer's id would let a malicious link send `deliver {id: X}` first
  and squat an id a local send will use later. The receiver keeps
  `(link, remote_id)` beside the message for dedup, and answers `stored` with
  the remote id.
- **Outbox.** The link task sends every `Message` with its `via_link` and
  `forwarded_at: None`, in `sent_at` order, and sets `forwarded_at` on
  `stored`. On `deferred` it waits `retry_after` and resends that message
  before later ones, so order holds. With Postgres, the outbox survives a
  restart, because it's just messages. `refused` turns into a notice to the sender, as MUS-48 does for
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
  Dedup is by `(link, remote_id)`, never by local id.
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
  hours down (`LINK_GIVE_UP`, a constant, with no variable), the rows are dropped, queued messages are refused back to their
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
| Squat a local message id | Remote mail is stored under a fresh local id; the peer's id is kept only as `(link, remote_id)` for dedup. |
| Flood | Each link gets the machine-level rate limit (MUS-60) as one machine named after the link, and each remote sender gets the session limit (MUS-9). One `/link` connection per link: a new one replaces the old. Roster at most 512 sessions, frames at most 128 KiB, bodies at most 64 KiB, as today. |
| Pass the sender allowlist | The allowlist (MUS-10) matches a remote sender by its **link name**, not its self-reported host. `allow murph` admits the link; a remote session calling itself `a1` doesn't become `a1`. |
| Drown the receiver in an outbox flush | Rate-limited mail gets `deferred` with `retry_after`, and the sender slows down. Nothing is dropped, and nothing gets past the limit. |
| Spoof host or operator on its sessions | Shown as reported, but always beside `via <link>`, and never used for bindings, the allowlist or operator routing. |
| Replay a captured `deliver` | TLS stops capture on the wire. A replay from the peer itself is deduplicated by id. |
| Keep a revoked link open | `unpair` on the listener ends the open link within 5 s (the gate's existing revocation check), and `unlink` on the dialer closes it. |
| Prompt injection in bodies | Unchanged: bodies are untrusted peer text, quoted as today. A peer is not the operator, and that includes a peer on another daemon. |

What a link does **not** protect: the far daemon's operator can read every
message sent to their sessions, and see our sessions' nicknames, hosts,
projects, `current_task` text and activity. `current_task` is free text an agent
writes, so it can name work in progress. Linking is trusting that operator with
all of that. Today both daemons are the same operator's.

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

## Decisions (watchman, 2026-11-04)

1. **One dialer, both directions.**
2. **`current_task` is shared** across a link, and the threat model says so.
3. **Operator-addressed messages across a link** stay out of v1. Operator
   routing has its own open issue.
4. **24 hours** before a down link gives up: the constant `LINK_GIVE_UP`.

Changes asked for and folded in above: remote mail stored under a fresh local
id; `deferred` for retryable rate-limit answers; `nickname@link` addressing;
closing on a `hello` version mismatch.

## As built

Where the build differs from, or adds to, the text above:

- **Link tokens** live in `tokens` as `link:<name>`. Machine names can't hold
  `:`, so neither kind can stand in for the other. The gate sends a link token
  only to `/link`, and refuses a machine token there.
- **The local id** of inbound mail is a UUIDv5 of `(link, remote_id)` in a
  namespace of its own. It's fresh (the peer doesn't choose it, and it can't
  equal an id a local send derives), and it makes dedup a lookup by id.
  `MessageLink` keeps the remote id beside it.
- **Outbox and labels** are a separate `MessageLink` entity keyed by message id,
  so `Message` itself is unchanged. `RemoteSession` isn't persisted;
  `MessageLink` is.
- **A recipient grace:** for 60 s after a link comes up, mail for a session this
  daemon doesn't know yet gets `deferred`, not `refused`, because sessions take
  a moment to register again after a restart. After that, an unknown recipient
  is refused, as designed.
- **The give-up** is `LINK_GIVE_UP` (24 h): queued mail older than that, for a
  link that's down, is refused back to its sender as a message labelled from
  the far session; remote rows unseen that long are dropped.
- **Answers are scoped to their link and connection.** `stored`, `deferred` and
  `refused` act only on an outbound entry queued for the answering link and
  sent on this connection; anything else is logged and ignored. Otherwise one
  link could drop, refuse, or (by `deferred`) redirect mail queued for another.
  Found by the watchman in review.
- **Remote senders are stored as `sid@link`**, never their bare id, so their
  mail can't pass for a local session's, even if a local session later takes
  that bare id. Found by the watchman in review.
- **Not in v1:** `muster-shim tell` resolves names on the roster itself, so it
  doesn't take `nickname@link` yet. `send_message` does.

