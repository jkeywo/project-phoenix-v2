# PRD 1562 — bounded verification record for increments 8–12

**Targeted verification completed with failures, 16 October 2026. Work paused; automated exits 8–12 remain open.**

This records the current targeted verification pass, not a release, an all-green
suite or acceptance of all five increments. The original broad native selection failed;
later fixture corrections and narrower passes do not replace that result.
Browser/native-device acceptance, performance/balance comparison and station
activity evidence are separate obligations. Root owns sequential native
execution and the eventual combined integration gates.

Latest recorded boundary: selected native owners **1618/1618**, current whole-host
checks **19/19**, selected energy resume **2/2**, matching-host browser **15/15**
and real Workshop render **3/3** pass at the separately pinned sources below.
The current same-machine baseline at `9bd7bab6` passes all six determinism checks,
but the unchanged Cruiser matrix **fails 11/16 checks**: zero wins, seven draws
and thirteen timeouts. Headless tick cost also increases materially. These
failures prevent an automated exit; no balance threshold was changed.

The requested verification runs and a bounded combat diagnostic are collected.
Work is now paused under the user's latest instruction. Increments 13–24 remain
parked, with their work preserved. Formal closure, Human/device acceptance and
pre-push integration gates are not claimed. The compact machine-readable
[comparison receipt](../gameplay-redesign/implementation/verification-8-12.json)
contains source hashes, timing samples, activity comparisons and failed checks.

## Tested source boundaries

| Source revision | Evidence boundary |
|---|---|
| `34aa3e04c559640983ee441bdcf8a5670a228e9e` | Renderer-free Simulation prerequisite compile and the broad selected native owner run. |
| `d8af35dc5d7937acae315e5bd91ce03204b6a70d` | Production panel correction: retain the edited recipe's authored position. Affected three client tests passed. |
| `450836ac1db45893ecfce0f4fdaafe2aa5820111` | Adds the whole-host production continuation proof. This was the clean pinned PASM source. Adding a test is not a native pass. |
| `9493bc5a605e749814d3f0759de78dffcdde095e` | Workshop mount/current authoring journey corrections, updated manifest fixture and five additional mechanism edits in the render journey. This is the compiled source for the completed seven-target host batch. |
| `4ceb7d8ac0ac7ddb2ffd680bfca49c0ff063a529` | Logical collision-attribution rules 0.74; expanded Simulation owner retry and the later clean pinned PASM checks. |
| `dd1490aeeb253536f1f34f5b03dd3de5580ce6e4` | Completed affected host retry with intact-salvage failure diagnostics; before its held-Helm correction. |
| `37c165a4df599b10316d133e402e8b3491b7cc5d` | Completed combined four-owner selection: 1616 passed, 2 failed. |
| `2cdc979b` | Current-runtime wiki reference corrections; documentation does not change the tested runtime. |
| `8ac62cce` | Integrates the unarmed-wrapper and Tactical-rating test corrections; the later selected owner union passes 1618/1618 at rules 0.74. |
| `5180261f` | Held-Helm salvage retry reaches Recovery admission, then exposes the real empty motion-plan restore mismatch at step zero; 0/1 passed. |
| `00cf53c3` | Rules 0.74 Repair and shield fresh-process continuation evidence; later browser-only changes do not relabel the preceding native executable. |
| `e027e56882a0e9b808920bd55a4857ba7aba542a` | Rules 0.75 empty-plan fix, new regression/capture checks, current eight-target 19/19 host batch, selected energy resume 2/2 and pinned PASM checks. |
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
| 9 — Shield energy and protection | Corrected shield owner/restore tests pass in the later selected run; paid shield whole-host proof passes separately at its stated revision; shield authoring edit is in the render journey. | Current matching-host redistribution/recovery UI and AI journeys, balance/activity evidence. |
| 10 — Decks, compartments and routes | Selected interior owners and explicit accepted physical fixture corrections; physical-route host target passed at the stated source. | Browser layout authoring, preview/Test and supported hull/team journeys. |
| 11 — Component damage and restoration | Selected damage/repair owners; strict impact/layout schemas retained; physical route/damage/repair continuation targets selected. | Resolve relevant owner failures; prove damage plus a real restoration path, malformed-state refusal and usable Human/AI repair presentation on the integrated source. |
| 12 — Logistics and production | Later Logistics 3/3, Recovery 3/3 and Production 2/2 host retries pass at their stated revisions, including finite goods, real work, full JSON continuation and malformed-state refusal. | Complete the outstanding intact-equipment recovery journey and current matching authoring/preview/Test operation; representative workload/resource and performance/balance evidence. |

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

