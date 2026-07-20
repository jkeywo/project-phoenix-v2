---
title: System
type: entity
tags: [system, systemid, control-source, ai, wire-protocol, damage-tier]
sources: [src/ship/config.rs, src/ship/system_registry.rs, src/ship/damage_sync.rs, src/ship/damage.rs, src/ship/control_source.rs, assets/entities/alliance_battleship.toml]
updated: 2026-07-03
---

# System

A fine-grained capability instance on the ship. Systems are the addressable
units for the wire protocol (`ControlSystem` envelope) and for AI/human gating
(`ShipSystemControlSources`).

The ship currently has **10 coarse systems declared per-ship in TOML**
(station-owned or ownerless capability): captain, red-alert, helm, repair,
sensors, shields, navigation, power, comms, viewscreen. (Tactical was the
11th but its `[[system]]` block was removed in issue #512.) The Helm coarse
kind was decomposed into fine kinds in issue #511, and the Tactical coarse
`[[system]]` block was **deleted** in issue #512 in favour of `phaser_bank`,
`torpedo_tube`, and `torpedo_magazine` fine kinds — see the
[coarse-system migration](../concepts/coarse-system-migration.md#fine-system-ids-in-flight-prd-c)
table for the status of each decomposition. Fine kinds for Sensors, Shields,
and Power (#513–#515) are not yet decomposed.

## SystemId

A `SystemId` is a stable, lowercase-kebab string — e.g. `"helm-thrust"`,
`"red-alert"`, `"viewscreen"`. It is the `target` field in every
`ControlSystem` client message:

```json
{ "type": "ControlSystem",
  "data": { "target": "helm-thrust",
             "payload": { "type": "SetThrust", "data": { "value": 1.0 } } } }
```

Constant helpers live in `src/ship/system_registry.rs`
(`helm_thrust_system_id()`, `tactical_radar_system_id()`, etc.). Note `"helm"` and `"tactical"` are STATION ids, not system ids (issue #801): they key console-level blackboards and coordination via `helm_station_key()` / `tactical_station_key()`, and are never `ControlSystem` targets.

## TOML schema (`[[system]]`)

```toml
[[system]]
id = "helm-thrust"    # SystemId
kind = "helm_thrust"  # SystemKind — determines which handler owns this system
station = "helm"      # owning StationId
power_group = "helm"  # power allocation bucket

[system.config]       # optional; kind-specific opaque TOML consumed by the handler
```

Parsed into `SystemInstanceConfig` by `src/ship/config.rs`. The validator
checks that every `station` reference resolves to a declared `[[station]]` id
and that every `automated_systems` reference in a rating resolves to a declared
system owned by that station.

## ControlSource and ShipSystemControlSources

`ShipSystemControlSources` (a `HashMap<SystemId, ControlSource>` Bevy component)
maps each system to its current operator: `Human`, `Ai`, or `Offline`.

`ControlSource` exposes three gates used by handler systems:

| Gate | Human | Ai | Offline |
|---|---|---|---|
| `accept_human_input` | `true` | `false` | `false` |
| `operate_ai` | `false` | `true` | `false` |
| `coordinate` | `true` | `true` | `false` |

Both `accept_human_input` and `operate_ai` are `false` when the system is
offline (Disabled or Destroyed tier due to damage, or powered down).

### offline_systems override

`ControlSourceResolver` also carries `offline_systems: HashSet<SystemId>`.
When a system is in this set, `policy_for` returns the offline policy
**regardless** of the `ControlSource` value in `sources`. This allows
damage-driven gating to override the station rating without mutating the
rating itself.

The `sync_console_damage_tiers` Bevy system (runs in `SimSet::Damage`,
`src/ship/damage_sync.rs`) updates `offline_systems` every tick:

- `Disabled` or `Destroyed` console → `SystemId` added to `offline_systems`
- `Operational` or `Damaged` console → `SystemId` removed

## DamageTier (issue #507)

Each `ConsoleHull` entry carries configurable HP-fraction thresholds that map
HP to a `DamageTier` (`src/ship/damage.rs`):

| Tier | Condition |
|---|---|
| `Operational` | `current / max >= damaged_threshold_pct` |
| `Damaged` | `disabled_threshold_pct <= ratio < damaged_threshold_pct` |
| `Disabled` | `0 < ratio < disabled_threshold_pct` |
| `Destroyed` | `current == 0` |

Default thresholds (TOML `[[hull.system_hull]]`): `damaged_threshold_pct = 0.75`,
`disabled_threshold_pct = 0.25`. Overridable per system. The block also carries
an optional `display_name = "..."` string used for wire labels; when omitted
the wire falls back to the raw `system_id`.

`tier_for(system_id)` is the public API on `SystemHull` (the post-#619
replacement for `ConsoleHull`). The tier is also included in
`SystemHullStatus.tier` on the wire so clients can render tier badges.

## ActiveStationRatings

`ActiveStationRatings` (`HashMap<StationId, String>`) holds the current rating
name for each station. When a rating change arrives, the handler resolves the
new rating's `automated_systems` list and updates `ShipSystemControlSources`
accordingly.

## SystemKind

Determines which plugin/handler owns the system. The ship registry
(`src/ship/system_registry.rs`) maps `SystemKind` → handler. All
`SystemKind` values must appear in the registry before `ShipConfig` can be
validated — unknown kinds are rejected at startup.

Current coarse kinds: `captain`, `red_alert`, `helm`, `tactical`, `repair`,
`sensors`, `shields`, `navigation`, `power`, `comms`, `viewscreen`. (The
`tactical` kind is still registered in the runtime registry, but under #512
no `[[system]] kind = "tactical"` block exists on any ship — the kind is
retained as a coordination surface for ship-level Tactical operations
(SetTarget / SetPhaserMode / SetPhaserFrequency), gated on "any phaser bank
accepts human input".)

Current fine kinds registered in `SystemKindRegistry::with_core_systems`:
`helm_joystick`, `helm_engine`, `helm_radar`, `helm_impulse` (from #511);
`lateral_thrust`; `helm_thrust`, `helm_steering` (the per-axis split from
#701 — `helm_thrust` owns `ThrustInput`, `helm_steering` owns
`SteeringInput`); `phaser_bank`, `torpedo_tube`, `torpedo_magazine` (from
#512); `blaster_bank` (#631); `tactical_radar`, `sensor_radar`;
`power_reactor`, `power_battery` (#513); `shield_arc` (#514).

## Related

- [Station](./station.md) — the owning seat; holds the rating table
- [Console](./console.md) — GUI layer; separate from System
- player_ship.toml — TOML source
- Issue #518 — B1–B6 migration
- PRD #487 — architecture context
- [Coarse-system migration](../concepts/coarse-system-migration.md)
- [Issue #507](https://github.com/jkeywo/project-phoenix-v2/issues/507) — Per-system damage tiers
