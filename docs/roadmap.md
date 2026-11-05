# Muster roadmap

Goal: Claude sessions on different machines message each other live, including
waking idle sessions, without `--dangerously-load-development-channels`, and
Muster is secure and portable enough to run unattended.

Muster is how agents securely reach and coordinate with each other. Tracking the
progress of their work belongs to [Rally](https://github.com/Castor-o7/rally), a separate
project (see Decisions).

Stories follow the process in [board.md](board.md). IDs are permanent.
"Needs a run" means `gh` can't show the result, so the submission must include
evidence from a real run by the person or team named.

## Status

2026-11-04: 34 of 68 stories ticked, about 50%, not counting the 3 moved to Rally. Live on
Murph since 2026-11-04 16:02:18: `14e2d1a`, deployed by Lamplighter on the operator's word, with
every port check passing. It carries MUS-7 (first live at `3746864`, 14:28) and MUS-59, besides
everything before. Merged and waiting for the next daemon restart: MUS-8 (which also needs a
rebuilt shim wherever a machine next pairs), MUS-9, MUS-10, MUS-60, and MUS-11's rustls
and tokio security updates. Sessions pick up a rebuilt shim
when they restart, and no keyed send (MUS-29) has been seen live yet. Verified live on 2026-11-04:
MUS-4, MUS-7, MUS-34, MUS-35, MUS-54 and MUS-57. Merged, live and not yet verified: MUS-46,
MUS-48, MUS-50 (its live half needs a session that loses its binding), MUS-55 and MUS-59.
Merged and needing a live run: MUS-36 and MUS-37.

## Decisions

- **Muster and Rally are separate.** Decided by the operator on 2026-11-02. Muster carries
  messages and wake-ups, and owns identity, the roster and whether a session can be woken.
  Rally owns claims, work states, evidence, check-ins and history. Rally depends on Muster,
  and Muster never depends on Rally, so Muster works the same with Rally down. Muster has no
  concept of a story or a stall.

- **Threat model.** Muster defends against a malicious website, and against one
  compromised session or paired machine; see [threat-model.md](threat-model.md). All machines belong to one operator.
  Other people with accounts on those machines are the next step, not yet in
  scope.
- **Networks beyond the LAN.** Use Tailscale or WireGuard. The gate is not
  exposed to the open internet.
- **Platforms.** macOS and Linux are fully supported. Windows is best effort,
  for the shim only.
- **Language.** New Muster code is Rust. No new TypeScript. The Pi, opencode and
  Prime Agent plugins are TypeScript inherited from Marshal, because they run inside
  those TypeScript hosts; they get no new features for now.
- **Architecture.** Muster stays a myko service wherever possible: Muster's own new state
  is myko entities and events in the daemon, not a separate system. This doesn't bind
  Rally.
- **Ordering.** Anything that lets a Claude act without the operator waits until
  Phase 2 is done.

## Phase 1: Reliable delivery

- [ ] **MUS-1: Fix watcher expiry.** Monitor stops at 30 minutes; the expiry
  notice wakes an idle session, so re-arming works but costs about 48 wake-ups
  per session per day. The `asyncRewake` hook `muster-shim claude-hook wait` avoids
  that: it costs no model turns while idle, and Stop restarts it after every turn. Done when: a session idle overnight still wakes on a
  message, and an idle session uses at most 2 model turns per hour, with 0 as the
  target. Needs a run: the Muster team, overnight.
- [x] **MUS-2: Close the first-prompt gap.** The same hook runs on SessionStart.
  Done when: a restarted session wakes on a message before anyone has typed to it.
  Needs a run: the Muster team, on both machines.
- [ ] **MUS-3: Stable identity across restarts.** Done when: a session keeps its
  nickname through a restart, and the watchman can be reached by role.
- [x] **MUS-4: Show dead watchers.** Done when: the roster distinguishes a
  session that can be woken from one that can't.
  Verified live 2026-11-04 by gentle-pulsar against Murph's `e797fa3`: `muster-shim roster` showed
  `waiting: true` with a `waiting_until` for idle sessions, `waiting: false` for busy ones, and
  `bound_to_connection: false` for one whose process had exited. Only the daemon creates
  watchers (`2faeaa8`), so this is the daemon's view.
- [x] **MUS-25: Reach the daemon when `localhost` resolves to `::1` first.** Found
  during live-delivery testing. Done when: an unpaired Linux host's watcher wakes
  on a message with no explicit URL. Evidence: `090b2a9`, tested on Murph by
  crimson-avocet.
- [x] **MUS-27: Deliver long messages whole through the watcher.** Monitor cuts
  a line at about 500 characters and an event at about 3,000. Done when: a message
  up to the daemon's 2,000-character limit reaches an idle session complete. Needs
  a run: the Muster team, with a peer sending.
- [x] **MUS-34: Don't lose mail between delivery and ack.** From the 2026-10-30 review.
  A message is marked read once it reaches the gate's loopback socket, so a silent network
  drop loses it. A waiter whose session just closed can also take mail and drop it, a plain
  `watch` has no parent check, and a resumed session can lose its wake to the old waiter.
  Concurrent readers can deliver a message twice. Done when: delivery is acknowledged by
  the client, and a test covers a dropped path.
  Verified live 2026-11-04 by gentle-pulsar on `e797fa3`: direct messages the Stop hook delivered
  show as read on the daemon, and room messages read only through `muster://messages` stay unread,
  so the ack follows delivery. The dropped path is covered by
  `held_wait_returns_a_message_sent_during_it_and_acks_it` and
  `abandoned_wait_does_not_ack_a_later_message` in `crates/daemon/tests/hook_wait_e2e.rs`.
- [x] **MUS-35: Say why a watcher can't reach the daemon.** A 401 or 403 and a real outage
  print the same "can't reach" line, and a daemon whose hook port failed to bind keeps
  running with hooks dead. Done when: the message names the cause, and a failed hook bind
  stops the daemon or is plainly reported.
  Verified 2026-11-04 by gentle-pulsar: a1's `refused-<session>` files from 23:03 on 2026-11-03
  read "the daemon refused this machine's token (401); it may need pairing again", the notice those
  sessions got; `muster-shim watch` against a dead port printed "can't reach the daemon (down, or no
  network); retrying"; and a `muster-daemon` built at `61ee4c1` with its hook port already taken
  exited 1 with "hook listener on 127.0.0.1:46256: Address already in use".
