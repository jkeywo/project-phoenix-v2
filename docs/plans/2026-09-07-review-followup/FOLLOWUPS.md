# Follow-up implementation and measurements — 8 September 2026

The follow-up removes the measured Combat Test streaming stalls, restores claimable native consoles after return to the lobby, and keeps revealed Settings above the mission HUD. The opt-in `scale2` experiment reduces Ultralight render cost and improves pane-thread cadence in a three-console workload; it does not establish a main-frame improvement or justify changing the default resolution.

This record supplements the historical [results](RESULTS.md) and [execution ledger](PROGRESS.md). It preserves the original measurements rather than replacing their baselines. T2 owns the combined integration, its final gates and publication. The original full architecture/reuse branch remains preserved separately; these performance measurements do not establish that its complete history is included in T2's narrower assembly.

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

The Settings layer fix keeps revealed lobby chrome above the mission HUD. On the frozen candidate SDK, actual UI checks passed in Lobby, InProgress, and the real mission-ending state; F9 hide/reveal and fullscreen/windowed resize kept the popup legible. The backdrop dismissed it. This was Computer Use automation on the actual desktop, not a human usability judgment. See [Settings observations](C:/Coding/project-phoenix-v2/.worktrees/review-followup-fixes/.phoenix/native-return-lobby-03/OBSERVATIONS.md).

The retained-world return fix sends a new Welcome with the selected world/hull and claimable roster. An actual mission ended, then ordinary ReturnToLobby restored both original outer panes to stationless, unready Lobby. Computer Use reclaimed Helm and Tactical, acknowledged each Ready state, and observed the all-crew-ready countdown and both live Station UIs on the next GameStarted. Their outer connections/documents remained the same. This does not assert fresh World materialization or a replayed scenario timeline. See [same-pane return evidence](C:/Coding/project-phoenix-v2/.worktrees/review-followup-fixes/.phoenix/native-return-02/OBSERVATIONS.md).

Those checks exposed two additional issues: native Escape was not forwarded to the SDK, and a world selected from the empty native lobby did not refresh the display station roster. Their isolated fixes have independent source review; the Escape selector/SDK mapping and roster/lifecycle regression checks, plus real profile/no-profile acceptance, belong to the forthcoming combined native artifact. T2 owns that build and its final integration gate. This paragraph is a pending acceptance marker, not a completed runtime claim.

## Rejected attempts and remaining boundary

The first native profiling attempt ended before sampling because the outer PowerShell process supplied an empty surface-capture path. The application rejected it; no comparison was extracted. Fresh outputs 04/05/06 used explicit absolute companion paths, with the unchanged wrapper and independent review. [Failed native evidence](C:/Coding/project-phoenix-v2/.worktrees/review-followup-fixes/.phoenix/native-followup/three-console-01-native-before/FAILURE.md) remains intact.

The first headless matrix stopped after a full control simulation because Windows PowerShell 5 did not retain its exit code. Its null code was correctly rejected. The launcher now opens and retains the process handle immediately and still rejects an unavailable code. Harmless known-exit 0 and 37 probes and independent review passed; all twelve measurements were restarted in fresh `pairs-02` directories. [Rejected headless record](C:/Coding/project-phoenix-v2/.worktrees/review-followup-fixes/.phoenix/streaming-followup/pairs-01/runs/00-combat_test-0-base/analysis.json) and [launcher proof](C:/Coding/project-phoenix-v2/.worktrees/review-followup-fixes/.phoenix/streaming-followup/exit-code-validation/results.json) remain intact.

The quiet-machine block is complete and released. Scale2 human quality/adoption and literal three-1080p hardware acceptance remain separate. The original architecture/reuse branch is preserved, no source or measurement was discarded, and no publication or issue closure is claimed by this follow-up record.
