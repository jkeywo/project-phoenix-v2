---
title: Server App Composition (server_app.rs)
type: concept
tags: [architecture, plugins, server, composition]
sources: [src/server_app.rs, src/server/bridge.rs]
updated: 2026-05-16
---

# Server App Composition

`src/server_app.rs` is the **composition root** for the server-side Bevy simulation. It replaces the former `simulation.rs` god-module (deleted in issue #261, the final slice of the simulation-split series #227).

## Entry point

```rust
pub fn add_simulation_plugins(app: &mut App)
```

Called from `wasm_init()` in `src/server/bridge.rs` instead of the old `.add_plugins(SimulationPlugin)`. All plugin registrations are centralised here.

## Plugin catalogue

| Plugin | File | Responsibility |
|---|---|---|
| `RapierPhysicsPlugin` | bevy_rapier3d | 3-D rigid-body physics |
| `RegionPlugin` | `src/regions/server.rs` | Region entry/exit, effect membership |
| `ConsoleAiPlugin` | `src/console_ai/server.rs` | Server-side AI for hidden consoles (Low complexity) |
| `AiPlugin` | `src/ai/server.rs` | NPC entity state machines (PRD #142) |
| `CaptainPlugin` | `src/captain_plugin.rs` | Red-alert toggle, view mode switching |
| `ShipPlugin` | `src/ship_plugin.rs` | Helm physics, impulse drive |
| `WeaponsPlugin` | `src/weapons_plugin.rs` | Phasers, torpedoes, target locking |
| `RepairPlugin` | `src/repair_plugin.rs` | Shape-matching repair, team dispatch, breakdown queue |
| `PowerPlugin` | `src/power_plugin.rs` | 6+2 power allocation, battery exhaustion |
| `SciencePlugin` | `src/science_plugin.rs` | Science target hand-off, CancelImpulse |

## Shared resources initialised here

Resources that do not belong to any single plugin live in `server_app.rs`:

| Resource | Type | Purpose |
|---|---|---|
| `ShipState` | `Resource` | Ship position, yaw, red-alert, view-mode |
| `ShipShields` | `Resource` | Four-quadrant shield model |
| `WorldResource` | `Resource` | `WorldData` snapshot for reconnect Welcome |
| `SimOutbox` | `Resource` | Per-frame outbound message buffer (drained by `sim_outbox_broadcaster`) |
| `TrackedEntities` | `Resource` | Entity reconciliation registry (EntitySpawned / EntityDespawned) |

`ShipHullIntegrity` was previously listed here as a `Resource`; it was deleted
in PRD #597 PR 10 (`EntitySystemHull` is the sole hull store — see
PRD #597). `ShipImpulse` was previously
listed here as a `Resource` too; issue #606 removed its `Resource` derive, so
it is now a per-entity `Component` only, populated on the ship entity at spawn
(`spawn_game_start_entities` for the player ship) rather than initialised as a
shared resource in this file.

## Systems in server_app.rs

Systems that are too cross-cutting to belong to a single plugin remain here:

- `handle_set_sensors_target` — Sensors→Tactical target suggestion routing
- `handle_set_shield_focus` — Shield-focus message handler
- `tick_shields` — Shield regen + offline timer
- `handle_collisions` — Rapier collision → hull damage (needs Rapier context + ShipState + shields + breakdowns)
- `broadcast_shield_status` — ShieldStatus at 10 Hz
- `broadcast_world_setup_on_start` — One-shot WorldSetup on first InProgress frame
- `reconcile_runtime_entities` — EntitySpawned / EntityDespawned delta tracking
- `setup_world` / `spawn_game_start_entities` / `render_spawned_entities` — World and entity lifecycle

## Broadcaster registrations

`add_simulation_plugins` also wires:

- `weapons_update_broadcaster()` — WeaponsUpdate at 10 Hz → Tactical holder
- `sim_state_broadcaster()` — SimState at 10 Hz → All
- `modifier_events_broadcaster()` — ModifierAdded / ModifierRemoved on every change → All
- `sim_outbox_broadcaster()` — drains `SimOutbox` every frame

See [Broadcaster Seam](./broadcaster-seam.md) for the full catalogue and API.

## Backward-compatible alias

`src/lib.rs` declares `pub use server_app as simulation;` so all existing `crate::simulation::*` import paths in other modules continue to compile without modification. The alias will be removed in a future cleanup pass once callers are updated to import from `crate::server_app` directly.

## Related

- [CaptainPlugin](./captain-plugin.md) — view mode + red alert
- [ShipPlugin](./ship-plugin.md) — helm + impulse
- [WeaponsPlugin](./weapons-plugin.md) — phasers + torpedoes
- [RepairPlugin](./repair-plugin.md) — breakdown repair loop
- [PowerPlugin](./power-plugin.md) — 6+2 power allocation
- [SciencePlugin](./science-plugin.md) — science target + impulse cancel
- [Broadcaster Seam](./broadcaster-seam.md) — SimBroadcaster API
- [WorldPlugin](./world-plugin.md) — scenario + world content (registered separately in bridge.rs)
