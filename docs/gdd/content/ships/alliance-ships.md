# Project Phoenix — Alliance Ship Content

| Field | Value |
|---|---|
| Document | GDD-CONTENT-ALLIANCE-SHIPS |
| Status | Working draft |
| Owner | Unassigned |
| Last updated | 2026-10-02 |
| Scope | Current Alliance player-capable hull family and its intended contrasts |
| Authority | Content overview. The four `assets/entities/alliance_*.toml` files are balance and runtime truth. |

The Alliance hull family provides the current player-ship ladder. It scales from a two-station courier to a nine-station battleship while preserving the same underlying systems and Backfill model. These are not four difficulty settings for one ship: each should change crew workload, manoeuvre planning, weapon employment, damage response, and the amount of opposition a scenario can support.

Related documents: [Ships and Ship Systems](../../systems/ships-and-systems.md), [Station Experiences](../../systems/station-experiences.md), [AI and Backfill](../../systems/ai-and-backfill.md), [Difficulty and Balance](../../foundation/difficulty-balance-playtesting.md), and [Thin Margin Setting](../../foundation/thin-margin-setting.md).

## Faction identity

Alliance ships follow the broadly optimistic, professional starship tradition of TNG/DS9/Voyager-era *Star Trek*. They belong to an exploratory and humanitarian polity whose decentralised crews exercise meaningful discretion while remaining answerable to public legitimacy. They are capable general-purpose vessels whose crews solve problems through coordination, technical competence, diplomacy, science, and force when necessary. Their player-facing design should feel legible and dependable rather than crude or specialised solely for destruction.

The Alliance faction is identified in authored content by its faction UUID, with player identity added only to the selected runtime instance. A world may spawn an Alliance hull as an NPC without it appearing or behaving as the player ship.

## Current fleet

| Hull | Role | Primary seats | Workload distinction |
|---|---|---:|---|
| Courier | Compact, mobile hull | 2 | Few seats bundle flight/combat/support; fewer people does not guarantee less workload. |
| Destroyer | Flexible combat/operations default | 4 | Mixed weapons and damageable tractor/dock/umbilical support the operational crisis. |
| Cruiser | Generalist with separated expertise | 6 | Science/Comms primary seats; broader weapons/arcs and supported rating rungs widen coordination. |
| Battleship | Capital-scale hull | 9 | Separate Repair/Sensors/Shields/Navigation/Power; broad systems and slower handling reward a larger crew. |

Numeric balance, movement, weapons, shields, sections and power remain in composed hull TOML. The former static stat table predated operational damage sections and Cruiser tuning; player-work contrasts are more useful here for comparison.

## Alliance Courier

The Courier is a compact two-player hull. Captain owns command plus the support and operations workload; Tactical owns weapons, helm, and sensors. Every capability remains a normal fine system, so this is a deliberate station bundling rather than a simplified simulation.

Its speed, acceleration, and turning authority are its primary defence. The single blaster and two shield arcs keep the tactical picture readable, while one repair team makes damage prioritisation sharp. Its low power rating deliberately stays below Combat Test’s bonus-spawn thresholds, making it the least intense way to run that scenario and the clearest current option for solo or duo crews.

The Courier should feel busy but comprehensible: fewer simultaneous firing controls and less shield geometry, counterbalanced by each player carrying several functions. It is a good onboarding hull only if the console flow makes that combined workload clear; “few stations” must not be mistaken for “few systems.”

## Alliance Destroyer

The Destroyer is the current general-purpose default and the sole selectable hull in Falling Skyway. Its four stations—Captain, Helm, Tactical, and Engineering—create recognisable bridge roles without requiring a large group. Sensors, navigation, comms, shields, power, and repair are folded into those stations through authored assignments and ratings.

Its mixed armament rewards coordination: an omni phaser supplies energy fire, port/starboard blasters make facing matter, and torpedo tubes add ammunition/loading decisions. Helm creates those opportunities. Shield facings, damageable internal/external Systems and limited repair teams expose the connected-ship model; live geometry, strength and timing remain in the hull.

The Destroyer's operational suite carries scan, tractor, dock and umbilical capabilities required by authored crises. Their damage sections add 90 hull to the earlier 300 baseline. That generalism supports its use as an operational reference rather than assuming every larger hull is preferable.

## Alliance Cruiser

The Cruiser is the flagship-scale generalist and carries the proper-name identity of AEV Phoenix in current content. Its six stations—Captain, Helm, Tactical, Science, Engineering, and Comms—separate information gathering and external relations from direct ship operation, creating more reasons for officer reports and handoffs.

Shield arcs, multiple tubes, fore/aft phaser coverage and a broader damage map create a different coordination problem from the Destroyer. Current handling and durability reflect the latest hull tuning rather than the former stat table. Its Combat Test power rating activates the first bonus reinforcement threshold. The damageable operational suite is part of the hull rather than an invisible scenario-only capability.

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

## Current readiness and Plan

Destroyer and Cruiser are in the curated demo. Cruiser tutorials/LODs/seeded balance and distinct supported rating rungs are implemented. **Plan:** world-authored detail-floor selectors in C2; Courier readiness in C3; Battleship readiness in C4; full/mixed-fleet closure in C5. See [Plan Baseline](../../future/roadmap-baseline.md). The [Dynasty player Cruiser](./dynasty-ships.md) is now a fifth playable hull, separate from this Alliance family.

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
- `assets/factions/alliance.toml`
