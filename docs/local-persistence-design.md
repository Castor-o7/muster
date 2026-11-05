# Keeping messages across restarts without Postgres (MUS-17)

Status: **design for review**, no code yet. Written 2026-11-05 for the watchman and
fiery-snapper. The Postgres path is out of scope and stays exactly as it is.

## The problem

Without `MYKO_POSTGRES_URL` the daemon runs ephemeral: every entity goes to
myko's `BlackholePersister`, and a restart loses all mail, read or not. The
startup line says so (`persistence: EPHEMERAL`). Most operators won't run
Postgres for a one-machine bus, so for them an unread message dies with a
restart, an upgrade, or a reboot.

Done when (roadmap): an unread message survives a daemon restart with no
Postgres configured.

## Does myko allow it?

Yes, with what it already exposes; no myko patch needed.

- **Writes.** `myko::server::Persister` is a public trait with one required
  method, `persist(&self, MEvent)`. The daemon already installs its own
  persisters per entity (`with_persister_override`) and a default
  (`with_default_persister`), which is where the blackhole goes today.
- **Replay.** `CellServerCtx::apply_event_batch(Vec<MEvent>)` (myko 6.11.5,
  `server/context.rs:960`) applies events to the store with relationship
  cascades. The JSONL persister removed in c66db28 replayed through exactly this
  call.
- **Ordering.** The cleanup sweeper must not run before the store is loaded
  (MUS-58), or it reaps everything as stale. With Postgres it waits for myko's
  catch-up. A file replay is synchronous and runs before the server is handed
  out, so the sweeper can't start early. The sweeper's start condition gets one
  more case: "loaded from the file".

Why the old JSONL persister was removed (c66db28), and how this design answers
each reason:

| Then | Now |
|---|---|
| It grew forever: a full `Session` SET every ~5 s per session | `SessionLivenessFilter` (lv-6731) already drops liveness-only SETs before any durable persister. Compaction (below) bounds the rest |
| Boot replayed the whole file into memory | Compaction keeps the file at about one line per live entity, so boot reads the current state, not its history |
| A file watcher tailed the log for hot reload | Not needed. One daemon owns the file: it holds an exclusive lock on it (`flock`, as `credentials.rs` does for invites and bindings), and a second daemon on the same state directory refuses to start, naming the first. Nothing takes such a lock today |

## What survives a restart

The rule: **whatever Postgres would keep, the file keeps**. The file persister
takes the default persister's place, so today's overrides stay as they are, and
the two modes can't drift.

| Entity | Kept | Why |
|---|---|---|
| `Message` | yes | the point |
| `MessageRead` | yes | otherwise every kept message comes back unread |
| `Room`, `RoomMember` | yes | room mail cascades with its room, and ad hoc memberships must survive (MUS-58) |
| `Session` | yes, through `SessionLivenessFilter` | direct mail is addressed by session id; a returning session is matched by stable identity and takes over its predecessor's unread mail (MUS-46, MUS-47) |
| `SessionIdentity`, `SessionNickname`, `Succession`, `MessageLink`, `SandboxRun` | yes | same as Postgres: identity and takeovers must survive for MUS-46/47 to find a returning session |
| `Client`, `Server`, `WakeWatcher`, `WakePause`, `RemoteSession` | no | blackholed today: connection bookkeeping that would point at dead sockets after a restart |

After a restart, mail, read state, rooms and memberships come back. Sessions
come back as stale rows that the sweeper's reconnect grace either revives or
reaps, which is what happens with Postgres today. The 14-day message sweep keeps
working, because its DELs are persisted too.

## Options

### A. An append-only file with compaction (recommended)

`$MUSTER_STATE_DIR/store.jsonl`, one `MEvent` per line, the same JSON myko
already serializes.

- **Write.** `persist` appends one line with `O_APPEND`. Lines are flushed at
  once and `fsync`ed in batches (at most every 100 ms, and at once for
  `Message`), so a crash loses at most a fraction of a second, never a
  half-written state.
- **Boot.** Read the file and fold it to the last SET per entity, dropping DEL'd
  ones. Apply the result with `apply_event_batch`, then write that result as a
  new file and `rename` it over the old one (atomic on the same filesystem). A
  torn last line from a crash is logged and skipped. Any other line that doesn't
  parse, or names an entity kind not in the list above, stops startup with the
  line number, rather than silently dropping mail.
- **Compaction while running.** When the file is more than 4 times the size of
  the last compaction and more than 1 MB, rewrite it from the in-memory store on
  a background task, the same way as at boot. Also once a day, so deleted
  bodies don't sit on disk for long (see the threat model).
- **Cost.** No new dependency. Most of the code can be borrowed from the
  `DiskPersister` in c66db28 (replay, dedup, torn-line handling), minus its
  watcher and migration helpers.

### B. SQLite

`$MUSTER_STATE_DIR/store.sqlite`: one table keyed by (entity kind, id) holding
the latest JSON, with an upsert on SET and a delete on DEL. Boot is one
`SELECT`.

