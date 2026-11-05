# What Muster adds to Marshal

Goal G7. This text is meant for the descriptions of the upstream PRs that
bring Muster's changes back to [Marshal](https://github.com/ignition-is-go/marshal).
Each section says what the change adds, why it matters, what it costs, and
what stays out. The per-commit detail is in
[upstream-inventory.md](upstream-inventory.md). The delivery mechanics are in
[hook-delivery.md](hook-delivery.md).

Muster forked Marshal at `9b71fd6` to run on a small set of the operator's
own machines (Linux and macOS, Claude Code and Codex) and to fix what broke
there. The changes below are the ones that are not specific to that setup.

## Hooks: delivery without the channels flag

**What it adds.** Claude Code receives direct messages, and an idle session
wakes for them, without `--dangerously-load-development-channels`. The plugin
ships `hooks/hooks.json`:

- `SessionStart`, `UserPromptSubmit` and `PostToolUse` pull unread mail into
  the turn.
- A `wait` hook on `SessionStart` and `Stop`, run with `asyncRewake`, holds a
  `/hook/wait` long-poll and exits 2 when mail arrives, which starts a turn.

On macOS the shim also reads its parent's command line, so a session without
the flag reports `channels_enabled = false` and the daemon stops pushing to it.

**Why it matters.** Without the flag, Marshal mail to Claude Code is either
lost or never fetched. On macOS the daemon pushed a notification Claude
dropped, then marked the message read. Everywhere, no hook pulled the inbox
(issue #139). The flag is a development option that each launch has to
remember. Hooks work with a plain `claude`.

**What it costs.**

- One local HTTP request after every tool call (2 s timeout, silent on
  failure).
- One waiter process per idle session.
- One 500 ms inbox check per waiter in the daemon.
- Possible duplicates: delivery is at-least-once, so a lost ack shows a
  message again.

The push path is unchanged. A flagged session still gets pushes, and its
hooks then find nothing unread.

**What stays out.** Muster's board and roadmap notes about the feature, and
the `muster-*` names.

## Durability: mail and sessions that survive what goes wrong

**What it adds.**

- Mail is acked only once the client has handed it on, with a short lease so
  two readers don't take the same message.
- A timed-out send can be retried with an `idempotency_key` and is not
  delivered twice.
- A session whose connection drops is kept while it is still waiting.
- Mail to a reaped session is kept until the 14-day sweep, not deleted at
  restart.
- A sender can see whether its mail was read before the recipient went away.
- On SIGTERM, held waits end with a "restarting" signal.
- Wake watchers are in-memory only.

**Why it matters.** Each one fixes mail that was lost or doubled in real use:
a daemon restart, a dropped connection, a reply lost in transit, a session
that ended with unread mail. `docs/agent-disappearance.md` records the
cases.

**What it costs.**

- Messages can arrive twice instead of not at all.
- A send takes an optional extra field.
- Unread mail for ended sessions stays on disk for up to 14 days.
- Read records gain one more query.

**What stays out.** The Murph restart and rollout scripts.

## Gate: optional TLS and per-machine pairing

**What it adds.**

- A TLS listener in front of the daemon, with a pinned self-signed
  certificate.
- Pairing a machine with a one-time code that expires.
- A per-machine token.
- A frame filter that lets a paired machine act only as its own sessions,
  including on the hook listener.
- Attested host and operator for paired machines.
- A local token on the hook listener, and a local gate in front of myko.
- Myko's own listener admits only forwarded requests.
- Message-rate and connection limits.
- An operator switch to pause wake-ups and limit which machines may send.
- `docs/threat-model.md` and a `cargo-deny` audit in CI.

**Why it matters.**

- Marshal's WebSocket accepts any client that can reach it, and any client
  can act as any session.
- On a LAN, any device can send mail that wakes and steers an agent.
- Any web page open in a browser on the host can reach `127.0.0.1`.

The gate closes those holes for the four attackers in the threat model: a
malicious web page, one compromised session, one stolen paired machine, and
other devices on the LAN.

**What it costs.**

- New dependencies: `rustls`, `tokio-rustls`, `tokio-tungstenite`, `sha2`,
  `ciborium`, `socket2`.
- Two more loopback listeners.
- A pairing step for each extra machine.
- A local token that every local tool must present.
- A patched copy of myko-server, which is AGPL and pinned to 6.9.0 while
  upstream is on 6.11.5. This should become a request-filter hook in myko
  itself before the gate lands.

The gate must be **opt-in**. With it off, Marshal should behave as it does
today. The design of that switch is G3.

**What stays out.** Muster's port numbers, which become proposed 6157/6158
for the new listeners, and the Murph firewall notes.

## Other

**What it adds.**

- `marshal-shim send` to message the muster from a terminal. The text says a
  terminal message is never the operator's approval.
- A read-only `marshal-shim roster` for scripts.
- Sessions are removed from their old operator rooms when the operator
  changes.
- The sweeper logs why it reaped a session.
- Free-text room names stay out of mention banners.
- Notices of AGPL dependencies, and a bump past a yanked `chacha20`.

**Why it matters.** These are small fixes and tools that came up in daily
use. None depends on the buckets above.

**What it costs.** Two subcommands and a few lines each.

## What Muster drops entirely

These stay in Muster and are not offered upstream:

- The rename to Muster: crate, binary, plugin and environment-variable
  names, and ports 6255–6258.
- The board, roadmap, Rally and watchman process docs, and the team notes.
- Muster's README, banner and crates.io release.
- The CI changes that removed Marshal's release pipeline.
- The Murph deployment scripts (`rollout.sh`, `restart-preview.sh`).

The full list, entry by entry, is in [upstream-inventory.md](upstream-inventory.md).