- [ ] **MUS-50: Repair a roster row that has lost its client binding.** Found 2026-11-02: a
  session listed on the roster with `pid` 0 and no `client_id` got "no session on the roster"
  on every send, while its status looked fine. The event log shows a hook write landing just
  after the shim's registration, so `register_hook_session`'s read-then-write probably cleared
  the binding; that race is inferred, not reproduced. Done when: the shim treats a row with no
  binding as unregistered and re-registers, shown by a test, and a live session's send recovers
  without a reconnect. Closing the race in the daemon is left open.
  The test half is shown by `a_row_without_a_client_binding_is_not_registered` and
  `roster_self_check_reregisters_only_after_the_full_window` in `crates/shim/src/main.rs`
  (`1c9fedf`). The live half wasn't reproducible on 2026-11-04: no session lost its binding.
- [x] **MUS-29: Make send timeouts trustworthy.** `send_message` reported "command
  timed out" for a message that did arrive, so a retry could send a duplicate. Done
  when: a test shows a timed-out send can be retried without delivering twice.
- [x] **MUS-31: Don't lose an agent's work when it disappears.** Research story. A
  session can end or be reaped with its work unsaved. Direct messages to a reaped
  session are deleted with it (`cleanup.rs`), and six went at the 2026-10-30 daemon
  restart, including `briny-tortoise`'s field notes. Questions: what disappears and
  when, how to keep it recoverable, how to hand work to a successor, and how a
  session can save as it goes. Findings and design: [agent-disappearance.md](agent-disappearance.md).
  Done when: `docs/` records the ways work gets lost and a recommended design, with
  follow-up stories filed here.
- [x] **MUS-45: Pin what `del_by_id` cascades.** From MUS-31, and do this first: the
  cause there is from reading the code. Done when: a test shows whether reaping a
  session leaves its mail live until the next boot, and fails if myko changes that.
- [x] **MUS-46: Keep mail to a reaped session.** From MUS-31. Don't just switch to a
  cascading delete, which loses mail sooner. Done when: tests show a restart after a
  reap removes no message under 14 days old, and a session returning under its old
  id doesn't get read mail again (`MessageRead` also belongs to `Session`).
- [ ] **MUS-47: Hand unread mail to a successor.** From MUS-31. Retained mail reaches
  nobody without this. Depends on MUS-3. Done when: a session taking over an identity
  receives its predecessor's unread mail, shown by a test.
- [x] **MUS-48: Tell a sender about mail unread at reap.** From MUS-31. Done when: a
  sender can see that a message went unread when its recipient was reaped.
- [x] **MUS-49: Report what a restart will remove.** From MUS-31: one restart is now
  due to remove 422 messages. Done when: the operator can see the count before
  approving a restart.
- [ ] **MUS-26: Keep `current_task` fresh.** It can go stale: one session showed
  `feature/comments` while on `feature/search`. Done when: the roster's task
  follows the branch, unless a session has set its own text.
- [x] **MUS-44: Let other programs read the observed roster.** The Muster half of what
  MUS-32 proposed, and what Rally needs to detect a stall. Sessions' observed facts (alive,
  can be woken, last activity, and how fresh each is) are readable by a local program
  without acting as a session, and declared text such as `current_task` is labelled as
  declared. Done when: a documented read-only command or endpoint returns those facts, and
  a test shows it can't change anything. Depends on MUS-4.
  `muster-shim roster` does this. `last_activity_at` turned out to be declared, since a shim
  can set it, and `bound_to_connection` can outlast a dropped connection until the row is reaped.
- [ ] **MUS-58: Drop ended sessions from rooms.** Found by the watchman on 2026-11-04: `everyone`
  still held room rows for ten sessions that had ended the night before, so the takeover
  broadcast reached 4 of 14 members, each miss failing as "session vanished from roster
  mid-broadcast". Done when: a session that leaves the roster leaves its rooms too, a
  broadcast counts only live members, and a test covers a session reaped during a broadcast.

## Phase 2: Security