## Follow-up through `8ac62cce` — verification still open

The expanded Simulation retry at `4ceb7d8a` completed **425 passed, 15 failed**
in 507.60s. Three sparse restore fixtures still omitted the accepted Cruiser's
physical turret owners; additional weapon-fixture failures were also exposed.
The actual accepted mount profile/state and required clocks are now constructed
in those fixtures (`8b4e5e72`), including a valid full restore before the deliberate
missing-team-owner negative. Restore guards were retained. The failures remain
recorded in `.phoenix/verify-8-12-canonical-clock-owner-retry.log`.

The subsequent combined owner command at `37c165a4` completed **FAIL: 1616 passed,
2 failed**, after a 4m 12s build. It includes the original mechanism filters plus
the affected weapon owners; this remains targeted, renderer-free evidence,
not a whole-workspace or pre-push gate pass.

| Native owner | Executed result |
|---|---|
| Content Preparation | **12 passed, 0 failed**, 3.66s. |
| Gameplay | **521 passed, 0 failed**, 0.44s. |
| World | **11 passed, 0 failed**, 0.01s. |
| Simulation | **1072 passed, 2 failed**, 674.68s. The shield/actor full-restore and missing-team-owner checks now pass. |

Evidence: `.phoenix/verify-8-12-final-native-owners.log`. Its two actual failures
and subsequent source corrections are:

| Failure | Integrated correction; execution status at this cutoff |
|---|---|
| `absent_equipment_never_borrows_another_ships_or_global_weapons` incorrectly required the accepted civilian Hauler's entire WeaponsConsole declaration to be absent. | `feb5a748` checks the real AI-only wrapper separately from hardware: empty phaser/blaster lists, no weapon System kinds, no Player phaser owner, empty NPC phaser owner and absent NPC torpedo/blaster owners. Existing refusal, ammunition and inactive-beam checks remain. **Source-reviewed; targeted native retry running, result pending.** |
| `strike_blaster_charge_and_cancel_are_free_and_emission_pays_exactly_once` failed its charging/progress assertion. | `8ac62cce` applies the actual Tactical rating before charging and checks normal Human ownership/admission while preserving finite energy, cancellation and exact emission-payment assertions. **Source-reviewed; targeted native retry running, result pending.** |

The affected host batch compiled at `dd1490ae` completed **FAIL: 8 passed,
1 failed**, after a 3m 23s build. It does not include the held-Helm correction.

| Host target | Executed result |
|---|---|
| `fitting_facility_continuation` | **1 passed, 1 failed**, 104.78s. Actual powered allied facility refit and continuation pass; intact recovery fails before admission because the current offered equipment list is empty. |
| `production_continuation` | **2 passed, 0 failed**, 16.61s. The second fabrication batch completes with real work and every full JSON360 state/digest comparison; malformed owner/WIP/claim refusal also passes. |
| `recovery_continuation` | **3 passed, 0 failed**, 20.31s. Paid finite-goods continuation, finite-source competition and malformed-state no-write pass after the logical collision timestamp correction. |
| Selected `snapshot_resume` checks | **2 passed, 0 failed**, 19.70s; 89 filtered out. Power next-tick/modifier continuation over a different bootstrap and Dynasty strike/Backfill continuation pass through the complete schedule. |

Evidence: `.phoenix/verify-8-12-canonical-host-retry.log`. The separate previous
Logistics retry remains **3/3 PASS**, 24.94s at `fee6b25e`, rather than being counted
again in this four-target host batch.

