---
title: Draft 1 — Entity Config Files
type: source
tags: [draft, design, config, data-driven, entity, asteroid, ship]
source_path: docs/1. Draft Design - Entity config files.md
status: draft
updated: 2026-05-08
---

# Draft 1 — Entity Config Files

Proposes that entities be defined by data files rather than hard-coded constants.

## Proposed shapes

**Asteroid type**
- Size range — min to max radius
- Colour list — surface palette
- Hull integrity points

**Ship type**
- Size — radius and length of capsule
- Speed values — max forward, max reverse, acceleration, deceleration, turn speed
- Total hull integrity, repair rate, repair cooldown
- Phaser damage, phaser colour, phaser cooldown
- Helm radar range
- Weapons radar range

## Implications

- Multiple ships and multiple asteroid varieties become a content change, not a code change.
- The current `ShipPhysicsConfig` (in `ship_physics.rs`) is effectively the "Ship type" struct — most fields already exist.
- Asteroid radius is already in `AsteroidInfo`; colour and HP would be additions.
- Combines naturally with [Draft 2 — Game Map](./design-02-game-map.md) which references entity types by name.

## Open questions

- File format: TOML? JSON? Bevy `.scn` / asset format? Not specified.
- How are types looked up at runtime — string keys or typed handles?

## Cross-references

- Entity: [Asteroid](../entities/asteroid.md), [Ship](../entities/ship.md)
- Roadmap: [Data-Driven Content](../roadmap/data-driven-content.md)
