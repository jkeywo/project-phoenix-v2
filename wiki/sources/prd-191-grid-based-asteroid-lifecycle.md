---
title: PRD #191 — Grid-Based Asteroid Lifecycle with Deterministic Ring Buffer Window
type: source
tags: [prd, asteroid, lifecycle, streaming, deterministic, shipped]
source_url: https://github.com/jkeywo/project-phoenix-v2/issues/191
status: shipped
updated: 2026-05-13
---

# PRD #191 — Grid-Based Asteroid Lifecycle with Deterministic Ring Buffer Window

A player-centred ring-buffer grid streams asteroids in and out as the ship moves. Per-cell density is deterministic — destroyed asteroids respawn fresh when the player leaves the cell and returns.

## Status

Shipped (2026-05-12).

## Problem

The previous asteroid field generated a single fixed-size patch at world origin via `asteroid_spawner.rs`. Move far enough and you ran out of asteroids; come back and they were either still gone (destroyed) or popped instantly (no streaming). Memory grew unbounded if the field was sized to support long flights.

## Solution

- **Player-centred grid.** The world is divided into `resolution × resolution` cells. A `WindowedGrid` of size `(2 × despawn_cells + 1)²` sits centred on the player's current cell.
- **Per-cell deterministic seed.** Density at `(field_idx, gx, gz)` is derived from a fixed seed plus Perlin noise — same world cell always produces the same asteroid layout.
- **Ring-buffer window.** `update_asteroid_window` Bevy system tracks the player's current grid cell. When the player moves between cells, it computes which cells just entered the despawn ring (`None` them out, broadcast `EntityDespawned`) and which cells just entered the spawn ring (evaluate density, spawn an `EntitySnapshot`-broadcast asteroid if the check passes).
- **No persistent destroyed-asteroid set.** Destroyed asteroids are simply forgotten when their cell leaves the window. They respawn fresh when the player returns.
- **Donut bounds preserved.** The `asteroid_spawner.rs` density formula still excludes a tight ring around world origin (the donut hole) and an outer cutoff.
- **Large-jump fallback.** If the player teleports more than the window's radius in a single frame, the system flushes and rebuilds the entire window.

## Schema additions

- New pure module: `src/asteroid_window.rs` (`WindowedGrid`, `eval_on_player_move`).
- New Bevy systems in `src/asteroid_lifecycle.rs`.
- Reuses `EntitySnapshot` / `EntitySpawned` / `EntityDespawned` from PRD #153.
- `assets/maps/default.toml` asteroid-field section gains `resolution`, `spawn_cells`, `despawn_cells`.

## Out of scope

- Persistent destroyed-asteroid memory across cell visits (intentional — respawning is a feature).
- Non-asteroid streaming (stations, ships use different lifecycles).
- LOD / mesh swapping.

## Cross-references

- Builds on [PRD #153 — Region Entities + EntitySnapshot wire](./prd-153-region-entities.md)
- Replaces the single-shot field formerly built at startup by [PRD #22 — Helm and Game World](./prd-022-helm-and-game-world.md)
- [Asteroid](../entities/asteroid.md) · [Asteroid Field concept](../concepts/asteroid-field.md)
- [Roadmap Overview](../roadmap/overview.md)
