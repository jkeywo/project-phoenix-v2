# Increment 4 comparison: common observations and reports

Capture revision `d111f020` contains runtime `faf88fb6` and the final historical
client-fixture migration. [Raw receipt](observations-baseline.json) records source
and executable hashes, hardware, configuration and capture directories. Compare
with [Increment 3](tree-baseline.md) on the same machine and `measure` profile.

Both complete bounded scenario reports match Increment 3 byte for byte. Repeated
seeded runs reproduce identical reports, replay checkpoints agree, and three
performance continuations per world agree. The Cruiser matrix remains 20/20
victories with all 16 unchanged thresholds passing; merged results are identical.
No balance threshold or authored scenario value changed.

| World | Increment 3 p50 ms | Increment 4 p50 ms | Increment 3 p95 ms | Increment 4 p95 ms |
|---|---:|---:|---:|---:|
| Combat Test | 1.388–1.392 | 1.362–1.373 | 1.948–1.975 | 1.904–1.931 |
| Falling Skyway | 1.330–1.337 | 1.305–1.315 | 1.835–1.858 | 1.745–1.829 |

These ranges cover three captures after excluding the first 300 harness updates.
Timings are similar, with modestly lower measurements in this run; they do not
establish a speed improvement or release-budget acceptance. No competing Phoenix
simulation, build or browser acceptance was active during measurement.

Snapshot 40 and rules 0.12 intentionally extend continuation state. A changed
digest is expected from report metadata and recovery watermarks; agreement and
restore/replay are the checks, rather than equality with the previous rules.

Commands:

```powershell
node scripts/redesign-baseline.mjs --out .phoenix/redesign-baseline/2026-10-14-observations-measure --profile measure
node scripts/redesign-balance-baseline.mjs --capture .phoenix/redesign-baseline/2026-10-14-observations-measure/summary.json --out .phoenix/redesign-baseline/2026-10-14-observations-cruiser
```

The runner uses the six-seat Alliance Cruiser, seed 42, and 60 simulated seconds
for Combat Test and Falling Skyway. Large raw captures remain under ignored
`.phoenix/`; compact comparisons are committed here. Unrelated documentation
drafts remain present and are recorded as repository dirt; runtime sources were
clean and unchanged during both captures. Physical phones, native multi-monitor,
real networks, rendering performance and human workload remain separate
acceptance; see [the kit](../../acceptance/1562-common-observations.md).
