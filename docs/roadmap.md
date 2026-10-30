# Muster roadmap

Goal: Claude sessions on different machines message each other live, including
waking idle sessions, without `--dangerously-load-development-channels`, and
Muster is secure and portable enough to run unattended.

## Decisions

- **Threat model.** Muster defends against a malicious website, and against one
  compromised session or paired machine. All machines belong to one operator.
  Other people with accounts on those machines are the next step, not yet in
  scope.
- **Networks beyond the LAN.** Use Tailscale or WireGuard. The gate is not
  exposed to the open internet.
- **Platforms.** macOS and Linux are fully supported. Windows is best effort,
  for the shim only.
- **Ordering.** Anything that lets a Claude act without the operator waits until
  Phase 2 is done.

## Phase 1: Reliable delivery

- [ ] Fix watcher expiry. Monitor stops at 30 minutes, and an idle session may
  not get the chance to re-arm. Done when a session idle overnight still wakes.
- [ ] Close the first-prompt gap. Done when a restarted session can be reached
  before anyone types to it.
- [ ] Keep nicknames across restarts.
- [ ] Show dead watchers on the roster.

## Phase 2: Security

- [ ] Tie sessions to the machine's token. The gate discards the machine name
  and `session_id` is taken on trust, so any paired machine can claim another
  session and read and clear its mail.
- [ ] Verify `operator` and `host` on the daemon. The client supplies both.
- [ ] Protect the local ports. 6256 and 6257 have no authentication, and the hook
  listener doesn't check `Origin`. Test whether a website can reach the WebSocket
  port.
- [ ] Make pairing codes single-use and expiring.
- [ ] Limit connections and message rates at the gate.
- [ ] Treat peer messages as untrusted: sender allowlists and a pause switch.
- [ ] Add `cargo audit` or `cargo deny` to CI, and write up the threat model.

## Phase 3: Portability

- [ ] Run CI on macOS. CI is Linux-only.
  `recovery_tracks_codex_effective_working_directory` fails on macOS.
- [ ] Publish prebuilt binaries.
- [ ] One-command setup and `muster doctor`, with launchd and systemd units.
- [ ] Support addresses beyond one LAN IP.
- [ ] Keep messages across daemon restarts without Postgres, if myko allows it.
- [ ] Extend pairing to Codex, Pi, opencode and Prime Agent.
- [ ] Fix stale docs. The README still says `ws://localhost:6255`, which now
  requires TLS.

## Phase 4: Coordination

- [ ] Retire Marshal and remove its `:6155` config.
- [ ] Port the ghost-row and operator fixes from the Marshal repo.
- [ ] Print nicknames at session start.
- [ ] Stretch goals, once the operator has listed them.
