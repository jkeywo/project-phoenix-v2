# Project Phoenix — Game Design Overview

| Field | Value |
|---|---|
| Document | GDD-OVERVIEW |
| Status | Working draft |
| Owner | Unassigned |
| Last updated | 2026-08-18 |
| Scope | Vision and high-level game experience |
| Authority | Orientation only. Code and assets are runtime truth; PASM is design and architecture truth. |

This is the front door to a federated Game Design Document. It describes the identity of Project Phoenix and links outward to canonical sources; it does not duplicate detailed mechanics, balance values, content catalogues, or technical specifications.

Related documents: [Game and Session Lifecycle](./game-lifecycle.md), [Campaign Continuity and Persistence](./campaign-continuity.md), [Station Experiences](../systems/station-experiences.md), [Thin Margin Setting](./thin-margin-setting.md), [Onboarding and Accessibility](./onboarding-accessibility.md), [AI and Backfill](../systems/ai-and-backfill.md), [Difficulty and Balance](./difficulty-balance-playtesting.md), [World and Environmental Systems](../systems/world-environmental-systems.md), and [Future Modes](../future/future-modes.md).

## High-level concept

**Project Phoenix is a cooperative spaceship bridge simulator for browsers in which a group gathers around a shared viewscreen, joins from their phones, and operates one ship through specialised stations.** Each player sees and controls only part of the situation, so success comes from communicating, setting priorities, and combining decisions into a coherent response.

| Attribute | Overview |
|---|---|
| Genre | Cooperative spaceship bridge simulator; scenario-driven command and operations game |
| Platforms | Zero-setup browser host and browser-based phone consoles by default; optional native and physical-bridge integrations may extend the same game |
| Session model | One host-authoritative simulation with local or remote players connected as crew |
| Possible players | 0 through the selected ship's authored maximum; unclaimed stations operate through Backfill AI |
| Recommended players | Authored per scenario and, where needed, per ship. Combat Test: 2–4. Falling Skyway: to be determined. |
| Typical length | Scenario-dependent. The design boundary is a 30–90 minute mission; the current Combat Test is shorter and Falling Skyway targets 30–60 minutes. |
| Current content | Combat Test, an escalating defence scenario; Falling Skyway, an operational crisis with traffic, infrastructure, negotiation, evidence, deadlines, and consequences |
| Long-term structure | A growing catalogue of authored missions, followed by campaign continuity and a generated Patrol Mode |
| Commercial model and release targets | Deliberately deferred |

## Player fantasy

**Be the crew of a working starship under pressure.** Read an unfolding situation through imperfect, role-specific information; rely on other officers; make consequential calls; and feel the ship respond as one connected machine.

The fantasy is not that every player is an all-seeing captain or an individual action hero. Each officer owns a meaningful part of the ship, while the shared viewscreen and crew conversation turn those partial responsibilities into a collective command experience.

## Experience promise

A Phoenix session should let a group move quickly from “we want to play a bridge game” to making decisions together:

1. Open the host in a browser and choose a scenario and ship.
2. Scan a QR code; join without installing an app or configuring a LAN.
3. Claim a station and choose how much of its workload to operate directly.
4. Read the shared situation through the viewscreen and specialised consoles.
5. Communicate observations, intent, priorities, and requests across the bridge.
6. Issue commands into one authoritative simulation and observe their shared consequences.
7. Adapt as threats, failures, deadlines, discoveries, and competing demands develop.
8. Reach a scenario outcome, review what happened, and return together for another mission.

## Design pillars

### 1. The bridge is a conversation

No single station should possess or control everything that matters. Information and authority are distributed so that officers have reasons to report, recommend, request, confirm, and negotiate. Coordination between people is the primary play space; the consoles give that conversation concrete stakes.

### 2. One situation, many consequences

Ship systems, actors, hazards, and scenario logic consume the same authoritative state. State has causes, objects continue to behave when unattended, and actions should propagate across more than one officer's concerns. Phoenix aims for depth within a bounded operational situation rather than breadth across a simulated galaxy.

### 3. Scenarios frame the problem; crews shape the response

Phoenix supports both direct scenarios with clear objectives, such as Combat Test, and more open operational crises. A simple scenario is not a lesser or forbidden form of play. When a scenario presents competing pressures, evidence, or incompatible demands, it should frame the problem without requiring one designer-approved response. Physical properties and prior choices may create legitimate, occasionally unexpected answers.

