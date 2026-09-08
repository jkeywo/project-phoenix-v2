# Follow-up implementation and measurements — 8 September 2026

The follow-up removes the measured Combat Test streaming stalls, restores claimable native consoles after return to the lobby, and keeps revealed Settings above the mission HUD. The opt-in `scale2` experiment reduces Ultralight render cost and improves pane-thread cadence in a three-console workload; it does not establish a main-frame improvement or justify changing the default resolution.

This record supplements the historical [results](RESULTS.md) and [execution ledger](PROGRESS.md), preserving their original measurements and artifact boundaries. The full reviewed architecture/reuse batch and six later fixes are integrated on main. The [integration record](MERGE.md) gives the exact scope and passing final publication checks. The pinned Vellum animation fix is published at its verified revision. The user authorized Phoenix main's push; CI results remain pending until observed. T2 retains ownership of its separate unfinished GM/scheduler batch, which is excluded from this assembly.

## Frozen sources and validation

| Item | Exact source / artifact |
| --- | --- |
| Original completed batch | `6e77d705153fc145e103663103e0cd46ed549abe`, `codex/review-followup-plan` |
| Follow-up candidate | `734683cdf2577789dffe8f97ade15ebc758bbee5`, `codex/review-followup-fixes` |
| Streaming control | `b29b8cd614962a9d75688962247873fd878889b2`, `codex/review-followup-control` |
| Control measure/headless EXE SHA-256 | `861195dac4ea2baca47b1472471f38165c51e4cfc41bed6b8203f064f303d2ef` |
| Candidate measure/headless EXE SHA-256 | `d93cef6cda18e130596958d944784bf149296de862c331ec9ea1d7423f973267` |
| Candidate measure/host,ultralight EXE SHA-256 | `c64b7aa13e30434e5ef824ee3f8543f67b4344a3e4ba3c8044a270ad8a083fa6` |

The two headless trees differ in exactly `src/asteroids/lifecycle.rs` and `src/entities/model_markers.rs`. Both omit the separate T2 content-resolution changes. Their assets, toolchain, feature set and build environment match. Both builds completed with exit 0, fresh project library and binary compiler-artifacts bound to the correct source, and unchanged clean source before/after. Executables, PDBs and build evidence are frozen outside the shared target directory. The common content SHA-256 is `9fed941859163fc4742474094f09a7803c7eaf3e1574cee186a47d0375ac9814`.

The candidate's focused native checks passed **32 tests, 0 failed, 0 ignored**: 25 library tests, the real asteroid delivery/replacement integration test, five native-lobby tests, and the streamed-belt snapshot continuation. The configuration was `host,headless,perf`; the [receipt and log](C:/Coding/project-phoenix-v2/.worktrees/review-followup-fixes/.phoenix/validation-01/receipt.json) retain the actual commands. These are targeted checks, not the combined integration's final gate. Independent source reviews passed. Applicable prior JavaScript and SDK-independent geometry evidence remains recorded at its tested source.

## Streaming tails

The implementation reads the current single entity-template entry per asteroid instead of cloning the whole template cache. Marker synchronization reuses exact-path geometry within one invocation, preserving entity transforms, variant distinctions, ordering, fixed-step placement, and retry/replacement behavior on later invocations. See [the bounded change](STREAMING-TAILS.md).

Twelve serial, interleaved captures used seed 42 and Alliance Destroyer: three control/candidate pairs of 120 simulated seconds in Combat Test, and three of 60 seconds in Falling Skyway. The first 300 updates are excluded, leaving 6,901 measured Combat updates and 3,301 Skyway updates per run. The `measure` build uses thin LTO; these values are not numerically interchangeable with the older fat-LTO release baseline. Per-run asset verification warms files, so this measures warm-file operation.

| World / pair | Mean ms, base → candidate | p99 ms, base → candidate | Maximum ms, base → candidate | Updates >16.67 ms, base → candidate |
| --- | ---: | ---: | ---: | ---: |
| Combat 1 | 1.436 → 1.370 | 2.280 → 2.130 | 20.853 → 4.362 | 6 → 0 |
| Combat 2 | 1.610 → 1.403 | 2.969 → 2.191 | 24.312 → 4.688 | 11 → 0 |
| Combat 3 | 1.509 → 1.415 | 2.469 → 2.208 | 24.967 → 4.440 | 9 → 0 |
| Skyway 1 | 1.121 → 1.113 | 1.650 → 1.607 | 2.145 → 2.086 | 0 → 0 |
| Skyway 2 | 1.107 → 1.113 | 1.643 → 1.633 | 1.961 → 1.985 | 0 → 0 |
| Skyway 3 | 1.149 → 1.148 | 1.687 → 1.737 | 2.461 → 2.475 | 0 → 0 |

