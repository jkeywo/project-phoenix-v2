---
title: AI Ship Unification
type: concept
tags: [ai, npc, ship, ecs, components, control-source, backfill]
sources: [src/entities/spawner.rs, src/ship/control_source.rs, src/ship_plugin.rs, src/ship/helm_ai/, src/console/weapons/server.rs, src/console/weapons/beam.rs, src/tractor/server.rs, src/console/navigation/server.rs, src/ai/server.rs, src/ai/host.rs]
updated: 2026-08-27
---

# AI Ship Unification

The LocalShip and NPC ships use the same ECS model and the same authoritative
appliers. AI is a command producer selected per fine system, not a second
simulation path.

## Per-ship state

Runtime ships carry their own authored `ShipConfigComponent`,
`ShipSystemControlSources`, active station ratings, system blackboards,
physics/damage state, and any capability-specific components their template
declares. Coordination queues and policy state are also scoped to the ship that
owns them.

An NPC template with behaviour is seeded for AI operation. The LocalShip changes
each system's control source from live station tenure/rating: an occupied
eligible station is human-operated; a vacant or disconnected station uses its
Backfill rating. The downstream system sees only the resulting control policy.

## One command path

Each AI host runs only when its fine system's `ControlTickPolicy.operate_ai` is
true. It reads the same blackboard/coordination facts exposed to that station,
chooses a deterministic action, and emits an ordinary `ControlSystem` command
through the admission seam. The domain's shared applier consumes human and AI
commands identically.

Examples:

- the six Helm hosts live under `src/ship/helm_ai/`;
- `ai_target_selection` and weapon-family hosts live under
  `src/console/weapons/`;
- Captain, Sensors, Navigation, Repair, Comms, Shields, and Power each own their
  domain host;
- scenario operation hosts use the same fine-system objectives and command
  adapters as their human controls.

All policy hosts use the logical-tick cadence in `src/ai/cadence.rs`. World
snapshots and policy memory are deterministic and snapshot-safe.

A named operation objective may acquire its exact non-hostile target inside
Tactical radar or the installed operation's authored three-dimensional reach
(Tractor or external repair); ordinary contacts remain radar-gated. Once a
Tractor actually couples that exact active target, the authoritative coupling
keeps it eligible beyond those horizons until either the Tractor directive or
the coupling ends. Other operation kinds, including FieldRepair, do not inherit
that Tractor-only retention.

## Fidelity

`AiHighFidelity` selects the full ship simulation path for nearby NPCs. Distant
NPCs use the deterministic low-fidelity adapter in `src/ai/server.rs`; they
retain authored objectives and cursors rather than becoming a separate class of
entity. The LocalShip always runs the full authoritative path.

## Related

- [AI Helm Decomposition](./ai-helm-decomposition.md)
- [Information-Parity Audit](./information-parity-audit.md)
- [Ship](../entities/ship.md)
- [System](../entities/system.md)