- [ ] **MUS-39: Operator-verified, scoped approvals.** Top priority, set by the operator on
  2026-11-03. A message can't prove who wrote it: a session runs as the operator's own account,
  so it can send "the operator says yes" as easily as a relay can, and MUS-5 and MUS-6 don't
  change that. So a restart or a new service has needed the operator's word in the acting
  session's own terminal, which held up the Rally rollout on Murph repeatedly. Fix: the operator
  approves with a step a session can't perform, such as a `sudo` password on the daemon host or
  Touch ID on a laptop. That signs an approval naming one action and target, which expires and
  works once. The daemon checks it, and a relay carrying a valid one is shown as
  operator-verified. Standing approvals, for example "restarts to a commit on main that passed
  CI", could live in a root-owned file that sessions can read but not change. MUS-7 comes first,
  since an unauthenticated local port lets any process act as a session. Done when: a relayed
  approval with a valid signed token is accepted for exactly its action; a forged, expired,
  reused or mismatched one is refused; and a session on the operator's account can't produce a
  valid one. Each is shown by a test.
- [x] **MUS-7: Protect the local ports.** The first step of MUS-39. 6256 and 6257 have no
  authentication, and the hook listener doesn't check `Origin`, so any process on the daemon's
  host, or a web page in its browser, can reach them. Done when: a test shows a cross-origin web
  page can't reach either port, including by WebSocket, and a local process without
  credentials can't act as a session.
  Progress 2026-11-04 (gentle-pulsar): the daemon writes a local token to
  `~/.config/muster/local-token` (owner-only); the hook listener on 6256 requires it and refuses
  any request with an `Origin` or an upgrade; a plain local gate on 6257 requires it, refuses
  `Origin`, and relays only myko's WebSocket route through the frame filter as `(local)`; myko's
  own listener moves to `MUSTER_MYKO_BIND` (6258). The shim and TUI present the token. Shown by
  `crates/daemon/tests/local_access.rs`. Not ticked: myko's own listener on 6258 still takes
  anyone on the host, which is MUS-59; until then a local process can still act as a session by
  dialing it.
  Done 2026-11-04 with MUS-59: no port on the daemon's host now answers a browser origin or a
  local process without the local token. Shown by `local_access.rs`, `myko_filter.rs` and the
  gate end-to-end script. The 6258 half is live only after the next daemon restart.
- [x] **MUS-59: Guard myko's own listener.** From MUS-7. myko-server binds its WebSocket/MCP
  listener itself and checks nothing, so a process on the daemon's host can dial it and skip the
  local gate. myko offers no accept hook or origin check, so this needs one upstream, a patched
  copy, or a listener the daemon owns and hands connections to. Done when: a process without the
  local token can't reach myko by any port, shown by a test, and the README's port table has no
  unguarded row.
  Found 2026-11-04 by gentle-pulsar, on a throwaway daemon built from `a054c58`: a WebSocket
  upgrade carrying a browser `Origin` and no token gets `101` on myko's own port and `403` on
  the local gate, and an MCP `tools/list` POST with a browser `Origin` gets `200` with the tool
  list. So a web page in a browser on the daemon's host can still drive myko, as it could
  on 6257 before MUS-7; this isn't a regression, but it is the gap that matters most. myko-server
  6.9.0 has no Origin check anywhere, and the daemon can't take over the listener: `run()` binds
  `bind_addr` itself, and the startup steps it runs first (Postgres catch-up, the saga runtime)
  are private, though the per-connection handlers (`ws_handler::WsHandler`, `router`) are public.
  Options, for the operator: (a) carry a patched myko-server through `[patch.crates-io]`, adding
  a way to run on a listener the daemon owns (or an accept filter), the smallest change that
  closes it; myko is AGPL-3.0-or-later and Muster is MIT or Apache-2.0, so the patched copy
  keeps its own license in its own directory, and it must be carried across myko upgrades.
  This changes what Muster carries, not its license position: every Muster crate already
  depends on AGPL myko crates (`myko`, `myko-server`, `myko-macros`), unpatched;
  (b) ask myko's maintainers for that hook upstream, which is cleaner but has no timeline; or
  (c) both, with (a) dropped once (b) ships.
  Done 2026-11-04 by gentle-pulsar, with option (c) as the operator chose: a patched
  myko-server in `vendor/myko-server` (AGPL-3.0-or-later, see its `MUSTER-PATCH.md`) adds a
  request filter, and the daemon's filter on myko's own listener refuses any browser `Origin`
  (403) and anything without the local token (401), which only the gates add. Shown by
  `crates/daemon/tests/myko_filter.rs` and three checks in `scripts/test-gate-e2e.sh`. The patch
  is dropped once upstream myko-server ships an equivalent hook; that request goes through the
  watchman first. Live on Murph only after the next daemon restart.
- [ ] **MUS-56: Let only authorized devices reach Murph's ports.** Requested by the operator on
  2026-11-03. Murph's firewall (`/etc/nftables.conf`) opens 6255 to the whole `192.168.1.0/24`,
  and Rally's 6300 isn't open at all, so a1 can't reach it. The services already refuse a
  client without a pairing, but any device on the LAN can still reach the gate. Fix: open
  6255 and 6300 only to listed machines (a1 today, with its address reserved on the router),
  and keep pairing as the real check. Changing the firewall needs root, so the operator applies
  it. Done when: from a LAN device not on the list, 6255 and 6300 can't be reached; from a1 both
  work; and an unpaired client that does reach either is refused. Each is shown by a run.