Every pair passed the capture, unchanged source/artifact/content, duration, power/CPU identity and sampled-contention checks. All six Combat runs finished at tick 7200 with digest `8b50cca91c63d28b`; all six Skyway runs finished at tick 3600 with digest `69d4fd9cbded1475`. Final digest equality supports matching final outcomes; the targeted streaming/fixed-boundary/continuation regressions supply separate behavioral coverage.

The reduction persists into Combat's second minute: per-pair maxima change from
16.157/23.593/16.940 ms to 3.212/4.102/3.464 ms. All sixteen second-minute update
indices above 8 ms in every control repetition also become shorter in every
candidate repetition. The [detailed interpretation](C:/Coding/project-phoenix-v2/.worktrees/review-followup-fixes/.phoenix/streaming-followup/HEADLESS-RESULTS.md)
preserves exact indices, minute splits, residual tails and the observed mixed
Skyway changes. The table's 16.67 ms label abbreviates the actual `1000/60` ms
comparison threshold for update wall time, not a measured rendering deadline.

[ai] The repeated removal of the large Combat tails supports retaining these two bounded cache-access changes. The Skyway differences are small and mixed. No new named-body capture separates the individual savings from template access versus marker resolution, and this headless comparison does not establish native/WASM rendered frame-rate gains. Percentiles are per run, never pooled.

Raw evidence: [matrix summary](C:/Coding/project-phoenix-v2/.worktrees/review-followup-fixes/.phoenix/streaming-followup/pairs-02/analysis/summary.json), [aligned per-update pairs](C:/Coding/project-phoenix-v2/.worktrees/review-followup-fixes/.phoenix/streaming-followup/pairs-02/analysis/per-tick-pairs.csv), [control build receipt](C:/Coding/project-phoenix-v2/.worktrees/review-followup-fixes/.phoenix/streaming-followup/artifacts/base/build-receipt.json), [candidate build receipt](C:/Coding/project-phoenix-v2/.worktrees/review-followup-fixes/.phoenix/streaming-followup/artifacts/candidate/build-receipt.json).

## Three-console Ultralight comparison

These are historical measurements of frozen `734683cd`, before the display-refresh correction described below. That correction advances animation work the old runtime omitted; its performance has not been measured, so this table cannot establish the corrected runtime's cost or cadence.

One same-binary native/`scale2`/native triplet used Combat Test, seed 42, Alliance Destroyer, 40 seconds warm-up and 30 seconds measurement. Helm and Engineering occupied separate 1920×1080 monitors at 100%; Tactical occupied the laptop's current 1920×1200 desktop at 125%; the Viewscreen occupied another 1080p monitor. This is three active consoles, but does not satisfy a literal three-1080p-Station hardware criterion. The laptop's physical panel specification is not its current desktop mode.

All three consoles were visibly mounted, Ready, AFK and on Backfill through the sample window. Exact raster checks proved native 1920×1080/1.0 and 1920×1200/1.25 console rasters changed to 960×540/0.5 and 960×600/0.625 respectively under `scale2`. Physical window/input rectangles stayed unchanged. HUD remained native and visible; lobby remained native and hidden. All three complete captures and the additional raster/provenance/workload guards passed.

| Arm | Main mean ms | Main p99 ms | Pane-thread iterations/s | SDK render mean ms/iteration | Uploaded MB/s |
| --- | ---: | ---: | ---: | ---: | ---: |
| Native before | 17.581 | 35.208 | 8.500 | 63.724 | 279.337 |
| `scale2` | 17.181 | 32.073 | 14.567 | 25.269 | 191.508 |
| Native after | 17.061 | 31.661 | 8.267 | 65.939 | 272.241 |

[ai] The repeated native controls bracket a clear SDK render-cost reduction and faster pane service under `scale2`; the main-frame difference lies within their drift. Keep the experiment opt-in. Timing does not settle small-text legibility, radar-line quality, input comfort or adoption. The active worker overlaps Bevy's main/render work; its phases cannot be added to main-frame timing. Upload MB/s counts submitted pixel bytes, not measured GPU bandwidth. Sampled private-memory peaks fell by about 221–223 MB, which is process memory rather than GPU memory.

