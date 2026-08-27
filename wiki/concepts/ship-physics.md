---
title: Ship Physics
type: concept
tags: [ship, physics, rapier, controller, pure-function]
sources: [src/ship/physics.rs, src/ship/physics_systems.rs, src/ship/state.rs, src/entities/config.rs, src/entities/spawner.rs, src/server_app/collision.rs, src/server_app/registration.rs, assets/entities/alliance_destroyer.toml]
updated: 2026-08-27
---

# Ship Physics

The ship's motion kernel is a pure Rust function — no Bevy, Rapier, or globals.
It advances forward/reverse, lateral, and vertical speed plus X/Y/Z position and
yaw from one logical-tick input surface.

```rust
fn compute_physics(
    state: ShipPhysicsState,   // current speed, yaw
    input: ShipPhysicsInput,   // thrust/steering/lateral/vertical, each -1..1
    dt: f32,                   // fixed-step delta seconds
    config: &ShipPhysicsConfig // effective per-hull tunables
) -> ShipPhysicsResult { ... }
```

`integrate_ship_physics` calls it for every ship, stores the result in that
ship's `ShipPhysics` component, and `sync_ship_position` writes the authoritative
pose into `Transform` before Rapier sync.

## Authored model

Each hull's `[helm_console]` supplies forward/reverse caps, acceleration,
deceleration, yaw rate, low-speed turn boost, and lateral tuning. The spawner
copies those values into a per-entity physics-config component; only fixtures
or unauthored fallbacks use `ShipPhysicsConfig::new()`. Impulse, boost, power,
damage, and other modifiers produce an effective config before the pure kernel
runs. Collider shape and dimensions are likewise authored on the entity, not
fixed by this module.

## Behaviour rules

- Non-zero thrust approaches its signed authored cap at the authored
  acceleration; zero thrust decelerates toward zero. A cap reduced beneath the
  current speed bleeds the excess at the hull's deceleration rate instead of
  snapping in one tick.
- Steering uses the authored yaw rate, optionally boosted at low speed. Lateral
  and permitted vertical axes are independently rate-limited.
- Destroyed fine helm actuators contribute zero new intent, while damaged
  engines scale forward thrust. Power, impulse, boost, and modifier effects are
  folded before integration.
- `handle_collisions` sorts contacts deterministically, stops forward motion,
  separates the hull using authored collider geometry, and applies
  shield/hull damage from the captured impact speed.

## Why a pure function

Three reasons:

1. **Tunability.** Tuning constants live on `ShipPhysicsConfig`. Pen-and-paper math + fast unit tests, no Bevy boot.
2. **Testability.** Unit tests cover signed thrust, coasting, low-speed turning,
   over-cap bleed, all movement axes, and fixed-step scaling without booting a
   Bevy app.
3. **Refactor seam.** If we ever swap Rapier or change Bevy, the physics model is untouched.

## Collision damage

Collision damage is a consumer of `forward_speed`, not a parameter to the pure physics function. `handle_collisions` snapshots the impact speed before zeroing movement, scales `collision_damage(speed)` through `ModifierSlot::HullDamageTaken`, then routes it through shields and hull.

## Related

- [Helm Console](../entities/helm-console.md) · [Ship](../entities/ship.md)
