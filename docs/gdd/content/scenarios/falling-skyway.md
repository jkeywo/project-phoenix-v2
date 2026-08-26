# Project Phoenix — Falling Skyway Content

| Field | Value |
|---|---|
| Document | GDD-CONTENT-FALLING-SKYWAY |
| Status | Implemented authored scenario; balance and recommended crew remain TBD |
| Owner | Unassigned |
| Last updated | 2026-08-26 |
| Scope | Premise, intensity eras, actors, systems, choices, outcomes, and authoring contract for Falling Skyway |
| Authority | `assets/worlds/falling_skyway.toml` and referenced templates are content truth. |

Falling Skyway is the current operational-crisis scenario. An orbital skyhook, its fuel-depot ladder, and the traffic depending on them are failing as a radiation storm approaches. A labour dispute, corporate interests, civilian movement, incomplete records, limited transfer capacity, and an armed picket make the crisis political as well as technical. The crew cannot preserve every claim in full; it must learn what is true, keep enough infrastructure alive, and decide who receives scarce passage.

Related documents: [Scenario Authoring](../../systems/scenario-authoring.md), [Campaign Continuity and Persistence](../../foundation/campaign-continuity.md), [Thin Margin Setting](../../foundation/thin-margin-setting.md), [World and Environmental Systems](../../systems/world-environmental-systems.md), [Station Experiences](../../systems/station-experiences.md), [Difficulty and Balance](../../foundation/difficulty-balance-playtesting.md), and [AI and Backfill](../../systems/ai-and-backfill.md).

Detailed mechanics: [Sensors and Epistemics](../../mechanics/sensors-epistemics.md), [Navigation and Relative Motion](../../mechanics/navigation-relative-motion.md), [Comms and Commitments](../../mechanics/comms-commitments.md), [External Operations](../../mechanics/external-operations.md), and [Damage, Diagnosis and Repair](../../mechanics/damage-repair.md).

## Player-facing summary

| Attribute | Design |
|---|---|
| Form | Multi-era operational crisis / intended campaign episode |
| Expected length | Ratified 20–30 minute design envelope; the allocation window closes at 1800 seconds and ending facts land 26 seconds later |
| Recommended crew | TBD by playtest |
| Possible crew | 0–4 on the currently offered Alliance Destroyer |
| Offered ship | Alliance Destroyer only |
| Core pressures | Infrastructure failure, storm timing, civilian traffic, labour dispute, evidence, commitments, force escalation, insufficient transfer capacity |
| Principal choice | Two effective berths and limited lift/transfer capacity against three claimant groups |
| Public demo | Not included in the restricted demo catalogue; available in the base/dev catalogue |

## Experience goals

- Make the crew operate a starship as a practical instrument of rescue, investigation, traffic control, diplomacy, and deterrence—not only combat.
- Give different stations partial views of one crisis so scans, comms, navigation, engineering, command, and tactical restraint all matter.
- Make promises and evidence consequential, with outcomes that record not only survival but how the crew achieved it.
- Allow negotiation, coercion, technical recovery, traffic handling, and selective force without declaring one universally correct method.
- Force a genuine allocation decision: authored maximum available transfer supply is 52 against total claimant demand of 66, so a perfect everyone-gets-everything result is not available.

## Situation

The Falling Skyway is a city-world orbital lift and fuel-transfer network. The skyhook begins in poor condition near its lift-capable threshold. Depot Ladder A remains operational; Depot Ladder B starts below its certified operating threshold. A workforce strike has stopped part of the system, Havelock operations contest the workers’ position, civilian and claimant traffic occupies the approaches, and a radiation storm will cross the corridor before the allocation decision is complete.

The scenario uses one shared map of the skyway head, station-keeping area, traffic corridor, depot berths, worker and corporate moorings, picket, storm bands, shelter, and debris. These are anchors used by routes, objectives, operations, spawns, and script acknowledgement.

## Principal actors

