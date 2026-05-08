---
title: Draft 4 — Combat Update
type: source
tags: [draft, design, combat, phaser, torpedo, shield]
source_path: docs/4. Draft Design - Combat update.md
status: draft
updated: 2026-05-08
---

# Draft 4 — Combat Update

Extends the Weapons system from [PRD #66](./prd-066-weapons-and-engineering.md) with phaser banks, torpedoes, and four-quadrant shields.

## Phaser banks

- **Two banks**, port and starboard, each with a **270° fire arc**.
- (PRD #66 has a single forward 180° arc with no bank distinction.)

## Torpedoes

- Limited supply: **10 torpedoes**, **50 damage** each (5 vs shielded targets).
- Two **fore** torpedo tubes (90° fire arc, 10 s reload each) + **one aft** tube.
- Torpedoes **travel fast** and **adjust course** to hit the target.
- Visual: bright yellow ball.

## Shields

- **Four quadrants** tracked individually: fore, aft, port, starboard.
- **20 HP each**, regenerate at **1 HP / 3 s**.
- A fully destroyed shield goes **offline for 10 seconds**.
- **Science** sees the shield status (ties into Draft 3).

## Implications

- Conflicts with PRD #66's "single hull pool, no shields" — Draft 4 supersedes that decision.
- Quadrant shields require collision/beam contact-point detection to determine which quadrant takes damage. Rapier collision events expose contact normals.
- Torpedoes need a homing-projectile entity type (new). Course-adjustment is non-trivial.
- The `Vec<Console>` in `Player.consoles` plus per-console messaging makes "Science sees shield status" a new SimSnapshot field broadcast `Target::One(science_token)`.

> "All the numbers in this file should be configurable." — same data-driven theme.

## Open questions

- Do torpedo tube fire arcs (90°) compose with phaser arcs (270°), or do they share the 360°?
- Damage routing when shields are mixed: 50 dmg torpedo into a 20 HP shield — does the overflow hit hull or absorb fully?
- Shield "offline 10 s" — does it recharge from 0 HP or stay at 0 until the offline period ends?

## Cross-references

- Source: [PRD #66](./prd-066-weapons-and-engineering.md) (which this extends/contradicts)
- Roadmap: [Combat & Damage](../roadmap/combat-and-damage.md)
