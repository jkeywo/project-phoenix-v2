# Issue #1449 execution evidence

Started 8 September 2026 on `codex/1449`, base
`349dea207b6a27cec81b696fc84dd1b029aea730`. This is an execution record,
not completion of the programme. Existing dirty wiki and audit/plan files
are unrelated and are not included in this work.

## A1: named regressions

At the base revision, with no Rust or console-source changes:

```powershell
cd tests/smoke
$env:PHOENIX_SMOKE_PORT='3150'
npx playwright test tactical-keyboard.spec.js combat-keyboard.spec.js comms-ops-keyboard.spec.js --project chromium --reporter=list
```

Exit 0; **4 passed** in 3.5 seconds, zero failed/skipped:

- Combat family: fire a weapon, adjust shields, order a repair — all from the keyboard, no pointer.
- Comms console: a hail is selected and answered from the keyboard, with no pointer.
- Captain console: an objective is boosted and a scan taken from the keyboard, with no pointer.
- Tactical console: every principal action fires from the keyboard, with no pointer.

All 273 files under served `dist/gui/` were compared byte-for-byte with `gui/`;
zero differences. These standalone pages do not boot host WASM. Port 3150
disabled server reuse. The first sandboxed attempt failed to launch Chromium
with `spawn EPERM`; it established no application result. The successful run
used permission to launch the local Chromium executable.

```powershell
cargo test --lib --features headless -- a_one_hit_destruction_files_a_repair_request_as_well_as_the_alert threat_warning_emitted_for_hostile_in_range threat_warning_re_emitted_on_bearing_change
```

Exit 0; **3 passed, 0 failed, 0 ignored**, 7,413 filtered out:

- `ship::damage_sync::tests::a_one_hit_destruction_files_a_repair_request_as_well_as_the_alert`
- `ship::sensors::tests::threat_warning_emitted_for_hostile_in_range`
- `ship::sensors::tests::threat_warning_re_emitted_on_bearing_change`

Cargo rebuilt the current project and ran
`target/debug/deps/project_phoenix-b3d2dd215d375226.exe`, test profile,
optimized with debuginfo. Existing fixes include `368bde38` (keyboard family
metadata) and `25d00011` (typed Coordination receivers). These named results
discharge the defects in #1270 and #1271; no new runtime fix was required.

## A2: retrospective scope and criterion disposition

The user accepted the existing fire-accent contrast correction and authorised
retinting the remaining inline stylesheet using existing tokens, including visual
changes. Commit f84cce55 removes all colour/type-size literals from the inline
stylesheet. All three host stylesheets also contain zero such literals. The
whole server document retains 23 colour and nine size syntaxes in JavaScript
and attributes; these are outside the stylesheet criterion.

The nine unchanged accents plus --fire #e14330 -> #e64a34 (RGB 230,74,52)
are the accepted exception to #1357's original all-ten criterion. Current
--surface-ridge is #343a44. Issue comments record that disposition.

The retrospective #1359 inventory is #asset-loading,
#scenario-picker-overlay, #waiting-overlay and #join-entry, in priority order.
Connection chrome can coexist; #game-over-overlay is post-play. The pure owner
is gui/pre-play-view.js and client.html's applyPrePlaySurfaces is its consumer.
This inventory was published retrospectively, not represented as pre-work.

Historical screenshots were not recovered. Six current real-render before/after
PNGs and their receipt are in docs/acceptance/1449-preplay/ (landing, picker,
lobby at 1440x1000, with the real vendored QR). They establish the approved
current retint, not historical visual identity. The explicit missing-history
criterion disposition and retrospective process record remain visible on the
owning issues.

Focused token/pre-play tests: 524 passed in three files. Current capture smoke:
two passed in 8.2 seconds. An initial wider browser selection had 13 passes,
one landing timeout and one automation loading skip; landing passed alone.
The loading skip was not positive loading-flow evidence. The test is now
`loading-progress.render.spec.js`, using the existing SwiftShader project and
real preload path. Fresh port 3162: **1 passed**, exit 0, 15.0 seconds (17.6 total).
It observes an intermediate host percentage and client wire fraction, then
GameStarted hides the overlay. Earlier authoring failures assumed a bare numeric
label rather than the current percentage string and sampled client delivery too
early; parsing and waiting were corrected without removing the assertions.
Independent review requested a visibility assertion beside the intermediate
percentage; that was added. Final port 3163: **1 passed**, exit 0, 16.6 seconds
(19.2 total). This uses the shared minimal fixture world with real GLBs and
rendering; it does not claim to load the production default world.