The [report](C:/Coding/project-phoenix-v2/.worktrees/review-followup-fixes/.phoenix/native-followup/three-console-comparison-01/report.md) and [complete surface/phase/copy evidence](C:/Coding/project-phoenix-v2/.worktrees/review-followup-fixes/.phoenix/native-followup/three-console-comparison-01/report.json) preserve each arm separately, including rectangle knowledge, forced copies, losses, queue ages and boundary cohorts. This triplet is one intervention, not three repetitions or a universal speedup. All launches used the actual Windows desktop, the same frozen SDK/bundle/profile, and the NVIDIA 5090 Laptop Vulkan adapter. Hardware was inventoried at 00:04:31 UTC; High performance remained selected afterward. Sampled CPU checks are a lower bound, not proof of identical thermals or total idleness.

All measured Console copies were forced full rasters after applied bridge work;
their natural dirty area is unknown, not zero. Console uploads fell from
219.341/214.180 MB/s to 94.188 MB/s while the faster worker consumed more distinct
HUD revisions: 213/206 became 333, with zero repeated revisions. The native HUD
therefore uploaded more, 59.996/58.061 MB/s becoming 97.321 MB/s. No particular
animated element is attributed. Hidden lobby applications, copies and uploads
stayed zero.

Measured frames had no copy failures, starvation or discards. The four final
native-before frames crossed the 70-second window boundary and uploaded
afterward; none remained unresolved at capture close. Whole captures separately
retain 32/8/4 startup buffer-starvation decisions and four post-window shutdown
`buffer_dropped` frames each. The [independently reviewed interpretation](C:/Coding/project-phoenix-v2/.worktrees/review-followup-fixes/.phoenix/native-followup/ULTRALIGHT-RESULTS.md)
accounts for those boundaries and the one bounded Engineering message deferral;
it does not describe the whole recordings as loss-free.

## Native functional follow-ups

Settings and the picker work in the corrected native bundle, and the moved-console reconnect correction passed its on-screen claim/Ready/second-start sequence twice. **One run crashed afterward** during display restoration; one repeat with additional diagnostics restored both consoles and exited normally. The renderer crash remains unresolved. All on-screen actions here were root-agent Computer Use automation, not human/HITL acceptance; the instrumented runs are not comparative performance measurements.

| Outcome | Actual evidence and boundary |
| --- | --- |
| Settings and visible picker: passed | The [r5c profiled run](C:/Coding/project-phoenix-v2/.claude/worktrees/t2-final-integration/.phoenix/review-final/profiled-r5c-01/OBSERVATIONS.md) passed host/Station Escape, backdrop, F9 and windowed Settings dismissal, followed by ordinary picker selection. |
| Retained outer pages after Return: passed | That run received the authored ending, returned to stationless/unready `0/4`, accepted ordinary claim/Ready and reached the second GameStarted on both retained pages. It does not establish fresh World materialization or a complete second authored timeline. Eventual normal exit was 0 after 603.4257037 seconds; Station windows lingered briefly. |
| Page moved during GameOver: corrected sequence passed twice; renderer fault unresolved | The [first 59b run](C:/Coding/project-phoenix-v2/.worktrees/review-gameover-reconnect/.phoenix/reconnect-runtime/no-profile-59b-01/OBSERVATIONS.md) completed reconnect/Return/claim/Ready/second-start on the same recreated page, then crashed while reopening Engineering: child −1073740791 after 292.8572052 seconds, not forced. The [single diagnostic repeat](C:/Coding/project-phoenix-v2/.worktrees/review-gameover-reconnect/.phoenix/reconnect-runtime/no-profile-59b-02/OBSERVATIONS.md) repeated that sequence, restored both live consoles and exited 0 after 427.7051089 seconds, not forced. No recovery change or causal explanation is established. |
| Stationary radar: bounded evidence passed | The [Combat run](C:/Coding/project-phoenix-v2/.claude/worktrees/t2-final-integration/.phoenix/review-final/stationary-r5c-01/OBSERVATIONS.md) retained own x400/z200, zero received speeds and 227.447 seconds of held-condition evidence while science-target/waypoint markers and canvas counters changed. The [hostile run](C:/Coding/project-phoenix-v2/.claude/worktrees/t2-final-integration/.phoenix/review-final/cooldown-r5c-probe-r6-01/OBSERVATIONS.md) adds six spans of unchanged **received** own pose with moving `probe_hostile` ship radar data. Retained blackboard state does not independently prove physics immobility; draw calls do not prove GPU presentation. |
| Cooldown: cycles and sampled width progression passed; smoothness remains unproved | In the hostile run, blaster-port decreased 4.6166615→0 seconds of 5 and phaser-omni 3.8500001→0 seconds of 4. Complete 46/47-point traces show computed-width progression/lag, but no adjacent unchanged inline target. No between-target interpolation or perceptual smoothness claim follows. |