- [x] **MUS-57: Keep pairing secrets out of sessions.** On 2026-11-02 a session printed a1's
  whole pairing code, token included, into its own output while looking for Murph's address,
  so it reached that session's transcript and the model API. Nothing public. The token was
  rotated on 2026-11-03 at 23:03 EST on the operator's word, with the new code written only to a
  private file. Fix: make the safe way the easy one. A command shows the pairing's address,
  name and fingerprint with the token masked, and the shim and daemon redact `muster-pair:v1:`
  codes and Bearer tokens from their logs and from message bodies. Done when: that command
  exists, and a test shows a code or token in a log line or message comes out redacted.
  Done 2026-11-04 by gentle-pulsar: `muster-shim pairing` shows the pairing with its token masked,
  and `pairing::redact` masks `muster-pair:v1:` codes and Bearer tokens (any case) in both binaries'
  log lines and in message bodies before they are stored. Shown by the unit tests in
  `crates/entities/src/pairing.rs` and `crates/daemon/tests/redaction.rs`.
- [x] **MUS-5: Tie sessions to the machine's token.** The gate discards the
  machine name and `session_id` is taken on trust, so any paired machine can claim
  another session and read and clear its mail. Paired clients can also write raw events
  to the store. Design: [session-binding.md](session-binding.md). Done when: a test shows
  one paired machine can't read, clear or send as another machine's session, including
  by raw event.
- [x] **MUS-42: Run the gate e2e script in CI.** It turned out CI already ran it, in the
  `check` job since `7d2c5d7`; we'd filed this believing it only ran by hand. Done when: CI
  runs it on each PR, and a deliberately broken gate check fails the build. Shown by run
  41245316360 on PR #28, closed unmerged: `check` failed with 47 passed, 1 failed.
- [ ] **MUS-41: Replace the gate's frame filter with authorization in myko.** The filter is
  an allowlist over a format myko parses separately, which is where three bypasses came
  from. Open for the operator: propose a hook upstream, or carry a fork meanwhile. Done
  when: myko refuses these writes itself, a test shows it, and the filter is retired.
- [x] **MUS-33: Harden the gate further.** From the 2026-10-30 adversarial review, not
  fixed yet: a compromised machine can capture operator-addressed mail by being the most
  active session, including with a far-future `lastActivityAt`; `project` is unattested,
  so it can join any `project:` room; `unpair` can race the daemon's bindings file; a
  `/hook/wait` opened before `unpair` keeps running for up to 10 minutes; and oversized
  message bodies can break a peer's `ReadMessages`. Done when: each is fixed or recorded
  as accepted, with a test for each fix.
  Fixed: a future `lastActivityAt` no longer wins operator mail, `unpair` and the daemon
  lock the bindings across processes, `unpair` closes open connections within seconds, and
  bodies over 64 KiB are refused. Accepted for now by the operator 2026-11-02: a compromised machine can still win
  operator mail by being genuinely most active, because routing to the busiest session is
  the feature; and `project` stays client-claimed, so a machine can join any `project:` room
  and read its broadcasts, which the threat model's one operator makes low value. Revisit
  both before unattended running, and when other people's machines come into scope.
- [x] **MUS-6: Verify `operator` and `host` on the daemon.** The client supplies
  both. Done when: the roster shows values the daemon established, and a test
  shows a client can't fake them.
- [x] **MUS-40: Clear rooms left by self-reported operators.** Found rolling out MUS-6.
  Sessions are still in `op:josh` (a1) and `op:joshd` (Murph) beside `op:joshd@pdx.edu`,
  so a broadcast to an old room reaches only one machine. Done when: a paired session is
  only in its attested `op:` room, shown by a test and on the live roster.
- [x] **MUS-8: Single-use, expiring pairing codes.** Done when: a used or expired
  code is refused.
  Done 2026-11-04 by gentle-pulsar: `muster-daemon pair` prints a `muster-pair:v2:` code holding
  a one-time invite, not the token, good for 15 minutes by default (`--expires`). `muster-shim
  pair` exchanges it at the gate's `/pair` route for a token; the invite is deleted on first use,
  and the daemon keeps only its hash. Shown by the invite tests in
  `crates/daemon/src/credentials.rs` and five checks in `scripts/test-gate-e2e.sh` (no token in
  the code, a used code refused, an expired code refused and given no token). Live on Murph only
  after the next daemon restart; v1 codes and stored pairings keep working.
- [x] **MUS-9: Limit connections and message rates.** Done when: a test shows a
  flooding client is throttled and other sessions keep working.
  Done 2026-11-04 by gentle-pulsar: each sending session has a token bucket (a burst of 20, then
  30 a minute; a broadcast counts as one), and each listener caps its open connections, the gate
  per IP and per paired machine. Shown by `crates/daemon/tests/rate_limit.rs` (a flooding session
  is refused and another keeps sending), the limiter unit tests, and three checks in
  `scripts/test-gate-e2e.sh` (a machine at its cap gets 503, another machine keeps working, and
  the first gets back in once its connections close). Live on Murph only after a daemon restart.
  The message limit is per session, so a machine with many sessions (it can register any number
  of its own) gets that many allowances; a per-machine total is MUS-60.
