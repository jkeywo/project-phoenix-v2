# Project Phoenix — Alliance Ship Content

| Field | Value |
|---|---|
| Document | GDD-CONTENT-ALLIANCE-SHIPS |
| Status | Working draft |
| Owner | Unassigned |
| Last updated | 2026-08-18 |
| Scope | Current Alliance player-capable hull family and its intended contrasts |
| Authority | Content overview. The four `assets/entities/alliance_*.toml` files are balance and runtime truth. |

The Alliance hull family provides the current player-ship ladder. It scales from a two-station courier to a nine-station battleship while preserving the same underlying systems and Backfill model. These are not four difficulty settings for one ship: each should change crew workload, manoeuvre planning, weapon employment, damage response, and the amount of opposition a scenario can support.

Related documents: [Ships and Ship Systems](../../systems/ships-and-systems.md), [Station Experiences](../../systems/station-experiences.md), [AI and Backfill](../../systems/ai-and-backfill.md), [Difficulty and Balance](../../foundation/difficulty-balance-playtesting.md), and [Thin Margin Setting](../../foundation/thin-margin-setting.md).

## Faction identity

Alliance ships follow the broadly optimistic, professional starship tradition of TNG/DS9/Voyager-era *Star Trek*. They belong to an exploratory and humanitarian polity whose decentralised crews exercise meaningful discretion while remaining answerable to public legitimacy. They are capable general-purpose vessels whose crews solve problems through coordination, technical competence, diplomacy, science, and force when necessary. Their player-facing design should feel legible and dependable rather than crude or specialised solely for destruction.

The Alliance faction is identified in authored content by its faction UUID, with player identity added only to the selected runtime instance. A world may spawn an Alliance hull as an NPC without it appearing or behaving as the player ship.

## Current fleet

| Hull | Role | Power rating | Hull HP | Stations | Mobility | Armament | Shields | Repair |
|---|---|---:|---:|---:|---|---|---|---:|
| Courier | Compact, fast, low-workload ship for solo or duo play | 25 | 200 across 7 sections | 2 | Speed 22, yaw 0.9 | 1 forward blaster | 2 arcs, 40 HP base per facing | 1 team |
| Destroyer | Flexible combat/operations ship and current default | 70 | 300 across 11 sections | 4 | Speed 18, yaw 0.6 | 1 omni phaser, 2 blasters, 2 torpedo tubes, 12 torpedoes | 2 arcs, 80 HP base per facing | 2 teams |
| Cruiser | Flagship-style generalist with broad officer separation | 90 | 500 across 12 sections | 6 | Speed 14, yaw 0.4 | 2 phaser banks, 3 torpedo tubes, 6 torpedoes | 4 arcs, 100 HP base per facing | 2 teams |
| Battleship | Large-crew capital ship with maximum durability and firepower | 120 | 800 across 13 sections | 9 | Speed 10, yaw 0.27 | 2 phaser banks, heavy forward blaster, 5 torpedo tubes, 30 torpedoes | 5 arcs, 140 HP base per facing | 2 teams |

Values in this table are a readable snapshot, not a second balance authority. Cooldowns, arcs, projectile behaviour, power curves, damage-section mapping, AI policy, markers, presentation, audio, and detailed ratings remain in the TOML.

## Alliance Courier

The Courier is a compact two-player hull. Captain owns command plus the support and operations workload; Tactical owns weapons, helm, and sensors. Every capability remains a normal fine system, so this is a deliberate station bundling rather than a simplified simulation.

Its speed, acceleration, and turning authority are its primary defence. The single blaster and two shield arcs keep the tactical picture readable, while one repair team makes damage prioritisation sharp. Its low power rating deliberately stays below Combat Test’s bonus-spawn thresholds, making it the least intense way to run that scenario and the clearest current option for solo or duo crews.

The Courier should feel busy but comprehensible: fewer simultaneous firing controls and less shield geometry, counterbalanced by each player carrying several functions. It is a good onboarding hull only if the console flow makes that combined workload clear; “few stations” must not be mistaken for “few systems.”

## Alliance Destroyer

The Destroyer is the current general-purpose default and the sole selectable hull in Falling Skyway. Its four stations—Captain, Helm, Tactical, and Engineering—create recognisable bridge roles without requiring a large group. Sensors, navigation, comms, shields, power, and repair are folded into those stations through authored assignments and ratings.

