# Increment 6 comparison: stores, transfers and service links

Final capture revision `47324314` contains the stock migration, authoring and
Preview prerequisites. [Raw receipt](resources-baseline.json) records source and
executable hashes, hardware, configuration and capture directories. Compare with
[Increment 5](jobs-baseline.md) on the same machine and `measure` profile.

Both complete bounded scenario reports match Increment 5 byte for byte. Seeded
repeats, replay checkpoints and three performance continuations per world agree.
The unchanged Cruiser matrix passes 20/20 victories and all 16 thresholds; merged
results are identical. No balance threshold or scenario tuning changed.

| World | Increment 5 p50 ms | Increment 6 p50 ms | Increment 5 p95 ms | Increment 6 p95 ms |
|---|---:|---:|---:|---:|
| Combat Test | 1.420–1.451 | 1.472–1.486 | 2.008–2.042 | 2.088–2.093 |
| Falling Skyway | 1.375–1.401 | 1.374–1.400 | 1.820–2.012 | 1.833–1.922 |

Ranges cover three captures after excluding the first 300 harness updates.
Combat Test median/p95 tick times measured slightly higher; Falling Skyway ranges
are similar. Architecture refactors between `e858616a` and the pre-increment base
`dc42aa20` are part of this comparison too, so these measurements cannot attribute
cost solely to resources. They are same-machine diagnostics, not release-budget
or rendering-performance acceptance. No competing Phoenix simulation, build or
browser acceptance was active during measurement.

Snapshot 42/rules 0.14 replace scalar stock continuation with canonical stocks,
loading claims and fractional flow carry. Digest changes are expected;
deterministic agreement and replay/restore are the checks, rather than equality
with the earlier rules. The two conservation corrections—partial-volley claim
release and carry retention—receive explicit regression coverage.

Commands:

```powershell
node scripts/redesign-baseline.mjs --out .phoenix/redesign-baseline/2026-10-14-resources-final-measure --profile measure
node scripts/redesign-balance-baseline.mjs --capture .phoenix/redesign-baseline/2026-10-14-resources-final-measure/summary.json --out .phoenix/redesign-baseline/2026-10-14-resources-final-cruiser
```

The runner uses the six-seat Alliance Cruiser, seed 42 and 60 simulated seconds
for Combat Test and Falling Skyway. The earlier capture before Preview fixes is
retained separately under `.phoenix/redesign-baseline/2026-10-14-resources-measure`;
the final comparison uses the distinct final directory above. Large raw captures
remain ignored; compact receipts are committed here. Runtime sources and revision
were clean/unchanged across final capture and matrix; repository dirt consisted
of documentation evidence being prepared. Physical phones, native multi-monitor
displays, real networks and human workload remain separate acceptance; see
[the kit](../../acceptance/1562-resource-stores.md).
