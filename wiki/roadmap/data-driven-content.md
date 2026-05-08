---
title: Data-Driven Content
type: roadmap
tags: [roadmap, data, config, scenario, world, asteroid, ship]
sources: [docs/1.md, docs/2.md, docs/6.md, docs/7.md]
updated: 2026-05-08
---

# Data-Driven Content

Move game content (entities, maps, scenarios) out of Rust source and into data files. Ship the engine, ship the content separately.

## Today

Everything is hardcoded in Rust:

- Asteroid count, spawn radius, clear zone, HP — constants in `src/server/asteroid_spawner.rs`.
- Ship physics tunables (max speed, accel, decel, yaw rate) — constants in `src/server/ship_physics.rs`.
- The world is a single asteroid field around origin. No planets, no stations, no destinations.
- "Game start" is "Captain presses Engage in lobby". No scenario, no objective.

## Drafts

Four drafts push toward data-driven content. They build on each other.

### Draft 1 — Entity Config Files

[Draft 1](../sources/design-01-entity-config-files.md) starts small: pull *Asteroid* and *Ship* out into config files with fields like model, scale, mass, HP. The spawner reads from a registry of entity types instead of constants.

Foundation for everything else. Without this, scenarios can't reference "spawn an Asteroid type 3" because there's no concept of a type.

### Draft 2 — Game Map

[Draft 2](../sources/design-02-game-map.md) extends the world from "one asteroid blob" to a navigable solar system:

- **Stars, planets, asteroid fields** as map features. Asteroid fields can be ring-shaped (around a planet) or volumetric.
- **Streaming.** Asteroids spawn only near the ship instead of all at game start. The radar projection iterator already does range filtering — same pattern, applied to spawn gating.
- **System map** drives Science Console's chart tab.

Big change to `WorldData`: today it's a one-shot snapshot at game start; Draft 2 makes it streaming.

### Draft 6 — Space Stations *(stub)*

[Draft 6](../sources/design-06-space-stations.md) — one-line TODO. Presumed to add docking, repair, refuel, mission origin points. Not designed yet.

### Draft 7 — Scenario File *(stub)*

[Draft 7](../sources/design-07-scenario-file.md) — one-line TODO. Presumed to define a session's objective, starting conditions, victory/failure states. The thing that turns "drive a ship around asteroids" into a *game*.

## Why this matters

- **The Bevy build is heavy.** Iterating on content via Rust recompiles is slow. Data files unblock fast iteration.
- **Tabletop campaigns.** A scenario file is what a Game Master ships to a crew. Without one, every session is identical.
- **Determinism.** Today's asteroid layout is seeded; same session = same field. A scenario file is the natural place for that seed plus everything else needed to reproduce a session.
- **AI playability.** A scenario file is also a contract a future AI Game Master could read and arbitrate.

## Risks

- **Schema drift.** Adding a new entity type means editing the file format and the Rust loader. Without TypeScript-style schema validation in Rust, this is a runtime-error footgun. Use `serde` `deny_unknown_fields` from the start.
- **Hot reload temptation.** Avoid until requested. The build is slow but predictable; hot reload introduces a class of bugs (stale resources, mid-tick mutation) that a tabletop session can't tolerate.
- **`serde_json` constraint.** [Codec Seam](../concepts/codec-seam.md) restricts `serde_json` to one module. Loading content files is a different concern (file format, not wire format) and may use any serde format — but pick *one* and put its loader in a single module too.

## Cross-references

- Entity: [Asteroid](../entities/asteroid.md), [Ship](../entities/ship.md), [World Data](../entities/world-data.md)
- Concept: [Asteroid Field](../concepts/asteroid-field.md), [Codec Seam](../concepts/codec-seam.md)
- Roadmap: [Open Architectural Questions](./open-architectural-questions.md)
