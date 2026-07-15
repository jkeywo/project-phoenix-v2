---
title: AI Ship Unification
type: concept
tags: [ai, npc, ship, ecs, components, per-kind-plugin, control-source, prd-520]
updated: 2026-07-03
---

# AI Ship Unification

Both the player ship and NPC ships are represented as ECS entities with the same `Ship` marker and the same four per-entity Components. AI control for each system kind (helm, tactical, etc.) is routed through a per-kind plugin (`operate_<kind>_ai`) gated by `ControlSourceResolver`. `src/ai/server.rs` writes intent only; the per-kind plugin applies physics.

## The unified Ship entity model

Every ship — player or NPC — carries these four Components:

| Component | Role |
|-----------|------|
| `ShipConfigComponent` | Parsed `ShipConfig`; defines stations, systems, and AI rules |
| `ShipSystemControlSources(ControlSourceResolver)` | Maps `SystemId → ControlSource` (Human or Ai) |
| `ActiveStationRatings` | Live complexity ratings from connected station holders |
| `CoordinationQueue` | Channel-3 lag queue for advisories |
| `PendingArcBearingRequest` | Set by `process_coordination_lag` when AI Helm consumes a weapons `ArcBearingRequest`; biases `operate_helm_ai` steering via `steer_toward` until the target is visible or enters firing arc |

Before PRD #520 these were singleton `Resource`s for the player ship only. After the unification, the player ship and NPC ships all carry them as Components on their ECS entity.

## Control flow

```
Lobby/Spawn
  └─ ShipSystemControlSources seeded
       ├─ Player ship: Human for all systems (toggled by rating changes)
       └─ NPC ship:    Ai for all systems (fixed at spawn)

Per-tick (SimSet::Physics)
  tick_ai_controllers (AiTickLabel)
    ├─ runs behaviour tree → outputs AiInput list
    ├─ writes last_helm_intent to AiControllerComponent (intent only)
    └─ dispatches SetTarget / FirePhaser as synthetic InboundMessages

  operate_helm_ai (after AiTickLabel)
    ├─ Player ship path (Without<AiControllerComponent>)
    │    policy.operate_ai? → write LastHelmInput { thrust: 0, steering: 0 }
    └─ NPC ship path (With<AiControllerComponent>)
         policy.operate_ai? → apply physics to Transform using last_helm_intent

  ai_target_selection (SimSet::Input)
    └─ every ship whose tactical surface is AI-operated: picks a target and
       writes both WeaponsBlackboard.locked_target (intent) and WeaponsTarget
       (truth). Firing is separate: ai_phaser_auto_fire / ai_torpedo_auto_fire
       decide, integrate_weapons_state applies.

  operate_shields_ai, operate_power_ai, operate_comms_ai, …  (stubs, #551)
    └─ one system per kind; gated on policy.operate_ai
```

## ControlSourceResolver

Lives in `src/ship/control_source.rs`. Maps a `SystemId` string to a `ControlSource` (Human or Ai). The derived `ControlTickPolicy` has three flags:

| Flag | Human | Ai |
|------|-------|----|
| `accept_human_input` | ✅ | ❌ |
| `operate_ai` | ❌ | ✅ |
| `coordinate` | ✅ | ✅ |

Every per-kind plugin and message handler checks `policy_for(system_id)` before acting.

## Per-kind AI plugins

Each system kind has (or will have) a dedicated Bevy system that runs after `AiTickLabel`:

