---
title: PRD #153 — Region Entities, Component-Driven Spawning & Modifier Flags
type: source
tags: [prd, region, entity, modifier, flag, hull, shipped]
source_url: https://github.com/jkeywo/project-phoenix-v2/issues/153
status: shipped
updated: 2026-05-13
---

# PRD #153 — Region Entities, Component-Driven Spawning & Modifier Flags

Unified the entity pipeline so asteroids, ships, stations, and regions all flow through a single `[[entity]]` TOML loader and a single `EntitySnapshot` wire type. Promoted hull integrity to `f32` end-to-end and added typed boolean flags carried by `ShipModifiers`.

## Status

Shipped (2026-05-12, via slice issues #143–#152).

## Problem

Asteroids, the player ship, and (planned) stations had three different spawn paths, three different snapshot shapes, and no shared concept of "an entity in the world that the camera can see and the simulation can collide with." Region effects (damage zones, slow zones, jammers) had no representation at all. Hull was an integer that lost precision under fractional damage from per-tick region effects.

## Solution

- **Single `[[entity]]` TOML pipeline.** All entity configs go through `entity_config.rs`. Asteroids, the player ship, stations, and regions all parse the same way.
- **`EntitySnapshot` wire type.** Replaces `AsteroidSnapshot` and ad-hoc ship-state fields. Carries uuid, position, orientation, kind tag, and per-kind state.
- **Six region effects:** `damage_zone`, `slow_zone`, `comms_jammer`, `sensor_blind`, plus two more variants. Region containment runs each tick; entry/exit fires `RegionEntered` / `RegionExited` events that register or remove modifiers via `ShipModifiers` (PRD #117).
- **`f32` hull integrity.** `HullIntegrity` and the `hull_integrity` field on `SimSnapshot` are both `f32`. Shared `apply_hull_damage(world, delta)` helper called by both collisions and `damage_zone` regions.
- **`FlagKind` enum.** Typed boolean flags (`CommsJammed`, `SensorBlind`) live on `ShipModifiers` as a set keyed by `(source, FlagKind)`, OR-aggregated for read-out. Sources are `RegionEffect { uuid }`.
- **`EntitySpawned` / `EntityDespawned` deltas.** Replace the one-shot `WorldData` for everything except the initial snapshot.

## Schema additions

- `entity_config.rs` extended with region + station + ship variants.
- `region.rs` (pure containment + per-tick effect dispatch).
- `flag_kind.rs` (pure enum + serde).
- `modifiers.rs` extended with a flag set; `ModifierAdded` / `ModifierRemoved` carry an optional `FlagKind`.
- `messages.rs`: `EntitySnapshot`, `EntitySpawned`, `EntityDespawned`, `RegionEntered`, `RegionExited`; `SimSnapshot.hull_integrity` is now `f32`; `flags: Vec<FlagKind>` added.

## Out of scope

- Region visualisation on the viewscreen (no client-side rendering of region bounds).
- Per-region custom effect scripting.
- Region authoring tools.

## Cross-references

- Builds on [PRD #117 — Modifier System](./prd-117-modifier-system.md)
- [Draft 10 — Region Entities](./design-10-region-entities.md) (superseded by this PRD)
- Enabled [PRD #191 — Grid-Based Asteroid Lifecycle](./prd-191-grid-based-asteroid-lifecycle.md) by giving it a unified `EntitySnapshot` wire
- Required by [PRD #119 — Stations + Scenarios + Comms](./prd-119-stations-scenarios-comms.md) for station entities
- [Roadmap Overview](../roadmap/overview.md)
