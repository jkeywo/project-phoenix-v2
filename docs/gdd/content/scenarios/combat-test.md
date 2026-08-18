# Project Phoenix — Combat Test Content

| Field | Value |
|---|---|
| Document | GDD-CONTENT-COMBAT-TEST |
| Status | Implemented content overview; balance remains iterative |
| Owner | Unassigned |
| Last updated | 2026-08-18 |
| Scope | Premise, structure, scaling, entities, outcomes, and authoring contract for Combat Test |
| Authority | `assets/worlds/combat_test.toml` and referenced templates are content truth. |

Combat Test is a direct cooperative defence scenario. The crew protects Starbase Alpha against eight timed Harrow waves, using a selectable Alliance ship. It is intentionally simpler than Falling Skyway and is fully compatible with the design pillars: the scenario may provide a scripted problem and answer while the crew still coordinates a connected ship to execute that answer.

Related documents: [Scenario Authoring](../../systems/scenario-authoring.md), [Station Experiences](../../systems/station-experiences.md), [AI and Backfill](../../systems/ai-and-backfill.md), [Difficulty and Balance](../../foundation/difficulty-balance-playtesting.md), [Alliance Ships](../ships/alliance-ships.md), and [Harrow Ships](../ships/harrow-ships.md).

Detailed mechanics: [Movement and Helm](../../mechanics/movement.md), [Targeting and Weapons](../../mechanics/targeting-weapons.md), [Damage, Diagnosis and Repair](../../mechanics/damage-repair.md), [Power and Resource Network](../../mechanics/power-resource-network.md), and [Shields](../../mechanics/shields.md).

## Player-facing summary

| Attribute | Design |
|---|---|
| Form | Combat drill / defence mission |
| Recommended crew | 2–4 |
| Possible crew | 0 through the selected hull’s station maximum |
| Offered ships | Alliance Courier, Destroyer, Cruiser, Battleship |
| Protected objective | Starbase Alpha |
| Opposition | Eight timed Harrow wave groups with power-rating scaling |
| Victory | All hostiles destroyed after all eight waves have spawned |
| Defeat | Starbase Alpha is destroyed or the player ship reaches its terminal destruction condition |
| Public demo | Included, with the demo catalogue restricted to the Alliance Destroyer |

## Experience goals

- Move a crew from lobby into meaningful bridge communication quickly.
- Exercise targeting, manoeuvre, weapon employment, shields, power, damage, repair, red alert, and Backfill under sustained pressure.
- Provide a deterministic enough environment for smoke, headless, balance, and regression testing while remaining a playable scenario.
- Make larger player hulls feel stronger but answer their power with additional opposition.
- End clearly rather than continuing after the combat problem is resolved.

## World setup

The world uses seed 475 and places a sun, an ecumenopolis, a gas giant, an ice moon, two asteroid fields, an Alliance ambient destroyer, and Starbase Alpha. The player begins at `[400, 0, 200]`. Named anchors surround the defended area and provide wave-release positions plus a starbase patrol circuit.

Starbase Alpha is both a physical combat entity and the defended mission target. Harrow forces approach the starbase, engage it within their authored objective range, and may redirect against the player as the tactical situation develops. The setting objects and asteroid fields create a spatial battle rather than an abstract wave arena.

## Wave structure

| Wave | Release time | Purpose |
|---:|---:|---|
| 1 | 0 seconds | Immediate opening contact and baseline threat |
| 2 | 45 seconds | Forces sustained operation beyond the opening volley |
| 3 | 90 seconds | Escalates composition and tests recovery between contacts |
| 4 | 135 seconds | Midpoint pressure |
| 5 | 180 seconds | Late-run attrition and repair demand |
| 6 | 225 seconds | Continued escalation |
| 7 | 270 seconds | Penultimate pressure |
| 8 | 315 seconds | Final release; clearing all hostiles can now end the scenario |

Each wave is tracked as a group and has a cleared handler. The scenario also tracks all hostiles as one group. Victory cannot fire early: the all-hostiles-destroyed trigger is gated by `waves_spawned >= 8`.

## Difficulty scaling

