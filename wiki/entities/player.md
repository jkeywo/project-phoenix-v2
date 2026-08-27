---
title: Player
type: entity
tags: [player, session, identity, station, reconnect]
sources: [src/core/messages.rs, src/lobby/session.rs, src/lobby/handler.rs, src/lobby/server.rs]
updated: 2026-08-27
---

# Player

A Player is one participant record in authoritative session state. The stable
key is the browser's UUID session token, not its current PeerJS connection id.

The wire record carries the player's name, connection and readiness state,
optional directly claimed `StationId`, last human rating for reconnect
restoration, and the public Spectator and AFK flags.

## Presence roles

- A crew participant may hold one direct station. Console access and command
  authority derive from that station plus the selected hull's configuration.
- A Spectator holds no station, is excluded from collective readiness, cannot
  issue simulation commands, and receives only crew-public broadcasts.
- AFK retains the direct station and reconnect identity but delegates all of
  its systems to Backfill and makes that player ineligible to host a visiting
  station. Leaving AFK restores the saved control configuration.

## Lifecycle

1. `Identify` registers or reconnects the token and returns `Welcome`.
2. `SelectStation` claims an available non-auxiliary seat; `SetSpectator`
   explicitly enters or leaves the no-seat Spectator role.
3. `SetReady` participates in collective lobby readiness. During an active
   round, a seatless participant can claim a free station and then confirm the
   handoff from Backfill.
4. Disconnect marks the player absent, clears readiness, keeps the station id
   for restoration, and applies the station's Backfill rating.
5. Reconnect restores the remembered seat and rating when the seat is still
   available. Otherwise the player returns seatless and can choose another.

Disconnected records remain in `SessionManager` because pruning them would
destroy token-based identity and rating restoration.

## Related

- [Session](./session.md)
- [Station](./station.md)
- [Console](./console.md)
- [Networking](../concepts/networking.md)