### 4. Every crew size gets a real ship

Stations are authored bundles of systems, not fixed assumptions about the number of players. Station ratings let a holder choose direct, simplified, or automated control, while Backfill AI keeps vacant systems operating through the same command paths as humans. A smaller crew should face a different workload, not a disabled simulation.

### 5. Zero setup first; deeper setup by choice

The default Phoenix experience starts with a browser host, QR-first joining, and installation-free phone consoles. That zero-setup route remains the most important and must stay complete. Optional layers may add native hosts and clients, multi-ship networking, custom console hardware, and event systems such as lighting or smoke for groups building a fuller bridge. These extensions deepen the setting and setup by choice; none may become a prerequisite for ordinary play.

## Core play loop

```text
Choose scenario and ship
          ↓
Join, claim stations, set workload, ready up
          ↓
Observe the world and diagnose the situation
          ↓
Share information and agree priorities
          ↓
Operate ship systems and commit to decisions
          ↓
Authoritative simulation produces consequences
          ↓
Reassess threats, objectives, resources, and promises
          └─────────────── back to observe and diagnose
          ↓
Scenario outcome and debrief
          ↓
Return to selection for the next mission
```

Within that loop, individual stations have shorter loops—acquire information, choose or recommend an action, execute it, and report the result. Detailed station mechanics belong in their own system specifications rather than in this overview.

## What makes Phoenix distinctive

- **Bring-your-own-phone bridge play:** the shared 3D viewscreen and personal consoles require no installed client application.
- **Elastic division of labour:** the crew can redistribute cognitive load at runtime through station ratings and human/AI hand-offs.
- **Symmetric human and AI operation:** both operate the same ship systems through the same admitted commands; automation is part of the game model, not a separate simplified simulation.
- **Operational science fiction:** combat, navigation, damage, power, sensors, comms, traffic, infrastructure, evidence, and promises can meet in the same mission state.
- **Authored crises built from reusable systems:** scenarios compose world capabilities rather than replacing the bridge with bespoke minigames.
- **Deterministic, inspectable outcomes:** fixed logical ticks, authored data, scenario scripts, headless simulation, and after-action evidence support repeatable balancing and meaningful debriefs.
- **A bridge that can become physical:** the default browser experience can grow into native displays, custom controls, multiple ships, and venue effects without splitting into a different game.

## Content direction

Phoenix currently demonstrates two complementary forms of play:

- **Combat Test** is a short, escalating defence of Starbase Alpha. Timed enemy waves create accumulating pressure and exercise the complete combat stack.
- **Falling Skyway** is the first *Thin Margin* mission. The crew enters a place already in motion and must handle damaged infrastructure, civilian traffic, a labour dispute, corporate security, a radiation storm, incomplete evidence, limited transfer capacity, and promises that can be kept or broken.

The accepted roadmap expands this into a sequence of Thin Margin missions. Each release adds a bounded physical or operational foundation, one mission that uses it, and another playable hull. Later work adds campaign continuity, Game Master facilitation, and eventually Patrol Mode: coherent generated assignments rather than procedural variety for its own sake.

## Tone and fiction

Phoenix takes its broad tonal lead from the *Star Trek* era represented by *The Next Generation*, *Deep Space Nine*, and *Voyager*: an ensemble of capable professionals using knowledge, technology, diplomacy, and courage to solve difficult problems. The setting can be hopeful without being comfortable. Institutions have interests, evidence may be incomplete, promises matter, and there may be no outcome that protects everyone.

Missions may range from clean episodic premises to politically or morally complicated crises. Combat is valid and sometimes central, but it is one instrument among navigation, engineering, sensors, communication, negotiation, rescue, and restraint. The bridge should feel competent and purposeful rather than grimdark, cynical, or dependent on characters behaving foolishly to create drama.

Presentation should support the ensemble fantasy: the viewscreen supplies shared drama, consoles supply role-specific facts and controls, and the crew's spoken exchange connects them. Interface fiction should remain readable enough for players to act without needing to decode decorative technobabble.

## Setup and bridge scale

Zero-setup browser play is the product baseline. Work has begun on native host delivery; later extensions may include native clients, P2P multi-ship sessions, custom peripherals, and event-output integrations for lighting, smoke, sound, or other venue systems. A group may therefore play with a television and phones, build one physical console, or construct a complete event bridge.

