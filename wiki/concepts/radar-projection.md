---
title: Radar Projection
type: concept
tags: [radar, helm, viewscreen, pure-iterator, shared]
sources: [src/shared/radar.rs, CONTEXT.md]
updated: 2026-05-08
---

# Radar Projection

A single pure iterator that turns 3D asteroid positions into 2D radar dots, ship-relative.

## API

```rust
pub fn radar_dots(
    asteroids: &[AsteroidInfo],
    ship_x: f32,
    ship_z: f32,
    ship_yaw: f32,
) -> impl Iterator<Item = (f32, f32, f32)>  // (radar_x, radar_y, scaled_radius)
```

Lives in `src/shared/radar.rs` so both server and client can use it.

## Two consumers

1. **Helm console (client):** the helm UI renders an overhead mini-radar of nearby asteroids.
2. **Server viewscreen Radar mode:** when `ViewMode == Radar`, the viewscreen renders the same projection full-screen.

Same input → same output → same visual semantics. This was an explicit deepening (commit `f3ef92c`) — before that, server and helm had two separate projection implementations that drifted.

## Why an iterator

- Caller decides how to consume (filter by range, take top-N, collect).
- No allocation in the hot path.
- Easy to test: collect into a `Vec` and assert.

## Future filters

PRD #66 mentions extending `radar.rs` for **range-based** and **type-based** filtering — Weapons console wants 60-unit targeting range; Science (Draft 3) wants only stars/planets, no asteroids. The current iterator can be wrapped in standard `.filter()` calls; explicit filter combinators may follow.

## Related

- [Helm Console](../entities/helm-console.md) · [View Modes](./view-modes.md)
- [PRD #66](../sources/prd-066-weapons-and-engineering.md)
