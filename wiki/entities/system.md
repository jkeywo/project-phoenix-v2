---
title: System
type: entity
tags: [system, systemid, console-family, control-source, ai, wire-protocol, damage-tier]
sources: [crates/phoenix-sim-gameplay/src/ship/config.rs, crates/phoenix-sim-gameplay/src/ship/system_registry.rs, crates/phoenix-simulation/src/command_admission/policy.rs, crates/phoenix-simulation/src/command_admission/router.rs, crates/phoenix-simulation/src/server_app/registration.rs, crates/phoenix-simulation/src/world/server/mod.rs, crates/phoenix-model/src/messages.rs, crates/phoenix-simulation/src/entities/spawner.rs, crates/phoenix-simulation/src/dock/server.rs, crates/phoenix-simulation/src/lobby/server.rs, crates/phoenix-sim-gameplay/src/ship/control_source.rs, crates/phoenix-sim-gameplay/src/ship/coordination.rs, crates/phoenix-simulation/src/ship/damage_sync.rs, crates/phoenix-sim-gameplay/src/ship/damage.rs, gui/sim-state.js, gui/console-state.js, gui/console-families.js, gui/console-payload.js, gui/dirty-consoles.js, gui/action-map.js, assets/entities/alliance_destroyer.toml, crates/phoenix-sim-gameplay/src/ship/composition.rs, crates/phoenix-simulation/src/server_app/system_status.rs, gui/components/ph-system-status.js]
updated: 2026-10-14
---

# System

A System is an addressable capability instance on a ship. `SystemId` is the
target of every `ClientMessage::ControlSystem` command, the key used for
human/AI control policy, and the unit that can be disabled by system damage.

## Identity and authoring

Hull TOML declares systems with a stable id, a kind, an owning Console, and optional power/config data:

```toml
[[system]]
id = "helm-thrust"
kind = "helm_thrust"
console = "helm"
power_group = "helm"
```

`SystemInstanceConfig` parses this shape. Ship validation requires every kind
to be registered, every Console reference to resolve, and every system named by
a rating to exist and belong to its Console or home Station.

`SystemKindDescriptor` in `crates/phoenix-sim-gameplay/src/ship/system_registry.rs` is the authoritative
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
An owner using payload-specific registrations explicitly marks capability
ownership beside its real consumers. This establishes machinery coverage and
GM availability for that kind without widening command or feedback payloads.
Supplementary stock, fitting and reference handlers do not establish this
coverage merely by handling one shared command.

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
`crates/phoenix-sim-gameplay/src/ship/system_registry.rs` instead of duplicating stable ids in Rust.

Station, Console and System ids are separate namespaces. `helm` and `tactical`
remain publication keys for composed observations; machinery commands target
Systems such as `helm-steering`, `tactical-radar` and `phaser-control`.
Coordination uses explicit `Station`, `Console` or `Ship` addresses. Queued
Console messages resolve the current host at delivery.

## Supported composition

Dock is the first composed System. Its typed `dock_mating` component binds a
`power_input` component and shared boolean engagement/entity-target channels.
Component IDs and channel names are local to the owning System. The existing
Dock/Undock admission and individual AI write the same canonical intent;
components do not introduce another controller or command authority.

`crates/phoenix-sim-gameplay/src/ship/composition.rs` owns the supported palette
and validates references before spawn. Active tuning lives in the component;
passive berths retain their endpoint declaration. Current docking supports one
active Dock System per entity. Other System kinds retain their existing shape.
`crates/phoenix-simulation/src/dock/server.rs` carries mating and undocking
clearance separately from intent, with both persisted and included in digests.
Its explicit `moves_body` equipment flag separates paid moving Dock machinery from stationary berth/service markers. Whole-candidate and single-row Dock validation checks accepted channel shape, phase, finite clearance and the existing physical pose before application; the helper is `dock::server::validate_saved_entity`. No additional saved timer or pose is introduced.

## Common status

`crates/phoenix-simulation/src/server_app/system_status.rs` derives status from
current System damage, control-source availability and Dock hardware/power.
It publishes observations, not another authoritative machinery store. Detailed
rows follow the owning Console's current host; co-hosted tabs still filter to
that Console. Shared Repair feeds retain their existing damage-only scope.

`gui/components/ph-system-status.js` mounts through Console core, including GM
Station operation. It lists owned equipment with green, amber, red or unknown
indicators and a live details popup; delegated equipment is grouped into an
inspectable summary. Idle equipment and a missing nearby docking target are
not failures. EMP/heat conditions are not fabricated ahead of their mechanisms.

## Console Family

A Console Family selects the client payload builder and dirty-console routing
for a System's presentation. It does not own the System, grant access, or
replace a command target: System → Console ownership and current Console hosting determine authority; the
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

Console ratings choose Human or AI per owned System. The implicit Backfill
rating delegates every System of an unhosted Console to AI. Damage adds an
independent offline override, so a Disabled or Destroyed system remains inert
regardless of the station rating until repair clears the override.

Command admission follows System → Console → current hosting Station and checks
the effective policy and Station tenure before a
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
- [Individual System AI State Trees](../concepts/ai-state-trees.md)
- [Damage and Repair Runtime](../concepts/damage-and-repair-intent.md)
