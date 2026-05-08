---
title: Combat & Damage
type: roadmap
tags: [roadmap, combat, weapons, engineering, hull, shields, torpedoes, repair]
sources: [PRD-066, docs/4.md, docs/5.md]
updated: 2026-05-08
---

# Combat & Damage

The first combat loop, hull integrity, and the crew-coordination repair mechanic. Spans one shipping PRD and two follow-on drafts.

## Phase 1 — PRD #66 (in flight)

[PRD #66 — Weapons & Engineering](../sources/prd-066-weapons-and-engineering.md) introduces:

- **Asteroids gain identity and HP.** UUID per asteroid, 30 HP, destructible.
- **Phaser beam.** 6 s burn, 5 dmg/s, 40 u range, 180° forward arc. 6 s cooldown after burn or sever.
- **Lock vs Fire are separate gates.** Lock: 360°/60 u. Fire: arc + 40 u. Sever on out-of-arc, out-of-range, or destruction.
- **Hull Integrity 0..100.** Single pool, no shields, no subsystems.
- **Collision damage.** `5 + (forward_speed / max_speed) * 10`, clamped 5..15.
- **Breakdown queue.** Every 10 HP lost queues one breakdown to a random console (never the same twice in a row). Engineering shows one active assignment.
- **Repair = crew coordination.** Every console has a Repair button; only the *correct* console for the active breakdown actually repairs. Wrong press = 30 s red-flash penalty.

Net experience: damage is felt, repair requires the crew to talk over voice.

Explicit non-goals in PRD #66: enemy ships, subsystem damage, debris, beam travel time, raycasting LOS, PvP, multiple beams, **shields, ship destruction / game-over**.

## Phase 2 — Draft 4 (Combat Update)

[Draft 4](../sources/design-04-combat-update.md) supersedes PRD #66's "single hull pool" with a richer model:

- **Four-quadrant shields** (fore/aft/port/starboard). Each shield absorbs damage before hull. Shields regenerate over time.
- **Phaser banks per quadrant.** Fire arc is determined by which bank.
- **Torpedoes.** Limited ammo, travel time, larger damage, ignores shields or hits hardest. (Exact model TBD in draft.)
- **Targeting moves to the directionally-relevant bank.**

This is a rewrite, not an extension. PRD #66's `ShipHullIntegrity` becomes one quadrant of a four-pool model. The upgrade path is real but non-trivial.

## Phase 3 — Draft 5 (Ship's Power)

[Draft 5](../sources/design-05-ships-power.md) makes Engineering's role active rather than reactive:

- **6 power points** distributed across Helm, Weapons, Shields, Sensors, Repair, etc.
- **Auxiliary battery** for short-term boosts.
- Power level **modulates tunables**: max thrust, weapon cooldown, shield regen rate, repair speed.

Effect on this roadmap: every constant in PRD #66 (5 dmg/s, 6 s cooldown, 1 HP / 3 s repair, 30 HP asteroid) becomes a *base* multiplied by Engineering's allocation. PRD #66 should keep these constants in one config struct so Draft 5 can wrap them later.

## Tensions and ordering

- **Ship hull pool model.** PRD #66 lands a single pool; Draft 4 replaces with quadrants. Either accept the rewrite later, or prototype quadrants now and ship "1 quadrant active, others pending" to reduce later churn. PRD #66's text is explicit about the single-pool choice.
- **Game-over.** PRD #66 explicitly excludes ship destruction. Combat without consequence is a stopgap. A future PRD must define what hull = 0 means.
- **Tuning surface.** Draft 5 multiplies almost every combat number. Centralise constants now.

## Cross-references

- Entity: [Asteroid](../entities/asteroid.md), [Ship](../entities/ship.md), [Bridge Crew Stations (planned)](../entities/bridge-crew-stations-planned.md)
- Source: [PRD #66](../sources/prd-066-weapons-and-engineering.md), [Draft 4](../sources/design-04-combat-update.md), [Draft 5](../sources/design-05-ships-power.md)
- Roadmap: [Console Expansion](./console-expansion.md), [Open Architectural Questions](./open-architectural-questions.md)
