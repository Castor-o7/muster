# What Muster adds to Marshal

Goal G7. This text is meant for the descriptions of the upstream PRs that
bring Muster's changes back to [Marshal](https://github.com/ignition-is-go/marshal).
Each section says what the change adds, why it matters, what it costs, and
what stays out. The per-commit detail is in
[upstream-inventory.md](upstream-inventory.md), the order the PRs go in and
what each overlaps upstream is in [upstream-pr-strategy.md](upstream-pr-strategy.md),
and the delivery mechanics are in [hook-delivery.md](hook-delivery.md).
Updated 2026-11-05 for Muster `3fafe4a` and the port stack on Marshal
`94ecc24`. The section names match the port branches (`port/hooks-v3`,
`port/durability-v2`, `port/other-v3`, `port/gate-v2`).

Muster forked Marshal at `9b71fd6` to run on a small set of the operator's
own machines (Linux and macOS, Claude Code and Codex) and to fix what broke
there. The changes below are the ones that are not specific to that setup.

## Additive features vs conflicts

What each change does to a Marshal user who changes nothing:
- **Additive:** new code that nothing existing calls, or that runs only when
  someone opts in. Their workflow is unchanged.
- **Default change:** what happens without opting in is different. Each one
  is listed, with why it's safe.
- **Conflicts:** where the change edits the same code as upstream's main or
  its open PRs.

The aim, from the operator: **no PR of ours should change how upstream works
today** unless upstream chooses it.

### Live communication (the Hooks layer): the first priority

| Part | Additive? | Effect on a user who changes nothing |
|---|---|---|
| `marshal-shim claude-hook session-start/prompt-submit/post-tool-use/wait` subcommands | additive | none: nothing runs them unless hooks are installed |
| Daemon `/hook/wait` long-poll, `ack=client`, `/hook/ack`, the 60 s lease | additive | none: only the new hooks call them. A caller without `ack=client` keeps today's ack |
| `WakeWatcher` entity and its sweep, the roster's wake field | additive | a new field on the roster, which existing readers ignore |
| SIGTERM "restarting" reply for held waits | additive | none: there are no held waits without the hooks |
| Send result: `wake: expected` also when a waiter is held | additive | only differs for sessions running the waiter |
| `plugins/marshal-shim/hooks/hooks.json`, **enabled in the plugin** | **default change** | every Claude Code user of the plugin gets the hooks: one local HTTP request per tool call (2 s timeout, silent on failure), and one waiter process per idle session that starts a turn when mail arrives |
| macOS: read the parent's command line, so a session without the flag reports `channels_enabled = false` | **default change** (a bug fix) | only for macOS sessions started *without* the flag. Their pushes were dropped by Claude Code and marked read (issue #139); now the mail stays unread. Flagged sessions are unchanged |

**The live-communication PR ships both default changes**, at the operator's
choice: reliable live delivery is the priority. Without the hooks on and the
macOS fix, a Claude Code session started without the flag doesn't reliably get
its mail.

Everything else in the layer is additive, so a user who wants none of it can
turn the hooks off and be back where upstream is today. Turning them off means
disabling the plugin's hooks, or Claude Code's `disableAllHooks`. The PR also
keeps upstream's behaviour where it can: a hook listener that can't bind is
logged, and the daemon keeps running.

The description is in [upstream-pr-2a.md](upstream-pr-2a.md), and the strategy
in [upstream-pr-strategy.md](upstream-pr-strategy.md) ("2. Hooks").

