# Project Phoenix — Navigation and Relative Motion

| Field | Value |
|---|---|
| Status | Current waypoint and chart mechanic with accepted 3D extension |
| Scope | System chart, destinations, anchored waypoints, route advice, traffic orders and reference frames |
| Audience | Design, content, UI, simulation and playtest |

Navigation decides where the ship should go and how the wider situation is changing. It provides a stable strategic picture and shared destinations while leaving the physical act of flying to Helm.

Related documents: [Movement and Helm](./movement.md), [Sensors and Epistemics](./sensors-epistemics.md), [Comms and Commitments](./comms-commitments.md), [Station Experiences](../systems/station-experiences.md), and [Falling Skyway](../content/scenarios/falling-skyway.md).

## Experience goals

- Give Navigation a useful scale and timescale distinct from Helm's immediate controls.
- Make destinations shared crew intent without turning waypoints into autopilot.
- Represent moving targets and civilian traffic without forcing players to estimate everything by eye.
- Keep reference frames consistent enough that spoken bearings, chart positions and radar agree.
- Prepare for richer three-dimensional and orbital scenarios without making simple scenarios harder.

## System chart

The chart is a north-up, world-anchored view of the scenario. Its authored configuration controls range and which entity tags are shown or selectable. This stable frame supports route planning and conversation about fixed places even while the ship rotates.

Chart selection is local UI state until the player commits an action. Selecting a contact does not change the Combat Lock, Science Target or waypoint. This prevents browsing the map from issuing accidental ship commands.

The chart should show own-ship position, eligible contacts, the active waypoint and enough relative-motion cues to answer whether a destination is closing, receding or crossing. Detail must remain legible on a phone; history trails and vectors should be optional layers rather than permanent clutter.

## Waypoints

Each ship has at most one authoritative shared waypoint. A free waypoint stores fixed world coordinates. An anchored waypoint stores an entity reference and follows that entity's live position. If the anchor ceases to exist, the waypoint clears rather than silently becoming a stale coordinate.

Navigation sets or clears the waypoint through the admitted command path. Helm and other relevant radar surfaces receive the result. A waypoint is a desired destination and carries no direct thrust, steering or arrival action.

The host rejects non-finite coordinates and invalid anchors. The UI should distinguish a fixed point from a moving anchor and confirm which entity will be followed before committing.

## Helm boundary

Helm decides how to pursue the destination. It weighs hazards, ship handling, current combat geometry and other requests. A human Helm player is never forced onto the Navigation route.

AI Helm follows the shared waypoint only when the scenario or command clearance permits NavigateTo behavior. Reclaiming or backfilling either station must preserve the waypoint. If Helm changes from human to AI, the existing shared destination may be reissued to the AI rather than reconstructed from client state.

## Navigation AI

AI Navigation ranks positive Helm-relevant objectives and eligible chart contacts through authored policy. It then emits the same set or clear waypoint command available to a human. It does not bypass Helm by writing velocity or thrust.

An AI-generated waypoint should be explainable through the current objective or selected contact. If no valid destination exists, clearing the waypoint is preferable to inventing a patrol point in code.

## Civilian traffic and orders

Navigation may receive a traffic picture for civilian or scenario craft: current route, route leg, standing order and compliance state. Orders such as hold, divert or dock are requests to another actor, not direct movement commands.

An order can be delivered and then accepted, refused or violated. A transport failure is different from an in-fiction refusal. Authored response clocks allow the crew to wait for a decision rather than treating silence as immediate noncompliance.

This makes Navigation a relationship-facing station as well as a map station. The crew may need Comms context, authority or a promise before traffic follows an otherwise sensible route.

## Relative-motion language

Player-facing calculations should distinguish world position, ship-relative bearing and closing behavior. The chart remains world-anchored; Helm and Tactical radar may be ship-relative. Labels and coordination payloads must identify the frame rather than presenting an unexplained angle.

The minimum useful relative-motion set is distance, bearing, relative speed or closing state, and estimated arrival where a stable estimate exists. The design avoids presenting false precision during abrupt manoeuvres or when a target's intent is unknown.

## Accepted 3D direction

The accepted movement extension allows Navigation to contribute desired three-dimensional position or velocity to a shared motion plan, along with hazard assessment. Different hulls may author bounded vertical lanes, full 3D movement or scenario-specific constraints.

Orbital or strongly relative scenarios should build on the same identity, waypoint and reference-frame rules. They must not require a separate navigation command model for every scenario. Zero-setup and simple combat remain valid with a primarily planar chart.

Band C adds docking as a relative-motion operation: approach, request clearance, enter the authored docking envelope, dock and undock. Band D adds docked repair, replenishment and resource transfer. Band F adds loadout refit and campaign reactor rewards at authored starbases.

Band F's authored continuing-run foundation uses discrete warp jumps between systems rather than continuous interstellar flight. Warp fuel is distinct from local propellant and normally abundant; aid, damage, sabotage, unusual route costs or hazards may make it consequential. Band H adds the coarse planetary Orbital Mode described in [Release Bands G–I](../future/release-bands-g-i.md).

## Authoring and tuning

Chart range and filters, selectable tags, navigation AI policy, traffic routes, response clocks and movement constraints belong in hull, entity or scenario TOML as appropriate. Rhai coordinates narrative reactions and changes objectives; it should not become a second physics solver.

## Playtest questions

- Can Navigation set a destination and confirm that Helm sees the same intent?
- Do players understand the difference between selecting a chart contact and setting a waypoint?
- Are anchored waypoints visibly moving and safely cleared when their target disappears?
- Can Navigation and Helm discuss bearing and closing motion without frame confusion?
- Do civilian orders feel like negotiations with actors rather than remote control?
- Can simple scenarios ignore advanced 3D tools without violating the mechanic's assumptions?

## Canonical sources

- [Navigation design](../../../pasm/spec/design/navigation.yaml)
- [Navigation architecture](../../../pasm/spec/architecture/navigation.yaml)
- [Helm controls design](../../../pasm/spec/design/helm-controls.yaml)
- [Navigation console wiki](../../../wiki/entities/navigation-console.md)
