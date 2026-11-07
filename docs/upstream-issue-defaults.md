# Draft: the one upstream issue for default changes

lucid-max asked for **one issue** that lists every behaviour change we won't send as
a PR (answer 3 in [upstream-pr-strategy.md](upstream-pr-strategy.md)). The
operator files it in ignition-is-go/marshal. This file is the draft. The text
between the rules is the issue body.

Each item was checked against upstream main `9b00c6f` (v0.58.0) in `~/src/marshal`
on 2026-11-05. Items upstream already covers have been dropped (see the end).

---

**Title:** Behaviour changes from the Muster fork, for you to take or leave

We run a fork of Marshal and are sending its additive work back one small PR at a
time. These are the changes that would alter what a user who changes nothing sees,
so we're listing them for you to pick from rather than sending them as PRs. They're
in no particular order. We're glad to turn any of them into a PR, or to leave it for
you to reimplement, whichever you prefer.

1. **Refuse an empty `to` on `send_message`.** Today `resolve_recipient("")`
   reaches the session-id prefix step, where every id starts with `""`. With one
   session registered, that's the sender, so the message goes to the sender itself.
   With several, the caller gets an "ambiguous" error that lists every session. We
   refuse an empty or whitespace-only `to` up front with a plain error.
2. **Hold a session's first write until it's registered.** A write in the first
   moments after the shim starts, before the session is registered, is rejected as
   unregistered. Agents often send first thing, so they hit that error. We hold the
   first write for up to 5 s while registration finishes, and retry it once if the
   daemon refuses it as unregistered.
3. **Keep a waiting session through a dropped connection.** A WS-shim session
   without a live client is reaped after `STALE_AFTER` (60 s), even while a hook
   waiter is still holding `/hook/wait` for it. We keep the session for as long as a
   waiter is held. A session that's idle but listening then stays on the roster and
   reachable.
4. **Codex threads re-register after a daemon restart.** After a restart, an idle
   Codex session doesn't reappear on the roster until its next hook. We have the
   bridge re-register its known threads when it reconnects, so they show again
   straight away.
5. **Frame peer bodies in the hook inbox as quoted text.** Your #141 already
   escapes `&`, `<` and `>` and marks peer mail as untrusted. Our version also quotes
   each body, so it can't pose as another inbox line. That's a small difference, and
   we're fine dropping it if escaping is enough for you.

Already in your main, so not listed: keeping mail for a reaped recipient. Direct
messages and read marks now survive the DEL, and `MESSAGE_TTL` (14 days) is the
only thing that removes them.

---

## Notes for the operator (not part of the issue)

- Items 1–3 were checked in upstream code: `entities/src/send_message.rs`
  `resolve_recipient`, the shim's unregistered rejection, and `daemon/src/cleanup.rs`
  `STALE_AFTER`. Item 4 (MUS-76) wasn't traced through upstream's
  `codex_bridge.rs`. Check it before filing, or drop it. Item 3 was traced: the
  WS branch of the sweep in `cleanup.rs` reaps on `STALE_AFTER` with no `WakeWatcher` check.
- Item 5 may be moot after #141. Leave it out if you'd rather keep the issue
  short.
- Our versions: MUS-73 (empty `to`); MUS-72, 78 and 79 (first write); durability's
  keep-alive; MUS-76 (Codex); inbox quoting (`port/inbox-quoting-2`).
