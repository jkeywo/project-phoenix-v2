# Project Phoenix — Difficulty, Balance, and Playtesting Framework

| Field | Value |
|---|---|
| Document | GDD-DIFFICULTY-BALANCE |
| Status | Working draft; numerical targets remain content-specific until measured |
| Owner | Unassigned |
| Last updated | 2026-08-18 |
| Scope | Balance dimensions, evidence, hull/scenario scaling, crew workload, accessibility, metrics, and review process |
| Authority | Design framework. Authored assets, deterministic reports, tests, and recorded playtests remain evidence truth. |

Phoenix balance is the quality of a crew’s decision space, not merely a 50% win rate. A balanced scenario gives the intended crew and hull enough information, time, capability, and recovery to make consequential choices, while preserving genuine risk and differentiated roles. Combat numbers, workload, scenario pacing, AI doctrine, and accessibility all contribute but should not be collapsed into one score.

Related documents: [Game Design Overview](./overview.md), [Station Experiences](../systems/station-experiences.md), [AI and Backfill](../systems/ai-and-backfill.md), [Combat Test](../content/scenarios/combat-test.md), [Falling Skyway](../content/scenarios/falling-skyway.md), [Alliance Ships](../content/ships/alliance-ships.md), and [Onboarding, Tutorials, and Accessibility](./onboarding-accessibility.md).

Mechanic-level playtest questions are collected in the nine pages under [Movement and Helm](../mechanics/movement.md), with peer links from each mechanic to its principal dependencies.

## Balance dimensions

| Dimension | Core question | Typical evidence |
|---|---|---|
| Mechanical | Do movement, weapons, shields, power, and repair create viable roles and counterplay? | Seeded duels, state/event reports, focused playtests |
| Scenario | Can intended approaches reach coherent outcomes under the authored clocks and state? | Whole-scenario headless runs and branch playtests |
| Crew workload | Does each occupied station have meaningful work without sustained overload? | Observation by player count, hull, station, and rating |
| Information | Can the crew discover causes and make informed decisions without omniscience? | Debrief recall, missed-state analysis, parity audit |
| Automation | Does Backfill preserve viability while leaving human value? | Zero/small-crew runs and handoff tests |
| Accessibility | Can workload/input/sensory needs be adjusted without hiding rules or trivialising another player’s role? | Alternative-path and disabled-player testing |
| Content differentiation | Do hulls, factions, scenarios, and roles feel distinct through play rather than labels? | Comparative tests and qualitative reports |
| Performance | Can supported devices run the required simulation and presentation? | Performance captures; tracked separately from game balance |

## Evidence ladder

No single layer is sufficient. Use the cheapest reliable evidence first, then advance questions that require human judgement.

1. **Schema and invariant tests** prove that authored values and relationships are valid: systems resolve, budgets fit, AI policies exist, arcs and groups are legal, and outcome paths are structurally reachable.
2. **Pure mechanic tests** establish deterministic rules and edge cases without claiming a fun result.
3. **Seeded headless duels** compare hulls and doctrine across fixed matchups and seeds, reporting win/loss/draw, time to kill, damage margin, ammunition, and relevant events.
4. **Whole-scenario headless runs** establish progress, two-sided interaction, terminal outcomes, branch invariants, and absence of stalls.
5. **Automated browser/smoke tests** establish real client/host routing, lifecycle, and basic visual output, not tactical fidelity.
6. **Human playtests** establish comprehension, workload, communication, tension, agency, comfort, and memorability.
7. **Field/event observation** establishes setup friction, device diversity, facilitator needs, and repeat play under realistic conditions.

Deterministic simulation is load-bearing for comparison. Fixed seeds make an A/B meaningful; they do not prove that five seeds represent the full game. Record the exact content digest, hulls, doctrine, scenario, seed set, duration, build, and report schema with every numerical result.

## Hull balance

A hull is balanced as a role, not against every other hull in a vacuum. Compare movement, geometry, effective damage, ammunition, shield arcs, repair capacity, system degradation, power reserve, operational capabilities, station count, and AI doctrine together.

The current `power_rating` is a coarse scenario-scaling signal on Alliance hulls: Courier 25, Destroyer 70, Cruiser 90, Battleship 120. Combat Test uses rating thresholds to add opposition. Power rating does not measure crew workload or guarantee equivalence; Harrow hulls currently omit it. Do not use the field as a universal combat-point system without a separate ratified design.

Seeded duel goals should define an acceptable band rather than demand exact parity. A role may deliberately win one matchup and lose another if counterplay and fleet composition remain healthy. Investigate step changes caused by discrete volley counts, shield breaks, reload cycles, target selection, or damage-tier crossings rather than tuning only means.

## Scenario balance

Each scenario specifies offered hulls, possible and recommended crew, target duration, recovery opportunities, terminal states, and which outcomes should be common, rare, or impossible. Direct scenarios may target a narrow victory band. Operational crises may instead require that several defensible resolution patterns remain reachable and that costs are legible.

Combat Test balances through wave composition, fixed release timing, accumulation pressure, protected-objective durability, and selected-ship power gates. Falling Skyway balances through deadlines, infrastructure thresholds, operation durations, traffic movement, information acquisition, actor disposition, commitments, and deliberately insufficient transfer supply.

An authored clock must include the real time needed to read, speak, navigate console depth, reconnect, and recover from an understandable mistake. Headless completion time is not human completion time.

## Crew and station balance

The possible range is zero through the selected ship’s station maximum, but the recommended range identifies where the authored workload is expected to be strongest. Test at minimum practical, recommended low/high, and station maximum. Record station assignment and rating; “four players” is incomplete when the same four may occupy different seats or automation levels.

