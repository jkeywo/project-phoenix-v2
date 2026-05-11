---
title: Draft 10 — Region Entities
type: source
tags: [draft, region, environment, modifier, scenario]
source_path: docs/10. Draft Design - Region Entities.md
status: draft
updated: 2026-05-11
---

# Draft 10 — Region Entities

Invisible trigger volumes in space. Position + radius, no visible representation during gameplay. Used by the scenario system for trigger conditions and by ship systems for environmental effects.

## Entity config

```toml
tags = ["region", "blocks_impulse"]
position = [5000.0, 0.0, -2000.0]
radius = 1500.0

# Only applied if tagged "radar_dampening"
radar_range_multiplier = 0.4

# Only applied if tagged "damage_zone"
damage_per_second = 5.0
```

## Tags

| Tag | Effect |
|---|---|
| `region` | Base tag identifying a region volume. |
| `blocks_impulse` | Cancels active impulse charge and prevents new charges while inside. |
| `radar_dampening` | Applies `radar_range_multiplier` to Science radar range while inside. |
| `damage_zone` | Applies `damage_per_second` hull damage while inside (bypasses shields). |

Multiple effect tags can combine (e.g. a nebula could be `radar_dampening` + `damage_zone`).

## Trigger integration

Regions can be referenced by an `on_entered_region` trigger condition. The region must carry an `id` for that:

```toml
id = "forbidden_zone"
tags = ["region", "blocks_impulse"]
```

```toml
[[trigger]]
condition = "on_entered_region"
entity = "forbidden_zone"
actions = [ { load_scenario = "scenarios/warning_broadcast.toml" } ]
```

## Spawning

Regions are spawned by scenarios like any other entity. Owned by the spawning scenario and despawn on unload.

## Cross-references

- Effects map naturally onto [PRD #117 — Modifier System](./prd-117-modifier-system.md) (`RegionEffect { region_id }` source variant exists for this).
- [Draft 7 — Scenario File](./design-07-scenario-file.md) — `on_entered_region` is region-specific.
- [PRD #119 — Stations, Scenarios, Comms](./prd-119-stations-scenarios-comms.md) — explicitly out of scope; regions are a follow-on.
- [Roadmap Overview](../roadmap/overview.md)