- Crash safety and compaction come free (WAL, no history kept).
- But it's a new C dependency (`rusqlite` with bundled SQLite). That's one more
  C build for the Windows cross-check, whose mingw toolchain has been the
  flakiest part of CI (MUS-91), and for the new macOS job (MUS-13). It's also
  another crate for `cargo deny`, and a binary format that can't be inspected
  with `grep` when diagnosing lost mail.

### C. Ask upstream for a file persister in myko

The cleanest home in the long run, but it waits on another team's release, and
the upstream round after #164 is already busy. Worth raising once A is in use.

**Recommendation: A.** It's no new code path for Postgres users and no new
dependency. Its two historical problems are already solved (the liveness filter)
or solved here (compaction). B is a reasonable alternative if the reviewers
would rather pay for a C dependency than own compaction code.

## Turning it on

- `MYKO_POSTGRES_URL` set: Postgres, exactly as today. The file is not used.
- Otherwise: the file, unless `MUSTER_PERSIST=off`, which gives today's ephemeral
  mode. Tests that rely on starting empty set it.
- The startup line names the mode: `persistence: file (…/store.jsonl, N
  entities loaded)`, `postgres`, or `ephemeral (MUSTER_PERSIST=off)`.

**Question for the reviewers:** should the file be the default (as above), or
opt-in (`MUSTER_PERSIST=file`) for a release first? Defaulting to it is what
makes the done-when true for everyone, but it changes what a plain `serve`
leaves on disk.

## Threat model changes

Today, without Postgres, nothing a session says touches the disk. With the file:

1. **Message bodies at rest.** Mail, room names, nicknames, `current_task`
   text, hosts, operators and project names are written in plain text to the
   state directory. That directory already holds the local token, machine tokens
   and pairings, which are more sensitive, and it's in the operator's own trust
   zone ("A compromised session" in threat-model.md can already read it). So no
   new attacker gains access. But the cost of a disk image, a backup or a stolen
   laptop goes up from "tokens" to "tokens and two weeks of mail".
   - The file is created `0600` and the directory `0700`, as the token files
     are. Startup refuses a store file that another user owns or that's readable
     by the group or others, naming the fix.
   - Pairing codes and tokens never reach it: every send path already redacts
     them from bodies before storing (MUS-57: `send_message`,
     `broadcast_message`, `links`).
   - It isn't encrypted. A key would sit in the same directory as the file, so
     encryption would add nothing against the operator's account. Against disk
     theft, full-disk encryption is the answer, and the install doc should say
     so.
2. **Deleted mail lingers until compaction.** A message the 14-day sweep (or a
   room's deletion) removes stays in the file until the next compaction: at most
   a day, given the daily compaction. Compaction rewrites the file and renames
   it; the old blocks aren't scrubbed, which no filesystem guarantee would make
   reliable anyway.
3. **Backups and sync carry mail off the machine.** `~/.local/state` is often in
   Time Machine and sometimes in a synced home folder. The install doc should
   say the file is there, and `MUSTER_STATE_DIR` can move it. On macOS, the file
   can be excluded from Time Machine with `tmutil addexclusion`; that's worth
   doing by default, so a question for the reviewers.
4. **Replay bypasses the gate.** `apply_event_batch` trusts its input: lines in
   the file skip the frame filter, rate limits and ownership checks. Anyone who
   can write the file can plant mail "from" any session. That's the same
   account that can already read the local token and act as any local session,
   so it's no new power. But it makes the file a convenient place to do it
   quietly. Mitigations: the permission check above, refusing unknown entity
   kinds, and logging the loaded counts per kind at startup.
5. **Disk use.** Bounded by the 14-day TTL and compaction. It's also capped: past
   64 MB after compaction, the daemon logs a warning each hour naming the
   largest kinds, rather than refusing writes and losing mail.

`docs/threat-model.md` gets a paragraph under "What's protected" and a known gap
for (1) and (3) when the code lands.

## Tests (when the code is written)

- Unit: fold and compaction (last SET wins, DEL removes, a DEL'd room takes its
  messages), a second daemon on the same file is refused, a torn last line is skipped, a bad middle line stops startup, an
  unknown kind is refused, permissions are checked, and Postgres configured
  means no file is opened.
- Conformance: cell 5b (`restart-keeps-unread-mail`) gains a no-Postgres
  variant. Mail sent while a session is busy, then a restart with the file, must
  be kept and wake the session once it goes idle. Today that row only runs with
  Postgres.
- e2e: `scripts/test-restart-e2e.sh` runs once more without Postgres, covering
  the MUS-58 room membership and an unread direct message.
- The `daemon-restart-mid-wait` cells that expect ephemeral behaviour set
  `MUSTER_PERSIST=off`.

## Out of scope

The Postgres path (sturdy-meerkat and ultra-dune are working there). Encrypting
the store. Moving an existing Postgres database to a file, or back.