All such integrations are optional adapters around the same authoritative game and station model. Scenario rules and ordinary controls must remain playable through the baseline browser route. PASM currently records external show hardware as not planned while requiring the cue path not to preclude future integration; this overview treats it as a permitted extension direction, not committed roadmap scope.

## Scope boundaries and non-goals

Phoenix deliberately does not pursue:

- walkable ship interiors or avatar-scale exploration;
- routine simulation of bodily crew needs;
- moment-to-moment simulation of every member of a large NPC crew as an autonomous person;
- galaxy-scale traversal or billions of star systems;
- valve-level or component-by-component hardware manipulation;
- explorable planetary surfaces;
- astronomical formation simulation;
- isolated station minigames whose state does not matter to the rest of the ship;
- a setup flow that requires every player to install software or configure a local network.

A detail earns simulation depth when another bridge officer can make a consequential decision about it within the span of a mission. A scenario may still mention any of the above as fiction without turning it into a permanent game system.

This boundary does not prohibit a trait-bearing crew roster. Named or abstract crew may be assigned to ship systems or away missions, with traits influencing results in the style of a duty-officer system, without individually simulating their continuous lives and actions. The earlier design for that feature was not found in the current repository or its visible history during this pass and should be linked here when recovered.

## Target audience

### Primary audience

- Social groups who enjoy cooperative science-fiction, tabletop, role-playing, escape-room, or bridge-command experiences and want verbal coordination to matter more than reflex skill.
- Players who want distinct responsibilities within a team, including people who prefer analysis, communication, planning, or technical operation to direct combat.
- Clubs, conventions, game nights, and event facilitators who need a group experience that can begin with ordinary devices and little technical preparation.

### Secondary audience

- Starship-simulation players who enjoy learning interconnected systems and improving crew performance across repeated missions.
- Solo players and undersized crews who want to operate alongside configurable Backfill AI.
- Hobbyists, makers, streamers, and venue operators who may extend Phoenix with native displays, physical consoles, custom peripherals, or show-control integrations.
- Scenario and mod authors who want to build operational science-fiction crises from reusable simulation systems.

This audience definition is proposed positioning for review, not a restriction that every scenario must serve every audience equally.

## Accessibility targets

