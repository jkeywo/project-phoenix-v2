# Project Phoenix — Future Modes and Optional Bridge Extensions

| Field | Value |
|---|---|
| Document | GDD-FUTURE-MODES |
| Status | Directional design; most sections are roadmap or permitted extension, not committed current functionality |
| Owner | Unassigned |
| Last updated | 2026-08-18 |
| Scope | Multi-ship, generated Patrol Mode, ship customisation, crew assignments, native/physical bridges, event outputs, and GM tools |
| Authority | Product/design direction only. PASM roadmap and specific future PRDs govern implementation commitments. |

Phoenix may scale from a browser, television, and phones to multi-ship sessions and purpose-built bridge venues. Every extension must preserve the complete zero-setup route and the same authoritative station/system model. Optional depth should add ways to participate, author, and present the game without splitting it into incompatible “real” and “casual” versions.

Related documents: [Game Design Overview](../foundation/overview.md), [Game and Session Lifecycle](../foundation/game-lifecycle.md), [Campaign Continuity and Persistence](../foundation/campaign-continuity.md), [Station Experiences](../systems/station-experiences.md), [Onboarding, Tutorials, and Accessibility](../foundation/onboarding-accessibility.md), [AI and Backfill](../systems/ai-and-backfill.md), [Scenario Authoring](../systems/scenario-authoring.md), and [Difficulty, Balance, and Playtesting](../foundation/difficulty-balance-playtesting.md).

## Non-negotiable baseline

- One browser host/shared display plus pure-web phone clients remains a complete supported experience.
- No native binary, local-network configuration, account, custom peripheral, GM, or venue effect becomes required for ordinary scenarios.
- A scenario’s rules and all time-critical station actions remain accessible through standard browser controls.
- Extensions adapt the same authoritative commands and state; they do not create a separate simulation authority.
- Failure or absence of an optional adapter degrades gracefully to browser presentation/input.

## Status overview

| Direction | Current foundation | Status in this GDD |
|---|---|---|
| Native delivery | Native `phoenix-host` can serve a built bundle/catalogue/version pin; simulation authority remains browser/headless. | Started foundation; native client/simulation hosting not implied |
| Multi-ship | Per-ship state, AI ownership, command logs, deterministic fixed ticks, and P2P design work exist. | Planned roadmap direction |
| Patrol Mode | Scenario systems, seeded world content, campaign projection, and future field/orbit foundations. | Post-banded future direction |
| Ship customisation | Entity fragments/composition and accepted supply/demand design. | Accepted staged roadmap, not current player feature |
| Crew assignments | Earlier Star Trek Online-inspired concept reported by the designer; no current canonical document recovered. | Permitted concept needing reconstruction |
| Physical bridge | Web consoles, optional native delivery, stable commands/state. | Permitted extension, not current planned core requirement |
| Lighting/smoke/show control | Alert and presentation events could supply cues. | Permitted future adapter; PASM currently does not commit show hardware |
| GM tools | Debug/host surfaces, scenario effect vocabulary, snapshots, activity/balance events. | Planned staged roadmap |

## Multi-ship play

### Experience

Several independently crewed ships share one scenario. Each bridge has incomplete local knowledge, its own stations and Backfill, and reasons to communicate with other ships. Multi-ship should create command, trust, formation, rescue, information-sharing, and divided-objective problems rather than simply add more guns to one encounter.

### Design requirements

- Each ship remains an authority-scoped unit with its own systems, control sources, observations, evidence, objectives where appropriate, and failure state.
- Cross-ship information travels through explicit channels such as Comms, shared orders, or authored sensor/evidence transfer. A player does not automatically see another bridge’s private radar or dossier.
- Scenarios state possible/recommended ships and crew per hull, not only total players.
- Command hierarchy is authored/social unless a scenario creates formal fleet authority; one Captain does not acquire hidden control over another ship.
- A ship’s destruction, disconnect, or full AI takeover has a scenario-defined continuation path.
- Deterministic P2P/lockstep work cannot weaken current same-tick local-host semantics without an explicit negotiated command-delay design.

