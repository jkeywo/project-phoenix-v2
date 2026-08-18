# Project Phoenix — World and Environmental Systems

| Field | Value |
|---|---|
| Document | GDD-WORLD-ENVIRONMENT |
| Status | Working draft; current systems and accepted future field direction are separated below |
| Owner | Unassigned |
| Last updated | 2026-08-18 |
| Scope | Spatial world, celestial bodies, terrain, regions, fields, traffic, infrastructure, scanning/evidence, and external operations |
| Authority | Player-facing systemic design. Entity/world TOML, runtime systems, validators, and PASM remain canonical. |

Phoenix worlds are bounded operational spaces whose objects continue to matter when the crew is not looking at them. The environment is not backdrop alone: geometry affects navigation and weapons, fields affect ships and observation, traffic follows routes, infrastructure has condition and capacity, and external operations consume time, range, power, teams, and position.

Related documents: [Entity Authoring](./entity-authoring.md), [Scenario Authoring](./scenario-authoring.md), [Station Experiences](./station-experiences.md), [Falling Skyway](../content/scenarios/falling-skyway.md), [Difficulty, Balance, and Playtesting](../foundation/difficulty-balance-playtesting.md), and [Thin Margin Setting](../foundation/thin-margin-setting.md).

Detailed mechanics: [Sensors and Epistemics](../mechanics/sensors-epistemics.md), [Navigation and Relative Motion](../mechanics/navigation-relative-motion.md), [Comms and Commitments](../mechanics/comms-commitments.md), and [External Operations](../mechanics/external-operations.md).

## Simulation principles

1. State has causes rather than appearing solely because a script needs a beat.
2. Several systems consume the same authoritative state.
3. Sensors reveal state rather than scenario text declaring a parallel truth.
4. Objects continue behaving when unattended.
5. Physical properties may occasionally permit an unexpected but legitimate solution.
6. Procedural generation creates coherent situations, not variety for its own sake.

Depth is earned when it gives another bridge officer a consequential decision within a mission. Phoenix does not simulate astronomical formation histories, planetary surfaces, walkable interiors, or a galaxy merely to claim scale.

## Spatial model

World TOML places entities with transforms and defines named anchors for objectives, routes, AI directives, operations, and scripted spawns. Ships move through continuous authoritative space. The current game presents a 3D world, while many established tactical calculations remain primarily planar; future vertical capability must be introduced as real shared simulation truth rather than presentation-only height.

Distance is operationally scaled. Ranges, speeds, body sizes, and route lengths must be internally coherent and readable on bridge displays, but do not promise real astronomical units. Celestial bodies establish place, collision terrain, silhouettes, light, and scenario relationships within that scale.

## Celestial bodies and terrain

Stars, planets, moons, stations, asteroids, and infrastructure use ordinary entity templates. Their visible extents and colliders should agree. Static bodies author `movable = false`; ships author `movable = true`, which affects hazard assessment and whether size-based avoidance rules may ignore them.

Celestial bodies are currently authored set pieces rather than simulated orbital systems. Future on-rails orbits and relative-motion operations are roadmap direction, not present universal behaviour. A scenario should not imply orbital timing that the authoritative positions do not support.

## Asteroids and fields

Asteroid fields produce deterministic populations from authored density and seeded spatial cells. The same seed and cell produce the same population. Destroyed asteroids return fresh if the player leaves and later re-enters the cell under the current lifecycle; they are environmental population, not persistent named campaign objects.

Asteroids provide collision hazard, tactical occlusion/terrain where supported, targetable objects, and visual motion reference. Density and body size should create navigational decisions without turning routine travel into unavoidable collision noise. Field overlap should be authored as one coherent density composition rather than competing spawners.

## Regions: current implementation

Regions are authored sphere, box, or torus volumes. The host tracks ship membership and resolves effects from that relation. Current effects include damage zones, slow zones, impulse blocking, radar dampening, comms jamming, sensor blindness, and nebula fog. Entry adds source-keyed modifiers/flags; exit clears that region’s contribution without erasing another source.

A region’s radar presentation comes from its authored shape or asteroid-field geometry plus `radar_appearance.region_colour`. Human and AI systems consume the same resulting modifiers and availability state. Scenario scripts may respond to flags/events caused by region interaction but should not manually apply a duplicate effect.

## Environmental fields: accepted direction

The accepted Band-B field design upgrades the region system in place rather than introducing a second volume model. A field adds analytic parameters for intensity, falloff, motion, growth, and a logical-tick envelope. Sampling is a pure function of field parameters, position, and tick; snapshots carry parameters, not per-position samples. Existing binary regions are the step-falloff special case.

Planned field consumers are radiation damage through the normal damage path; interference that degrades observation, comms, and the AI coordination bus symmetrically; and debris that creates seeded strike risk plus Helm/Navigation advisories. Falling Skyway’s current storm is intended to migrate without changing behaviour first. This is accepted design direction, not all current runtime capability.

