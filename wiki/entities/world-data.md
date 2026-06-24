---
title: World Data
type: entity
tags: [world, scenario, transform, ambient_light, snapshot]
sources: [src/world/config.rs, src/world/server.rs, src/server/renderer.rs, src/entities/config.rs, assets/worlds/default.toml]
updated: 2026-06-24
---

# World Data

The TOML-defined static layout of a single game session: anchors, entity instances, triggers, comms templates, and the global ambient light. Loaded once at startup, processed into ECS entities + runtime state.

## Source schema (`src/world/config.rs`)

```toml
seed = 42

[global]
seed = 42                                # optional global block; merged into WorldConfig

[ambient_light]                          # src/world/config.rs:115
color = [0.6, 0.55, 0.5]                 # sRGB; default Color::srgb(0.6, 0.55, 0.5)
brightness = 300.0                       # default 300.0

[[anchor]]
name = "starbase"
position = [0.0, 0.0, 0.0]               # normalised to [f32; 3]

[[entity]]                               # src/world/config.rs:142
template = "assets/entities/station_axiom.toml"
name = "axiom"                           # optional; overrides EntityConfig.name
transform = { anchor = "starbase", offset = [10.0, 0.0, 0.0] }

[[entity]]
template = "assets/entities/star_sun.toml"
transform = { position = [0.0, 50.0, 0.0], rotation = [0.0, 0.0, 0.0], scale = [1.0, 1.0, 1.0] }
```

`[[trigger]]` and `[[comms]]` blocks are documented under [World Plugin](../concepts/world-plugin.md) and [Comms Templates](../concepts/comms-templates.md).

## TransformConfig (`src/world/config.rs:48`)

Single struct replaces the old flat `position` / `anchor` / `relative_to` / `offset` fields on `WorldEntity`. Resolution precedence (see `TransformConfig::resolve` and `resolve_entity_position_with` at `src/world/config.rs:752`):

1. `relative_to = "<entity-name>" + offset` — resolved against a previously-spawned named entity
2. `anchor = "<anchor-name>" + offset` — resolved against a `[[anchor]]`
3. `position = [x, y, z]` — absolute
4. otherwise origin

Additional fields:
- `rotation: [f32; 3]` — XYZ Euler radians, applied via `Quat::from_euler(EulerRot::XYZ, x, y, z)`. Default `[0, 0, 0]`.
- `scale: [f32; 3]` — uniform-per-axis scale; default `[1, 1, 1]`. **Scale lives only on `TransformConfig`**; there is no `EntityConfig.scale` field.

## AmbientLightConfig (`src/world/config.rs:115`)

Optional top-level `[ambient_light]` block on the world TOML. Applied by the `spawn_world_ambient_light` system (`src/server/renderer.rs:209`, registered in `PostStartup` at `src/server/renderer.rs:91`) after `insert_world_config_resource` (`src/world/server.rs:152`) has placed the `WorldConfig` resource. If absent, the renderer falls back to `Color::srgb(0.6, 0.55, 0.5)` at brightness `300.0`.

## EntityConfig name + lights

- `EntityConfig.name: Option<String>` (`src/entities/config.rs:640`) is a template-level default. A `WorldEntity.name` override beats it. Both are stored as the `EntityName` component (`src/entities/spawner.rs:22`).
- `[[light]]` array-of-tables on `EntityConfig` (`src/entities/config.rs`) spawns Bevy lights as children of the entity. Each `LightConfig` has `kind = "point" | "directional"`, `colour: [f32; 3]`, `intensity: f32`, optional `range: f32`. Collected into the `Lights` component (`src/entities/spawner.rs:28`) and instantiated by `render_spawned_entities` (`src/server_app.rs:1147`).
- `[mesh].emissive: Option<f32>` on `EntityConfig` controls the StandardMaterial emissive multiplier (renderer default `0.4`; star templates use `2.0`).

## Lifecycle

1. **Startup:** Trunk fires `wasm_load_world`; `parse_world` populates the `WORLD_CONFIG` thread-local.
2. **`WorldPlugin` startup chain:** `insert_world_config_resource` → `spawn_world_entities` → `init_world_runtime`. Fallback `setup_fallback_world` runs only if no `WorldConfig` resource exists.
3. **Renderer backdrop:** `RendererPlugin` attaches the shared `assets/skybox/phoenix_space_cubemap.png` cubemap to `GameCamera`; it is independent of world TOML content.
4. **`spawn_world_ambient_light` (`PostStartup`)** reads `WorldConfig.ambient_light` and inserts the `AmbientLight` resource.
5. **`WorldSetup` broadcast** carries the per-instance entity snapshots to clients on `GameStart` and re-broadcasts via `Welcome` to late joiners.
6. **For the rest of the session:** anchors and ambient light are immutable. Entities can be destroyed (asteroids, hull-zero stations); triggers and objectives mutate via the wire.

## Migration notes (2026-05 entity-schema refactor)

- Old per-section blocks `[star]`, `[planet]`, `[station]`, `[science_console]` on entity TOMLs are **gone**. Stars are now plain entities with `[mesh] + [[light]]`; planets are `[mesh] + [collider]`; stations are `[mesh] + [hull]`, with hull damage tracked via `[hull].hull_integrity`.
- Old `WorldEntity` flat fields `position` / `anchor` / `relative_to` / `offset` have been folded into the single `transform = { ... }` table.
- See [refactor-2026-05-entity-schema](../sources/refactor-2026-05-entity-schema.md) for the full slice-by-slice account.

## Related

- [Asteroid](./asteroid.md) · [Asteroid Field](../concepts/asteroid-field.md)
- [World Plugin](../concepts/world-plugin.md)
- [Draft 1 — Entity Config Files](../sources/design-01-entity-config-files.md)
- [Draft 2 — Game Map](../sources/design-02-game-map.md)
- [Refactor 2026-05 — Entity Schema](../sources/refactor-2026-05-entity-schema.md)
