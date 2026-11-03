# When an agent disappears: what gets lost (MUS-31)

The recommendation below is a proposal. jaunty-zenith reviewed its main points from
reading the code, and nothing here has been built yet.

## How we know this

- Event log (`myko_events`) on Murph, read-only, 2026-10-29 to 2026-10-31.
- Code: `crates/daemon/src/cleanup.rs` and myko 6.9.0.
- A prediction made before the 2026-10-31 restart, checked after it.

`a_reaped_sessions_mail_stays_live_until_the_next_boot` in `cleanup.rs` shows the
cause directly (MUS-45).

## What disappears, and when

| What | When it leaves live state | Recoverable? |
| --- | --- | --- |
| The session row | At reap, by `sweep_once` | Event log |
| Direct mail to that session | At the next daemon start | Event log, backups |
| Read records for that mail | At the next daemon start | Event log, backups |
| Room memberships of that session | At the next daemon start (seen in the log; cause not traced per row) | Event log, backups |
| Messages over 14 days old | Swept while running, at boot and about every 5 minutes | Event log, backups |
| Mail the session sent | Stays while its recipient lives | Live |
| Work never sent as a message | At session end | Not by Muster |

Mail a session *sent* is not at risk from that session's removal: `Message` has
no `belongs_to` on its sender. It does go when the recipient is removed.

## Why mail goes at restart, not at reap

This was the open question. `sweep_once` removes a session with
`ctx.del_by_id` (`cleanup.rs:307`). In myko 6.9.0 that path updates the store and
writes the DEL, but doesn't call the relationship manager, so nothing cascades.
`del_dyn` and the batch paths do cascade.

So the children stay live. At startup, `establish_relations` runs
`cleanup_belongs_to_orphans`, which deletes every child whose parent is gone.

`sweep_messages` and `sweep_rooms` use `del_by_id` too, so the "cascades" comments
in `cleanup.rs` only become true at the next boot.

## What the data shows

Message removals, all during four restarts:

| Restart | Messages removed | Age when removed |
| --- | --- | --- |
| 2026-10-29 23:56 | 2 | 0.8 hours |
| 2026-10-30 13:26 | 6 | 2.6 to 14.4 hours |
| 2026-10-30 15:19 | 25 | 1.0 to 4.4 hours |
| 2026-10-31 16:35 | 2 | Not measured |

That is 35 in total. Of the first 33, every removal came after the recipient's own
DEL (83 minutes later on average), 32 had been read and 1 hadn't. All are far
younger than the 14-day limit.

Two checks support the explanation:

- At the 15:19 restart, 25 of 25 messages with an already-removed recipient were
  removed, and none survived.
- Before the 16:35 restart I predicted exactly 2 removals, both read. Exactly 2
  went.

On 2026-11-02, 422 live messages had removed recipients (421 read, 1 unread), so
the next restart should remove them all. That is by far the largest count so far,
and it comes from six sessions reaped between 2026-10-31 and 2026-11-01.

Removal from live state is not erasure. The original SET rows stay in the event
log, and `briny-tortoise`'s messages were recovered from it on 2026-10-30.

## Ways work gets lost

1. **A restart after a reap.** Mail to the reaped session leaves live state with
   no warning to its sender.
2. **A session reaped and resumed.** 4 of the 25 sessions DEL'd by 2026-10-31 were
   SET again. Returning before a restart should keep the mail; returning after
   wouldn't. These 4 weren't examined individually, so this is reasoning, not an
   observation.
3. **A successor session.** Sessions restarted in the same directory get a new id,
   so mail to the old id never reaches them. `4a212405` lost 23 messages this way,
   and `62b5a64b` lost 1. Whether they were successors in intent is inferred from
   their shared working directory.
4. **Unsent work.** Anything only in a session's context or working tree. Muster
   can't see it.
5. **Recovery depends on the event log.** It works today only because rows are
   appended and never compacted.

`scripts/restart-preview.sh` reports what the next restart will remove (MUS-49). Run against
the backups taken before the last two restarts, it predicted 422 messages, 421 read records
and 32 memberships for 2026-11-02 (35 memberships went, the other 3 to MUS-40's room tidy-up,
which it doesn't count), and exactly the 323, 323 and 12 removed on 2026-11-03.

## Recommendation

1. **Don't simply switch reaping to a cascading delete.** That moves the loss from
   the next restart to the moment of reap, closing the window that made recovery
   possible.
2. **Keep mail to a removed session for a retention period** instead of deleting
   it as an orphan. Either remove the `belongs_to(Session)` on `to_session_id` and
   let the 14-day sweep bound it, or tombstone it. From code reading only,
   jaunty-zenith found nothing that assumes a recipient exists: labels fall back to
   the session id, and `read_messages.rs` and `codex_bridge` only compare ids.
   `to_room_id` still cascades.

   The catch is that `MessageRead.session_id` is also `belongs_to(Session)`. If mail
   stays but read rows go at restart, a session returning under its old id sees old
   mail as unread and gets it again. Drop that relation too, or test that case.
3. **Hand mail to a successor.** Depends on stable identity (MUS-3): when a session
   claims a predecessor's identity, it gets the unread mail. This is what makes
   retention useful, since retained mail is otherwise offered to nobody.
4. **Tell the sender.** A sender should be able to see that mail went unread when
   its recipient was reaped.
5. **Make the comments true.** Fix or correct the `cleanup.rs` cascade comments
   whichever way the design goes.

Saving as it goes (unsent work) stays a convention for now: send field notes to a
room or commit them. RAL-1 (the ledger, now in Rally) may be the better home.

## Follow-up stories

Filed in the [roadmap](roadmap.md) as MUS-45 to MUS-49.
