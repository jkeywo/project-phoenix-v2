# Project Phoenix — Harrow Ship Content

| Field | Value |
|---|---|
| Document | GDD-CONTENT-HARROW-SHIPS |
| Status | Working draft |
| Owner | Unassigned |
| Last updated | 2026-08-18 |
| Scope | Current Harrow NPC hull family, combat roles, and doctrine |
| Authority | Content overview. `assets/entities/ship_harrow_*.toml`, faction data, and scenario overrides are runtime truth. |

The Harrow fleet is House Harrow military content from the Imperium, the power later called the Dynasty. It is the current principal opposing ship family. Its content purpose is to create readable roles that force different crew responses, not to mirror the Alliance station experience or establish that every House Harrow vessel is always hostile. Harrow hulls still use the common ship, system, weapon, power, faction, and AI substrates, which means their behaviour remains inspectable and scenario authors can compose groups without a bespoke enemy framework.

Related documents: [Ships and Ship Systems](../../systems/ships-and-systems.md), [AI and Backfill](../../systems/ai-and-backfill.md), [Difficulty and Balance](../../foundation/difficulty-balance-playtesting.md), [Thin Margin Setting](../../foundation/thin-margin-setting.md), and [Combat Test](../scenarios/combat-test.md).

## Faction role

House Harrow associates rightful authority with decisive strength, victory and recognised glory. Its ships should present a coherent tactical language: disciplined movement, recognisable silhouettes and weapon signatures, and escalating roles from screening ships to capital artillery. Their faction relationship determines hostility; scenarios may change relationships or objectives through supported world state rather than hard-coding “Harrow always attacks” into weapon logic.

The shared setting establishes House Harrow’s high-level identity but not its exact Phoenix-era leadership, local command, visual language, naming, rules of engagement, or relationship with the Alliance. This document therefore treats current mechanical content as established, the House identity as inherited direction, and scenario-specific hostility as authored context.

## Current fleet

| Hull | Intended battlefield role | Hull HP | Mobility | Armament | Current authored topology |
|---|---|---:|---|---|---|
| Patrol | Durable patrol/screening vessel with broad beam pressure | 300 | Speed 12, yaw 0.196 | 2 phaser banks | 4 NPC-facing station bundles; common power and one shield arc |
| Destroyer | Fast attack craft and mobile ranged pressure | 200 | Speed 16, yaw 0.55 | 2 long-range blasters | 4 NPC-facing station bundles; common power and one shield arc |
| Cruiser | Heavier line combatant with torpedo threat | 300 | Speed 10, yaw 0.3 | 2 phaser banks, 2 torpedo tubes, 8 torpedoes | 4 NPC-facing station bundles; common power and one shield arc |
| Warhawk | Slow capital artillery threat | 400 | Speed 8, yaw 0.2 | 2 phaser banks, heavy bow artillery, 2 torpedo tubes, 12 torpedoes | 4 NPC-facing station bundles; common power and one shield arc |

Unlike current Alliance player ships, these hulls generally use a single hull-integrity pool and a compact Captain/Helm/Tactical/Engineering topology. That is an NPC content choice, not a limitation of the shared ship model. The table does not restate all arcs, cooldowns, effects, AI selectors, doctrine goals, or per-scenario overrides.

## Harrow Patrol

The Patrol is a persistent screening and enforcement hull. It is not especially fast or agile, but its twin phaser banks and 300-point hull let it remain in contact and apply steady pressure. In Falling Skyway, a Harrow Patrol template is repurposed mechanically as the Havelock enforcer/picket; this does not by itself make Havelock part of House Harrow. Its scenario role can escalate from coercive presence to combat, and its weapons may be disabled without requiring its destruction.

This establishes an important content principle: a hostile-capable hull can be an actor in an operational crisis rather than a disposable combat wave. Scenario state, comms, faction relations, weapons-hold decisions, and disablement can all matter.

## Harrow Destroyer

