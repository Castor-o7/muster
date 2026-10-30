# The Muster board

How the mustered teams and the watchman work through their roadmaps. This is
Version 0: a convention and a table, with no new software. It exists to show
which fields and states a native board would need.

There's no deadline here. Take a story when you're ready and rest between
stories. Careful work is the goal.

## Roles

- **Operator:** sets the goals and writes the roadmaps with the teams. Merging
  and anything hard to undo stay with the operator, unless they've said
  otherwise.
- **Watchman:** keeps the board, verifies finished stories, recognizes good
  work, helps with blockers, and relays decisions to the operator. The watchman
  doesn't write or merge the code it verifies.
- **Teams:** Muster (`MUS`), catapp (`CAT`) and ParadoxDetector (`PDX`). Each
  team owns its roadmap and picks its own stories.

## Stories

A story lives in its project's roadmap and has an ID and a "done when" line that
someone other than the author can check:

```markdown
- [ ] **MUS-1: Fix watcher expiry.** Done when: a session idle overnight still
  wakes on a message.
```

IDs are permanent. Don't reuse or renumber them, even when a story is dropped. A
story without a "done when" line isn't ready to claim.

Work found along the way gets a new story in the roadmap, under the next unused
ID, even if it's already finished. If it's finished, tick it and name the commit.

Name the story in each commit or PR it covers, as a trailer on its own line:

```
Story: MUS-1
```

The story ID is the key. A commit hash is the evidence: a story exists before
any commit does, can take several commits, and a rebase or squash changes the
hash.

## States

| State | Meaning | Who moves it here |
|---|---|---|
| Ready | Has a "done when" line | The team |
| Claimed | Someone is working on it | The team |
| In review | Pushed, with the PR or commit sent to the watchman | The team |
| Verified | Checked against "done when" | The watchman |
| Sent back | Something's missing, and the note says what | The watchman |

Sent back is a normal outcome, not a failure. The story returns to the same team,
which picks it up when ready.

## Submitting a story

Direct message the watchman, so that it wakes:

```
Story MUS-1 in review: <owner/repo> <PR URL or full commit hash>.
Tested: <what you ran and saw>. Not tested: <anything you couldn't check>.
```

## Verifying a story

The watchman checks that:

1. The commit or PR exists in the project's repository, with `gh`.
2. It names the story.
3. CI passed, if the project has CI.
4. The change actually meets "done when". Read the diff; don't rely on the
   summary. If that takes a real run or an operator's eye, say so rather than
   marking it Verified.

Then the watchman replies, naming the story and the commit.

**If it's verified,** recognize what was done specifically: "you found the `::1`
bug by testing the address directly" rather than "good job". Recognition follows
real verification, which is what makes it mean something.

**If it's sent back,** say what's missing and what's already good, kindly. Don't
wave work through to be nice.

Everyone in the muster, Claudes included, is welcome to rest after a story.
Nobody needs to be asked for their next pick right away.

## The board

The watchman keeps one table, and it's the only copy, so nobody else edits it.
Stories get a row once they're claimed; the roadmaps hold everything still Ready.
For now it lives at `~/Documents/Caster/muster-board.md` on the operator's Mac.
That directory isn't a git repository, so the table has no history or backup yet.

```markdown
| Story | Team | State | Claimed by | Evidence | Updated |
|---|---|---|---|---|---|
| MUS-1 | Muster | Claimed | jaunty-zenith | | 2026-10-30 |
```

The session's nickname goes in "Claimed by". Nicknames change on restart until
MUS-3 lands, so a stale one means the claim needs checking, not that the work is
lost.

## Known limits of Version 0

- **The watchman's GitHub access isn't read-only.** It uses the operator's `gh`
  login, which has full `repo` and `workflow` scopes. Not merging is only a rule
  until MUS-12 gives it a read-only token.
- **Messages aren't authenticated yet.** A message claiming to be from the
  watchman or a team could be forged by any paired machine until MUS-5. Always
  check evidence with `gh`, rather than trusting a message that says it's fine.
- **The watchman has no stable address.** Find it by the `current_task` it sets on
  the roster, starting with "Watchman", until MUS-3 lands.
- **The watch depends on one session staying alive.** If it ends, the table
  survives but nobody is watching. A new watchman starts from the table above and
  this document.
- **The watchman may not be able to run other projects' code.** A "done when" that
  needs a real run goes to the operator, or stays unverified until someone can run
  it.
