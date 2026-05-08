---
title: PRD #66 — Weapons & Engineering Consoles
type: source
tags: [prd, weapons, engineering, combat, hull, repair, open]
source_url: https://github.com/jkeywo/project-phoenix-v2/issues/66
status: open (created 2026-05-08)
updated: 2026-05-08
---

# PRD #66 — Weapons Console & Engineering Console

Adds the first combat loop and a damage/repair system that requires the crew to communicate.

## Problem

There's no combat. Collisions zero velocity but cause no consequence. Players can't defend the ship, destroy obstacles, or feel ship integrity.

## Solution

Two new consoles plus a ship-wide Hull Integrity system.

- **Weapons** locks targets on a 60-unit ship-aligned radar (360°), fires a 6 s phaser beam (5 dmg/s = 30 total) within 40 u and the 180° forward arc, then cools down 6 s.
- **Engineering** owns Hull Integrity (0–100, 10-segment bar) and the breakdown queue.
- **Every console gets a Repair button** — but only the *correct* console for the active breakdown actually repairs. Wrong presses get a 30 s red-flash penalty cooldown. The crew has to talk.

## Key decisions

- **Lock vs Fire are different gates.** Lock works in 360° within 60 u. Fire requires 40 u + 180° forward arc. Arc check on Fire only.
- **Sever conditions** (any one): target destroyed, target out of arc, target out of range. Sever triggers immediate cooldown.
- **No damage refund on sever.**
- **Asteroids gain UUIDs and 30 HP.** No subsystem damage — single hull pool.
- **Collision damage:** `5 + (forward_speed / max_speed) * 10`, clamped 5..15.
- **Repair:** 1 HP / 3 s for 30 s = +10 HP per action. Same 30 s cooldown for wrong-console penalty.
- **Breakdown queue:** every 10 HP lost queues one breakdown to a random console. Never the same console twice in a row. No max queue length. Engineering sees one active assignment at a time.
- **Per-console message routing.** Direct `Target::One(token)` payloads at 10 Hz so Weapons/Engineering only receive what they need.
- **Beam rendered server-side only** (line/glow on viewscreen). Asteroid destruction plays a radial ripple and despawns.

## Schema additions (planned)

- `Console`: `Weapons`, `Engineering`
- `AsteroidInfo`: `id: Uuid`
- `SimSnapshot`: `hull_integrity: i32`, `authorized_repair_console: Option<Console>`
- New components: `AsteroidDamage`, `ShipHullIntegrity`, `BreakdownQueue`, `ActiveBeam`
- New `ClientMessage`: `SetTarget`, `FirePhaser`, `Repair`
- New `ServerMessage`: `TargetLock`, `RepairState`, `DamageReport`

## Out of scope

Enemy ships / counter-fire, subsystem damage, debris, beam travel time, line-of-sight raycasting, PvP, weapon resource management, multiple beams, advanced targeting modes, **shields**, ship destruction / game-over.

→ Many of these (shields, torpedoes) are picked up by [Draft 4 — Combat Update](./design-04-combat-update.md). Power management is in [Draft 5 — Ship's Power](./design-05-ships-power.md).

## Open questions

- The PRD says `Target::One(token)` direct routing. The current `OutboundMessage` routing target enum (`All` / `Token` / `AllExcept`) already supports this — implementation should reuse, not invent.
- Repair button on *every* console implies UI work in every existing Console Plugin. The first plugin to land Weapons/Engineering will need to retrofit Captain and Helm.

## Cross-references

- Entity: [Bridge Crew Stations (planned)](../entities/bridge-crew-stations-planned.md), [Asteroid](../entities/asteroid.md), [Ship](../entities/ship.md)
- Concept: [Console Plugin Pattern](../concepts/console-plugin-pattern.md), [Radar Projection](../concepts/radar-projection.md)
- Roadmap: [Combat & Damage](../roadmap/combat-and-damage.md), [Console Expansion](../roadmap/console-expansion.md)
