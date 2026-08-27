---
title: System Addressing
type: concept
tags: [stations, systems, system-registry, command-admission]
sources: [src/ship/system_registry.rs, src/ship/config.rs, src/ship/control_source.rs, src/command_admission/policy.rs, src/command_admission/router.rs, src/ship/coordination_systems.rs, src/ship/rating_systems.rs]
updated: 2026-08-27
---

# System Addressing

The runtime uses one system-addressed command path. Every human console and AI
controller emits `ControlSystem { target: SystemId, payload }`; command
admission resolves the target's owning station and effective control policy
before the payload reaches a system handler.

## Identifier namespaces

| Namespace | Meaning | Examples |
|---|---|---|
| `SystemId` | Declared capability instance; command, damage, power, and control-policy address | `helm-thrust`, `phaser-fore`, `navigation` |
| `StationId` | Direct or auxiliary operable surface; owns systems and ratings | `helm`, `tactical`, `engineering` |
| Blackboard key | Published state address; normally a system id, with explicit station-key helpers for composed console views | `power-reactor`, Helm station key |

Wire command targets are system ids. Station ids are not a fallback command
surface. Helm and Tactical are composed stations, so their commands target
their fine systems while their aggregate console blackboards use
`helm_station_key()` and `tactical_station_key()`.

## Coarse and fine systems

A system is coarse when one capability has one useful control boundary, such
as Captain, Comms, Navigation, Repair, or Command. A system is fine-grained
when independently operated/damaged instances matter:

- Helm has separate thrust, steering, impulse, boost, lateral, vertical, radar,
  joystick, and engine systems as supported by the hull.
- Tactical targets the tactical radar and phaser-control systems, individual
  phaser/blaster banks, torpedo tubes, and the magazine.
- Power uses reactor and battery systems while publishing an aggregate Power
  console view.
- Shield arcs are generated from authored `[[shield_arc]]` entries and are
  controlled and damaged per arc.
- Coupling capabilities such as tractor, dock, umbilical, and external repair
  are first-class systems with ordinary admission, power, damage, AI, and
  blackboard behavior.

The split is data-driven per hull. Client code and AI must inspect authored
systems rather than assume a fixed suite or instance count.

## Control and coordination

`ControlSourceResolver::policy_for` is the shared Human/AI/Offline gate. Station
ratings populate its per-system sources, while damage contributes an overriding
offline set. Human-seeking station resolution changes which directly held
station may present a surface; it does not transfer system ownership or invent
a new command target.

Cross-system requests use the coordination channel and target the appropriate
station key or system contract. They never bypass command admission to mutate
another system's state directly.

## Related

- [System](../entities/system.md)
- [Station](../entities/station.md)
- [AI Ship Unification](./ai-ship-unification.md)
- [Modifier Coordination](./modifier-coordination.md)
