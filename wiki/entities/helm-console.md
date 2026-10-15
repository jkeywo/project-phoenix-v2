---
title: Helm Console
type: entity
tags: [console, helm, input, ship, physics, radar, impulse, boost, dock]
sources: [editor/workshop-transit.js, gui/workshop-transit-panel.js, gui/transit.js, gui/components/ph-transit-drives.js, gui/stations/transit-actions.js, gui/battleship/helm.html, gui/cruiser/helm.html, gui/destroyer/helm.html, gui/stations/helm-console.js, gui/console-state.js, gui/components/ph-helm-radar.js, gui/components/ph-helm-joystick.js, gui/stations/helm-actions.js, gui/gamepad-input.js, crates/phoenix-simulation/src/console/helm/server.rs, crates/phoenix-sim-gameplay/src/ship/helm_admission.rs, crates/phoenix-sim-gameplay/src/ship/physics_systems.rs, crates/phoenix-sim-gameplay/src/ship/physics.rs, crates/phoenix-sim-gameplay/src/ship/impulse.rs, crates/phoenix-sim-gameplay/src/ship/boost.rs, crates/phoenix-sim-gameplay/src/ship/impulse_boost_systems.rs, crates/phoenix-sim-gameplay/src/modifiers/coordination.rs, crates/phoenix-simulation/src/dock/server.rs, crates/phoenix-simulation/src/entities/spawner.rs, assets/entities/alliance_destroyer.toml, assets/entities/alliance_cruiser.toml]
updated: 2026-10-15
---

# Helm Console

The Helm console operates a hull's movement systems. Its available controls
come from the selected hull's authored Helm capabilities and station rating;
the shipped hull families provide their own HTML layouts.

## Control path

The panel emits `ControlSystem` commands to the fine Helm systems, including
thrust, steering, impulse, boost, and any lateral or vertical axes the hull
mounts. `crates/phoenix-simulation/src/console/helm/server.rs` publishes Helm state, while
`crates/phoenix-sim-gameplay/src/ship/helm_admission.rs` applies admitted commands to the per-axis command
components. `crates/phoenix-sim-gameplay/src/ship/physics_systems.rs` consumes those components during the
fixed simulation tick and passes the resulting inputs through the pure physics
model in `crates/phoenix-sim-gameplay/src/ship/physics.rs`.

An explicit `helm_control` binding profile enables the shared
`ph-helm-movement` widget. It shows separate Assisted/Direct mode controls for
each actual owned axis, desired speed/heading, and a Match Speed picker built
from permitted dated observations. Configured axes use the retained intent map
and Simulation applier described in [Helm Runtime](../concepts/helm-control-intent.md);
the older command components remain the route for unmigrated axes. The dedicated
Workshop movement form edits those bindings through accepted source/history,
with explicit inherited materialisation and a false disable override.

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
payload carries a dock view that is available, engaged, docked or separating, so a Helm with
no dock system renders nothing extra. The Alliance destroyer and the Alliance
cruiser author one; the battleship does not.

Active Dock authors a `dock_mating` component with approach terms and a
`power_input` component bound through the System's existing power group. Typed
boolean engagement and entity-target channels hold its shared command intent.
The rig sidecar supplies `dock`-prefixed mating plates; the shipped-content walk
in `crates/phoenix-simulation/src/entities/config_tests.rs` verifies those plates.
Spawn resolves the composition and markers into `DockControl` and `DockMarkers`.
`moves_body` explicitly opts moving Dock equipment into approach/clearance; it defaults false so installing a stationary berth or provider does not grant propulsion. Controlled moving nonship equipment shares `ShipPhysics` for its captured pose without gaining a Ship marker or another integrator. Normal release accepts the actual mating marker's away direction and retains one clear destination; lost power, Operation or source availability holds that destination without free movement. Stationary providers can release their existing coupling without moving. The phone keeps own separation progress visible while paused.
Passive berths retain a top-level `[dock]` endpoint declaration without an active
System; active Systems reject that competing legacy tuning table.

The shared Equipment display also lists Dock when it has no nearby target.
Current damage, insufficient power and missing rig markers produce status
reasons; absence of a target does not mean the equipment has failed.

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

## LocalJump

Installed Transit drives publish their own typed Console rows. The shared
`ph-transit-drives` widget shows only those permitted rows: accepted course,
preparation, fuel requirement and own actual arrival. Begin sends no position;
the actual drive freezes its permitted Navigation course. Cancel is available
only before commitment. Controls require that exact System's current Human
ownership, including on a read-only feed. No private gravity inputs or future
placement recipe enter the phone view.

The dedicated Workshop Transit form edits installed drive policy, exact Navigation
and Local fuel bindings through accepted source/history. It requires explicit
materialisation of inherited equipment, preserves other equipment and comments,
and provides the existing Preview/Test route. Its separate fixed-body gravity
controls accept a finite exclusion radius or an explicit false disable. The
actual backend validates which World placements may supply that policy.

## Related

- [Ship Physics](../concepts/ship-physics.md)
- [AI Helm Decomposition](../concepts/ai-helm-decomposition.md)
- [Modifier Coordination](../concepts/modifier-coordination.md)
- [Radar Projection](../concepts/radar-projection.md)
