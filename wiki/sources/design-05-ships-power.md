---
title: Draft 5 — Ship's Power
type: source
tags: [draft, design, engineering, power, battery]
source_path: docs/5. Draft Design - Ship's Power.md
status: draft
updated: 2026-05-08
---

# Draft 5 — Ship's Power

Engineering distributes power between Helm, Weapons, and Science. Adds an aux battery.

## Distribution

- **6 power points** to distribute across Helm / Weapons / Science.
- Each console can have **1 to 4** points.
- **2 points = normal values** for that console.
- Power affects: movement values (Helm), damage (Weapons), shields (Science) **and** the speed of each console's repair button.

## Aux battery

- Adds **up to 2 extra power points**.
- Each assigned aux point drains the battery:
  - 1 aux point → 60 s to drain.
  - 2 aux points → 20 s to drain.
- Recharge: **60 s to full** from empty.

## Implications

- New Engineering UI: 3 sliders/dials (Helm/Weapons/Science) plus a battery widget.
- Power values multiply existing tunables. `ShipPhysicsConfig.max_speed`, weapons damage, shield regen, repair rate all become functions of (assigned points + aux).
- Crucially affects the [PRD #66](./prd-066-weapons-and-engineering.md) repair loop — repair speed is now power-modulated, so Engineering must trade repair speed against combat performance.
- Tunables-as-data ([Draft 1](./design-01-entity-config-files.md)) makes the multipliers configurable per ship type.

## Open questions

- What does each power level (1, 2, 3, 4) actually multiply by? Linear? Exponential?
- Does Helm at 1 power get *worse* than current values, or just slower? Same for the others.
- "Speed of repair button" — affects cooldown, the per-tick HP regen, or both?

## Cross-references

- Source: [PRD #66](./prd-066-weapons-and-engineering.md) (repair loop)
- Roadmap: [Console Expansion](../roadmap/console-expansion.md), [Combat & Damage](../roadmap/combat-and-damage.md)
