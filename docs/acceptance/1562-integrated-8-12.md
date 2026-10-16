# PRD 1562 — bounded verification record for increments 8–12

**Draft evidence snapshot, 16 October 2026. Verification remains open.**

This records the current targeted verification pass, not a release, an all-green
suite or acceptance of all five increments. The broad native selection failed;
later fixture corrections and narrower passes do not replace that result.
Browser/native-device acceptance, performance/balance comparison and station
activity evidence are separate obligations. Root owns sequential native
execution and the eventual combined integration gates.

## Tested source boundaries

| Source revision | Evidence boundary |
|---|---|
| `34aa3e04c559640983ee441bdcf8a5670a228e9e` | Renderer-free Simulation prerequisite compile and the broad selected native owner run. |
| `d8af35dc5d7937acae315e5bd91ce03204b6a70d` | Production panel correction: retain the edited recipe's authored position. Affected three client tests passed. |
| `450836ac1db45893ecfce0f4fdaafe2aa5820111` | Adds the whole-host production continuation proof. This was the clean pinned PASM source. Adding a test is not a native pass. |
| `9493bc5a605e749814d3f0759de78dffcdde095e` | Workshop mount/current authoring journey corrections, updated manifest fixture and five additional mechanism edits in the render journey. This is the compiled source for the completed seven-target host batch. |
| Later test-only commits listed below | Not covered by the original `34aa3e04` owner executable or the `9493bc5a` host executable. Their source reviews are not execution evidence. |

The run log files cited below are local `.phoenix/` artifacts, not committed
release artifacts. Keep each result associated with its stated source and
configuration; do not relabel a previous executable as the current main.

## Completed checks and observed failures

| Check | Actual result | Local evidence |
|---|---|---|
| Renderer-free Simulation prerequisite compile at `34aa3e04` | **PASS**, 46.54 seconds. This establishes compilation, not gameplay acceptance. | `.phoenix/verify-8-12-prerequisite-compile.log` |
| Selected native owners at `34aa3e04` | **FAIL: 1250 passed, 55 failed.** Content Preparation 12/0; Gameplay 521/0; World 11/0; Simulation 706/55. Build 9m05s; Simulation tests 322.17s. | `.phoenix/verify-8-12-native-owners.log` |
| Initial client mechanism/authoring selection | **FAIL: 223 passed, 2 failed**, 19 files, 13.61 seconds. Both failures were in `workshop-production-panel.test.js`. | `.phoenix/verify-8-12-client-authoring.log` |
| Affected production-panel retry | **PASS: 3/3**, 9.62 seconds in the isolated source correction before integration as `d8af35dc`. Existing work/target, undo/refusal and primary-output assertions remain; recipe-order and untouched-recipe checks were strengthened. | Worker command: `npx vitest run tests/client/workshop-production-panel.test.js`; source donor `3c8eadce`, integrated as `d8af35dc`. |
| Initial broader Workshop authoring selection | **FAIL: 60 passed, 51 failed**, eight files. The mounted authoring surface used the wrong operation adapter; this was an actual API integration failure. | `.phoenix/verify-8-12-current-workshop-fixture.log` |
| Corrected affected Workshop authoring surfaces | **PASS: 53/53**, two files, 67.73 seconds, after the API correction and installation of the locked JS dependencies. This is a two-file retry, not a rerun of all 111 earlier tests. | `.phoenix/verify-8-12-workshop-authoring-corrected-installed.log` |
| Render journey discovery | **Discovery only:** one test in one file. It confirms test registration, not browser execution. | `.phoenix/verify-8-12-workshop-smoke-discovery-installed.log` |
| Fabrication final-frontier diagnostic at `8bc01405` | **FAIL: 0 passed, 1 failed**, build 5m27s; execution 10.71s. All 360 snapshot/digest comparisons still matched; batch 2 stopped at 9.355852/10 work seconds, reserve 0, no thermal stop and no allocated intake. This identifies power allocation as the unfinished-work cause; an admitted Engineering priority correction is prepared and awaits execution, not a pass. | `.phoenix/verify-12-production-frontier-diagnostic.log`; diagnostic donor `77db78ba`, integrated as `6a832bf1`. |
| Shield whole-host continuation at runtime `895e234d` (proof donor `67d581cb`) | **PASS: 1/1**, build 50.95s; execution 7.49s. All 360 paid/current-credit continuation comparisons and malformed-candidate no-write checks passed. | `.phoenix/verify-9-shield-whole-host.log` |
| Clean pinned PASM scan at `450836ac` | **PASS**, 4154 files, 8685 dependencies, 18 packages. | `.phoenix/verify-8-12-pinned-pasm.log` |
| Clean pinned PASM traceability at `450836ac` | **PASS**, 246 rows. | `.phoenix/verify-8-12-pinned-pasm.log` |
| Clean pinned PASM validation at `450836ac` | **Status: OK**, 1202 entities, 630 non-error findings: 629 warnings and one concern; no error findings. It does not validate later changes or establish playable acceptance. | `.phoenix/verify-8-12-pinned-pasm.log`, clean `verify-8-12-pasm` worktree. |

