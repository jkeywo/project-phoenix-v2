---
title: Asteroid
type: entity
tags: [asteroid, world, obstacle, collision]
sources: [src/asteroids/spawner.rs, src/asteroids/mod.rs, src/entities/spawner.rs]
updated: 2026-07-14
---

# Asteroid

Asteroids are world obstacles produced from authored asteroid-field configuration. The deterministic spawner derives each cell's candidates from the field configuration and seed, allowing the same cell to be recreated when it leaves and re-enters the active window.

The server owns asteroid lifecycle, collision participation, and world publication. Clients render authoritative world state only. See [Asteroid Field](../concepts/asteroid-field.md) for the field lifecycle.
