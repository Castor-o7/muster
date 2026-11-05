# Plan: a Marshal Zoo run for the Hooks change (G8)

Status: **plan only. Nothing runs until the operator approves the spend.**

Marshal's `scripts/sandbox/README.md` (at `94ecc24`) asks that every change
to Marshal's behaviour be assessed with a real-agent simulation before it is
reported as assessed. This plan covers the upstream Hooks PR (see
[upstream-inventory.md](upstream-inventory.md) and
[hook-delivery.md](hook-delivery.md)).

## What the Zoo can and can't exercise

The sandbox drives **Codex** agents only. `run.py` starts Codex app-server
threads with dynamic tools, from a Codex login. It has no Claude Code
harness. Most of the Hooks change is Claude-only:

| Part of the Hooks change | Path | Zoo exercises it? |
|---|---|---|
| `claude-hook session-start/prompt-submit/post-tool-use`, `hooks.json` | Claude only | **no** |
| `asyncRewake` waiter on `SessionStart`/`Stop` (`claude-hook wait`) | Claude only | **no** |
| macOS parent-cmdline read, so `channels_enabled=false` | Claude only | **no** |
| `/hook/wait`, `ack=client`, `/hook/ack`, the Monitor watcher | Claude only (Codex wakes through the bridge's `turn/start`) | **no** |
| `surface_unread` leases (60 s) and the deferred ack | **shared**: Codex's `prompt-submit` and tool-use hooks go through it | yes |
| `WakeWatcher` records and `wake: expected` in send results | shared daemon code; Codex doesn't hold waits, so they stay empty | partly: absence only |
| Truncation notice, re-registration on prompt-submit | shared | yes, if a message is long enough to be truncated |
| Send results (`live_push`/`wake`) | shared | yes, through the tools' replies |

So **the Zoo run is a regression check.** It answers one question: does the
Hooks change leave Codex coordination as it was? It cannot show that Claude
delivery works. That evidence comes from elsewhere (see "Claude side" below),
and the PR must say so.

## Builds

| Condition | Source | Notes |
|---|---|---|
| baseline | Marshal `94ecc24` (v0.54.1) | upstream as is |
| candidate | `94ecc24` + the Hooks PR branch, under Marshal names | exists only after G1 and the port; build from that commit |

Each condition gets its own checkout and an immutable binary directory
(`cargo build --release -p marshal-daemon -p marshal-shim`, then copy the two
binaries out), as `campaign.py` requires. Both builds must carry upstream's
`response_required` (#161). Pass
`--baseline-representation metadata --candidate-representation metadata` so
the only difference between the conditions is the Hooks change.

## Scenario, model, budgets

- **Scenarios:** the campaign's three: `independent`, `approval-wait`,
  `contention`.
  - `contention` matters most here. The owner's release reaches the waiter
    through the hook inbox, and it includes the late, duplicate release
    notice, which tests that leases and acks don't re-surface or swallow
    mail.
  - `approval-wait` checks that injected mail still arrives and is framed as
    peer context.
  - `independent` is the control: there should be no peer messages.
  - `three-way` is left out: it costs the most and adds nothing hook-specific.
- **Model:** `gpt-6.1-sol`, the model in both of upstream's recorded campaigns
  (`structured-replies-2026-11-04`, `notification-policy-2026-11-04-v1`), so
  the results can be set against them. It must be available to the Codex
  login used. If it isn't, pick one model and use it for both conditions.
- **Repeats:** 3 per condition per scenario, 18 trials in all, run as
  isolated baseline/candidate pairs with 2 trials at once (the campaign's
  default).
- **Budgets (campaign defaults, unchanged):** 650,000 reported tokens,
  420 s, and a 35 s quiet window per trial. The campaign stops starting new
  pairs after 8,000,000 reported tokens.

Command, once both binary directories exist:

```sh
python3 scripts/sandbox/campaign.py \
  --output /tmp/marshal-hooks-campaign --model gpt-6.1-sol --repeats 3 \
  --scenarios independent approval-wait contention \
  --baseline-representation metadata --candidate-representation metadata \
  --baseline-bin /tmp/marshal-baseline-bin --baseline-commit 94ecc24 \
  --candidate-bin /tmp/marshal-hooks-bin --candidate-commit <HOOKS_SHA>
python3 scripts/sandbox/assess.py /tmp/marshal-hooks-campaign
```

Free check first: `MARSHAL_TEST_SHIM=<candidate shim> python3 -m unittest
discover -s scripts/sandbox -v`. It makes no model calls and confirms the
candidate shim still passes the sandbox's routing and hook-trust contracts. If
it fails, nothing else runs.

## Cost estimate

These are the per-trial means from upstream's two campaigns, which used the
same model, scenarios and budgets: 30 trials of these three scenarios.

| Scenario | Total reported tokens | of which uncached input | output | wall time |
|---|---|---|---|---|
| independent | 91.5k | 33.9k | 0.3k | 46 s |
| approval-wait | 214.6k | 45.5k | 0.6k | 91 s |
| contention | 418.8k (max seen 497k) | 61.5k | 1.5k | 127 s |

For 18 trials (3 scenarios × 2 conditions × 3 repeats):

| | Estimate | Ceiling |
|---|---|---|
| total reported tokens | **≈ 4.4 M** (upstream's comparable campaign: 4.57 M) | 8 M campaign stop, plus overshoot of up to one pair's 1.3 M |
| uncached input | ≈ 0.85 M | |
| output | ≈ 15 k | |
| cached input (the rest) | ≈ 3.5 M | |
| wall time | ≈ 15–25 min | 18 × 420 s / 2 ≈ 63 min |

To price it: `0.85M × uncached-input rate + 3.5M × cached-input rate +
15k × output rate` for the chosen model. I don't have a verified price for
`gpt-6.1-sol`, so I haven't guessed one. On a ChatGPT-plan Codex login, the
run draws on the plan's usage limits rather than producing a per-token bill.
The operator should check which applies to the login used.

**Staged option:** run `--repeats 1` first, about 1.5 M tokens and about
5 min. Go to 3 only if the first pair shows no setup problems. The README
says one run "demonstrates a trace, not a reliable behavioral improvement",
so the PR needs the full 3.

## What we report

As the README asks: keep every run, including failed and incomplete ones.
Import `sandbox-run.json` into Zoo, and give per-scenario pass/fail for each
condition, with message, turn, wake and token counts. Read the `contention`
transcripts for duplicate or missing mail around the release and the late
notice. State plainly what the run did not exercise: everything Claude-only
in the table above.

Pass for the PR means the candidate is no worse than the baseline: the same
scenario passes, no new resource violations, and no messages after the
duplicate release notice. Lower token counts are not the goal.

## Claude side (not Zoo)

Claude delivery can't be shown in the Zoo without a Claude Code harness in
the runner. The options, in order of cost:

1. **No model cost, available now:** the existing tests,
   `crates/shim/tests/claude_hook.rs`, `crates/daemon/tests/hook_wait_e2e.rs`
   and `scripts/test-gate-e2e.sh`. They drive the real binaries through the
   hook, wait and ack paths, but with no model.
2. **Operator-observed:** Muster's own sessions have run on the hooks since
   late October with no channels flag. This session's mail from the watchman
   arrives through the `Stop` waiter. A short log of such deliveries can go
   in the PR as field evidence, not as a measurement.
3. **A Claude harness for the Zoo:** a separate proposal. It is runner code,
   it spends Anthropic API usage, and it would be an upstream PR in its own
   right. Not part of this plan.