The Destroyer is the fastest and most agile current Harrow hull. Its two long-range blasters reward manoeuvring and target pressure rather than close beam trading. Low hull integrity makes it a threat that can be removed quickly if the crew coordinates target acquisition and fire.

In a mixed group it should pull Helm and Tactical attention away from the heavier centre. It works as a flanker, reinforcement, or tempo escalator. Scenario authors should avoid using speed merely to prolong pursuit; its movement should create a tactical question the crew can answer.

## Harrow Cruiser

The Cruiser combines twin phasers with two torpedo tubes and eight rounds. It is a line threat that introduces ammunition-bearing burst damage without reaching capital-ship durability. Its moderate movement makes it suitable for advancing on a defended objective or supporting faster ships.

The crew should be able to read the difference between a Cruiser and a Patrol through contact data, behaviour, weapon use, and silhouette. If those distinctions are not apparent in play, statistical differences alone are insufficient content differentiation.

## Harrow Warhawk

The Warhawk is the slow capital threat. Twin phasers, bow artillery with a 200-unit authored range, torpedoes, and the largest Harrow hull pool make it a standoff and facing problem. Its role is to anchor an assault and force the player crew to decide whether to close, flank, suppress escorts, protect an objective, or break contact.

The bow artillery must remain telegraphed and positionally meaningful. The ship’s slow speed and limited turning are counterplay, not incidental weakness. Its AI movement policy should preserve the gun line rather than thoughtlessly using impulse to overrun it.

## Fleet composition

| Composition use | Design effect |
|---|---|
| Patrol alone | Enforcement, picket, or simple beam-combat test |
| Destroyer pair | Mobile pressure and target-priority communication |
| Cruiser with Destroyers | Line threat plus flanking pressure |
| Warhawk with screen | Capital objective that cannot be solved by focusing one stationary target without consequence |
| Timed mixed waves | Escalating combat drill, as in Combat Test |

Mixed forces should be authored as objectives and groups, not given scenario-only combat rules. The same hull may receive local doctrine overrides, spawn anchors, faction state, and group membership from a world.

## Content rules

- Harrow roles must be readable through behaviour and presentation as well as stat blocks.
- NPCs use the same target, movement, weapons, damage, power, and command-admission substrates as other ships.
- Scenario authors may alter doctrine, faction relations, tags, and initial state through supported overrides, but repeated variants should become templates or fragments.
- Disablement, retreat, negotiation, or objective loss may be valid resolutions where the scenario supports them; destruction is not the only permitted use of a Harrow hull.
- Capital artillery and torpedo threats need visible or sensor-readable counterplay.
- Combat groups should create differentiated coordination problems rather than simply multiply HP.

## Open content decisions

- House Harrow’s Phoenix-era leadership, voice, visual language, naming conventions, relationship with the wider Imperium, and local posture toward the Alliance are not consolidated.
- Current Harrow hulls do not carry authored `power_rating` values, so generic scenario scaling cannot compare them through that field.
- The single-pool damage model makes Harrow internal degradation less expressive than Alliance player damage; whether this is intentional long-term needs ratification.
- The one-arc shield treatment across the current family needs a content-level statement or differentiation pass.
- Non-combat Harrow ships and neutral/ally uses are not defined.

## Acceptance criteria

- A crew can identify each hull’s battlefield role before reading a balance table.
- Each hull’s doctrine uses its movement and weapon geometry coherently.
- Mixed groups create target, manoeuvre, defence, and timing decisions across multiple stations.
- Harrow ships respond correctly to faction changes, disablement, objectives, and scenario-local doctrine.
- Combat Test can spawn and resolve every current role without scenario-specific weapon or AI code.
- Falling Skyway’s enforcer can be negotiated with, disabled, held at weapons hold, attacked, or destroyed through supported shared state.

## Canonical sources

- `assets/entities/ship_harrow_patrol.toml`
- `assets/entities/ship_harrow_destroyer.toml`
- `assets/entities/ship_harrow_cruiser.toml`
- `assets/entities/ship_harrow_warhawk.toml`
- `assets/factions/harrow.toml`
