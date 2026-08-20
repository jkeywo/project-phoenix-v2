# Project Phoenix — Future Modes and Optional Bridge Extensions

| Field | Value |
|---|---|
| Document | GDD-FUTURE-MODES |
| Status | Directional design; most sections are roadmap or permitted extension, not committed current functionality |
| Owner | Unassigned |
| Last updated | 2026-08-19 |
| Scope | Multi-ship, Patrol, War and Sandbox modes, ship customisation, generated-mode foundations, native/physical bridges, event outputs, and GM tools |
| Authority | Product/design direction only. PASM roadmap and specific future PRDs govern implementation commitments. |

Phoenix may scale from a browser, television, and phones to multi-ship sessions and purpose-built bridge venues. Every extension must preserve the complete zero-setup route and the same authoritative station/system model. Optional depth should add ways to participate, author, and present the game without splitting it into incompatible “real” and “casual” versions.

Related documents: [Game Design Overview](../foundation/overview.md), [Game and Session Lifecycle](../foundation/game-lifecycle.md), [Campaign Continuity and Persistence](../foundation/campaign-continuity.md), [Station Experiences](../systems/station-experiences.md), [Onboarding, Tutorials, and Accessibility](../foundation/onboarding-accessibility.md), [AI and Backfill](../systems/ai-and-backfill.md), [Scenario Authoring](../systems/scenario-authoring.md), [Release Bands C7–C9](./release-bands-g-i.md), [Planned but Not Scheduled](./planned-not-scheduled.md), and [Difficulty, Balance, and Playtesting](../foundation/difficulty-balance-playtesting.md).

## Non-negotiable baseline

- One browser host/shared display plus pure-web phone clients remains a complete supported experience.
- No native binary, local-network configuration, account, custom peripheral, GM, or venue effect becomes required for ordinary scenarios.
- A scenario’s rules and all time-critical station actions remain accessible through standard browser controls.
- Extensions adapt the same authoritative commands and state; they do not create a separate simulation authority.
- Failure or absence of an optional adapter degrades gracefully to browser presentation/input.

## Status overview

| Direction | Current foundation | Status in this GDD |
|---|---|---|
| T1 Bridge Foundations (formerly Band A2) | Crew control plus front-loaded networking and native Windows bridge work. | Accepted first technical band after release A; split across two PRDs |
| Native delivery | Native `phoenix-host` can serve a built bundle/catalogue/version pin; T1 adds native simulation/viewscreen and Ultralight station hosting. | Started foundation with accepted T1 expansion |
| Multi-ship | Per-ship state, AI ownership, command logs and deterministic fixed ticks exist; T1 supplies transport/recovery, and the multi-ship host mesh follows in T5. | Planned roadmap direction with front-loaded foundation |
| Patrol Mode | Scenario systems, seeded world content and campaign projection. | Authored MVP in Band C6; procedural generation in C8 |
| War Mode | Multi-system run state, Pursuit, allies and coarse strategic consequences. | Authored MVP in Band C7; procedural generation in C8 |
| Sandbox Mode | Shared generator/Director plus persistent actor, economy and sector state. | Accepted Band C9 direction |
| Ship customisation | Entity fragments/composition and accepted supply/demand design. | Accepted staged roadmap, not current player feature |
| Duty Teams and Away Missions | Fixed typed teams, named Duty Officers, Operations systems, Medical and off-screen assignments. | Accepted; planned but not scheduled |
| Physical bridge | Web consoles, optional native delivery, stable commands/state. | Native Windows bridge foundation accepted for T1 |
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

Patrol Mode is a continuing series of assignments. Band C6 ships a complete hand-authored Patrol run and the persistent multi-system shell. Band C8 later adds a seeded system and situation generator producing physical context, actors, needs, infrastructure, evidence, commitments and cascades. The goal is coherent episodes, not a catalogue of random planets.

### Authored and generated responsibility

