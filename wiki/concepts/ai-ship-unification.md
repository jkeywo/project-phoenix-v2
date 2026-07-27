---
title: AI Ship Unification
type: concept
tags: [ai, npc, ship, ecs, components, per-kind-plugin, control-source, prd-520]
updated: 2026-07-16
---

# AI Ship Unification

Both the player ship and NPC ships are represented as ECS entities with the same `Ship` marker and the same per-entity Components. AI control for each system kind is routed through a per-kind AI system (helm: four per-axis systems; other consoles: `operate_<kind>_ai` / `ai_<kind>` systems) gated by `ControlSourceResolver`. AI systems write intent only; shared integrators apply the effects.

## The unified Ship entity model

Every ship — player or NPC — carries these four Components:

| Component | Role |
|-----------|------|
| `ShipConfigComponent` | Parsed `ShipConfig`; defines stations, systems, and AI rules |
| `ShipSystemControlSources(ControlSourceResolver)` | Maps `SystemId → ControlSource` (Human or Ai) |
| `ActiveStationRatings` | Live complexity ratings from connected station holders |
| `CoordinationQueue` | Channel-3 lag queue for advisories |
| `PendingArcBearingRequest` | Set by `process_coordination_lag` when AI Helm consumes a weapons `ArcBearingRequest`; `ai_helm_steering` biases steering toward it via `steer_toward` until the target is gone or a phaser arc bears |

Before PRD #520 these were singleton `Resource`s for the player ship only. After the unification, the player ship and NPC ships all carry them as Components on their ECS entity.

## Control flow

```
Lobby/Spawn
  └─ ShipSystemControlSources seeded
       ├─ Player ship: Human for all systems (toggled by rating changes)
       └─ NPC ship:    Ai for all systems (fixed at spawn)

Per-tick (SimSet::Physics, on the shared AI-helm sim tick — issue #803)
  ai_helm_thrust / ai_helm_steering / ai_helm_lateral_thrust / ai_helm_impulse
    ├─ each gated on its OWN axis's policy_for(...).operate_ai (never coarse)
    ├─ each calls the pure operate_helm / operate_lateral_thrust and keeps
    │  only its own axis, writing one intent component
    │  (ThrustInput / SteeringInput / LateralThrustInput / ImpulseCommand)
    └─ integrate_ship_physics consumes the intent components for the player
       ship and every AiHighFidelity NPC alike

  ai_target_selection (SimSet::Input)
    └─ every ship whose tactical surface is AI-operated: picks a target and
       writes both TacticalRadarBlackboard.selected_target (intent) and
       TacticalRadarSelection (truth). Firing is separate: ai_phaser_auto_fire / ai_torpedo_auto_fire
       decide, integrate_weapons_state applies.

  ai_shield_focus, ai_power_allocation, operate_comms_ai, …
    └─ one system per kind; gated on policy.operate_ai
```

