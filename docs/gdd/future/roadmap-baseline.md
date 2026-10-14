# Project Phoenix — Plan and Delivery Baseline

| Field | Value |
|---|---|
| Document | GDD-ROADMAP-BASELINE |
| Last updated | 2026-10-02 |
| Status | Roadmap synthesis; every undelivered item is marked Plan |
| Authority | [Phoenix delivery roadmap](../../../pasm/spec/roadmap/phoenix-delivery-roadmap.yaml) and [GM milestones](../../../pasm/spec/roadmap/gm-console-milestones.yaml) |

This is the future half of [the current comparison baseline](../foundation/current-baseline.md). Content C1–C9 and technical T1–T6 are parallel tracks after release A, not a claim that every earlier band has completed public acceptance. PASM `accepted` describes a design decision; it is not an implementation status. Where summaries retain original sequencing language, use their delivery audits and current code/assets to establish what exists.

## Existing foundations

**Current:** deterministic fixed ticks/command logs/replay and snapshots; composable entity templates; authored fine-System AI and Backfill symmetry; Rhai scenario scripting; Falling Skyway infrastructure/traffic/commitment/evidence toolkit; code for native/cloud delivery; mod content packs; peer-local save UX; structured scenario/AI/activity observability; Crew Control and Distributed Ship Operations.

