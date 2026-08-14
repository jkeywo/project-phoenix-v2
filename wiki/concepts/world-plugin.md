---
title: WorldPlugin
type: concept
tags: [world, plugin, server]
sources: [src/world/server.rs, src/world/dispatch.rs, src/world/config.rs, src/world/content.rs, src/world/layers.rs, src/world/scenario.rs, src/world/delayed.rs, src/entities/config_cache.rs, src/server/bridge.rs, src/server_app.rs, src/ai/server.rs, src/ai/faction.rs, assets/worlds/default.toml, assets/worlds/combat_test.toml, assets/factions/]
updated: 2026-07-16
---

# WorldPlugin

`WorldPlugin` is a Bevy plugin that owns world bootstrap and runtime content lifecycle for the simulation.

## Unified world (PRD #341 + PRD #342)

The merger of *map* and *scenario* into a single *world* concept is complete:

- **One asset directory:** `assets/worlds/` (each session loads exactly one TOML)
- **One WASM loader:** `wasm_load_world(path, toml_str)` in `src/server/bridge.rs`, which delegates to `entities/config_cache::wasm_load_world`
- **One JS fetch** in `server.html`
- **One parser:** `world::config::parse_world` → `WorldConfig` (anchors + `[[entity]]` instances), single-pass, populates a `WORLD_CONFIG` thread-local. Scenario logic is not in that struct: the `[script]` block is lifted and compiled separately (`src/world/script/`). The declarative `[[trigger]]` / `[[comms]]` arrays were the other front-end; issue #985 deleted them, and a world that still authors either is refused at load
- **One block type in TOML:** `[[entity]]`. The legacy `[[spawn]]` block was folded in (PRD #341)
- **One immediate-spawn pipeline:** `world::server::spawn_world_entities`, driven by `world::config::partition_immediate_entities` to route asteroid-field templates and other templates through the shared spawner
- **Layered runtime state:** a root world can load `extra_worlds` and a script handler can add or unload named child layers. Each layer carries its own flags and loader path; entities and ad-hoc spawns are tracked for unload cleanup. A layer contributes ENTITIES and nothing else — the declarative trigger merge was its only scenario-logic channel, and script-in-layers (#1045) is where that capability returns.
- **Layer effects:** `load_world` and `unload_world` are live additive layer-management effects. They are not scenario replacement; the root world remains active throughout the session.

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

Run-once startup systems in `WorldPlugin`, chained in order (see `src/world/server.rs:632` for `insert_world_config_resource`):

1. `insert_world_config_resource` (`src/world/server.rs:632`) — copies `WORLD_CONFIG` thread-local → `Res<WorldConfig>`
2. `spawn_world_entities` — spawns the `[[entity]]` instances the unified pipeline owns: asteroid fields and any entry carrying a `name` (`is_owned_by_unified_pipeline`). Not all of them — anonymous non-asteroid entries (stars, planets, and `id`-only entries like `default.toml`'s `nebula-1`) belong to `setup_world` in `src/server_app.rs`, a separate `Startup` system with no ordering relationship to this one. Both call `world_activation_blocked` first, so an invalid world spawns neither half. Per-instance placement is resolved by `resolve_entity_position_with` (`src/world/config.rs:1700`), which delegates to `TransformConfig::resolve` (`src/world/config.rs:80`) with precedence `relative_to+offset` > `anchor` > `position` > origin; `rotation` (XYZ Euler radians) and `scale` (default `[1, 1, 1]`) are applied from the same `transform = { ... }` table
3. `init_world_runtime` — initialises `WorldContentRuntime`, `ObjectiveManagerRes` from the loaded `WorldConfig`; `init_comms_runtime` (`src/comms/server.rs`, runs `.after` it) initialises `CommsRuntime` + `CommsInboxRes` (comms state split out in #816)
4. `load_extra_worlds` — loads any additional worlds declared in `WorldConfig.extra_worlds`

Production always loads a world TOML via the WASM bridge; when no `WorldConfig` is present (native unit tests only), the startup chain is a no-op and the app boots with an empty world.

The viewscreen space backdrop is no longer spawned by `WorldPlugin`: `RendererPlugin` attaches a Bevy `Skybox` to `GameCamera` and loads `assets/skybox/phoenix_space_cubemap.png` via `prepare_space_skybox_cubemap` (`src/server/renderer.rs:247`), replacing the old runtime star-sphere field.

A separate `PostStartup` system, `spawn_world_ambient_light` (`src/server/renderer.rs:296`, registered at `src/server/renderer.rs:94`), reads the optional `[ambient_light]` block (`AmbientLightConfig` at `src/world/config.rs:117`) and inserts the `AmbientLight` resource. If absent, the renderer falls back to `Color::srgb(0.6, 0.55, 0.5)` at brightness `300.0`. Running it in `PostStartup` guarantees `insert_world_config_resource` has already executed.

## Update systems

- `handle_hail` — Comms officer hails a contact; range-gates it, records it on `open_hails`, and emits `WorldEvent::Hailed` for a scripted `on_hailed` handler to answer (lives in `src/console/comms/server.rs` since #608; registered by `CommsWorldPlugin` since #816)
- `handle_respond_to_message` — player picks a response; runs its `on_pick` fn and injects the follow-up node the fn returned (also in `src/console/comms/server.rs`)
- `handle_clear_comms` — drops orphaned and read messages (also in `src/console/comms/server.rs`)
- `broadcast_comms_state` (`src/comms/server.rs`) / `broadcast_objective_summary` — push deltas on change
- `tick_infrastructure_condition` (`src/infrastructure/server.rs`, added by `InfrastructurePlugin`, `SimSet::Modifiers`, issue #1025) — advances the condition track on every entity that authored `[infrastructure]`, folding in authored decay, the hull it lost since the previous tick, and any condition adjustment a script queued this tick. Each threshold crossing writes the base-world `FlagStore` and queues a `FlagSet`/`FlagCleared` on `pending_world_events`, so an `on_flag_set` / `on_flag_cleared` handler fires one tick later — the same bridge `WaypointReached` rides, because `collect_world_events` has already run for the tick by the time this does. Capacities are mirrored onto plain counters a `when` predicate can read, and re-published whenever the structure did any work — since #1027 a `transfer` operation moves a capacity's level, and moves queue on `WorldContentRuntime::pending_capacity_adjustments` and drain here for the same one-write-site reason condition moves do. A capacity move fires no flag event: a published quantity is not an operational state.
- `tick_operations` (`src/operations/server.rs`, added by `OperationsPlugin`, `SimSet::Modifiers`, ordered `.before(tick_infrastructure_condition)`, issues #1026/#1027) — drains the operation starts a script queued this tick, then advances each ship's timed hold against this tick's real conditions. **Eligibility** first: proximity from `Transform`, the hull's authored `[operations]` capability, the power group's level plus the grid's exhaustion lock, free repair teams, and both ends' `[[infrastructure.capacity]]` levels for a `transfer`. Then the capability's authored **interrupt rules**, which read `RecentCombatActivity` (landed hits only — firing your own guns is not being attacked) and `RegionMembership`, and which `slow`, `pause` or `fail` the hold as the author said. Losing eligibility for something the crew can fix STALLS the hold — progress freezes rather than decaying — and losing it for something they cannot FAILS it; a `slow` keeps it running at a fraction of the rate and never spends the stall budget. Payoffs: `field_repair`'s per-tick condition slice (scaled by the rate) and `stabilise`'s lump on completion both push onto `pending_condition_adjustments`, and a completed `transfer` pushes both ends onto `pending_capacity_adjustments` — which is why the ordering matters: the payoff lands on the tick it was earned, and any crossing it causes is detected by the one system that owns flag edges. `move_towed_targets` (`SimSet::Modifiers`, `.after(tick_operations)`) holds a towed craft on the operator's authored rig and is a sanctioned `ShipPhysics` writer — see the writer-policy table on `ShipPhysics` in `src/ship/state.rs`. `handle_operation_commands` (`SimSet::Input`) and `publish_operations_blackboard` (`SimSet::Publish`) are the same plugin's other systems
- `tick_civilian_traffic` (`src/civilian/server.rs`, added by `CivilianPlugin`, `SimSet::Input`, issue #1028) — advances the compliance clock on every entity that authored `[civilian]`, takes the orders both surfaces queued for it, mirrors the entity's `PatrolCursor` index onto its state, and keeps ONE `civilian-route` doctrine objective on the hull pointed at whatever the craft is currently trying to do. It steers nothing: a route becomes an `AiDirective::Patrol` over the lane's anchors, a diverted anchor a `Reach`, a berth the `Dock` directive #1028 added, and a hold no helm-relevant directive at all — after which the ordinary NPC helm flies it. `SimSet::Input`, so a console order admitted this tick is acted on this tick; a scripted order is one tick later, because its applier runs in `Physics`.
- Trigger pipeline (issues #707–#719), chained in `SimSet::Physics`: `collect_world_events` (`src/world/server.rs`, drains queued `WorldEvent`s — attacked, destroyed, hull-threshold crossings, hailed, timer, region, flag — into the per-tick `WorldEventBuffer` resource) → `tick_trigger_pipeline` (`src/world/server.rs`, evaluates trigger conditions and runs each fired trigger's script handler) → `tick_script_callbacks` → `open_scripted_comms_threads` (`src/comms/scripted.rs`, materialises queued `open_comms` requests into live threads) → `tick_delayed_actions`, which fires queued delayed actions through the same dispatch table. Comms systems live in `CommsWorldPlugin` (`src/comms/server.rs`), added by `WorldPlugin` (#816)

## Trigger conditions

Triggers registered from a world's `[script]` block are matched against `WorldEvent`s by `evaluate_single_trigger` / `evaluate_triggers_with_flags` in `src/world/content.rs`. All conditions are single-shot (set `TriggerState.fired = true` once dispatched). The full list:

| `condition = ` | Required fields | Fires on |
|---|---|---|
| `on_destroyed` | `entity = "<name>"` | `WorldEvent::Destroyed { uuid }` whose `uuid` resolves to the named entity. |
| `on_all_destroyed` | `entities = ["<name>", ...]` | The tick the **last** named entity is destroyed. Stateful: tracks observed `Destroyed` events in `TriggerState.seen_destroyed`. Names that are never registered in `name_to_uuid` cause the trigger to never fire (#470). |
| `on_attacked` | `entity = "<name>"` | `WorldEvent::Attacked` for the named entity. |
| `on_hull_below` | `entity = "<name>"`, `threshold = <fraction>` | The named entity's aggregate hull crosses strictly from at/above the threshold to below it. |
| `on_timer` | `after_secs = <f32>` | `WorldEvent::TimerElapsed` once `elapsed_secs >= after_secs`. |
| `on_hailed` | `entity = "<name>"` | `WorldEvent::Hailed` for the named entity. |
| `on_flag_set` / `on_flag_cleared` | `name = "<flag>"` | False→true / true→false transitions of a world flag (with `parent:` walks for sub-world layers). |
| `on_world_loaded` | (none) | Once at world load (or sub-world load). |
| `on_entered_region` / `on_exited_region` | `entity = "<region>"` | Player ship enters / exits the named region. |

`OnAllDestroyed` is the only condition with non-trivial runtime state (`seen_destroyed: HashSet<String>` on `TriggerState`). `condition_matches` is stateless and read-only; the stateful `OnAllDestroyed` path is fast-pathed in `trigger_fires_for_events` before delegating. The mutation of `seen_destroyed` happens **before** the `when` predicate is evaluated, so a trigger with `on_all_destroyed` + `when = "flag(armed)"` will accumulate destruction events while the flag is unset and fire on the first tick where both conditions hold.

## Trigger actions

Trigger-fired actions are decided by a pure dispatch table in `src/world/dispatch.rs` (issues #710–#715): `dispatch_action` (`src/world/dispatch.rs:328`) covers every `TriggerAction` variant, routing grouped variants to five group functions (`dispatch_state_action`, `dispatch_entity_modifier_action`, `dispatch_world_flag_action`, `dispatch_destroy_entity`, `dispatch_spawn_entity` — spawn template loading goes through the injected `TemplateLoader` trait, `src/entities/loader.rs:70`). Each returns a `DispatchResult` that the applier `apply_dispatch_result` (`src/world/server.rs:1789`) turns into ECS mutations — the shared apply path for `tick_trigger_pipeline` (immediate actions) and `tick_delayed_actions` (delayed ones). Actions attached to a comms response still dispatch through a parallel inline `match` in `handle_respond_to_message` (`src/console/comms/server.rs:235`); the parity test `comms_response_dispatches_every_trigger_action_variant` (`src/console/comms/server.rs:1801`) guards against drift. Design rationale for the pipeline split lives in `pasm/spec/`.

Authoring shape per action variant:

| `type = ` | Required fields | Effect |
|---|---|---|
| `add_objective` / `complete_objective` / `fail_objective` | `id`, `text` (for add), `mandatory`, `targets` | Manage the objective list. |
| `set_ai_state` | `entity`, `state`, optional `target` | Legacy no-op since doctrine-based AI (#572); logs a warning. |
| `apply_modifier` / `remove_modifier` | `entity`, `tag`, `slot`, `bonus` | Float-valued ship stat modifier (`MaxSpeed`, `MaxYawRate`, `RadarRange`, `PhaserDamage`, `HullDamageTaken`, `RepairRate`). |
| `apply_flag` / `remove_flag` | `entity`, `tag`, `kind` | Boolean entity flag (`CommsJammed`, `SensorBlind`). |
| `apply_int_modifier` / `remove_int_modifier` | `entity`, `tag`, `slot`, `int_bonus` | Integer modifier (`RepairTeams`). |
| `game_over` | optional `message` | End the game with `GamePhase::GameOver`. |
| `load_world` / `unload_world` | `path` | Additive sub-world layer management (PRD #350). |
| `set_flag` / `clear_flag` / `increment_flag` / `set_flag_value` | `name`, plus `by` / `value` | World-flag mutators with `parent:` prefix walking for sub-world layers. |
| `spawn_entity` | `template_path`, `name`, one of `anchor` / `position`, optional `rotation` / `scale` | Ad-hoc spawn, registered in `name_to_uuid`; layer-tracked for `unload_world` cleanup. |
| `destroy_entity` | `entity` | Despawn by name; emits `AiEntityDestroyed` so chained `on_destroyed` triggers fire. |
| `repair_infrastructure` / `damage_infrastructure` | `entity`, `points` | Move the named structure's `[infrastructure]` condition (issue #1025). Whole points, or a `flt("…")` slice for the fractional per-tick step a timed operation applies. The applier resolves the name and QUEUES the delta on `WorldContentRuntime::pending_condition_adjustments` rather than writing the component, so every condition move goes through the one system that owns threshold edges. No delay-builder twin. |
| `stabilise` / `tow` / `escort` / `transfer` / `field_repair` | `ship`, `target` | Start an external operation: the named ship performs that verb on the named target (issues #1026, #1027). One host fn per VERB rather than one taking a verb string, so a misspelling is a load-time unknown-function; the five are registered from a loop because they differ only in the constant they push — everything that separates a tow from a field-repair is authored on the hull's `[[operations.capability]]` block. The applier resolves both names and QUEUES the start on `WorldContentRuntime::pending_operation_starts`; `tick_operations` — the only thing that can see the hull's capability table, its power grid and the target's position — decides whether a hold opens. The hold opens whatever the ship's position: range and power are re-tested every tick, so a start from out of position simply opens stalled. No delay-builder twin. |
| `order_hold` / `order_divert_route` / `order_divert_anchor` / `order_dock` | `entity`, plus a destination for the last three | Order the named civilian craft (issue #1028). The applier resolves the name and QUEUES the order on `WorldContentRuntime::pending_civilian_orders` rather than writing the component, so a scripted order goes through the same acknowledgement delay and the same authored disposition a crew's order does — a scenario cannot remote-control traffic a crew has to negotiate with. Four verbs rather than one taking a verb string, and `divert` split by destination, because a single verb could not tell a route id from an anchor name. No delay-builder twin. |
| `add_faction_enemy` / `remove_faction_enemy` | `faction`, `enemy` | Mutate the live `FactionRegistry` by faction `name` (resolved via `FactionRegistry::uuid_by_name`, `src/ai/faction.rs:68`). Idempotent. `is_enemy` is asymmetric — flipping a relationship in both directions requires two actions. `remove_faction_enemy` additionally re-validates every AI controller's remembered target (via `revalidate_ai_targets_after_faction_change`, `src/world/server.rs:2352`) so an in-progress engagement does not stick on a now-friendly target. |

The editor used to mirror this catalogue in `editor/action-schema.js`, which existed to drive its card-based trigger editor. Both went with the declarative front-end (issues #983, #985): scenario logic is authored in the editor's script panel now, and the catalogue has one home again.

### Objective directives

`add_objective` can carry AI-facing directive fields as well as the human-facing text: `directive_kind = "Patrol"` with `directive_anchors` / `directive_loop`, `directive_kind = "Destroy"` with `target`, or `directive_kind = "Reach"` with `directive_anchor`. `ObjectiveManager` scores active objectives into the viewscreen blackboard; player Backfill Helm and Tactical now read that shared pool as a bridge until the per-entity blackboard model from issue #581 lands.

`assets/worlds/combat_test.toml` uses this path: `obj-defend` is a low-priority Patrol loop around Starbase Alpha, and each spawned `wave_N` immediately adds a higher-priority Destroy objective targeting the runtime `name_to_uuid` entry for that wave. Matching `on_all_destroyed group = "wave_N"` triggers complete those objectives so the AI falls back to the starbase patrol. Since issue #960 there is no chain: every wave is released by its own `on_timer` at an authored offset, and all eight completion triggers are standalone `on_all_destroyed` triggers over their own group, so waves may be cleared out of order.

### Factions

Factions are loaded from `assets/factions/*.toml` (`FactionConfig` at `src/ai/faction.rs:17`) into a `FactionRegistry` (`src/ai/faction.rs:29`) exposed as `FactionRegistryResource` (`src/entities/config_cache.rs:478`). The asymmetric `is_enemy(a, b, registry)` predicate (`src/ai/faction.rs:116`) returns `true` only when `a`'s `enemies` list contains `b`; factionless entities are neutral to everyone. The AI's shared nearest-hostile scan (`find_nearest_hostile`, `src/ai/core.rs:691`) consults this predicate when picking a target.

**Defaults:** Federation is hostile to Pirate only. Harrow defaults to neutral so non-combat worlds (Starbase Alpha in `default.toml`, Before the Fire in `before_the_fire.toml`) can reuse the same Harrow ship templates as ambient patrols. Combat scenarios (`combat_test.toml`) flip the Federation↔Harrow relationship hostile on `on_world_loaded` via two `add_faction_enemy` actions before the first wave's hostile scan.


## Resources

`WorldContentRuntime`, `ObjectiveManagerRes`, `WorldConfig` (when loaded). Since #1025 `WorldContentRuntime` also carries `pending_condition_adjustments`, drained every tick by the infrastructure system and therefore empty at every tick boundary; #1026 added `pending_operation_starts` beside it, drained by `tick_operations` on the same terms, and #1027 added `pending_capacity_adjustments`, drained by the infrastructure system alongside the condition queue so a completed `transfer` re-publishes the counter a scenario predicate reads. Since #1028 it also carries `pending_civilian_orders`, drained the same way by `tick_civilian_traffic`. Since #1029 it also carries `commitments`, a `CommitmentLedger` sitting beside the `deadlines` table — and unlike the queues above it is **not** drained: it is a standing record of the promises the run has made, written only when a script call says so. Comms state lives in `CommsRuntime` + `CommsInboxRes` (`src/comms/server.rs`, split out in #816).

## Modules

| File | Contents |
|------|----------|
| `src/world/server.rs` | `WorldPlugin`, `insert_world_config_resource`, `spawn_world_entities`, `init_world_runtime`, `load_extra_worlds`, the trigger-pipeline systems (`collect_world_events`, `tick_trigger_pipeline`, `tick_delayed_actions`), the `apply_dispatch_result` applier, and world broadcast systems |
| `src/comms/server.rs` | `CommsWorldPlugin`, `CommsRuntime`, `CommsInboxRes`, `init_comms_runtime`, `update_comms_range_flags`, `broadcast_comms_state` (consolidated in #816) |
| `src/comms/scripted.rs` | `open_scripted_comms_threads` — the one path that opens a comms thread (#984) |
| `src/comms/content.rs` | Pure (Bevy-free) comms runtime types: `CommsDialogueNode`, `CommsResponse`, `ActiveDialogue`, `ScriptedDialogue`, `OpenCommsRequest`, `response_views` |
| `src/world/dispatch.rs` | Pure trigger-action decision layer: `dispatch_action` + five group functions returning `DispatchResult` for the applier |
| `src/world/delayed.rs` | Pure (Bevy-free) delayed-action scheduling: `DelayedAction`, `partition_delayed_actions` deciding ready vs. still-pending for the `tick_delayed_actions` applier (#821) |
| `src/world/layers.rs` | Pure (Bevy-free) world-layer decisions: `evaluate_layer_load` (dedup, parse handling, name→UUID assignment) for the `apply_world_layer_changes` applier (#821). The unload half computed a trigger-removal set until #985 left a layer with no triggers to remove |
| `src/world/scenario.rs` | Pure (Bevy-free) additive scenario-load decisions: `evaluate_scenario_load` (dedup / requeue / parse branches) for the `apply_pending_scenario_loads` applier (#821) |
| `src/world/config.rs` | Pure (Bevy-free): `WorldConfig`, `parse_world`, `entity_template_paths`, `partition_immediate_entities` |
| `src/world/content.rs` | Pure (Bevy-free) runtime types: `TriggerState`, `FiredTrigger`, `WorldEvent`, `evaluate_triggers`, `condition_matches`. Schema types re-exported from `world/config` |
| `src/entities/config_cache.rs` | WASM-side storage: `wasm_load_world` (the real loader), `WORLD_CONFIG` thread-local, `get_world_config` |
| `src/server/bridge.rs` | `#[wasm_bindgen]` exports including `wasm_load_world` (thin delegate to `config_cache::wasm_load_world`) |

## Shipped worlds

| Path | Contents |
|---|---|
| `assets/worlds/default.toml` | Starbase Alpha, asteroid field, initial pirate raider patrol, hailable starbase comms |
| `assets/worlds/patrol.toml` | Three-anchor patrol with a single raider and an on-destroyed objective |
| `assets/worlds/combat_test.toml` | Eight-wave Harrow defence scenario; starbase patrol objective plus per-wave Destroy objectives for Backfill Helm/Tactical |

See also: [World Data](../entities/world-data.md)