Intact salvage's actual diagnostic shows receiver position
`[-5.1542025,0,0.05501263]` and source position `[7.364777,0,0]`: distance 12.52
exceeds the unchanged eight-unit tool range. The source remains neutralized;
its installed beam retains a Healthy condition with nonzero physical incident
dose, and the Recovery Console receives the earned current Science selection.
The failure is not evidence for granting a guessed contact or extending tool
range. Integrated source correction `0931723a` applies the same single disabling
incident earlier, claims Human Helm through ordinary admission, sends normal
zero thrust/steering/lateral controls and checks physical plus permitted measured
range until Recovery admission. Both fresh resume hosts establish matching crew
ownership. **That correction is source-reviewed but has not executed at this
record's cutoff; intact salvage remains open.**

The latest pinned PASM checks at `4ceb7d8a` all pass:

- Validation: **Status: OK**, 1202 entities, 630 non-error findings
  (629 warnings and one concern), `.phoenix/verify-8-12-final-pasm.log`.
- Scan: **Validation: OK**, 4157 files, 8694 observed dependencies, 18 Cargo packages,
  `.phoenix/verify-8-12-final-scan.log`.
- Traceability: **Validation: OK**, 246 rows,
  `.phoenix/verify-8-12-final-traceability.log`.

These pinned checks do not silently cover later changes. Current compatibility
remains protocol 45 / snapshot 88 / rules 0.74; those version identifiers are not
acceptance results. Wiki reference corrections at `2cdc979b` record current code
and do not establish another runtime pass.

The previously recorded client 121/121, component Chromium 43/43 and affected
Workshop 53/53 results retain their own source/configuration boundaries. Current
matching-host Workshop/render, reconnect/multiplayer, native-device and Human
acceptance, representative performance/balance and station activity remain open.
The two owner retries and intact-salvage retry must be recorded before describing
those corrections as verified. No all-green integrated gate, increment closure,
push or completion through 24 is claimed. Finish the agreed 8–12 verification and
necessary corrections, commit the evidence, then pause; later work remains parked.


## Native follow-up at `e027e568` - targeted selections green

The owner union at `8ac62cce`, runtime rules 0.74, completes **1618 passed,
0 failed**. This executes the two fixture follow-ups that remained pending in
the earlier `37c165a4` record, without erasing that failed result.

| Native owner | Executed result |
|---|---|
| Content Preparation | **12 passed, 0 failed**, 3.11s. |
| Gameplay | **521 passed, 0 failed**, 0.42s. |
| World | **11 passed, 0 failed**, 0.01s. |
| Simulation | **1074 passed, 0 failed**, 694.40s. |

Evidence: `.phoenix/verify-8-12-green-native-owners.log`. This is the selected
four-owner union, not the full workspace suite; it predates rules 0.75.

The next intact-salvage retry at `5180261f` still **failed: 0 passed, 1 failed**,
3.12s after a 30.89s build. Its new normal Human-held Helm setup earns the
permitted offer and Recovery admission. The first full-state comparison then
finds `motion_plan=null` in the live capture but a retained avoidance plan in
the freshly restored bootstrap. No field was masked. Evidence:
`.phoenix/verify-8-12-held-helm-salvage.log`.

PASM first records authoritative absence for the complete retained motion-plan
frontier. Runtime correction `18e99858` clears the already-installed bootstrap
map when every captured row lacks a plan; mixed captures retain the existing
UUID rebinding. It does not replan, change cadence, add work or widen folding.
Compatibility commit `e027e568` advances simulation rules to **0.75**, retaining
protocol **45** and snapshot **88**.

The first regression invocation compiled but matched **zero tests**; its exit
status is not counted as verification. The corrected `full_restore_` filter
passes **3/3**, 0.13s: the two new empty/mixed-plan regressions and the existing
Personnel full-restore test. The actual capture owner group subsequently passes
**5/5**, 0.00s. Logs:
`.phoenix/verify-8-12-motion-restore-regressions.log`,
`.phoenix/verify-8-12-motion-restore-regressions-exact.log`, and
`.phoenix/verify-8-12-capture-owner-current.log`.

The current eight-target whole-host batch at `e027e568` / rules 0.75 completes
**19 passed, 0 failed**, after a 3m 49s build:

| Host target | Executed result |
|---|---|
| `activity_foundation_continuation` | **3/3 PASS**, 35.50s. |
| `facility_supply_continuation` | **2/2 PASS**, 32.02s. |
| `fitting_facility_continuation` | **2/2 PASS**, 125.97s, including real intact-equipment recovery, preserved condition/identity and ordinary vacant Repair fetching and installing it through full continuation. |
| `logistics_continuation` | **3/3 PASS**, 20.79s. |
| `physical_damage_routes` | **3/3 PASS**, 28.02s. Strict actual content corpus: 83 worlds, 277 entity instances, 119 overrides, 186 layouts and 91 zero-team layouts. |
| `production_continuation` | **2/2 PASS**, 19.61s. |
| `recovery_continuation` | **3/3 PASS**, 22.90s. |
| `shield_energy_continuation` | **1/1 PASS**, 7.52s. |

Evidence: `.phoenix/verify-8-12-current-hosts-rules75.log`. The two selected
`snapshot_resume` energy/strike checks also pass **2/2**, 20.03s after a 16.40s
build, with 89 other tests filtered out:
`.phoenix/verify-8-12-current-energy-resume-rules75.log`. These executed results
supersede the pending-retry status at the earlier cutoff, not the historical
failed logs themselves.

### Fresh-process continuation receipts at `00cf53c3` / rules 0.74

The Repair reconnect parent check passes **1/1**, 47.00s after a 9.10s build,
comparing two default
and one pinned fresh child traces. Each child passes **1/1**, respectively
14.93s, 13.56s and 12.63s. The collected log is
`.phoenix/verify-8-12-repair-reconnect.log`; its bounded extracted receipt is
`.phoenix/verify-8-12-repair-reconnect-summary.txt`. The intact full log has SHA-256
`12f95ee9ae54068ef97f6724f0486214fdc23e617798a37c1ac4b4ba6d284a05`.
This is a native fresh-process
continuation comparison, not a browser transport-reconnect or physical-device
acceptance result.

The shield fresh-process runs also each pass **1/1**, respectively 6.60s, 4.73s
and 4.61s. The complete TRACE payload is identical across all three runs:
**3,971,765 bytes**, SHA-256
`b53106db6abefc77226eb620273b2872d576b9efc81128f356e419a4ca1fcfa1`.
The bounded receipt is `.phoenix/verify-8-12-shield-reconnect-receipt.json`, naming
the two default logs and pinned log. These process comparisons retain their
rules 0.74 boundary; they are not silently attributed to rules 0.75.

### Pinned design and current remaining work

At clean pinned `e027e568`, all three PASM commands return success:

- Validation: **Status: OK**, 1202 entities, 630 non-error findings.
- Scan: **Validation: OK**, 4157 files, 8696 observed dependencies, 18 packages.
- Traceability: **Validation: OK**, 246 rows.

Logs: `.phoenix/verify-8-12-rules75-validate.log`,
`.phoenix/verify-8-12-rules75-scan.log`, and
`.phoenix/verify-8-12-rules75-traceability.log`. The bounded affected-wiki lint
also passes at `e027e568` for seven affected pages and their index/source
references, with no issues, per the collected PDC receipt
`.phoenix/verify-8-12-bounded-wiki-lint-e027e568.json`; this does not claim a
repository-wide wiki or whole integration gate pass.

A matching development WASM build is underway at this cutoff. Current
matching-host browser/Workshop and actual browser reconnect evidence,
representative balance/performance and station-workload acceptance have not yet
passed. Native displays and Human acceptance remain separate. The old projected
component/browser results remain pinned above, and the native passes do not
substitute for these outstanding surfaces. No full CI pass, push, formal
increment closure or completion through 24 is claimed. Finish the agreed 8-12
verification and necessary corrections, commit the evidence, then pause;
implementation work for increments 13-24 remains parked.

## Final browser and Workshop results

