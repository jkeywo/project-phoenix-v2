---
title: Asteroid
type: entity
tags: [asteroid, world, obstacle, collision]
sources: [src/server/asteroid_spawner.rs, src/shared/messages.rs, PRD-022, PRD-066]
updated: 2026-05-08
---

# Asteroid

A static obstacle in the asteroid field.

## Wire shape

`src/shared/messages.rs:90`:

```rust
pub struct AsteroidInfo {
    pub x: f32,
    pub z: f32,
    pub radius: f32,
}
```

Sent inside [`WorldData`](./world-data.md) once on `WorldSetup` and replayed in `Welcome` to mid-game joiners.

## Server entity

A Bevy entity with a `RigidBody::Static` Rapier sphere collider. Spawned during `StartGame` setup from positions produced by the `asteroid_spawner` module.

## Behaviour today

- **Static.** No movement, tumbling, or asteroid-asteroid interaction.
- **Indestructible.** No HP model. A ship hitting one zeros its own velocity and bounces off; the asteroid is unchanged.

## Behaviour planned (PRD #66)

- Each asteroid gains a stable **UUID** (used for client-to-server target locks).
- Damage component with **30 HP**.
- Phaser fire deals **5 damage/sec** for **6 s** (30 total) per beam.
- Destruction plays a radial ripple effect on the viewscreen and despawns the entity.

## Spawning

See [Asteroid Field](../concepts/asteroid-field.md) for the deterministic seeded generator and clear-zone rules.

## Related

- [World Data](./world-data.md) · [Asteroid Field](../concepts/asteroid-field.md)
- [Draft 1 — Entity Config Files](../sources/design-01-entity-config-files.md) — proposed type-driven config
- [Draft 2 — Game Map](../sources/design-02-game-map.md) — ring fields, damaged-vs-destroyed tracking