## Traffic and routes

Civilian-capable entities follow authored routes composed of anchor legs, speeds, and optional holds. Route completion may loop or follow another authored policy. Live traffic holds its current leg and order independently; the route remains map data.

Supported orders such as hold, divert, and dock should alter real movement and compliance state. Civilian actors may be destroyed, delayed, stranded, sheltered, escorted, or towed. A script can acknowledge waypoint arrival or loss, but it should not teleport traffic through a problem the route/operation model claims the crew solved.

Traffic makes an area socially inhabited and creates shared work for Navigation, Helm, Sensors, Comms, and Captain. It should have enough persistence to create consequences without requiring a full economy simulation.

## Infrastructure

Infrastructure entities carry a structural condition track, named capacities, operational thresholds/flags, optional decay, hull-damage coupling, workforce, publication policy, and ordinary hull state. Hull answers whether weapons destroy the object; condition answers whether the surviving structure can still perform its job.

Thresholds create named facts such as transfer-capable or lift-capable, with hysteresis where authored. Capacities say how much of a named resource or service remains. Published condition/capacity becomes client-visible and enters dossiers; unpublished internal state remains authoritative but needs another legitimate observation path if players must act on it.

Workforce is separate from faction. A strike can stop or slow work without making a structure hostile. Disposition and strike state are world runtime facts; operation capability authoring decides what a stoppage does to each kind of work.

## External operations

The common operation spine supports stabilise, tow, escort, transfer, and field repair where the operating hull authors the capability. Starting work checks target eligibility, range, power, available teams, capacities, and other verb requirements. Operations advance as timed holds, may be slowed or interrupted by authored hazards/fire/work stoppage, and pay results into authoritative condition, capacity, or spatial state.

Operations should create bridge coordination:

- Sensors establishes the target’s state and whether work is needed.
- Navigation and Helm put the ship in the correct place and hold it there.
- Power funds the relevant group.
- Repair commits a team where required.
- Tactical and Shields protect a vulnerable hold.
- Comms and Captain negotiate permission, priority, or consequences.

An operation refusal or interruption must state its reason in player language. A completed animation without authoritative payment is not success; authoritative payment without visible acknowledgement is not adequate feedback.

## Scanning, observation, and evidence

Current scanning capability is authored on the observing hull through range bands, condition precision, capacity reporting, power requirements, and environmental degradation. It reads the target’s authoritative published or scannable structure state. Scenario evidence and dossiers combine identity, faction, comms standing, infrastructure state, commitments, and gathered evidence with provenance.

The accepted future epistemic model keys observations/evidence by observing ship. Confidence advances through discrete detected, rough, fair, and precise tiers, with deterministic uncertainty and interference caps. One scan axis chooses wide accumulation across subjects or focused rapid work on one subject. Evidence transfers between ships only through Comms with provenance. This model is planned direction and especially relevant to future multi-ship play.

## Dossiers and commitments

A dossier is a projection of what the crew knows about one subject, not an independent truth store. It combines current authorised inputs each tick. Evidence entries record provenance; commitments record promises and their open/kept/broken state. A scenario should let these systems constrain future dialogue and outcomes rather than use them only as collectible text.

## Environmental authoring questions

1. What physical or social system exists here, and who depends on it?
2. Which state changes without player attention, and what causes that change?
3. Which stations can observe it, at what quality, and which cannot?
4. What actions can change it, and what do they cost in time, position, power, teams, ammunition, or political capital?
5. How does the same state affect AI, traffic, combat, comms, and objectives?
6. Which failure is recoverable, which closes an option, and which ends the scenario?
7. Could an unexpected physical solution work without contradicting the fiction?

## Acceptance criteria

- World geometry, collider mobility, visible extents, radar form, and authored scale agree closely enough for navigation.
- Environmental effects resolve through shared modifier/damage/availability systems for humans and AI.
- Traffic continues to follow authoritative routes/orders when not observed and exposes compliance or failure.
- Infrastructure condition, thresholds, capacity, workforce, hull, and script text do not contradict one another.
- External operations validate capability and eligibility, expose progress/refusal/interruption, and pay authoritative results.
- Required knowledge has a legitimate sensor/comms/dossier path; scripts do not reveal a parallel unearned truth.
- Deterministic populations, fields, and scheduled state reproduce from the same authored inputs and seed.
- Scenario and station playtests show at least three station families responding to each major environmental pressure where that complexity is intended.

## Canonical sources

- `pasm/spec/design/simulation-differentiation.yaml` and `fields-epistemics.yaml`.
- `pasm/spec/architecture/world-files.yaml`, `power-modifiers-regions.yaml`, and `scenario-scripting.yaml`.
- `src/regions/`, `src/asteroids/`, `src/infrastructure/`, `src/operations/`, `src/civilian/`, and `src/science/`.
- `assets/worlds/falling_skyway.toml` and its infrastructure/civilian/region entity templates.
