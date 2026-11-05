# Patched myko-server 6.11.5 (MUS-59)

This directory is myko-server 6.11.5 from crates.io, licensed AGPL-3.0-or-later
(see its `Cargo.toml`; full text: https://www.gnu.org/licenses/agpl-3.0.txt).
It keeps that license. It is not covered by Muster's MIT or Apache-2.0 license.

Muster carries it through `[patch.crates-io]` in the workspace `Cargo.toml`
for one change: a request filter.

- `router::RequestFilter`, a check on each parsed request head, run before
  any route. A refusal is answered with its status and the connection closes.
- `MykoServerBuilder::with_request_filter` sets it. Without one, the server
  behaves exactly as upstream 6.11.5.

The daemon uses it so myko's own listener refuses any request carrying a
browser `Origin`, and any request without the host's local token, which only
Muster's gates add when they forward (MUS-59).

Removed from the crate as published: `Cargo.lock`, `examples/`, `tests/`,
`flux.toml`, and the example, test and dev-dependency entries in `Cargo.toml`.

Drop this directory and the `[patch.crates-io]` entry once upstream myko-server
ships an equivalent hook, and use that instead.

Rebased from 6.9.0 onto 6.11.5 on 2026-11-04 (G1 catch-up with upstream
Marshal v0.54.1). 6.11.5 adds `GatewayAccess` and a private
`route_connection_with_paths_and_access`; the filter is threaded through it, so
it still runs before every route, including the read-only MCP refusal. The
public `route_connection` and `route_connection_with_access` pass no filter,
as before; Muster's daemon serves only through the builder's listener.
