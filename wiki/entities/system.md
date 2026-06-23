---
title: System
type: entity
tags: [system, systemid, control-source, ai, wire-protocol]
sources: [src/ship/config.rs, src/ship/system_registry.rs, src/ship_plugin.rs, assets/entities/player_ship.toml]
updated: 2026-06-23
---

# System

A fine-grained capability instance on the ship. Systems are the addressable
units for the wire protocol (`ControlSystem` envelope) and for AI/human gating
(`ShipSystemControlSources`).

The ship currently has **11 coarse systems**: captain, red-alert, helm,
tactical, repair, sensors, shields, navigation, power, comms, viewscreen.
Fine-grained per-bank and per-tube systems (phaser-fore, torpedo-tube-fore-port,
etc.) are out of scope for PRD #518 and addressed in #511–515.

## SystemId

A `SystemId` is a stable, lowercase-kebab string — e.g. `"helm"`,
`"red-alert"`, `"viewscreen"`. It is the `target` field in every
`ControlSystem` client message:

```json
{ "type": "ControlSystem",
  "data": { "target": "helm",
             "payload": { "type": "HelmInput", "data": { "thrust": 1.0, "steering": 0.0 } } } }
```

Constant helpers live in `src/ship/system_registry.rs`
(`helm_system_id()`, `tactical_system_id()`, etc.).

## TOML schema (`[[system]]`)

```toml
[[system]]
id = "helm"           # SystemId
kind = "helm"         # SystemKind — determines which handler owns this system
station = "helm"      # owning StationId
power_group = "helm"  # power allocation bucket

[system.config]       # optional; kind-specific opaque TOML consumed by the handler
```

Parsed into `SystemInstanceConfig` by `src/ship/config.rs`. The validator
checks that every `station` reference resolves to a declared `[[station]]` id
and that every `automated_systems` reference in a rating resolves to a declared
system owned by that station.

## ControlSource and ShipSystemControlSources

`ShipSystemControlSources` (a `HashMap<SystemId, ControlSource>` Bevy resource)
maps each system to its current operator: `Human` or `Ai`.

`ControlSource` exposes two gates used by handler systems:

| Gate | Human | Ai |
|---|---|---|
| `accept_human_input` | `true` | `false` |
| `operate_ai` | `false` | `true` |

Both gates are `false` when the system is offline (damaged / powered down).

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
`sensors`, `shields`, `navigation`, `power`, `comms`, `viewscreen`.

## Related

- [Station](./station.md) — the owning seat; holds the rating table
- [Console](./console.md) — GUI layer; separate from System
- [player_ship.toml](../sources/player_ship_toml.md) — TOML source
- [Issue #518](../sources/issue-540-config-migration-docs.md) — B1–B6 migration
- [PRD #487](../sources/prd-487-station-console-system-redesign.md) — architecture context
- [Coarse-system migration](../concepts/coarse-system-migration.md)
