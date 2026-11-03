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

2026-11-03: 11 of 51 stories ticked, about 22%, not counting the 3 moved to Rally. Live on
Murph since 2026-11-03: `c3dbd45`, which includes MUS-4, MUS-5, MUS-6, MUS-29, MUS-33,
MUS-34, MUS-35, MUS-40, MUS-44, MUS-50 and MUS-53. Sessions pick up a rebuilt shim when they
restart, and no keyed send (MUS-29) has been seen live yet. Merged, live and not yet
verified: MUS-4, MUS-34, MUS-35, MUS-44, MUS-50 and MUS-53. Merged and needing a live run:
MUS-36 and MUS-37.

## Decisions

- **Muster and Rally are separate.** Decided by the operator on 2026-11-02. Muster carries
  messages and wake-ups, and owns identity, the roster and whether a session can be woken.
  Rally owns claims, work states, evidence, check-ins and history. Rally depends on Muster,
  and Muster never depends on Rally, so Muster works the same with Rally down. Muster has no
  concept of a story or a stall.

- **Threat model.** Muster defends against a malicious website, and against one
  compromised session or paired machine. All machines belong to one operator.
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
- [ ] **MUS-4: Show dead watchers.** Done when: the roster distinguishes a
  session that can be woken from one that can't.
- [x] **MUS-25: Reach the daemon when `localhost` resolves to `::1` first.** Found
  during live-delivery testing. Done when: an unpaired Linux host's watcher wakes
  on a message with no explicit URL. Evidence: `090b2a9`, tested on Murph by
  crimson-avocet.
- [x] **MUS-27: Deliver long messages whole through the watcher.** Monitor cuts
  a line at about 500 characters and an event at about 3,000. Done when: a message
  up to the daemon's 2,000-character limit reaches an idle session complete. Needs
  a run: the Muster team, with a peer sending.
- [ ] **MUS-34: Don't lose mail between delivery and ack.** From the 2026-10-30 review.
  A message is marked read once it reaches the gate's loopback socket, so a silent network
  drop loses it. A waiter whose session just closed can also take mail and drop it, a plain
  `watch` has no parent check, and a resumed session can lose its wake to the old waiter.
  Concurrent readers can deliver a message twice. Done when: delivery is acknowledged by
  the client, and a test covers a dropped path.
- [ ] **MUS-35: Say why a watcher can't reach the daemon.** A 401 or 403 and a real outage
  print the same "can't reach" line, and a daemon whose hook port failed to bind keeps
  running with hooks dead. Done when: the message names the cause, and a failed hook bind
  stops the daemon or is plainly reported.
- [ ] **MUS-50: Repair a roster row that has lost its client binding.** Found 2026-11-02: a
  session listed on the roster with `pid` 0 and no `client_id` got "no session on the roster"
  on every send, while its status looked fine. The event log shows a hook write landing just
  after the shim's registration, so `register_hook_session`'s read-then-write probably cleared
  the binding; that race is inferred, not reproduced. Done when: the shim treats a row with no
  binding as unregistered and re-registers, shown by a test, and a live session's send recovers
  without a reconnect. Closing the race in the daemon is left open.
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
- [ ] **MUS-46: Keep mail to a reaped session.** From MUS-31. Don't just switch to a
  cascading delete, which loses mail sooner. Done when: tests show a restart after a
  reap removes no message under 14 days old, and a session returning under its old
  id doesn't get read mail again (`MessageRead` also belongs to `Session`).
- [ ] **MUS-47: Hand unread mail to a successor.** From MUS-31. Retained mail reaches
  nobody without this. Depends on MUS-3. Done when: a session taking over an identity
  receives its predecessor's unread mail, shown by a test.
- [ ] **MUS-48: Tell a sender about mail unread at reap.** From MUS-31. Done when: a
  sender can see that a message went unread when its recipient was reaped.
- [ ] **MUS-49: Report what a restart will remove.** From MUS-31: one restart is now
  due to remove 422 messages. Done when: the operator can see the count before
  approving a restart.
- [ ] **MUS-26: Keep `current_task` fresh.** It can go stale: one session showed
  `feature/comments` while on `feature/search`. Done when: the roster's task
  follows the branch, unless a session has set its own text.
