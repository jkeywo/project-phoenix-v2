---
title: Draft 3 — Science Console
type: source
tags: [draft, design, science, radar, impulse, system-chart]
source_path: docs/3. Draft Design - Science Console.md
status: draft
updated: 2026-05-22
---

# Draft 3 — Science Console

> **Status (2026-05-22) — as shipped:** The Science console was split into three consoles (`Sensors`, `Shields`, `Navigation`) plus a separate `Comms` console; see [Module Map in AGENTS.md](../../AGENTS.md). The early `[science_console]` block on `EntityConfig` (and its `ScienceConsoleConfig` type) was **deleted** in the 2026-05 entity-schema refactor — consoles are code-driven plugins, not data-driven entity sub-configs. The original design intent below still reads as the source of record for the long-range-radar / impulse-cancel / system-chart split that ultimately landed across Sensors + Shields + Navigation.

A new console with three responsibilities: long-range scanning, target designation, and a navigation aid.

## Long-range radar

- Like the Helm radar but **only shows important objects** (stars, planets) — no asteroids.
- Sendable to the viewscreen (a new Captain capability or a cross-console request).
- Science can **target** objects by clicking them.
- Science targets are highlighted on the **Weapons** radar.

## Impulse drive

- 10× normal speed, no steering.
- Helm requests a 6-second charge. Damage during charge cancels it.
- **Science can cancel impulse** from their console.

## System chart

- A new tab in the Science console — first design that introduces **multi-tab consoles**.
- Shows: star, planets, ring for each asteroid field, ship position.
- Sendable to the viewscreen.

> "All the numbers in this file should be configurable." — same content-as-data theme as [Draft 1](./design-01-entity-config-files.md).

## Implications

- Adds a third [View Mode](../concepts/view-modes.md) family: client-driven viewscreen content (Science's chart / long-range radar). Currently all view modes are captain-controlled.
- Cross-console interaction: Science targets influence Weapons radar. Needs a new `Target::One(weapons_token)` payload or a shared `ActiveScienceTarget` resource broadcast in `SimSnapshot`.
- Multi-tab consoles change the [Console Plugin Pattern](../concepts/console-plugin-pattern.md) — currently each plugin owns one UI tree.

## Open questions

- Who can send to the viewscreen — only the captain (current rule), or any console? Design implies the latter for Science.
- Does impulse "no steering" mean the steering axis is locked, or yaw rate is forced to zero?

## Cross-references

- Entity: [Bridge Crew Stations (planned)](../entities/bridge-crew-stations-planned.md)
- Concept: [Radar Projection](../concepts/radar-projection.md), [View Modes](../concepts/view-modes.md), [Console Plugin Pattern](../concepts/console-plugin-pattern.md)
