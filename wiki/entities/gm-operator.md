---
title: GM Operator
type: entity
tags: [gm, operator, identity, reconnect, roster, host-mesh]
sources: [src/gm_roster.rs, src/core/messages.rs, src/lobby/server.rs, src/server/bridge.rs, gui/host-mesh.js, gui/fleet-session.js, gui/lobby-state.js, server.html, client.html]
updated: 2026-08-31
---

# GM Operator

A GM Operator is a reconnectable participant admitted only through the
privileged fleet-host path. It is deliberately not a [Player](./player.md),
Spectator, Station holder, or player ship. Several GMs may be present and all
have the same role; the host mesh's technical star centre and private mesh slot
do not create a public leader or permission tier.

## Identity and roster

The fleet owner mints two separate identities for a GM:

- a stable public operator id such as `gm-1`, projected with only `name` and
  `connected`; and
- a private opaque reconnect capability retained by that operator's server
  page and presented on a later privileged reconnect.

Rendezvous peer ids and technical mesh slots are ephemeral transport details.
Neither they nor the reconnect capability enter the public roster. A known
disconnected GM can therefore bind a new connection to the same operator row
without consuming a ship slot or displacing a connected operator.

## Authoritative projection

`GmRoster` is the Rust-side bounded, canonical full-replacement projection. The
browser fleet layer sends only public rows through `wasm_set_gm_roster`; Rust
validates unique ids, bounds names and row count, and broadcasts changes through
`Welcome`, `GmRosterChanged`, and the host lobby channel. Host and crew lobby
views render GMs in their own labelled presence group, outside Station,
Spectator, crew-capacity, and readiness calculations.

This implementation establishes admission, reconnect identity, and presence.
The dedicated renderer-free GM simulation surface is a later runtime layer and
is not inferred from the role selector.

## Related

- [Session](./session.md)
- [Player](./player.md)
- [Networking](../concepts/networking.md)
- [Server Lobby UI](../concepts/server-lobby-ui.md)
