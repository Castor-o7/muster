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
| Internal plain WS behind the gate (`MUSTER_INTERNAL_BIND`) | 6257, retired in MUS-70 | none; the port/gate series drops it |
| Myko's own listener (`MUSTER_MYKO_BIND`, #68/#72) | 6258 | none upstream; propose 6158 |

The 6157/6158 numbers are a proposal that follows the +100 shift. Upstream
has to agree to them.

### Environment variables

These are `MUSTER_*` → `MARSHAL_*`. The ones marked new do not exist upstream
and arrive with the PR named.

| Muster | Marshal | |
|---|---|---|
| `MUSTER_BASE_URL`, `_BIND`, `_BIN_DIR`, `_CODEX_APP_SERVER_ENDPOINT`, `_CODEX_APP_SERVER_SOCKET`, `_CODEX_BIN`, `_DAEMON`, `_DAEMON_ADDRESS`, `_DAEMON_BIN`, `_HARNESS`, `_HOOK_BIND`, `_HOST`, `_OPERATOR`, `_STATE_DIR` | same with `MARSHAL_` | existing |
| `MUSTER_MYKO_BIND` | `MARSHAL_MYKO_BIND` | new, Gate (`MUSTER_INTERNAL_BIND` was retired in MUS-70) |
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

## Since the baseline: entries #79 to #124

Added 2026-11-05 by tangerine-wombat, against Muster `3fafe4a` (#124). The 42
first-parent entries since `ea472f6`, with the same decisions as above. "Layer" is the
port branch the entry belongs on (see [upstream-pr-strategy.md](upstream-pr-strategy.md)
for the stack). "In the stack" says whether a port branch carries it yet.

| PR | Entry | What it does | Decision | Layer | In the stack |
|---|---|---|---|---|---|
| #79 | `4f68283` | This inventory (G2) | drop | — | — |
| #80 | `d82a867` | `docs/hook-delivery.md`: hooks deliver and wake without the flag (G4) | split: the doc, without board refs | Hooks | no |
| #82 | `7ad45e2` | TLS-as-option design draft (G3) | split: the design, for the Gate PR description | Gate | n/a (doc) |
| #83 | `9ab6ee3` | Zoo plan for the Hooks change (G8) | drop (it plans our evidence) | — | — |
| #86 | `0eaa45f` | `scripts/rename-to-marshal.sh`; #22/#56 move into Hooks | drop (porting tool) | — | — |
| #84 | `900fe1c` | Catch-up merge with Marshal v0.54.1 (MUS-62); myko-server vendored at 6.11.5 | drop (it's the base) | — | — |
| #85 | `1bd36b2` | Roadmap: Phase 5 goals | drop | — | — |
| #81 | `d9c35d3` | `docs/muster-vs-marshal.md` (G7) | drop (feeds the PR descriptions) | — | — |
| #89 | `d243941` | TLS gate optional, plain by default (MUS-64, G3) | port, with the operator's `--tls`/`--plain` and no variable | Gate | yes (`a5de77c`) |
| #90 | `07f13be` | Design for messaging between daemons (MUS-66) | split: with the links PR | later | no |
| #91 | `ac1f849` | Daemon links (MUS-66, G5) | port if upstream wants it | later (after Gate) | no |
| #95 | `5e22a8f` | Hold first writes until registered; refuse an empty recipient (MUS-72, MUS-73) | port | Other | yes (`d9124c1`, `d403770`) |
| #98 | `2b8159e` | Board: who verifies with no watchman | drop | — | — |
| #94 | `1f914db` | Conformance suite stage 1: Claude, plain and TLS (MUS-69) | port if upstream wants it | later | no |
| #101 | `f5005dd` | Board: Rally and Muster are optional | drop | — | — |
| #99 | `0364722` | A prompt hook acks only after printing (MUS-75) | port | Hooks | yes (`3202b04`) |
| #102 | `9c74679` | Re-send the session if it isn't registered 1 s into a write (MUS-78) | port | Other | yes (`55c5868`) |
| #96 | `59db0a0` | Conformance stage 2: Codex; MUS-74–76 found | port with #94 | later | no |
| #103 | `9bb206a` | MUS-78's re-send gets a daemon-free test | port with #102 | Other | yes (`hold_until` and its tests, in `5717e22`'s layer) |
| #97 | `14c2b25` | `wake: expected` only when a waiter can wake (MUS-71) | port | Hooks | **no**: forward-port |
| #104 | `4199d2e` | Retry once a write refused as unregistered (MUS-79) | port | Other | yes (`5717e22`) |
| #105 | `edf5ea8` | Roadmap: file MUS-80 | drop | — | — |
| #100 | `7e75534` | The Codex bridge starts no turns while wakes are paused (MUS-74) | port (needs `WakePause`) | after Gate | no |
| #106 | `97b0f2c` | Conformance stage 3a: Pi; MUS-81/82 found | port with #94 | later | no |
| #107 | `bb1b164` | Roadmap: tick MUS-66 | drop | — | — |
| #109 | `61c94fd` | `unpair` can't revoke a machine and a link together (MUS-84) | port with #91 | later | no |
| #108 | `deae551` | Conformance stage 3b: opencode | port with #94 | later | no |
| #112 | `bdd2e70` | Roadmap: tick MUS-71/75/78 | drop | — | — |
| #114 | `e02f826` | Roadmap: file MUS-85 | drop | — | — |
| #92 | `6a89e29` | Docs: the operator chose `--tls` for upstream, `--plain` as the escape | split: the decision goes in the Gate PR description | Gate | yes (in the code) |
| #115 | `d17448f` | A stable code for the unregistered refusal, and retry on it (MUS-80) | port | Other | **no**: the port matches the text; forward-port |
| #110 | `a97e274` | Board: the watchman keeps the rally moving | drop | — | — |
| #119 | `37ad00d` | Fail closed when the paired machines can't be read (MUS-85) | port | Gate | yes (`mode::has_pairings`) |
| #111 | `d44ef24` | Conformance stage 3c: Prime Agent, the grid complete | port with #94 | later | no |
| #118 | `24fcc2d` | A session can ack its own pushes (`acks_pushes`, MUS-81) | port if upstream wants it; overlaps lobo235's #140 | Durability or later | no |
| #113 | `61a8152` | Design: ack a push only once it's shown (MUS-81) | split: with #118 | Durability or later | no |
| #120 | `8549500` | Retire the 6257 local-gate alias (MUS-70) | port (already so: the port never had it) | Gate | yes |
| #116 | `d8e962f` | Roadmap: tick MUS-84, recount | drop | — | — |
| #121 | `ae8de9e` | CI: free runner disk, line tables, retry the bun download (MUS-87) | drop (Muster's CI); offer separately if upstream's CI hits the same | — | — |
| #117 | `ff2252a` | Say when the local token is refused (MUS-83) | port | Gate | **no**: forward-port |
| #123 | `9aa2862` | Pi acks a push only once it has taken it (MUS-81) | port with #118; overlaps lobo235's #132, #134, #136 | Durability or later | no |
| #124 | `3fafe4a` | Roadmap: file MUS-88 | drop | — | — |

Open in Muster as this is written and bound for upstream if they merge: #127 (MUS-86,
ack and re-offer logging; Hooks), #122 (MUS-76, Codex threads re-register after a
restart), #125 (MUS-77, the bridge's wake path on the roster) and #130 (MUS-18, Codex
through the gate; after Gate). #129 (MUS-91, Muster's Windows CI) is dropped like #121.

