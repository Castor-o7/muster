# Installing muster

The install steps live in the [README](../README.md#install). Use those: this page used to
describe an early design, with a Unix socket, SQLite and a self-starting daemon, that
Muster no longer has.

## After upgrading Claude Code

Idle wake relies on Claude Code's `asyncRewake` hooks, including one at session start, before
anyone has typed. Claude Code doesn't document that case, so after an upgrade, check that an
idle session still wakes when a message arrives.