The first corrected Workshop retry stopped during tool startup because
`vitest/config` was unavailable; it is not counted as an executed pass or a new
product failure. The installed-dependency retry above is the executed result.

## Native commands and outstanding retries

The broad native owner result used this exact command at `34aa3e04`:

```text
cargo test -p phoenix-simulation -p phoenix-sim-gameplay -p phoenix-sim-world -p phoenix-content-preparation --lib --no-default-features --no-fail-fast -- energy power shield interior damage repair logistics production recovery ports sim_digest::tests
```

The seven-target host batch uses this exact command, compiled at `9493bc5a`:

```text
cargo test -p project-phoenix --features headless,host --no-fail-fast --test physical_damage_routes --test logistics_continuation --test recovery_continuation --test facility_supply_continuation --test fitting_facility_continuation --test activity_foundation_continuation --test production_continuation -- --test-threads=1 --nocapture
```

The seven-target batch finished **FAIL: 10 passed, 8 failed** after its 8m10s
build. This is the actual run at `9493bc5a`; later fixture/source corrections
have not been substituted for these results. The local log is
`.phoenix/verify-8-12-whole-host.log`.

| Host target | Executed result |
|---|---|
| `activity_foundation_continuation` | **3 passed, 0 failed**, 33.85s. |
| `facility_supply_continuation` | **2 passed, 0 failed**, 28.68s. |
| `fitting_facility_continuation` | **0 passed, 2 failed**, 9.08s. |
| `logistics_continuation` | **1 passed, 2 failed**, 24.28s. |
| `physical_damage_routes` | **3 passed, 0 failed**, 31.39s. |
| `production_continuation` | **1 passed, 1 failed**, 20.59s. Malformed-candidate no-write passed. Paid second-batch live/restored full capture and digest matched through all 360 steps, but final fabrication completion failed; this is not a continuation-test pass. |
| `recovery_continuation` | **0 passed, 3 failed**, 20.95s. |

Later integrated source corrections requiring native retry include:

| Main commit | Source correction; execution remains separately required |
|---|---|
| `de21ea5f24c8518adaccc655753d70364ddde64b` | Sparse shield owner fixtures now supply the actual clock and crew owners. |
| `78f3bfd2d1c725e5c6c5b12aef2f84e142cab6eb` | Craft fixture supplies the actual TacticalRadar publisher inputs. |
| `e839a8d0699e1f1a965ad4655f49dd743adf40c5` | Sensors-jamming protection fixture honors priority reacquisition cadence. |
| `923b6a3c656bfd8605820f2ccbd981be6b0a51f6` | Port/actor continuation fixtures retain normal publications/Backfill owners; medical transfer probes paid whole goods plus fractional work. |
| `67d581cbf75fcedd512ef593d06b2fe0ec1ed4f2` | Adds paid shield recovery/whole-host continuation evidence source, absent from the seven-target command above. |
| `599da96c7e171ff53af045c3d5c7131769239ee3` | Config/spawner fixture layouts and impact profiles; exact source-derived arc AI coverage and battery-only reactor classification. |

The admission fixture's Repair kind correction was independently reviewed on
root's working tree after both TOML literals and supported-kind arrays matched
`REPAIR_KIND`. All authority assertions remained. A dirty source review has no
commit pin or native pass until the integrator records one.

## Capability and acceptance boundaries