At scenario start the selected hull’s authored `power_rating` is copied into scenario state. Ratings currently read Courier 25, Destroyer 70, Cruiser 90, and Battleship 120. Ships rated at least 90 receive additional Destroyer opposition on alternating wave patterns; ships rated at least 100 receive additional Cruiser opposition on the other configured patterns. The exact spawn composition is canonical in the Rhai functions, but the design intent is stable: power rating changes the encounter, not the underlying behaviour of either fleet.

Power rating is a coarse content-scaling measure, not a promise of equal difficulty across crew sizes. A Battleship operated by two people may have more raw capacity and more workload than the same hull with nine. Playtest reporting should therefore record selected hull, connected players, occupied stations, and ratings.

## Information and pacing

The opening arms the scenario, adds the standing mission, and sends a brief. Timed reports announce each wave. Starbase hull reports fire at 75%, 50%, and 10%, making the defence state visible through both simulation and scenario acknowledgement. Enemy approach and red-alert behaviours are authored through doctrine and shared combat state rather than a separate wave-only AI.

The 45-second cadence is long enough to let a crew acquire, engage, and begin repairs, but short enough that uncleared forces accumulate. This creates a self-adjusting pressure curve: efficient crews reset between waves, while a struggling crew faces overlapping targets.

## Scenario TOML/Rhai contract

```toml
[global]
seed = 475
title = "world.combat_test.global.title"
description = "world.combat_test.global.description"
attacked_memory_secs = 18

[[available_ships]]
template_path = "assets/entities/alliance_destroyer.toml"
label = "world.combat_test.available_ships.0.label"

[player_spawn]
position = [400.0, 0.0, 200.0]

[script]
setup = '''
on_world_loaded("arm_the_scenario");
on_timer(0, "release_wave_1");
on_timer(45, "release_wave_2");
# ...waves 3–8 at 45-second intervals...
on_all_destroyed("hostiles", "on_raid_broken").when("counter(waves_spawned) >= 8");
on_destroyed("world.entity.starbase_alpha.name", "on_starbase_lost");
'''
```

This excerpt documents structure only. It must not be copied in place of the full authored spawn functions, objectives, messages, groups, power gates, and outcome handlers.

## Failure and recovery

Damage to the player ship is recoverable through shields, manoeuvre, power, and repair until terminal destruction. Damage to the starbase is persistent pressure and cannot be dismissed as scenario prose. Letting waves overlap is not an immediate script failure, but it increases the physical risk to both defended entities. The starbase hull callouts are warning thresholds, not alternate state.

## Playtest questions

- Does a new 2–4 player crew understand the defence objective and receive useful first tasks within one minute?
- Do wave spacing and composition create recovery decisions without long dead periods?
- Can crew members explain how Helm, Tactical, Shields/Engineering, and command choices affected starbase survival?
- Does power-rating scaling feel like appropriate opposition rather than punishment for choosing a larger hull?
- Are the Courier and Battleship both legible outside the recommended 2–4 range, and where does workload become unhelpful?
- Are defeat and victory clear to every participant, including reconnected clients?

## Success measures

- At least 80% of first-time test groups can state the objective without facilitator correction after the opening brief.
- Every recommended-crew session produces at least one meaningful cross-station report or request before wave two; this is observed qualitatively rather than inferred from button counts.
- Testers can distinguish why they won or lost: target handling, position, damage recovery, starbase protection, or accumulated waves.
- Seeded headless runs continue to produce two-sided damage, at least one kill, and a terminal result within the test budget.
- No offered hull creates an obvious no-threat or unavoidable-failure band under its intended crew/rating conditions.

## Open content decisions

- Target session length and final tuning for all hull/crew combinations need measured playtest results.
- Whether the ambient Alliance destroyer should materially assist, provide flavour, or be removed needs an explicit content decision.
- Difficulty choices beyond automatic ship-power scaling are not defined.
- Tutorial overlays and an explicit first-session variant need evaluation.
- The scenario currently tests combat well but does not aim to teach every console equally.

## Canonical sources

- `assets/worlds/combat_test.toml`
- `assets/scenarios.toml` and `assets/scenarios.demo.toml`
- Referenced Alliance, Harrow, station, planet, star, moon, and asteroid entity templates
- `tests/headless_runner.rs` and Combat Test script validation tests
