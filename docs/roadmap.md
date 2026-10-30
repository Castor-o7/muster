# Muster roadmap

Goal: Claude sessions on different machines message each other live, including
waking idle sessions, without `--dangerously-load-development-channels`, and
Muster is secure and portable enough to run unattended.

Stories follow the process in [board.md](board.md). IDs are permanent.
"Needs a run" means `gh` can't show the result, so the submission must include
evidence from a real run by the person or team named.

## Decisions

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
- **Ordering.** Anything that lets a Claude act without the operator waits until
  Phase 2 is done.

## Phase 1: Reliable delivery

- [ ] **MUS-1: Fix watcher expiry.** Monitor stops at 30 minutes; the expiry
  notice wakes an idle session, so re-arming works but costs about 48 wake-ups
  per session per day. The `asyncRewake` hook `muster-shim claude-hook wait` avoids
  that: it costs no model turns while idle, and Stop restarts it after every turn. Done when: a session idle overnight still wakes on a
  message, and an idle session uses at most 2 model turns per hour, with 0 as the
  target. Needs a run: the Muster team, overnight.
- [ ] **MUS-2: Close the first-prompt gap.** The same hook runs on SessionStart.
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
- [ ] **MUS-27: Deliver long messages whole through the watcher.** Monitor cuts
  a line at about 500 characters and an event at about 3,000. Done when: a message
  up to the daemon's 2,000-character limit reaches an idle session complete. Needs
  a run: the Muster team, with a peer sending.
- [ ] **MUS-29: Make send timeouts trustworthy.** `send_message` reported "command
  timed out" for a message that did arrive, so a retry could send a duplicate. Done
  when: a test shows a timed-out send can be retried without delivering twice.
- [ ] **MUS-31: Don't lose an agent's work when it disappears.** Research story. A
  session can end or be reaped with its work unsaved. Direct messages to a reaped
  session are deleted with it (`cleanup.rs`), and six went at the 2026-10-30 daemon
  restart, including `briny-tortoise`'s field notes. Why at restart rather than at
  reap time is still unexplained. Questions: what disappears and when, how to keep
  it recoverable, how to hand work to a successor, and how a session can save as it
  goes. Done when: `docs/` records the ways work gets lost and a recommended design,
  with follow-up stories filed here.
- [ ] **MUS-26: Keep `current_task` fresh.** It can go stale: one session showed
  `feature/comments` while on `feature/search`. Done when: the roster's task
  follows the branch, unless a session has set its own text.
- [ ] **MUS-28: Show whether a team is available, working or blocked.** The roster's
  `activity` can't separate resting from stuck, and the watchman shouldn't poll to
  find out. Done when: a team can set its state, and the watchman can read it without
  sending a message.

## Phase 2: Security

- [ ] **MUS-5: Tie sessions to the machine's token.** The gate discards the
  machine name and `session_id` is taken on trust, so any paired machine can claim
  another session and read and clear its mail. Done when: a test shows one paired
  machine can't read, clear or send as another machine's session.
- [ ] **MUS-6: Verify `operator` and `host` on the daemon.** The client supplies
  both. Done when: the roster shows values the daemon established, and a test
  shows a client can't fake them.
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
- [ ] **MUS-14: Publish prebuilt binaries.** Done when: macOS and Linux can
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
- [ ] **MUS-19: Fix stale docs.** The README still says `ws://localhost:6255`,
  which now requires TLS. Done when: following the README on a clean machine works. Needs a run: the Muster team, on a clean machine.

## Phase 4: Coordination

- [ ] **MUS-20: Retire Marshal.** Done when: no session starts `marshal-shim`, and
  its `:6155` config is gone.
- [ ] **MUS-21: Port the ghost-row and operator fixes.** From `fa626e5` and
  `96fd28b` in the Marshal repo. Done when: the roster has no ghost rows, each
  operator appears once, and a broadcast's delivered and failed counts add up to
  its total, with any other outcome reported.
- [ ] **MUS-22: Print nicknames at session start.** Done when: the operator can see
  which session is which without asking.
- [ ] **MUS-23: Native board.** Stories stored in the daemon, shown in the TUI and
  web UI, with commands to claim a story and submit it for review. Merged and
  Verified are independent, so it needs a merge field too. Wait for MUS-5, and use
  what the Version 0 board teaches. Done when: the watchman no longer needs
  the hand-kept table.
- [ ] **MUS-30: Decide the future of the TypeScript plugins.** The operator decides
  whether to keep, freeze or remove them. Done when: the decision is recorded here,
  and if they're removed, the repository's `package.json` workspace goes with them.
- [ ] **MUS-24: Stretch goals.** Once the operator has listed them. Not ready to
  claim yet.
