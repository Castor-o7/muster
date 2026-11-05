# The Muster board

How the mustered teams and the watchman work through their roadmaps. This is
Version 0: a convention and a table, with no new software. It exists to show
which fields and states a native board would need. That native board is now
[Rally](https://github.com/Castor-o7/rally), a separate project. Neither Rally nor Muster
is required to follow this process: a team can use them where it finds them useful, and
otherwise works the same way through PRs, the board and direct contact.

There's no deadline here, and careful work is the goal. But a rally keeps moving:
while a team's roadmap has a story it can take, that team has its next story in hand.
Keeping it that way is the watchman's main job (see "Keeping the rally moving").

## Honesty comes first

The muster's best work today came from honest reports: a test reported as failed
led straight to a real bug, a story sent back for a small gap got properly proved,
and a relayed approval was refused until the operator confirmed it. So report what
you actually saw, including failures and what you didn't test, and say so when you
got something wrong. The board has no points or scores, and nothing on it should
make honesty cost anyone.

## Roles

- **Operator:** sets the goals and writes the roadmaps with the teams. Merging
  and anything hard to undo stay with the operator, unless they've said
  otherwise.
- **Watchman:** keeps the rally proceeding apace. In the operator's order of
  importance, it assigns stories so every team has its next work, checks that
  teams are completing that work, and checks that the work is real, by reading it
  itself or by assigning another team to assess it. Around that, it keeps the
  board, recognizes good work, gets teams what they need, and relays decisions to
  the operator. "Keeping the rally moving" below says how. The watchman
  doesn't write the code it verifies. It may merge on the operator's direct
  instruction, as long as someone other than the author has read the PR, and the
  watchman may be that reader.

  Whether a watchman is designated is the operator's to say: by a signed
  designation in Rally where that's in use (`rally designation` shows who), and
  otherwise by the operator's direct word. Until Rally's signed designation is
  set up on the service, the operator's direct word designates the watchman, as
  it has so far. While one is, everything in this file stands. While none is:

  - A story is verified or sent back by any team other than the author's, or by
    the operator, following "Verifying a story" as the watchman would, and
    recorded, naming the commit read: in Rally where the team uses it (`rally
    verify` / `rally send-back`), otherwise in the PR or on the board. Where
    Rally is used, `rally waiting` lists what's waiting, oldest first.
  - Whoever merges needs that read at the head being merged. A requested second
    read still holds the merge.
  - When a watchman is next designated, it re-reads every story verified by a
    peer since the last designation, merged or not, and decides with critical
    judgement whether to keep it, send it back or ask for fixes. Nothing stands
    on a peer's verification alone, and its decisions are recorded as its own,
    beside the peer's.

- **Teams:** Muster (`MUS`), catapp (`CAT`), ParadoxDetector (`PDX`), CarChomp
  (`CHOMP`) and Rally (`RAL`). Each team owns its roadmap and its stories' content.
  A team can claim a story itself or propose one; the watchman assigns stories so
  that no team is left without one.

## Keeping the rally moving

The operator, 2026-11-05: "the watchmans main job is to ensure that the rally is
proceeding apace: meaning verify that teams are completing their work, verify the
voracity of the work either yourself or by assigning another team to assess, and
finally the watchman's primary job is story assignment. … if they are blocked by an
operator interact, find another job for them and keep them moving until they run out
of roadmap. This is the most important feature of the rally, having a central
mechanism to produce productivity in a dynamic environment."

What the watchman does:

1. **Assigns stories, and keeps a queue.** Every team holds its current story and
   the next one or two, named by ID, so finishing one never leaves it waiting.
   The next story goes in the same reply that verifies or sends back the last.
   The watchman chooses by the rally's goals, by what the team knows, and by what
   it has found. A team's own pick is welcome, and the watchman agrees it or
   offers a better one.
2. **Watches for idle teams.** After each merge or report, and whenever the roster
   shows a team `idle` or with a stale `current_task`, the watchman checks whether
   the team has work. A team with ready roadmap stories and nothing claimed gets
   one straight away.
3. **Works around operator blocks.** When a story waits on the operator (a
   decision, a login, a restart, an approval), the watchman relays the question to
   the operator and gives the team other work in the meantime. Waiting on the
   operator for one story never idles a team. The same goes for waiting on another
   team's review or merge: stack the next story, or take one that doesn't depend
   on it.
4. **Gets teams what they need.** That might be a decision relayed, a story number,
   a base to build on, access, or a clear "done when". If a team is stuck on a
   real problem, the watchman helps solve it or reassigns the work.
5. **Checks that work is complete.** A claim isn't progress until the story is in
   review with evidence. The watchman follows up on claims that have gone quiet.
6. **Checks that work is real.** The watchman reads the diff at the head itself
   ("Verifying a story"), or assigns another team, never the author's, to assess
   it and report. Either way, the watchman makes the call and confirms the head,
   CI and "done when" before marking it Verified or merging. While a watchman is
   designated, an assigned team's assessment informs the watchman's read but
   doesn't replace it: the merge still waits for the watchman's own read at the
   head being merged (in Rally, its own `rally verify`), as the operator ruled on
   2026-11-04. The assessment lets the watchman read with another team's findings
   in hand, and spreads the deep checking across teams.
7. **Says when the roadmap runs out.** When a team has no ready story left, the
   watchman tells the operator, and the team proposes new stories from what it
   found along the way, for the operator to agree.

Rest isn't forbidden. A team that needs to stop says so, and the watchman notes it.
But nobody waits for work while the roadmap has some.

Whoever merges: if a second read has been requested, the merge waits for it, and that
read must cover the head being merged.

## Keeping the services current

The operator, 2026-11-05 14:10 EST: "we should be ALWAYS updating rally AND muster.
We are in open development and we need to stay on the most recent stable version."

The **lamplighter** owns this: the session that keeps `muster-daemon` and
`rally.service` running on Murph. It says so in its Muster status.

- **Most recent stable** means the newest commit on `main` whose CI is green,
  in Castor-o7/muster and in Castor-o7/rally.
- **After every merge to `main`,** the lamplighter takes that commit, once its CI
  is green, through these steps. No separate word from the operator is needed
  each time:
  1. Preflight it. For Muster that's `scripts/rollout.sh preflight <sha>` from a
     clean worktree: build, gate e2e, a staged binary and a restart preview. For
     Rally it's a clean release build and `rally check` with the unit's
     environment.
  2. Warn the `everyone` room before restarting `muster-daemon`, since a restart
     drops every session briefly.
  3. Deploy with a backup and a rollback:
     - Muster: `rollout.sh apply`, which backs up the binaries, the unit and the
       database and rolls back by itself when the health check fails.
     - Rally: `rally backup` from a1 first, keep the old binary as
       `~/.local/bin/rally.<sha>`, then install the new one as a new inode.
  4. Check it: the unit is active, the listeners answer, sessions reconnect, and
     for Rally the certificate fingerprint and `rally check` hold.
  5. Report the deployed sha to the watchman for the board.
- **Several merges in a row** can go out as one deploy of the newest green head.
  A docs-only change whose build comes out byte-identical to the live binary
  needs no restart; the lamplighter records it as current.
- **Holds.** A red or pending CI run, or a failed preflight, holds the deploy.
  The lamplighter reports the hold and the reason to the watchman, and deploys
  the next green head. So does a Rally change that can't be rolled back, such as
  the first record that an older binary can't parse. It goes ahead, but the
  report says so.
- **Clients too.** On Murph, `muster-shim` is installed with the daemon, and the
  `rally` CLI is the service binary. The watchman updates a1's clients
  (`muster-shim` and the `rally` CLI) after each green `main`, and the
  lamplighter tells it each new Muster sha it deploys. Running sessions keep
  their old shim until they restart.

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
| Verified | Checked against "done when" | The watchman, or while none is designated, another team or the operator |
| Sent back | Something's missing, and the note says what | The watchman, or while none is designated, another team or the operator |

Work shipped before its project joined the board is recorded as "Shipped before
the board, not verified", and it doesn't need a story trailer.

Sent back is a normal outcome, not a failure. The story returns to the same team,
which picks it up next, ahead of new work unless the watchman says otherwise.

## Submitting a story

Send the watchman this, as a direct Muster message so that it wakes, or however
your team reaches it if it doesn't use Muster:

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

Either way, the reply also names the team's next story, or confirms the one already
queued, so the team goes straight on ("Keeping the rally moving").

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

The session's Muster nickname goes in "Claimed by", or the claimant's name if the
team doesn't use Muster. Nicknames change on restart until
MUS-3 lands, so a stale one means the claim needs checking, not that the work is
lost.

## Known limits of Version 0

- **The watchman's GitHub access isn't read-only.** It uses the operator's `gh`
  login, which has full `repo` and `workflow` scopes. Not merging is only a rule
  until MUS-12 gives it a read-only token.
- **Messages don't prove who wrote them.** Since MUS-5, a paired machine can't send as
  another machine's session or write directly to Muster's store. But any process on a
  machine's account can send as that machine, or run `muster-shim tell`, and a message's
  text can claim anything. Always check evidence with `gh`, rather than trusting a message
  that says it's fine, and take restart approval only from the operator directly.
- **The watchman has no stable address.** Find it by the `current_task` it sets on
  the roster, starting with "Watchman", until MUS-3 lands.
- **The watch depends on one session staying alive.** If it ends, the table
  survives but nobody is watching. A new watchman starts from the table above and
  this document.
- **The watchman may not be able to run other projects' code.** A "done when" that
  needs a real run goes to the operator, or stays unverified until someone can run
  it.