### Scenario opportunities

Escort and protected transit, split investigation, pincer or screen tactics, mutual repair/tow, asymmetric ships, contested evidence, simultaneous infrastructure work, and one bridge carrying information another needs. Scenarios should avoid long periods where one ship waits for the other to complete private play.

## Patrol Mode

### Experience

Patrol Mode is a continuing series of generated assignments. A seeded system generator provides the physical context—star, bodies, orbits, hazards, stations, and traffic—while Thin Margin situation generation supplies actors, needs, infrastructure, evidence, commitments, and cascades. The goal is coherent episodes, not a catalogue of random planets.

### Authored and generated responsibility

Authors define grammars, constraints, actor types, system relationships, failure patterns, tone, pacing bands, and outcome projections. Generation selects and binds compatible pieces from one seed, then validates that objectives, knowledge paths, operations, and terminal states remain coherent. Hand-authored scenarios remain the quality reference.

A hand-authored first patrol should teach exploration, database/library use, mission logging, wide/focused observation, and campaign handoff before procedural assignments open. Repetition controls should track recently used structures, actor conflicts, hazards, and resolution patterns, not only names.

### Boundaries

Patrol Mode does not generate a galaxy-scale continuous universe or simulate every system between missions. It produces bounded playable situations with persistent facts. Generated content must use the same world/entity/Rhai or successor authoring contracts and pass equivalent validation.

## Ship customisation

### Principle

Customise, do not simply upgrade. The crew chooses what systems a ship carries, and every option should be a net-neutral sidegrade with a visible operational cost. Stronger output costs power demand, heat, geometry, ammunition, mass/role constraints, or another authored capacity rather than consuming an abstract lobby point alone.

### Staged direction

The accepted MVP follows base-fleet balance closure and uses existing fragment composition to swap variants on one hull across two axes: a reactor profile and one Tactical combat choice. No new mount schema is required for that tracer. Later work adds typed and sized mounts, optional/empty slots, fleet-wide variants, and starbase refit.

The accepted future power model makes every component draw energy, retains ship-wide battery then hard exhaustion lock, and adds per-system heat/overpower. The reactor trades generation, battery, recovery, and dissipation. This future model is driven by customisation needs and must not be read back into current Power behaviour.

### Authority and campaign

The player holding a station in the lobby owns choices for that station’s loadout section; an unclaimed station uses the hull’s authored default. The composed loadout freezes at spawn into deterministic content. Mid-mission refit occurs only at an authored starbase path. Campaign scenarios may grant a reactor improvement; fixed skirmish balance remains sidegrade-only through authoring rather than a hidden mode switch.

## Crew roster and assignments

The permitted concept is a named or abstract crew roster inspired by Star Trek Online duty officers. Crew entries have traits and may be assigned to ship systems, projects, or away missions. Their traits modify outcome, speed, risk, information, or recovery through explicit authored rules.

This is not a persistent simulation of hundreds of individual lives. Crew do not need continuous location, hunger, sleep, conversation, or autonomous schedules. An assignment is a bounded decision with a result and availability cost. Named crew can become characters through authored events and consequences without turning Phoenix into an avatar-management game.

Open decisions include roster size, trait vocabulary, injury/death, player ownership, away-mission resolution, relationship to repair teams/workforces, campaign persistence, duplication, and whether assignments occur only between missions or can be changed during a scenario.

## Native hosts and clients

The current native host is a delivery server: it serves a built client bundle, manifest, catalogue, and version stamp. It does not replace the authoritative browser/headless simulation or PeerJS signalling. Future native simulation hosting or clients must remain protocol-compatible with the web route and justify themselves through reliability, deployment, peripheral access, performance, or venue operation.

A native client should be an optional shell around the same console actions and state. It must not gain exclusive controls, higher simulation authority, or content incompatible with a browser participant.

## Physical consoles and peripherals

Custom panels may map buttons, encoders, sliders, touchscreens, lamps, and displays to station commands and state. A bridge may build one Helm console or a complete room. Hardware profiles should bind semantic actions such as `set thrust`, `select shield arc`, or `raise red alert`, not DOM coordinates or private component internals.

