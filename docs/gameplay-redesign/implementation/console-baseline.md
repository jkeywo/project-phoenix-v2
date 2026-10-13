# Increment 1 comparison — 13 October 2026

Runtime revision `23b43ec0b69f006a2267d5d809031206ccd23c3c`, compared with
increment 0 at `8d8143e9899747591cde4ee6c292e3c89859bf8d`.
[The compact receipt](console-baseline.json) records source/executable hashes,
matching hardware/features and the measured results.

## Gameplay and continuation

The six-seat Alliance Cruiser ran Combat Test and Falling Skyway, seed 42,
for 60 simulated seconds. Each passed byte-identical repeat reports,
record/replay checkpoints and three-run continuation agreement. Both remained
InProgress at tick 3600; these are bounded samples, not whole-mission victories.

Ship state, damage, AI reports, narrative, objectives, outcomes and message
counts match increment 0 exactly. Falling Skyway's Station activity also
matches. Combat Test's chart drops two Navigation AI entries and one Command
AI entry: those detached human-seeking Consoles have no Station host in the
all-Backfill run. Their commands still execute, but are no longer attributed
to fictitious crew seats. All remaining Station counters match.

The ratified Cruiser matrix passed **20/20 victories and all 16 unchanged
thresholds**. Each opponent's win/loss counts, TTK distribution and damage
margin match increment 0 exactly.

| Opponent | Wins | Median TTK (seconds) | Mean damage margin |
|---|---:|---:|---:|
| Harrow Destroyer | 5/5 | 4.7833 | 163.00 |
| Harrow Cruiser | 5/5 | 13.8167 | 283.80 |
| Harrow Patrol | 5/5 | 8.5833 | 310.20 |
| Harrow Warhawk | 5/5 | 15.7000 | 47.24 |

## Measured cost

Same Intel Core Ultra 9 275HX, optimized `measure` profile, default plus
`headless` features. Three captures per world, each excluding the first 300
updates and retaining 3,301 samples. No competing build/test job ran during
capture. The checkout had documentation drafts; runtime sources were clean.

| World | Increment 0 p50 range (ms) | Increment 1 p50 range (ms) | Increment 0 p95 range (ms) | Increment 1 p95 range (ms) |
|---|---:|---:|---:|---:|
| Combat Test | 1.224–1.328 | 1.461–1.569 | 1.791–1.943 | 2.030–2.236 |
| Falling Skyway | 1.157–1.166 | 1.346–1.367 | 1.553–1.558 | 1.817–1.977 |

Mean update cost increases by about 0.20 ms in Combat Test and 0.21 ms in
Falling Skyway. Publication aggregation increases, but several other phases
also rise; this sample does not establish a causal breakdown. The cost is
recorded rather than hidden by a changed threshold. No performance budget or
balance threshold was modified.

These are same-machine headless diagnostics. They do not establish browser or
native rendering performance, remote-network performance, human workload or
release-budget compliance. Physical/device acceptance remains pending in
[the acceptance record](../../acceptance/1562-explicit-consoles.md).

## Reproduction

Use the existing `redesign-baseline.mjs` and `redesign-balance-baseline.mjs`
commands in [console-validation.json](console-validation.json). Raw reports,
replay files, captures and the pinned executable remain under ignored
`.phoenix/redesign-baseline/2026-10-13-console-measure/` and the balance
receipts under `.phoenix/redesign-baseline/2026-10-13-console-cruiser/`.
