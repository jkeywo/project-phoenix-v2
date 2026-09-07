---
title: Helm Console
type: entity
tags: [console, helm, input, ship, physics, radar, impulse, boost, dock]
sources: [gui/battleship/helm.html, gui/cruiser/helm.html, gui/destroyer/helm.html, gui/stations/helm-console.js, gui/console-state.js, gui/components/ph-helm-radar.js, gui/components/ph-helm-joystick.js, gui/stations/helm-actions.js, gui/gamepad-input.js, src/console/helm/server.rs, src/ship/helm_admission.rs, src/ship/physics_systems.rs, src/ship/physics.rs, src/ship/impulse.rs, src/ship/boost.rs, src/ship/impulse_boost_systems.rs, src/modifiers/coordination.rs, src/dock/server.rs, src/entities/spawner.rs, assets/entities/alliance_destroyer.toml, assets/entities/alliance_cruiser.toml]
updated: 2026-09-06
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

The real `helm.steering` semantic action maps the selected standard gamepad's
left-stick X axis onto the existing `SetSteering` command at an authored
100-millisecond cadence. The parent client owns explicit device selection,
deadzone/inversion tuning and disconnect/reconnect neutral gating; the Helm
iframe adapter and `action-map.js` keep using `helm-steering`, so this adds no
authority or protocol route. `ph-helm-joystick` retains pointer and WASD input
but no longer polls arbitrary gamepads itself.

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

## Contextual dock control

A hull whose Helm owns a `kind = "dock"` System carries a Dock control, and only
that hull: the control is a panel the shared renderer keeps hidden until the
payload carries a dock view that is available, engaged or docked, so a Helm with
no dock system renders nothing extra. The Alliance destroyer and the Alliance
cruiser author one; the battleship does not.

Two things have to be authored together for the control to work. The `[dock]`
table gives the approach terms — the reach the control appears within, the
engage distance, the approach speed, the mate tolerance, how far Undock backs
clear, and the lowest powered rung — and its presence alone is what makes a hull
dockable. The dock PLATES are markers in the hull's model rig sidecar whose names
begin with `dock`; a hull whose rig declares none can never mate, and validation
cannot catch that because it never opens a model file (the guard is the shipped
content walk in `src/entities/config_tests.rs`). Spawn reads both: `[dock]` plus
markers gives `DockMarkers` (dockable), and the `kind = "dock"` System on top of
them gives `DockControl` (an active docker).

The one button toggles Dock and Undock through the `helm.dock` semantic action,
so a human sends exactly the admitted command a dock AI sends; the adapter reads
the current authoritative dock view for the verb and the authored System id,
rather than the rendered label. Docking is FLOWN — the server closes the nearest
viable dock-marker pair — which is why the dock is Helm's and not Engineering's,
even though the umbilical that runs across the finished dock is Engineering's.

## Under tow load

A ship that mounts a `kind = "tractor"` System also carries an under-tow-load
banner on its Helm — the Alliance destroyer, and the Alliance cruiser since it
gained a tractor; the battleship mounts none and shows nothing. The banner stays
hidden until this ship's beam actually holds a target, and then names the held
hull, so the seat can see both that it is under load and why.

The beam itself is ENGINEERING's control on both hulls: an engineer grips a hull,
a helm officer flies one. What reaches the Helm is the consequence — the tow's
mass penalty on top speed and turn rate — which is why the banner is here and the
Engage/Release button is not. Both consoles read the one authoritative `tractor`
blackboard (`buildHelmTowLoadView` in `gui/console-state.js` on this side) rather
than each deriving a hold of their own, so they cannot disagree about whether a
tow is held.

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
