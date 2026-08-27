---
title: System
type: entity
tags: [system, systemid, control-source, ai, wire-protocol, damage-tier]
sources: [src/ship/config.rs, src/ship/system_registry.rs, src/ship/control_source.rs, src/ship/damage_sync.rs, src/ship/damage.rs, src/command_admission/policy.rs, assets/entities/alliance_destroyer.toml]
updated: 2026-08-27
---

# System

A System is an addressable capability instance on a ship. `SystemId` is the
target of every `ClientMessage::ControlSystem` command, the key used for
human/AI control policy, and the unit that can be disabled by system damage.

## Identity and authoring

Hull TOML declares systems with a stable id, a kind, an owning station where
applicable, and optional power/config data:

```toml
[[system]]
id = "helm-thrust"
kind = "helm_thrust"
station = "helm"
power_group = "helm"
```

`SystemInstanceConfig` parses this shape. Ship validation requires every kind
to be registered, every station reference to resolve, and every system named by
a station rating to exist and belong to that station.

System ids are lowercase kebab strings. A fine system normally combines its
capability and instance (`phaser-fore`, `torpedo-tube-aft`); a single coarse
capability can use the bare id (`captain`, `navigation`, `comms`). Kind strings
are registry keys and may use snake case. Use the helpers in
`src/ship/system_registry.rs` instead of duplicating stable ids in Rust.

Station ids and system ids are separate namespaces. `helm` and `tactical` are
station keys used for console-level blackboards and coordination; Helm axes and
Tactical operations target declared systems such as `helm-steering`,
`tactical-radar`, and `phaser-control`.

## Control policy

`ShipSystemControlSources` holds a `ControlSourceResolver` for each ship. Each
system resolves to one policy:

| Source | Human input | AI operation | Coordination |
|---|---:|---:|---:|
| Human | yes | no | yes |
| AI | no | yes | yes |
| Offline | no | no | no |

Station ratings choose Human or AI per owned system. The implicit Backfill
rating delegates every system of an unmanned station to AI. Damage adds an
independent offline override, so a Disabled or Destroyed system remains inert
regardless of the station rating until repair clears the override.

Command admission checks the effective policy and station tenure before a
human command enters the simulation. AI policy hosts consult the same
`operate_ai` field before emitting their corresponding command.

## Damage state

`[[hull.system_hull]]` entries give damageable systems their HP and thresholds.
`SystemHull::tier_for` classifies them as Operational, Damaged, Disabled, or
Destroyed. `sync_console_damage_tiers` translates Disabled/Destroyed into the
resolver's offline set and removes that override after repair. The tier and
display label are also published for console presentation.

## Related

- [Station](./station.md)
- [Console](./console.md)
- [System Addressing](../concepts/coarse-system-migration.md)
- [Damage and Repair Runtime](../concepts/damage-and-repair-intent.md)