- [x] **MUS-10: Treat peer messages as untrusted.** Done when: the operator can
  set sender allowlists and pause all wake-ups.
  Done 2026-11-04 by gentle-pulsar: `muster-daemon pause` and `resume` stop and restart every
  wake-up (held waits hand out nothing and no live push goes out, while mail is kept and seen on
  the next prompt), and `allow`, `disallow` and `senders` set which machines may send. The
  daemon's own host is always allowed. Both are files in the state directory that the daemon
  applies within 2 s, and the pause shows on rosters as `wakes_paused`. Shown by
  `crates/daemon/tests/pause_and_allow.rs`, the unit tests in `crates/daemon/src/controls.rs`, and
  three checks in `scripts/test-gate-e2e.sh`. Limit: any process on the operator's account can
  change those files, so this is only as strong as that account until MUS-39.
- [x] **MUS-11: Audit dependencies and write the threat model.** Done when: CI
  runs `cargo audit` or `cargo deny`, and the threat model is in `docs/`.
  Done 2026-11-04 by gentle-pulsar: CI runs `cargo deny check` (advisories, licences, bans and
  sources) from `deny.toml`, and the threat model is `docs/threat-model.md`. The first run found
  five rustls advisories (the gate's TLS) and one each in rustls-webpki, tokio, ctutils and
  webpki-roots, all fixed here by in-range updates. Three unmaintained build-time crates are
  ignored with reasons, and four unlicensed web UI dependencies are excluded, which is MUS-61.
- [ ] **MUS-12: Give the watchman a read-only GitHub token.** Done when: the
  watchman can verify a story in all three repositories and a merge attempt with
  its token is refused. Needs a run: the operator, who holds the token.

- [x] **MUS-60: Cap messages per machine, not only per session.** From MUS-9's review. A paired
  machine may register any number of sessions, and each gets its own message allowance, so a
  machine can multiply its rate. Done when: a machine's sessions share a total, and a test shows a
  machine spreading a flood across new sessions is throttled while another machine keeps sending.
  Done 2026-11-04 by gentle-pulsar: besides its own bucket, each message spends one from a bucket
  its machine's sessions share (a burst of 60, then 60 a minute), keyed on the session's host, which
  the gate attests for a paired machine. A refusal spends neither. Shown by
  `a_flood_spread_across_new_sessions_is_throttled_per_machine` in
  `crates/daemon/tests/rate_limit.rs` and the unit tests in `crates/entities/src/rate.rs`.
  Two limits remain, both needing the daemon host's local token (today, the operator's account):
  a session with no host gets no machine bucket, and a session through the local gate keeps the
  host name it claims, so a process on the daemon's host could skip the shared limit or spend
  another machine's by claiming its name.
- [ ] **MUS-61: License or replace the web UI's unlicensed dependencies.** From MUS-11. `mullion`,
  `css-styled`, `css-styled-derive` and `css-spec-data`, from ignition-is-go's git repositories,
  declare no licence, so Muster has no stated permission to use them. `deny.toml` excludes them
  from the check, which also hides rkyv 0.7 under them. Open for the operator: ask their authors
  for a licence, or drop or replace them in `crates/ui`. Done when: `deny.toml` excludes nothing.

## Phase 3: Portability

- [ ] **MUS-13: Run CI on macOS.** Done when: CI runs on macOS and passes,
  including `recovery_tracks_codex_effective_working_directory`, which fails on
  macOS today.
- [ ] **MUS-14: Publish prebuilt binaries.** There are no releases yet, so
  `scripts/install-codex.sh` can't work and the plugins aren't on npm. Done when: macOS and Linux can
  install without a Rust toolchain.
- [ ] **MUS-15: One-command setup.** Done when: one command sets up a new machine,
  `muster doctor` reports what's wrong, and launchd and systemd units keep the
  daemon running across reboots. Needs a run: the operator, on a clean machine and
  through a reboot.
- [ ] **MUS-16: Support addresses beyond one LAN IP.** Done when: a paired laptop
  stays connected when it moves between home and a Tailscale or WireGuard network. Needs a run: the operator, who
  has the laptop.
- [ ] **MUS-17: Keep messages across daemon restarts without Postgres.** Check
  first whether myko allows it. Done when: an unread message survives a daemon
  restart with no Postgres configured.
- [ ] **MUS-18: Extend pairing to Codex.** Codex goes through the Rust shim. Done
  when: Codex can connect through the gate. The Pi, opencode and Prime Agent plugins
  are left out under the language decision.
- [ ] **MUS-19: Fix stale docs.** Most were corrected on 2026-10-30. Done when: following the README on a clean machine works. Needs a run: the Muster team, on a clean machine.

## Phase 4: Coordination

- [ ] **MUS-20: Retire Marshal.** Done when: no session starts `marshal-shim`, and
  its `:6155` config is gone.
- [ ] **MUS-21: Port the ghost-row and operator fixes.** From `fa626e5` and
  `96fd28b` in the Marshal repo. Done when: the roster has no ghost rows, each
  operator appears once, and a broadcast's delivered and failed counts add up to
  its total, with any other outcome reported.
- [ ] **MUS-22: Print nicknames at session start.** Done when: the operator can see
  which session is which without asking.