### Upstream's open PRs that overlap the ports

Upstream main is still `94ecc24`. These upstream PRs are open and touch the same files
as a port layer. None has been reviewed, and the fork PRs among them haven't run CI,
which needs a maintainer's "Approve and run".

| Upstream PR | Author | What | Overlaps |
|---|---|---|---|
| #154 | Castor-o7 | Deliver direct messages to Claude Code without the channels flag (2026-10-29) | Hooks: an earlier cut that `port/hooks-v3` supersedes |
| #141 | lobo235 | Pull the inbox into Claude Code turns with plugin hooks; escape bodies in the inbox block | Inbox-quoting and Hooks: the same goal and files |
| #140 | lobo235 | Keep a Claude push unread until the shim has written it | Hooks (`hooks.rs`, `send_message.rs`); MUS-81 |
| #148 | lucid-max | Keep messages and receipts across a session resume | Durability (`cleanup.rs`, `message.rs`, `message_read.rs`) |
| #131 | lobo235 | Read-state sticky across a session reload; ahead of ignition-is-go/myko#96 | Durability (the same files) |
| #135 | lobo235 | `--help`/`--version` for the daemon, shim and TUI | Gate's `--tls`/`--plain` and Other's terminal commands (`main.rs` argument handling) |
| #143 | lobo235 | Validate room names and descriptions | Other (`cleanup.rs`, the room tools) |
| #132, #134, #136, #137 | lobo235 | Pi, Prime Agent and opencode: ack only what was shown; flag truncated pushes | MUS-81's plugin work |