Its mixed armament rewards coordination: an omni phaser supplies reliable energy fire, port and starboard blasters make facing matter, and two torpedo tubes add ammunition and loading decisions. Strong speed and turning let Helm actively shape those opportunities. Two shield arcs remain readable during combat, while eleven damage sections and two repair teams expose the connected-ship model.

The Destroyer’s operational suite makes it suitable beyond combat. It can carry scan and external-operation capabilities required by authored crises. That generalism is why scenario balance should use it as a baseline rather than assuming every larger hull is simply preferable.

## Alliance Cruiser

The Cruiser is the flagship-scale generalist and carries the proper-name identity of AEV Phoenix in current content. Its six stations—Captain, Helm, Tactical, Science, Engineering, and Comms—separate information gathering and external relations from direct ship operation, creating more reasons for officer reports and handoffs.

Four shield arcs, three torpedo tubes, fore/aft phaser coverage, a larger damage map, and a full 90-capacity reactor create a broader coordination problem than the Destroyer. It is tougher but less agile, so positional mistakes are slower to correct. In Combat Test its power rating activates the first bonus reinforcement threshold.

The Cruiser is the clearest current expression of the core bridge fantasy: enough specialist stations for information and authority to be distributed, without the capital-ship scale of the Battleship. It should be a reference hull when assessing whether station UIs produce useful conversation rather than parallel solo play.

## Alliance Battleship

The Battleship is a nine-station capital ship: Captain, Helm, Tactical, Repair, Sensors, Shields, Navigation, Power, and Comms. It exposes nearly every major function as its own player responsibility and therefore represents the current maximum conventional crew size.

It is slow and deliberate, but carries the fleet’s greatest authored durability, five shield arcs, five torpedo tubes, thirty torpedoes, two phaser banks, and a heavy forward blaster. Its artillery movement policy and forward weapon make facing and standoff discipline important. The power rating of 120 activates all Combat Test bonus reinforcement gates.

The Battleship should not invalidate smaller hulls. Its strength is purchased with more demanding coordination, slower repositioning, more arcs and weapon systems to manage, and a scenario workload that may exceed a small crew even with automation. It is most appropriate where the scenario supplies enough pressure and players to make its system breadth meaningful.

## Station scaling

| Hull | Authored station bundle |
|---|---|
| Courier | Captain; Tactical |
| Destroyer | Captain; Helm; Tactical; Engineering |
| Cruiser | Captain; Helm; Tactical; Science; Engineering; Comms |
| Battleship | Captain; Helm; Tactical; Repair; Sensors; Shields; Navigation; Power; Comms |

The fleet should preserve a consistent mental model across these layouts. A system may move between bundles, but it remains the same system and uses the same authoritative commands. Ratings must make every hull viable below its station count without removing the reasons to add another human.

## Content rules

- Alliance ships remain credible at combat, exploration, rescue, negotiation support, and infrastructure operations where their authored capabilities permit them.
- Larger ships gain capacity and separation of duties but also gain coordination and positioning costs.
- The selected player instance receives player identity at spawn; templates do not author it.
- All shipped Alliance player hulls explicitly author station topology, ratings, fine systems, damage sections, power groups, and AI declarations.
- Shared AI fragments may provide common policy, but hull files retain hull-owned capability facts and tune their own role.
- Scenario offerings must identify a recommended crew and account for the hull power/role rather than relying only on nominal class.

## Open content decisions

- The final in-fiction class names, registry conventions, and visual-language guide are not yet consolidated in this GDD.
- The relationship between the Alliance as a faction label and the wider Thin Margin political setting needs a setting bible.
- Falling Skyway is currently Destroyer-only; whether other Alliance hulls receive authored operational balance is undecided.
- The station-rating names and workload targets should be playtested consistently across all four hulls.
- The Courier’s onboarding role needs explicit tutorial and playtest confirmation.

## Acceptance criteria

- Each hull has a distinct, observable operating role in movement, survival, armament, and station workload.
- Each hull remains operable from zero humans through its authored station maximum.
- Adding a human reduces AI-held responsibility and creates meaningful control, not mere duplicate visibility.
- Combat Test scales opposition as authored and remains winnable/legible for the intended crew and hull.
- The Destroyer supports every physical and informational operation required by Falling Skyway.
- Fleet-wide UI vocabulary and system behaviour remain consistent when systems are assigned to different stations.

## Canonical sources

- `assets/entities/alliance_courier.toml`
- `assets/entities/alliance_destroyer.toml`
- `assets/entities/alliance_cruiser.toml`
- `assets/entities/alliance_battleship.toml`
- `assets/factions/federation.toml`