The matching development bundle was built at `ded6653a`: host WASM, client and
Workshop preview WASM, using runtime `e027e568` / rules 0.75 plus the WASM-only
message import correction `a794ac49`. The missing import was an actual build
failure, retained in `.phoenix/verify-8-12-wasm-host-build-current.log`; it did
not require a simulation behaviour change. Successful build logs and hashes are
in `.phoenix/verify-8-12-wasm-host-build-typed-imports.log`,
`.phoenix/verify-8-12-matching-client-build.log`,
`.phoenix/verify-8-12-wasm-preview-build.log`,
`.phoenix/verify-8-12-copy-preview.log` and
`.phoenix/verify-8-12-browser-artifact-receipt.json`. This is a development
browser build, not the release/deployment gate.

The complete matching-host browser selection at test source `dbfd0027` passes
**15/15 in 3.3 minutes**. It covers actual two/four-seat Energy journeys,
Engineering projection and isolation, browser refresh/replacement and sever/revive,
actual save export/import and refusal, damage popup and paid firing with restraint.
Evidence: `.phoenix/verify-8-12-final-host-browser.log` and the corresponding
`-artifacts` directory. Earlier 12/15 and diagnostic failures remain in their
logs. Corrections select the active Console, assert the dated Sensors restraint
record, and fire the currently ready bank for the exact target. They do not
bypass ownership, grant private target knowledge or change weapon geometry.

Workshop initially failed to start because the build copy list omitted eight
existing required modules. Packaging fix `2ea96d97` adds those modules;
`.phoenix/verify-8-12-workshop-startup-rebuild.log` records the successful rebuild.
The initial run has one failure and two interrupted cases, not three failures.
Two subsequent three-case attempts pass factions and disposable Test but fail
resource-authoring validation. The first fixture duplicated the new primary
transfer resource in its additional choices; the second tried to change exact
fitted-default machinery without authoring a sidegrade. Both guards correctly
refuse the edits. Fixture corrections `956a385c` and `9bd7bab6` retain those
guards and select actual unfitted weapon/Energy hardware for positive edits.

The final complete real three-case render selection at `9bd7bab6` passes
**3/3 in 6.1 minutes**: resources 5.1 minutes, factions 19.0 seconds and disposable
Test 35.1 seconds. It exercises accepted resource/Energy/shield/interior/damage
and weapon edits, undo/history, reload/export, actual mesh Preview and disposable
Test. The Test uses unsaved captured source and retires without Live storage or
delivery. Evidence: `.phoenix/verify-8-12-workshop-unfitted-render.log` and
`.phoenix/verify-8-12-workshop-unfitted-artifacts`. Earlier attempts remain in
`.phoenix/verify-8-12-current-workshop-render.log`,
`.phoenix/verify-8-12-current-workshop-render-retry.log` and
`.phoenix/verify-8-12-final-workshop-render.log`.

## Same-machine baseline comparison at 9bd7bab6

The source and revision stayed fixed throughout the baseline and Cruiser matrix.
The wrapper builds and copies the actual measure-profile executable, then hashes
it and the runtime/content sources. Current protocol/snapshot/rules are **45/88/0.75**.
The machine, measure profile and feature set match increment 0. Unrelated
untracked `output/` is preserved; runtime/content sources are clean.

- Source: `9bd7bab63230f1e653802cc995d15a3353d5076c`.
- Runtime/content SHA-256: `66b54ee0734a6f19822c6e51912d310658d3b341c73238303e085872fc4b5cb4`.
- Executable SHA-256: `72a004dbc0467cee5b39267baa4ecd705bfe5a62449708397a3aaa038a59afa8`.
- Original baseline: `8d8143e9`, retained in `implementation/baseline.json`.

Both 60-second, seed-42 scenarios pass byte-identical repeated reports,
record/replay checkpoints and agreement between all three timing-run
continuations: **6/6 checks, command exit 0**. Raw artifacts are in
`.phoenix/redesign-baseline/2026-10-16-verify-8-12`; the command log is
`.phoenix/verify-8-12-baseline-capture.log`.

Each timing capture excludes 300 warm-up ticks and retains 3301 samples. The
following values are medians across three captures, in milliseconds:

