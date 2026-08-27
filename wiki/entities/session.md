---
title: Session
type: entity
tags: [session, server, identity, reconnect]
sources: [src/lobby/session.rs, src/lobby/handler.rs, src/lobby/server.rs, src/server/bridge.rs]
updated: 2026-08-27
---

# Session

`SessionManager` is the authoritative server-side record of every connected or
recently disconnected [Player](./player.md). Identity is a UUID session token
stored by the browser, not the ephemeral PeerJS peer id. The host bridge maps a
peer to that token after `Identify` and passes only the token into simulation
message handling.

## Owned state

Each player record carries connection/readiness state, its directly claimed
station, spectator and AFK state, and the rating snapshots needed to restore a
seat after Backfill. Disconnected records are retained because the same token
must find the original identity on reconnect.

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
- [Station](./station.md)
- [Game Phases](../concepts/game-phases.md)
