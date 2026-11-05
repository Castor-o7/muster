# Upstream inventory: Muster against Marshal v0.54.1

Goal G2. This lists every Muster change since the fork and says what should
go back to [Marshal](https://github.com/ignition-is-go/marshal). Nothing here
changes code.

## Baseline

| | |
|---|---|
| Muster `origin/main` | `ea472f6` (#78) |
| Marshal `main` | `94ecc24` (v0.54.1) |
| Merge base | `9b71fd6` |
| Muster-only commits | 193 (`git rev-list --count 94ecc24..ea472f6`) |
| Marshal-only commits | 37 (`git rev-list --count ea472f6..94ecc24`) |
| Muster first-parent entries | 83: five direct commits and PRs #1–#78 (#28 was never merged) |
| Already upstream | none: `git cherry 94ecc24 ea472f6` matches no patch, and searches of upstream for each feature (asyncRewake, the hook wait, idempotent send, connect-any, SIGTERM handling, the gate) find nothing |

The tables below go by first-parent entry, since one entry is one reviewed
unit. The commits inside a merged PR travel with it.

## What upstream changed since the base

Muster's changes conflict with these. Each one has a code that the
"Conflicts" column uses.

| Code | Upstream change | Files it touches that Muster also changed |
|---|---|---|
| **U-RR** | #161 `response_required` message metadata (`bcd107e`) | `entities/src/{message,send_message,broadcast_message,read_messages}.rs`, `daemon/src/hooks.rs` (the inbox line becomes `- from X [response_required=…]:`), `daemon/src/cleanup.rs` (new field on a `Message` literal), `shim/src/tools.rs` (tool schemas), `shim/src/codex_bridge.rs`, all three TS plugins, daemon e2e tests |
| **U-CG** | #158 shared coordination guidance: the `COORDINATION_GUIDANCE` / `PEER_MESSAGE_NOTICE` consts (`a1791be`, `ee9f8d0`, `302eb36`, `7620e09`, `f26b7f5`, `59ca807`) | `daemon/src/main.rs` (MCP instructions become `format!`), `daemon/src/hooks.rs` (inbox notice), `shim/src/main.rs` (instructions), `shim/src/codex_setup.rs` (AGENTS block), `shim/src/tools.rs` (tool descriptions), `.github/workflows/ci.yml` |
| **U-SB** | #159, #160, #162 sandbox and "Marshal Zoo" UI | `entities/src/lib.rs` (`mod sandbox`), `ui/src/lib.rs`, new `scripts/sandbox/`, `daemon/tests/sandbox_import.rs` |
| **U-MY** | #157 Myko stack 6.9.0 → 6.11.5 | `Cargo.toml`, `Cargo.lock` |
| **U-WIN** | #155, #156 Windows Codex app-server discovery | `shim/src/codex_bridge.rs`, `shim/src/codex_setup.rs` |
| **U-UI** | #163 secure WebSocket behind HTTPS | `ui/src/lib.rs` |
| **U-REL** | v0.53.7 to v0.54.1 version bumps | every `Cargo.toml`, plugin `package.json`, `plugin.json`, `Cargo.lock` |

In general, U-RR and U-CG conflict for real: both sides edit the same
lines. U-SB and U-REL conflict only on adjacent lines (module lists and
version strings). U-MY is a real conflict for one entry, #72, which carries a
patched copy of myko-server 6.9.0.

## Target upstream PRs

| Bucket | What upstream gets |
|---|---|
| **Hooks** | No-flag hook delivery for Claude Code: SessionStart/UserPromptSubmit/PostToolUse inbox hooks, the held `wait` hook and Monitor watcher, the `asyncRewake` waiter, wake-watcher reporting on the roster, and the fixes on that path |
| **Gate** | Optional TLS gate and per-machine pairing: the gate, session binding, the WebSocket filter, host/operator attestation, the local token and local gate, the myko request filter, single-use pairing codes, rate and connection limits, and the threat model. It should be off by default so a plain upstream install behaves as it does today |
| **Durability** | Mail and session durability: ack only after hand-off, idempotent send retry, keeping a waiting session, keeping mail to a reaped session, graceful SIGTERM, read-state for senders, non-durable wake watchers |
| **Other** | Small standalone fixes and features: terminal send, the read-only roster command, the stale operator-room fix, the sweeper reason, the dependency audit |
| — | Drop: Muster-only content (renames, board/roadmap/Rally process, team notes, Muster's README and branding, the crates.io release under `muster-*` names, the Murph rollout scripts) |

## Inventory

Decision: **port**, **drop**, or **split** (port part, drop the rest; the
cell says which part).

| # | Entry | What it does | Decision | Upstream PR | Conflicts expected |
|---|---|---|---|---|---|
| 1 | `37731f2` (direct, pre-fork) | `claude-hook session-start\|prompt-submit\|post-tool-use` plus `hooks.json`. On macOS it reads the parent's command line with `ps`, so a session without the flag reports `channels_enabled=false` and keeps its mail unread. Written under Marshal names; it fixes upstream issue #139 | port | Hooks (first commit) | `shim/src/main.rs` U-CG (instructions block nearby); otherwise clean, as it predates the rename |
| 2 | `8cea618` (direct) | Fork: rename everything marshal→muster, ports 6155/6156→6255/6256, licences, attribution | drop | — | Touches 116 files; the inverse is the rename list below |
| 3 | `989e4d9` (direct) | Docs: make the hook inbox Muster's default delivery path | split: the plugin README hook section | Hooks | `plugin.json` U-REL |
| 4 | `5c05690` (direct) | TLS gate with per-machine pairing (`gate.rs`, `credentials.rs`, `pairing.rs`, `link.rs`, `muster-shim pair`) | port | Gate (foundation) | `Cargo.toml/lock` U-MY, U-REL; `daemon/src/main.rs` U-CG; `shim/src/codex_setup.rs` U-CG, U-WIN; `entities/src/lib.rs` U-SB; plugin `index.ts` U-RR |
| 5 | `7d2c5d7` (direct) | CI on GitHub-hosted runners; drop upstream's release and public-CI pipelines | drop | — | `ci.yml` U-CG |
| 6 | `6d55631` (direct) | The sweeper logs why it reaps a session | port | Durability (folded into port/durability: woven through the same sweep code as #51) | `cleanup.rs` U-RR (adjacent lines) |
| 7 | #1 `58f6ec2` | Live inbox without the flag: the `/hook/wait` held request and Monitor watcher; the shim tries every resolved address; roadmap doc | split: code (roadmap is Muster-only) | Hooks | `hooks.rs` U-RR, U-CG; `shim/src/main.rs` U-CG |
| 8 | #2 `a24b92f` | Board process and story IDs | drop | — | none |
| 9 | #3 `77fd01d` | Board study findings | drop | — | none |
| 10 | #4 `969d6d1` | Keep long messages whole through the Monitor watcher | port | Hooks | none |
| 11 | #5 `884e4f8` | One-shot watcher, no time limit, no idle cost | port | Hooks | `hooks.rs` U-RR; `shim/src/main.rs` U-CG |
| 12 | #6 `7270624` | Wake idle Claude Code sessions with an `asyncRewake` waiter hook | port | Hooks | `hooks.rs` U-RR; `shim/src/main.rs` U-CG |
| 13 | #7 `92410d5` | Roadmap: Rust-first decision | drop | — | none |
| 14 | #8 `1c42886` | Roadmap: research story for lost work | drop | — | none |
| 15 | #9 `e842ccf` | Roadmap: muster ledger story | drop | — | none |
| 16 | #10 `e4b0f35` | Roadmap/board: track records | drop | — | none |
| 17 | #11 `63ac8be` | `docs/session-binding.md` design | split: the design doc, stripped of board refs | Gate | none |
| 18 | #12 `e63cab7` | Refuse hook requests for another machine's session | port | Gate | `daemon/src/main.rs` U-CG |
| 19 | #13 `8765e47` | Filter what paired machines send over the WebSocket (`gate_filter.rs`) | port | Gate | `Cargo.lock` U-MY |
| 20 | #14 `55f61c6` | README refresh and Muster banner | drop | — | none |
| 21 | #15 `09172f1` | Docs accuracy pass (install, plugin READMEs, `codex-live-delivery.md`) | split: recheck each fix against upstream docs; most are about Muster | Other | `codex-live-delivery.md` (upstream rewrote 75 lines) |
| 22 | #16 `c606485` | Attest paired machines' host and operator; close gate bypasses; keep free-text room names out of mention banners | port | Gate (the mention-banner fix can stand alone in Other) | `hooks.rs`, `send_message.rs`, `broadcast_message.rs` U-RR |
| 23 | #17 `7bcc1d9` | Roadmap: operator terminal messaging | drop | — | none |
| 24 | #18 `e24fbb2` | Roadmap: team name story | drop | — | none |
| 25 | #19 `12a603f` | `muster-shim send`: message the muster from a terminal | port | Other | `shim/src/main.rs` U-CG; `tools.rs` U-RR, U-CG |
| 26 | #20 `5024ae4` | A terminal message is never the operator's approval (wording) | port with #19 | Other | none |
| 27 | #21 `7e7869b` | Say why a watcher can't reach the daemon | port | Hooks | `daemon/src/main.rs` U-CG |
| 28 | #22 `865f01b` | Ack mail only after the client has handed it on | port | Hooks (moved from Durability: the waiter's `ack=client`, `/hook/ack` and `X-Muster-Ack` come from here) | `hooks.rs` U-RR |
| 29 | #23 `58c584a` | Tell a Monitor watcher about a refusal on stdout | port | Hooks | none |
| 30 | #24 `d85b289` | Gate hardening (rest of the review); keep a session's os/arch across a wait hook | port | Gate (the os/arch part goes with Hooks) | `hooks.rs`, `message.rs`, `send_message.rs`, `broadcast_message.rs` U-RR |
| 31 | #25 `665d6c1` | Gate e2e: clear mail and the open WS after unpair | port | Gate | none |
| 32 | #26 `ca56d8a` | Gate e2e: check the connection is open before unpairing | port | Gate | none |
| 33 | #27 `38923b3` | Roadmap: structured messages | drop | — | none (U-RR is upstream's own take on this) |
| 34 | #29 `ab14100` | Roadmap: CI runs the gate e2e | drop | — | none |
| 35 | #30 `bf9e487` | Remove sessions from their old operator rooms | port | Other | none |
| 36 | #31 `8688659` | README/roadmap/board refresh | drop | — | none |
| 37 | #32 `48a6243` | `idempotency_key` lets a timed-out send be retried without a duplicate | port | Durability | `send_message.rs`, `tools.rs`, daemon tests U-RR (both add a field to `SendMessage` and the tool schema); `ui/src/console.rs` U-RR |
| 38 | #33 `c0ece8c` | Test: a lost send reply can be retried | port with #32 | Durability | `send_message.rs` U-RR |
| 39 | #35 `68914b4` | `docs/agent-disappearance.md`: how work gets lost; mail goes at restart | drop for now: Muster process notes; the port/durability commit message carries the cases instead | — | none |
| 40 | #34 `cc421d4` | Split work tracking out into Rally | drop | — | none |
| 41 | #36 `af5c582` | Roadmap ticks | drop | — | none |
| 42 | #37 `dc68b67` | Re-register when our roster row has no client binding | port | Hooks | `shim/src/main.rs` U-CG |
| 43 | #38 `dd85975` | Roadmap: watchman role | drop | — | none |
| 44 | #39 `de51888` | Roster shows whether a message would wake a session (`WakeWatcher`); keep reporting a waiter when an older one drops | port | Hooks | `cleanup.rs`, `hooks.rs` U-RR; `entities/src/lib.rs` U-SB; `shim/src/main.rs` U-CG; `tools.rs` U-RR, U-CG |
| 45 | #41 `c45a973` | Board: watchman merge rules | drop | — | none |
| 46 | #42 `bae5cc0` | Stop reporting a wait once it has handed out mail | port | Hooks | `hooks.rs` U-RR |
| 47 | #40 `8b094dd` | `muster-shim roster`: read-only roster for other programs | port | Other | `shim/src/main.rs` U-CG |
| 48 | #43 `293c61e` | Don't persist wake watchers | port | Hooks (with #39) | `daemon/src/main.rs` U-CG |
| 49 | #44 `5565b10` | Test: a reaped session's mail goes at the next boot | port | Durability | `cleanup.rs` U-RR |
| 50 | #45 `c3dbd45` | Truncation notice names a working address | port | Hooks | `hooks.rs`, `hook_inbox_e2e.rs` U-RR (same function as the inbox line) |
| 51 | #46 `f31ff6b` | README front page; `SECURITY.md`; install doc | split: `SECURITY.md` content goes with Gate; README drop | Gate | none |
| 52 | #47 `e77506d` | Roadmap: MUS-54 | drop | — | none |
| 53 | #49 `0be702e` | Roadmap ticks | drop | — | none |
| 54 | #50 `2635b0e` | `scripts/restart-preview.sh`: what the next restart removes | drop (Muster ops; offer separately if upstream wants it) | — | none |
| 55 | #51 `2fbee42` | Keep a working session when its connection drops | port | Durability | `cleanup.rs`, `send_message.rs`, `send_message_command.rs` U-RR |
| 56 | #48 `3f8553e` | crates.io 0.1.0 under `muster-*`: crate READMEs, AGPL disclosure, rust-version 1.95, chacha20 0.10.1→0.10.2 (yanked) | split: AGPL disclosure, yanked-crate bump; drop the rest | Other | every `Cargo.toml` U-REL, U-MY; `Cargo.lock`; `ci.yml` U-CG; entity files U-RR; `codex_setup.rs` U-CG |
| 57 | #53 `ac07b69` | Roadmap ticks | drop | — | none |
| 58 | #52 `8bf207c` | Tests: keep a refusing port reserved in the `connect_any` tests | port with #1 | Hooks | `Cargo.lock`, `shim/Cargo.toml` U-REL |
| 59 | #54 `5a19735` | Comment: clock skew in the kept-session check | port with #51 | Durability | `cleanup.rs` U-RR |
| 60 | #55 `b92523a` | Roadmap: MUS-55 | drop | — | none |
| 61 | #56 `1e4d0e0` | End held waits cleanly on SIGTERM | port | Hooks (moved from Durability: the waiter handles its `X-Muster-Restarting` reply) | `daemon/src/main.rs` U-CG |
| 62 | #57 `cd7fc14` | Keep mail to a reaped session until the 14-day sweep | port | Durability | `cleanup.rs`, `message.rs` U-RR |
| 63 | #58 `0e76849` | restart-preview fixes | drop | — | none |
| 64 | #60 `74385c5` | `scripts/rollout.sh`: roll out a daemon commit with rollback | drop (Muster ops) | — | none |
| 65 | #59 `e797fa3` | Show a sender whether its mail was read before the recipient went | port | Durability | `read_messages.rs` U-RR (both add a field to the read row); `entities/src/lib.rs` U-SB |
| 66 | #61 `be8ec58` | README: rollout limits | drop | — | none |
| 67 | #62 `f85e12b` | Roadmap ticks | drop | — | none |
| 68 | #63 `a3511c5` | Team summary 2026-11-03 | drop | — | none |
| 69 | #64 `6cc83bc` | Stop tracking team notes | drop | — | none |
| 70 | #65 `b43b035` | Roadmap: security first | drop | — | none |
| 71 | #66 `61ee4c1` | Roadmap: MUS-58 | drop | — | none |
| 72 | #67 `cc2c109` | Roadmap: verifications | drop | — | none |
| 73 | #68 `3746864` | Local token on the hook listener; local gate in front of myko (new internal port) | port | Gate | `daemon/src/main.rs`, `shim/src/main.rs` U-CG |
| 74 | #69 `6173f5f` | Keep pairing secrets out of sessions (redaction) | port | Gate | `daemon/src/main.rs`, `shim/src/main.rs` U-CG; `send_message.rs`, `broadcast_message.rs` U-RR |
| 75 | #70 `a054c58` | rollout.sh checks myko's own port | drop | — | none |
| 76 | #71 `200d436` | Roadmap: MUS-59 findings | drop (the analysis can go in the Gate PR description) | — | none |
| 77 | #72 `14e2d1a` | Myko's own listener admits only what a gate forwards: a vendored, patched myko-server 6.9.0 (`RequestFilter`) | port, but rework: rebase the patch on 6.11.5 or, better, upstream a request-filter hook to myko-server first | Gate | **U-MY, hard**: upstream moved to myko 6.11.5, and the vendor dir and `[patch.crates-io]` pin 6.9.0. Also `Cargo.toml/lock`, `daemon/src/main.rs` U-CG |
| 78 | #73 `db7d3c3` | Single-use, expiring pairing codes | port | Gate | `daemon/src/main.rs`, `shim/src/main.rs` U-CG |
| 79 | #74 `22df549` | Rate limits per session and connection limits per listener (`limits.rs`, `rate.rs`) | port | Gate | `send_message.rs`, `broadcast_message.rs` U-RR; `entities/src/lib.rs` U-SB |
| 80 | #75 `821c2ac` | Share the message allowance across a machine's sessions | port with #74 | Gate | `send_message.rs`, `broadcast_message.rs` U-RR |
| 81 | #76 `5f0f5f0` | Operator can pause wake-ups and limit who may send (`controls.rs`, `wake_pause.rs`) | port | Gate (sender allow-list), or Other if upstream wants it without the gate | `Cargo.lock`, `daemon/Cargo.toml` U-MY, U-REL; `daemon/src/main.rs`, `shim/src/main.rs` U-CG; `send_message.rs`, `broadcast_message.rs` U-RR; `tools.rs` U-RR, U-CG; `entities/src/lib.rs` U-SB |
| 82 | #77 `586fa75` | README: the operator's opening | drop | — | none |
| 83 | #78 `ea472f6` | `cargo-deny` audit in CI (`deny.toml`); `docs/threat-model.md` | split: `deny.toml` + CI step to Other; threat model with Gate | Other / Gate | `ci.yml` U-CG; `Cargo.lock` U-MY |

### Counts

| Decision | Entries |
|---|---|
| port (whole, or "with" another) | 40 |
| split | 8 |
| drop | 35 |

| Upstream PR | Entries |
|---|---|
| Hooks | 1, 3, 7, 10–12, 27, 28, 29, 42, 44, 46, 48, 50, 58, 61 (and the os/arch part of 30) |
| Gate | 4, 17–19, 22, 30–32, 51, 73, 74, 77–81, part of 83 |
| Durability | 6, 37, 38, 49, 55, 59, 62, 65 |
| Other | 21, 25, 26, 35, 47, part of 56, part of 83 |

Suggested order: Hooks, then Durability, then Gate (the largest; #72 waits
on myko), then Other. Each port is its own branch on upstream main
(`94ecc24`, or wherever upstream is when it is built), carrying only that
bucket's entries under Marshal names. Where an entry collides with U-RR or
U-CG, the branch reuses the resolution from modest-roach's G1 catch-up
merge rather than resolving the conflict a second time.

**Hooks (port/hooks-v3, `3202b04`).** port/hooks-v2 (`dcbb316`) plus one
commit: MUS-75 (#99, `71483ca`), so a prompt hook acks its mail only after
printing it (`ack=client`, `X-Marshal-Ack`), as the waiter does. Its gate
branch and Muster doc change are not ported. This is the G8 candidate, since
it's what goes upstream. port/hooks-v2 is kept as it was.

**Durability (port/durability-v2).** It's stacked on port/hooks-v3, not on
upstream main. port/durability-v2 is port/durability (`51d0ec4`) rebased onto
hooks-v3, with `idempotency_key: None` added to the new hook_client_ack test. #51 keeps a dropped session that still holds a `WakeWatcher`,
and #57's test lives in `hook_wait_e2e.rs`, so both need Hooks. That matches
the landing order. Entry 6 (`6d55631`) is folded in, because its reap reasons
run through the same sweep code. Entry 39's doc is not ported.

**Other (port/other-v3).** It's stacked on port/durability-v2, so it lands
after Durability. (port/other-v3 is port/other-v2 rebased onto it, unchanged.) The roster command reads `WakeWatcher` and `wake_state` from Hooks,
and terminal send builds a `SendMessage`, which has Durability's
`idempotency_key` field. (The first cut, port/other, sat on port/hooks-v2 and
didn't compile once merged with Durability.) One commit per item, so the ones
that need neither (35, 21, part of 56, part of 83) can be cherry-picked onto
upstream main on their own:

- 35 (#30): old operator rooms, as is.
- 25, 26, 47 (#19, #20, #40): terminal send and the roster command, in one
  commit, since they share the shim's `main.rs` and README changes. Without the
  gate nothing is attested, so roster rows have `observed` and `declared`
  only, with machine and operator under `declared`. No `wakes_paused`.
- 21 (#15): only the plugin README's two upstream errors (nicknames, and
  `mcpServers` in `settings.json`). The rest is about the gate, ports or
  releases.
- Part of 56 (#48): the AGPL disclosure in the README. The chacha20 bump goes
  with 83's lockfile updates.
- Part of 83 (#78): `deny.toml` and the CI job. On upstream's lockfile the
  audit found the same advisories #78 fixed, so the same in-range updates are
  applied, changing only those lock entries.

The #16 mention-banner fix isn't on this branch.

**Correction (port/hooks prep).** The Hooks entries can't be cherry-picked
cleanly onto upstream: they are entangled with the Gate.

- `crates/shim/src/link.rs` was created by the gate commit `5c05690`. The
  hooks then added `connect_any`, `PostFailure` and `Reply` to it.
- At `ea472f6` the hook code also calls pairing (`https_post`), the local
  token, `machine_may_act`, `ConnLimiter` (#74), `wakes_paused` (#76) and
  `attested()` (#16).

The port therefore renames each commit with `scripts/rename-to-marshal.sh`,
cherry-picks it onto upstream main, and drops the gate hunks as it goes. The
PR lists every dropped hunk. Entries 28 (#22) and 61 (#56) moved into Hooks
because the waiter depends on both.

## Renames to restore for upstream

`8cea618` did these. Anything ported must undo them, and Muster-only names
added since the fork need Marshal equivalents.

### Binaries, crates, packages

| Muster | Marshal |
|---|---|
| `muster-daemon`, `muster-shim`, `muster-tui`, `muster-ui` (crates and bins) | `marshal-daemon`, `marshal-shim`, `marshal-tui`, `marshal-ui` |
| `muster-entities` crate / `muster_entities` path | `marshal-entities` / `marshal_entities` |
| `muster_ui` lib | `marshal_ui` |
| `plugins/muster-{shim,opencode,pi,prime-agent}` | `plugins/marshal-{shim,opencode,pi,prime-agent}` |
| npm packages without a scope | `@agent-marshal/*` (upstream's scope) |
| crate versions `0.1.0` (Muster's crates.io line) | upstream's `0.54.x` |
| `muster-shim.cmd` / `.exe` (Windows shims) | `marshal-shim.cmd` / `.exe` |

### Ports

| Listener | Muster | Marshal |
|---|---|---|
| WebSocket (public; the TLS gate when one is set) | 6255 | 6155 |
| Hook HTTP | 6256 | 6156 |
| Internal plain WS behind the gate (`MUSTER_INTERNAL_BIND`; shim default `ws://localhost:6257`) | 6257 | none upstream; propose 6157 |
| Myko's own listener (`MUSTER_MYKO_BIND`, #68/#72) | 6258 | none upstream; propose 6158 |

The 6157/6158 numbers are a proposal that follows the +100 shift. Upstream
has to agree to them.

### Environment variables

These are `MUSTER_*` → `MARSHAL_*`. The ones marked new do not exist upstream
and arrive with the PR named.

| Muster | Marshal | |
|---|---|---|
| `MUSTER_BASE_URL`, `_BIND`, `_BIN_DIR`, `_CODEX_APP_SERVER_ENDPOINT`, `_CODEX_APP_SERVER_SOCKET`, `_CODEX_BIN`, `_DAEMON`, `_DAEMON_ADDRESS`, `_DAEMON_BIN`, `_HARNESS`, `_HOOK_BIND`, `_HOST`, `_OPERATOR`, `_STATE_DIR` | same with `MARSHAL_` | existing |
| `MUSTER_INTERNAL_BIND`, `MUSTER_MYKO_BIND` | `MARSHAL_INTERNAL_BIND`, `MARSHAL_MYKO_BIND` | new, Gate |
| `MUSTER_MAX_CONN_HOOKS`, `_MAX_CONN_LOCAL`, `_MAX_CONN_PER_IP`, `_MAX_CONN_PER_MACHINE` | `MARSHAL_…` | new, Gate (#74) |
| `MUSTER_ACK_LEASE_SECS` | `MARSHAL_ACK_LEASE_SECS` | new, Durability (#22) |
| `MUSTER_WAITER_LOCK_WAIT_SECS` | `MARSHAL_WAITER_LOCK_WAIT_SECS` | new, Hooks (#6) |
| `MUSTER_UNIT` | none: used only by the Muster ops scripts | drop |

Upstream-only variables Muster dropped or never had: `MARSHAL_CODEX_APP_SERVER_CONNECTION_FILE` (U-WIN), and `MARSHAL_EVAL_*`, `MARSHAL_CHROMIUM_PATH`, `MARSHAL_PLAYWRIGHT_MODULE`, `MARSHAL_UI_SCREENSHOT_DIR`, `MARSHAL_TEST_SHIM` (U-SB). Keep them as they are.

### Config and state paths

| Muster | Marshal |
|---|---|
| `$XDG_CONFIG_HOME/muster`, `~/.config/muster`, `%APPDATA%\muster` (shim config, `daemon-address`) | `…/marshal` |
| `~/.config/muster/pairing` | `~/.config/marshal/pairing` (new, Gate) |
| daemon state dir `<data dir>/muster` (`credentials.rs`) | `<data dir>/marshal` |
| Codex bin dir `<base>/muster/bin` | `<base>/marshal/bin` |
| Codex `config.toml` section `[mcp_servers.muster]` | `[mcp_servers.marshal]` |

### Protocol-visible names

| Muster | Marshal |
|---|---|
| resource scheme `muster://` | `marshal://` |
| inbox / identity tags `<muster_inbox>`, `<muster_session>` | `<marshal_inbox>`, `<marshal_session>` |
| MCP server name `muster`; tools `muster_send_message`, `muster_broadcast`, `muster_roster`, `muster_whoami`, `muster_messages`, `muster_ack`, `muster_join_room`, `muster_leave_room`, `muster_set_status` (non-Claude plugins) | `marshal`, `marshal_*` |
| channel name `muster-channel`; status line `muster-statusline`, `muster-status`; `muster-inbox`, `muster-codex-bridge`, `muster-wake-canary` | `marshal-*` |
| UI local-storage keys `muster-ui-operator`, `muster-ui-layout`, `muster-ui-console-id` | `marshal-ui-*` |
| `--dangerously-load-development-channels` match on `muster` (`shim/src/channels.rs`) | match on `marshal` |
| Repository links `Castor-o7/muster` | `ignition-is-go/marshal` |

### Not renames, but drop as well

- `vendor/myko-server/MUSTER-PATCH.md`: rename it, or drop it once myko
  takes the filter upstream (entry 77).
- Muster's `LICENSE-*` files and attribution note: upstream has its own.
- `docs/board.md`, the board parts of `docs/roadmap.md`, `docs/muster/`,
  `scripts/rollout.sh`, `scripts/test-rollout.sh`, `scripts/restart-preview.sh`.

## How this was made

```sh
git fetch origin
git fetch https://github.com/ignition-is-go/marshal.git main   # FETCH_HEAD = 94ecc24
git merge-base origin/main 94ecc24                             # 9b71fd6
git rev-list --left-right --count origin/main...94ecc24        # 193  37
git log --first-parent 9b71fd6..origin/main                    # the 83 entries
git cherry 94ecc24 origin/main                                 # no '-' lines: nothing upstream
git diff --name-only 9b71fd6 94ecc24                           # upstream's files, mapped marshal→muster
```

The conflict column compares each entry's files with upstream's files, then
reads upstream's hunks in each shared file to say whether they would really
collide. It predicts; it was not tested with a trial merge. modest-roach's G1
catch-up merge will show the real conflicts for the tree as a whole.
