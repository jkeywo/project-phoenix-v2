# Cruiser evaluation — 27 September 2026

## Result and final tuning

The ratified **200-duel** matrix passed: **86 Alliance wins / 114 Dynasty wins
(43% / 57%)**, with **zero draws, timeouts or failed runs**. The accepted band
is 40–60% across the whole matrix. This is automated numerical evidence;
the crew usability checks remain unobserved.

Only `assets/entities/dynasty_player_cruiser.toml` changes: paid strike damage
is **1.15×** for both phaser banks and all three torpedo tubes, reduced from
1.5×. Reserve capacity remains 90, enable threshold 60, charging rate 2 units
per allocated level, beam cost 12 and torpedo cost 8. The ordinary charging,
approach, attack and recovery policies remain intact. The Alliance cruiser and
existing Harrow NPC cruiser content are unchanged.

| Authored strike multiplier | Alliance wins | Dynasty wins | Duel rate A/D | Team A/D/timeouts |
| --- | ---: | ---: | --- | --- |
| Baseline 1.5 | 44 | 156 | 22% / 78% | 10 / 8 / 2 |
| Candidate 1.2 | 76 | 124 | 38% / 62% | 14 / 2 / 4 |
| **Final 1.15** | **86** | **114** | **43% / 57%** | **16 / 2 / 2** |

All three batches contain the same 200 duels and 20 team fights. No tuning
candidate is omitted. The two-run smoke batch is retained but excluded from
evaluation. No duel needed a half-win contribution in any batch.

## Condition and team observations

Each condition contains 20 fixed seeds, each with original and mirrored starts.

| Final condition | Alliance wins | Dynasty wins |
| --- | ---: | ---: |
| Close head-on, separation 40 | 28 | 12 |
| Medium head-on, separation 80 | 32 | 8 |
| Long head-on, separation 160 | 6 | 34 |
| Alliance initially facing away, separation 80 | 4 | 36 |
| Dynasty initially facing away, separation 80 | 16 | 24 |

The cruisers retain distinct conditional strengths: Dynasty dominates the long
start and the Alliance-facing-away start; Alliance wins most close and medium
head-on duels. The aggregate criterion does not establish parity within every
condition. Final duel duration is 18.8–496.1 simulation seconds, median 32.025.

The reference **2v2** follow-up uses seeds 1547001–1547010, both placements, with
ordinary Backfill on every Station. Its 16 Alliance wins, 2 Dynasty wins and
2 timeouts give half-win-adjusted shares of **85% / 15%**. This is a strong
Alliance advantage in this team setup, not team parity. Both cruisers win team
fights, and the raw Dynasty phase telemetry contains charge, approach, attack
and recovery. The result shows that the duel advantage does not simply carry
over to the four-ship engagement; it does not isolate the cause of that change.
Team duration is 32.6667–600 seconds, median 46.60835. Seed 1547005 times out
in both placements; neither timeout is hidden as a win.

These condition differences and team asymmetry are concrete topics for the
[crew acceptance kit](../acceptance/1549-cruiser-elimination.md). No claim is
made about crew usability, physical devices, rendered layout or human tactics.

## Provenance and reproduction

The complete [evidence archive](dynasty-evidence-2026-09-27.zip) retains every
generated world, raw AAR, process log, per-run result, manifest and summary for
the smoke, baseline and both candidates. The final
[manifest](dynasty-final-manifest.json) and [summary](dynasty-final-summary.json)
are also readable separately.

- Source revision: `df1986c13b53501825493ece583ee2fd67a0ce96`, plus the exact
  authored content patch in each candidate manifest. Rust source patch: empty.
- Executable: `phoenix-headless.exe`, native Windows x86-64, Cargo dev profile
  (`opt-level = 1`, optimized dependencies), `headless` feature. The same bytes
  ran all batches; no elapsed-time measurement or performance claim is made.
- Executable SHA-256: `10b17cdcf57ec44d8695ac9c2bdcba346fdb9c4e96d1b9a26ef32a40d11fd48d`.
- Final runner SHA-256: `21fbd227baefe18cc9f0eb05fe5ec02df9f3e2e3423fd78618a845abfcdcc8fb`.
- Final Dynasty hull SHA-256: `b97f7222235b7d2a76b9dff1ca9132f10ad9be911030cd103720a882c490bb88`.
- Reference world SHA-256: `6912245f19b6b4bcf10d8003aa31d22d67c302b5855828c8340e841e057400a7`.
- Archive SHA-256: `106a644dfa869e32d3d2c92a28d384c255fa26489644167480753197c63f126b`.

All runs use the explicit seeds/conditions in the [evaluation rule](dynasty-evaluation.md),
fixed 1/60-second simulation steps, a 600-second limit and concurrency 2.
Manifests pin all authored content hashes, generated-world hashes, exact
arguments and report hashes. To reproduce the final candidate from its source
revision, apply the manifest's content patch and use the recorded runner
version (the current script includes that correction). Run into a fresh folder:

```powershell
node scripts/balance-dynasty.mjs --binary target/debug/phoenix-headless.exe --out .balance-dynasty/reproduction --concurrency 2
```

The baseline exposed one reporting correction: the generic AAR calls a
low-activity unfinished fight `draw`. The original baseline summary therefore
marks team runs 209/210 as failed classification, although both processes
completed successfully at 600 seconds. Their untouched raw reports establish
two timeouts under the scenario rule. The table above states that correction;
the archive preserves the original failure records. Both later candidates use
the corrected, independently reviewed classifier. An early unfinished report
or contradictory terminal flags still fails; process failures never become
half-wins. Condition summaries use the whole-matrix `complete/parity` fields,
so their 40-run subsets are not separate acceptance gates.

## Focused validation

- `npx vitest run tests/client/balance-dynasty.test.js`: **5/5**, including fresh
  output refusal, matrix mirroring, result flags, timeout-limit validation and
  half-win/failed-batch accounting.
- Final `uv run pasm validate`, `uv run pasm scan` and
  `uv run pasm traceability`: **all passed**; validation reports 948 entities
  and `Status: OK`. Changed wiki source references and index links: 115 checked,
  none missing. Independent source and result-interpretation review: **PASS**.
- Before tuning, `cargo test --features headless --lib --test cruiser_elimination -- crew_wreck competitive_world --test-threads=1`:
  **2/2** wreck repair/retarget regressions and **2/2** scenario outcomes.
- Against the final authored tuning, the freshly linked
  `cruiser_elimination-568d456e4e7ae7b6.exe --test-threads=1`: **2/2**. It verifies
  natural full and mixed Backfill resolution, real Human Station occupancy,
  private team Objectives, fixed hulls/headings, both team victories,
  simultaneous elimination and retained destroyed-crew identity.
  Test executable SHA-256: `f3db05d6a4e5de4dd3602c1c6bdc9fd62c96eb7bf68302657e72dadaee949fdd`.

The test executable and headless binary were built from the same final Rust
source; subsequent changes are authored tuning and runner/documentation only.
Per-condition differences, the two unresolved team seeds and unobserved crew
acceptance remain explicit limitations. Final integration gates belong to the
integration task.
