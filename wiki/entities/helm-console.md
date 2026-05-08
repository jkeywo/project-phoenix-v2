---
title: Helm Console
type: entity
tags: [console, helm, input, ship, physics, radar]
sources: [src/client/helm_plugin.rs, src/server/simulation.rs, src/server/ship_physics.rs, PRD-022]
updated: 2026-05-08
---

# Helm Console

The pilot's seat. The **only** console that can move the ship.

## Controls

- **Joystick:** up/down → thrust (0.0 to 1.0); left/right → steering (−1.0 to +1.0).
- **Steering snaps to centre on release** so the ship stops turning when the operator lets go.
- Sends `HelmInput { thrust, steering }` at **10 Hz** while controls are active.

## Server reception

Each simulation tick:
1. Look up `helm_token()` from `SessionManager`.
2. Drain `HelmInput` messages tagged with that token; keep the latest.
3. Pass `(state, input, dt, config)` into `compute_physics()` from `src/server/ship_physics.rs` — a pure Rust function, no Bevy.
4. Apply the resulting velocity directly to the [Ship](./ship.md)'s Rapier rigid body.

If no one is at Helm, no `HelmInput` is read and the ship coasts/decelerates.

## Helm radar

The helm console renders an overhead radar showing nearby asteroids. The projection is computed by `radar_dots()` in `src/shared/radar.rs` — the **same pure iterator** used by the server viewscreen Radar mode. See [Radar Projection](../concepts/radar-projection.md).

## Tuning constants

From PRD #22:

- Max forward speed: **50 units/s** (1 unit ≈ 1 m).
- Acceleration: **16.7 units/s²** (~3 s to max).
- Deceleration on zero thrust: **50 units/s²** (~1 s to stop).
- Max yaw rate at full steering: **π/2 rad/s** (90 °/s).
- Movement plane: XZ; Y-up. Forward = −Z when yaw = 0.

These live as constants in `compute_physics`'s `ShipPhysicsConfig` so they can be tuned without touching simulation/Bevy code.

## Related

- [Ship](./ship.md) · [Ship Physics](../concepts/ship-physics.md)
- [Radar Projection](../concepts/radar-projection.md)
- [PRD #22 — Helm and Game World](../sources/prd-022-helm-and-game-world.md)
