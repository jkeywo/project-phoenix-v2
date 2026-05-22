---
title: Draft 6 — Space Stations
type: source
tags: [draft, design, stub]
source_path: docs/6. Draft Design - Space Stations.md
status: draft (stub)
updated: 2026-05-22
---

# Draft 6 — Space Stations

> **Status (2026-05-22) — partially shipped:** Space stations are now first-class data-driven entities. They live in `assets/entities/station_*.toml`, composed from the generic `[mesh]` + `[hull]` + optional `[collider]` blocks on `EntityConfig` — there is **no** dedicated `[station]` block or `StationConfig` type (deleted in the 2026-05 entity-schema refactor). Station hull damage is tracked via `[hull].hull_integrity`, which feeds the same `apply_hull_damage` path as the player ship (`src/ship/damage.rs`). Interaction surfaces (docking, repair, refuel, mission start) remain unscoped.

> Original source content: `-TODO-`

The file exists as a placeholder. No design has been written yet.

## Likely scope (inferred from project context)

Space stations would presumably:

- Be persistent world entities (more like planets than asteroids).
- Provide some interaction (docking, repair, refuel, mission start?).
- Possibly host scenarios — see [Draft 7 — Scenario File](./design-07-scenario-file.md).

## Cross-references

- Roadmap: [Open Architectural Questions](../roadmap/open-architectural-questions.md)
