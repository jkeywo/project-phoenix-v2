# Console redesign acceptance (#1399)

Integrated code/test candidate: `eacd312e`, rebased onto clean local main
`227fa04d`. The integrated native and full browser suites passed. #1399 acceptance is
complete. No remote push is part of this acceptance run.

## Integration status

Main `227fa04d` includes the independent test/CI cleanup (`91793735`). The
seven #1399 commits replayed without conflicts: range-diff from
`ca8791f5..696248df` to `227fa04d..ad0ff28c` reported every entry unchanged.
Incoming changes affect test fixtures, test coverage, CI and documentation;
production runtime behavior is unchanged.

Candidate `eacd312e` then freezes Bevy wall time in exactly two native fixtures
that explicitly assign SimTick. Under suite contention, automatic fixed time
could otherwise advance the assigned tick from 100 to 101. All exact deadline
assertions remain. Both targeted tests passed, followed by the full integrated workspace
headless run: **7,550 passed, 0 failed, 80 ignored** across 53 result summaries
(exit 0). Its log is `target/console-redesign-resume/1399/20-integrated-cargo-test-headless.log`.

The third complete browser run at `696248df` finished with **304 passed,
1 failed, 1 skipped**, including all seven render tests passing. Its fleet
case incorrectly required an instantaneous absence of a watermark barrier.
The revised case requires both hosts to advance another delay-plus-30-tick
window and each report a cleared barrier in the same readback, within the
existing 60-second bound. The full fleet spec passed **4 cases** against the
actual bundle. The fourth full browser run passed **305 cases, 0 failed,
1 deliberately skipped** across both Chromium and render projects (306 planned
cases in 88 files, 21.9 minutes). All seven render cases passed. The single
skip is `loading-progress.spec.js:32`: automation forces `preload_complete=true`,
so the loading animation phase is not observable; the Rust codec test covers
its wire format. The full run used no `@core` filter. Its log is
`C:/Coding/ppv2-track-b/target/console-redesign-resume/1399-wasm/16-fourth-playwright-full.log`.

Integrated Vitest at `eacd312e` passed **6,761 tests in 279 files**; incoming
main removed three tests from the earlier 6,764-test suite. Bundle preflight
again verified all 249 root GUI paths/hashes, a successful client rebuild and
an unchanged optimized WASM hash. Earlier release, balance and production-code
checks below retain their original revision scope. Changes after the integrated
code/test candidate are acceptance documentation and balance comments only;
parsed balance configuration is unchanged.

The post-rebase wiki lint at `eacd312e` found **no errors**: 62 Markdown files,
24 numbered references within file bounds, all 60 index links and all entity/
concept pages indexed, and **883 frontmatter source paths** resolving. The
older 822 count covers only the selected source-directory prefixes; the
expanded count also includes root files and other paths. Frontmatter fields
were checked. The incoming testing-strategy page reflects the current CI/test
layout; batch console prose retains the current chrome and semantic actions.
None of the seven batch-touched wiki pages carries a numbered Rust reference,
so that semantic anchor audit required no edits. Audit result:
`target/console-redesign-resume/1399/wiki-lint-integrated.json`.

Targeted native and fleet logs are respectively
`target/console-redesign-resume/1399/native-clock-targeted.log` and
`target/console-redesign-resume/1399-dock/fleet-final.log`.

## Focused checks completed on 2026-09-07

- Wiki SCHEMA lint: all 62 Markdown files inspected; 24 line-number references
  resolve within their files; 822 source-directory frontmatter paths exist; all 60 index
  page links exist and every entity/concept page is indexed. No broken references.
- Current runtime prose no longer describes the retired Captain Weapons Hold
  lever. Fire restraint is documented as a weapons Power group order. Console
  UI navigation now describes the shell-owned chrome and the shipped shared
  segments, tractor/umbilical composition, Dock control and named repair cards.
- `npx vitest run tests/client/hero-bar.test.js tests/client/console-chrome.test.js
  tests/client/engineering-actions.test.js tests/client/helm-actions.test.js
  tests/client/interaction-floors.test.js tests/client/control-floors.test.js`:
  **410 passed in 6 files**.
