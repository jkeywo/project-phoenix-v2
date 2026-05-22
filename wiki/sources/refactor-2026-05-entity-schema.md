---
title: Refactor 2026-05 — Entity Schema
type: source
tags: [refactor, entity, world, transform, ambient_light, schema]
source_path: (in-tree refactor, no external PRD)
status: shipped
updated: 2026-05-22
---

# Refactor 2026-05 — Entity Schema

A four-slice refactor of the entity + world TOML schemas that consolidated several ad-hoc per-section blocks into a uniform composition of `[mesh] + [collider] + [hull] + [[light]]`, and replaced the flat `WorldEntity` placement fields with a single `transform = { ... }` table. No new wire messages; no new gameplay. Pure schema + spawner cleanup.

## Slice 1 — Delete `ScienceConsoleConfig`

- **Removed:** `ScienceConsoleConfig` type and `EntityConfig.science_console` field.
- **Rationale:** Consoles are code-driven Bevy plugins (`src/console/science/`). The `[science_console]` block on entity TOMLs was never read by gameplay code.
- **Migration:** none — no shipped TOML carried the block.

## Slice 2 — Stations are plain entities

- **Removed:** `StationConfig`, `StationShape`, `EntityConfig.station`.
- **Added:** stations now compose `[mesh] + [hull] + [collider]` like any other entity. Station hull damage flows through the same `apply_hull_damage` path as the player ship via `[hull].hull_integrity` (`src/ship/damage.rs`).
- **Fix delivered:** the old `StationConfig` path silently bypassed `apply_hull_damage`, so station hull damage from collisions and `damage_zone` regions had no effect. Consolidation closed the latent bug.
- **Migration:** `assets/entities/station_*.toml` rewritten to use the unified blocks.

## Slice 3 — Stars and planets are plain entities; lights become an array

- **Removed:** `StarConfig`, `PlanetConfig`, `EntityConfig.star`, `EntityConfig.planet`.
- **Added:**
  - `EntityConfig.name: Option<String>` (`src/entities/config.rs:640`) — template-level default name. Overridable per-instance via `WorldEntity.name`.
  - `MeshConfig.emissive: Option<f32>` — StandardMaterial emissive multiplier. Renderer default `0.4`; star templates ship `2.0`.
  - `[[light]]` array-of-tables on `EntityConfig` — `LightConfig { kind: "point" | "directional", colour: [f32; 3], intensity: f32, range: Option<f32> }`. Replaces the old single-light-per-entity assumption.
  - `EntityName` component (`src/entities/spawner.rs:22`).
  - `Lights` component (`src/entities/spawner.rs:28`).
- **Spawner:** `render_spawned_entities` (`src/server_app.rs:1147`) rewritten to apply emissive from `[mesh].emissive` and instantiate each `[[light]]` as a child entity.
- **Migration:** `assets/entities/star_sun.toml` and `assets/entities/planet_earth.toml` recomposed; star uses `emissive = 2.0` plus a `[[light]] { kind = "point", colour = [...], intensity = ..., range = ... }`.

## Slice 4 — `transform = { ... }` and `[ambient_light]`

- **Removed:** flat `WorldEntity.position`, `.anchor`, `.relative_to`, `.offset` fields.
- **Added:**
  - `TransformConfig` (`src/world/config.rs:48`) with fields `position`, `anchor`, `relative_to`, `offset`, `rotation` (XYZ Euler radians via `Quat::from_euler(EulerRot::XYZ, x, y, z)`), `scale` (default `[1, 1, 1]`). Resolution precedence: `relative_to + offset` > `anchor` > `position` > origin.
  - `WorldEntity.transform: Option<TransformConfig>` (`src/world/config.rs:142`).
  - `resolve_entity_position_with` (`src/world/config.rs:752`) — thin wrapper over `TransformConfig::resolve`, used by both startup and `GameStart` spawn paths in `src/server_app.rs`.
  - `AmbientLightConfig { color, brightness }` (`src/world/config.rs:115`) on `WorldConfig` (`src/world/config.rs:243`, `:499`). Optional top-level `[ambient_light]` block on world TOMLs.
  - `spawn_world_ambient_light` system (`src/server/renderer.rs:209`), registered in `PostStartup` (`src/server/renderer.rs:91`) so it runs after `insert_world_config_resource` (`src/world/server.rs:152`). Fallback when absent: `Color::srgb(0.6, 0.55, 0.5)` at brightness `300.0`.
- **Scale precedence:** lives **only** on `TransformConfig.scale_vec()`. There is no `EntityConfig.scale` field — per-entity-template scaling is not a concern; per-instance scale is the only knob.
- **Migration:** `assets/worlds/{default,patrol}.toml` switched every `[[entity]]` to `transform = { ... }`; `assets/worlds/default.toml` got an `[ambient_light]` block.

## Worked TOML example

```toml
# assets/worlds/default.toml (excerpt)

[ambient_light]
color = [0.6, 0.55, 0.5]
brightness = 300.0

[[anchor]]
name = "starbase"
position = [0.0, 0.0, 0.0]

[[entity]]
template = "assets/entities/star_sun.toml"
transform = { position = [0.0, 200.0, -1000.0], scale = [50.0, 50.0, 50.0] }

[[entity]]
template = "assets/entities/station_axiom.toml"
name = "axiom"
transform = { anchor = "starbase", offset = [10.0, 0.0, 0.0], rotation = [0.0, 1.5708, 0.0] }
```

```toml
# assets/entities/star_sun.toml

name = "Sol"

[mesh]
kind = "sphere"
radius = 1.0
colour = [1.0, 0.9, 0.6]
emissive = 2.0

[[light]]
kind = "point"
colour = [1.0, 0.95, 0.8]
intensity = 1500000.0
range = 5000.0
```

## What did not change

- Wire types (`ClientMessage`, `ServerMessage`, `EntitySnapshot`) — unchanged.
- Asteroid lifecycle (PRD #191) — unchanged.
- Console plugins, AI, repair, power — unchanged.
- Editor scenario branch — still consumes the flat shape via the documented `sidebar.js` shim; out of scope for this refactor.

## Related

- [Draft 1 — Entity Config Files](./design-01-entity-config-files.md)
- [Draft 2 — Game Map](./design-02-game-map.md)
- [Draft 3 — Science Console](./design-03-science-console.md)
- [Draft 6 — Space Stations](./design-06-space-stations.md)
- [World Plugin](../concepts/world-plugin.md)
- [World Data](../entities/world-data.md)
