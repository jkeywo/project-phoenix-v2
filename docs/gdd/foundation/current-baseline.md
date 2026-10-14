# Project Phoenix — Current Game Design Baseline

| Field | Value |
|---|---|
| Document | GDD-CURRENT-BASELINE |
| Status | Current implementation snapshot with separately marked Plan |
| Last updated | 2026-10-02 |
| Revision reviewed | `26f19f17` |
| Purpose | Baseline for competitor comparison, station workload review and future design decisions |
| Authority | Code and assets establish Current; PASM establishes intended design and roadmap scope. This document summarises those sources. |

Phoenix is a scenario-driven spaceship bridge game. Players share a Viewscreen and operate a ship through separate browser Stations. The current implementation also supports native Windows bridges, several independently crewed ships, optional equal GM operators and a standalone Workshop. Its existing fiction favours capable professional crews and operational science fiction. That is the baseline under review, rather than a newly ratified restriction on future game design.

## Reading the baseline

- **Current** means implemented in this checkout. It does not imply public deployment, completed human acceptance, or proven enjoyment.
- **Plan** means work recorded in the PASM roadmap that is not fully delivered. Each item names its content or technical band, or says unscheduled.
- **Acceptance open** means the feature exists but the roadmap still requires integration, hardware, network or human evidence. It is not an absent mechanic.
- **Design target / open decision** means an experience goal or unresolved choice, not a capability to credit in a competitor comparison.

Use [the roadmap baseline](../future/roadmap-baseline.md) for the complete planned scope and [Station Experiences](../systems/station-experiences.md) for deeper role descriptions. Compare Current and Plan separately; do not give current Phoenix credit for the C2–C9 simulation extensions.

## Current play and participation

| Dimension | Current | Boundary / Plan |
|---|---|---|
| Basic setup | Browser host draws the shared 3D Viewscreen; phones scan a QR or type a join code. Phone consoles are pure HTML/CSS/JS. No player account or client installation is required. | Native setup is optional. A supported-device matrix and completed accessibility audit remain open. |
| Session | Host-authoritative fixed-tick simulation; players send inputs and receive permitted projections. Crew can join, disconnect and reconnect. | Losing the sole authoritative host stops that session; host-mesh recovery requires surviving peers and compatible state. |
| Lobby | Participants select scenario and hull through first-valid-wins arbitration, claim authored Stations, choose ratings and ready. Vacant Stations use Backfill; the host can deliberately start an AI crew. | Recommended human crew counts remain content/playtest judgments, not engine limits. |
| Division of labour | Hulls define primary seats, auxiliary human-seeking Stations, fine Systems and authored ratings. Complete visiting Stations appear as stable Hero Bar tabs. | **Plan — C2:** finish the star/detail ladder and world-authored detail floors; per-system floor resolution and distinct Cruiser rungs already exist. |
| Delegation | Backfill uses the same admitted System commands as humans. AFK retains a seat while delegating Systems. Command selects authored Station stances for AI-controlled Stations. | A working delegation path does not establish that automation leaves every human an interesting task. |
| Spectators | Explicit crew-public summary role, readiness exclusion and manual entry into eligible seats. Destroyed crews can remain cinematic spectators while surviving ships continue. | **Plan — C9:** richer authorised system-monitor participation. |
| Several bridges | Fixed ship slots, per-ship commands and addressed Objectives, deterministic host mesh, GM peers and two multi-ship reference worlds exist. | **Plan — T5:** close larger-session failure/replacement recovery, convoy scaling/pacing, performance and real-network/one-hour/human acceptance. |
| Mission ending | Authored results and report rows; ship loss is distinct from whole-fleet completion in multi-ship worlds. Reconnect identification also runs during GameOver. | A full campaign episode-selection/debrief flow is still **Plan**. Browser clean-round reload remains an unresolved lifecycle boundary. |
| Saves | Compatible authoritative snapshots, portable import/export, peer-local named saves and rolling autosave; native catalogue/CLI and GM checkpoint surfaces. | Saves resume the same compatible run. They do not provide a multi-episode campaign product. |
| Optional GM | Equal host-class operators inspect, direct, perform Comms, control knowledge/presentation and recover supported actions/checkpoints. | Several live human exits remain open; deep campaign/pacing is **Plan — T6**. |
| Authoring | Browser/native Workshop edits exact-source worlds, entities, Stations, scripts, roles, spatial/model data and mod packs, with validation, history, recovery and disposable Test. | Golden browser/native and dock human acceptance remains open. Source-level Rhai stepping/locals are **Plan — unscheduled**. |

