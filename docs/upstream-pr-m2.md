# Upstream PR M2: message the marshal and read the roster from a terminal

The description for the next additive PR to [ignition-is-go/marshal](https://github.com/ignition-is-go/marshal),
after #164 (live delivery for Claude Code) and #165 (the AGPL notice). The
operator decides when it's sent. Everything below the line is the PR's text.
Where it fits in the series is in [upstream-pr-strategy.md](upstream-pr-strategy.md).

| | |
|---|---|
| Branch | **`port/terminal-cmds-v3` at `3af92d7`** on Castor-o7/marshal (ultra-dune, 2026-11-05): one commit on upstream `9b00c6f` (v0.58.0), cherry-picked from `481f18a`, `2211da4` and `4a53e00`. It replaces the older `port/terminal-cmds` cut (`5044c3e` on `62552e6`) |
| Contents | 1 commit, 5 files, +663 −9: `crates/shim/src/terminal_send.rs`, `roster_read.rs`, `main.rs` (subcommands and `usage()`), `README.md`, and `plugins/marshal-shim/README.md` |
| Checked | at `3af92d7`: a clean clone passes 257/0 for daemon, shim and entities; clippy and fmt are clean; native Windows is green (Castor-o7/marshal run 42114245402, windows-native-wake); there are no Muster names. The lamplighter re-verified the branch, diff and log |

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
  - The terminal registers a short-lived session of kind `terminal`, under a
    fresh id for each run, so two runs at once can't remove each other's
    sender (fixed in 8d53df3 after the watchman's read; the first cut kept one
    id per machine).
  - It sends each recipient a direct message, so an idle session wakes for
    it, the same as for any direct mail.
  - It prints one line per recipient, then removes its own roster row.
  - It refuses an unknown or ambiguous name, and says when a name that matches
    nothing is too short to be an id prefix (under 8 characters). A terminal
    can't be addressed.
  - The body is labelled `[from a terminal on <machine>; it can't receive replies]`.
    The README says a terminal message names the machine, not the person:
    it carries no authority and is never the operator's approval.
- **Roster for scripts.**
  - `marshal-shim roster` only subscribes. It registers no session and
    doesn't appear on the roster.
  - Each row separates the daemon's record (`recorded`: the connection
    binding, a held wait and until when) from what the session declared, and
    includes the `wake` state that #164 added. The daemon's hook listener
    writes the wait and myko fills the binding from the connection, but any
    connected client can also write either, so they're the daemon's record,
    not proof.
- **A plugin README fix.** The README told users to put `mcpServers` in
  `.claude/settings.json`, where Claude Code silently ignores it. It now goes
  in `<project>/.mcp.json`, with the status line and the hooks in
  `settings.json`. The README also said sessions have no nickname; they do,
  and peers can be addressed by nickname or `session_id`.
- **`--help`:** `marshal-shim --help` lists `tell`, `all` and `roster`. Both
  new paths reach the daemon through the shim's existing `daemon_address()`, so
  nothing else in the shim changes.

## What changes by default

Nothing, apart from the README fix, which corrects instructions that didn't work. The only visible effect is while someone runs `all` or `tell`: a
`terminal` row appears on the roster for the length of the send, then is
removed. Sessions see the labelled message like any other direct mail.

## Overlaps

- Built on `main` at v0.58.0, so it already includes #135's `--help`/`--version`:
  `tell`, `all` and `roster` are added to the shim's `usage()` text.
- Uses the roster `wake` field that #164 added.

## Evidence

- Unit tests: 10 for terminal send and 2 for the roster's JSON.
  - 7 cover naming and refusal: everyone but terminals, a team, a full id or
    unique prefix, unknown, ambiguous and short names, a name that's both a
    team and a session, and a terminal.
  - 1 checks the label.
  - 2 check the line printed for each recipient: each send outcome, a failed
    send, and a recipient with no team, host or stored nickname.
- On this exact head: a clean clone passes
  `cargo test -p marshal-daemon -p marshal-shim -p marshal-entities` (257 passed,
  0 failed); `clippy -D warnings` and `fmt` are clean; native Windows CI is green on
  the fork. No version bump.
- The same commands have run in Muster, a fork of Marshal, since 2026-11-03.
  Operators message every session from a terminal with them, and scripts read
  the roster.
- **Zoo:** not run. Nothing agent-facing changes unless someone runs the
  commands, and a terminal message is ordinary direct mail with a label.
