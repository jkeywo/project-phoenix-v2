# Increment 5 comparison: shared jobs and team execution

Capture revision `e858616a` contains the final migrated runtime and
integration fixtures. [Raw receipt](jobs-baseline.json) records source and executable
hashes, hardware, configuration and capture directories. Compare with
[Increment 4](observations-baseline.md) on the same machine and `measure` profile.

Both complete bounded scenario reports match Increment 4 byte for byte. Repeated
seeded runs reproduce identical reports, replay checkpoints agree, and three
performance continuations per world agree. The unchanged Cruiser matrix passes
20/20 victories and all 16 thresholds; merged results are identical. No balance
threshold or authored scenario value changed.

| World | Increment 4 p50 ms | Increment 5 p50 ms | Increment 4 p95 ms | Increment 5 p95 ms |
|---|---:|---:|---:|---:|
| Combat Test | 1.362–1.373 | 1.420–1.451 | 1.904–1.931 | 2.008–2.042 |
| Falling Skyway | 1.305–1.315 | 1.375–1.401 | 1.745–1.829 | 1.820–2.012 |

These ranges cover three captures after excluding the first 300 harness updates.
Median tick times measured approximately 4–7% higher, and the p95 ranges also
rose, with more variation in Falling Skyway. These captures do not establish a
causal allocation of that cost. They are same-machine diagnostics, not release-budget or rendering-performance
acceptance. No competing Phoenix simulation, build or browser acceptance was
active during measurement.

Snapshot 41 and rules 0.13 intentionally replace duplicated repair continuation
with canonical jobs and workflow memory. Changed digest values are expected;
deterministic agreement and restore/replay are the checks, rather than equality
with the previous rules.

Commands:

```powershell
node scripts/redesign-baseline.mjs --out .phoenix/redesign-baseline/2026-10-14-jobs-measure --profile measure
node scripts/redesign-balance-baseline.mjs --capture .phoenix/redesign-baseline/2026-10-14-jobs-measure/summary.json --out .phoenix/redesign-baseline/2026-10-14-jobs-cruiser
```

The runner uses the six-seat Alliance Cruiser, seed 42, and 60 simulated seconds
for Combat Test and Falling Skyway. Large raw captures remain under ignored
.phoenix/; compact comparisons are committed here. Unrelated documentation
drafts remain present and are recorded as repository dirt; runtime sources and
revision were clean/unchanged during both captures. Physical phones, native
multi-monitor displays, real networks and human workload remain separate
acceptance; see [the kit](../../acceptance/1562-shared-jobs.md).