## Current scenario catalogue

The [base manifest](../../../assets/scenarios.toml) offers four roots. The [curated demo manifest](../../../assets/scenarios.demo.toml) offers only Combat Test with the Alliance Destroyer and Alliance Cruiser; this is catalogue curation, not evidence that a deployment has been accepted.

| Scenario | Player work and resolution | Hulls / scale | Status |
|---|---|---|---|
| [Combat Test](../content/scenarios/combat-test.md) | Defend Starbase Alpha against escalating authored waves; manoeuvre, acquire targets, manage fire, shields, power and repairs. | Four Alliance classes plus the Dynasty player Cruiser in the base world; demo restricts to Destroyer/Cruiser. | **Current** combat reference; balance remains iterative. |
| [Falling Skyway](../content/scenarios/falling-skyway.md) | Survey infrastructure, handle a labour dispute and records, shelter traffic, rescue/stabilise/transfer, make passage commitments and allocate scarce capacity. | Alliance Destroyer; four primary seats plus hosted auxiliary Stations. | **Current** operational crisis and campaign-fact producer; narrative/crew workload follow-ups remain open. |
| [Alliance Convoy Escort](../content/scenarios/alliance-convoy-escort.md) | Protect three civilian transports; shared escort work plus addressed recovery and pursuit tasks. Ends on actual arrival/loss of all transports. | Four authored Alliance slots, each offering Destroyer or Cruiser. | **Current** T5 reference; scale and pacing ratification remain **Plan — T5**. |
| [Cruiser Elimination](../content/scenarios/cruiser-elimination.md) | Two Alliance Cruisers fight two Dynasty player Cruisers. Last surviving team wins; simultaneous team elimination draws. | Four fixed Cruiser slots; unclaimed ships run on Backfill. | **Current** competitive reference; no campaign or War Mode is implied. |

The current game therefore includes cooperative crisis play, cooperative multi-ship escort and a competitive team fight. Patrol, War, generated assignments, Orbital Mode and Sandbox are **Plan**, not selectable modes.

## Current hulls and workload

Primary seat counts are opportunities for direct participation, not recommended crew counts or proof that all seats remain busy. Navigation, Comms and Command can be auxiliary/hosted surfaces depending on hull data.

| Hull family | Primary seats | Current distinction |
|---|---:|---|
| Alliance Courier | 2 | Bundled Captain/support and Tactical/flight workload; compact, mobile hull. |
| Alliance Destroyer | 4 | Captain, Helm, Tactical, Engineering; mixed weapons and external-operation capability; Falling Skyway baseline. |
| Alliance Cruiser | 6 | Captain, Helm, Tactical, Science, Engineering, Comms; more separated expertise and distinct supported rating rungs. |
| Alliance Battleship | 9 | Captain, Helm, Tactical, Repair, Sensors, Shields, Navigation, Power, Comms; broad separation and capital-scale handling. |
| [Dynasty player Cruiser](../content/ships/dynasty-ships.md) | 6 | Command, Helm, Gunnery, Sensors, Power and Damage Control; distinct console presentation and generation-backed strike reserve over shared equipment. |
| Harrow NPC family | NPC-authored topology | Patrol, Destroyer, Cruiser and Warhawk encounter roles. The playable Dynasty Cruiser is a separate template. |

See [Alliance Ships](../content/ships/alliance-ships.md) and [Harrow Ships](../content/ships/harrow-ships.md). Numeric balance belongs in the composed hull assets and seeded balance evidence, not in a second static GDD stat sheet.

## Current station decisions and cooperation

These rows describe actions the game makes available. Their frequency, workload and fun must be measured in the scenario and rating being compared.

