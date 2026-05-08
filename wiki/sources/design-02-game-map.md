---
title: Draft 2 — Game Map
type: source
tags: [draft, design, map, solar-system, asteroid-field, deterministic]
source_path: docs/2. Draft Design - Game Map.md
status: draft
updated: 2026-05-08
---

# Draft 2 — Game Map

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