Authors define grammars, constraints, actor types, system relationships, failure patterns, tone, pacing bands, and outcome projections. Generation selects and binds compatible pieces from one seed, then validates that objectives, knowledge paths, operations, and terminal states remain coherent. Hand-authored scenarios remain the quality reference.

A hand-authored first patrol teaches exploration, database/library use, mission logging, wide/focused observation and campaign handoff in Band C6. Procedural assignments open in Band C8. Repetition controls track recently used structures, actor conflicts, hazards and resolution patterns, not only names.

Band C6 introduces the mode-profile and run-orchestration seams. Band C8 introduces the shared seeded generator and Director for both Patrol and War. Sandbox consumes the same generator and Director under a less constrained profile rather than forking them.

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

## Duty Teams, Duty Officers, and Away Missions

The accepted unscheduled model generalises repair teams into fixed anonymous Duty Teams by type. Each team has a named Duty Officer leader whose compatible traits provide bonuses, but a leaderless team remains functional. Duty Officers can also occupy authored ship-system slots, gaining benefits and exposure to discrete damage-event casualties while assigned.

Multiple Operations systems launch compatible off-screen missions through shuttles, transporters or carried craft. Required and optional slots may take teams, individual officers or both. The bridge supports missions through ordinary scans, Comms, deadlines and scenario effects; exact check probabilities reflect preparation, personnel, transport and current conditions. Medical treats persistent personnel consequences without introducing continuous crew simulation.

The complete contract is in [Duty Teams, Officers and Operations](../mechanics/duty-teams-and-operations.md).

## War, procedural generation and Sandbox

Band C7 ships an authored War Mode run toward one decisive mission, letting the crew gather allies and supplies and weaken enemies across intervening systems. Band C8 procedurally generates Patrol assignments and War runs. Band C9 removes the fixed assignment chain or destination for Sandbox and gives the shared Director wider latitude in a persistent actor-and-economy simulation suitable for player-chosen work and GM-led sessions.

All modes retain bridge-first play. Allied vessels are autonomous actors directed through Comms rather than directly controlled units. Warp is discrete system-to-system transit. The complete C7–C9 scope and distributed accessibility programme are defined in [Release Bands C7–C9](./release-bands-g-i.md).

## Native hosts and clients

The current native host is a delivery server: it serves a built client bundle, manifest, catalogue, and version stamp. T1 extends this into an optional Windows host that runs the authoritative simulation, renders the viewscreen natively and presents isolated Ultralight station panes across full-screen monitors. It replaces PeerJS with Phoenix WebSocket rendezvous/signalling and direct or TURN-relayed WebRTC while preserving the browser route.

A native client is an optional shell around the same console actions and state. Each local pane has its own player identity and projection; it must not gain exclusive controls, higher simulation authority, or content incompatible with a browser participant. Bridge profiles author one or two station panes per station monitor, a native viewscreen, independent touchscreen routing and per-surface cameras, microphones and audio outputs. See [Native and Network Foundation](../systems/native-network-foundation.md).

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
- Human-readable client codes are private to each ship host; the fleet-wide server code admits or recovers fixed host slots through a separate project namespace.
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
- Duty Teams and Away Missions remain bounded assignments rather than continuous NPC-person simulation.
- Hardware and show control use semantic commands/cues, browser fallback, accessibility equivalents, and venue safety boundaries.
- GM actions are scoped, visible, auditable, and recoverable in proportion to impact.

## Canonical sources

- `pasm/spec/roadmap/phoenix-delivery-roadmap.yaml` and `gm-console-milestones.yaml`.
- `pasm/spec/design/ship-customisation.yaml`, `fields-epistemics.yaml`, and P2P design slices.
- `pasm/spec/architecture/native-delivery.yaml`, `pasm/spec/design/p2p-design-deltas.yaml`, command-log/snapshot architecture, and world campaign projection.
- [Native and Network Foundation](../systems/native-network-foundation.md) and [Command and Crew Control](../mechanics/command-and-crew-control.md).
- [Game Design Overview](../foundation/overview.md) for the zero-setup-first rule and non-goals.