**Conflicts.**
- **Upstream main:** U-RR (the `[response_required=…]` inbox line) and U-CG
  (shared guidance text) touch the same `hooks.rs` and `shim/src/main.rs`
  lines. The catch-up merge (#84) already resolved them, and the port reuses
  that resolution.
- **Upstream's open PRs:**
  - our own #154, which this supersedes;
  - lobo235's #141, which pulls the inbox into Claude turns with plugin hooks
    and escapes bodies: the same goal and files;
  - #140, which keeps a Claude push unread until written.

### The other layers

| Layer | Additive | Default changes | Conflicts with upstream |
|---|---|---|---|
| Inbox quoting | — | peer bodies in the hook inbox are quoted as untrusted text, so what an agent reads changes. Only bodies, not the line format | #141 escapes the same bodies; U-CG's `PEER_MESSAGE_NOTICE` is built on, not replaced |
| Durability | `idempotency_key` on send (optional field); read state for senders | mail to a reaped session is kept until the 14-day sweep instead of going at restart; a waiting session is kept through a dropped connection | lucid-max's #148 and lobo235's #131, in the same `cleanup.rs`, `message.rs` and `message_read.rs`; U-RR on `send_message.rs` |
| Other | `marshal-shim send` and `roster`; the `cargo-deny` audit; the AGPL notice | an empty `to` is refused instead of delivered to the sender; a first write waits up to 5 s for registration instead of failing | #135 (`--help`/`--version`) and #143 (room validation) near the same code |
| Gate | **everything, while `--tls` isn't given**: plain is the default, and Marshal runs as today | none in plain mode. With paired machines and neither flag, the daemon refuses to start, which only affects someone who has paired machines | #135 in `daemon/src/main.rs`'s argument handling; the vendored myko-server against ignition-is-go/myko |
| Later: links, the conformance suite, Codex bridge fixes | links (TLS-only) and the suite (test-only) are additive; the bridge's pause check (MUS-74) acts only while the operator has paused wake-ups | Codex threads re-register after a restart (MUS-76), so an idle Codex session reappears on the roster sooner | lobo235's plugin PRs (#132, #134, #136, #137) for MUS-81 |

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

Around the hooks:

- Mail is acked only once the client has handed it on. The waiter and the
  prompt hooks ask for `ack=client`, get the ids in `X-Muster-Ack`, print the
  mail and only then POST `/hook/ack`. A 60 s lease keeps a second reader from
  taking the same message, and mail whose reply is lost is offered again when
  the lease lapses (MUS-75).
- The roster says whether a message would wake a session (a `WakeWatcher`,
  held in memory only), and a send result says `wake: expected` only when a
  live waiter can wake the recipient (MUS-71, still to be forward-ported).
- On SIGTERM, held waits end with a "restarting" reply, so a waiter re-arms
  instead of reporting an error.

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

**Overlaps upstream.** It supersedes our own open #154, an earlier cut. Its
aim and files overlap lobo235's open #141, which pulls the inbox into Claude
turns with plugin hooks, and #140, which keeps a Claude push unread until it's
written. See [upstream-pr-strategy.md](upstream-pr-strategy.md).

## Durability: mail and sessions that survive what goes wrong

**What it adds.**

- A timed-out send can be retried with an `idempotency_key` and is not
  delivered twice.
- A session whose connection drops is kept while it is still waiting.
- Mail to a reaped session is kept until the 14-day sweep, not deleted at
  restart.
- A sender can see whether its mail was read before the recipient went away.

Muster also has a push-side ack (MUS-81): a session that says it acks its own
pushes (`acks_pushes`) has a push marked read only once the harness has taken
it. Whether it goes with this PR is still to decide, since it overlaps
lobo235's plugin PRs (#132, #134, #136, #137).

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

**Overlaps upstream.** lucid-max's own open #148 (messages and receipts kept
across a session resume) and lobo235's #131 (read-state kept across a reload)
change the same `cleanup.rs`, `message.rs` and `message_read.rs`. If either
lands first, this PR is rebased onto it and says which cases it already covers.

## Gate: optional TLS and per-machine pairing

**What it adds.**

- A TLS listener in front of the daemon, with a pinned self-signed
  certificate.
- Pairing a machine with a one-time code that expires.
- A per-machine token.
- A frame filter that lets a paired machine act only as its own sessions,
  including on the hook listener.
- Attested host and operator for paired machines.
- A local token on the hook listener and for local tools. Local tools reach
  the daemon on the gate's own port (the first byte tells TLS from a local
  plain connection, and plain from another machine gets 426), so there's no
  separate internal listener (MUS-70).
