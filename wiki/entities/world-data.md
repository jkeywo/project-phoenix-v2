---
title: World Data
type: entity
tags: [world, asteroid, deterministic, snapshot]
sources: [src/shared/messages.rs, src/server/asteroid_spawner.rs, src/server/simulation.rs]
updated: 2026-05-08
---

# World Data

The fixed, static layout of a single game session.

## Wire shape

`src/shared/messages.rs:99`:

```rust
pub struct WorldData {
    pub asteroids: Vec<AsteroidInfo>,
}
```

Carried by:

- `ServerMessage::WorldSetup { world }` — broadcast once when the captain hits Engage.
- `ServerMessage::Welcome { state }` — `state.world` is `Some(WorldData)` for clients that connect after `StartGame`, so late joiners see the same field.

## Lifecycle

1. **Lobby phase:** `GameState.world == None`.
2. **On `StartGame`:** `asteroid_spawner` generates positions deterministically; world stored in the game state; `WorldSetup` broadcast.
3. **For the rest of the session:** the world never changes. (Asteroid destruction, when PRD #66 lands, will require an additional event channel — `WorldData` is a snapshot, not a stream.)

## Why static

- Simpler clients: render once, no diffing.
- Simpler reconnect: `Welcome` carries everything.
- Lower bandwidth: no streaming of asteroid positions.

This is explicitly a PoC simplification. Draft 2 (Game Map) sketches a future where asteroids spawn near the ship as it moves, only destroyed asteroids being tracked.

## Related

- [Asteroid](./asteroid.md) · [Asteroid Field](../concepts/asteroid-field.md)
- [WorldPlugin](../concepts/world-plugin.md) — owner of map + scenario load via the unified `assets/worlds/*.toml`
- [Draft 2 — Game Map](../sources/design-02-game-map.md)
