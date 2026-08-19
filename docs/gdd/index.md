# Project Phoenix — Federated GDD Index and Coverage Gaps

| Field | Value |
|---|---|
| Document | GDD-INDEX |
| Status | Working draft |
| Owner | Unassigned |
| Last updated | 2026-08-19 |
| Scope | Navigation, document authority, current coverage, and recommended next documents |
| Authority | Orientation only. Linked code, assets, PASM, and project planning remain canonical for their domains. |

The Project Phoenix GDD is federated: short design documents explain player-facing intent and authoring contracts, while code and assets remain runtime truth, PASM remains design/architecture truth, and GitHub remains planning truth. The GDD should not become a stale duplicate schema or backlog.

## Current documents

| Document | Coverage |
|---|---|
| [Game Design Overview](./foundation/overview.md) | Vision, audience, pillars, experience promise, scope, accessibility direction, and success questions |
| [Game and Session Lifecycle](./foundation/game-lifecycle.md) | Enter game, connect/disconnect, lobby, scenario entry/end, replay, and exit |
| [Entity Authoring](./systems/entity-authoring.md) | Generic template/instance model, composition, common TOML surfaces, and validation |
| [Ships and Ship Systems](./systems/ships-and-systems.md) | Stations, ratings, fine systems, damage, power, AI control, and generic ship TOML |
| [Scenario Authoring](./systems/scenario-authoring.md) | World TOML, Rhai choreography, objectives, outcomes, pacing, and validation |
| [Alliance Ships](./content/ships/alliance-ships.md) | Current player hull ladder, station scaling, roles, and open fleet decisions |
| [Harrow Ships](./content/ships/harrow-ships.md) | Current House Harrow hull roles, fleet composition, doctrine, and open faction decisions |
| [Combat Test](./content/scenarios/combat-test.md) | Direct defence scenario, waves, hull scaling, outcomes, and playtest measures |
| [Falling Skyway](./content/scenarios/falling-skyway.md) | Operational crisis, acts, actors, systems, allocation choice, and consequence dimensions |
| [Campaign Continuity and Persistence](./foundation/campaign-continuity.md) | Episode loop, durable fact vocabulary, save/resume boundary, debrief, failure, branching, refit, identity, and compatibility |
| [Station Experiences](./systems/station-experiences.md) | Captain, Helm, Tactical, Sensors, Navigation, Comms, Shields, Power, Repair, and shared viewscreen experience |
| [Thin Margin Setting](./foundation/thin-margin-setting.md) | Synthesised tone, institutions, technology, dialogue, visual/audio language, and open canon decisions |
| [Onboarding and Accessibility](./foundation/onboarding-accessibility.md) | First-session flow, tutorials, manuals, handover, facilitation, sensory/input alternatives, and testing |
| [AI and Backfill](./systems/ai-and-backfill.md) | Control-source symmetry, station automation, NPC doctrine, information parity, transparency, and handoff |
| [Native and Network Foundation](./systems/native-network-foundation.md) | Band A2 join codes, RTC transport, host recovery, Windows native bridge, Ultralight, displays, touch and media devices |
| [Difficulty and Balance](./foundation/difficulty-balance-playtesting.md) | Balance dimensions, evidence ladder, crew workload, accessibility boundary, metrics, and tuning process |
| [World and Environmental Systems](./systems/world-environmental-systems.md) | Space, terrain, asteroids, regions/fields, traffic, infrastructure, operations, observation, and evidence |
| [Future Modes](./future/future-modes.md) | Multi-ship, Patrol Mode, customisation, crew assignments, native/physical bridges, show control, and GM tools |
| [Release Bands G–I](./future/release-bands-g-i.md) | Authored War, Patrol/War PCG, Sandbox, supporting features, technical work, polish and distributed accessibility |
| [Movement and Helm](./mechanics/movement.md) | Motion authority, axes, impulse, boost, coordination, damage and accepted 3D direction |
| [Targeting and Weapons](./mechanics/targeting-weapons.md) | Combat lock, phasers, blasters, torpedoes, readiness and arc-bearing coordination |
| [Damage, Diagnosis and Repair](./mechanics/damage-repair.md) | Damage routing, information boundaries, repair teams, priorities and defeat |
| [Power and Resource Network](./mechanics/power-resource-network.md) | Current allocation, battery exhaustion and accepted supply, demand and heat model |
| [Shields](./mechanics/shields.md) | Authored arcs, focus, damage routing, collapse, recovery and AI operation |
| [Sensors and Epistemics](./mechanics/sensors-epistemics.md) | Radar, science targeting, scans, evidence, dossiers and accepted confidence model |
| [Navigation and Relative Motion](./mechanics/navigation-relative-motion.md) | Chart, waypoints, Helm boundary, traffic orders, reference frames and 3D direction |
| [Comms and Commitments](./mechanics/comms-commitments.md) | Dialogue, reachability, response authority, promises, settlement and continuity |
| [External Operations](./mechanics/external-operations.md) | Stabilise, tow, escort, transfer and field-repair holds across ship systems |
| [Command and Crew Control](./mechanics/command-and-crew-control.md) | Band A2 Command stances, spectators, AFK, human-seeking stations and shared Hero Bar |
| [Duty Teams, Officers and Operations](./mechanics/duty-teams-and-operations.md) | Unscheduled workforce, personnel, Away Missions, boarding, Medical and Operations systems |
| [Planned but Not Scheduled](./future/planned-not-scheduled.md) | Accepted future clusters outside Bands A2–I and their dependency boundaries |

