# Patched myko-server 6.9.0 (MUS-59)

This directory is myko-server 6.9.0 from crates.io, licensed AGPL-3.0-or-later
(see its `Cargo.toml`; full text: https://www.gnu.org/licenses/agpl-3.0.txt).
It keeps that license. It is not covered by Muster's MIT or Apache-2.0 license.

Muster carries it through `[patch.crates-io]` in the workspace `Cargo.toml`
for one change: a request filter.

- `router::RequestFilter`, a check on each parsed request head, run before
  any route. A refusal is answered with its status and the connection closes.
- `MykoServerBuilder::with_request_filter` sets it. Without one, the server
  behaves exactly as upstream 6.9.0.

The daemon uses it so myko's own listener refuses any request carrying a
browser `Origin`, and any request without the host's local token, which only
Muster's gates add when they forward (MUS-59).

Removed from the crate as published: `Cargo.lock`, `examples/`, `tests/`,
`flux.toml`, and the example, test and dev-dependency entries in `Cargo.toml`.

Drop this directory and the `[patch.crates-io]` entry once upstream myko-server
ships an equivalent hook, and use that instead.