Required properties:

- hot-plug and reconnect without changing player identity or system ownership;
- calibration, clear neutral state, and conflict handling when web and hardware inputs coexist;
- visible browser fallback for every command;
- no command rate or precision advantage that makes ordinary clients non-viable;
- profile portability and per-station test mode;
- safe handling of stuck switches, noisy axes, and unavailable outputs.

## Lighting, smoke, sound, and show control

Venue systems may subscribe to semantic cues such as lobby ready, countdown, scenario start, Red Alert, impact, shield failure, brownout, critical hull, comms hail, victory, and defeat. Cues enhance atmosphere but never carry unique information.

The adapter should receive structured state/events and own venue-specific timing, device protocols, and safety. The game should not directly toggle arbitrary mains-powered equipment. Smoke/haze requires local safety procedures, ventilation, venue permission, emergency stop, and content warnings. Flash and strobe outputs default off and respect accessibility settings. Loss of the show controller must not affect simulation.

## Game Master and host tools

### Experience

A GM supports pacing, performs characters, directs scenario systems, facilitates access, and recovers a live event. The GM is not required for ordinary play and does not replace authored automation.

### Planned progression

The roadmap stages GM capability from a session role with omniscient map, activity feed, and pause; through mission/effect directing and a comms performance studio; into facilitation, safety, checkpoints/undo, information control, in-sim presentation, campaign persistence, and pacing heuristics.

### Authority and safety

GM commands require a distinct command tier and audit trail. The interface distinguishes reversible presentation from authoritative world changes, previews affected entities where practical, and confirms destructive/irreversible actions. Pause state is visible to all bridges. Checkpoint restore or undo uses supported snapshot boundaries rather than ad hoc reversal.

The public/demo build may deliberately omit debug, cheat, client pause, or mod-upload routes even when a local host build offers them. GM scope must be an explicit build/session contract, not a hidden keyboard shortcut.

## Cross-mode compatibility

- A customised ship still exposes ordinary stations, ratings, manuals, Backfill, and validation.
- A physical console can join a single- or multi-ship session through the same action vocabulary.
- Patrol assignments can be played in browsers without GM or peripherals.
- A GM can facilitate accessibility but cannot make an inaccessible critical action acceptable by performing it for the player.
- Multi-ship observations and campaign facts preserve per-ship provenance.
- Native delivery, mod packs, and generated content bind to content/version digests for replay and resume safety.

## Sequencing and proof

Future scope should land as tracer bullets that prove the cross-cutting contract before breadth. Multi-ship proves two real bridges, separate knowledge, reconnect, and one shared objective. Customisation proves one hull and two balanced axes. Physical integration proves one semantic input and one output cue with browser fallback. GM proves one authenticated role, one observable effect, pause, and audit. Patrol proves one hand-authored first patrol before generation.

## Acceptance criteria

- The zero-setup browser path remains complete and receives the same scenario/system authority.
- Every extension declares current, accepted, planned, or merely permitted status.
- Optional adapters fail without corrupting or pausing the authoritative simulation unless the host explicitly chooses that policy.
- Multi-ship and Patrol preserve bounded situations, information provenance, and crew communication.
- Customisation choices are legible sidegrades and are covered by deterministic balance evidence before catalogue breadth.
- Crew assignments remain bounded trait-based decisions rather than continuous NPC-person simulation.
- Hardware and show control use semantic commands/cues, browser fallback, accessibility equivalents, and venue safety boundaries.
- GM actions are scoped, visible, auditable, and recoverable in proportion to impact.

## Canonical sources

- `pasm/spec/roadmap/phoenix-delivery-roadmap.yaml` and `gm-console-milestones.yaml`.
- `pasm/spec/design/ship-customisation.yaml`, `fields-epistemics.yaml`, and P2P design slices.
- `pasm/spec/architecture/native-delivery.yaml`, command-log/snapshot architecture, and world campaign projection.
- [Game Design Overview](../foundation/overview.md) for the zero-setup-first rule and non-goals.