- Myko's own listener admits only forwarded requests.
- A tokens file that can't be read stops the daemon instead of reading as "no
  pairings" (MUS-85).
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
- One more loopback listener: myko's own, proposed as 6158.
- A pairing step for each extra machine.
- A local token that every local tool must present.
- A vendored copy of myko-server 6.11.5 (AGPL), changed only by a
  `RequestFilter`. That's about 12,200 lines upstream would carry. It should
  become a request-filter hook in ignition-is-go/myko before the gate lands.

The gate is **opt-in**, and with it off Marshal behaves as it does today. The
operator's choice for upstream (2026-11-05):
- `marshal-daemon --tls` turns the gate on.
- `--plain` runs plain even though machines are paired.
- With pairings and neither flag, the daemon refuses to start, naming both.
- Both flags together are refused.
- There's no environment variable.

Muster also keeps `MUSTER_TLS`, which the upstream PR leaves out.

**What stays out.** Muster's port numbers (6255–6258) and the Murph firewall
notes.

**Overlaps upstream.** lobo235's open #135 (`--help`/`--version`) rewrites the
same argument handling in `daemon/src/main.rs`. Whichever lands second adopts
the other's parser.

## Other

**What it adds.**

- `marshal-shim send` to message the muster from a terminal. The text says a
  terminal message is never the operator's approval.
- A read-only `marshal-shim roster` for scripts.
- Sessions are removed from their old operator rooms when the operator
  changes.
- The sweeper logs why it reaped a session.
- Notices of AGPL dependencies, a bump past a yanked `chacha20`, and a
  `cargo-deny` audit in CI.
- `send_message` refuses an empty recipient instead of delivering to the
  sender (MUS-73).
- A session's first writes wait until it's registered, re-send the session
  if it hasn't landed in 1 s, and retry once a write refused as unregistered
  (MUS-72, MUS-78, MUS-79). Before this, an agent's first tool call could fail
  "has no session on the roster".

**Why it matters.** These are small fixes and tools that came up in daily
use. None depends on the buckets above.

**What it costs.** Two subcommands and a few lines each. A first write can
wait up to 5 s for registration.

**Overlaps upstream.** lobo235's #143 (room-name validation) works next to the
operator-room fix in `cleanup.rs` and the room tools.

## Later, if upstream wants them

None of these is in a port branch yet. Each would follow the Gate.

- **Messaging between daemons (MUS-66).** Two TLS daemons link with a
  one-time code, and their sessions message each other as `nickname@link`,
  with an outbox that survives a restart. It needs the gate on both sides.
- **The delivery conformance suite (MUS-69).** It runs the real daemon and shim
  against a fake of each harness (Claude Code, Codex, opencode, Pi, Prime
  Agent) and prints a harness × scenario grid. It's test-only, and could
  check upstream's plugins in CI.
- **Codex bridge fixes.** The bridge respects the operator's pause (MUS-74),
  re-registers its threads after a daemon restart (MUS-76), shows its wake
  path on the roster (MUS-77), and goes through the gate (MUS-18).
- **The local-token refusal message (MUS-83).** A shim whose token the daemon
  refuses says so and names the file, instead of timing out.

## What Muster drops entirely

These stay in Muster and are not offered upstream:

- The rename to Muster: crate, binary, plugin and environment-variable
  names, and ports 6255–6258.
- The board, roadmap, Rally and watchman process docs, and the team notes.
- Muster's README, banner and crates.io release.
- The CI changes that removed Marshal's release pipeline.
- The Murph deployment scripts (`rollout.sh`, `restart-preview.sh`).

The full list, entry by entry, is in [upstream-inventory.md](upstream-inventory.md).
