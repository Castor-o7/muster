# Acking a push only once it's shown (MUS-81)

Design, for reading before anything is built. MUS-81 is the push-side twin of
MUS-75: a pushed message is marked read as soon as the push reaches the
plugin's connection, so if the harness then fails to take it, it's gone. The
conformance suite shows it on `pi`, `opencode` and `prime-agent` ×
`dropped-ack-redelivers`.

## What upstream already has

Four open Marshal PRs cover most of this. Muster's plugins are forks of
Marshal's, so the plan is to port them rather than solve it twice, which also
keeps the eventual merge into Marshal clean.

| Upstream | What it does |
|---|---|
| #140 | `Session.acks_pushes: Option<bool>`. When it's `Some(true)`, `SendMessage` doesn't write the push-time `MessageRead`; the client acks with `AckMessages` once it has shown the message. A session without the field behaves as today. Opts in only the Claude shim, and leaves the plugins for later. |
| #134 | marshal-pi: no inbox drain after a live push. The push is injected, then that message alone is acked. |
| #136 | marshal-pi and marshal-prime-agent: the per-turn pull is split into read, then `commit()` once shown or `release()` if it isn't, so a pull that misses its deadline doesn't ack what it never showed. A message held by one pull isn't handed out by another. |
| #137 | marshal-opencode: no drain after a live push, and a truncation notice on previews. |

The drain-after-push in #134 and #137 can ack a different unread message
without showing it, which is upstream's #133. Muster's plugins have that drain
too, except Prime Agent's.

## The change

1. **Daemon: port #140's `acks_pushes`**, with the same name and meaning, so
   the merge into Marshal is a no-op. `SendMessage` skips the push-time
   `MessageRead` for a session with `acks_pushes: Some(true)`. #140's Claude
   shim half isn't needed for MUS-81, because Muster's Claude sessions run
   without channels and are delivered by hooks; it can come later with the
   rest of #140.
2. **Each plugin opts in.** pi, opencode and Prime Agent register their
   sessions with `acksPushes: true`. On a push with a body, the plugin hands
   it to the harness (`pi.sendMessage`, `session.prompt`), and only when that
   resolves sends `AckMessages` for `meta.message_id`. If the hand-off throws,
   nothing is acked, and the message stays unread for the next pull.
3. **No drain after a push** in pi and opencode (#134, #137).
4. **The per-turn pull reads, then commits** (#136): `pullInbox` returns the
   text with `commit()` and `release()`. The turn hook commits once the text
   is in the turn, and a late or failed pull releases. Ported to all three
   plugins (#136 covers pi and Prime Agent; opencode's `chat.message` needs
   the same).
5. **No double show.** With pushes unread until acked, a message whose push
   has been handed over but not yet acked could also turn up in the next pull.
   The push path holds its message id, in the same held set #136 uses between
   pulls, until its ack lands or fails. A held message isn't pulled.

Mixed versions are safe both ways, as #140 notes. An old daemon ignores the
field and marks the push read itself, so the plugin's ack comes back as
already acked. An old plugin never sets the field.

## Tests

- **Conformance:** `dropped-ack-redelivers` must pass on all three plugins and
  leave `KNOWN_FAILURES`. Today the harness throws after the push and the
  message is lost; after the change, the next turn's pull shows it.
- **New scenario, mail waiting at reconnect** (upstream #134's case): the
  plugin's connection drops, a direct message arrives and stays unread
  because there's no live client to push to, and the plugin reconnects. No
  push comes for that message. The next turn's pull must show it exactly once
  and ack it, and the turn after must not show it again. The plugin fakes gain
  a driver command to drop and restore the plugin's daemon connection.
- **Unit tests:** the plugins' own `bun test` gets #136's `inbox-pull` tests
  and #134's `live-push` tests, plus one for the held set between a push and a
  pull.
- **Rust:** #140's `send_message_e2e` cases, where a session with
  `acks_pushes` keeps a push unread until it acks.

## Order

1. The daemon field and its Rust tests: one PR, which changes nothing until a
   client opts in.
2. pi, opencode and Prime Agent: one PR each. Each opts in, ports its
   upstream fixes, and flips its conformance cell. The reconnect scenario
   arrives with the first.

## Limits

- **"Shown" means handed to the harness's API.** If pi accepts the message and
  then crashes before the model sees it, it's acked and lost. The plugin can't
  see further than its harness's API, as #140 notes for the Claude shim.
- **A push the plugin received but never handled**, because it died first,
  stays unread and comes back at the next pull. That's the point of the
  change.
