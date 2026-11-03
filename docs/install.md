# Installing muster

The install steps live in the [README](../README.md#install). Use those: this page used to
describe an early design, with a Unix socket, SQLite and a self-starting daemon, that
Muster no longer has.

## After upgrading Claude Code

Idle wake uses Claude Code's `asyncRewake` hooks, which Claude Code documents as waking an idle
session when the hook exits with code 2. Muster also runs one at session start, so a session
wakes before anyone has typed to it. The hooks documentation doesn't cover that case
specifically, so after an upgrade, check that a new session still wakes when a message arrives.
