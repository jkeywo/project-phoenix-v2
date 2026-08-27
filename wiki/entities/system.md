---
title: System
type: entity
tags: [system, systemid, console-family, control-source, ai, wire-protocol, damage-tier]
sources: [src/ship/config.rs, src/ship/system_registry.rs, src/command_admission/policy.rs, src/command_admission/router.rs, src/server_app/registration.rs, src/world/server.rs, src/core/messages.rs, src/entities/spawner.rs, src/dock/server.rs, src/lobby/server.rs, src/ship/control_source.rs, src/ship/damage_sync.rs, src/ship/damage.rs, gui/sim-state.js, gui/console-state.js, gui/console-families.js, gui/console-payload.js, gui/dirty-consoles.js, gui/action-map.js, assets/entities/alliance_destroyer.toml]
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

`SystemKindDescriptor` in `src/ship/system_registry.rs` is the authoritative
metadata record for an authored kind. A kind chooses the server behaviour and
presentation classification; the instance id remains the identity carried by
topology, control-source and command state. It also declares whether the kind
accepts admitted commands, so passive topology does not manufacture a consumer
requirement.

The descriptor registry remains separate from the `AdmittedConsumerRegistry`:
domain plugins read `AdmittedCommands` on their existing schedules and register
the System kind plus the address domain their reader accepts. Fixed-id readers
claim an exact id, generated banks/tubes/arcs claim a prefix, and readers such
as Dock that carry the authored id claim any instance of their kind. Undeclared
host capabilities such as God Mode retain a separate exact-target claim.

The production coverage guard composes the real simulation and World plugins,
then derives required coverage from commandable descriptors and every resolved
top-level shipped hull. Each authored instance must fall inside its consumer's
declared domain; there is no parallel expected-id fixture. The same resolution
drives the end-of-frame warning for admitted commands with no matching
consumer.

System ids are lowercase kebab strings. A fine system normally combines its
capability and instance (`phaser-fore`, `torpedo-tube-aft`); a single coarse
capability can use the bare id (`captain`, `navigation`, `comms`). Kind strings
are registry keys and may use snake case. Use the helpers in
`src/ship/system_registry.rs` instead of duplicating stable ids in Rust.

Station ids and system ids are separate namespaces. `helm` and `tactical` are
station keys used for console-level blackboards and coordination; Helm axes and
Tactical operations target declared systems such as `helm-steering`,
`tactical-radar`, and `phaser-control`.

## Console Family

A Console Family selects the client payload builder and dirty-console routing
for a System's presentation. It does not own the System, grant access, or
replace a command target: Station topology remains the ownership source and the
authored `SystemId` remains command authority.

Every descriptor requires Console Family metadata. The host resolves it into
`ShipClientConfig.system_console_families`, keyed by the selected ship's actual
authored System instance ids, for `Welcome`. Tests parse every shipped hull and
assert that each instance projects through its kind descriptor, so arbitrary
instance ids work without client naming conventions.

Reserved and aggregate blackboard keys are deliberately not descriptors in the
System-kind table. `BlackboardKeyDescriptor` gives them an affected Console
Family in the separate `blackboard_console_families` projection. The six current
channels are Helm, Tactical, Power and Shields aggregates plus Dossiers and
Scan. They gain presentation routing only—not ownership, damage, control source
or command authority.

Dock is the complete identity tracer. Spawn resolves the authored
`kind = "dock"` instance into `DockControl.system_id`; command consumption, AI
policy, damage lookup and blackboard publication use that field. The client
reads the blackboard by the projected instance id and sends the same id back as
the Dock/Undock `ControlSystem.target`. The `"dock"` helper is only the
conventional shipped topology spelling.

The client consumes the complete projections directly. Builder selection,
dirty routing, flat normalization, visiting Systems and composite renderers use
actual owned ids; semantic blackboard lookup uses the wire discriminant. The
old exact/prefix matcher, inverse family-to-System list and Station-name boot
fallback are deleted.

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
