---
title: WorldPlugin
type: concept
tags: [world, scenario, map, plugin, server]
sources: [src/world/server.rs, src/world/content.rs, src/entities/map_config.rs, src/entities/config_cache.rs, src/server/bridge.rs, assets/worlds/default.toml, assets/worlds/patrol.toml]
updated: 2026-05-19
---

# WorldPlugin

`WorldPlugin` is a Bevy plugin that owns world bootstrap logic for the simulation.

## Map/Scenario merger — partial (PRD #337 open)

The merger of *map* and *scenario* into a single *world* concept is **partial**. What has shipped:

- One asset directory: `assets/worlds/` (was: `assets/maps/` + `assets/scenarios/`)
- One WASM loader entry point: `wasm_load_world(path, toml_str)` in `src/server/bridge.rs`
- One JS fetch in `server.html` (was: map fetch → read `default_scenario` → scenario fetch)
- Removed `TriggerAction::LoadScenario` / `UnloadScenario` — scenario chaining is gone
- Renamed `ModifierSource::Scenario` → `ModifierSource::World`

What has NOT yet been merged (tracked by PRD #337):

- `MapConfig` and `ScenarioConfig` remain as separate Rust types. `WorldConfig` is currently `struct { map: MapConfig, scenario: ScenarioConfig }` and `parse_world` runs both parsers over the same TOML string (each silently ignores the sections the other owns).
- TOML still has two block types: `[[entity]]` (map-side, immediate or game-start spawn) and `[[spawn]]` (scenario-side, named, trigger/comms-eligible). They should collapse into one `[[entity]]` block with an optional `name` field.
- Three spawn pipelines remain: `setup_world_from_config` (map `[[entity]]`), `spawn_scenario_entities` (scenario `[[spawn]]`), `setup_world_hardcoded` (no-world fallback). They should collapse into one `spawn_world_entities` system.
- `ScenarioManager`, `ScenarioOwner`, `ScenarioManagerRes`, and the `scenario_path` field threaded through runtime structs remain. With no scenario layering they should be deleted.
- Legacy `[[star]]`/`[[planet]]`/`[[asteroid_field]]` shorthand parsing remains in `MapConfig` even though no shipped world uses it.
- Two WASM loaders still exist under the hood: `wasm_load_world` is a 3-line shim that calls `wasm_load_map` then `wasm_load_world_content`. The thread-locals `MAP_CONFIG` and `WORLD_CONTENT_CONFIG` are both still in use.

## Current load path

```
JS (server.html)
  fetch('assets/worlds/default.toml')
    → wasm_load_world(path, toml_str)        ← single JS-visible call
        → wasm_load_map(toml_str)             // internal: stores MapConfig
        → wasm_load_world_content(path, toml_str)  // internal: stores ScenarioConfig
            → queues entity template paths into the preload pipeline
```

## Current scope

`WorldPlugin` is the single owner of world content lifecycle: parse, spawn, triggers, comms, broadcast, and AI-event reaction.

**Startup systems:**
- `setup_world_hardcoded` — starfield + player ship fallback when no `MapConfig` is present (dev/test)
- `spawn_scenario_entities` — resolves scenario spawn positions and calls the entity-spawn pipeline
- `init_scenario_runtime` — initialises `ScenarioRuntime`, `CommsInboxRes`, `ObjectiveManagerRes` from loaded `ScenarioConfig`

**Resources:** `ScenarioRuntime`, `CommsInboxRes`, `ObjectiveManagerRes`, `ScenarioManagerRes`, `WorldContentRuntime`

**Update systems:** `handle_hail`, `handle_respond_to_message`, `handle_clear_comms`, `broadcast_comms_state`, `broadcast_objective_summary`, `handle_ai_events`, `evaluate_world_triggers`

## Modules

| File | Contents |
|------|----------|
| `src/world/server.rs` | `WorldPlugin`, `setup_world_hardcoded`, `init_scenario_runtime`, comms/trigger/objective Bevy systems |
| `src/world/content.rs` | Pure (Bevy-free) scenario types: `ScenarioConfig`, triggers, comms templates, position resolution, `parse_scenario`, `parse_world`, `WorldConfig`, `ScenarioManager` |
| `src/entities/map_config.rs` | Pure `MapConfig` + `parse_and_validate_map_config` |
| `src/entities/config_cache.rs` | WASM-side storage: `wasm_load_map`, `wasm_load_world_content`, `wasm_load_world` |
| `src/server/bridge.rs` | `#[wasm_bindgen]` exports including `wasm_load_world` |

## Triggers — removed actions

`TriggerAction::LoadScenario` and `TriggerAction::UnloadScenario` have been removed. Scenario chaining (one scenario loading another at runtime) is not supported — each session loads exactly one world TOML and runs it to completion. The remaining trigger actions cover objectives, AI state, modifiers, flags, and game-over transitions.

Internally, `ScenarioManager::load_scenario`/`unload_scenario` remain as plumbing for `CommsInbox::unload_scenario` and `ObjectiveManager::unload_scenario` cleanup paths, but no trigger action exposes them to TOML authors. PRD #337 will delete this plumbing alongside `ScenarioManager` itself.

## Bootstrap precedence

On startup, `WorldPlugin` selects the world bootstrap:

1. **World-driven** — if both a `MapConfig` and a `ScenarioConfig` were preloaded (via `wasm_load_world`), `spawn_scenario_entities` handles named `[[spawn]]` entities, the map-half handles `[[entity]]` instances, and `init_scenario_runtime` seeds comms/trigger state.
2. **Map-only fallback** — if a `MapConfig` is present but no `ScenarioConfig`, the map's `[[entity]]` instances spawn but no named spawns, triggers, or comms templates run. (Mostly for native dev.)
3. **Hardcoded fallback** — if neither is present, `setup_world_hardcoded` spawns a procedural starfield and player ship.

PRD #337 will collapse these three paths into one.

## Shipped worlds

| Path | Contents |
|---|---|
| `assets/worlds/default.toml` | Starbase Alpha, asteroid field, initial pirate raider patrol, hailable starbase comms |
| `assets/worlds/patrol.toml` | Three-anchor patrol with a single raider and an on-destroyed objective |

See also: [World Data](../entities/world-data.md)
