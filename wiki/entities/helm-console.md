---
title: Helm Console
type: entity
tags: [console, helm, input, ship, physics, radar, impulse, boost]
sources: [gui/battleship/helm.html, gui/cruiser/helm.html, gui/destroyer/helm.html, gui/console-state.js, gui/components/ph-helm-radar.js, src/console/helm/server.rs, src/ship/helm_admission.rs, src/ship/physics_systems.rs, src/ship/physics.rs, src/ship/impulse.rs, src/ship/boost.rs, src/ship/impulse_boost_systems.rs, src/modifiers/coordination.rs, assets/entities/alliance_destroyer.toml]
updated: 2026-08-27
---

# Helm Console

The Helm console operates a hull's movement systems. Its available controls
come from the selected hull's authored Helm capabilities and station rating;
the shipped hull families provide their own HTML layouts.

## Control path

The panel emits `ControlSystem` commands to the fine Helm systems, including
thrust, steering, impulse, boost, and any lateral or vertical axes the hull
mounts. `src/console/helm/server.rs` publishes Helm state, while
`src/ship/helm_admission.rs` applies admitted commands to the per-axis command
components. `src/ship/physics_systems.rs` consumes those components during the
fixed simulation tick and passes the resulting inputs through the pure physics
model in `src/ship/physics.rs`.

Human and Backfill Helm use the same commands and physics path. Admission and
the station's active rating decide which source may operate each fine system;
the physics layer does not branch on who issued the command.

## Impulse and boost

Impulse and boost are optional, authored capabilities rather than universal
console constants.

- Charging impulse clears stale Helm inputs. Active impulse applies its
  authored acceleration, speed, and steering behavior through the same physics
  configuration used for ordinary movement.
- Boost applies authored speed, acceleration, and steering multipliers. Its
  battery drain scales with the absolute thrust and steering demand, so an
  engaged but idle drive does not spend charge.
- Damage and regions that block impulse can cancel or reject impulse through
  the authoritative simulation path.

The client derives the charging/active presentation from the published Helm
state; it does not run either state machine locally.

## Radar and coordination

`ph-helm-radar` renders the local Helm blackboard, including contacts, the
navigation waypoint, and hostile weapon-arc sectors published for visible
hostiles while Red Alert is active. Arc geometry is computed by the server and
sent to the panel; the component only projects it into scope coordinates.

Tactical can send an arc-bearing coordination request when a locked target is
in range but outside the selected usable weapon family's carried direct-fire
arcs. A human Helm sees the request on the console. Backfill Helm's receiver
accepts only an AI delivery for the authored Helm Station, rechecks the live
`helm-steering` policy, and preserves the exact arc geometry for the next
steering decision. A withdrawal clears that request across weapon families.

## Related

- [Ship Physics](../concepts/ship-physics.md)
- [AI Helm Decomposition](../concepts/ai-helm-decomposition.md)
- [Modifier Coordination](../concepts/modifier-coordination.md)
- [Radar Projection](../concepts/radar-projection.md)
