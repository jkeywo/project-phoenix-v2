---
title: WorldPlugin
type: concept
tags: [world, plugin, server]
sources: [src/world/server.rs, src/world/config.rs, src/world/content.rs, src/entities/config_cache.rs, src/server/bridge.rs, src/server_app.rs, src/ai/server.rs, src/ai/faction.rs, assets/worlds/default.toml, assets/worlds/combat_test.toml, assets/factions/]
updated: 2026-06-27
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
- **Layered runtime state:** a root world can load `extra_worlds` and triggers can add or unload named child layers. Each layer carries its own flags and loader path; entities and ad-hoc spawns are tracked for unload cleanup. Comms and objectives are not yet fully layer-owned, which is recorded as proposed PASM lifecycle work.
- **Layer trigger actions:** `load_world` and `unload_world` are live additive layer-management actions. They are not scenario replacement; the root world remains active throughout the session.

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

Run-once startup systems in `WorldPlugin`, chained in order (see `src/world/server.rs:152` for `insert_world_config_resource`):

1. `insert_world_config_resource` (`src/world/server.rs:152`) — copies `WORLD_CONFIG` thread-local → `Res<WorldConfig>`
2. `spawn_world_entities` — spawns all `[[entity]]` instances (asteroid-field and non-asteroid-field routed via `partition_immediate_entities`). Per-instance placement is resolved by `resolve_entity_position_with` (`src/world/config.rs:752`), which delegates to `TransformConfig::resolve` (`src/world/config.rs:48`) with precedence `relative_to+offset` > `anchor` > `position` > origin; `rotation` (XYZ Euler radians) and `scale` (default `[1, 1, 1]`) are applied from the same `transform = { ... }` table
3. `init_world_runtime` — initialises `WorldContentRuntime`, `CommsInboxRes`, `ObjectiveManagerRes` from the loaded `WorldConfig`
4. `load_extra_worlds` — loads any additional worlds declared in `WorldConfig.extra_worlds`

Production always loads a world TOML via the WASM bridge; when no `WorldConfig` is present (native unit tests only), the startup chain is a no-op and the app boots with an empty world.

The viewscreen space backdrop is no longer spawned by `WorldPlugin`: `RendererPlugin` attaches a Bevy `Skybox` to `GameCamera` and loads `assets/skybox/phoenix_space_cubemap.png` via `prepare_space_skybox_cubemap` (`src/server/renderer.rs:231`), replacing the old runtime star-sphere field.

A separate `PostStartup` system, `spawn_world_ambient_light` (`src/server/renderer.rs:209`, registered at `src/server/renderer.rs:91`), reads the optional `[ambient_light]` block (`AmbientLightConfig` at `src/world/config.rs:115`) and inserts the `AmbientLight` resource. If absent, the renderer falls back to `Color::srgb(0.6, 0.55, 0.5)` at brightness `300.0`. Running it in `PostStartup` guarantees `insert_world_config_resource` has already executed.

## Update systems

- `handle_hail` �?" Comms officer hails a contact; matching comms templates fire and inject messages
- `handle_respond_to_message` �?" player picks a response, may emit follow-up dialogue, runs response actions
- `handle_clear_comms` �?" drops orphaned and read messages
- `broadcast_comms_state` / `broadcast_objective_summary` �?" push deltas on change
- `tick_trigger_pipeline` �?" `WorldEvent` (attacked, destroyed, hailed, timer) drives trigger evaluation and the matching trigger actions

## Trigger conditions

Triggers in `[[trigger]]` blocks are matched against `WorldEvent`s by `evaluate_single_trigger` / `evaluate_triggers_with_flags` in `src/world/content.rs`. All conditions are single-shot (set `TriggerState.fired = true` once dispatched). The full list:

| `condition = ` | Required fields | Fires on |
|---|---|---|
| `on_destroyed` | `entity = "<name>"` | `WorldEvent::Destroyed { uuid }` whose `uuid` resolves to the named entity. |
| `on_all_destroyed` | `entities = ["<name>", ...]` | The tick the **last** named entity is destroyed. Stateful: tracks observed `Destroyed` events in `TriggerState.seen_destroyed`. Names that are never registered in `name_to_uuid` cause the trigger to never fire (#470). |
| `on_attacked` | `entity = "<name>"` | `WorldEvent::Attacked` for the named entity. |
| `on_timer` | `after_secs = <f32>` | `WorldEvent::TimerElapsed` once `elapsed_secs >= after_secs`. |
| `on_hailed` | `entity = "<name>"` | `WorldEvent::Hailed` for the named entity. |
| `on_flag_set` / `on_flag_cleared` | `name = "<flag>"` | False→true / true→false transitions of a world flag (with `parent:` walks for sub-world layers). |
| `on_world_loaded` | (none) | Once at world load (or sub-world load). |
| `on_entered_region` / `on_exited_region` | `entity = "<region>"` | Player ship enters / exits the named region. |

`OnAllDestroyed` is the only condition with non-trivial runtime state (`seen_destroyed: HashSet<String>` on `TriggerState`). `condition_matches` is stateless and read-only; the stateful `OnAllDestroyed` path is fast-pathed in `trigger_fires_for_events` before delegating. The mutation of `seen_destroyed` happens **before** the `when` predicate is evaluated, so a trigger with `on_all_destroyed` + `when = "flag(armed)"` will accumulate destruction events while the flag is unset and fire on the first tick where both conditions hold.

## Trigger actions

Action dispatch lives in two parallel `match` blocks: `tick_trigger_pipeline` (`src/world/server.rs:2001`) for actions fired by trigger conditions, and `handle_respond_to_message` (`src/world/server.rs:1073`) for actions attached to a comms response. The two sites must dispatch the **same** set of `TriggerAction` variants — a compile-time exhaustiveness test (`comms_response_dispatches_every_trigger_action_variant`, `src/world/server.rs:8721`) guards against drift.

Authoring shape per action variant:

| `type = ` | Required fields | Effect |
|---|---|---|
| `add_objective` / `complete_objective` / `fail_objective` | `id`, `text` (for add), `mandatory`, `targets` | Manage the objective list. |
| `set_ai_state` | `entity`, `state`, optional `target` | Switch the named NPC's AI controller to a named behaviour state. |
| `apply_modifier` / `remove_modifier` | `entity`, `tag`, `slot`, `bonus` | Float-valued ship stat modifier (`MaxSpeed`, `MaxYawRate`, `RadarRange`, `PhaserDamage`, `HullDamageTaken`, `RepairRate`). |
| `apply_flag` / `remove_flag` | `entity`, `tag`, `kind` | Boolean entity flag (`CommsJammed`, `SensorBlind`). |
| `apply_int_modifier` / `remove_int_modifier` | `entity`, `tag`, `slot`, `int_bonus` | Integer modifier (`RepairTeams`). |
| `game_over` | optional `message` | End the game with `GamePhase::GameOver`. |
| `load_world` / `unload_world` | `path` | Additive sub-world layer management (PRD #350). |
| `set_flag` / `clear_flag` / `increment_flag` / `set_flag_value` | `name`, plus `by` / `value` | World-flag mutators with `parent:` prefix walking for sub-world layers. |
| `spawn_entity` | `template_path`, `name`, one of `anchor` / `position`, optional `rotation` / `scale` | Ad-hoc spawn, registered in `name_to_uuid`; layer-tracked for `unload_world` cleanup. |
| `destroy_entity` | `entity` | Despawn by name; emits `AiEntityDestroyed` so chained `on_destroyed` triggers fire. |
| `add_faction_enemy` / `remove_faction_enemy` | `faction`, `enemy` | Mutate the live `FactionRegistry` by faction `name` (resolved via `FactionRegistry::uuid_by_name`, `src/ai/faction.rs:64`). Idempotent. `is_enemy` is asymmetric — flipping a relationship in both directions requires two actions. `remove_faction_enemy` additionally re-validates every `AiControllerComponent.blackboard.target` (via `revalidate_ai_targets_after_faction_change`, `src/world/server.rs:2812`) so an in-progress engagement does not stick on a now-friendly target. |

The editor mirrors this catalogue in `editor/action-schema.js`'s `ACTION_SCHEMA` map (plus a `covers every action type` regression test in `editor/tests/action-schema.test.js`).

### Objective directives

`add_objective` can carry AI-facing directive fields as well as the human-facing text: `directive_kind = "Patrol"` with `directive_anchors` / `directive_loop`, `directive_kind = "Destroy"` with `target`, or `directive_kind = "Reach"` with `directive_anchor`. `ObjectiveManager` scores active objectives into the viewscreen blackboard; player Backfill Helm and Tactical now read that shared pool as a bridge until the per-entity blackboard model from issue #581 lands.

`assets/worlds/combat_test.toml` uses this path: `obj-defend` is a low-priority Patrol loop around Starbase Alpha, and each spawned `wave_N` immediately adds a higher-priority Destroy objective targeting the runtime `name_to_uuid` entry for that wave. Matching `on_destroyed wave_N` triggers complete those objectives so the AI falls back to the starbase patrol.

### Factions

Factions are loaded from `assets/factions/*.toml` (`FactionConfig` at `src/ai/faction.rs:17`) into a `FactionRegistry` (`src/ai/faction.rs:29`) exposed as `FactionRegistryResource` (`src/entities/config_cache.rs:588`). The asymmetric `is_enemy(a, b, registry)` predicate (`src/ai/faction.rs:72`) returns `true` only when `a`'s `enemies` list contains `b`; factionless entities are neutral to everyone. The AI's `enemy_in_range` transition (`src/ai/core.rs:879`) consults this predicate every tick when picking a target.

**Defaults:** Federation is hostile to Pirate only. Harrow defaults to neutral so non-combat worlds (Starbase Alpha in `default.toml`, Before the Fire in `before_the_fire.toml`) can reuse the same Harrow ship templates as ambient patrols. Combat scenarios (`combat_test.toml`) flip the Federation↔Harrow relationship hostile on `on_world_loaded` via two `add_faction_enemy` actions before the first wave's `enemy_in_range` tick.


## Resources

`WorldContentRuntime`, `CommsInboxRes`, `ObjectiveManagerRes`, `WorldConfig` (when loaded).

## Modules

| File | Contents |
|------|----------|
| `src/world/server.rs` | `WorldPlugin`, `insert_world_config_resource`, `spawn_world_entities`, `init_world_runtime`, `load_extra_worlds`, comms/trigger/objective Bevy systems |
| `src/world/config.rs` | Pure (Bevy-free): `WorldConfig`, `parse_world`, `entity_template_paths`, `partition_immediate_entities` |
| `src/world/content.rs` | Pure (Bevy-free) runtime types: `TriggerState`, `CommsTemplateState`, `ActiveDialogue`, `FiredTrigger`, `FiredCommsTemplate`, `WorldEvent`, `evaluate_triggers`, `evaluate_comms_templates`, `process_response`, `trigger_states_from_world`, `comms_template_states_from_world`. Schema types re-exported from `world/config` |
| `src/entities/config_cache.rs` | WASM-side storage: `wasm_load_world` (the real loader), `WORLD_CONFIG` thread-local, `get_world_config` |
| `src/server/bridge.rs` | `#[wasm_bindgen]` exports including `wasm_load_world` (thin delegate to `config_cache::wasm_load_world`) |

## Shipped worlds

| Path | Contents |
|---|---|
| `assets/worlds/default.toml` | Starbase Alpha, asteroid field, initial pirate raider patrol, hailable starbase comms |
| `assets/worlds/patrol.toml` | Three-anchor patrol with a single raider and an on-destroyed objective |
| `assets/worlds/combat_test.toml` | Eight-wave Harrow defence scenario; starbase patrol objective plus per-wave Destroy objectives for Backfill Helm/Tactical |

See also: [World Data](../entities/world-data.md)
