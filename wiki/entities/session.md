---
title: Session
type: entity
tags: [session, server, identity, reconnect, readiness]
sources: [src/lobby/session.rs, src/lobby/start_policy.rs, src/lobby/handler.rs, src/lobby/server.rs, src/gm_roster.rs, src/server/bridge.rs]
updated: 2026-08-31
---

# Session

`SessionManager` is the authoritative server-side record of every connected or
recently disconnected [Player](./player.md). It does not contain
[GM Operators](./gm-operator.md), whose privileged host-mesh identity and
public presence live in the separate `GmRoster`. Player identity is the
browser's 32-hex session token — held per tab in `sessionStorage`, with a
persistent `localStorage` copy the first tab adopts — not the ephemeral
rendezvous peer id.
The host bridge maps a peer to that token after `Identify` and passes only the
token into simulation message handling.

## Owned state

Each player record carries connection/readiness state, its directly claimed
station, spectator and AFK state, and the rating snapshots needed to restore a
seat after Backfill. Disconnected records are retained because the same token
must find the original identity on reconnect.

`readiness_tally` is the one crew projection used by the local countdown and
the fleet policy. It counts every connected non-Spectator participant,
including a participant who has not selected a Station, and counts the ready
subset separately. Disconnected players and Spectators contribute to neither
field. GM readiness stays in the separate `GmRoster` and is combined only by
the fleet start policy.

Fleet technical participants are separate again from both records. The frozen
private fleet roster names the ordered owner and every simulation participant,
then lists only the participant slots that also own player ships. This is why a
GM can contribute a lockstep watermark and start vote without becoming a
`SessionManager` player or consuming a ship.

`holder_for_station` returns only a connected direct holder. This distinction
lets a disconnected player retain the station on their record for restoration
without blocking another connected player from claiming the seat.

## Disconnect and reconnect

On disconnect, readiness is cleared. If the player held a station, the handler
records its current rating and changes the station to the runtime-only
`Backfill` rating so AI operates the unmanned systems. AFK and spectator state
survive the transport drop.

On reconnect with the same token:

- if the remembered station is still available, the direct claim and saved
  rating are restored;
- if another connected player has claimed it, the returning player rejoins
  without that station and can select an available seat;
- a spectator remains a spectator until they explicitly claim a seat.

The lobby handler and its server adapter own these transitions and broadcasts;
`SessionManager` supplies the pure identity and occupancy operations.

## Related

- [Player](./player.md)
- [GM Operator](./gm-operator.md)
- [Station](./station.md)
- [Game Phases](../concepts/game-phases.md)
