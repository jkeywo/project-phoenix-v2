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
| `process_helm_inputs` | Turns admitted per-axis `ControlSystem` commands (`SetThrust` → `helm-thrust`, `SetSteering` → `helm-steering`, #801) into the shared helm intent components for every ship, human- and AI-admitted alike (authority is checked once at admission, #824) |
| `sync_ship_position` | Syncs `ShipState` → Rapier `Transform` for the ship entity |
| `handle_impulse_messages` | Handles `StartImpulseCharge` / `CancelImpulse`, auto-cancels on hull damage |
| `process_coordination_lag` | Delivers channel-3 `CoordinationEnqueue` messages from each ship's `CoordinationQueue`; sets `PendingArcBearingRequest` for AI Helm on `ArcBearingRequest` delivery; emits popup for human Helm |
| `ai_helm_thrust` / `ai_helm_steering` / `ai_helm_lateral_thrust` / `ai_helm_impulse` | Per-axis AI helm ([details](./ai-helm-decomposition.md)); `ai_helm_steering` reads `PendingArcBearingRequest` and biases steering toward the requested bearing via `steer_toward` |
| `integrate_ship_physics` | Sole helm-path writer of `ShipPhysics`; consumes the intent components for the player ship and every `AiHighFidelity` NPC |

### Systems that stayed in `simulation.rs`

| System | Reason |
|---|---|
| `handle_collisions` | Requires `ReadRapierContext` — tightly coupled to Rapier, which `SimulationPlugin` owns |

### Resources

| Resource | Defined In | Purpose |
|---|---|---|
| `AiTickTimer` / `AiTickReady` / `AiSnapshotReady` | `src/ai/cadence.rs` | The ONE shared AI decision cadence (issues #803, #889): a `run_if(ai_tick_ready)` gate on every AI policy host — the six per-axis helm systems plus shield focus, power allocation, torpedo load/auto-fire, frequency hint, phaser and blaster auto-fire, AI target selection — decoupling AI decision cadence from frame rate. Rate is TOML-authored via `[global] ai_tick_hz` (default 30 Hz, alias `ai_helm_tick_hz`); `AiSnapshotReady` is derived from it as a whole number of base ticks (`[global] ai_snapshot_hz`, default 10 Hz) and gates the `WorldSnapshot` rebuild, Captain and Sensors. Installed by `register_ai_cadence` from every plugin that registers a gated system |
| `LastHelmInput` (pub) | `src/ship/components.rs` | Holds last thrust/steering (read by `ConsoleAiPlugin`) |
| `CollisionCooldown` | `simulation.rs` | 1-second immunity after a collision hit |
| `PendingArcBearingRequest` | `src/ship/components.rs` | Set by `process_coordination_lag` when AI Helm consumes an `ArcBearingRequest`; `ai_helm_steering` biases steering via `steer_toward`; cleared when the target is no longer visible or a phaser arc already bears |

## Registration

```rust
.add_plugins(crate::ship_plugin::ShipPlugin)
```

Registered by `SimulationPlugin` in `simulation.rs` (after `CaptainPlugin`, before `AsteroidDestroyedVfx` message type).

## Future

Final home will be `src/ship/server.rs` once Deepening A ([#223](https://github.com/jkeywo/project-phoenix-v2/issues/223)) lands.

## Sources

- `src/ship_plugin.rs` (plugin registration + re-exports)
- `src/ship/components.rs`
- `src/ship/helm_ai.rs`
- `src/ship/helm_admission.rs`
- `src/ship/physics_systems.rs`
- `src/ship/impulse_boost_systems.rs`
- `src/ship/rating_systems.rs`
- `src/ship/coordination_systems.rs`
- `src/ship/damage_sync.rs`
- `src/server_app.rs:handle_collisions`
- Issue [#239](https://github.com/jkeywo/project-phoenix-v2/issues/239)
