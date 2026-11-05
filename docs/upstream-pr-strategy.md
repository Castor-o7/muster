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
  adds Session fields (the Gate's machine fields). `docs/pr-followups.md` shows
  upstream keeps ideas from closed PRs and reimplements them itself. Expect the
  same with ours.
- **Reference checkout:** `~/src/marshal` on Murph is a clean checkout of
  ignition-is-go/marshal main, with remotes `upstream` and `castor` (the
  Castor-o7/marshal fork that PR heads come from). Use it for upstream code.
  `~/src/muster` is Muster, not Marshal.
- Levi `main` is still `aed6027` (v0.9.5), so L1–L3 are current. Castor-o7/levi now
  exists as a fork of ignition-is-go/levi, which was what held L1.

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
4. **Gate**, in two or three PRs: TLS, then pairing, then the myko listener. It's
   additive while `--tls` isn't given, and plain mode stays the default. Ask about
   the vendored myko-server code before sending it.
5. **Later, still additive:** daemon links (TLS only) and the conformance suite
   (test only).

**Not sent as PRs: the default changes.** Inbox quoting (#141 already escapes the
same bodies), durability's retention and keep-alive, Other's empty-`to` refusal and
first-write wait, and MUS-76's Codex re-register. They're written up for lucid-max
as proposals, one issue each or one list, so upstream decides whether it wants them.

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

### For the operator to raise with lucid-max

- Thanks, and whether one PR per layer is still the shape they want.
- How they'd like the Gate split, and whether they'll take vendored code.
- Whether the Zoo run (G8) is still required. It was blocked on Codex on a1 when
  #164 merged.

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
