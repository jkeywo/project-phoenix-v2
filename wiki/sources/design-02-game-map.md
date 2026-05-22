---
title: Draft 2 — Game Map
type: source
tags: [draft, design, map, solar-system, asteroid-field, deterministic]
source_path: docs/2. Draft Design - Game Map.md
status: draft
updated: 2026-05-22
---

# Draft 2 — Game Map

> **Status (2026-05-22) — as shipped:** Worlds are now defined as a single TOML (`assets/worlds/*.toml`) parsed by `parse_world` (`src/world/config.rs`) into a `WorldConfig` carrying anchors, `[[entity]]` instances, `[[trigger]]` blocks, and `[[comms]]` templates. Per-instance placement uses a single `transform = { ... }` table (`TransformConfig` at `src/world/config.rs:48`) with `position` / `anchor` / `relative_to` + `offset` / `rotation` (XYZ Euler radians) / `scale` (default `[1, 1, 1]`); resolution precedence is `relative_to+offset` > `anchor` > `position` > origin (`resolve_entity_position_with` at `src/world/config.rs:752`). An optional top-level `[ambient_light] { color, brightness }` block (`AmbientLightConfig` at `src/world/config.rs:115`) is applied by `spawn_world_ambient_light` (`src/server/renderer.rs:209`, `PostStartup`). Streaming asteroid lifecycle landed via PRD #191 (`src/asteroids/window.rs`). The original "damaged not tracked, destroyed tracked" rule was inverted in the shipped grid: destroyed asteroids respawn fresh when the player leaves and returns to a cell — see [Asteroid Field](../concepts/asteroid-field.md).

Each solar system defined by a file. Entities can be inline or reference an entity-type file ([Draft 1](./design-01-entity-config-files.md)).

## Proposed contents

**Star** — radius, colour.
**Planets** — radius, colour, orbital radius (circular orbits for now).
**Asteroid field** — list of asteroid types, list of cosmetic asteroid types, density, inner orbital radius, outer orbital radius.

## Spawning rules

- Asteroids only spawn when the ship is near them.
- Spawning is deterministic (same seed → same field).
- Damaged asteroids are not tracked; **destroyed** ones are.

## Implications & deltas from current shipped code

- Today's `WorldData` snapshot (everything sent up front in `WorldSetup`) becomes a streaming model — only nearby asteroids are live entities. The server still owns the seed so the layout is consistent.
- "Damaged not tracked" implies asteroid HP resets if the player flies away and back — design choice that reduces server state but means hit-and-run play doesn't accumulate.
- Adds **stars and planets** as world entities. Currently only ships and asteroids exist.
- Tracking destroyed asteroids needs a stable id — same UUID concept introduced by [PRD #66](./prd-066-weapons-and-engineering.md).

## Open questions

- "Inner / outer orbital radius" implies asteroid fields are *rings* around the star, not random clouds. Current spawner produces a square box around origin.
- How is "ship near" defined — fixed radius, view distance, something else?

## Cross-references

- Entity: [Asteroid](../entities/asteroid.md), [World Data](../entities/world-data.md)
- Concept: [Asteroid Field](../concepts/asteroid-field.md)
- Roadmap: [Data-Driven Content](../roadmap/data-driven-content.md)
