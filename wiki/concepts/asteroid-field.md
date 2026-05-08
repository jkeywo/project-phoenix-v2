---
title: Asteroid Field
type: concept
tags: [asteroid, world, deterministic, seed]
sources: [src/server/asteroid_spawner.rs, PRD-022]
updated: 2026-05-08
---

# Asteroid Field

The fixed-per-session backdrop the ship navigates through.

## Generator

`src/server/asteroid_spawner.rs` is a **pure Rust** module. Inputs:

- spawn radius (bounds)
- asteroid count
- clear-zone radius around origin
- seed

Output: a `Vec<AsteroidInfo>` with `(x, z, radius)` for each.

Properties guaranteed by the unit tests:

- Exact requested count.
- All positions within the spawn bounds.
- No asteroid centre inside the clear zone.
- No duplicate positions (for moderate counts).

## Determinism

Seeded — the same seed produces the same field. PRD #22 requires "the asteroid field layout consistent for the entire game session." The current implementation goes one step further and makes it consistent across runs with the same seed, useful for replays and debugging.

## Why fixed per session

PRD #22's PoC stance: simpler clients (render once), simpler reconnect (`Welcome` carries everything in `WorldData`), lower bandwidth (no streaming). Draft 2 (Game Map) sketches a future where asteroids spawn near the ship and only destroyed ones are tracked, but the current model is intentionally static.

## Cargo notes

`getrandom` needs the `wasm_js` feature on the WASM target (see `Cargo.toml`'s `[target.'cfg(target_arch = "wasm32")'.dependencies]` block in `AGENTS.md`).

## Related

- [Asteroid](../entities/asteroid.md) · [World Data](../entities/world-data.md)
- [Draft 2 — Game Map](../sources/design-02-game-map.md)