- `console-redesign-accessibility.spec.js`: **2 browser cases passed**, at
  390x844 portrait and 844x390 landscape. The shipped Station Bar renderer and
  shell stylesheet are exercised with direct, overlay and visiting tabs.
  Arrow navigation, Home/End, wrapping, selected state and exactly one tab stop
  are asserted. Up/Down now work for the vertical rail as well as Left/Right.
- Those browser cases also read the real selected tab's computed text/border
  colours under standard/high contrast, and the real Red Alert bezel's motion
  under full/reduced motion. The deterministic renderer test suppresses the
  disconnected lobby's scripts; the separate accessibility round-trip specs
  retain responsibility for settings persistence and whole-shell propagation.

Temporary focused logs: `target/console-redesign-resume/1399/targeted-client.log`
and `accessibility.log`. The focused browser harness served the existing client
bundle with the changed `hero-bar.js` copied into it. The final full smoke used
the rebuilt release bundle described below.

## Input coverage and scope

| Surface | Acceptance evidence |
| --- | --- |
| Station and overlay tabs | Browser roving test above; `console-tabs.spec.js` covers real host/iframe overlay routing. |
| Captain and Engineering segments | Shared segment roving contract in `console-chrome.test.js`; `captain-console.spec.js` and `engineering-console.spec.js` exercise keyboard selection. |
| Helm Dock, steering, impulse and boost | `helm-actions.test.js` covers the two binding slots, standard gamepad metadata and admitted contextual action variants; responsive Dock smoke checks populated control geometry. |
| Tractor and umbilical | `engineering-actions.test.js` checks authoritative start/stop variants and control-system ownership; operations smoke covers the actual Cruiser transfer/tow paths. |
| Repair team destination and recall | `engineering-actions.test.js` covers exact team/target dispatch and parameter-free gamepad recall, including a team abroad; repair smoke covers the named field destination. |
| Tactical and other station controls | Full client semantic-structure/action suites and full smoke keyboard specs are included in the final gate below. |

Gamepad evidence is virtual standard-mapping and semantic-adapter coverage,
not a physical controller session. The field-dispatch regression drives the
real `createGamepadInputRuntime` with neutral and pressed D-pad snapshots into
the real Engineering registry. It verifies named-team external dispatch,
recall of that exact team abroad, and no request with no lock, a refused
candidate or AUTO. An explicit team context is preserved; absent one, the
existing first-available team convention applies. Human field shortcuts now
send `dispatch_repair_team` / `recall_repair_team`, never the AI fieldless verbs.
Generic parameter-free internal dispatch still does not choose the field.
Native Ultralight OS accessibility preference detection and physical controller
behaviour are outside this browser acceptance run.

The additional focused gamepad run (`engineering-actions.test.js` and
`gamepad-input.test.js`) passed **46 tests in 2 files**. Its temporary log is
`target/console-redesign-resume/1399/gamepad.log`.

## Earlier gate evidence (before main integration)

These results were collected before rebasing onto `227fa04d`: native checks
used the `ca8791f5` Rust runtime; client checks used the revisions named below.
The integrated status above supersedes any pending-run description here.

| Gate | Verified result |
| --- | --- |
| Rust formatting and explicit-feature clippy | PASS |
| Workspace tests with headless | PASS: 7,553 passed, 0 failed, 80 ignored across 53 result summaries |
| No-default-features check | PASS |
| Viewer no-run build | PASS: 49 test executables built; this is compile coverage, not an executed viewer test suite |
| Full Vitest at pre-rebase runtime revision `9dbc9159` | PASS: 6,764 tests in 279 files; subsequent `0ab507c9` changes only smoke tests |
| Strict strings, debug surfaces, LODs and LOD captures | PASS |
| PASM validate, scan and traceability | PASS |
| Cruiser balance matrix | PASS: 20 reports, all 16 configured thresholds; Warhawk damage margin 47.24 against the unchanged approved floor of 25 |
| Release Trunk build | PASS; Binaryen reduced WASM from 73.89 MB to 47.44 MB |
| Final client build at `0ab507c9` | PASS |
| Integrated full Playwright at `eacd312e`, Chromium and render projects | PASS: 305 passed, 0 failed, 1 deliberate skip; 306 planned tests in 88 files |

