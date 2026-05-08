---
title: Ship
type: entity
tags: [ship, physics, rapier, collision, viewscreen]
sources: [src/server/ship_state.rs, src/server/ship_physics.rs, src/server/simulation.rs, PRD-022]
updated: 2026-05-08
---

# Ship

The single player-controlled vessel in the world.

## Representation

- **Server:** a Bevy entity with a `RigidBody::Dynamic` Rapier capsule collider, DOFs locked to translation in XZ and rotation around Y. State held in `ShipState` resource (`src/server/ship_state.rs`):
  ```rust
  pub struct ShipState {
      pub red_alert: bool,
      pub view_mode: ViewMode,
      // position, yaw, speed are read from the Rapier transform
  }
  ```
- **Client:** never simulated locally. Clients receive `SimState { snapshot }` at 10 Hz and render from it.
- **Viewscreen:** the camera is parented to the ship and offset by **6.0 units** (the capsule radius) in the chosen [View Mode](../concepts/view-modes.md) direction.

## Capsule geometry

From PRD #22 / `simulation.rs`:

```rust
Collider::capsule_y(half_height = 3.0, radius = 6.0)
```

Aligned along the ship's yaw axis so the bow strikes asteroids before the stern.

## Movement model

See [Ship Physics](../concepts/ship-physics.md) — pure-Rust controller in `src/server/ship_physics.rs`.

## Collision

PRD #22 contract: **on ship-asteroid contact, ship velocity is zeroed.** Implemented via Rapier collision events in `simulation.rs`. There is no damage model in the shipped code — that arrives in [PRD #66](../sources/prd-066-weapons-and-engineering.md), which adds Hull Integrity (0–100, formula `5 + (forward_speed / max_speed) * 10` clamped 5–15 per collision).

## Snapshot fields

`SimSnapshot` (`src/shared/messages.rs:79`) is what every client sees:

```rust
pub struct SimSnapshot {
    pub red_alert: bool,
    pub view_mode: ViewMode,
    pub ship_x: f32,
    pub ship_z: f32,
    pub ship_yaw: f32,
}
```

Broadcast every 100 ms. PRD #66 plans to extend with `hull_integrity: i32` and `authorized_repair_console: Option<Console>`.

## Related

- [Helm Console](./helm-console.md) — what controls it
- [Asteroid](./asteroid.md) — what it crashes into
- [World Data](./world-data.md) — the asteroid layout
- [Ship Physics](../concepts/ship-physics.md)
