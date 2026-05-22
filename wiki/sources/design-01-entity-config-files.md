---
title: Draft 1 — Entity Config Files
type: source
tags: [draft, design, config, data-driven, entity, asteroid, ship]
source_path: docs/1. Draft Design - Entity config files.md
status: draft
updated: 2026-05-22
---

# Draft 1 — Entity Config Files

> **Status (2026-05-22) — as shipped:** Data-driven entities landed via PRD #153 and were further consolidated by the 2026-05 entity-schema refactor. `EntityConfig` (`src/entities/config.rs:637`) now carries an optional top-level `name`, an optional `[mesh].emissive`, and an `[[light]]` array-of-tables. The per-section blocks `[star]`, `[planet]`, `[station]`, and `[science_console]` (and their backing `StarConfig` / `PlanetConfig` / `StationConfig` / `ScienceConsoleConfig` types) have been **deleted** — stars, planets, and stations are now ordinary entities composed from `[mesh]` + `[collider]` + `[hull]` + `[[light]]`. Station hull damage is tracked via `[hull].hull_integrity`. The original design intent below remains the source of record for the data-driven principle; the concrete schema lives in [refactor-2026-05-entity-schema](./refactor-2026-05-entity-schema.md).

Proposes that entities be defined by data files rather than hard-coded constants.

## Proposed shapes

**Asteroid type**
- Size range — min to max radius
- Colour list — surface palette
- Hull integrity points

**Ship type**
- Size — radius and length of capsule
- Speed values — max forward, max reverse, acceleration, deceleration, turn speed
- Total hull integrity, repair rate, repair cooldown
- Phaser damage, phaser colour, phaser cooldown
- Helm radar range
- Weapons radar range

## Implications

- Multiple ships and multiple asteroid varieties become a content change, not a code change.
- The current `ShipPhysicsConfig` (in `ship_physics.rs`) is effectively the "Ship type" struct — most fields already exist.
- Asteroid radius is already in `AsteroidInfo`; colour and HP would be additions.
- Combines naturally with [Draft 2 — Game Map](./design-02-game-map.md) which references entity types by name.

## Open questions

- File format: TOML? JSON? Bevy `.scn` / asset format? Not specified.
- How are types looked up at runtime — string keys or typed handles?

## Cross-references

- Entity: [Asteroid](../entities/asteroid.md), [Ship](../entities/ship.md)
- Roadmap: [Data-Driven Content](../roadmap/data-driven-content.md)
