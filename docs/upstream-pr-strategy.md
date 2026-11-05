# Upstream PR strategy: Muster into Marshal

How Muster's changes go back to [Marshal](https://github.com/ignition-is-go/marshal)
as pull requests for the ignition-is-go team: in what order, what each PR
contains and overlaps, what evidence it carries, and how the port branches
stay in step with Muster. The per-commit decisions are in
[upstream-inventory.md](upstream-inventory.md), and the text for each PR's
description is in [muster-vs-marshal.md](muster-vs-marshal.md).

## The next round, after #164 and #165 (2026-11-05)

**#164 (Hooks, as `port/hooks-2a`) and #165 (the AGPL notice) are merged upstream**,
by lucid-max, with a welcome from the ignition-is-go team. The operator approved this
plan on 2026-11-05. It supersedes the landing order below wherever the two disagree.

### What changed upstream

- Marshal `main` is **`9b00c6f` (v0.58.0, #166, about 16:30 EST 2026-11-05)**. It was `8362227`
  (v0.57.0) when this section was first written, 16 commits past `94ecc24`. Along with ours,
  lucid-max merged lobo235's #131, #132, #134–#137, #140, #141 and #147.
- Two of those overlap our layers. **#141** pulls the inbox into Claude turns with
  plugin hooks, which sits next to `port/inbox-quoting`. **#140, #131 and #136**
  cover keeping mail unread until it's shown, which is much of `port/durability-v2`.
- **How the review ran.** lucid-max first requested changes at the receiving
  boundary: `link::parse_response` accepted a body cut short of its Content-Length.
  After that, each pushed head got an independent run of the 240 package tests, a
  real-process takeover check, and native Windows CI on the exact head. A Windows
  fixture fix, setting `APPDATA` as well as `XDG_CONFIG_HOME`, came out of that.
  The version bumps are done upstream.
- **Overlaps that no longer apply.** lobo235's PRs that the layers below were
  measured against (#131, #132, #134–#137, #140, #141, #147) have merged, and our
  #154 is closed. They're upstream main now, so the rebase deals with them and
  they're not open PRs to coordinate with. Wherever the older sections below call
  #140 or #141 an open "overlap", read "already in main". **Only two upstream PRs
  are open:** lobo235's **#143** (validate room names and descriptions where rooms
  are created), which is near Other's empty-`to` and room code, and trevorsargent's
  **#59** (TUI session nicknames and agents-pane grouping), which is worth checking
  against our nickname and roster changes before M2.
- **What v0.58.0 (#166) overlaps.** It's mostly CI (`ci.yml` +80, `release.yml`,
  actionlint, workflow validation) plus embedding-agent metadata (#104's idea,
  `entities/src/session.rs` +52, `docs/embedding-agent-id.md`). It also edits
  `daemon/src/hooks.rs` (+54 −45), `shim/src/main.rs` (+118 −29),
  `shim/src/tools.rs` and `shim/src/codex_hook.rs`. So it touches **M2**
  (`shim/src/main.rs`), **Other's `cargo-deny` audit** (`ci.yml`, which now
  validates workflows and pins action references, so the audit job must follow
  those rules), the **Codex bridge items** (`codex_hook.rs`), and anything that
  adds Session fields (the Gate's machine fields). `docs/pr-followups.md` lists
  ideas upstream kept from **its own** closed PRs (#91, #113 and #125 are lucid-max's, and
  #104 is the pulse-agents app's), so it doesn't show how outside PRs are treated.
  Our #164 is the evidence for that: it merged as our 38 commits squashed into one
  (`f47c932`, with upstream's own merge-from-main `be59701` on the PR branch). So
  upstream does merge outside PRs, but squashed, which leaves every `port/*` branch
  on a base whose hooks commits aren't ancestors of upstream main (see "Live cuts").
- **Reference checkout:** `~/src/marshal` on Murph is a clean checkout of
  ignition-is-go/marshal main, with remotes `upstream` and `castor` (the
  Castor-o7/marshal fork that PR heads come from). Use it for upstream code.
  `~/src/muster` is Muster, not Marshal.
- Levi `main` is still `aed6027` (v0.9.5), so L1–L3 are current. Castor-o7/levi now
  exists as a fork of ignition-is-go/levi, which was what held L1.

### Live cuts (checked by cobalt-bream, 2026-11-05 ~16:45)

Every `port/*` branch is still on `94ecc24`. The live cut of each layer is the
newest head, and these form one stack, each containing the ones above it:

| Layer | Live cut | Head | Sits on | Superseded, kept, not to be used |
|---|---|---|---|---|
| Hooks (merged as #164) | `port/hooks-2a-offer` | `62552e6` | `94ecc24` | `hooks`, `hooks-v2`, `hooks-v3`, `hooks-ack`, `hooks-2a` (`7f0a56c`, a sibling of the offered head) |
| Inbox quoting (proposal only) | `port/inbox-quoting-2` | `3290e17` | hooks-2a-offer | `inbox-quoting` |
| Durability | `port/durability-v3` | `6e63bb1` | inbox-quoting-2 | `durability`, `durability-v2` |
| Other | `port/other-v4` | `0784ac2` | durability-v3 | `other`, `other-v2`, `other-v3` |
| Gate | `port/gate-v3` | `36a3c11` | other-v4 | `gate`, `gate-v2` |
| M2 | `port/terminal-cmds-v2` | `4a53e00` | hooks-2a-offer (**not** the stack above) | `terminal-cmds` |

The older cuts don't contain the newer ones. Each `-vN` was rebuilt, not extended. The
per-layer tables further down still name the older cuts. Read this table instead.

**`hooks-2a-offer` is in upstream as the squash `f47c932`.** Its commits will never be
ancestors of upstream main. So rebasing a layer means
`git rebase --onto upstream/main <layer below> <layer>`, never a plain rebase.

**M2 doesn't need Step 0.** `port/terminal-cmds-v2` has 10 commits above
`hooks-2a-offer`, but only three are M2: `481f18a` (`tell`/`all`/`roster`), `2211da4`
(plugin docs) and `4a53e00` (roster fix). The other seven are hooks follow-ups and the
Zoo campaign for #164. One of them, `7208485` (a body can't forge the inbox framing), is
the inbox-quoting default change, which goes to upstream as a proposal only. A trial
cherry-pick of the three onto `9b00c6f` had one conflict, in `shim/src/main.rs`.
Upstream's new `usage()` (`--help`, from #135) sits where M2 adds `run_terminal_send` and
`daemon_address`. The fix is to keep both and list `tell`, `all` and `roster` in
`usage()`. So M2 can be cut fresh on `9b00c6f` as `port/terminal-cmds-v3`, ahead of
and independent of Muster's catch-up. That's the recommendation, and it's ultra-dune's
call. The trial built nothing and pushed nothing.

### Step 0: catch up and re-inventory (nothing goes upstream)

1. Muster takes in v0.55–v0.57, the same way MUS-62 (#84) took in v0.54.
2. Rebase every live `port/*` branch onto upstream main (`9b00c6f` as of 16:30).
3. Check each remaining layer against what upstream merged, and drop, shrink or
   rework what they now cover. Inbox quoting may turn into a small change on top
   of #141, or disappear. Durability is cut down to what #140, #131 and #136 leave
   uncovered. Record each decision in [upstream-inventory.md](upstream-inventory.md).

### The new order: additive first (operator, 2026-11-05)

**Marshal: additive changes only for now.** That means new code nothing existing
calls, or code that runs only when someone opts in, so a Marshal user who changes
nothing sees no difference. Default changes aren't sent as PRs. They're listed for
upstream to choose (below). Which part is which comes from "Additive features vs
conflicts" in [muster-vs-marshal.md](muster-vs-marshal.md). Still one PR at a time,
each opened after the one before it has merged:

1. **M2, the terminal commands** (`marshal-shim tell`/`all`/`roster`;
   [upstream-pr-m2.md](upstream-pr-m2.md); the live cut is `port/terminal-cmds-v2`).
   New subcommands that nothing calls.
2. **Durability, the additive part only:** the optional `idempotency_key` on send,
   and read state for senders. It leaves out keeping mail for a reaped session and
   keeping a waiting session through a dropped connection, since both change
   defaults.
3. **Other, the additive part only:** the `cargo-deny` dependency audit, plus
   whatever of `marshal-shim send`/`roster` M2 doesn't already cover. It leaves out
   refusing an empty `to` and the 5 s first-write wait.
4. **myko first: the `RequestFilter` hook**, a small additive PR to ignition-is-go/myko.
   It's an optional filter that runs before myko handles a request, and nothing
   changes when none is set. It replaces vendoring myko-server 6.11.5. The Gate
   can't open until myko has released it.
   **The myko PR plan (commissioner, 2026-11-05 ~16:35):**
   - **The change.** Taken from `d4165f5`, minus the vendoring: about 37 changed lines in
     `libs/myko/server/src/router.rs` and `lib.rs`. It adds `router::RequestFilter`, an
     `Arc<dyn Fn(&HttpRequestHead, SocketAddr) -> Result<(), (u16, &str)>>` that runs on
     each parsed request head before any route, and `MykoServerBuilder::with_request_filter`.
     With no filter set, nothing changes. myko `main` (v6.11.6) has no changes in
     `libs/myko/server/src` since 6.11.5, so the patch applies as written.
   - **The overlap to settle first.** trevorsargent's **ignition-is-go/myko#129** (open,
     opened 2026-11-05, "authenticated gateways and persisted permissions") rewrites the
     same files: `router.rs` +195, `lib.rs` +34, `ws_handler.rs`, and `mcp/*`. It adds an
     opt-in `with_auth_provider` with a pluggable `Authenticator`. That could cover the
     gate's "401 without the token" check, as a bearer credential. It doesn't cover the
     "403 for a browser `Origin`" check, because `Credentials` carries only `bearer`. So
     opening our filter now would compete with #129 in the same lines. **Recommendation:**
     the operator asks the myko maintainers which they'd prefer: a small
     request-head filter on top of #129, or an `Origin` check (or the request head) made
     available to #129's gateway. Then build on #129 rather than race it. The Gate is
     several PRs away, so waiting costs nothing now.
   - **Decided (operator, 2026-11-05): trevorsargent leads.** We don't open our own
     filter PR. We follow #129: watch it, and when it merges, build the Gate on its
     `with_auth_provider`/`Authenticator` for the token check. If the browser-`Origin`
     check still needs the request head after #129 lands, that's a small follow-up
     built on #129, and only if trevorsargent and the myko maintainers want it. Any
     comment on #129 comes from the operator. Until #129 merges, the Gate waits, and
     the steps before it (M2, durability, Other) don't depend on it.
   - **Prerequisites.** A **Castor-o7/myko fork**, which doesn't exist yet. Creating it is
     the operator's action. A reference clone is at `~/src/myko`, where the `pr-129` branch
     is fetched. myko's rules (`AGENTS.md`): run checks with `cargo flux run`, run tests with
     `RUST_TEST_THREADS=1`, comments explain why, and TODOs carry initials. No version bump,
     since lucid-release does it.
   - **Then the Gate** bumps `myko-server` to the release that includes the hook, and drops
     `vendor/`, `[patch.crates-io]`, `MARSHAL-PATCH.md`, and the vendored-licence notes in
     `deny.toml` and the README.

5. **Gate**, in two or three PRs: TLS, then pairing, then the myko listener. It's
   additive while `--tls` isn't given, and plain mode stays the default. It depends
   on the released myko with the `RequestFilter` hook, and vendors nothing.
6. **Later, still additive:** daemon links (TLS only) and the conformance suite
   (test only).

**Not sent as PRs: the default changes.** They go upstream as **one issue**
(lucid-max's answer 3), drafted in [upstream-issue-defaults.md](upstream-issue-defaults.md).
The operator files it. The commissioner checked each item against `9b00c6f`:
- **Still real:** refusing an empty `to` (`resolve_recipient("")` prefix-matches every
  session, so with one session the message goes to its sender); the first-write
  hold; keep-alive while a waiter is held (the WS reap on `STALE_AFTER` doesn't
  check `WakeWatcher`); and MUS-76's Codex re-register, not yet traced in
  upstream's `codex_bridge`.
- **Optional:** inbox quoting. #141 already escapes bodies and frames them as
  untrusted.
- **Dropped:** keeping mail for a reaped session. Upstream already keeps DMs and read
  marks through the DEL, and only `MESSAGE_TTL` (14 days) removes them.

**Levi: on hold.** L1, L2 and L3 are already open upstream as ignition-is-go/levi
**#43** (the review gate), **#44** (`levi check-events`) and **#45**
(`levi session-start`). Nothing more goes to Levi until those have merged.

### The check before each PR, modelled on upstream's review

- Look for receiving-boundary gaps: short reads, Content-Length, framing, and acks
  sent for mail that was never shown.
- Rerun the tests on the exact head being sent, from a clean checkout, and keep the
  raw log.
- Windows CI green, with each fixture's config isolated (`APPDATA` as well as
  `XDG_CONFIG_HOME`).
- No version bumps in our PRs.

### lucid-max's answers (given in person to the operator, 2026-11-05)

The operator put the questions below to lucid-max in person. These answers settle
them, and they override anything the rest of this doc says:

| # | Question | Answer |
|---|---|---|
| 1 | One small additive PR at a time? | **Yes.** |
| 2 | Squash-friendly branches, or squash before opening? | **Yes to both**: keep branches squash-friendly, and squash before opening if necessary. |
| 3 | Default changes: one issue or one each? | **One issue** listing them all. |
| 4 | Wait for #143 and #59, or build on them? | **Build on them.** |
| 5 | The Gate: the split, a myko hook or vendored myko-server, port 6158 | **Not vendored** (the operator's decision, 2026-11-05, replacing an earlier "vendor" reading). The `RequestFilter` change goes upstream to **ignition-is-go/myko** as its own small additive PR first. The Gate then depends on the myko release that includes it, through an ordinary version bump in `Cargo.toml`, with no copy of myko-server in Marshal. The split and port 6158 are still open, so the TLS → pairing → myko-listener split stands as our proposal. |
| 6 | Zoo run for additive PRs with no agent-facing text? | **Not needed.** |
| 7 | Fork-PR Actions approval? | **Yes, but don't wait for it.** Open PRs without waiting for fork CI. |

What follows from these:
- M2 (`port/terminal-cmds-v3`) needs no Zoo run, and doesn't wait on fork CI.
  Squash it to one commit before opening if its history is noisy.
- The default changes go up as a single issue. Draft: [upstream-issue-defaults.md](upstream-issue-defaults.md) (owner:
  the commissioner).
- Other's room and `to` work and our roster and nickname work are built on top of
  #143 and #59, not held for them.

### For the operator to raise with lucid-max

A draft for the operator to send, in his own words:

1. **Thanks** for reviewing and merging #164 and #165.
2. **Shape.** Is one small additive PR at a time still what you want? Next is the
   `marshal-shim tell`/`all`/`roster` terminal commands, new subcommands only, about
   560 lines.
3. **Squash merges.** #164 merged as a squash. Should we keep our PR branches
   squash-friendly, or would you rather we squash before opening?
4. **Default changes.** We have behaviour changes we won't send as PRs: inbox
   quoting, keeping mail for reaped sessions, keep-alive for waiting sessions,
   refusing an empty `to`, a first-write wait, and Codex re-register after a
   restart. Would you like them as one issue listing them all, or one issue each? Or
   would you rather pick them up yourselves, as `docs/pr-followups.md` does with your
   own closed PRs?
5. **Overlap with open PRs.** #143 (room validation) and #59 (TUI nicknames) are
   near our Other and roster work. Should we wait for them, or build on them?
6. **The Gate.** Opt-in TLS (`--tls`), plain stays the default. How would you like it
   split (TLS, then pairing, then the myko listener)? Would you take a request-filter
   hook in ignition-is-go/myko first, rather than a vendored myko-server copy? And is
   port 6158 OK for myko's own listener?
7. **Zoo (G8).** Is a Zoo run still required for additive PRs that change no
   agent-facing text, such as the terminal commands and the dependency audit? It
   needs Codex on a1, which isn't installed.
8. **Fork CI.** Could fork-PR Actions be approved for our PRs, so Windows CI runs on
   the exact head?

### Who does it

ultra-dune takes Step 0 and the rebases (see "Who does what" below). The lamplighter deploys each catch-up
as it lands in Muster. The operator sends each PR, as before.

## Ground rules

These come from Phase 5 of [roadmap.md](roadmap.md) and hold for every PR:

- **The operator decides each upstream PR and sends it.** Nothing goes to
  ignition-is-go/marshal (or to the Castor-o7/marshal fork) until the Phase 5
  goals it depends on are met. Agents prepare the branches, the descriptions
  and the evidence, and stop there.
- **Marshal's names, not Muster's.** Each port is built with
  `scripts/rename-to-marshal.sh` on upstream main: `marshal-*` crates,
  `MARSHAL_*` variables, ports 6155/6156. The gate adds one listener, myko's
  own (proposed 6158). Local tools use 6155, so there's no 6157.
- **Nothing Muster-only.** No board, roadmap, Rally, watchman or rollout
  material, and no Muster branding (see "What Muster drops entirely" in
  [muster-vs-marshal.md](muster-vs-marshal.md)).
- **Upstream's assessment rule.** Marshal's `scripts/sandbox/README.md` asks
  for a real-agent simulation, imported into Zoo, after any change to
  Marshal's behaviour or agent-facing instructions: "Unit tests and a browser
  fixture do not replace that run." Each PR below gives its Zoo (G8, MUS-68)
  status.

## Where things stand (2026-11-05)

| | |
|---|---|
| Upstream `main` | was `94ecc24` (v0.54.1) when this table was written; now `9b00c6f` (v0.58.0), see "The next round" above |
| Muster `main` | `3fafe4a` (#124), caught up with `94ecc24` (MUS-62, #84) |
| Port stack | Branches in Castor-o7/muster, all on `94ecc24`, each stacked on the one before |
| Phase 5 goals | G1 (catch-up), G2 (inventory), G3 (optional TLS), G4 (hook delivery doc) and G5 (daemon links) are done. **G7** (MUS-67, the write-up) is this update. **G8** (MUS-68, the Zoo runs): the Hooks plan is written (`docs/zoo-plan-hooks.md`), but no run has happened. It is **blocked until Codex is installed on a1**, because the Zoo drives Codex agents only |

### How upstream is merging

This matters more than the state of our branches:

- **Every PR merged upstream since October is lucid-max's own, but one:** #144 by trevorsargent (2026-10-14), a dependency bump to Myko 6.8.0. No outside feature PR has been reviewed.
- **No outside PR has been reviewed yet.** lobo235's eleven open PRs (#131–#147) date from 2026-10-05 to 10-19.
  - Each was rebased onto v0.54.0 on 2026-11-04.
  - None has a maintainer comment.
  - None has run CI: fork PRs need a maintainer's one-time "Approve and run", or the repository setting "Allow GitHub Actions from fork pull requests".
- **Our own #154** ("deliver direct messages to Claude Code without the channels flag", 2026-10-29) is open, unreviewed, and older than the catch-up. It's a first cut of what is now `port/hooks-v3`.

So the bottleneck is maintainer attention and fork CI, not our branches. The
strategy has three consequences:

1. **The operator talks to upstream's lead developer before the series opens.**
   They should agree:
   - how upstream wants Muster's changes: one PR per layer, as here, or something else;
   - that fork-PR Actions are approved;
   - who reviews;
   - how to handle the overlap with lobo235's PRs (below).

   The lead developer has already said Muster merging is welcome, so this is a
   matter of process, not persuasion.
2. **Small first.** The first PR is 54 lines, so upstream can see how the
   series is built before the large ones arrive.
3. **Supersede, don't pile up.** When the Hooks PR opens, #154 is closed with a
   link to it, rather than kept alongside. We don't force-push or delete
   branches.

## The port stack

From upstream outwards. Each layer's own diff is against the layer below it,
with vendored code counted separately:

| Layer | Branch | Head | Commits | Own diff (not vendored) | Of which tests |
|---|---|---|---|---|---|
| 1 | `port/inbox-quoting` | `448671b` | 1 | +54 −2 in 2 files | 26 |
| 2 | `port/hooks-v3` | `3202b04` | 2 | +2255 −61 in 19 files | 860 |
| 3 | `port/durability-v2` | `bb741b4` | 2 | +885 −46 in 15 files | 269 |
| 4 | `port/other-v3` | `5717e22` | 9 | +1126 −26 in 11 files | 20 |
| 5 | `port/gate-v2` | `a5de77c` | 2 | +5815 −102 in 45 files, plus about 12,200 lines of vendored myko-server 6.11.5 (`91e57fc`, only the `RequestFilter` diff) | 1297 |

**Changed on 2026-11-05:** live communication goes first, so layer 2 is now
`port/hooks-2a`, straight on upstream main, ahead of inbox quoting, with the
hooks on by default. It's being finished by peppy-skate, and its size is given
in [upstream-pr-2a.md](upstream-pr-2a.md) once its head is settled. The layers
above it are rebased onto it.

The other `port/*` branches (`hooks`, `hooks-v2`, `hooks-ack`, `durability`,
`other`, `other-v2`, `gate`) are superseded cuts, kept because we don't
delete.

## The PRs, in order

**Landing order: hooks-2a → inbox-quoting → durability-v2 → other-v3 →
gate-v2**, with hooks-2a going first by the operator's decision of 2026-11-05. One PR per layer, each opened only after the one before has merged,
so upstream always reviews one diff against its own main. After each merge,
the rest of the stack is rebased onto the new upstream main.

### 1. Quote peer message bodies in the inbox (`port/inbox-quoting`)

| | |
|---|---|
| Size | +54 −2, 2 files, 1 commit |
| Depends on | upstream main only |
| Overlaps upstream | **lobo235's #141**, which also escapes bodies (`&`, `<`, `>` as entities) so they can't close the inbox block. That's the same goal in the same `hooks.rs` formatting. The description should say how the two differ (quoting against escaping) or offer to take #141's. It builds on upstream's own `PEER_MESSAGE_NOTICE` (U-CG) |
| Zoo (G8) | needed, since agent-facing text changes; blocked on Codex on a1. A short before/after run of an existing scenario will do |

Peer mail in the hook inbox is framed as quoted, untrusted text, so a body
can't pose as instructions or as another inbox line. It's first because it's
small, it stands alone, and it shows how the series is built.

### 2. Hooks: delivery and wake without the channels flag (`port/hooks-v3`)

**This is the operator's first priority, and it goes first: the
live-communication features, on by default.** The operator decided on
2026-11-05: "the hooks should be on by default ... otherwise the live delivery
is not reliable and reliability is our biggest priority."

It's built as `port/hooks-2a`, straight on upstream `94ecc24`, ahead of inbox
quoting. Its description, for the operator to send, is
[upstream-pr-2a.md](upstream-pr-2a.md).

"Additive features vs conflicts" in [muster-vs-marshal.md](muster-vs-marshal.md)
goes through it part by part. Almost all of it is additive. It makes two
default changes, and the description states both plainly, with how to turn
the hooks off:
- `hooks.json` is on in the plugin, so every plugin user gets the hooks;
- the macOS flag detection, which only changes sessions started without the
  flag, whose pushed mail was being lost.

On everything else it keeps upstream's behaviour. A hook listener that can't
bind is logged, and the daemon keeps running, as upstream's does.

| | |
|---|---|
| Size | +2255 −61, 19 files, 2 commits (860 lines of tests) |
| Depends on | layer 1 |
| Overlaps upstream | **our #154**, which this supersedes (close #154 with a link when this opens). **lobo235's #141**: the same goal, pulling the inbox into Claude turns with plugin hooks, and the same files (`hooks.rs`, `shim/src/main.rs`, `codex_hook.rs`, `claude_hook.rs`, plugin `hooks.json`). **#140**: keep a Claude push unread until written, in `hooks.rs` and `send_message.rs`. Fixes upstream issue #139 |
| Zoo (G8) | the run in `docs/zoo-plan-hooks.md`, as a **regression check for Codex only**, since the Zoo has no Claude harness. The PR must say so. Blocked on Codex on a1, and the operator approves the spend |

It contains:
- The Claude Code hooks and the `asyncRewake` waiter.
- `ack=client` and `/hook/ack`, including MUS-75's client ack on prompt hooks.
- The wake-watcher roster field.
- The SIGTERM "restarting" reply.

This is the most visible fix: Claude mail lost or never fetched without the
flag. Durability and Gate build on it.

The evidence beyond Zoo:
- Muster's live record: MUS-1, MUS-2, MUS-4 and MUS-75, verified on Murph.
- The hook e2e tests.
- The Claude columns of the conformance grid (MUS-69), run against the port.

**#141 is the delicate one.** It came first and aims at the same thing. The
operator should raise it with upstream. Two ways to settle it:
- ours lands, crediting #141, and lobo235 is asked to close #141 or narrow it;
- #141 lands first and ours is rebased onto it.

Ours shouldn't open as an unexplained competitor.

Before this layer opens, forward-port:
- **MUS-71:** `wake: expected` only with a live waiter.
- **MUS-86:** ack and re-offer log lines.

### 3. Durability: mail and sessions that survive failures (`port/durability-v2`)

| | |
|---|---|
| Size | +885 −46, 15 files, 2 commits |
| Depends on | layer 2 (#51 and #57's tests need Hooks) |
| Overlaps upstream | **lucid-max's own open #148**: preserve messages and receipts across session resume, in `cleanup.rs`, `message.rs`, `message_read.rs` and `hook_inbox_e2e.rs`. **lobo235's #131**: sticky read-state across a reload, in the same files. lobo235 calls it "the linchpin" because ignition-is-go/myko#96 will turn `#[belongs_to(Session, optional)]` on `Message` into a compile error |
| Zoo (G8) | needed: the send result gains a field agents see. Blocked on Codex on a1 |

It contains:
- idempotent send retry (`idempotency_key`);
- keeping a waiting session through a dropped connection;
- keeping mail to a reaped session until the 14-day sweep;
- read state for senders.

**Rebase onto #148 and #131 if they land first.** #148 is the maintainer's
own, and #131 is tied to the coming myko release, so either may merge before
we open. The description should say which of our cases they already cover.

Before this layer opens, decide whether **MUS-81** goes upstream. It acks a
push only once the harness has taken it (`acks_pushes`), the push-side twin of
MUS-75. It overlaps #140 and the plugin PRs below.

### 4. Other: small fixes and tools (`port/other-v3`)

| | |
|---|---|
| Size | +1126 −26, 11 files, 9 independent commits |
| Depends on | layer 3 |
| Overlaps upstream | **lobo235's #135**: `--help`/`--version` for the daemon, shim and TUI, in `daemon/src/main.rs` and `shim/src/main.rs`, the same argument handling the terminal commands touch. **#143**: room-name validation, in `cleanup.rs`, `room_tools.rs` and `tools.rs`, next to the operator-room fix |
| Zoo (G8) | only for the send fixes (MUS-72/73/78/79), which change what an agent sees on its first call |

It contains:
- terminal send, and the read-only roster command;
- the stale operator-room fix;
- the AGPL notice and the `cargo-deny` audit;
- the empty-recipient refusal (MUS-73);
- the first-write hold and its retries (MUS-72, MUS-78, MUS-79).

If upstream prefers small PRs, it can go as three:
- the shim's terminal commands;
- the send fixes;
- the dependency audit.

Before this layer opens, forward-port **MUS-80**, a stable code for the
unregistered refusal. The port still matches the refusal's text.

### 5. Gate: optional TLS and per-machine pairing (`port/gate-v2`)

| | |
|---|---|
| Size | +5815 −102, 45 files of its own (1297 lines of tests), plus the vendored myko-server |
| Depends on | layer 4 |
| Overlaps upstream | **lobo235's #135**, in `daemon/src/main.rs`'s argument handling, where `--tls`/`--plain` also live. Whichever lands second takes the other's parser. **ignition-is-go/myko**: the vendored `RequestFilter` |
| Zoo (G8) | a regression run in plain mode, to show off means off. Blocked on Codex on a1 |

**Off by default**, per the operator's choice of 2026-11-05 (#92):
- `marshal-daemon --tls` turns the gate on.
- `--plain` runs plain even though machines are paired.
- With pairings and neither flag, the daemon refuses to start.
- Given both flags, it refuses to start.
- There's no environment variable.

The layer already has the MUS-85 fail-closed check (`mode::has_pairings`) and
the MUS-78/79 e2e relay checks.

**The open question is myko.** The gate needs myko's own listener to admit
only forwarded requests. Vendoring myko-server means upstream carries about
12,200 lines it doesn't own. The inventory's recommendation (entry 77) stands:
offer the request-filter hook to ignition-is-go/myko first, then open the Gate
PR without the vendored copy. **The operator decides** whether to wait for
that.

If the gate is too large for one review, it can go as two PRs:
- **5a:** the listener, pairing, tokens and the frame filter;
- **5b:** limits, pause and the sender allow-list (#74–#76).

The evidence:
- the port's own `scripts/test-gate-e2e.sh` run on `port/gate-v2` (`a5de77c`): 99/99, per peppy-skate. Muster main's 104 includes checks for features that aren't in the port;
- the threat model;
- the conformance grid's plain-mode cells, which show the default is unchanged.

### 6. After the Gate, only if upstream wants them

| Candidate | Muster story / PR | Depends on | Overlaps upstream |
|---|---|---|---|
| Codex bridge respects the operator's pause | MUS-74 (#100) | Gate (needs `WakePause`) | — |
| Codex threads re-register after a restart | MUS-76 (#122) | Hooks | — |
| The Codex bridge's wake path on the roster | MUS-77 (#125) | Hooks | — |
| Codex through the gate | MUS-18 (#130) | Gate | — |
| Messaging between daemons (links) | MUS-66 (#91), with MUS-84 | Gate | — |
| Plugin push acks (Pi, opencode, Prime Agent) | MUS-81 (#118, #123) | Durability | **lobo235's #132, #134, #136, #137**, which fix the same losses in the same plugin files. Coordinate rather than compete |
| Delivery conformance suite | MUS-69 (#94–#111), MUS-88 | Hooks, Durability | none; it's test-only and could run upstream's plugin CI |
| Local-token refusal message | MUS-83 (#117) | Gate | — |

## What each PR carries

A layer is ready to offer when all of these hold:

1. It is rebased on current upstream main. It builds, and passes clippy with
   `-D warnings`, `fmt` and its tests under upstream's own CI configuration.
2. Every Muster fix that touches its files since its last rebase is
   forward-ported, or listed as left out with the reason.
3. Its description follows the matching section of
   [muster-vs-marshal.md](muster-vs-marshal.md): what it adds, why, what it
   costs and what stays out. It also names:
   - every hunk dropped in porting;
   - every overlapping upstream PR, saying what each one already covers.
4. Its evidence exists: tests and the conformance cells, plus the Zoo run
   imported into Zoo, or the operator's waiver with a reason.
5. The watchman has read it, and **the operator decides to send it, and sends
   it**.

## Keeping the stack in step

Muster keeps changing while the stack waits. **A fix merged to Muster main
that touches a ported file gets a forward-port commit on the lowest layer
that contains that file**, and the layers above it are rebased.
"Since the baseline" in [upstream-inventory.md](upstream-inventory.md) is the
ledger of the newer entries. This table summarizes the fixes that matter:

| Muster fix | Story | Layer | In the stack? |
|---|---|---|---|
| Quote peer bodies in the inbox | from #16 | Inbox-quoting | yes (`448671b`) |
| Prompt hook acks only after printing | MUS-75 | Hooks | yes (`3202b04`) |
| Refuse an empty recipient | MUS-73 | Other | yes (`d403770`) |
| Hold first writes until registered; re-send; retry once | MUS-72, MUS-78, MUS-79 | Other | yes (`d9124c1`, `55c5868`, `5717e22`) |
| Fail closed when pairings can't be read | MUS-85 | Gate | yes (`mode::has_pairings`) |
| Retire the internal plain-WS alias | MUS-70 | Gate | yes: the port never had it, and local tools use 6155 |
| `wake: expected` only with a live waiter | MUS-71 | Hooks | **no** |
| Log client acks and lease re-offers | MUS-86 | Hooks | **no** (#127 in Muster) |
| Stable code for the unregistered refusal | MUS-80 | Other | **no**: the port matches the text |
| Say when the local token is refused | MUS-83 | Gate | **no** |
| A push acked once the harness takes it | MUS-81 | Durability or later | **no**; decide first |
| Codex bridge fixes, Codex through the gate | MUS-74, 76, 77, 18 | after Gate | **no** |
| Links, conformance suite | MUS-66, MUS-69 | after Gate | **no**: no bucket yet |

## Decisions for the operator

1. **Talk to upstream's lead developer first.** Agree:
   - the series' shape (one PR per layer);
   - approval of fork-PR Actions;
   - a reviewer;
   - the overlaps: #154 to be superseded, #141 against Hooks, and #131 and #148 against Durability.
2. **Decided (2026-11-05): live communication goes first**, as
   `port/hooks-2a` with the hooks on by default and the macOS detection
   included. Before it's sent, its Zoo run must exist, or the operator waives
   it.
3. **Zoo (G8).** Install Codex on a1 so the runs can happen, and approve the
   spend for layers 1–3 and 5.
4. **myko first, or vendored?** Offer the request-filter hook to
   ignition-is-go/myko before the Gate PR (recommended), or send the Gate with
   the vendored copy.
5. **Port 6158** for myko's own listener, to agree with upstream before the Gate PR.
6. **Which "after the Gate" items to offer**, and whether MUS-81 goes with
   Durability.

## Who does what

| Role | Work |
|---|---|
| ultra-dune | Step 0: the catch-up branch, the port/* rebases, and the additive re-inventory (agreed by frank-crocodile and fiery-snapper, 2026-11-05) |
| fiery-snapper | Reads the catch-up, with an independent reviewer |
| Lamplighter (robust-bunting, Murph) | Keeps this strategy current, tracks upstream's open PRs, and deploys each catch-up on Murph |
| The watchman (frank-crocodile on Murph tonight) | Reads each layer and its description before it's offered |
| The operator | Talks to upstream, decides each PR and sends it, and decides the Zoo spend and the questions above |