The first 59b sequence supplies the previously missing moved-page evidence. Tactical's new GameOver Welcome at 133.652997 seconds had `connectedAtWelcome:true`; Return produced a stationless/unready connected Welcome at 178.474674. Ordinary claim and Ready were acknowledged at 191.753184 and199.361808, then both Tactical and original Helm received GameStarted at 232.445/232.444 seconds. Screenshots show both live Station UIs. The later saved layout matches its pre-run SHA exactly, but reopened Engineering had not received Welcome before the crash. The [run receipt and guards](C:/Coding/project-phoenix-v2/.worktrees/review-gameover-reconnect/.phoenix/reconnect-runtime/no-profile-59b-01/build-receipt.json) bind actual native `59b74842`, executable `b5576e58…`, test-only producer deltas, SDK and independently frozen r5c GUI. They do not claim clean whole-checkout, new browser parity or final integrated r6 acceptance. Wrapper exit 0 is not success for that failed child.

The repeat used the same frozen executable and content, with additional logging/backtraces only. It reached second GameStarted at 229.745216/229.745910 seconds on original Helm and the same recreated Tactical page. Restored Tactical and Engineering then received connected InProgress Welcomes, retained live positive-size UIs through 426 seconds, and appeared in saved screenshots alongside the restored host rows. The original saved-layout hash matched after close. Root closed the main window and then the lingering Station windows sequentially; this was normal eventual shutdown, not instantaneous closure. Pre/post artifact guards passed in both runs. The [first-run renderer diagnosis](C:/Coding/project-phoenix-v2/.worktrees/review-gameover-reconnect/.phoenix/reconnect-runtime/no-profile-59b-01/RENDERER-DIAGNOSIS.md) preserves the swap-chain acquisition failure and later device-loss/teardown cascade. A future device-lost callback could record the missing initiating reason; this one successful repeat neither fixes nor explains the crash. No further reproduction was performed.

Two explicit limits remain in addition to that crash. The **pre-start AFK runtime defect is unfixed**: the newer probe avoids it by waiting for InProgress, a fresh snapshot and all owned systems `Ai`; Backfill alone is insufficient. The probe's flat `selectedTarget` observer omits the composite Tactical payload, so its null fields do not prove absence of a selected target. The Combat run did not exercise cooldowns; the later hostile run did, with one held-condition reset and six separate unchanged-received-pose spans. Those normal runs exited 0 after 242.620172 and 182.5087768 seconds respectively. Corrected-animation-workload performance remains unmeasured.

The preserved validation configurations are distinct; their counts must not be added as unique tests or treated as the final full gate:

| Source / correction | Validation and artifact evidence |
| --- | --- |
| Combined r4 runtime `9B18BD78…` | [66 selected passes, 0 failures, 1 manual-only ignored](C:/Coding/project-phoenix-v2/.claude/worktrees/t2-final-integration/.t2-batch/t2-final-native-r4/binding.json), features `host,headless,perf,ultralight`, SDK-bound test/dev artifact over the complete staged/reviewed-unstaged manifest; no clean-HEAD claim. |
| Vellum `606b0c06a6e6419a5b2a8c4f5bfb255146f87814` | [RefreshDisplay(0) before rendering](C:/Coding/project-phoenix-v2/.worktrees/review-vellum-refresh/crates/vellum-ultralight/src/runtime.rs), with [required upstream Rust/PASM gates and real-SDK test](C:/Coding/project-phoenix-v2/.worktrees/review-vellum-refresh/target/review-refresh/HANDOFF.md) passed. The SDK test passed 1/1 in 0.46 seconds with advancing rAF, completed transition and unforced final pixels. |
| Consumer r5c `c37f08434bf7d4cf6956708d0e82cfc627e598bb`, runtime `85436183…` | [Cargo exit 0 after 8m03s, then 17 selected passes, 0 failures, 1 manual-only ignored](C:/Coding/project-phoenix-v2/.claude/worktrees/t2-final-integration/.t2-batch/t2-final-native-r5c/binding.json); resolved Vellum source, SDK and artifact guards passed. Five-fixture selection, separate from r4. Failed r5/r5b pre-compilation launcher attempts remain preserved. Its functional executable is `b2f2ba24…`. |
| GameOver reconnect `7f4ab1d3d1d6d7a95e5defbbf51c21aa34a8d48b`, adopted as `59b74842c9af2ca86fd6a6feea4ca414edbea118` | [5 native return tests, 3 client tests, independent review and static checks passed](C:/Coding/project-phoenix-v2/.worktrees/review-gameover-reconnect/.phoenix/gameover-validation/HANDOFF.md). Actual PaneBus tests first failed after positive assignment because the Session remained disconnected. Only Identify moves outside the GameOver gate; station actions retain theirs. |