| Scenario | Increment 0 p50 | Current p50 | Increment 0 p95 | Current p95 |
|---|---:|---:|---:|---:|
| Combat Test | 1.3267 | 5.3880 | 1.8900 | 9.0916 |
| Falling Skyway | 1.1589 | 4.0296 | 1.5565 | 5.4523 |

No measured tick exceeds 16.67 ms across **19,806** retained samples. Nevertheless,
median and p95 tick costs increase roughly **3.5–4.8 times**; this is a measured
regression, not a performance acceptance. These are current integrated-headless
results, including already-landed later mechanisms; they do not isolate the
increment responsible or measure rendering, real networking or devices.

The receipt also compares the actual Backfill command buckets and combat AARs.
For Combat Test, Helm commands rise 11,139 to 16,044, Tactical 11 to 489 and
Science falls 34 to 10. Falling Skyway Helm commands rise 5397 to 10,794.
All Human counts are zero. Counts show changed autonomous behaviour; they do
not establish player engagement, communication, balanced workload or flow.

## Unchanged Cruiser balance: failed

The exact captured binary runs all four opponents across five seeds, for
120 simulation seconds per duel. The configuration hash still matches increment
0 exactly: `c6946bde4e6bfb9b702a9bb2dd1da38a59297ec5c1ef2a3eaf25bf89177be5ae`.
No threshold, opponent, seed or duration changed.

| Opponent | Wins | Draws | Timeouts | Mean damage margin |
|---|---:|---:|---:|---:|
| Destroyer | 0 | 1 | 4 | +111.3290 |
| Cruiser | 0 | 1 | 4 | +186.5218 |
| Patrol | 0 | 5 | 0 | +97.5028 |
| Warhawk | 0 | 0 | 5 | −324.9360 |

The old baseline won **20/20**. Current results are **0 wins, 0 losses, 7 draws,
13 timeouts**, with zero runner failures. Of sixteen checks, five pass, seven
explicitly fail and four TTK checks have no qualifying kills/data. The runner
correctly treats those four missing-data checks as failures: **11/16 fail,
command exit 1**. Zero runner failures is not a balance pass.

Evidence: `.phoenix/verify-8-12-cruiser-balance.log`, plus `receipt.json`,
`merged.json`, `summary.md` and every source AAR under
`.phoenix/redesign-baseline/2026-10-16-verify-8-12-cruiser`.

### Bounded diagnostic, without tuning

The same captured binary and Destroyer seed 1 run for 30 simulation seconds with
NDJSON output, exit 0. Its four beams last **0.9000, 2.0167, 0.1500 and 1.6500
seconds**, rather than completing their authored nominal six-second cycles.
At sampled cutoff boundaries the phaser reserves remain above 24/30 and no
thermal stop is reported. Two aft cutoffs cross the authored 40-unit range;
fore tracking errors change around the authored finite turret tolerance. This
makes engagement geometry and mount tracking concrete next investigations.

The 120-second seed-1 AAR has ten beam starts, 273 damage dealt, no torpedo launch
and zero enemy hull damage. That average damage is below the enemy's base shield
regeneration. Source review found no established free-regeneration, runaway
repair or blanket torpedo-loading defect. Dated measured aiming is another
candidate, but this trace does **not** establish a single exclusive cause or a
safe tuning value. No simulation changes or balancing edits follow this probe.
Raw stream, summary, bounded beam/cutoff extracts and file hashes are retained in
`.phoenix/verify-8-12-duel-probe*` and the comparison receipt.

## Paused disposition

The agreed automated verification runs are collected and their necessary build,
restore, packaging and fixture corrections are committed. **The automated exit
for increments 8–12 has not passed:** unchanged combat balance fails, and the
measured performance regression needs assessment. Further balance diagnosis must
separate already-landed sensing/mount/AI changes from resource and damage changes
before altering authored combat values. No threshold is relaxed to manufacture
an exit.

Work is paused as requested; increments 13–24 and their isolated unfinished work
remain parked. Initial automated exits stay **1–7**, and formal closure stays
**increment 0 only**. Human/device and station-workload acceptance remain separate.
No full workspace/pre-push gate, push or release is claimed.
