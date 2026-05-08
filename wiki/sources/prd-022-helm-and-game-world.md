---
title: PRD #22 — Helm and Game World
type: source
tags: [prd, helm, ship, physics, asteroid, world, foundational]
source_url: https://github.com/jkeywo/project-phoenix-v2/issues/22
status: closed (2026-05-04)
updated: 2026-05-08
---

# PRD #22 — Helm and Game World

Turns the PoC into an actual game: adds a navigable world, a ship, and a second console.

## Problem

PRD #1 shipped one console (Captain) and a rotating cube. There was nothing to *do*. Crews need a navigable environment and a helm station so two players can co-pilot a ship through an asteroid field.

## Solution

- Add `Console::Helm` and `ClientMessage::HelmInput { thrust, steering }`.
- Replace the rotating cube with a Rapier-physics ship (capsule collider, XZ plane, Y-up, locked DOFs).
- Add a deterministic asteroid field generator.
- Mount the viewscreen camera to the ship's front (later refined by PRD #36).
- Red Alert becomes a border overlay on the viewscreen and consoles.

## Key decisions

- **bevy_rapier3d** for physics; locked DOFs constrain the ship to XZ + Y-yaw.
- **World scale:** 1 unit ≈ 1 m. Max speed 50 u/s. Spawn box ±150 u. ~20 u clear zone around origin.
- **Capsule collider:** `Collider::capsule_y(half_height=3.0, radius=6.0)`, aligned with yaw axis (bow strikes first).
- **Arcade controls:** thrust slider sets target speed. Velocity lerps at 16.7 u/s² (3 s to max). Decel 50 u/s² (1 s stop). Steering sets angular velocity directly.
- **Helm @ 10 Hz** for `HelmInput`, matching `SimState`.
- **Strict role separation:** Captain ≠ Helm. Ship doesn't move unless Helm is occupied.
- **Asteroid field fixed per session** — randomised once on `StartGame`. No streaming, no infinite scroll.
- **Collision:** ship-asteroid contact zeros ship velocity. No damage model in this PRD.

## New pure modules

- `asteroid_spawner` — `(seed, bounds, count, clear_zone) -> Vec<position>`.
- `ship_physics` — `compute_physics(state, input, dt, config) -> result`.

Both framework-free, tested in isolation. See [Ship Physics](../concepts/ship-physics.md), [Asteroid Field](../concepts/asteroid-field.md).

## Out of scope

Ship visual model (camera only); additional consoles; combat/sensors/power; infinite/procedural asteroids; asteroid movement; ship damage; sound; minimap (later added as Radar mode in PRD #36).

## Cross-references

- Entities: [Helm Console](../entities/helm-console.md), [Ship](../entities/ship.md), [Asteroid](../entities/asteroid.md), [World Data](../entities/world-data.md)
- Concepts: [Ship Physics](../concepts/ship-physics.md), [Asteroid Field](../concepts/asteroid-field.md), [Game Loop](../concepts/game-loop.md)
