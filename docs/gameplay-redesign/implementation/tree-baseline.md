# Increment 3 comparison — 14 October 2026

Runtime `e78fb19486e9434c4acad77e8407316a2fd7341e`, compared with Increment 2
at `efac36b7a0a721a30feccab26cf2f7764a763ef1`. The [compact receipt](tree-baseline.json)
records source/executable hashes, hardware, individual measurements and checks.

## Gameplay and continuity

The six-seat Cruiser ran Combat Test and Falling Skyway, seed 42, for 60 simulated
seconds. Both complete reports match Increment 2 byte for byte, including ship
state, AI, activity, narrative, objectives, damage, outcomes and message counts.
Both pass byte-identical repeats, record/replay checkpoints and three-capture
continuation agreement. These are bounded samples ending InProgress at tick 3600.

The ratified Cruiser matrix passes **20/20 victories and all 16 unchanged
thresholds**. Its merged outcomes, TTK and damage margins match Increment 2 exactly.
No tuning values or thresholds changed.

The deterministic fold intentionally changes because it now includes individual
System tree state, memory and call continuation. Replay agrees within rules 0.11;
snapshot 39 explicitly rejects older private-state gaps.

## Measured cost

Same Intel Core Ultra 9 275HX and optimized `measure` profile, default plus
headless features. Three captures per world exclude 300 warm-up updates and retain
3,301 samples each. No other Phoenix build, test or browser job ran during the
captures. Runtime sources were clean; unrelated documentation drafts remained.

| World | Increment 2 p50 (ms) | Increment 3 p50 (ms) | Increment 2 p95 (ms) | Increment 3 p95 (ms) |
|---|---:|---:|---:|---:|
| Combat Test | 1.377–1.395 | 1.388–1.392 | 1.877–1.900 | 1.948–1.975 |
| Falling Skyway | 1.328–1.361 | 1.330–1.337 | 1.793–1.910 | 1.835–1.858 |

Median ranges overlap. Combat Test's measured p95 is modestly higher, by roughly
0.05–0.10 ms; Falling Skyway's ranges overlap. This diagnostic does not establish
causality or release-budget acceptance. No performance budget changed. Browser/
native rendering, remote-network performance and human workload are separate.

## Reproduction and limitations

Commands and hashed logs are in [tree-validation.json](tree-validation.json).
Raw reports, replay, captures and the pinned executable remain under ignored
`.phoenix/redesign-baseline/2026-10-14-tree-measure/`; balance evidence is under
`.phoenix/redesign-baseline/2026-10-14-tree-cruiser/`.
Physical/device acceptance remains pending in [the acceptance record](../../acceptance/1562-ai-state-trees.md).
