# Increment 0 baseline — 13 October 2026

The baseline is recorded against runtime revision
`8d8143e9899747591cde4ee6c292e3c89859bf8d`, before gameplay migration.
Runtime/content sources are clean; the checkout contains the existing design and
GDD drafts plus this increment's tooling/design records. The source fingerprint
and executable hash are retained in [baseline.json](./baseline.json).

Execution is tracked by [PRD #1562](https://github.com/jkeywo/project-phoenix-v2/issues/1562).

## Gameplay and determinism

The Alliance Cruiser ran Combat Test and Falling Skyway with seed 42 for 60
simulated seconds each. Both produced byte-identical repeated reports and passed
record/replay checkpoint verification. Each world's three performance runs also
ended on the same tick and continuation digest.

These are bounded samples: both missions were still `InProgress` at tick 3600.
Combat Test's report classified the cutoff as `timeout`; Falling Skyway classified
it as `draw`. Neither result is a whole-mission success claim.

The existing ratified Cruiser matrix also passed: 20 victories from 20 runs,
with all 16 thresholds unchanged and passing.

| Opponent | Runs won | Median TTK, seconds | Mean damage margin |
|---|---:|---:|---:|
| Harrow Destroyer | 5/5 | 4.78 | 163.00 |
| Harrow Cruiser | 5/5 | 13.82 | 283.80 |
| Harrow Patrol | 5/5 | 8.58 | 310.20 |
| Harrow Warhawk | 5/5 | 15.70 | 47.24 |

The balance capture reuses `scripts/balance-runs.cruiser.toml` and the existing
batch runner's argument construction, aggregation and threshold evaluation. It
uses the exact provenance-pinned baseline executable rather than an older binary
left in `target/release`.

## Performance reference

Optimised `measure` profile, `headless` feature with default features retained,
on an Intel Core Ultra 9 275HX. Each repetition contains 3,301 measured harness
updates after excluding the first 300 updates. Raw samples retain the full run.

| World | Repetitions | p50 update range, ms | p95 update range, ms |
|---|---:|---:|---:|
| Combat Test | 3 | 1.224–1.328 | 1.791–1.943 |
| Falling Skyway | 3 | 1.157–1.166 | 1.553–1.558 |

These measurements support same-machine migration comparisons. They do not
establish native/browser rendering performance, real-network acceptance, human
Station workload or release-budget compliance. No performance budget was adopted
or changed. Future comparisons must use matching profile, features and workload.

## Existing checks

- 490 Console/client tests passed across seven suites.
- 58 renderer-independent shipped-content checks passed, including every shipped
  world composing, hull parsing, command consumers, Backfill and Workshop validation.
- Six snapshot-continuation tests passed for power, controls, AI memory, frequency
  and Navigation state.
- Six native cross-target-probe tests passed, including the committed digest
  ledger, pacing independence and deliberate divergence detection. This is native
  execution of the probe tests; no new browser/WASM run is claimed.
- PASM validation, repository scan and traceability passed. The pre-existing
  finding count was 534; baseline reconciliation introduced no validation errors.
- Ledger coverage and generated HTML/filter checks passed.

Commands and counts are retained in [validation.json](./validation.json).

## Reproduction and artifacts

```powershell
node scripts/redesign-ledger.mjs --check
node scripts/redesign-baseline.mjs --out .phoenix/redesign-baseline/<new-run> --profile measure
node scripts/redesign-balance-baseline.mjs --capture .phoenix/redesign-baseline/<new-run>/summary.json --out .phoenix/redesign-baseline/<new-cruiser-run>
```

Raw evidence remains under ignored directories:

- `.phoenix/redesign-baseline/2026-10-13-measure/`: seeded reports, replay artifacts,
  performance/phase captures, continuation receipts, Cargo artifact output and a
  copy of the verified baseline executable.
- `.phoenix/redesign-baseline/2026-10-13-cruiser/`: all 20 duel reports, config and
  executable receipts, merged outcomes and unchanged threshold results.

Compact results and artifact hashes are committed-source candidates in this
directory. Large binaries and raw captures are intentionally local. Reproduction
requires the recorded runtime/content revision; incompatible inputs must be
reported rather than treated as equivalent evidence.
