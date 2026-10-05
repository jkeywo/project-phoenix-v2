---
title: ShipPlugin
type: concept
tags: [ship, helm, physics, coordination, control-source, ai]
sources: [crates/phoenix-simulation/src/ship_plugin.rs, crates/phoenix-simulation/src/ship/components.rs, crates/phoenix-simulation/src/ship/helm_ai/, crates/phoenix-simulation/src/ship/helm_admission.rs, crates/phoenix-simulation/src/ship/physics_systems.rs, crates/phoenix-simulation/src/ship/coordination_systems.rs, crates/phoenix-simulation/src/ship/shields.rs, crates/phoenix-simulation/src/ship/rating_systems.rs, crates/phoenix-simulation/src/ship/damage_sync.rs, crates/phoenix-simulation/src/console/helm/server.rs, crates/phoenix-simulation/src/console/weapons/server.rs, crates/phoenix-simulation/src/console/repair/server.rs, crates/phoenix-model/src/messages.rs, crates/phoenix-simulation/src/server_app/collision.rs]
updated: 2026-08-27
---

# ShipPlugin

`ShipPlugin` is the Bevy adapter for ship motion, helm policy, station-control state, and inter-station coordination. Pure state and calculations remain under `crates/phoenix-simulation/src/ship/`; plugin registration and cross-module ordering live in `crates/phoenix-simulation/src/ship_plugin.rs`.

## Helm command path

Human and AI commands enter the same per-ship `AdmittedCommands` queue. The AI helm hosts build one frozen decision surface, emit ordinary `ControlSystem` payloads for the authored helm systems, and run before `process_helm_inputs`. That applier is the sole writer of helm intent components. `apply_helm_commands` handles impulse/boost transitions, then `integrate_ship_physics` consumes the complete intent set and writes `ShipPhysics`.

The current AI axes are thrust, steering, lateral thrust, vertical thrust, impulse, and boost. Each host is gated by the `ControlSourceResolver` policy for its own fine system and by the shared logical-tick AI cadence. See [AI Helm Decomposition](./ai-helm-decomposition.md).

## Coordination and station state

The plugin registers `CoordinationEnqueue`, owns the per-ship coordination queue
adapters, resolves human-seeking station hosts, and updates control sources when
station ratings or tenure change. `process_coordination_lag` delays explicitly
Station- or Ship-addressed messages, preserves each producer-owned presentation
envelope beside its typed payload, resolves live recipient policy, emits
an ordered popup candidate or `DeliveredCoordination` for a human- or
AI-operated Station respectively, and fans Ship delivery to eligible human
seats in authored Station order. Owning domain receivers consume the same typed fact
after Station delivery. Helm's receiver accepts only an AI delivery, rechecks
the authored Station and live `helm-steering` policy, and owns arc-bearing and
waypoint-clearance state. Tactical's frequency-hint receiver lives with Weapons;
Repair's receiver merges AI requests into its severity queue or applies its
human escalation latch before returning the already-projected popup to the
shared sequence-ordered outbox flush. Shields' receiver verifies its authored
shield-arc Station and live focus capability before latching a threat bearing.
The generic router no longer reads any of those domains' private state.

## Physics ownership

`integrate_ship_physics` is the normal helm-path writer. `sync_ship_position` projects authoritative `ShipPhysics` into the entity `Transform` before Rapier sync. The sanctioned out-of-band writers are documented on `ShipPhysics` in `crates/phoenix-simulation/src/ship/state.rs`.

Collision response is deliberately outside `ShipPlugin`: `handle_collisions` lives in `crates/phoenix-simulation/src/server_app/collision.rs`, where it can consume Rapier contacts and apply damage while remaining explicitly ordered between the physics and damage sets.

## Registration and tests

`crates/phoenix-simulation/src/server_app/registration.rs` installs `ShipPlugin` as one member of the fixed `SimSet` chain. Unit and schedule tests live beside their owning ship modules; the plugin's larger integration fixtures remain in `crates/phoenix-simulation/src/ship_plugin.rs` and the server-app integration suite.

## Related

- [Ship Physics](./ship-physics.md)
- [Helm Runtime](./helm-control-intent.md)
- [Modifier Coordination](./modifier-coordination.md)
- [Station](../entities/station.md)
