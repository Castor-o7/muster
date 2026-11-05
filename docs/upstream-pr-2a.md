# Upstream PR 2a: live delivery for Claude Code without the channels flag

The description for the first PR to [ignition-is-go/marshal](https://github.com/ignition-is-go/marshal),
ready for the operator to send. **The operator decides when to send it, and sends it.**
Everything below the line is the PR's text. Where it goes in the series is in
[upstream-pr-strategy.md](upstream-pr-strategy.md) ("2. Hooks").

| | |
|---|---|
| Branch | `port/hooks-2a-offer` at `62552e6` (in Castor-o7/muster), cut from Marshal `94ecc24` (v0.54.1) |
| Size | 4 commits, 22 files, +2387 −79 |
| Checked | Workspace tests (193) green, `clippy -D warnings` and `fmt` clean, at the identical tree `7f0a56c`. The first commit passes `cargo check --tests` on its own |
| Before sending | Zoo (G8) run or the operator's waiver; the operator's talk with the lead developer about #154 and #141; whether to keep the provenance lines in the commit messages (asked of the operator) |

---

## Title

feat: deliver to and wake Claude Code without the channels flag (fixes #139)

## Summary

Claude Code started without `--dangerously-load-development-channels` doesn't
reliably get Marshal mail today (#139):
- Claude drops `notifications/claude/channel`.
- On macOS the shim can't read its parent's command line, so it reports
  `channels_enabled` as unknown. The daemon pushes, is told "delivered", and
  marks the mail read, so the mail is lost.
- Nothing pulls the inbox into a turn, so even unread mail sits until someone
  calls a tool for it.

This PR adds a hook path that needs no flag. A session gets its direct mail at
session start, before each prompt and after each tool call. An idle session is
woken when mail arrives.

## What it adds

- **Inbox hooks:** `marshal-shim claude-hook session-start | prompt-submit | post-tool-use`.
  - Wired by `plugins/marshal-shim/hooks/hooks.json`.
  - Pulls unread direct mail into the turn as a `<marshal_inbox>` block.
  - 2 s timeout, silent on failure.
- **Idle wake:** `marshal-shim claude-hook wait`.
  - An `asyncRewake` hook on `SessionStart` and `Stop`.
  - Holds `/hook/wait` open, then exits 2 with the mail on stderr, which starts a turn in an idle session.
  - A per-session lock keeps one waiter per session.
  - `marshal-shim watch [--once]` is the same loop, for Claude's Monitor tool.
- **No lost mail on the hook path:**
  - The waiter and the prompt hooks ask for `ack=client`, get the ids in `X-Marshal-Ack`, print the mail, and only then `POST /hook/ack`.
  - The daemon leases surfaced mail for 60 s (`MARSHAL_ACK_LEASE_SECS`), so two readers don't take the same message.
  - Mail whose reply is lost (a hook killed at its timeout, say) is offered again when the lease lapses. So a lost ack can mean a duplicate, never a loss.
- **Wake reporting:**
  - While a wait is held, the daemon records a `WakeWatcher`. It's in memory only and swept with its session.
  - The roster shows a session's `wake` state.
  - A send result says `wake: expected` only when a live waiter can wake the recipient and wakes aren't paused. A connected client alone no longer counts, which matches the roster.
- **Restarts:** on SIGTERM the daemon ends held waits with `X-Marshal-Restarting`, so waiters re-arm instead of reporting an error.
- **macOS:** the shim reads its parent's argv with `ps`, so a session started without the flag reports `channels_enabled = false` and is no longer pushed to. Its mail stays unread for the hooks.
- **Logging:** one `info` line per client ack, and one per re-offer after a lapsed lease. Session and count only, no bodies.
- **Smaller fixes on the same path:**
  - the shim tries every resolved address (`localhost` may resolve to `::1` first);
  - long Monitor lines are kept whole;
  - the truncation notice names a URI the resource serves;
  - the shim re-registers when its roster row has lost its client binding.

## What changes by default

Two things, both intended:

1. **The plugin turns the hooks on.** Every Claude Code user of `marshal-shim`
   gets the hooks:
   - one local HTTP request per tool call (2 s timeout, silent on failure);
   - one waiter process per idle session, which starts a turn when mail arrives.

   Sessions launched with the flag still get pushes, and their hooks then find
   nothing unread.
2. **macOS sessions started without the flag stop losing mail.** They are no
   longer pushed to, so the mail stays unread until a hook delivers it.
   Flagged sessions are unchanged.

**To run without the hooks:** install the MCP server without the plugin (its
`.mcp.json` alone; the README shows how), or set Claude Code's
`disableAllHooks`.

## What's unchanged

- **A hook listener that fails to bind or stops** is logged, and the daemon
  keeps running, as in v0.54.1.
- **The push path:** a flagged session gets live pushes as before.
- **Codex:** its hooks and app-server bridge work as before. The lease and the
  client ack apply to its prompt and tool hooks through the same shared code.
- **Callers that don't ask for `ack=client`** keep the daemon's own ack at
  flush.
- **Protocol:** nothing existing is renamed or removed.
  - The roster gains a `wake` field, and existing readers ignore it.
  - The send result's `wake` value is narrowed, as described above.

## Overlaps

- **Supersedes #154** (ours, 2026-10-29), an earlier cut of the same change.
  We'll close it with a link to this one.
- **#141 (lobo235)** pulls the inbox into Claude Code turns with plugin hooks,
  in many of the same files, and came first. We'd like to coordinate. Happy to
  rebase onto #141 if you'd rather take it first, or to fold in whatever of it
  this PR lacks. We'll credit it either way.
- **#140 (lobo235)** keeps a Claude push unread until the shim has written it.
  It's related, since it closes part of the same loss from the push side, and
  doesn't conflict in intent. We're glad to adjust whichever lands second.

## Evidence

- **Tests:**
  - `crates/shim/tests/claude_hook.rs`: inbox pull, empty inbox, unreachable and silent daemons, the shipped hooks answering their events, the waiter exiting 2 with mail, refusal notices, one waiter per session.
  - `crates/daemon/tests/hook_wait_e2e.rs`: held waits deliver and ack, an abandoned wait doesn't ack, watcher reporting, a stopping daemon ends waits cleanly, idle timeout.
  - `crates/daemon/tests/hook_client_ack.rs`: un-acked mail stays unread, and the daemon's ack is kept without `ack=client`.
  - Lease unit tests.
  - Workspace: 193 tests green, `clippy -D warnings` and `fmt` clean.
- **Live use:** the same change has run in Muster, a fork of Marshal, since
  2026-11-04 on Linux and macOS machines. These were verified on the live
  daemon:
  - a restarted session wakes on mail before anyone types to it;
  - the roster tells a wakeable session from one that can't be woken;
  - a prompt hook's lost reply leaves the mail unread and offers it again
    after the lease;
  - a session with no waiter is told `unobserved`, not `expected`.

  The overnight idle check, whether an idle session wakes all night within 2
  model turns an hour, is planned and hasn't run yet.
- **Conformance grid:** Muster's delivery conformance suite runs the real
  daemon and shim against a fake Claude Code harness. Its Claude columns pass
  in plain and TLS mode for:
  - an idle session waking on a DM;
  - a busy session getting it at its next hook;
  - a dropped ack redelivering, and a duplicate shown at most once;
  - a lost hook reply redelivering;
  - a restart mid-wait;
  - a session with no waiter not reported as `expected`;
  - a broadcast never waking anyone.

  None of the Claude cells is a known failure. That's in Muster. The suite
  isn't part of this PR, and its operator-pause cell tests a Muster control
  that isn't in this PR.
- **Zoo (`scripts/sandbox`): not run yet.** The Zoo drives Codex agents, and
  Codex isn't installed on the machine meant for the run. The Zoo has no Claude
  Code harness, so it can only check that Codex coordination is unchanged. It
  can't show the Claude delivery itself. That is still the run this change
  needs: a regression check for Codex, with the same scenario, model and
  budgets before and after. We'll attach it before review, unless you'd prefer
  to waive it.