Earlier evidence remains intact. Frozen `734683cd` [Settings checks](C:/Coding/project-phoenix-v2/.worktrees/review-followup-fixes/.phoenix/native-return-lobby-03/OBSERVATIONS.md) established legible chrome over the ending HUD and working backdrop/resize/F9; [same-pane return](C:/Coding/project-phoenix-v2/.worktrees/review-followup-fixes/.phoenix/native-return-02/OBSERVATIONS.md) established ordinary reclaim/Ready without a new outer document. The roster correction also removes obsolete hull stations once, cancels pending claims, preserves authored participant panes and defers across empty monitor reports. Its [original-branch preservation](C:/Coding/project-phoenix-v2/.worktrees/review-lobby-roster/.phoenix/roster-cleanup-preservation/HANDOFF.md), `a8375a0b…`, needs an Identify fixture under that branch's retained connection owner; that fixture adjustment was reviewed but not executed there. Combined r4 executed the shared production correction.

The r4 [profiled](C:/Coding/project-phoenix-v2/.claude/worktrees/t2-final-integration/.phoenix/review-final/profiled-r4-01/OBSERVATIONS.md) and [no-profile](C:/Coding/project-phoenix-v2/.claude/worktrees/t2-final-integration/.phoenix/review-final/no-profile-r4-01/OBSERVATIONS.md) attempts failed functional acceptance despite passing tests: Escape left Settings open and the acknowledged picker was unseen; neither reached a mission. The [DOM diagnostic](C:/Coding/project-phoenix-v2/.claude/worktrees/t2-final-integration/.phoenix/review-final/key-diagnostic-r4-01/OBSERVATIONS.md) corrects the visual-only Tab interpretation: **Tab arrives as `key:"Tab"`/9 and moves focus**; the blue category was not a focus indicator. Escape arrived as `Unidentified`, `U+001B`, keyCode/which27. Shared focus-trap fix `c621fca2…` accepts legacy fields only for absent/Unidentified named keys; review and 41 JavaScript tests passed before the successful r5c on-screen dismissal. The picker parent remained at opacity0/−28px for 56.1391355 seconds; the Vellum refresh fix above addressed the missing SDK animation advance. Computed-style observation may flush layout. These failed attempts remain separately recorded with normal exits and passing artifact guards.

The later [r5c no-profile failure](C:/Coding/project-phoenix-v2/.claude/worktrees/t2-final-integration/.phoenix/review-final/no-profile-r5c-01/OBSERVATIONS.md) isolates the reconnect defect fixed above: moved Tactical received assignment at 220.744 seconds but kept `1/4 AWAITING CREW` and stayed unready through 330.812. Another recreation repaired its projection; there was no second start. It exited normally after 442.9308631 seconds. The Windows Known Folder lookup ignored the wrapper's APPDATA isolation, so the actual saved layout was restored/read-verified explicitly. None of this failed evidence is replaced by the later scoped reconnect pass. T2 retains ownership of its separate unfinished integration branch; this reviewed batch follows the main publication described in MERGE.md. The historical `734683cd` A/B measurements above do not measure the newly advanced animation workload.


## Rejected attempts and remaining boundary

The first native profiling attempt ended before sampling because the outer PowerShell process supplied an empty surface-capture path. The application rejected it; no comparison was extracted. Fresh outputs 04/05/06 used explicit absolute companion paths, with the unchanged wrapper and independent review. [Failed native evidence](C:/Coding/project-phoenix-v2/.worktrees/review-followup-fixes/.phoenix/native-followup/three-console-01-native-before/FAILURE.md) remains intact.

The first headless matrix stopped after a full control simulation because Windows PowerShell 5 did not retain its exit code. Its null code was correctly rejected. The launcher now opens and retains the process handle immediately and still rejects an unavailable code. Harmless known-exit 0 and 37 probes and independent review passed; all twelve measurements were restarted in fresh `pairs-02` directories. [Rejected headless record](C:/Coding/project-phoenix-v2/.worktrees/review-followup-fixes/.phoenix/streaming-followup/pairs-01/runs/00-combat_test-0-base/analysis.json) and [launcher proof](C:/Coding/project-phoenix-v2/.worktrees/review-followup-fixes/.phoenix/streaming-followup/exit-code-validation/results.json) remain intact.

The quiet-machine block is complete and released. Scale2 human quality/adoption and literal three-1080p hardware acceptance remain separate. The original architecture/reuse branch is preserved, no source or measurement was discarded, and publication status is recorded in MERGE.md. These historical measurements make no new issue-closure or hardware-acceptance claim.
