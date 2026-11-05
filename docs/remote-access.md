# Reaching the daemon off the home LAN (MUS-16)

Status: design, for the watchman and the operator. No code yet. The attackers are the
ones in [threat-model.md](threat-model.md). This doc also bears on MUS-56, which limits
Murph's ports to listed devices.

## The problem

A pairing holds one gate address, such as `192.168.1.241:6255`. On the home LAN that
works. Away from home, a laptop can't reach that address, so its sessions go quiet: the
shim reconnects, the hooks give up after about two seconds, and nothing arrives until the
laptop is home again. MUS-16 is done when a paired laptop stays connected as it moves
between home and a Tailscale or WireGuard network.

What already helps:

- **The certificate is pinned by fingerprint**, not by name. TLS checks the fixed name
  `muster-daemon` and the pinned SHA-256, so the same pairing is just as safe at any
  address that reaches the gate.
- **Each tunnel connection dials afresh.** `start_tunnel` connects to the gate for every
  local connection, and myko's client reconnects after a drop. So a change of address
  takes effect at the next reconnect, with no restart.
- **`connect_any` tries every address a name resolves to** (since 090b2a9), but only for
  the hooks' blocking posts. The tunnel uses `TcpStream::connect`, which also tries each
  resolved address in turn.

Today neither a1 nor Murph has Tailscale or WireGuard installed (checked on 2026-11-05).
So every option below starts with the operator setting up an overlay network. Muster
shouldn't open a port to the internet. The roadmap rules out the open internet.

## Options

### A. One overlay address for everything (no code)

Install Tailscale (or WireGuard) on Murph and the laptop. Pair the laptop with the
overlay address, such as `--address 100.x.y.z:6255` or a MagicDNS name, and bind the gate
where that address arrives.

- **For:** no Muster change. Tailscale takes the direct LAN path at home, so it costs
  little there.
- **Against:** every Muster connection depends on the overlay being up, even at home. If
  Tailscale is logged out or down, the laptop is cut off from a daemon a few metres away.
- **Binding:** `MUSTER_BIND` takes one address, so the gate binds `0.0.0.0:6255` to serve
  both the LAN and the overlay. The firewall (MUS-56) then decides who reaches it.

### B. Several addresses in a pairing, tried in order (small code change)

The pairing gains `addresses: Vec<String>`, with the old `address` kept as the first
entry so existing pairings and v1/v2 codes still work. For example, the LAN address first,
then the overlay address. The tunnel and the hooks try each in order with a short connect
timeout, about 2 s, and remember the last one that worked so the next connection starts
there.

- **Usage:** `muster-daemon pair --address 192.168.1.241:6255 --address 100.x.y.z:6255`
  puts both in the code, and `muster-shim pairing` shows both. To add one to an existing
  pairing without re-pairing, use `muster-shim pairing --add-address 100.x.y.z:6255`.
  That's safe without the daemon's say-so, because the pinned fingerprint still has to
  match.
- **For:** home works without the overlay, and away works through it. The switch happens
  at the next reconnect.
- **Against:** a small code change in `link.rs` and the pairing format, plus tests. When
  the first address is unreachable, a fresh connection waits up to one connect timeout
  before falling through. Remembering the last good address avoids paying that every time.

### C. A name that resolves differently at home and away (no Muster code)

Split DNS: `murph.home` resolves to the LAN address at home and the overlay address
away, for example through Tailscale's split DNS or a router DNS override. `connect_any`
and `TcpStream::connect` already try every address a name resolves to.

- **For:** no Muster change.
- **Against:** it moves the logic into DNS config on two networks, which is harder to see
  and to debug ("why can't it connect?"). Resolver caches can also hold the wrong answer
  after a move.

### Recommendation

**B.** It keeps home independent of the overlay, it's visible in `muster-shim pairing`,
and it's a contained change. Start with **A** as the quickest way to get a laptop working
away from home today, then move to B. Choosing an overlay (Tailscale or WireGuard) and
installing it is the operator's call either way.

## Threat model

- **Who can reach the gate.** Today that's the LAN, and MUS-56 narrows it to listed LAN
  devices. With an overlay it's also every device on the tailnet or WireGuard peer list,
  so the overlay's own access control is now part of the boundary.
  - With Tailscale, use an ACL that lets only the operator's laptop reach Murph's port 6255.
  - On Murph, the firewall accepts 6255 on the overlay interface (`tailscale0` or `wg0`)
    only from the laptop's overlay address. That's the MUS-56 rule, extended to a second
    interface.
- **What reaching it gets.** Nothing without a pairing token. TLS comes first, with a
  pinned certificate, and an unpaired client gets 401. That doesn't change.
  - A tailnet device that steals a laptop's token is the existing "stolen paired machine"
    case. `muster-daemon unpair` revokes it.
- **Binding `0.0.0.0`.** It widens the listening interfaces to any added later. The
  firewall rule must be default-deny on 6255, with explicit accepts per interface and
  source. Option B doesn't need `0.0.0.0` if the daemon binds a list. That would mean
  `MUSTER_BIND` taking several addresses, a small daemon change worth doing with B.
- **Per-IP limits.** These still work, keyed on the overlay address.
- **Address hijack.** Something else on the LAN or overlay could take an address in the
  list. The pinned fingerprint makes that a failed connection, not a leak (`PostFailure::Certificate`).
  The token is only sent after the TLS handshake verifies.
- **Out of scope still:** the open internet. Muster isn't to be port-forwarded. A laptop
  away from home without the overlay simply can't connect, and that's intended.

## Run to show it's done

On the operator's laptop: pair with both addresses (option B) and confirm a session
works at home. Then leave the LAN, or turn Wi-Fi off and tether, with the overlay up.
Show that within a minute the same session is back on `muster://roster`, bound to its
connection, and that a message sent to it arrives and wakes it. Then come home with the
overlay off, and show it reconnects on the LAN address. MUS-56's run is extended too: an
overlay device that isn't the laptop can't reach 6255.

## Questions for the operator

1. Tailscale or WireGuard? Tailscale is less setup and gives ACLs and MagicDNS.
2. Option B, with A as a stopgap, as recommended, or A alone?
3. For MUS-56 on the overlay: is it only the laptop, or a1 too when it's away?
