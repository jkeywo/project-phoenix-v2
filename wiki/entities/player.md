---
title: Player
type: entity
tags: [player, session, identity, station, reconnect]
sources:
  - src/core/messages.rs
  - src/lobby/session.rs
  - src/lobby/handler.rs
  - wiki/sources/issue-545-c5-retire-player-consoles.md
updated: 2026-06-24
---

# Player

A connected human at the table. The server keys a `Player` by stable session token, not by PeerJS peer ID, and assigns that player to a station rather than to a mutable list of consoles.

## Wire Shape

Defined in `src/core/messages.rs:452`:

```rust
pub struct Player {
    pub token: String,
    pub name: String,
    pub connected: bool,
    pub ready: bool,
    pub station: Option<StationId>,
    pub last_rating: Option<String>,
}
```

The former per-player console list has been retired. Console access is derived from `Player.station` plus the loaded `ShipConfig`: the station owns one console id, and `SessionManager::console_holder` / `player_has_console` resolve that id through the ship config.

## Lifecycle

1. First visit: the client generates a UUIDv4, stores it in `localStorage`, and sends `Identify { token, name }`.
2. The server registers the token, replies with `Welcome`, and announces `PlayerJoined` to other peers.
3. The player sends `SelectStation { station }`; the server writes `Player.station = Some(StationId(...))` and broadcasts `StationAssigned`.
4. The player toggles `SetReady`; when all connected players are ready, the server starts `Loading` or `InProgress`.
5. On disconnect, the player record stays in `SessionManager`, `connected` flips to false, and the station rating changes to `Backfill` so AI can operate station-owned systems.
6. On reconnect with the same token, if no other connected player claimed the old station, the player keeps it and the server restores their `last_rating`; otherwise they reconnect as a spectator/no-station player.

## Identity Vs Presence

- Identity lives in `localStorage` on the device. Same browser, same player, even after refresh.
- Presence is `connected: bool` on the server-side record.
- Station ownership is `station: Option<StationId>`.
- `last_rating` remembers the human rating that should be restored after a Backfill reconnect.

PeerJS peer IDs are ephemeral. The session token is the stable identity. See [Networking](../concepts/networking.md).

## Related

- [Session](./session.md) - server-side storage for players and station ownership.
- [Console](./console.md) - UI/operator surfaces derived from station ownership.
- [Issue #545 C5](../sources/issue-545-c5-retire-player-consoles.md) - the slice that deleted the per-player console list.