| Actor | Starting role and pressure |
|---|---|
| Skyway Control | Operational authority and source of system status; needs the head and corridor kept viable. |
| Strike Committee | Represents skyway workers, begins on strike, seeks recognition and a passage claim. |
| Havelock Cutter / Operations | Corporate claimant with its own records, interests, and armed enforcement presence. |
| Convoy Meridian | Civilian/commercial claimant requiring scarce transfer access. |
| Havelock Enforcer | Harrow Patrol-derived picket capable of coercion and escalation; can be held, disabled, attacked, or destroyed. |
| Civilian traffic | Haulers Lark and Pell plus Shuttle Wick moving through authored routes and exposed to storm/traffic loss. |
| Rigger Tacket | Operational actor associated with repair/recovery around the skyhook. |
| Lyra Ascending | Civilian vessel placed in immediate storm danger and available for rescue/recovery. |

The actors are simulation entities with position, route, comms, damage, faction, or infrastructure facts as applicable. Dialogue should acknowledge those facts rather than pretending they exist only in conversation.

## Three intensity eras

### Opening era — Arrival, strike, and survey

The crew arrives to an already unstable system. Visible deadlines at 160 seconds and 360 seconds put the tether slip and survey requirement on the bridge throughout the opening era. The survey is actionable immediately and posts as an objective when the tether slips: scan Skyhook Verrow and both ladder depots, then hail Control and file the report through its gated response. Filing early is remembered and completes the objective when it posts. A partial survey cannot be filed; an unfiled survey fails at 360 seconds with an urgent Control transmission and Control proceeds on the existing falsified record.

The strike is a parallel thread, not the next act. The committee and Havelock are hailable from t=0; Havelock's mooring is 754 units from the player spawn, inside the courier's unchanged 800-unit reciprocal comms range. The line and corroboration objectives post with incoming channels at the t=160 tether slip. If nobody answers, the committee hardens at t=120 and t=300. The first rung therefore exists before the incoming post, so an unattended bridge encounters a hardened committee rather than having Backfill's first response erase every escalation. The first admitted response on either channel stops later hardening. Before hardening, any two of safe-passage promise, records promise, and the maintenance file carry a vote; afterwards all three are required. Each hardening rung lowers workforce disposition and makes a forced clearance costlier, but negotiation, stalling, evidence-gathering, refusal, warning, and immediate force remain available. The legacy `act` flag is only a narrative counter and gates none of these interactions.

The design purpose is to distribute first tasks quickly: Captain and Comms establish claims; Science/Sensors gather the condition and record picture; Helm and Navigation orient to the corridor and infrastructure; Engineering prepares power and external operations; Tactical decides whether an armed picket is a target, a deterrent, or an actor to hold at risk.

Depot Ladder B’s structural reading and filed record can disagree. Scanning, obtaining records, and gaining worker corroboration can build an evidence chain. Evidence matters later in negotiation and consequences; it is not just collectible lore.

### Storm era — Physical crisis

The storm front is due at 400 seconds, with bands at 580, 740, and 900 seconds and passage recorded at 1072 seconds. Lyra Ascending’s clear deadline falls at 768 seconds. Every weather beat has its own deadline trigger and waits for neither strike settlement nor an act checkpoint: the storm changes the physical and operational problem while civilian traffic remains in motion.

Navigation must issue one authored shelter diversion to Meridian, Lark, and Pell before the first band. Each civilian’s traffic row publishes an accessible **ORDER TO STORM SHELTER** control, and each payload-bearing objective gives Backfill Navigation the same target and route; both emit the ordinary admitted `OrderCivilian` command. The compliance machine then puts that craft on a route clear of all three bands. The front does not issue orders or mutate traffic for the crew. An unordered craft remains physically exposed, and its destruction immediately posts a named Control report, per-craft ledger fact, failed clearance objective, and aggregate loss objective. The ship’s own destruction remains the region damage system’s terminal defeat, not a scripted checkpoint.

