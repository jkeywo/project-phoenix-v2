---
title: GM Operator
type: entity
tags: [gm, operator, identity, reconnect, roster, readiness, force-start, host-mesh]
sources: [src/gm_roster.rs, src/lobby/start_policy.rs, src/core/messages.rs, src/lobby/server.rs, src/server/bridge.rs, gui/host-mesh.js, gui/fleet-session.js, gui/lobby-state.js, server.html, client.html]
updated: 2026-08-31
---

# GM Operator

A GM Operator is a privileged host-class participant admitted only through the
privileged fleet-host path. It is deliberately not a [Player](./player.md),
Spectator, Station holder, or player ship. Several GMs may be present and all
have the same role; the host mesh's technical star centre and private mesh slot
do not create a public leader or permission tier.

## Identity and roster

The fleet owner mints two separate identities for a GM:

- a stable public operator id such as `gm-1`, projected with only `name`,
  `connected`, and `ready`; and
- a private opaque reconnect capability retained by that operator's server
  page and presented on a later privileged reconnect.

Rendezvous peer ids and technical mesh slots are private transport details.
Neither they nor the reconnect capability enter the public roster. While the
Lobby topology is still mutable, a known disconnected GM can bind a new
connection to the same operator row without consuming a ship slot or displacing
a connected operator. Once frozen, that technical slot is part of Rust's
deterministic wait-set. A departed GM is then refused `recovery-only` even with
the correct capability until #1294 can restore the authoritative snapshot,
watermark, and any one-shot start boundary it missed.

## Authoritative projection

`GmRoster` is the Rust-side bounded, canonical full-replacement projection. The
browser fleet layer sends only public rows through `wasm_set_gm_roster`; Rust
validates unique ids, bounds names and row count, and broadcasts changes through
`Welcome`, `GmRosterChanged`, and the host lobby channel. Host and crew lobby
views render GMs in their own labelled presence group, outside Station,
Spectator, and crew-capacity calculations. Disconnect and reconnect retain the
operator id but clear readiness.

## Lobby readiness and force-start

Connected GM Operators participate in the same collective start policy as
connected non-Spectator crew. Every connected participant must be ready for an
automatic start, including a GM-only roster. Any connected GM may request a
force-start that bypasses readiness but never content, compatibility, preload,
or peer validation. The mesh authenticates the requesting connection and emits
one attributed, idempotent proposal. The proposal carries no apply tick. Rust on
the technical owner assigns the safe exact tick and puts the grant in its
authenticated `TickFrame`; every member adopts that canonical value. The
star-centre host does not gain a leader permission.

This implementation establishes admission, reconnect identity, presence, and
the collective lobby-start policy. It also establishes that a GM's private
technical slot is a full lockstep participant despite there being no
corresponding ship row. The dedicated renderer-free GM simulation surface is
#1291: #1290 does not claim convergence for renderer-derived authoritative
`ModelMarkers` and does not infer a headless runtime merely from the role
selector.

## Related

- [Session](./session.md)
- [Player](./player.md)
- [Networking](../concepts/networking.md)
- [Server Lobby UI](../concepts/server-lobby-ui.md)