| Function | Current decisions / tasks | Why another officer matters | Plan that would change the task |
|---|---|---|---|
| Captain / Command | Set Red Alert and Viewscreen intent, prioritise Objectives, select AI Station stances, coordinate mission decisions. | Specialists own movement, fire, repair and external information; a stance directs AI posture rather than issuing every fine command. | **Plan — C5/C6/T6:** deeper commitments/database/run summaries and campaign recap. |
| Helm | Thrust and turn; use hull-provided lateral/vertical controls, boost and impulse; establish weapon or operation geometry. | Navigation supplies destination; Tactical asks for a bearing; Engineering needs docking/coupling; Shields needs protected facings. | **Plan — C3:** relative-motion verbs, physical map/local tiers and basic station docking. **Plan — C6:** discrete warp. |
| Tactical / Gunnery | Choose one Combat Lock, judge range/arcs/readiness, operate phasers/blasters/tubes and magazine; Dynasty Gunnery can commit strike reserve. | Helm supplies geometry, Sensors supplies facts/designation, Power supports output, Captain controls alert posture. | **Plan — C5:** calibration, ammunition fabrication/payload sidegrades and EWar. |
| Sensors / Science | Select a separate Science Target, scan authored subjects, designate contacts, report bearing/frequency and scenario evidence. | Findings support Tactical, Shields, Comms and Captain; inspection alone does not make a political decision. | **Plan — C2:** repeat-observation confidence and scan ladder. **Plan — C3:** probes. **Plan — C5:** intrusive scans/signatures. |
| Navigation | Set shared Waypoints, interpret chart/traffic, issue authored civilian orders. | Helm retains actuator authority; traffic route choices affect rescue, corridor and mission work. | **Plan — C3:** relative kinematics/map tier. **Plan — C6:** continuing-run route/database work. |
| Comms | Hail reachable actors, follow threads, choose/confirm responses, read dossiers and track promises. | Sensors supplies evidence; Captain chooses commitments; Helm/Engineering must make promises physically feasible. | **Plan — C2/C6:** deeper evidence/library surfaces. **Plan — unscheduled:** one-to-one human media calls. |
| Shields | Select/clear focus among authored shield arcs; watch regeneration, outages and damage bearing. | Helm changes the facing under attack, Sensors reports threat bearing, Power funds regeneration. | **Plan — C4:** detailed frequency/modulation and arc management. |
| Power | Allocate Helm/Weapons/Shields levels, accept or shed overdraw, protect battery reserve and recover from exhaustion lock. | Other officers receive the benefit or lose capacity; reserve imposes a shared risk. | **Plan — C4:** component supply/demand, coolant, heat/overpower, life support and load-shedding/presets. |
| Repair | Dispatch limited teams to Core or Stations; wait for travel and exact on-site diagnosis; prioritise revealed damaged Systems. | Station holders can describe local failures before Engineering sees exact remote damage. | **Plan — C4:** symptom-to-order diagnosis, materials and deeper triage. **Plan — C7:** general Duty Teams/Medical. |
| Crew-owned external Systems | Lock/position, tractor tow/stabilise, mate a dock, transfer through an umbilical and commit a repair team. | Tactical, Helm, Engineering and Repair provide different parts of one operation. | **Plan — C3/C4/C6:** station-service docking, resources, replenishment and refit; current operational docking is not the whole future service loop. |

Operations and Medical as personnel/Away Mission functions are **Plan — C7**. Existing external-operation machinery and repair teams do not constitute delivered personnel play.

## Current simulation and information boundaries

- Movement, weapons, shields, power, damage and repair share the host simulation and admitted commands. AI uses the same machinery and availability rules.
- Tactical's Combat Lock and Sensors' Science Target are separate. Advisory Coordination does not transfer another Station's authority.
- Damage reduces fine-System availability. Station holders know local details; Engineering gains exact remote diagnosis when a team arrives.
- Power is currently a compact three-group/battery problem. C4's generic network, per-component heat and life-support clocks are **Plan**.
- Worlds compose entities, anchors, regions, civilian routes, infrastructure condition/capacity, workforces, Objectives, deadlines, Comms, commitments and evidence through TOML and deterministic Rhai.
- Current scripted scans/dossiers and GM-reported contact effects are not C2's complete physical epistemics model. The roadmap still includes per-ship evidence/knowledge extensions.
- Scenario outcomes may project versioned campaign facts, including named assets, structures, evidence, standing and commitments. A follow-on campaign shell and ratified consumer episode are **Plan**.
- GM omniscience is an operator role; ordinary crew projections remain scoped. GM role presets change interface focus, not GM authority.

