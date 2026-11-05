# Authorization inside myko (MUS-41)

Status: design, for the watchman and the operator. No code, and nothing proposed upstream
yet. It ends with the route for the operator to choose.

## What the gate's frame filter does today

`crates/daemon/src/gate_filter.rs` reads every WebSocket frame a paired machine sends
before myko sees it, and forwards only what a shim needs. Everything else is refused,
including anything it can't parse.

| Frame | Allowed | Refused |
|---|---|---|
| Query | `GetAllSessions`, `GetAllRooms`, `GetAllRoomMembers`, `GetAllSessionNicknames`, `GetAllWakeWatchers`, `GetAllWakePauses`, `GetAllRemoteSessions`, with an object body. Every query from the daemon's own host (`(local)`) | Any other query, such as `GetAllMessages`. Messages are read through `ReadMessages`, which checks the caller |
| Command, acting as the caller | `SendMessage`, `BroadcastMessage`, `ReadMessages`, `AckMessages`, `JoinRoom`, `LeaveRoom`, if `asSession` is absent or a session this machine owns | A body that isn't an object, or an `asSession` this machine doesn't own |
| Command on a session | `SetSession*` setters whose `id` is a session this machine owns | Any other command |
| Event, batch | `Session` events whose `id` this machine owns. The gate overwrites `host.name` with the machine's pairing name and `operator` with `MUSTER_OPERATOR` | Events on any other entity, such as `Message`, `Room`, `WakeWatcher` or `Succession` |
| Anything | — | NUL in any key or value; unparseable frames; `View`, `Report`, `Benchmark` |

Ownership is "first paired machine to use a session id", from the bindings file
(`credentials::claim_machine_session`). Paired machines may bind only UUIDs.

## Why it's fragile: the filter and myko parse separately

The filter decides on its own parse of a frame, and then myko parses the original
bytes again and acts on its own reading. Wherever the two readings differ, there's a
bypass. The 2026-10-30 adversarial review (`docs/session-binding.md`) found three:

1. **A NUL-suffixed key.** `"id\u0000"` passed the filter as an unknown key. myko's
   `sanitize_null_bytes` then stripped the NUL, so it became `id` and replaced the id
   the filter had checked. The fix refuses NUL anywhere.
2. **A tab in the request line.** It lost the machine name: the gate's reading of the
   request head and the identity it attaches came apart. The review records only that
   much. The fix is in the gate, with a regression test, and the exact mechanism isn't
   restated here.
3. **A command body sent as a JSON array.** serde fills a struct from an array by
   position, so `asSession` arrived with no `asSession` key for the name-based check to
   see. The fix refuses non-object bodies.

Each fix closes one difference. None closes the class: any future myko change in how it
parses, normalizes or deserializes reopens it. Examples are a new normalization step in
`normalize_incoming_event`, or a new message variant. The filter also has to be kept in
step with what the shim sends, by hand.

## What a myko-side hook needs

Check **after** myko's own parse, sanitize and normalize, on exactly the values myko is
about to apply, with an identity myko attached to the connection itself.

1. **An identity on the connection.** The patched `RequestFilter` (MUS-59) already sees
   each request head before routing. Let it return a principal instead of just
   allow/refuse, for example `Principal { machine: "a1", local: false }`. That's built
   from the header only the gates add (`X-Muster-Machine`, which the gate strips from
   clients and sets from the verified token) and the local token. myko keeps it on the
   `ClientSession` and puts it in each command's `RequestContext`.
2. **An authorizer, set on the builder:** `MykoServerBuilder::with_authorizer(Arc<dyn
   Authorizer>)`, with three checks:
   - `authorize_event(&Principal, &MEvent)`, called in `ws_handler` **after**
     `sanitize_null_bytes` and `normalize_incoming_event`, before `apply_event` or
     `apply_event_batch`. A batch is refused whole if any event is.
   - `authorize_command(&Principal, command_id, &Value)`, called **after** the command
     is deserialized into its type and re-serialized to its canonical form. Positional
     arrays, duplicate keys and unknown keys then can't differ from what the handler
     runs.
   - `authorize_query(&Principal, query_id, &Value)`, called before the query is
     registered.

   A refusal answers with myko's usual error frame and is logged without the payload.
   With no authorizer, behaviour is exactly as upstream.
3. **The same hook on every route.** That includes the HTTP-MCP router. Muster closes
   that route to paired machines today, and with the hook it could be opened with the
   same checks.

Muster's authorizer is then today's rules, moved: the allowlists, ownership through the
bindings file, and the `host` and `operator` attestation. Attestation is a rewrite, not a
check, so it needs either a `normalize` callback beside `authorize_event`, or the
authorizer refusing a `Session` event whose `host.name` or `operator` doesn't match the
principal. The second is simpler, and the shim already sends its real host name, so it
breaks nothing. The gate then stops reading frames and only copies bytes and sets the
header, and `gate_filter.rs` is retired. That's MUS-41's done-when: myko refuses the
writes itself, a test shows it, and the filter is gone.

What it doesn't fix: bypass 2 lives in the gate's own HTTP parse, which still sets the
header. It stays the gate's to get right. The hook makes that header the single input
that has to be right, instead of every frame.

## The two routes

### Upstream: propose the hook to myko's maintainers (lucid-max)

- **What:** a proposal, with the design above and a small reference patch, for a
  principal from the request filter and an `Authorizer` with the three checks. Sent
  under the operator's direction, not by a session.
- **Cost:** writing the proposal and the patch, and review time on their side, with no
  timeline. Their design may differ, and Muster would adapt. The request-filter hook
  (MUS-59) is still waiting to go upstream too, and it makes sense to propose the two
  together, since the principal comes from the filter.
- **Gain:** the vendored copy goes away on the next myko upgrade. myko's other users get
  the same protection.

### Carry it in the vendored patch meanwhile

- **What:** extend `vendor/myko-server` (already carried for MUS-59) with the principal
  and the `Authorizer`, and drop the gate's filter.
- **Cost:**
  - The patch grows from one small filter into `ws_handler` (event, command and query
    paths) and the MCP router. Every myko upgrade then means a rebase through code
    myko changes often; the 6.9.0 to 6.11.5 rebase already had to thread the filter
    through a new private routing function.
  - The vendored copy stays AGPL-3.0-or-later in its own directory, as it is now. That
    doesn't change Muster's licence position, since Muster already depends on AGPL myko
    crates.
  - Tests:
    - the three bypasses as regression tests against the hook;
    - every filter test moved to the authorizer;
    - the gate e2e unchanged.
- **Gain:** the class of bypass closes now, not on someone else's timeline.

### Both

Carry it now, and propose it upstream along with MUS-59's filter, dropping the vendored
copy once upstream ships an equivalent. This is the same shape as MUS-59's option (c),
which the operator chose then.

## Question for the operator

Which route: upstream only (keep the filter until myko ships a hook), carry it in the
vendored patch, or both (carry now, propose upstream with MUS-59's filter, drop the patch
when upstream ships)? The recommendation is **both**, as with MUS-59.
