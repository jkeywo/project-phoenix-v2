# Increment 2 comparison — 14 October 2026

Runtime `efac36b7a0a721a30feccab26cf2f7764a763ef1`, compared with Increment 1
at `23b43ec0b69f006a2267d5d809031206ccd23c3c`. The [compact receipt](composition-baseline.json)
records source/executable hashes, hardware, individual measurements and checks.

## Gameplay and continuity

The six-seat Cruiser ran Combat Test and Falling Skyway, seed 42, for 60 simulated
seconds. Both complete reports match Increment 1 exactly, including ship state,
AI, activity, narrative, objectives, damage, outcomes and message counts. Each
passed byte-identical repeats, record/replay checkpoints and three-run
continuation agreement. Both remained InProgress at tick 3600; these are bounded
samples, not whole-mission victories.

The ratified Cruiser matrix passed **20/20 victories and all 16 unchanged
thresholds**. Every opponent's outcome counts, TTK and damage margin match
Increment 1 exactly. No balance values or thresholds changed.

The deterministic fold intentionally changes because it now includes all active
Dock channels and undock clearance. Replay agrees within rules 0.10; older saves
require their original compatible version.

## Measured cost

Same Intel Core Ultra 9 275HX and optimized `measure` profile, default plus
headless features. Three captures per world, excluding 300 warm-up updates and
retaining 3,301 samples each. No other build/browser/test job ran during captures.
Runtime sources were clean; unrelated documentation drafts remained present.

| World | Increment 1 p50 (ms) | Increment 2 p50 (ms) | Increment 1 p95 (ms) | Increment 2 p95 (ms) |
|---|---:|---:|---:|---:|
| Combat Test | 1.461–1.569 | 1.377–1.395 | 2.030–2.236 | 1.877–1.900 |
| Falling Skyway | 1.346–1.367 | 1.328–1.361 | 1.817–1.977 | 1.793–1.910 |

These ranges overlap or improve on the preceding diagnostic. This is not a
causal claim that composition improves performance, nor release-budget evidence.
No performance budget changed. Browser/native rendering, real-network performance
and human workload remain outside this headless comparison.

## Reproduction and limitations

Commands and hashed logs are in [composition-validation.json](composition-validation.json).
Raw reports, replay, captures and the pinned executable remain under ignored
`.phoenix/redesign-baseline/2026-10-14-composition-measure/`; balance evidence is
under `.phoenix/redesign-baseline/2026-10-14-composition-cruiser/`.
Physical/device acceptance and the known landscape-phone diagnostics overlap
remain pending in [the acceptance record](../../acceptance/1562-composable-systems.md).