| System | File | Status |
|--------|------|--------|
| `operate_helm_ai` | `src/ship_plugin.rs` | ✅ Full (applies NPC Transform physics; takes `FactionRegistryResource` for hostile detection) |
| `ai_target_selection` | `src/console/weapons/server.rs` | ✅ Full (all ships; replaced `operate_tactical_ai` in #700) |
| `operate_captain_ai` | `src/console/captain/server.rs` | ✅ |
| `operate_power_ai` | `src/ship/power.rs` | Stub |
| `operate_shields_ai` | `src/ship/shields.rs` | Stub |
| `operate_sensors_ai` | `src/ship/sensors.rs` | Stub (via `tick_sensors_frequency_hint`) |
| `operate_comms_ai` | `src/console/comms/server.rs` | Stub |
| `operate_repair_ai` | `src/console/repair/server.rs` | Stub |
| `operate_navigation_ai` | `src/console/navigation/mod.rs` | Stub |

## Objective-driven Backfill bridge

Until issue #581 moves blackboards to per-ship components, the player ship's Backfill AI reads the singleton viewscreen blackboard as a bridge. `publish_viewscreen_blackboard` scores active `ObjectiveManager` entries; `player_ship_helm_ai` consumes Patrol, Destroy, and Reach directives from that scored pool, building a `WorldView` that includes runtime scenario aliases from `WorldContentRuntime.name_to_uuid`. `ai_target_selection` uses the same pool to lock the top positive Destroy target before the phaser/torpedo automation runs.

This is used by `assets/worlds/combat_test.toml`: `obj-defend` patrols four anchors around Starbase Alpha, while each spawned `wave_N` gets a higher-scored Destroy objective that resolves through the runtime `wave_N -> uuid` mapping. Missing named targets are ignored rather than falling back to an arbitrary hostile.

## Captain red alert automation

`operate_captain_ai` in `src/console/captain/server.rs` controls the `red-alert` system, so it is gated by `ControlSourceResolver::policy_for(red_alert_system_id())`, not by the umbrella `captain` system. The AI reads recent combat from `RecentCombatActivity` in `src/ship/combat_activity.rs`; `publish_viewscreen_blackboard` mirrors those timestamps into the viewscreen aggregate for UI and cross-system visibility. `update_combat_activity` treats an uninitialized previous-hull value as the configured maximum hull, so a first observed damaged hull records combat instead of being missed.

## Helm destroy steering

`operate_helm` in `src/ai/core.rs` uses the active Destroy target both as the facing target and as the range anchor. When the ship is already inside maintain range, thrust is zero and steering should only face the target. The active Destroy target is therefore excluded from `avoidance_steering`; otherwise a close enemy with a large collider can be treated as an obstacle and make an AI-controlled, stationary ship yaw left/right around the same target.

Directive selection distinguishes unresolved directives from resolved idle commands. `operate_helm` tries lower-priority directives only when a directive returns `None` (for example, a Destroy target name that is not yet visible); `Some((0.0, 0.0))` means the directive resolved and intentionally wants the ship to hold station. This prevents a high-priority Destroy objective that has reached weapons range from falling through to a lower-priority Patrol objective and sharply steering away from the target.

`operate_helm_ai` caps its physics integration step to the same `1/30s` maximum used by the human helm timer. AI helm still runs every frame, but a long browser frame cannot be consumed as one oversized yaw step, so Backfill/NPC steering cannot visibly rotate faster than joystick-driven helm input.

### Arc-bearing steering bias

When a `PendingArcBearingRequest` is set (by `process_coordination_lag` in `src/ship_plugin.rs` consuming a `CoordinationPayload::ArcBearingRequest` from the coordination queue), `operate_helm_ai` reads the pending entity's position and biases steering toward it using `steer_toward(physics.yaw, [dx/dist, dz/dist], PATROL_DEADBAND_RAD, PATROL_FULL_STEER_RAD)` from `src/ai/mod.rs` (`PATROL_DEADBAND_RAD = 0.05`, `PATROL_FULL_STEER_RAD = π/4`). The pending request is cleared when the target enters the firing arc or is no longer visible.

## NPC ship spawn

When `spawn_entity` in `src/entities/spawner.rs` detects a TOML `[behaviour]` section, it inserts:

```rust
entity_commands.insert((
    Ship,
    ShipConfigComponent::default(),
    ShipSystemControlSources(resolver_seeded_all_ai),
    ActiveStationRatings::default(),
    CoordinationQueue::default(),
));
```

The `AiControllerComponent` is attached separately by `attach_controllers_on_spawn` in `src/ai/server.rs`.

## Multi-ship readiness

Queries use `With<Ship>` + `.iter()` (never `single()`). The lobby handlers use `iter_mut().next()` as "first ship found," which is safe for the current single-player-ship scenario and won't panic when multiple ships exist.

## Cross-references

- PRD #520 — AI Ship Unification
- PRD #142 — AI and Behaviour System
- [Coarse-system migration](./coarse-system-migration.md) — `ControlSourceResolver` context
- [Ship entity](../entities/ship.md)