**Current — C1:** Alliance Cruiser tutorials, model LODs, seeded balance, distinct rating rungs and inclusion in the curated demo; Falling Skyway Scan/Operate/Order Backfill directives. World-authored floor selectors are still **Plan — C2 (#1067)**. Falling Skyway narrative review #1352 and PRD #1337 are separate open follow-ups, rather than missing C1 directive support. The 80–95% mandatory-objective completion interval in PASM is a target; this document does not claim a newly measured result.

**Plan — release acceptance:** code availability does not complete credentialed/manual deployment or catalogue playtests. The older demo milestone's one-hull wording is historical; the live curated manifest now offers Destroyer and Cruiser.

## Content track — Plan

| Band | Plan: content | Plan: new player work / systems | PASM component / issue anchor |
|---|---|---|---|
| C2 | Borrowed Sun; Cruiser default when the mission ships. | Environmental moving fields/falloff; observation confidence and detection→identification→classification→tactical assessment; infrastructure-network capacity/priority; debris; completed complexity/detail ladder with Sensors and Comms evidence surfaces. Retrofit Skyway storm without changing its observable behaviour. | `roadmap-borrowed-sun-foundation`, `roadmap-borrowed-sun-content`; #1016/#1017/#858, floor selectors #1067. |
| C3 | Narrow Season; Courier readiness. | Physical star/body descriptors, tick-derived orbits and map/local space tiers; source-derived fields; relative kinematics and match/hold/intercept/interpose/escort operations; basic station docking; Helm/Navigation detail and remote sensor probes; civilian response, emergency authority, traffic priority and congestion/panic/coercion. | `roadmap-narrow-season-foundation`, `roadmap-narrow-season-content`; #1018/#859/#877. |
| C4 | The Broken Tithe; Battleship readiness. | Unified producer/conduit/buffer/consumer networks for power, coolant, containment and life support; component demand, hard battery exhaustion plus local heat/overpower; load-shedding/presets; diagnosis/triage/materials and symptom-to-order Repair; Shields frequency/modulation; named bulk resources, propellant, docked repair/replenishment/transfer; hazardous cargo and contested intervention. | `roadmap-broken-tithe-foundation`, `roadmap-broken-tithe-content`; #1019/#860. |
| C5 | The Silent Corridor; full-fleet/mixed-fleet readiness. | Regional consequences from previous foundations; Tactical calibration, fabrication and frozen-at-launch payload sidegrades; intrusive risky scans; Jam/Spoof/Harden EWar and signatures; Captain commitments/objectives detail; one-hull loadout MVP on reactor plus Tactical variant axes. | `roadmap-silent-corridor-content`, `roadmap-ship-customisation`, `roadmap-post-demo-readiness`; #1020/#861/#1091/#922. |
| C6 | One hand-authored Patrol run. | Persistent multi-system run shell, discrete warp and warp fuel, mission log, station databases/Intelligence correlation, docking refit/rewards; typed/sized mounts, optional slots and broader fleet loadouts. | `roadmap-patrol-mode`, `roadmap-ship-customisation`; #1021. |
| C7 | One authored War run leading to a decisive finale. | Coarse fronts/fleets/allies, autonomous Comms orders, Pursuit; Duty Teams/Officers, Operations, off-screen Away Missions, boarding, surrender and Medical; inter-scenario consequences. | `roadmap-war-mode-mvp`; #1094. |
| C8 | Generated Patrol and War; Orbital Mode. | Shared seeded generator/Director, authored/versioned libraries, solvability constraints, preview, benchmark seeds and repetition control; generated operational/combat structures and Away Missions; anomalies/terrain; coarse wrapping planetary surface with points of interest. | `roadmap-patrol-war-pcg`; #1095. |
| C9 | Sandbox in a persistent curated sector. | Event-driven actors/economy, discrete cargo lots, layered standing, sector/fleet state and player-chosen work; wider Director profile, autonomous allies and advanced spectator participation. | `roadmap-sandbox-mode`; #1096. |

**Plan — customisation contract:** station claimants choose materiel; unclaimed Stations use defaults; the composed loadout freezes at spawn and refits only through an authored starbase path. Ordinary options are net-neutral sidegrades under the future power/heat model. A mission-authored reactor reward is the explicit exception. Entity fragment authoring today is not a delivered player loadout picker.

## Technical track — Current and Plan

| Band | Current evidence | Plan / acceptance still open |
|---|---|---|
| T1 — Bridge Foundations | Crew Control/Command, human-seeking Stations, Hero Bar, AFK/Spectator, Distributed Ship Operations, Phoenix transport, two-host protocol, native simulation/panes/lobby/layouts/input, debug and local-message registry work. | **Plan:** complete media-device setup and hardware proof #1126; private Accessibility/setup acceptance #1127/#1128; real-monitor kit #1335; direct-LAN burst defect #1354; roadmap defect backlog and unresolved native/network acceptance. |
| T2 — GM M1–M3, packs, saves, input | M1 peer/pause/puppet and M2 directing exits are closed; M3 Comms/knowledge/role-preset code exists. Mod packs, peer-local saves and much of semantic input/feedback/remapping are implemented. Native raster/determinism prerequisite work is recorded. | **Plan:** M3 two-equal-GM human run #1320 and remaining platform/input/native prerequisite acceptance. Partial implementation must be assessed by surface, not by crediting the entire band. |
| T3 — GM M4–M5, comfort | Attention queue, quiet-time/peer-health/workload/widgets; action history, supported undo, checkpoints/restore; presentation/comfort work. | **Plan:** busy-session facilitation #1440 and live-event recovery #1448 human exits. |
| T4 — GM M6–M8, audio | Integrated Workshop/Live Inspector and shared docks; browser/native Authoring and disposable Test; composition, roles, scripts, models/rig/LOD/capture; old editor/viewer interfaces retired to redirects; combined native ship/GM capability; information control and in-sim cues; audio/equivalents code. | **Plan:** golden browser/native workflow #1487, dock human acceptance #1514 and real-device audio acceptance #1467. |
| T5 — multi-ship scale/localisation | Digest widening, fixed slot selection, addressed Objective instances/recipient scopes, six-peer deterministic tests, bounded runtime paths, wreck spectators, Dynasty player Cruiser, escort/elimination references and German localisation. | **Plan:** #1090/#1153 and PRDs #1515–#1517 close failure/replacement recovery, convoy scale/pacing, ratified limits, integrated journeys, real-network/one-hour tests and human accessibility/language acceptance. |
| T6 — GM M9 / campaign | Snapshot/save machinery, campaign-fact projection and existing attention tools are foundations. | **Plan:** campaign continuity across sessions, deep advisory pacing heuristics and continuity summaries/recap. A complete campaign product does not exist yet. |

The supported T5 target is four player-ship hosts plus two equal GM peers. Implemented six-peer tests are evidence about determinism, not proof that this complete configuration has passed real-event acceptance. A combined native ship/GM process counts once in the technical topology while keeping independent ship/GM roles.

## Plan — accessibility and polish across bands

The roadmap keeps this as a continuous programme. Current profile, input, comfort, audio and localisation work is credited in the technical table; it does not establish conformance.

| Stream | Plan |
|---|---|
| Content C2–C5 | Complexity composition; alternatives for motion/docking/multi-ship work; mastery-task assistance; readable dense tactical information. |
| Content C6–C9 | Run/map/return summaries; strategic alternatives; accessibility-aware generation constraints; alternative participation and completion audit. |
| Technical T1–T6 | Close outstanding settings/input/setup proof, presentation/comfort and audio acceptance; localisation robustness; campaign continuity and recap. Every band also carries polish for the surfaces it introduces. |

## Plan — accepted but unscheduled

These are recorded by `roadmap-planned-not-scheduled` and, for the debugger, `gm-unscheduled-rhai-source-debugger`. They carry no release promise.

- One-to-one simulated Comms voice/video/typed-text calls, consent, private/Viewscreen endpoint handoff, transient captions and the Discord adapter. T1 device routing does not ship calls.
- Persistent mines; post-C5 cloaking; expanded EWar beyond Jam/Spoof/Harden; multi-interlock self-destruct.
- Custom peripherals and external show systems. Existing in-sim title/sound/Viewscreen cues are Current; venue hardware is not.
- Source-level Rhai stepping, expression breakpoints, locals and stack frames beyond Workshop's current diagnostics/traces/state breakpoints.
- Structured text/list alternatives for radar, Navigation map and Viewscreen canvases, and full screen-reader support.

Duty Teams/Medical/Pursuit/surrender/War are **Plan — C7**, Orbital/generated modes **Plan — C8**, and Sandbox/economy/advanced spectators **Plan — C9**. Older descriptions calling those unscheduled are superseded by the present roadmap.

## Planning limits

There are no dates, staffing or effort estimates inferred here. The roadmap is the scope source; GitHub issues hold execution status. Human acceptance kits prepared in the repo do not establish completed sessions. Read current design values through `uv run pasm design digest` before changing scenario pacing or consequences. Record a redesign in PASM and update this baseline only after the corresponding implementation or plan actually changes.
