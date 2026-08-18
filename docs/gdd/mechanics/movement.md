# Project Phoenix — Movement and Helm

| Field | Value |
|---|---|
| Status | Current mechanic with accepted extensions |
| Scope | Ship motion, Helm authority, impulse, boost and manoeuvre coordination |
| Audience | Design, content, UI, simulation and playtest |

Movement in Project Phoenix is a crew decision expressed through a ship with mass, limits and damaged machinery. Helm chooses how the ship moves; Navigation supplies intent, Tactical may request a bearing, and neither silently takes the controls away.

Related documents: [Ships and Ship Systems](../systems/ships-and-systems.md), [Station Experiences](../systems/station-experiences.md), [Navigation and Relative Motion](./navigation-relative-motion.md), [Targeting and Weapons](./targeting-weapons.md), and [AI and Backfill](../systems/ai-and-backfill.md).

## Experience goals

- Make piloting readable enough for a first-time Helm player and deep enough for coordinated manoeuvres.
- Give ships distinct handling through authored capabilities and ratings rather than bespoke control rules.
- Turn damage, power and hazards into changed choices, not merely smaller numbers.
- Let AI operate the same control surfaces as a human without acquiring extra manoeuvring powers.
- Keep other stations influential while preserving Helm's final authority over motion.

## Authority and ownership

The host owns position, orientation, velocity and every propulsion state. Human and AI Helm both submit the same per-system commands, and one authoritative applier converts those commands into movement intent before physics integrates the ship.

Helm owns thrust, steering, lateral and vertical motion, impulse and boost where the hull provides them. Navigation owns the shared destination, not the actuators. Tactical can send an arc-bearing request when a usable weapon is blocked by geometry, but the request is advisory and changes steering intent only; it never adds thrust or forces a manoeuvre.

## Current control model

The control surface is decomposed by axis and capability. Forward thrust and steering are the baseline. Authored hulls may also expose lateral thrust, vertical thrust, impulse and boost. Missing, disabled or destroyed systems remove the corresponding control rather than simulating an input that the ship cannot obey.

Controls express intent continuously while the simulation owns acceleration and integration. A ship's authored values and live modifiers determine maximum speed, turn rate and response. Power and damage can therefore make the same input produce a different result without changing the player's control vocabulary.

Position and motion are authoritative in three axes, with yaw and roll represented in the ship state. The current player experience is still principally planar: most encounters, radar decisions and authored movement assume a navigable horizontal plane, while vertical capability is an explicit hull feature rather than an implicit requirement for every scenario.

## Impulse and boost

Impulse is a committed high-speed transit state. It charges, enters an active phase and accelerates the ship along its forward line. Steering is constrained by an authored multiplier so that impulse is a course choice rather than a faster version of ordinary dogfighting.

Boost is a shorter tactical burst that consumes the ship's available energy model. It is intended for closing, disengaging or clearing danger. Its duration, strength, recovery and compatibility with impulse are authored. The default design does not treat impulse and boost as stackable bonuses.

Both states need unmistakable feedback: availability, charge or remaining duration, the reason an activation was refused, and the ship-level consequence. A control that is absent because the hull lacks the capability should not look like a temporarily unavailable control.

## Coordination loop

Navigation sets or clears a shared waypoint. Helm sees the destination, current relative bearing and movement state, then decides how to pursue it. Tactical can request that a named weapon family be brought to bear when its target is in range but outside every usable emitter arc. Helm can accept the implied bearing, choose a safer route, or ignore it when another concern is more important.

This separation is deliberate. A waypoint is strategic intent, an arc request is tactical advice, and a Helm command is an actuator decision. Scenarios should create pressure between those layers rather than bypassing them.

## Damage, power and hazards

Engine and steering systems degrade through the common damage model. Damage may reduce performance, disable an axis or destroy a capability. Power allocation modifies propulsion performance and can prevent energy-intensive actions. Environmental effects can apply damage, reduce safe manoeuvring space or later provide graded hazard advice.

The player-facing model should answer three questions quickly: what the ship is doing, what it can currently do, and why those differ. A sluggish turn caused by damage must read differently from an impulse steering limit or a low-power modifier.

## AI and backfill

AI Helm runs on the shared fixed decision cadence and emits ordinary admitted Helm commands. Its axis policies operate from one coherent snapshot so thrust, steering and special-movement decisions do not reason about different moments. A human reclaiming Helm replaces AI operation through the common system-control source; no second flight model is involved.

AI may follow a Navigation waypoint only when the scenario's clearance permits that behaviour. It may respond to a Tactical arc request, but the request remains steering advice. These rules stop backfill from inventing objectives or accepting coordination that a human would be free to reject.

## Accepted direction beyond the current model

The accepted extension is a shared three-dimensional desired-motion and hazard-assessment layer. Navigation, AI Helm and coordination requests would contribute desired position, velocity or facing; fine actuators would still decide what the ship can achieve. Hulls could author bounded vertical lanes, full three-dimensional flight or other movement modes without replacing the common command path.

This extension must preserve human agency, deterministic fixed-tick decisions and ship-specific capability. It must not become an autopilot that silently overrides a crewed Helm station.

## Authoring and tuning

Movement values belong in hull and system TOML: available axes, acceleration, maximum speeds, yaw and roll response, impulse and boost timing, power interactions, damage thresholds and AI policies. Scenario scripts may set destinations, hazards and objectives, but should not hardcode a particular hull's handling assumptions.

## Playtest questions

- Can a new Helm player explain the difference between thrust, boost and impulse after one attempt?
- Can Helm tell whether a poor response is caused by damage, power, a special-movement state or the hull's normal limits?
- Do Navigation waypoints and Tactical bearing requests create useful conversation without feeling like remote control?
- Does AI backfill fly credibly without doing anything a human station could not do?
- Do distinct hulls feel different through authored handling while retaining the same basic control grammar?

## Canonical sources

- [Helm controls design](../../../pasm/spec/design/helm-controls.yaml)
- [Helm controls architecture](../../../pasm/spec/architecture/helm-controls.yaml)
- [Navigation design](../../../pasm/spec/design/navigation.yaml)
- [Weapons architecture](../../../pasm/spec/architecture/weapons.yaml)
- [Helm control intent wiki](../../../wiki/concepts/helm-control-intent.md)

