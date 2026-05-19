---
title: WorldPlugin
type: concept
tags: [world, scenario, map, plugin, server]
sources: [src/world/server.rs, src/world/config.rs, src/world/content.rs, src/entities/map_config.rs, src/entities/config_cache.rs, src/server/bridge.rs, src/server_app.rs, src/ai/server.rs, assets/worlds/default.toml, assets/worlds/patrol.toml]
updated: 2026-05-19
---

# WorldPlugin

`WorldPlugin` is a Bevy plugin that owns world bootstrap logic for the simulation.

## Map/Scenario merger — partial (PRD #337 open; PRD #338 slice 1 shipped)

The merger of *map* and *scenario* into a single *world* concept is **partial**. What has shipped:

- One asset directory: `assets/worlds/` (was: `assets/maps/` + `assets/scenarios/`)
- One WASM loader entry point: `wasm_load_world(path, toml_str)` in `src/server/bridge.rs`
- One JS fetch in `server.html` (was: map fetch → read `default_scenario` → scenario fetch)
- Removed `TriggerAction::LoadScenario` / `UnloadScenario` — scenario chaining is gone
- Renamed `ModifierSource::Scenario` → `ModifierSource::World`
- **PRD #338 slice 1:** `wasm_load_world` is no longer a shim. It now performs a single-pass parse via `world::config::parse_world` into a new unified `WorldConfig` (anchors + `[[entity]]` instances), and that `WorldConfig` is what `world::server::spawn_world_entities` and `ai::server::tick_ai_controllers` read. The legacy `MapConfig` and `ScenarioConfig` are still populated transitionally so untouched callers (asteroid_field spawner, scenario triggers, comms) keep working.

