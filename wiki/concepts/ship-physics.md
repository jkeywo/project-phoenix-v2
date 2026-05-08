---
title: Ship Physics
type: concept
tags: [ship, physics, rapier, controller, pure-function]
sources: [src/server/ship_physics.rs, src/server/simulation.rs, PRD-022]
updated: 2026-05-08
---

# Ship Physics

The ship's motion model is a **pure Rust function** — no Bevy, no Rapier, no globals.

```rust
fn compute_physics(
    state: ShipPhysicsState,   // current speed, yaw
    input: HelmInputs,         // thrust 0..1, steering -1..1
    dt: f32,                   // frame delta seconds
    config: ShipPhysicsConfig, // tunables
) -> ShipPhysicsResult { ... }
```

`simulation.rs` calls this each tick, then applies the result to the ship's Rapier rigid body as a **direct velocity set** (not force application).

## The model

| Quantity | Value | Source |
|---|---|---|
| Max forward speed | 50 units/s | PRD #22 |
| Acceleration | 16.7 units/s² (~3 s to max) | PRD #22 |
| Deceleration on zero thrust | 50 units/s² (~1 s to stop) | PRD #22 |
| Max yaw rate at full steering | π/2 rad/s (90°/s) | PRD #22 |
| Movement plane | XZ, Y-up | PRD #22 |
| Forward direction at yaw=0 | −Z | PRD #22 |
| Collider | `Collider::capsule_y(half_height=3.0, radius=6.0)` | PRD #22 |
| Locked DOFs | Translate Y, rotate X/Z | PRD #22 |

## Behaviour rules

- **Arcade lerp.** Velocity lerps toward `thrust * max_speed`. Steering directly sets angular velocity around Y.
- **No reverse on zero thrust** — deceleration brings velocity to zero, not negative. Negative thrust (reverse) is supported via the model but the joystick only outputs 0..1 today.
- **Steering snaps to centre** on release (client-side).
- **Collision kills velocity.** Server collision event sets ship velocity to zero. Helm must re-apply thrust.

## Why a pure function

Three reasons:

1. **Tunability.** Tuning constants live on `ShipPhysicsConfig`. Pen-and-paper math + fast unit tests, no Bevy boot.
2. **Testability.** Inline `#[cfg(test)] mod tests` covers: zero input → zero velocity, thrust at 1.0 approaches max, decel from max reaches zero in ~1 s, steering at extremes hits max yaw rate, dt scales motion linearly, etc.
3. **Refactor seam.** If we ever swap Rapier or change Bevy, the physics model is untouched.

## Future: damage

PRD #66 adds `5 + (forward_speed / max_speed) * 10` clamped 5..15 collision damage to a separate Hull Integrity pool. The physics function itself is unchanged — damage is a *consumer* of `forward_speed`, not a parameter.

## Related

- [Helm Console](../entities/helm-console.md) · [Ship](../entities/ship.md)
- [PRD #22](../sources/prd-022-helm-and-game-world.md)
