---
title: PRD #581 — Unified Ship Entity Model
type: source
tags: [ai, ship, unification, npc, refactor]
source_url: https://github.com/jkeywo/project-phoenix-v2/issues/581
updated: 2026-07-01
---

## Status

**In progress** — core structural work done; some dual-write bridges remain.

## Problem

Player ship state lived in global singleton resources; NPC ships had equivalent state as per-entity components. Two parallel simulations, divergent helm AI, NPC SetTarget contamination bug.

## Solution

Unify player and NPC ships into one ECS entity model. `LocalShip` is the sole viewscreen selector. All simulation runs uniformly on any `Ship`-marked entity.

## Implementation State (as of 2026-07-01)

### Fully Done

- **`Ship` marker on all ships**: player ship has `Ship` + `LocalShip`; NPC ships have `Ship` only. `LocalShip` is the rendering/networking gate.
- **`ShipPhysics` component**: on all ship entities; position/motion fields removed from `ShipState`. `sync_ship_position` syncs all `ShipPhysics` to `Transform`.
- **`AdmittedCommands` per-entity**: inserted on all ships; `admit_system_commands` routes `ai:` tokens to owning entity via `AiTokenRegistry`.
- **`ShipSystemBlackboards` per-entity**: `publish_viewscreen_blackboard`, `publish_power_blackboard`, `publish_shields_blackboard`, `publish_sensors_blackboard`, `publish_navigation_blackboard`, and `publish_comms_blackboard` all write directly to the component; `broadcast_blackboard_updates` reads from it.
- **`tick_ai_controllers` deleted**: replaced by `register_npc_tokens_on_spawn` (token registration) and `process_attacker_this_tick` (attacker tracking).
- **`AiControllerComponent` empty marker**: all fields removed; kept as a query filter marker for NPC entities.
- **`attach_controllers_on_spawn` deleted**: replaced by `register_npc_tokens_on_spawn`.
- **`ShipAiMemory(pub AiMemory)` per-entity**: wraps `AiMemory`; inserted on all ships.
- **`operate_helm_ai` unified**: single per-entity loop covering player (Backfill) and NPC helm; reads `WorldSnapshot` for avoidance; `player_ship_helm_ai` (Reach-only stub) deleted.
- **`handle_fire_phaser_npc` deleted**: replaced by `handle_npc_beam_fire` (activation) + `tick_npc_beams` (damage), both using per-entity `ActiveBeam`/`PhaserCooldown`.
- **All console `operate_*_ai` loops**: cover all `ShipSystemControlSources` entities (no `With<Ship>` filter).
- **Console handlers use `With<LocalShip>`**: `handle_toggle_red_alert`, `handle_set_view`, `handle_dispatch_repair_team`, `process_helm_inputs`, `handle_impulse_messages`, `handle_boost_messages`, `handle_sensors_messages`, `handle_set_phaser_frequency`, `operate_tactical_ai`, and most weapon handlers.
- **`ShipRedAlert` + `ShipViewMode` per-entity**: inserted at spawn; `handle_toggle_red_alert` and `handle_set_view` dual-write to both global `ShipState` and per-entity component.
- **`EntityConsoleHull` on player ship**: player ship entity keeps `EntityConsoleHull` (not removed at spawn); hull dual-write bridge (`sync_player_hull_to_resource`, `sync_resource_hull_to_entity`) keeps it in sync with `ShipHullIntegrity` resource.
- **`LastHelmInput` derives `Component`**: inserted on player ship entity; still dual-derives `Resource`.
- **`NavigationWaypoint`, `SensorsTarget`, `ShipRepairTeams`, `ShipPowerSystem`, `WeaponsTarget`, `ActiveBeam`, `PhaserCooldown`**: all dual-derive `Component` + `Resource`; inserted on all ship entities at spawn.

### Dual-Write Bridges (Migration In Progress)

- `SystemBlackboards` global resource → `ShipSystemBlackboards` component: `dual_publish_blackboards` still copies global → component for the remaining publishers not yet migrated (only `publish_viewscreen_blackboard` was pre-migrated; power/shields/sensors/navigation/comms now write directly). Will be removed once all `publish_*_blackboard` functions write directly to per-entity.
- `ShipHullIntegrity` resource ↔ `EntityConsoleHull` component: two sync systems keep them in sync; `ShipHullIntegrity` still read by ~20 systems.
- `ShipState` (red_alert, view_mode, phaser_frequency): still a global Resource; per-entity `ShipRedAlert`/`ShipViewMode` are secondary writes.
- `LastHelmInput`: still primarily used as Resource; Component derive added but not yet used as per-entity in production.

### Remaining Work

- Delete `ShipState` resource (migrate remaining readers to per-entity `ShipRedAlert`, `ShipViewMode`, phaser_frequency component).
- Delete `ShipHullIntegrity` resource (migrate all readers to `EntityConsoleHull` on `LocalShip`).
- Delete `SystemBlackboards` / `FrozenBlackboards` / `LastBroadcastBlackboards` global resources (migrate all `publish_*_blackboard` functions to write directly to `ShipSystemBlackboards`).
- `LastHelmInput`: make per-entity only; remove Resource derive.
- Fix remaining `With<Ship>` in `world/server.rs` handlers (`handle_hail`, `handle_respond_to_message`, etc.) — these use `.single()` and should be `With<LocalShip>`.
- Remove `dual_publish_blackboards` once all publishers write directly to per-entity component.
- Remove `AiMemory` struct once `ShipAiMemory` fields have fully migrated to viewscreen blackboard.

## Key Decisions

- `AiTokenRegistry` retained as the `ai:<uuid>` → `Entity` lookup for the admission gate.
- `WorldSnapshot` resource built once per Physics tick; per-system `operate_*_ai` handlers read it.
- `aggregate_doctrine_blackboards` runs in `PublishAggregate` for all `BehaviourSection` entities.
- NPC weapon fire: `tick_phaser_auto_fire` handles auto-fire for unclaimed stations; `handle_npc_beam_fire` processes synthetic `FirePhaser` messages from `operate_*_ai`.

## Cross-references

- [AI Ship Unification concept](../concepts/ai-ship-unification.md)
- Issues #582 (admission gate fix), #583 (LocalShip marker), #584 (SystemBlackboards per-entity), #585 (WorldSnapshot), #586 (shields tracer bullet), #587–#596 (system-by-system migration)