Current port 3159 landing/lobby selection: **14 passed**, exit 0, 58.2 seconds
(three landing, four lobby, five fleet-lobby and two native GM lobby cases).
Both runs used `project-phoenix-5f8fb494bdf5d61a`. Final integration/push remains
separate; historical captures are still unavailable as recorded above.

## A3: performance evidence recovery

Recent full runs `34214002585` (revision `a11fd4df`) and `34202861006`
(`36a25ec7`) skipped the performance job after smoke failures. They supply no
current performance receipt. The historical defect comments are adverse results,
not evidence of recovery. No baseline has been adopted by this task.

## A4: delivery evidence recovery

The most recent successful `deploy-demo.yml` run found was `32017571571`,
revision `286aa6e46f4648c93c6cae4a835c5b0d00f6ef5c`. No runs were found for
`check-deploy-headers.yml`. This does not identify the currently served revision
without an additional build-identity check.

On 8 September, the existing read-only checker against
`https://pp-demo.kiwigamedesign.co.uk/` fetched six paths, with no unreachable
responses, and reported **FAIL: 1 error, 0 warnings**. The served bundle names
were `project-phoenix-1bf740dd92d9f921.js` and
`project-phoenix-1bf740dd92d9f921_bg.wasm.gz`.

`/assets/logo.png` returned `public, max-age=14400, must-revalidate`; the
contract rejects unhashed assets cached at least 14,400 seconds. The old
diagnostic incorrectly called this a year. The diagnostic now reports the
actual seconds, with the policy and threshold unchanged.
`npx vitest run tests/client/deploy-headers.test.js`: exit 0, **25 passed**.

The raw initial probe is `target/issue-1449/deploy-headers.json` (local,
untracked). The shell subsequently printed that JSON and returned 0;
the probe's JSON says `ok: false`, which is the result, not that shell exit.
No publish, cache purge, worker or dashboard mutation was performed.

Public release/playtest decisions, compatibility evidence, the deployed cache
failure, and the remaining #1449 work packages are still outstanding.


### Deployed identity and compatibility inspection

The 8 September read-only probe found content identity `phoenix-base`, epoch 1
in both served scenario catalogues (SHA-256
`5fffa9c7bcb9891a490c41e050b2297e769231d9ac8d15077f08138543ea40a4`).
The served `/client/` page has no `phoenix-client-stamp` metadata; its SHA-256 is
`1c0cc08ee6df36c31ece2955c46f682c573bc73712ad6d9b9db75f105753c03f`.
This is a compatibility evidence gap, not a successful mixed-version join.
Current native stamp-policy tests passed 11/11, including unstamped refusal,
protocol/content mismatch and startup pinning. That tests current policy, not
the deployed client. Raw probe: `target/issue-1449/deployed-identity.json`.
The live cache failure and missing stamp need deployment-owner action; this
issue expressly forbids silently reconfiguring or publishing the deployment.

## E2: current manual scenario suite

At revision `18013e6b`, with `RUST_TEST_THREADS=2`:
`cargo test -p project-phoenix --features falling-skyway-sim-tests --test headless_runner falling_skyway_`
finished with exit 0: **94 passed, 0 failed, 0 ignored, 110 filtered out**,
1349.80 seconds execution. Both
`falling_skyway_act_2_rescue_lands_when_the_crew_start_before_the_band` and
`falling_skyway_the_lift_runs_out_and_the_third_claimant_is_never_offered_one`
passed. The log is `target/issue-1449/falling-skyway-manual.log`.
No authored values, expected outcomes or test selection were weakened.
The command filters Falling Skyway tests; unrelated demolition-engine tests
remain part of the ordinary suite. This does not replace human crew acceptance.