The crew may need to order traffic, open or hold the lane, shelter vessels, rescue or tow Lyra, recover lost objects, and stabilise the skyhook. Operations take time, range, power, and capability; beginning one is a commitment of ship position and crew attention. Traffic destruction and missed recovery opportunities are recorded rather than erased when the narrative counter advances.

The skyhook has several load-bearing facts. If condition and stabilisation do not preserve them, warning flags clear in sequence and a projected failure deadline can end in structural loss. The scenario recognises both successful stabilisation and collapse.

### Crowded back-half era — Allocation and consequences

After the storm, the skyhook’s projected failure falls at 1528 seconds, then the transfer window opens at 1600 seconds and closes at 1800 seconds. Three claimant groups make requests against two berths and insufficient total capacity. The crew can make, keep, or break earlier commitments, confront actors with evidence, and decide who receives lift/transfer access.

The decision is not a detached dialogue menu. Infrastructure capacity, claimant presence, prior promises, strike settlement, evidence, casualties, and the surviving corridor constrain what can be offered and how actors respond. The scenario resolves the berth choices and promises, then writes campaign-facing facts describing passage, labour outcome, evidence, casualties, structure, commitments, and aggregate/per-craft storm traffic losses.

## Authored clocks

| Deadline | Due | Design function |
|---|---:|---|
| `tether_slip` | 160s | Immediate physical warning |
| `skyway_survey_due` | 360s | Forces early investigation |
| `storm_front_due` | 400s | Opens the environmental crisis |
| `storm_band_one_due` | 580s | Storm escalation |
| `storm_band_two_due` | 740s | Storm escalation |
| `lyra_clear_due` | 768s | Rescue urgency |
| `storm_band_three_due` | 900s | Final storm escalation |
| `storm_passed_due` | 1072s | Opens post-storm resolution work |
| `skyhook_failure_due` | 1528s | Structural recovery/failure horizon |
| `skyway_transfer_window` | 1600s | Opens final allocation |
| `skyway_window_closes` | 1800s | Ends the available decision window |

These values are implemented current design and place every deadline once inside the three PASM intensity eras. Human playtests must still test console navigation, dialogue reading, reconnects, and small-crew Backfill before the recommended player count is final.

## Core system interactions

### Infrastructure and operations

The skyhook and depots carry condition and named capacities. The Destroyer carries explicit capabilities for scanning and external work. Stabilise, tow, escort, transfer, and related operations are authoritative timed actions with range and interruption rules. Scenario effects begin or alter those operations; the script does not declare success merely because the crew chose a line of dialogue.

### Traffic and navigation

Three authored routes—`skyway_lane`, `ladder_run`, and `storm_shelter_run`—give civilian entities a persistent spatial life. The three endangered craft author finite `order_options` that Navigation receives through the authoritative traffic snapshot; their buttons emit the existing `order_civilian` action rather than a scenario-only verb. Backfill reads the same target and route from `Order` objectives and enters through that command path. Hold, divert, dock, and corridor decisions affect real movement. Waypoint arrival and destruction can trigger acknowledgement and consequences.

### Labour and disposition

Two workforces are authored: `skyway_workers` begins on strike with disposition 25; `havelock_operations` begins working with disposition 55. Negotiation, evidence, promises, force, and outcomes can change the political state. A structure’s workforce link makes labour operationally relevant to capacity rather than purely narrative.

### Evidence and dossiers

The Ladder B reading, documentary record, and worker corroboration form separate facts. The crew can discover disagreement, name it, and use it. Evidence is filed through the scenario’s evidence/dossier substrate so later choices can test what the crew actually established.

### Commitments

Promises of passage or records access are recorded commitments. The final resolution can determine whether they were fulfilled, broken, or superseded. The design goal is for Comms choices to become operational obligations the rest of the bridge must understand.

### Force and restraint

The Havelock enforcer can be hailed, threatened, attacked, disabled, or destroyed as supported by current state. The scenario observes weapons hold, attacks, disablement, and destruction. Force is neither forbidden nor consequence-free; a clean technical or negotiated outcome may require restraint, while some states may make coercion useful.

