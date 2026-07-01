---
title: PRD #581 — Unified Ship Entity Model
type: source
tags: [ai, ship, unification, npc, refactor]
source_url: https://github.com/jkeywo/project-phoenix-v2/issues/581
updated: 2026-07-01
---

## Status

**Substantially complete** — all major resource-to-component migrations done. One dual-write bridge (`ShipHullIntegrity` ↔ `EntityConsoleHull`) remains.

## Problem

Player ship state lived in global singleton resources; NPC ships had equivalent state as per-entity components. Two parallel simulations, divergent helm AI, NPC SetTarget contamination bug.

## Solution

Unify player and NPC ships into one ECS entity model. `LocalShip` is the sole viewscreen selector. All simulation runs uniformly on any `Ship`-marked entity.

## Implementation State (as of 2026-07-01)

### Fully Done

- **`Ship` marker on all ships**: player ship has `Ship` + `LocalShip`; NPC ships have `Ship` only. `LocalShip` is the rendering/networking gate.
- **`ShipPhysics` component**: on all ship entities; position/motion fields removed from `ShipState`. `sync_ship_position` syncs all `ShipPhysics` to `Transform`.
- **`AdmittedCommands` per-entity**: inserted on all ships; `admit_system_commands` routes `ai:` tokens to owning entity via `AiTokenRegistry`.
- **`ShipSystemBlackboards` per-entity (W3 complete)**: all 10 `publish_*_blackboard` systems (helm, captain, repair, weapons, power, shields, sensors, navigation, comms, viewscreen) write directly to the component on the `LocalShip` entity. `SystemBlackboards`, `FrozenBlackboards` resources and `dual_publish_blackboards`, `snapshot_blackboards` systems deleted. `broadcast_blackboard_updates` reads from `ShipSystemBlackboards` component. `LastBroadcastBlackboards` kept as broadcaster change-cache.
- **`ShipState` resource deleted (W1 complete)**: all readers migrated to per-entity `ShipRedAlert`, `ShipViewMode`, `ShipPhaserFrequency` components.
- **`LastHelmInput` per-entity only (W4 complete)**: `Resource` derive removed; `process_helm_inputs`, `operate_helm_ai`, `tick_boost`, `operate_power_ai` all read/write via `Query<&LastHelmInput, With<LocalShip>>`.
- **`EntityConsoleHull` is primary hull store (W2 complete)**: all production systems read/write hull HP via `EntityConsoleHull` on `LocalShip`. `ShipHullIntegrity` resource and its bridge systems still exist but are no longer the production path.
- **`With<Ship>` → `With<LocalShip>` in world/server.rs (W5 complete)**: `handle_comms_channel2` fixed; all single-entity ship handlers now use `With<LocalShip>`.
- **`tick_ai_controllers` deleted**: replaced by `register_npc_tokens_on_spawn` and `process_attacker_this_tick`.
- **`AiControllerComponent` empty marker**: kept as query filter marker for NPC entities.
- **`attach_controllers_on_spawn` deleted**: replaced by `register_npc_tokens_on_spawn`.
- **`ShipAiMemory(pub AiMemory)` per-entity**: wraps `AiMemory`; inserted on all ships.
- **`operate_helm_ai` unified**: single per-entity loop covering player (Backfill) and NPC helm.
- **`handle_fire_phaser_npc` deleted**: replaced by `handle_npc_beam_fire` + `tick_npc_beams`.
- **All console `operate_*_ai` loops**: cover all `ShipSystemControlSources` entities.
- **Console handlers use `With<LocalShip>`**: all single-entity ship query handlers corrected.

### Dual-Write Bridges (Remaining)

- `ShipHullIntegrity` resource ↔ `EntityConsoleHull` component: `sync_player_hull_to_resource` / `sync_resource_hull_to_entity` still run. `ShipHullIntegrity` is kept because some smoke tests and the legacy bridge systems reference it. Can be deleted once verified smoke tests don't rely on the resource.

### Remaining Work

- Delete `ShipHullIntegrity` resource once smoke tests (Playwright) confirm they don't rely on it. The dual-write bridge and resource definition can then be removed.
- Remove `AiMemory` struct once `ShipAiMemory` fields have fully migrated to viewscreen blackboard (out of scope for PRD #581 core).

## Key Decisions

- `AiTokenRegistry` retained as the `ai:<uuid>` → `Entity` lookup for the admission gate.
- `WorldSnapshot` resource built once per Physics tick; per-system `operate_*_ai` handlers read it.
- `aggregate_doctrine_blackboards` runs in `PublishAggregate` for all `BehaviourSection` entities.
- NPC weapon fire: `tick_phaser_auto_fire` handles auto-fire for unclaimed stations; `handle_npc_beam_fire` processes synthetic `FirePhaser` messages from `operate_*_ai`.

## Cross-references

- [AI Ship Unification concept](../concepts/ai-ship-unification.md)
- Issues #582 (admission gate fix), #583 (LocalShip marker), #584 (SystemBlackboards per-entity), #585 (WorldSnapshot), #586 (shields tracer bullet), #587–#596 (system-by-system migration)