Native/client/PASM/balance logs and exit records are under
`C:/Coding/ppv2-fe-d/target/console-redesign-resume/1399/`. Balance source reports
are in `C:/Coding/ppv2-fe-d/.balance-out/`, including `merged.json`, `summary.md`
and the 20 per-run reports. Browser build and full-run logs are under
`C:/Coding/ppv2-track-b/target/console-redesign-resume/1399-wasm/`.
The second browser run used the rebuilt client at `0ab507c9` with the release
WASM; its log is `09-final-playwright-full.log`. The third run is recorded in
`13-third-playwright-results.json`. Neither run was fully green.

## Failures found by the initial full browser run

The initial run at `7423b647` executed 304 tests: **296 passed, 7 failed,
1 skipped**. All seven render-project tests passed. Every failure was diagnosed
and its affected case passed a focused rerun before the final full run began:

| Failed case | Cause and correction |
| --- | --- |
| GM spatial Helm | Unscoped `#overlay` also matched the GM map's private shadow DOM. The assertion now selects the host QR overlay by its direct `#qr-panel` child. |
| Cruiser docking | Real runtime bug on Cruiser and Destroyer: both buttons called an activation method absent from the `initConsole` return handle. Both now call the installed window semantic seam. Real click regressions assert exact Dock/Undock envelopes on both hulls; the Cruiser also completed a real WASM-host mate in the focused rerun. |
| Helm desktop binding hints | Flex-item blockification made visible spans compute as `block`. The test now verifies visible, nonempty, unclipped hint geometry while retaining the phone-hidden checks. |
| Reduced-motion persistence | The test read the retired accessibility-only storage key. It now verifies the persisted accessibility section of the current operator profile. |
| Host QR lifecycle | Lobby keeps the join code visible by design. The test now launches a real mission and waits for InProgress before testing operator visibility control and remapped input. |
| Red Alert profile import | The fixture imported KeyU, which conflicts with shipped actions in overlapping contexts. It now imports a distinct Ctrl+Shift+Y chord and verifies its authoritative action; reserved/conflicting-input refusal checks remain. |
| Tactical keyboard traversal | The test expected the old DOM order and a retired blaster release alias. It now traverses the current desktop columns, verifies every principal action, and asserts one named charge-start with no repeat/release duplicate or cancellation. |

Focused corrective logs are under
`C:/Coding/ppv2-fe-d/target/console-redesign-resume/1399-dock/`. These focused
passes were followed by the successful fourth full browser run above.

Full Playwright includes `accessibility-contrast`, `accessibility-reduced-motion`,
`modal-focus-keyboard`, the new Station Bar cases, keyboard/control-floor cases,
and the populated Cruiser responsive and operations cases. No `@core` filter.
The manually enabled Falling Skyway timeline simulations are not this gate;
known pre-existing failures are tracked in #1401.

## Second full run and corrected bundle verification

The second full run at `0ab507c9` completed with **302 passed, 3 failed,
1 skipped**; all seven render tests passed. Two new standalone Dock regression
cases encountered stale `dist/gui/` HTML: rebuilding the client refreshed
`dist/client/gui/`, while the host/GM copy remained from the earlier Trunk
static stage. Both are real product surfaces. The test was retained on the
host path. Trunk's static GUI copy stage was reproduced from final source,
with all 249 root GUI paths and SHA hashes matching and no missing/extra files;
the client bundle was rebuilt as well. The optimized release WASM SHA remained
unchanged. This refresh changes assets only, not the Rust artifact.

The third failure was NavigationChart admission immediately after GameStarted,
before the auxiliary Navigation Station had its first allocation snapshot.
An instrumented correlated request at that point returned Refused. After the
authoritative snapshot reported Navigation hosted by Comms at Std rating,
the same request returned Applied and its view readback passed. The smoke now
waits for that exact host projection before sending the original request.
Its five-second view readback deadline is unchanged; there is no blind delay.

The two unchanged Dock regressions and the synchronized NavigationChart case
were rerun against the actual refreshed built bundle, without source routing.
All three passed (23.2 seconds); the focused result is recorded in
`C:/Coding/ppv2-fe-d/target/console-redesign-resume/1399-dock/round4-fixed.log`.
The subsequent fourth full 306-case run passed as recorded above, completing
the browser gate.
