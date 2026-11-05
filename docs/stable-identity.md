# Stable identity across restarts

MUS-3. A session's nickname used to belong to its session id. A Claude
session started again in the same directory, or a new Codex thread, gets a new
id, so it got a new nickname. The old row lingered with the old one, and mail
to the old nickname went nowhere. Now a new session takes over a dead
predecessor's nickname. The code is `crates/daemon/src/succession.rs`.

## Which sessions match

Each session has an identity key, recorded when it's first named:

- **By default**, its harness and working directory, on the same machine for
  the same operator: `cwd:claude|/home/me/project|me@example.com|a1`.
- **`MUSTER_IDENTITY=<role>`** declares a role instead, such as `watchman`.
  A role matches across working directories.
- **`MUSTER_IDENTITY=none`** opts out. The session gets a fresh nickname and
  is never taken over.

The shim's hooks and MCP shim send `MUSTER_IDENTITY` from the session's
environment. Only Claude and Codex report a harness so far, so the plugins
have no inferred identity yet.

## When a predecessor counts as dead

A wrong takeover hands a live session's mail to another session, while a
missed one only costs a fresh nickname. So the rules lean towards missing.

| The predecessor's row | Dead when |
|---|---|
| Had a client (a Claude MCP shim) | The client left at least the grace ago (`MUSTER_SUCCESSION_GRACE_SECS`, 30 s), it has no waiter, and it hasn't hooked since the client dropped. |
| Never had a client (hooks only) | It has no waiter, and nothing for the quiet time (`MUSTER_SUCCESSION_QUIET_SECS`, 600 s). |
| A Codex row still on the roster | Never, until MUS-77 gives the daemon a way to see the bridge's threads. |
| Gone | This daemon saw it go: Codex's SessionEnd, or a reap. After a daemon restart every row is gone, and none counts as dead until it's seen again. |

A role is never taken over while any session holding it is live.

## Late takeovers

Claude starts again within seconds, while its predecessor's client is still
inside the grace. Then the new session gets a fresh nickname first. For its
first 10 minutes (`MUSTER_SUCCESSION_WINDOW_SECS`), the daemon checks again
on every sweep. Once the predecessor counts as dead:

- the new session takes its nickname;
- its fresh nickname keeps reaching it for 10 minutes
  (`MUSTER_SUCCESSION_ALIAS_SECS`), since peers may have used it;
- its next prompt or tool hook says `Your Muster nickname is now …`, because
  the session block it printed at start named the fresh one.

## What a takeover does

In one step, the daemon:

- re-keys the nickname to the new session;
- moves the predecessor's ad hoc room memberships to it, keeping when the
  predecessor joined;
- retires the predecessor's row (its mail is kept, MUS-46);
- writes a `Succession` record: who took over from whom, the nickname, the
  key, why the predecessor counted as dead, and any alias.

Mail to the old session id follows the record to the new session. Handing the
predecessor's unread mail to its successor is MUS-47.

## Addressing

`send_message` resolves a name in this order:

1. nickname;
2. session id;
3. an alias from a late takeover;
4. an old session id, through its succession records;
5. a declared role, if exactly one session claims it;
6. operator;
7. session id prefix.

The roster shows a session's harness and its role. The role is a claim; see
the threat model.
