---
title: ShipPlugin
---

# ShipPlugin

Extracted from `simulation.rs` as part of the simulation split ([PRD #227](https://github.com/jkeywo/project-phoenix-v2/issues/227), issue [#239](https://github.com/jkeywo/project-phoenix-v2/issues/239)).

## Ownership

`ShipPlugin` owns the ship's motion and impulse drive, but not collision handling (which remains in `SimulationPlugin` because it depends on `RapierPhysicsPlugin`).

### Systems moved from `simulation.rs`

| System | Responsibility |
|---|---|
| `process_helm_inputs` | Reads `HelmInput` messages at 10 Hz, feeds into `compute_physics` |
| `sync_ship_position` | Syncs `ShipState` → Rapier `Transform` for the ship entity |
| `handle_impulse_messages` | Handles `StartImpulseCharge` / `CancelImpulse`, auto-cancels on hull damage |
| `process_coordination_lag` | Delivers channel-3 `CoordinationEnqueue` messages from each ship's `CoordinationQueue`; sets `PendingArcBearingRequest` for AI Helm on `ArcBearingRequest` delivery; emits popup for human Helm |
| `operate_helm_ai` | Applies NPC physics from AI intent; reads `PendingArcBearingRequest` and biases steering toward the requested bearing via `steer_toward` |

### Systems that stayed in `simulation.rs`

| System | Reason |
|---|---|
| `handle_collisions` | Requires `ReadRapierContext` — tightly coupled to Rapier, which `SimulationPlugin` owns |

### Resources

| Resource | Defined In | Purpose |
|---|---|---|
| `HelmInputTimer` | `ship_plugin.rs` | 10 Hz throttle for physics ticks |
| `LastHelmInput` (pub) | `ship_plugin.rs` | Holds last thrust/steering (read by `ConsoleAiPlugin`) |
| `CollisionCooldown` | `simulation.rs` | 1-second immunity after a collision hit |
| `PendingArcBearingRequest` | `ship_plugin.rs` | Set by `process_coordination_lag` when AI Helm consumes an `ArcBearingRequest`; biases steering via `steer_toward`; cleared when the target entity is visible or arrives in firing arc |

## Registration

```rust
.add_plugins(crate::ship_plugin::ShipPlugin)
```

Registered by `SimulationPlugin` in `simulation.rs` (after `CaptainPlugin`, before `AsteroidDestroyedVfx` message type).

## Future

Final home will be `src/ship/server.rs` once Deepening A ([#223](https://github.com/jkeywo/project-phoenix-v2/issues/223)) lands.

## Sources

- `src/ship_plugin.rs`
- `src/server_app.rs:handle_collisions`
- Issue [#239](https://github.com/jkeywo/project-phoenix-v2/issues/239)
