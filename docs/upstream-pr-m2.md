# Upstream PR M2: message the marshal and read the roster from a terminal

The description for the next additive PR to [ignition-is-go/marshal](https://github.com/ignition-is-go/marshal),
after #164 (live delivery for Claude Code) and #165 (the AGPL notice). The
operator decides when it's sent. Everything below the line is the PR's text.
Where it fits in the series is in [upstream-pr-strategy.md](upstream-pr-strategy.md).

| | |
|---|---|
| Branch | `port/terminal-cmds` at `da4b719` (Castor-o7/muster, by peppy-skate), on `62552e6`, #164's head. Pushed to Castor-o7/marshal when it's sent |
| Contents | 2 commits, 6 files, +570 −14. `100d7ea`: the terminal commands (Muster's `9fdf775`) without `idempotency_key`, which arrives with Durability. `da4b719`: the plugin README fix (Muster's `abb4c94`) |
| Opens | after #164 merges, since the roster's `wake` field comes from #164 |
| Checked | workspace tests (201) green, `clippy -D warnings` and `fmt` clean, the sandbox unittest passes, and the Muster-name grep finds 0 (peppy-skate). It rebases cleanly onto #164's evidence and comment commits, since it doesn't touch those files |

---

## Title

feat(shim): message the marshal and read the roster from a terminal

## Summary

Two small `marshal-shim` subcommands for people and scripts outside any agent
session:

- **`marshal-shim all <message>`** and **`marshal-shim tell <team, nickname or session id> <message>`**
  send a direct message from a terminal.
- **`marshal-shim roster`** prints the roster as JSON for other programs.

Both are **additive**. They're new subcommands that nothing existing calls, so
a Marshal user who doesn't run them sees no change.

## What it adds

- **Terminal send.**
  - The terminal registers a short-lived session of kind `terminal`, under
    one id kept per machine.
  - It sends each recipient a direct message, so an idle session wakes for
    it, the same as for any direct mail.
  - It prints one line per recipient, then removes its own roster row.
  - It refuses an unknown, ambiguous or too-short name. A terminal can't be
    addressed.
  - The body is labelled `[from a terminal on <machine>; it can't receive replies]`.
    The README says a terminal message names the machine, not the person:
    it carries no authority and is never the operator's approval.
- **Roster for scripts.**
  - `marshal-shim roster` only subscribes. It registers no session and
    doesn't appear on the roster.
  - Each row separates what the daemon observed (the connection binding, a
    held wait and until when) from what the session declared, and includes
    the `wake` state that #164 added.
- **A plugin README fix.** The README told users to put `mcpServers` in
  `.claude/settings.json`, where Claude Code silently ignores it. It now goes
  in `<project>/.mcp.json`, with the status line and the hooks in
  `settings.json`. The README also said sessions have no nickname; they do,
  and peers can be addressed by nickname or `session_id`.
- **Internal tidy-up:** the shim's address lookup moves into one
  `daemon_address()` that all three paths share, and `tools::await_command`
  becomes `pub(crate)`. Behaviour is unchanged.

## What changes by default

Nothing, apart from the README fix, which corrects instructions that didn't work. The only visible effect is while someone runs `all` or `tell`: a
`terminal` row appears on the roster for the length of the send, then is
removed. Sessions see the labelled message like any other direct mail.

## Overlaps

- **#135 (lobo235)** adds `--help` and `--version` to the daemon, shim and
  TUI, and edits the same argument match in `shim/src/main.rs`. The two don't
  conflict in intent. Whichever lands second takes the other's arms, and we're
  happy to rebase onto #135.
- **Depends on #164**, whose roster `wake` field this prints.

## Evidence

- Unit tests: 6 for terminal send (naming, refusal of unknown or ambiguous
  names, the label, one line per recipient) and 2 for the roster's JSON.
- The same commands have run in Muster, a fork of Marshal, since 2026-11-03.
  Operators message every session from a terminal with them, and scripts read
  the roster.
- **Zoo:** not needed. Nothing agent-facing changes unless someone runs the
  commands, and a terminal message is ordinary direct mail with a label. The
  operator can ask for a run if upstream wants one.