- [ ] **MUS-36: Message every active team from a terminal.** Requested by the operator
  on 2026-10-31. A Rust command for the operator, for example `muster-shim all "<message>"`.
  The web console can already broadcast, but not from a terminal. Active means a session
  the daemon sees as live, including an idle one that can be woken. Done when: from a
  terminal on a1 or Murph, one command reaches every active team on both machines and
  wakes the idle ones; reaped sessions get nothing; the output names who received it and
  who didn't; and recipients see which machine sent it. That label is the machine, not
  the person (see MUS-39). Needs a run: installed on both machines.
- [ ] **MUS-37: Message one team directly from a terminal.** Requested by the operator on
  2026-10-31. The same command as MUS-36, for example `muster-shim tell <team> "<message>"`,
  without being inside a Claude session. Takes a team name, nickname or session id. Done
  when: from another terminal on either machine, a message reaches that team and wakes it
  if idle; a team with several sessions gets one clear result for each; an unknown or
  ambiguous name is refused, not guessed; and the label is as in MUS-36.
- [ ] **MUS-38: Muster team name not displaying correctly (adjective-animal).** Reported
  by the operator on 2026-10-31. Sessions show an adjective-animal nickname. The cause,
  and where the wrong name appears, aren't confirmed yet; the first step is to confirm
  both with the operator. Related: MUS-3, MUS-22. Done when: the operator confirms that
  the screens they named show the name they expect, on both machines.
- [ ] **MUS-43: Give muster messages a deterministic structure.** Requested by the operator
  on 2026-10-31: Muster is meant to be a reactive protocol, and today a message is free text
  that only an agent can interpret. Starts with a design meeting with the operator; no code
  until then. Done when: the design from that meeting is written down with the operator's
  agreement, and follow-up stories are filed.
- [ ] **MUS-51: Show the operator-assigned watchman on the roster.** Requested by the operator
  on 2026-11-02. A new team can read who the watchman is without asking anyone. Only the
  operator can set the role, which shows who assigned it and when, and it clears when the
  session ends or the operator hands it on. It says who keeps the board and takes submissions,
  and gives no authority: restarts, merges and device changes still need the operator's direct
  word. Done when: a test shows a session can't assign itself the role, including by writing
  its own `Session` row, and that the role clears when its session is reaped; and a new session
  can read it from the roster. Rally reads the role from Muster and doesn't store it.
  Depends on MUS-39, but MUS-39 only covers terminal messages, so it needs extending to prove
  the operator set a role. It can't be a `Session` field, because the gate lets a machine write
  its own row whole. MUS-3 lets peers reach a watchman by role, but until it lands a restarted
  watchman loses the role and needs assigning again. MUS-3's role wording should use this one.
- [ ] **MUS-52: Show `wake` in the TUI and web UI.** Follow-up to MUS-4, which added it to
  `muster://roster`. Done when: both show each session's wake state, and tell observed
  `waiting` from declared `channel` and `busy`.
- [x] **MUS-53: Make the truncation notice name a working address.** Found by `copper-gulch`
  on 2026-11-02: a reader following the notice to `muster://messages/<id>` gets a client error
  saying the server doesn't implement resource reads. Done when: the notice names a URI that
  returns the full message, shown on a real truncated message.
- [x] **MUS-54: Keep a working session reachable when its connection drops.** Board finding 59,
  2026-11-03: desert-hawk's MCP WebSocket dropped at about 11:36:19 EST while its hooks kept
  reporting, and the daemon deleted its row at 11:37:25 under the 60 s grace. At 12:09 a message
  to it failed with "no session matches". A deletion therefore doesn't mean the session ended.
  Whether its next hook or tool call brings the row back is unchecked. Related: MUS-50, which
  repairs a row left without a client binding. Done when: a session whose connection drops but
  whose hooks are still active stays reachable, or its client reconnects and re-registers, shown
  by a test that drops the connection, and a message sent in that window reaches it.
  Verified 2026-11-04 by gentle-pulsar on `e797fa3`: zesty-quail's row (connected 10:30:16 EST; the
  watchman saw its process gone before 10:31) stayed on the roster unbound for over ten minutes
  and was then reaped under `KEPT_STALE_AFTER`, where `c3dbd45` deleted such rows after 60 s.
  Delivery into that window is shown by the e2e test in `74d35ed`.
- [ ] **MUS-55: Make a daemon restart one command.** Requested by the operator on 2026-11-03.
  A restart already disrupts little: in the last three, sessions were back on the roster within
  about 2 s and no wake misfired. The cost is the operator's and the daemon-holder's time: about
  20 to 30 minutes of manual steps each rollout (build and e2e, count mail at risk, back up
  Postgres and the binaries, swap the binary, restart, health checks, a 5-minute watch), with no
  rollback. Two parts. A rollout command that runs those steps against a named commit on main and
  rolls back by itself if the daemon doesn't come up healthy. And a graceful stop: on SIGTERM the
  daemon ends held waits with an empty reply, so waiters re-poll quietly instead of reporting the
  daemon unreachable. Done when: one command rolls a commit out on the daemon host and reports
  what it did, a test shows it rolls back a daemon that fails its health check, and a test shows a
  held wait ends cleanly on SIGTERM. The operator's direct OK is still needed to run it. Related:
  MUS-15, `scripts/restart-preview.sh`.
