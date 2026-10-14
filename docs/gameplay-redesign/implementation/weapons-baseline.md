# Increment 7 comparison: shared mounts and configurable weapons

The [capture receipt](weapons-baseline.json) pins revision `e848e2f3`, runtime
source bytes and the copied executable. That revision adds approved future
planning documents to tested runtime revision `5e7264d5`; the runtime is unchanged.
Compare with [Increment 6](resources-baseline.md) on the same machine, features
and `measure` profile.

Both complete bounded scenario reports match Increment 6 byte for byte. Seeded
repeats, replay checkpoints and three performance continuations per world agree;
their terminal digests also match Increment 6. The unchanged Cruiser matrix
passes 20/20 victories and all 16 thresholds, with byte-identical merged results.
No scenario values or balance thresholds changed.

| World | Increment 6 p50 ms | Increment 7 p50 ms | Increment 6 p95 ms | Increment 7 p95 ms |
|---|---:|---:|---:|---:|
| Combat Test | 1.472–1.486 | 1.414–1.454 | 2.088–2.093 | 1.958–2.094 |
| Falling Skyway | 1.374–1.400 | 1.443–1.450 | 1.833–1.922 | 1.931–1.990 |

Ranges cover three captures after excluding the first 300 harness updates.
Combat Test medians measured slightly lower, with overlapping p95 ranges;
Falling Skyway medians and p95 measured slightly higher. These same-machine
diagnostics do not establish a release-budget result or isolate mechanism cost
from ordinary timing variation. No competing Phoenix simulation, Cargo build or
browser acceptance ran during these measurements. Existing budgets remain intact.

No authoritative lifecycle state was added. Snapshot 42, simulation rules 0.14
and protocol 6 remain unchanged. Existing continuation tests cover live beams,
fractional damage, cooldowns, bolt volleys, tube claims and pending bursts.

The initial capture ran in the main checkout before Increment 8 started. Under
the later user-approved workflow, the Cruiser matrix then ran from the isolated
validation worktree pinned to `e848e2f3`, using that capture's copied executable.
The worktree's runtime source hash was verified equal to the capture before the
handoff. Its 193 initial checkout byte differences were newline-only and were
matched to the pinned source without changing committed content. Matrix inputs
therefore stayed fixed while implementation continued on main.

```powershell
node scripts/redesign-baseline.mjs --out .phoenix/redesign-baseline/2026-10-14-weapons-measure --profile measure
# From the isolated worktree pinned to e848e2f3:
node scripts/redesign-balance-baseline.mjs --capture C:/Coding/project-phoenix-v2/.phoenix/redesign-baseline/2026-10-14-weapons-measure/summary.json --out .phoenix/redesign-baseline/2026-10-14-weapons-cruiser
```

Large raw artifacts are retained under the shared ignored
`.phoenix/redesign-baseline/2026-10-14-weapons-measure` and
`2026-10-14-weapons-cruiser` directories. The matrix artifacts were copied from
the pinned validation worktree after completion. The receipt records their
hashes and provenance. Physical phones, native monitors, real networks and human
attention remain separate acceptance in [the kit](../../acceptance/1562-configurable-weapons.md).
