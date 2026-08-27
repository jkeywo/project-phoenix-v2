---
title: ShipPlugin
type: concept
tags: [ship, helm, physics, coordination, control-source, ai]
sources: [src/ship_plugin.rs, src/ship/helm_ai/, src/ship/helm_admission.rs, src/ship/physics_systems.rs, src/ship/coordination_systems.rs, src/ship/rating_systems.rs, src/server_app/collision.rs]
updated: 2026-08-27
---

# ShipPlugin

`ShipPlugin` is the Bevy adapter for ship motion, helm policy, station-control state, and inter-station coordination. Pure state and calculations remain under `src/ship/`; plugin registration and cross-module ordering live in `src/ship_plugin.rs`.

## Helm command path

Human and AI commands enter the same per-ship `AdmittedCommands` queue. The AI helm hosts build one frozen decision surface, emit ordinary `ControlSystem` payloads for the authored helm systems, and run before `process_helm_inputs`. That applier is the sole writer of helm intent components. `apply_helm_commands` handles impulse/boost transitions, then `integrate_ship_physics` consumes the complete intent set and writes `ShipPhysics`.

The current AI axes are thrust, steering, lateral thrust, vertical thrust, impulse, and boost. Each host is gated by the `ControlSourceResolver` policy for its own fine system and by the shared logical-tick AI cadence. See [AI Helm Decomposition](./ai-helm-decomposition.md).

## Coordination and station state

The plugin registers `CoordinationEnqueue`, owns the per-ship coordination queue adapters, resolves human-seeking station hosts, and updates control sources when station ratings or tenure change. `process_coordination_lag` delivers the typed coordination payload after the hull's authored lag; recipients then consume the same delivered fact whether the station is human- or AI-operated.

## Physics ownership

`integrate_ship_physics` is the normal helm-path writer. `sync_ship_position` projects authoritative `ShipPhysics` into the entity `Transform` before Rapier sync. The sanctioned out-of-band writers are documented on `ShipPhysics` in `src/ship/state.rs`.

Collision response is deliberately outside `ShipPlugin`: `handle_collisions` lives in `src/server_app/collision.rs`, where it can consume Rapier contacts and apply damage while remaining explicitly ordered between the physics and damage sets.

## Registration and tests

`src/server_app/registration.rs` installs `ShipPlugin` as one member of the fixed `SimSet` chain. Unit and schedule tests live beside their owning ship modules; the plugin's larger integration fixtures remain in `src/ship_plugin.rs` and the server-app integration suite.

## Related

- [Ship Physics](./ship-physics.md)
- [Helm Runtime](./helm-control-intent.md)
- [Modifier Coordination](./modifier-coordination.md)
- [Station](../entities/station.md)