## Scenario TOML/Rhai shape

```toml
[global]
seed = 1034
title = "world.falling_skyway.global.title"
description = "world.falling_skyway.global.description"

[[available_ships]]
template_path = "assets/entities/alliance_destroyer.toml"
label = "world.falling_skyway.available_ships.0.label"

[[workforce]]
id = "skyway_workers"
label = "world.falling_skyway.workforce.skyway_workers.label"
on_strike = true
disposition = 25

[[deadline]]
id = "storm_front_due"
label = "world.falling_skyway.deadline.storm_front.label"
due_secs = 400
visible = true

[script]
setup = '''
on_world_loaded("on_arrival");
on_deadline("storm_front_due", "on_storm_front");
on_flag_set("skyway_storm_passed", "on_act_three_opens");
on_deadline("skyway_window_closes", "on_transfer_window_closes");
'''
```

This excerpt documents the architecture, not the full scenario. The authoritative script is roughly 159,000 characters and includes dialogue nodes, handler functions, objectives, operations, evidence, traffic, failure projection, allocation, and campaign fact resolution.

## Outcome dimensions

Falling Skyway should not collapse to a single score. Its debrief/campaign handoff distinguishes at least:

- Whether the passage/berth decision was made and who received access.
- Whether the strike was settled by negotiation, coercion, or remained unresolved.
- Whether the records discrepancy and corroborating evidence were established and used.
- Whether civilian and named-actor casualties occurred.
- Whether the skyhook, depots, traffic corridor, Lyra, lighter, and pod survived or were recovered where tracked.
- Which commitments were made, honoured, or broken.

A “victory” may therefore be operationally successful while carrying political, ethical, or human costs. The terminal presentation should make those dimensions legible rather than hiding them behind one label.

## Playtest questions

- What recommended crew size lets players read the crisis without either idling or drowning in simultaneous information?
- Can a first-time crew identify an actionable opening before the 160-second tether deadline?
- Are scans, records, and corroboration understood as different evidence rather than repeated flavour text?
- Do traffic and infrastructure feel physically present through movement, capacity, and operations?
- Do players understand that total demand exceeds supply before making final promises?
- Are negotiation, restraint, disablement, and force all legible options without implying equal consequences?
- Does each intensity era acknowledge prior choices, and can players explain the final outcome dimensions?
- Are the 200 seconds between the 1600-second opening and 1800-second close enough for the allocation scene’s reading and coordination?

## Success measures

- Most test crews can name at least two simultaneous pressures and delegate work within the opening two minutes.
- Every occupied station has at least one consequential observation, recommendation, or action during the scenario; any consistently idle station indicates bundling or pacing work.
- At least three meaningfully different resolution patterns emerge across tests without facilitator invention.
- Players can state why a claimant was accepted or denied and which earlier fact or promise affected that decision.
- Failures produce acknowledged consequences and do not strand progression.
- The scenario completes with coherent campaign facts for every common branch.

## Open content decisions

- Recommended player count and dialogue reading allowance remain TBD; the authored deadline spacing now needs human validation rather than another provisional clock.
- Final prose, voice, faction terminology, and setting consistency need a Thin Margin editorial pass.
- Other Alliance hulls are not currently offered and would require operational-capability and pacing validation.
- The exact win/defeat framing and debrief presentation need consolidation with the lifecycle document.
- The campaign contract now defines how the facts can be consumed, but the first specific follow-on episode and its bindings remain undecided.
- Accessibility alternatives for time pressure, dense dialogue, colour-coded state, and audio cues remain to be specified.

## Canonical sources

- `assets/worlds/falling_skyway.toml`
- `assets/scenarios.toml`
- `assets/entities/skyhook.toml`, `depot_transfer.toml`, civilian hulls, `ship_harrow_patrol.toml`, and the Alliance Destroyer
- Scenario scripting, infrastructure, operations, civilian traffic, science/evidence, deadlines, workforce, and campaign handoff runtime modules