See [AI Helm Decomposition](./ai-helm-decomposition.md) for the full per-axis helm architecture, the intent-component surface, and LOD.

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
| `ai_helm_thrust` / `ai_helm_steering` / `ai_helm_lateral_thrust` / `ai_helm_impulse` | `src/ship/helm_ai.rs` | ✅ Full — per-axis, replaced the `operate_helm_ai` monolith in #704 ([details](./ai-helm-decomposition.md)) |
| `ai_target_selection` | `src/console/weapons/mod.rs` | ✅ Full (all ships; replaced `operate_tactical_ai` in #700) |
| `operate_captain_ai` | `src/console/captain/server.rs` | ✅ |
| `ai_power_allocation` | `src/console_ai/server.rs` | ✅ (replaced the `operate_power_ai` stub) |
| `ai_shield_focus` | `src/console_ai/server.rs` | ✅ (replaced `operate_shields_ai`; high-LOD only) |
| `operate_sensors_ai` | `src/ship/sensors.rs` | ✅ |
| `operate_comms_ai` | `src/console/comms/server.rs` | Stub |
| `operate_repair_ai` | `src/console/repair/server.rs` | ✅ |
| `operate_navigation_ai` | `src/console/navigation/mod.rs` | ✅ (writes `NavigationWaypoint` + channel-3 `NavigateTo`) |

## Objective-driven Backfill bridge

The player ship's Backfill AI is the same code path as NPC AI: `publish_viewscreen_blackboard` scores active `ObjectiveManager` entries into the ship's viewscreen blackboard; the per-axis helm AI systems consume Patrol, Destroy, and Reach directives from that scored pool, building a `WorldView` that includes runtime scenario aliases from `WorldContentRuntime.name_to_uuid`. `ai_target_selection` uses the same pool to lock the top positive Destroy target before the phaser/torpedo automation runs.

This is used by `assets/worlds/combat_test.toml`: `obj-defend` patrols four anchors around Starbase Alpha, while each spawned `wave_N` gets a higher-scored Destroy objective that resolves through the runtime `wave_N -> uuid` mapping. Missing named targets are ignored rather than falling back to an arbitrary hostile.

## Captain red alert automation

`operate_captain_ai` in `src/console/captain/server.rs` controls the `red-alert` system, so it is gated by `ControlSourceResolver::policy_for(red_alert_system_id())`, not by the umbrella `captain` system. The AI reads recent combat from `RecentCombatActivity` in `src/ship/combat_activity.rs`; `publish_viewscreen_blackboard` mirrors those timestamps into the viewscreen aggregate for UI and cross-system visibility. `update_combat_activity` treats an uninitialized previous-hull value as the configured maximum hull, so a first observed damaged hull records combat instead of being missed.

## Helm destroy steering

`operate_helm` in `src/ai/core.rs` uses the active Destroy target both as the facing target and as the range anchor. When the ship is already inside maintain range, thrust is zero and steering should only face the target. The active Destroy target is therefore excluded from `avoidance_steering`; otherwise a close enemy with a large collider can be treated as an obstacle and make an AI-controlled, stationary ship yaw left/right around the same target.

Directive selection distinguishes unresolved directives from resolved idle commands. `operate_helm` tries lower-priority directives only when a directive returns `None` (for example, a Destroy target name that is not yet visible); `Some((0.0, 0.0))` means the directive resolved and intentionally wants the ship to hold station. This prevents a high-priority Destroy objective that has reached weapons range from falling through to a lower-priority Patrol objective and sharply steering away from the target.

Since issue #803 the four per-axis AI helm systems run on a shared fixed-rate sim tick (`[global] ai_tick_hz`, default 30 Hz) rather than every frame — and since issue #889 so does every other AI policy host, so a 144 Hz host makes the same decisions at the same cadence as a 60 Hz one. Physics integration (`integrate_ship_physics`) still runs every frame and caps its step at `1/30s`, so a long browser frame cannot be consumed as one oversized yaw step.

### Arc-bearing steering bias

When a `PendingArcBearingRequest` is set (by `process_coordination_lag` in `src/ship/coordination_systems.rs` consuming a `CoordinationPayload::ArcBearingRequest` from the coordination queue), `ai_helm_steering` (via `apply_arc_bearing_request`) reads the pending entity's position and biases steering toward it using `steer_toward(physics.yaw, [dx/dist, dz/dist], PATROL_DEADBAND_RAD, PATROL_FULL_STEER_RAD)` from `src/ai/mod.rs` (`PATROL_DEADBAND_RAD = 0.05`, `PATROL_FULL_STEER_RAD = π/4`). The pending request is cleared when some phaser bank's arc already bears on the target or the target is no longer visible.

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

`BehaviourSection` is the "this entity is AI-driven" predicate (issue #832 retired the separate `AiControllerComponent` marker). `register_ai_tokens_on_spawn` in `src/ai/server.rs` registers a synthetic `ai:<uuid>` token the tick each `BehaviourSection` first appears (`Added<BehaviourSection>`), and `unregister_on_despawn` releases it on `RemovedComponents<BehaviourSection>`.

## Multi-ship readiness

Queries use `With<Ship>` + `.iter()` (never `single()`). The lobby handlers use `iter_mut().next()` as "first ship found," which is safe for the current single-player-ship scenario and won't panic when multiple ships exist.

## Cross-references

- PRD #520 — AI Ship Unification
- PRD #142 — AI and Behaviour System
- [Coarse-system migration](./coarse-system-migration.md) — `ControlSourceResolver` context
- [Ship entity](../entities/ship.md)
