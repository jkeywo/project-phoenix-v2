---
title: WorldPlugin
type: concept
tags: [world, plugin, server]
sources: [src/world/server.rs, src/world/config.rs, src/world/content.rs, src/entities/config_cache.rs, src/server/bridge.rs, src/server_app.rs, src/ai/server.rs, assets/worlds/default.toml, assets/worlds/patrol.toml]
updated: 2026-05-19
---

# WorldPlugin

`WorldPlugin` is a Bevy plugin that owns world bootstrap and runtime content lifecycle for the simulation.

## Unified world (PRD #341 + PRD #342)

The merger of *map* and *scenario* into a single *world* concept is complete:

- **One asset directory:** `assets/worlds/` (each session loads exactly one TOML)
- **One WASM loader:** `wasm_load_world(path, toml_str)` in `src/server/bridge.rs`, which delegates to `entities/config_cache::wasm_load_world`
- **One JS fetch** in `server.html`
- **One parser:** `world::config::parse_world` → `WorldConfig` (anchors + `[[entity]]` instances + `[[trigger]]` + `[[comms]]` templates), single-pass, populates a `WORLD_CONFIG` thread-local
- **One block type in TOML:** `[[entity]]`. The legacy `[[spawn]]` block was folded in (PRD #341)
- **One immediate-spawn pipeline:** `world::server::spawn_world_entities`, driven by `world::config::partition_immediate_entities` to route asteroid-field templates and other templates through the shared spawner
- **Flat runtime state (PRD #342):** there is no per-world ownership tracking. Triggers, comms templates, dialogues, inbox messages, and objectives all live for the duration of the session. The legacy multi-world layering machinery (per-element ownership tags, owner components, runtime layer maps, per-world unload paths) has been deleted
- **Removed trigger actions:** the old chaining actions (`load_world`-style) no longer exist; each session loads exactly one world TOML and runs it to completion

## Load path

```
JS (server.html)
  fetch('assets/worlds/default.toml')
    → wasm_load_world(path, toml_str)
        → world::config::parse_world(toml_str)
            → stores WORLD_CONFIG thread-local
            → queues entity template paths into the preload pipeline (deduped)
```

At `Startup`, `insert_world_config_resource` copies the `WORLD_CONFIG` thread-local into a Bevy `Resource` so downstream systems can read it via `Res<WorldConfig>`.

## Startup chain

Run-once startup systems in `WorldPlugin`, chained in order:

1. `insert_world_config_resource` — copies `WORLD_CONFIG` thread-local → `Res<WorldConfig>`
2. `spawn_world_entities` — spawns all `[[entity]]` instances (asteroid-field and non-asteroid-field routed via `partition_immediate_entities`)
3. `init_world_runtime` — initialises `WorldContentRuntime`, `CommsInboxRes`, `ObjectiveManagerRes` from the loaded `WorldConfig`
4. `setup_fallback_world` — gated by `run_if(not(resource_exists::<WorldConfig>))`; spawns a procedural starfield + player ship for native dev when no world TOML is loaded

## Update systems

- `handle_hail` — Comms officer hails a contact; matching comms templates fire and inject messages
- `handle_respond_to_message` — player picks a response, may emit follow-up dialogue, runs response actions
- `handle_clear_comms` — drops orphaned and read messages
- `broadcast_comms_state` / `broadcast_objective_summary` — push deltas on change
- `handle_ai_events` — `WorldEvent` (attacked, destroyed, hailed, timer) drives trigger evaluation and the matching trigger actions

## Resources

`WorldContentRuntime`, `CommsInboxRes`, `ObjectiveManagerRes`, `WorldConfig` (when loaded).

## Modules

| File | Contents |
|------|----------|
| `src/world/server.rs` | `WorldPlugin`, `insert_world_config_resource`, `spawn_world_entities`, `init_world_runtime`, `setup_fallback_world`, comms/trigger/objective Bevy systems |
| `src/world/config.rs` | Pure (Bevy-free): `WorldConfig`, `parse_world`, `entity_template_paths`, `partition_immediate_entities` |
| `src/world/content.rs` | Pure (Bevy-free) runtime types: `TriggerState`, `CommsTemplateState`, `ActiveDialogue`, `FiredTrigger`, `FiredCommsTemplate`, `WorldEvent`, `evaluate_triggers`, `evaluate_comms_templates`, `process_response`, `trigger_states_from_world`, `comms_template_states_from_world`. Schema types re-exported from `world/config` |
| `src/entities/config_cache.rs` | WASM-side storage: `wasm_load_world` (the real loader), `WORLD_CONFIG` thread-local, `get_world_config` |
| `src/server/bridge.rs` | `#[wasm_bindgen]` exports including `wasm_load_world` (thin delegate to `config_cache::wasm_load_world`) |

## Shipped worlds

| Path | Contents |
|---|---|
| `assets/worlds/default.toml` | Starbase Alpha, asteroid field, initial pirate raider patrol, hailable starbase comms |
| `assets/worlds/patrol.toml` | Three-anchor patrol with a single raider and an on-destroyed objective |

See also: [World Data](../entities/world-data.md)
