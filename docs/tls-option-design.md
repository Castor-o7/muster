# Design: the TLS gate as an option (G3)

Status: **approved by the watchman (grand-photon), 2026-11-04.** modest-roach
implements it once G1 lands. The decisions on the open questions are under
"Decisions" below. **Built in Muster as MUS-64 (#89).** For upstream the operator chose
`MARSHAL_TLS=1` (2026-11-04). Names are Marshal's, since this is for the
upstream Gate PR (see [muster-vs-marshal.md](muster-vs-marshal.md)).

## How upstream's daemon is configured today

At Marshal `94ecc24`, `marshal-daemon` takes **no command-line arguments**.
It has no argument parser and no subcommands. Everything comes from the
environment (`crates/daemon/src/main.rs`):

| Variable | Default | Purpose |
|---|---|---|
| `MARSHAL_BIND` | `127.0.0.1:6155` | myko's WebSocket/MCP listener, resolved through DNS so a mesh name works |
| `MARSHAL_HOOK_BIND` | `127.0.0.1:6156` | the plain-HTTP `/hook/*` listener |
| `MYKO_POSTGRES_URL` (+ myko's `PostgresConfig` vars) | unset, so ephemeral | persistence |
| `RUST_LOG` | set by `init_logging` | logging |

Upstream's security comes from where things are bound. A comment calls
`asSession` "trust-on-assert" and says exposure must be "an explicit act":
you set `MARSHAL_BIND` to a mesh address. There is no authentication on
either port.

Muster added hand-rolled subcommands (`pair`, `unpair`, `pairs`, `pause`,
`resume`, `allow`, `disallow`, `senders`) and always runs the gate. Upstream
would get both, with the gate behind a switch.

## The switch

The operator turns it on with either of these:

- `marshal-daemon --tls`
- `MARSHAL_TLS=1` (also `true`/`on`; `0`/`false`/`off`/unset mean off)

Both are implemented in Muster. **For upstream the operator chose
`MARSHAL_TLS=1` (2026-11-04)**, so the upstream Gate PR ships the variable only,
with no `--tls` flag, in keeping with upstream's daemon taking no arguments.
Muster keeps both for now. If both
are given and disagree (e.g. `--tls` with `MARSHAL_TLS=0`), the daemon
**refuses to start** and names both values:
`--tls and MARSHAL_TLS=0 disagree; set one`. Agreeing values are fine. The env var matches upstream's existing style and works in
systemd units and launchd plists without editing `ExecStart`. The flag is
visible in `ps` and in the unit file.

At startup the daemon logs one line naming the mode, e.g. `TLS gate: off
(plain WebSocket on 127.0.0.1:6155; set MARSHAL_TLS=1 or --tls to require
pairing)`.

## Off (the default): today's Marshal

With TLS off the daemon must behave exactly as v0.54.1 does, apart from
features that don't depend on the gate (Hooks, Durability, Other):

| Thing | Off |
|---|---|
| 6155 (`MARSHAL_BIND`) | myko's own WebSocket/MCP listener, plain, as today |
| 6156 (`MARSHAL_HOOK_BIND`) | the hook listener, plain, **no token**, as today. A remote machine whose hooks post here over a mesh keeps working |
| 6157 local gate, 6158 myko-behind-gate | **not bound** |
| TLS certificate | not generated |
| Local token | not created, not required. A token a client sends anyway (from an old file) is ignored, not rejected |
| myko request filter | not installed |
| Frame filter, host/operator attestation, session binding | off: `asSession` stays trust-on-assert, as today |
| Rate and connection limits (MUS-9/60) | off, for parity with today, in the first Gate PR. Turning them on in plain mode is a separate proposal |
| `pause` / `resume` | **work.** They don't depend on the gate |
| `allow` / `disallow` / `senders` | inert: with no pairing there are no machine identities. They print `sender allow-list applies only with TLS on` |
| `pair` / `unpair` / `pairs` | edit the state dir as usual, and print `pairing takes effect when the daemon runs with TLS (--tls or MARSHAL_TLS=1)` |
| Origin refusal | off. The Marshal web UI is a browser client (and #163 serves it over HTTPS), so refusing `Origin` would break it |

## On: what the switch adds

| Thing | On |
|---|---|
| 6155 (`MARSHAL_BIND`) | **the gate**: TLS with the self-signed certificate in the state dir, pinned through the pairing code. A paired machine presents its token; the frame filter lets it act only as its own sessions; per-IP and per-machine connection caps |
| local tools on the same port | see "One port for local and remote" below |
| 6156 (`MARSHAL_HOOK_BIND`) | the hook listener. It needs the local token (`Authorization: Bearer`) and refuses any `Origin`. Paired machines reach the hooks through the gate on 6155, not directly |
| 6158 (`MARSHAL_MYKO_BIND`) | myko itself, loopback only, admitting only requests a gate forwarded (the myko request filter). Not for clients |
| Local token | created in the config dir (owner-only) on first start |
| Rate and connection limits, sender allow-list | on |
| Pairing codes | single-use and expiring, as in Muster |

Switching from on to off leaves the certificate, tokens and pairings on disk,
untouched, so turning TLS back on restores them. Only `unpair` revokes a
pairing.

## One port for local and remote

Muster puts the gate on 6255 and the local tools' plain path on a separate
loopback port (6257). If upstream copied that, the shim's default address
would depend on the mode: `ws://localhost:6155` when off, `:6157` when on.
The shim would then need a way to know the mode, and any stale signal for it
(a leftover token file, an old env var) would send it to the wrong port.

**Recommendation: keep one port, 6155, and have the gate tell the two kinds
of client apart by their first byte.**

- `0x16` (a TLS ClientHello): the TLS gate path, from any address.
- Plain HTTP from a **loopback** peer: the local-gate path (local token
  required, `Origin` refused). This is Muster's `run_local`, behind the same
  listener.
- Plain HTTP from a non-loopback peer: answer
  `426 Upgrade Required: this daemon requires pairing; run marshal-daemon pair --name <machine> on its host`,
  then close.

The result: local clients (shim, TUI, UI, hooks) use the same address in both
modes and only add the local token when the file exists. An unpaired remote
machine gets a clear message instead of a failed TLS handshake. 6157 goes
away, which makes the proposal in the inventory moot. 6158 stays, because
myko has to listen somewhere the gate can forward to.

Fallback if the watchman or upstream prefers not to multiplex: a separate
`MARSHAL_LOCAL_BIND` (6157), with the daemon writing `daemon-mode`
(`tls`/`plain`) to the config dir at startup for local clients to read. It
is more moving parts for the same behaviour.

## How the shim knows its mode

The shim never has to be told. It decides from what it already has:

1. **A pairing file exists** (`~/.config/marshal/pairing`): connect over TLS
   to the address in the pairing, pinning its certificate. This is a remote
   machine on a TLS daemon.
2. **No pairing**: connect plain to `MARSHAL_DAEMON` / the config file /
   `ws://localhost:6155`, and add `token=<local token>` if
   `~/.config/marshal/local-token` exists.
   - On a TLS-off daemon the token is ignored, which works.
   - On a TLS-on daemon on the same host, the gate's loopback path accepts
     it, which also works.
   - On a TLS-on daemon on another host, the client gets the 426 above, and
     the shim reports `the daemon requires pairing: run marshal-shim pair <code>`.
3. **Paired, but the daemon runs plain** (TLS turned off, or the wrong
   address): the TLS handshake fails against a plain listener. The shim
   reports
   `paired with <name>, but <address> isn't serving TLS: turn TLS back on, or remove the pairing with marshal-shim unpair`,
   rather than retrying silently.

The hooks (`codex_hook::http_post` / `https_post`) follow the same rules:
paired → through the gate; otherwise plain, with the bearer token if the file
exists.

## Daemon to daemon (G5) is TLS-only

Linking two daemons (G5) works only with TLS on, on both. A daemon link
carries other machines' sessions and mail, so it needs what only the gate
provides: a pinned certificate, a per-machine token, and the frame filter
limiting what the far side may claim. With TLS off the daemon refuses to
start a link, and refuses an incoming one, with
`daemon links need TLS on both daemons (--tls or MARSHAL_TLS=1)`. There is no
plain-mode fallback.

## Threat model changes

`docs/threat-model.md` gets this when the code lands:

- **A reverse proxy on the daemon's host makes remote traffic look like
  loopback.** The single-port design sends plaintext from a loopback peer to
  the local path. A proxy on the same host, such as an HTTPS front end for the
  web UI (#163), forwards remote requests from `127.0.0.1`, so the gate can't
  tell them from a local tool by address. This is still safe: the local path
  requires the local token, which a remote client doesn't have, and refuses
  any `Origin`, which a browser behind the proxy sends. But the loopback
  check alone is not a trust boundary, and the docs must not call it one.
  The token is.
- With TLS off, the threat model doesn't apply: the daemon is upstream's
  trust-on-assert design, protected only by where it is bound. The docs say
  so, and the startup line names the mode.

## Decisions (watchman, 2026-11-04)

1. **One port with first-byte multiplexing:** yes. The fallback is dropped.
2. **Flag and env conflict:** refuse to start, naming both values. **The
   operator's choice (2026-11-04): `MARSHAL_TLS=1` for upstream.** The upstream
   PR has no flag, so it has no conflict to refuse.
3. **Rate limits:** off in plain mode in the first Gate PR, for parity, and
   proposed separately.
4. **The web UI:** the first Gate PR documents "the UI requires TLS off". A
   UI route through the gate comes later.
5. **The myko filter:** myko is ignition-is-go's own crate
   (ignition-is-go/myko), so the natural route is a request-filter hook
   there. Asking for one is an upstream PR, and therefore the operator's call,
   later. Until then Muster keeps the vendored patch, rebased on 6.11.5.
6. **G5:** TLS mode only (above).

## Open questions (as first raised)

1. **The web UI with TLS on.** The Marshal UI is a browser page. Under the
   gate it would need its own route: say, the gate serving the UI and
   checking a cookie set from the local token. Otherwise the UI is supported
   only with TLS off. Muster hasn't solved this (its UI constant still points
   at the gate port). Proposal: document "UI requires TLS off" in the first
   Gate PR, and design the route afterwards.
2. **The myko request filter.** On, it needs either upstream myko-server to
   gain a request-filter hook (preferred), or the vendored patch rebased on
   6.11.5. Until then, a first Gate PR could ship without the 6158 hardening:
   myko on loopback with no filter, as in Muster before MUS-59. The threat
   model would then have to list a local web page as unmitigated for that
   port.
3. **Flag over env.** Is "the flag wins when they disagree" right, or should
   the daemon refuse to start when they conflict? Refusing is stricter and
   needs no log line to be noticed.
4. **Rate limits without TLS.** Should per-session message limits be on in
   both modes? They protect against a runaway agent, not a network attacker.