- [ ] **MUS-30: Decide the future of the TypeScript plugins.** The operator decides
  whether to keep, freeze or remove them. Done when: the decision is recorded here,
  and if they're removed, the repository's `package.json` workspace goes with them.
- [ ] **MUS-24: Stretch goals.** Once the operator has listed them. Not ready to
  claim yet.

## Phase 5: Merging into Marshal

From the rally of 2026-11-04 evening. The operator's word, through the watchman: upstream
Marshal's lead developer welcomes Muster merging into Marshal. No PR goes to
ignition-is-go/marshal, or to the Castor-o7/marshal fork, until these goals are met, and the
operator decides each upstream PR. The goal numbers are the watchman's board's; G6 belongs to
Rally.

- [x] **MUS-62: Catch up with upstream Marshal (G1).** Merge upstream/main (v0.54.1, `94ecc24`)
  into Muster and keep Muster's features. Done when: build, tests and clippy are green, and the
  Zoo sandbox builds. Merged as #84; `vendor/myko-server` moved to 6.11.5 with it. Live on Murph
  only after a daemon restart.
- [x] **MUS-63: Inventory Muster's changes against upstream (G2).** A table of Muster's commits:
  already upstream, to port, to drop, and the names to restore for upstream (`marshal-shim`,
  the 6155 ports, `MARSHAL_*`). Merged as #79.
- [x] **MUS-64: Make the TLS gate optional (G3).** Plain WebSocket by default, like upstream, and
  a switch ("cert mode") that turns on the gate, pinning, tokens and pairing. Done when: tests
  cover both modes. Offer both a `--tls` flag and a `MUSTER_TLS=1` variable, and check upstream's
  daemon for which it uses; the operator asks upstream which it wants. Design in #82. Built with
  both, one port for local and remote, and a refusal to run plain while pairings exist unless
  `MUSTER_TLS=0` says so; the gate e2e covers both modes. **Murph's unit needs
  `Environment=MUSTER_TLS=1` before the restart that brings this in**, or the daemon refuses to
  start and the rollout rolls back.
- [ ] **MUS-70: Retire the 6257 alias.** From MUS-64. With TLS on, the daemon still binds the
  old local-gate port (`MUSTER_INTERNAL_BIND`, 6257) so sessions configured for it survive one
  restart. Done when: no machine's config names 6257 (a1's and Murph's `.mcp.json`, the TUI,
  the TS plugins), and the alias, its variable and its rollout check are removed. The upstream
  Gate PR ships without it.
- [x] **MUS-72: Hold a session's first writes until it's registered.** Found by tangerine-wombat
  in the G5 run on Murph: a tool call right after `initialize` reached the daemon before the
  shim's Session SET and failed "caller has no session on the roster". A real agent's first
  turn can do the same. Done when: a write waits, up to 5 s, for the session's own row, and a
  test shows it goes ahead once the row lands and gives up after the bound.
- [x] **MUS-73: Refuse an empty recipient.** Found by tangerine-wombat: `send_message` with
  `to: ""` matched every id by prefix and was delivered to the sender itself. Upstream
  `94ecc24` behaves the same; a small upstream fix, when the operator allows upstream PRs.
  Done when: an empty or blank `to` is refused with a clear error and nothing is stored.
- [x] **MUS-65: Document live delivery without the channels flag (G4).** How hooks give live
  delivery and wake, checked against the code, and how that differs from Marshal's channel
  push. Merged as #80.
- [ ] **MUS-66: Message between daemons (G5).** Secure and bidirectional, on the TLS mode. The
  watchman reads a design and threat model before any building. The design answers what
  configuration links two daemons (peer address, token or certificate fingerprint, and whether
  `muster-daemon pair` is reused), whether a test session needs its own shim pairing file, what
  happens to messages in flight when either daemon restarts, and whether it needs the TLS mode.
  Done when: it's implemented with tests and a real a1↔Murph run. Needs a run: Lamplighter, with a
  second daemon on Murph (loopback ports 6265–6268, its own state dir, config dir and database).
  Design merged as #90. Built: `pair --link` and `link`, the `/link` route and protocol, remote
  sessions on the roster, `nickname@link` addressing, an outbox that survives a restart (with
  Postgres), and the threat-model checks, all covered by `scripts/test-links-e2e.sh`. Still
  needs the two runs: Lamplighter's two-daemon test on Murph, then the real a1↔Murph run with
  the operator's OK.
- [ ] **MUS-67: Write up Muster against Marshal (G7).** What Muster adds and changes, written
  for the eventual upstream PR descriptions. Draft in #81.
- [ ] **MUS-68: Simulate each behaviour change in Zoo (G8).** Upstream requires a real-agent
  simulation, imported into Marshal Zoo, for every change to behaviour or agent-facing
  instructions (`scripts/sandbox/README.md`, upstream `8563800`). Done when: each change bound
  for upstream has its simulation. Plan for the hooks change in #83.