- [ ] **MUS-44: Let other programs read the observed roster.** The Muster half of what
  MUS-32 proposed, and what Rally needs to detect a stall. Sessions' observed facts (alive,
  can be woken, last activity, and how fresh each is) are readable by a local program
  without acting as a session, and declared text such as `current_task` is labelled as
  declared. Done when: a documented read-only command or endpoint returns those facts, and
  a test shows it can't change anything. Depends on MUS-4.
  `muster-shim roster` does this. `last_activity_at` turned out to be declared, since a shim
  can set it, and `bound_to_connection` can outlast a dropped connection until the row is reaped.

## Phase 2: Security

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
- [ ] **MUS-7: Protect the local ports.** 6256 and 6257 have no authentication,
  and the hook listener doesn't check `Origin`. Done when: a test shows a
  cross-origin web page can't reach either port, including by WebSocket.
- [ ] **MUS-8: Single-use, expiring pairing codes.** Done when: a used or expired
  code is refused.
- [ ] **MUS-9: Limit connections and message rates.** Done when: a test shows a
  flooding client is throttled and other sessions keep working.
- [ ] **MUS-10: Treat peer messages as untrusted.** Done when: the operator can
  set sender allowlists and pause all wake-ups.
- [ ] **MUS-11: Audit dependencies and write the threat model.** Done when: CI
  runs `cargo audit` or `cargo deny`, and the threat model is in `docs/`.
- [ ] **MUS-12: Give the watchman a read-only GitHub token.** Done when: the
  watchman can verify a story in all three repositories and a merge attempt with
  its token is refused. Needs a run: the operator, who holds the token.

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
- [ ] **MUS-39: Show that a terminal message came from the operator.** Found building
  MUS-36. A session runs as the same account as the operator's terminal, so it can read
  the pairing token and run the same command; MUS-5 and MUS-6 don't change that. Telling
  them apart needs something a session can't reach, such as a key that needs the
  operator's touch or passphrase. Until then these messages carry no extra authority.
  Done when: a session on the operator's own account can't produce a message recipients
  see as from the operator, shown by a test that tries.
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
- [ ] **MUS-53: Make the truncation notice name a working address.** Found by `copper-gulch`
  on 2026-11-02: a reader following the notice to `muster://messages/<id>` gets a client error
  saying the server doesn't implement resource reads. Done when: the notice names a URI that
  returns the full message, shown on a real truncated message.
- [ ] **MUS-54: Keep a working session reachable when its connection drops.** Board finding 59,
  2026-11-03: desert-hawk's MCP WebSocket dropped at about 11:36:19 EST while its hooks kept
  reporting, and the daemon deleted its row at 11:37:25 under the 60 s grace. At 12:09 a message
  to it failed with "no session matches". A deletion therefore doesn't mean the session ended.
  Whether its next hook or tool call brings the row back is unchecked. Related: MUS-50, which
  repairs a row left without a client binding. Done when: a session whose connection drops but
  whose hooks are still active stays reachable, or its client reconnects and re-registers, shown
  by a test that drops the connection, and a message sent in that window reaches it.
- [ ] **MUS-30: Decide the future of the TypeScript plugins.** The operator decides
  whether to keep, freeze or remove them. Done when: the decision is recorded here,
  and if they're removed, the repository's `package.json` workspace goes with them.
- [ ] **MUS-24: Stretch goals.** Once the operator has listed them. Not ready to
  claim yet.

## Moved to Rally

Moved on 2026-11-02 when the operator split work tracking out of Muster. They continue in
[Rally's roadmap](https://github.com/Castor-o7/rally/blob/main/docs/roadmap.md). The IDs
stay reserved here so old commits and notes still make sense.

- **MUS-23: Native board.** Now RAL-3, and no longer stored in the daemon.
- **MUS-28: Show whether a team is available, working or blocked.** Now RAL-2. MUS-44 keeps
  the observed half in Muster.
- **MUS-32: Design the muster ledger.** Now RAL-1. MUS-44 keeps the observed facts in Muster.