## Current delivery, input and presentation

Phoenix-owned rendezvous resolves typed crew/host codes and signals browser WebRTC. Direct gameplay uses reliable commands and a separate lossy snapshot class. When no direct link forms, the same game frames can travel over the rendezvous WebSocket. Native hosting also accepts crew directly over its LAN listener and can add the cloud leg. PeerJS is retired.

The native host runs the ordinary authoritative simulation and native Viewscreen. Its lobby supports scenario/hull selection, AI launch, join QR, runtime Viewscreen monitor moves and Station console assignment. Station screens hold one or two consoles, account for authored participant reservations and rebuild/reconcile consoles on their retained identities. Saved per-class bridge layouts and input routing exist. Media setup, private Accessibility/setup acceptance and the real-monitor kit retain open roadmap work; direct-LAN reliable-burst defect #1354 is also recorded as open in the roadmap audit.

Current platform work includes private effect-named Accessibility profiles, semantic actions and remapping, keyboard/gamepad ownership, correlated action feedback, presentation/comfort controls, audio categories and visual/text equivalents, and German localisation. Coverage and real-device acceptance vary by surface. Structured text/list alternatives for canvas displays and full screen-reader support remain **Plan — unscheduled**; broader assistance continues across content bands. Do not claim complete accessibility conformance.

The browser/native Workshop has Authoring and disposable Test contexts; connected GM Live inspection is a separate context and authority. Current Test supports deterministic restart/pause/step/speed, captured views, traces and scenario-state breakpoints. Models, rigs, markers, LOD metadata and assets use the same draft/history path; native asset generation/capture adds reviewed results. See [GM and Workshop](../systems/gm-and-workshop.md).

## Baseline questions for competitor comparison

For each scenario, hull, rating and crew count, record:

1. What each person repeatedly observes, decides and does; distinguish decision time from waiting for progress.
2. Which tasks demand another person's information or action, and whether players communicate voluntarily.
3. Where automation removes chores, where it removes the interesting decision, and what remains for an additional human.
4. Whether roles offer judgement, timing, diagnosis, negotiation, planning or dexterity, and which of those the group enjoys.
5. How threats and mistakes create changed choices, recovery and memorable shared moments.
6. How much instruction, reading, interface navigation and setup precede those moments.
7. Which limits come from current mechanics, scenario content, presentation, or setting expectations.

Station-activity counters, GM workload states, headless results and seeded duels provide evidence about actions and outcomes. They do not establish engagement by themselves. Human observation is needed for idle time, useful conversations, felt agency and enjoyment. The existing pillars and fiction are design targets to test against those observations.

## Canonical sources

- [Phoenix delivery roadmap](../../../pasm/spec/roadmap/phoenix-delivery-roadmap.yaml) and [GM milestones](../../../pasm/spec/roadmap/gm-console-milestones.yaml), including their delivery/acceptance audits.
- [Base catalogue](../../../assets/scenarios.toml), [demo catalogue](../../../assets/scenarios.demo.toml), hull/world assets and the linked content pages.
- [Domain vocabulary](../../../CONTEXT.md), [Station Experiences](../systems/station-experiences.md) and the GDD's linked mechanic pages.
- [Networking](../../../wiki/concepts/networking.md), [Native Host](../../../wiki/concepts/native-host.md), [Save Catalogues](../../../wiki/concepts/save-catalogues.md) and [Workshop](../../../wiki/entities/editor.md), with their code sources.
- [GameOver reconnect registration](../../../crates/phoenix-simulation/src/lobby/server.rs) and [browser round reuse](../../../server.html).

This review updates documentation only. It does not constitute a fresh gameplay, deployment, hardware or release-acceptance run.