The proposed accessibility goal is that joining, understanding a station, communicating essential state, and taking every time-critical action should not depend on one sensory channel, precise touch control, rapid repeated input, or colour recognition alone. The conventional web interface should use [WCAG 2.2](https://www.w3.org/TR/WCAG22/) as its conformance reference, while game-specific interaction work should use the [Xbox Accessibility Guidelines](https://learn.microsoft.com/en-us/xbox/accessibility/guidelines) as design and test guidance rather than as a legal-compliance checklist.

- The lobby, settings, help, and other conventional interface surfaces should target WCAG 2.2 AA where applicable.
- Critical state should combine text or symbols with colour; critical audio cues should have visual equivalents; spoken or recorded dialogue should support captions and speaker identification.
- Text should remain legible on supported phones under browser text enlargement and zoom, with high contrast and layouts that reflow rather than hide controls.
- Time-critical controls should use generous targets and provide alternatives to precision dragging, holding, or rapid repeated input where those gestures are not intrinsic to the decision.
- Motion, screen shake, flashing, volume, and haptics should be independently reducible or disabled where technically possible.
- Tutorials, contextual help, clear error recovery, and the station-rating ladder should let players reduce cognitive load without removing them from the crew.
- Accessibility preferences should normally be local presentation and input choices. Any option that changes authoritative timing or scenario rules should be an explicit host or scenario setting shared by the group.
- The zero-setup browser route is the accessibility baseline. Physical peripherals may add alternatives but must not become the only way to perform an action.

These are initial targets, not a claim that the current build already meets them. Each station specification should translate them into testable interaction requirements.

## Player counts

Every scenario is technically possible with zero human players through Backfill AI and supports humans up to the selected ship's authored station maximum. The recommended range is a content property: it describes where the scenario's workload, communication, and pacing are expected to work best, rather than imposing an engine-wide limit.

| Scenario | Possible | Recommended | Status |
|---|---:|---:|---|
| Combat Test | 0–selected ship maximum | 2–4 | Working recommendation |
| Falling Skyway | 0–selected ship maximum | To be determined by playtest | Open |

Future scenario briefs should state a recommended range, the hulls used to establish it, and any material change in experience at the low and high ends.

## Experience success measures

The first playtest rounds should establish baselines before setting numerical pass thresholds. Candidate measures are:

- **Setup friction:** elapsed time from opening the host to the first meaningful in-scenario action; number and type of facilitator interventions; connection or station-claim failures.
- **Role comprehension:** whether each player can explain their responsibility, the information they owned, and at least one consequential contribution after the session.
- **Crew communication:** whether play naturally produces reports, requests, recommendations, confirmations, and cross-station decisions rather than parallel solo play.
- **Causal clarity:** whether the crew can explain why the major outcome occurred, including at least one consequence that crossed system or station boundaries.
- **Workload fit:** periods of overload, avoidable idleness, or unwanted automation at each player count and station rating.
- **Recovery:** whether disconnection, station hand-off, human/AI transition, mistakes, and partial failure can be understood and recovered from without restarting.
- **Scenario fit:** whether direct scenarios remain satisfying without artificial branching, and whether open crises support multiple defensible responses without becoming vague.
- **Memorability and replay intent:** whether the group can name a moment worth retelling and expresses interest in another scenario, hull, role, or approach.

Core playtest questions:

1. What did you know only because another officer told you?
2. Which decision felt most consequential, and what changed because of it?
3. When the game surprised you, could you work out the cause?
4. Did your station ever leave you with too little, too much, or the wrong kind of work?
5. Did Backfill AI feel helpful, legible, and under crew control?
6. Was anything essential difficult to see, hear, understand, or physically operate?
7. Did setup, connection, or device handling delay the part you considered the game?
8. Would you play another mission, and what would you change next time?

## Current implementation versus intended product

The full development catalogue already contains the bridge simulation, the major station families, human/AI authority hand-off, scenario scripting, Combat Test, and Falling Skyway. The public demo is intentionally narrower: Combat Test with the Alliance Destroyer. The roadmap broadens content and delivery in release bands rather than treating every planned capability as part of the current player promise.

Detailed documents should keep these labels distinct:

- **Implemented:** observable in the current code and authored assets.
- **Accepted:** an intended design recorded in PASM but not necessarily shipped.
- **Proposed:** under discussion and not yet an approved product promise.
- **Inferred:** overview wording synthesised from implementation and accepted design; it should be ratified before being treated as a pillar or marketing claim.

## Open vision decisions

The following remain deliberately open or require validation:

- Falling Skyway's recommended player range and the method for recording per-hull recommendations;
- detailed art, audio, fiction, and interface principles beneath the broad Star Trek-era tonal direction;
- station-by-station accessibility requirements and the supported-device test matrix;
- numerical success thresholds after baseline playtests;
- commercial model, release platforms, and minimum supported devices, which are deliberately deferred;
- roadmap commitment and interface standards for optional native clients, peripherals, and event systems;
- the location and current status of the earlier trait-based crew assignment design.

The pillar wording is sufficient as internal, LLM-facing guidance. It can be rewritten for a human-facing or marketing document without changing the underlying principles. Proposed audience, accessibility, and success-measure language should be ratified or revised before this overview becomes the canonical vision layer.

## Canonical sources

- [Project overview](../../../wiki/concepts/project-overview.md) — current orientation
- [Simulation differentiation](../../../pasm/spec/design/simulation-differentiation.yaml) — accepted scope guard and non-goals
- [Game flow](../../../pasm/spec/design/game-flow.yaml) — selection, station claim, readiness, and round return
- [Station ratings](../../../pasm/spec/design/station-ratings.yaml) — workload and automation ownership
- [Console complexity](../../../pasm/spec/design/console-complexity.yaml) — authored control-depth ladder
- [P2P design deltas](../../../pasm/spec/design/p2p-design-deltas.yaml) — accepted multi-ship direction
- [Delivery roadmap](../../../pasm/spec/roadmap/phoenix-delivery-roadmap.yaml) — demo scope and mission sequence
- [Game Master milestones](../../../pasm/spec/roadmap/gm-console-milestones.yaml) — in-sim cues and the current boundary around external show hardware
- [Combat Test](../../../assets/worlds/combat_test.toml) and [Falling Skyway](../../../assets/worlds/falling_skyway.toml) — current authored scenario truth
- [Domain vocabulary](../../../CONTEXT.md) — canonical terms