- [ ] **MUS-69: A delivery conformance suite.** Prove live delivery works for every harness
  (Claude Code hooks, the Codex app-server bridge, opencode, Pi, Prime Agent) without running their
  TUIs or any model: the real `muster-daemon` and `muster-shim` binaries against a fake harness for
  each, all passing one scenario table. The table: an idle session wakes on a DM; a busy session gets
  it at its next hook or turn; a dropped ack means redelivery; a duplicate is shown at most once; a
  daemon restart in the middle of a wait; operator pause stops wakes; broadcasts never wake anyone;
  and a session the roster calls `wake: none` isn't told `wake: expected` (MUS-71). Each fake is
  pinned to a harness version, uses recorded fixtures, and says which TUI behaviour it assumes. Claude
  runs in plain and TLS modes. Done when: it runs in CI with a harness × scenario grid, gaps are
  listed openly, and there's a short doc on re-recording fixtures
  ([delivery-conformance.md](delivery-conformance.md)). Stage 1: the suite and Claude in both modes.
  Stage 2: Codex. Stage 3: the TypeScript plugins, with bun in CI from its GitHub release, checked
  against a committed SHA-256.
- [ ] **MUS-71: Don't report `wake: expected` for a session that can't wake.** From MUS-69. The
  roster's `wake_state` counts a live client as a wake path only with channels on, but
  `SendMessage` reports `wake: expected` for any recipient with a live `client_id`
  (`crates/entities/src/send_message.rs`). So a Claude session whose MCP shim is connected, with no
  waiter and channels off, reads `wake: none` on the roster while a sender is told `wake: expected`.
  Done when: the send result says `expected` only when the roster would say `waiting` (a live
  waiter) and wakes aren't paused, a live client alone no longer counts, and the conformance
  suite's `wake-none-not-expected` cells pass and leave `KNOWN_FAILURES`. A bridge-watched Codex
  thread reads `none`/`unobserved` in both places until MUS-77.
- [ ] **MUS-77: Show a Codex bridge's wake path on the roster.** From MUS-71. The roster calls a
  Codex session `wake: none`, and a send to it says `wake: unobserved`, though its bridge will
  start a turn, because the bridge leaves no trace on the row: Codex's MCP shim registers no
  session, and the bridge writes no `WakeWatcher`. Done when: the bridge refreshes a
  `WakeWatcher` for each thread it can wake, so the roster says `waiting` and a send says
  `expected` for it, and stops when it can't (paused per MUS-74, or the app-server is gone).
- [ ] **MUS-74: The Codex bridge ignores the operator's pause.** From MUS-69. `muster-daemon
  pause` stops Claude's waiter (`/hook/wait` hands out nothing) and the live push, but
  `codex_bridge.rs` sends `turn/start` for any unread direct message without checking it, so a
  paused Codex session is still woken. Done when: the bridge doesn't wake while wakes are paused,
  mail still arrives at the next hook, and the conformance suite's `codex` ×
  `operator-pause-stops-wakes` cell passes.
- [ ] **MUS-75: Mail is lost when a prompt hook's reply doesn't arrive.** From MUS-69. The hook
  listener marks prompt-hook mail read once it has flushed the reply (`deferred_ack`). If the reply
  is then lost, the mail is gone. That can happen when the harness kills the hook at its timeout
  after the daemon wrote, or when a paired machine's tunnel breaks after the gate wrote.
  `hook-delivery.md` says a dropped hook leaves the mail unread, which holds only for a failure
  before the flush. It affects Claude's busy-path hooks and every Codex delivery. Done when: a
  reply that doesn't reach the harness leaves the mail unread (a client ack like the waiter's, or
  another design), and the suite's `lost-hook-reply-redelivers` cells and `codex` ×
  `dropped-ack-redelivers` pass.
- [ ] **MUS-76: Restarts under test, with Postgres, and Codex sessions after a restart.** From
  MUS-69. The suite restarts the daemon without Postgres, so mail sent before a restart isn't
  covered, and the roster empties. A Claude session's waiter re-registers it, but an idle Codex
  session stays off the roster until its next hook, because the bridge registers a thread only on
  `thread/started`. Done when: the suite runs the restart cells against Postgres in CI, covering
  mail sent before the restart, and the bridge re-registers its threads when its daemon connection
  comes back, so the `codex` × `daemon-restart-mid-wait` cell passes.
- [ ] **MUS-78: After a TLS daemon restart, a shim's first write can beat its registration.**
  From MUS-69, against MUS-72. MUS-72 holds a shim's first write until its session is on the
  roster, for at most 5 s (`REGISTER_WAIT`). With TLS on, right after the daemon restarts, a fresh
  shim's registration can take longer, so the hold expires and the daemon refuses the send with
  "has no session on the roster". The conformance suite's `claude-tls` ×
  `daemon-restart-mid-wait` cell hit it in 2 of 8 runs; it retries that one refusal for now,
  naming this story. Done when: the first write after a TLS restart isn't refused, and that
  retry is removed.

## Moved to Rally

Moved on 2026-11-02 when the operator split work tracking out of Muster. They continue in
[Rally's roadmap](https://github.com/Castor-o7/rally/blob/main/docs/roadmap.md). The IDs
stay reserved here so old commits and notes still make sense.

- **MUS-23: Native board.** Now RAL-3, and no longer stored in the daemon.
- **MUS-28: Show whether a team is available, working or blocked.** Now RAL-2. MUS-44 keeps
  the observed half in Muster.
- **MUS-32: Design the muster ledger.** Now RAL-1. MUS-44 keeps the observed facts in Muster.