| Increment | Evidence addressed in this pass | Still required before its verified exit |
|---|---|---|
| 8 — Power, batteries, coolant and heat | Selected finite energy/Power owners, authoring controls and actual-hardware AI pins. | Resolve owner failures and rerun affected owners; actual UI operation/delegation, continuation evidence and representative resource/balance/activity measurements. |
| 9 — Shield energy and protection | Selected shield owners; new paid-recovery/continuation test source; shield authoring edit in the render journey. | Native retry of the corrected owners; the added shield host proof passed separately at its stated revision; actual redistribution/recovery UI and AI journeys, balance/activity evidence. |
| 10 — Decks, compartments and routes | Selected interior owners and explicit accepted physical fixture corrections; physical-route host target passed at the stated source. | Browser layout authoring, preview/Test and supported hull/team journeys. |
| 11 — Component damage and restoration | Selected damage/repair owners; strict impact/layout schemas retained; physical route/damage/repair continuation targets selected. | Resolve relevant owner failures; prove damage plus a real restoration path, malformed-state refusal and usable Human/AI repair presentation on the integrated source. |
| 12 — Logistics and production | Selected finite stores/deliveries/ports/production owners; production authoring retry; logistics, recovery, facility supply and new fabrication host targets selected. | Resolve the observed Logistics, recovery and final fabrication-completion failures and rerun their actual continuation/no-write targets; actual authored replenishment/production operation, preview/Test and representative workload/resource evidence. |

The corrected `tests/smoke/workshop-resources.render.spec.js` retains the real
Workshop providers/WASM validator, inherited-data refusal/materialisation,
undo/redo, export/reimport, preview and disposable Test flow. It now also edits
shield energy, compartment HP, a damage remedy, Logistics handling and a
fabrication recipe, and reloads those saved values. Its positive pack uses the
current content epoch and complete inherited capacities. Source review and one
registered test are recorded; **the real browser/render journey remains
pending**, as do native display/device and 2/4/6/8-seat Human acceptance.

No current Cruiser balance batch, before/after performance comparison or
station-activity playtest result is supplied by these logs. No networking,
multiplayer, whole-workspace CI or final pre-push gate pass is implied. Increments
8–12 and PRD 1562 remain open until their applicable playable exits and the
integrated checks are demonstrated.

## Follow-up at `fee6b25e` / `964b4a98`

The corrected renderer-free Simulation selection at `9a3b2f89` completed
**141 passed, 6 failed** in 42.34 seconds. The remaining failures concern the
discrete-shot fixture, engine fixture, shield recovery notification and three
strict restore fixtures. Source-reviewed fixture corrections are integrated;
their native retry remains required. Separate retained-beam diagnostics at
`5cb35b1a` completed **2 passed, 2 failed** in 28.95 seconds. The server fixture
has empty combat-bank definitions despite actual paid emission; its correction
must use accepted equipment rather than add a runtime fallback.

The four-target whole-host retry compiled at `fee6b25e` in 2m46s and completed
**7 passed, 3 failed**:

- Fitting facility: 1/1, 123.39s. Real paid facility refit and continuation pass;
  intact salvage still lacks an eligible current equipment projection.
- Logistics: 3/0, 24.94s, including finite six-round conservation, full JSON
  continuation and malformed-state refusal before writes.
- Production: 1/1, 16.61s. The priority correction provides current flow/intake,
  reserve and paid fraction. The positive now fails at the fresh restored
  checkpoint's transient emitted-work counter before its first update; investigate
  that boundary without dropping final completion or full-state comparisons.
- Recovery: 2/1, 24.41s. Finite-source competition and malformed-state refusal
  pass. Full continuation diverges at step51 in `collisions[0].sim_t`
  (live2.033333374, resumed1.533333364). This is an open runtime clock issue,
  not an acceptable ignored field.

The locked current client build passes **121 tests across21 files** at
`5cb35b1a` in 22.72s. A client-only Chromium run after Repair fixture correction
`964b4a98` passes **43/43** in 25.4s: Power, Shields, Repair, Engineering
responsiveness and text-scale workflows. These use projected fixture data, not
an authoritative WASM host. The earlier six-file selection was stopped after
reproducing Repair's empty keyed System fixture and remains failed/incomplete;
its actual host-dependent Station popup case was not executed. That case stays
in the matching-host queue. Source corrections and narrower passes do not
establish a green original combined native selection or a closed increment.

Logs:
`.phoenix/verify-8-12-corrected-owner-retry.log`,
`.phoenix/verify-8-12-beam-frontier.log`,
`.phoenix/verify-8-12-corrected-host-retry.log`,
`.phoenix/verify-8-12-current-client-retry.log`,
`.phoenix/verify-8-12-browser-dom.log`,
`.phoenix/verify-8-12-browser-dom-corrected.log`.

The coverage ledger also passes at `fee6b25e`:216 catalogue entries across25
increments. No current all-green integration gate, increment closure or push is
claimed.
