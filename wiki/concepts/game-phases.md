---
title: Game Phases
type: concept
tags: [phases, lobby, loading, in-progress, game-over, reconnect]
sources:
  - src/core/messages.rs
  - src/lobby/handler.rs
  - src/lobby/server.rs
  - wiki/sources/issue-542-c6-delete-startgame.md
  - wiki/sources/issue-544-c3-ai-backfill-disconnect.md
  - wiki/sources/issue-544-c4-reconnect-yields.md
updated: 2026-06-24
---

# Game Phases

`GamePhase` is a Bevy state in `src/core/messages.rs:440`:

```rust
pub enum GamePhase {
    Lobby,
    Loading,
    InProgress,
    GameOver,
}
```

## Lobby

- Players connect, set names, and claim stations with `SelectStation`.
- Station ownership is stored as `Player.station: Option<StationId>`.
- Players send `SetReady { ready }`; the legacy captain-only start command is gone.
- When every connected player is ready, the lobby handler moves to `Loading` if assets still need preloading, or directly to `InProgress` if preload is complete.
- When a player **disconnects**, auto-start is re-evaluated: the disconnecting player's ready flag is cleared, then `all_ready()` is checked again. If the remaining players are all ready, the same phase-transition + `GameStarted` broadcast fires (including the `Loading` gate when preload is incomplete). A returning player must re-confirm readiness. (#600)
- The server derives available consoles from unclaimed stations in the loaded `ShipConfig`.

## Loading

`Loading` is the transient asset-preload phase after all connected players are ready. The server broadcasts `LoadingProgress { fraction }` while render assets warm up. Once preload finishes, the server broadcasts `GameStarted` and transitions to `InProgress`.

Reconnect is still accepted during `Loading`; a refreshing player receives `Welcome` and any restored station assignment before play begins.

## In-Progress

- Console handlers process station-authorized inputs.
- Simulation systems run in `SimSet::Input -> Physics -> Damage -> Modifiers -> Broadcast`.
- `SimState` broadcasts at the simulation cadence, and `Welcome` for late join/reconnect includes live `WorldData`.
- World, region, asteroid, NPC AI, objective, comms, repair, and power systems run server-side.

## Disconnect And Reconnect

Disconnect no longer vacates a per-player player console vector because players do not own one. Instead:

1. `process_disconnect_with_stations` resolves the player's `StationId`.
2. It records the station's current rating in `Player.last_rating`.
3. It applies the station's `Backfill` rating through `rating::apply_rating`, switching station-owned systems to AI control.
4. It broadcasts `PlayerLeft` and `RatingChanged { rating_name: "Backfill" }`.
5. Auto-start is re-evaluated (see Lobby above).

On reconnect, `Identify` restores the previous station only if no other connected player has claimed it. If restore succeeds, the server broadcasts `StationAssigned` and `RatingChanged` with the saved `last_rating` (or the fallback used by the handler). If the station was claimed, the reconnecting player remains without a station and can select another available one.

**Spectator queue removed (#605):** The write-only `spectator_queue` field on `SessionManager` was deleted (all 5 API methods, push/remove call sites, and 6 unit tests). At-capacity join behaviour is unchanged — players still receive `StationAssigned { station: None }` when all slots are taken, but there is no FIFO queue backing the empty-station assignment: a cap-reached player simply has no station and must wait for a slot to open via another player's departure.

## GameOver

`GameOver` exists in the wire/state enum, but the current gameplay loop still treats end-of-scenario ceremony as future polish rather than a full round-management system.

## Related

- [Player](../entities/player.md)
- [Session](../entities/session.md)
- [Game Loop](./game-loop.md)