For each station, observe:

- meaningful decisions per phase rather than raw clicks;
- periods of avoidable idleness or sustained overload;
- information waiting on another officer;
- unwanted automation and work the player wished to delegate;
- urgent actions hidden behind planning/detail interactions;
- whether bundling several families creates coherent work or context switching;
- whether another human adds communication and capability rather than duplicate visibility.

Automation is a workload lever with a performance cost, not a shame state. A lower rating should preserve participation. A highly automated crew may be less adaptable than specialists without becoming non-viable.

## Difficulty levers

Preferred levers are explicit and authored:

- force composition, arrival direction, spacing, objectives, and reserves;
- scenario deadlines, capacity, decay, interruption, and recovery windows;
- NPC doctrine, selector priorities, risk thresholds, and reaction cadence within the shared timing contract;
- starting damage, ammunition, position, information quality, and relationships;
- offered hulls and scenario-specific capability/detail floors;
- optional shared assists such as a deadline multiplier or clearer prediction where authored.

Avoid hidden enemy stat multipliers, private AI knowledge, different physics, unannounced rubber-banding, or input-reading counters. If a difficulty variant changes hull/entity TOML, doctrine, or world data, it should be inspectable and included in the content digest.

## Accessibility versus difficulty

An accessibility option changes how a player receives information or supplies an input; a difficulty option changes the authoritative problem. Larger text, reduced motion, discrete Helm controls, captions, and non-colour cues are not easier difficulty. A shared deadline multiplier or reduced enemy force is.

Station rating sits between these categories: it changes workload and control allocation while preserving the same ship simulation. Present it as an operating preference, and report it in playtest evidence. Never withhold accessibility alternatives to protect a difficulty target.

## Quantitative measures

Use measures that illuminate a design question:

| Question | Useful measures |
|---|---|
| Is a duel one-sided? | Win rate, draw rate, TTK distribution, surviving hull/shield margin, damage dealt by source |
| Is ammunition meaningful? | Rounds granted/loaded/launched/hit, remaining mission threat, kills per round |
| Is power pressure real? | Time above sustainable total, reserve minimum, lock count/duration, allocation changes |
| Is repair consequential? | Dispatch delay, travel/on-site time, unrepaired critical duration, systems restored/destroyed |
| Is a scenario paced? | Time to first decision/contact, overlap between pressures, idle gaps, terminal time, branch reached |
| Is a crew overloaded? | Missed alerts, unhandled requests, reported overload, time-critical action delay—not clicks alone |
| Is onboarding working? | Join-to-action time, facilitator interventions, role explanation, control errors |

An average without its distribution can hide step functions and bad seeds. A numerical pass should retain representative outliers for diagnosis.

## Qualitative playtest protocol

Record build/content digest, scenario, hull, player count, station assignments, ratings, prior experience, device/access needs, facilitator interventions, start/end time, outcome, and major technical interruptions. Observe without coaching unless the session is explicitly facilitated.

After play, ask every player:

1. What was your responsibility?
2. What did you know only because another officer told you?
3. Which decision most changed the outcome?
4. What state or consequence was confusing?
5. When were you idle, overloaded, or fighting automation?
6. Did Backfill behave helpfully and legibly?
7. What moment would you retell, and would you try another role or approach?

The facilitator separately records causal truth from the report/log. A player’s incorrect explanation is valuable evidence about presentation, not a response to be corrected out of the dataset.

## Tuning process

1. State the design question and intended player experience.
2. Identify the smallest authoritative knobs that can affect it and the cross-system consequences of each.
3. Capture a repeatable baseline with fixed seeds or a documented playtest setup.
4. Change one conceptual variable or a clearly coupled set.
5. Re-run the same evidence and inspect distributions/events, not only outcome.
6. Run adjacent matchups, crew sizes, or branches to detect transfer damage.
7. Record why the chosen value sits on its side of any step function.
8. Ratify through human play when the question concerns feel, communication, comprehension, or fairness.

Do not tune around a bug, invalid content, divergent human/AI path, nondeterministic schedule, or stale report schema. Fix the model first, then re-baseline.

## Release readiness

A hull/scenario slice is ready when its authoring validates; contextual tutorials and presentation are complete for the offered hulls; representative seeded evidence is recorded; critical whole-scenario paths terminate; recommended crew and expected duration have observed support; accessibility alternatives for primary interactions work; and known out-of-band matchups or branches are documented rather than silently excluded.

## Acceptance criteria

- Every balance claim names its question, evidence layer, content version, and limits.
- Hull comparisons include doctrine and operating state rather than reading stats in isolation.
- Scenario recommendations record hull, crew, station, rating, and real elapsed time.
- Accessibility alternatives are tested separately from authoritative difficulty changes.
- Fixed-seed A/B results are reproducible and broader seed runs are used before claiming population-level stability.
- Human playtests establish communication, causal clarity, workload, agency, and memorable outcomes.
- No tuning introduces a separate human/AI rule or hard-coded scenario exception where authored state can express the difference.

## Canonical sources

- `src/headless/`, `src/core/balance.rs`, and `scripts/balance-runs.mjs`.
- `scripts/balance-runs.demo.toml` and related matchup matrices.
- `tests/headless_runner.rs` and whole-scenario tests.
- `pasm/spec/architecture/headless-balance-telemetry.yaml` and the roadmap balance/readiness records.
- [Combat Test](../content/scenarios/combat-test.md) and [Falling Skyway](../content/scenarios/falling-skyway.md) for content-specific questions.