What has NOT yet been merged (tracked by PRD #337):

- `MapConfig` and `ScenarioConfig` remain as separate Rust types alongside the new `WorldConfig`. PRD #337 will collapse the three storages into one.
- TOML still has two block types: `[[entity]]` (map-side, immediate or game-start spawn) and `[[spawn]]` (scenario-side, named, trigger/comms-eligible). They should collapse into one `[[entity]]` block with an optional `name` field.
- Three spawn pipelines still exist, but they now coordinate via a pure partitioner (`world::config::partition_immediate_entities`):
  - `world::server::spawn_world_entities` (PRD #338 slice 1) — handles immediate-spawn `[[entity]]` instances whose resolved template carries an `[asteroid_field]` section. Reads `Res<WorldConfig>`.
  - `server_app::setup_world_from_config` — handles every other immediate-spawn `[[entity]]` instance. Carries a mirror skip guard so asteroid_field templates are not double-spawned.
  - `spawn_scenario_entities` — handles named `[[spawn]]` instances from the legacy scenario half.
  - `setup_world_hardcoded` — no-world fallback for native dev.
- `ScenarioManager`, `ScenarioOwner`, `ScenarioManagerRes`, and the `scenario_path` field threaded through runtime structs remain. With no scenario layering they should be deleted.
- Legacy `[[star]]`/`[[planet]]`/`[[asteroid_field]]` shorthand parsing remains in `MapConfig` even though no shipped world uses it.

## Current load path

```
JS (server.html)
  fetch('assets/worlds/default.toml')
    → wasm_load_world(path, toml_str)             ← single JS-visible call
        → world::config::parse_world(toml_str)    // NEW: single-pass parse → WorldConfig
            → stores WORLD_CONFIG thread-local
            → queues entity template paths into the preload pipeline (deduped)
        → wasm_load_map(toml_str)                 // transitional: stores MapConfig
        → wasm_load_world_content(path, toml_str) // transitional: stores ScenarioConfig
```

At `Startup`, `insert_world_config_resource` copies the `WORLD_CONFIG` thread-local into a Bevy `Resource` so `spawn_world_entities` and `ai::server::tick_ai_controllers` can read it via `Res<WorldConfig>`. The AI ticker falls back to `MapConfig::anchors` only when `WorldConfig` is not present (native tests).

## Current scope

`WorldPlugin` is the single owner of world content lifecycle: parse, spawn, triggers, comms, broadcast, and AI-event reaction.

**Startup systems (chained):**
- `insert_world_config_resource` — copies `WORLD_CONFIG` thread-local → `Res<WorldConfig>` (PRD #338 slice 1)
- `spawn_world_entities` — spawns asteroid-field `[[entity]]` instances (PRD #338 slice 1)
- `spawn_scenario_entities` — resolves scenario named-spawn positions and calls the entity-spawn pipeline
- `init_scenario_runtime` — initialises `WorldContentRuntime`, `CommsInboxRes`, `ObjectiveManagerRes` from loaded `ScenarioConfig`
- `setup_world_hardcoded` — starfield + player ship fallback when no `MapConfig` is present (dev/test)

**Resources:** `WorldContentRuntime`, `CommsInboxRes`, `ObjectiveManagerRes`, `ScenarioManagerRes`, `WorldConfig` (when loaded)

**Update systems:** `handle_hail`, `handle_respond_to_message`, `handle_clear_comms`, `broadcast_comms_state`, `broadcast_objective_summary`, `handle_ai_events`

## Modules

| File | Contents |
|------|----------|
| `src/world/server.rs` | `WorldPlugin`, `insert_world_config_resource`, `spawn_world_entities`, `setup_world_hardcoded`, `init_scenario_runtime`, comms/trigger/objective Bevy systems |
| `src/world/config.rs` | Pure (Bevy-free): `RawWorld`, `WorldConfig`, `parse_world`, `entity_template_paths`, `partition_immediate_entities` (PRD #338 slice 1) |
| `src/world/content.rs` | Pure (Bevy-free) scenario types: `ScenarioConfig`, triggers, comms templates, position resolution, `parse_scenario`, legacy `WorldConfig` wrapper, `ScenarioManager` |
| `src/entities/map_config.rs` | Pure `MapConfig` + `parse_and_validate_map_config` |
| `src/entities/config_cache.rs` | WASM-side storage: `wasm_load_map`, `wasm_load_world_content`, `wasm_load_world` (real loader), `WORLD_CONFIG` thread-local, `get_world_config` |
| `src/server/bridge.rs` | `#[wasm_bindgen]` exports including `wasm_load_world` (thin delegate to `config_cache::wasm_load_world`) |

## Triggers — removed actions

`TriggerAction::LoadScenario` and `TriggerAction::UnloadScenario` have been removed. Scenario chaining (one scenario loading another at runtime) is not supported — each session loads exactly one world TOML and runs it to completion. The remaining trigger actions cover objectives, AI state, modifiers, flags, and game-over transitions.

Internally, `ScenarioManager::load_scenario`/`unload_scenario` remain as plumbing for `CommsInbox::unload_scenario` and `ObjectiveManager::unload_scenario` cleanup paths, but no trigger action exposes them to TOML authors. PRD #337 will delete this plumbing alongside `ScenarioManager` itself.

## Bootstrap precedence

On startup, `WorldPlugin` selects the world bootstrap:

1. **World-driven** — if both a `MapConfig` and a `ScenarioConfig` were preloaded (via `wasm_load_world`), `spawn_scenario_entities` handles named `[[spawn]]` entities, `spawn_world_entities` handles asteroid-field `[[entity]]` instances, `setup_world_from_config` handles all other `[[entity]]` instances, and `init_scenario_runtime` seeds comms/trigger state.
2. **Map-only fallback** — if a `MapConfig` is present but no `ScenarioConfig`, the map's `[[entity]]` instances spawn but no named spawns, triggers, or comms templates run. (Mostly for native dev.)
3. **Hardcoded fallback** — if neither is present, `setup_world_hardcoded` spawns a procedural starfield and player ship.

PRD #337 will collapse these three paths into one.

## Shipped worlds

| Path | Contents |
|---|---|
| `assets/worlds/default.toml` | Starbase Alpha, asteroid field, initial pirate raider patrol, hailable starbase comms |
| `assets/worlds/patrol.toml` | Three-anchor patrol with a single raider and an on-destroyed objective |

See also: [World Data](../entities/world-data.md)