## What these documents now cover

Together, the set defines the game’s high-level identity, complete session and campaign journeys, principal authoring layers, station experiences, detailed interconnected mechanics, both Band A2 Bridge Foundation tracks, B–I direction, accepted unscheduled dependency clusters, shared-setting relationship, onboarding/accessibility target, balance framework, world systems, current ship families and both current scenarios. Each page distinguishes shipped behavior from accepted direction and unscheduled extension; detailed schemas and tunables remain in their canonical sources.

## Remaining gaps

### High priority: ratified setting canon and content guidance

[Thin Margin Setting](./foundation/thin-margin-setting.md) now establishes Phoenix as an earlier-era, potentially different-region use of *The Neutral Zone* setting, with the Alliance, Imperium/later Dynasty, principal Houses, Honour, Mandate, and Singularity carried across at high level. It still needs human decisions about exact chronology, Phoenix’s region, Alliance and Imperial institutions, House Harrow’s local command, Havelock, travel, ranks, naming cultures, recurring characters, and original visual/music references. Once ratified, faction and dialogue style sheets should turn those decisions into repeatable authoring guidance.

### High priority: first campaign consumer and campaign shell

[Campaign Continuity and Persistence](./foundation/campaign-continuity.md) defines the intended boundaries, and the runtime already projects Falling Skyway’s facts, but no follow-on episode consumes them. Remaining design and implementation work includes choosing that episode, binding prior structures and standing into a new world, defining the campaign-file/checkpoint format, creating campaign selection and debrief surfaces, and testing branch, migration, export, and recovery behaviour.

### High priority: mechanic ratification and scenario proof

The detailed mechanic pages now translate the current implementation and accepted PASM direction into human-facing design. Their remaining work is empirical: ratify the stated experience goals, information boundaries and coordination loops through Combat Test and Falling Skyway playtests, then amend PASM or create issues where observed play disagrees with the intended mechanic. Detailed schemas and implementation contracts remain in PASM rather than being duplicated here.

### Medium priority: shared presentation specifications

The setting, station, and accessibility documents establish principles, but final production specifications remain for the viewscreen/radar information hierarchy, console visual system, alerts, camera/cinematic grammar, audio mix and warning priority, reduced-motion variants, and room-distance readability. These should be worked hull by hull alongside the roadmap’s visual/UX release slices.

### Medium priority: future content

The GDD describes current scenarios and future modes but does not yet contain content documents for Borrowed Sun, Narrow Season, The Broken Tithe, The Silent Corridor, the hand-authored first patrol, or later campaign episodes. Add each only when its premise and supporting systemic slice move into active design.

### Medium priority: production-ready accessibility conformance

[Onboarding and Accessibility](./foundation/onboarding-accessibility.md) states requirements but is not an audit claiming conformance. Remaining work includes a supported-device/browser matrix, verified keyboard and screen-reader flows, contrast and reflow results, reduced-motion implementation, alternative Helm/Tactical inputs, captions if voice is introduced, and testing with disabled players.

### Product decisions deliberately deferred or external to the design set

- Commercial model, release platforms, and minimum supported devices.
- Account, privacy, moderation, analytics, and online-service policy.
- Shipping budget, staffing, localisation markets, legal ratings, and live operations.
- Final performance targets and browser/device support matrix.

These should remain visibly deferred rather than filled with speculative recommendations until the project needs them.

## Recommended next sequence

1. Break the two Band A2 Bridge Foundations PRDs—[Crew Control](./mechanics/command-and-crew-control.md) and [Native and Network](./systems/native-network-foundation.md)—into implementation work before Band B begins.
2. Review and ratify the Phoenix-era setting relationship, especially the Imperium name, House identities, chronology, and which facts are inherited versus local.
3. Choose the first episode that consumes Falling Skyway’s projection and define its input bindings before expanding the full campaign shell.
4. Run Combat Test and Falling Skyway playtests using the station, onboarding, AI, accessibility, balance, debrief, and continuity questions; ratify crew counts, pacing, workload, and measurable targets.
5. Turn the accessibility target into an implementation/audit matrix for the supported browser route.
6. Review the detailed mechanic pages against playtest evidence and use their diffs to drive PASM amendments or implementation issues.
7. Write each future scenario, mode or [unscheduled cluster](./future/planned-not-scheduled.md) as it enters active design, preserving its dependency and status boundaries.

## Maintenance rule

Update a GDD page when player-facing intent or a content summary changes. Update PASM when design/architecture decisions change, assets when authored truth changes, code when runtime truth changes, and GitHub when planned work changes. If a GDD table repeats live balance values, label it as a snapshot and link to the authoritative asset.
