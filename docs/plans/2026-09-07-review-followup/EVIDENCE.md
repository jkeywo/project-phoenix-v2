# Evidence and scope for the follow-up plan

This note makes the essential review evidence available with the plan. Full source references, captures, scripts and binary provenance remain in the [original review pack](C:/Coding/project-phoenix-v2/.phoenix/reviews/2026-09-07/README.md). The large raw artifacts and proprietary SDK binaries are not copied into this branch.

## Source and active work

- Reviews were anchored to source commit 99932cfdbcba290efadd5c389aa0cc59a75124e8.
- This planning worktree starts at committed main 40af72382d731c789d3b904bd6e86ae5a73e1f60. The intervening commit repairs CI/demo/smoke dependencies; the reviewed host-policy findings were checked again against the committed source.
- Main contains another task's uncommitted pane-thread implementation and supporting metadata. The updated user-supplied AGENTS instructions describe separate pane-iteration and main-frame statistics. That newer behavior is the implementation baseline to integrate when delivered, not a performance result established by the earlier captures.
- [#1404](https://github.com/jkeywo/project-phoenix-v2/issues/1404) and [#1405](https://github.com/jkeywo/project-phoenix-v2/issues/1405) were open when inspected on 7 September 2026. #1404's last published progress records slices 1–4 on main; local dirty work goes beyond that. Do not equate an open checkbox with absent code or an edited file with accepted delivery.
- Related existing work: [#1363 native Load Game](https://github.com/jkeywo/project-phoenix-v2/issues/1363), [#1181 bridge Resources](https://github.com/jkeywo/project-phoenix-v2/issues/1181), and [#1362 World/Ship pickers](https://github.com/jkeywo/project-phoenix-v2/issues/1362). A targeted tracker search identified these overlaps; it is not an exhaustive duplicate audit. Check again before publishing new issues.

## What was measured

The rig was an Intel Core Ultra 9 275HX with 24 logical processors and an NVIDIA RTX 5090 Laptop GPU, Vulkan, driver 610.74. The Viewscreen and Helm/Tactical Station screens were 1920×1080 at scale 1. Both scenarios used the Alliance Destroyer and seed 42.

| Evidence | Observation | What it supports |
|---|---|---|
| Six original release headless runs | Three 60-simulation-second captures each for Combat Test and Falling Skyway; 3,301 measured updates after excluding 300. Means 1.32–2.35 ms; Combat Test had occasional 26.6–39.6 ms maxima. | Ordinary opening-minute work has headroom; investigate the spikes and named Systems before broad engine changes. |
| Eight original native conditions | Both scenarios, each with renderer only, HUD/lobby, one Station, and two Stations; 40 s warm-up + 30 s observation. | Real workload phase/pixel evidence. Compiler load reached roughly 10–17 cores in many runs, so FPS differences cannot establish pane-count scaling. |
| HUD-only native captures | One forced 2.07-million-pixel copy and about 8.29 MB uploaded per frame at 1080p. | The source-traced repeated-HUD-update mechanism is a concrete optimization target. Roughly 497.7 MB/s at 60 FPS is calculated payload volume, not measured bus throughput. |
| Instrumented headless controls | Phoenix Physics about 0.49–0.50 ms/update; Rapier phases together 0.10–0.14 ms/update. Final digests match within both control/wrapped/control comparisons. | Target Physics/publication Systems; do not conflate their work with Rapier. Changing build load prevents an isolated overhead estimate. |
| Instrumented native renderer pair | Control 17.654 ms/main-frame; wrapped 18.744 ms. Largest accumulated render categories: bind-group preparation 5.379, view management 3.769, asset preparation 3.543 ms/main-frame. | Useful targets for named critical-path attribution; parallel System wall durations overlap and cannot be summed into a frame budget. |

Every measured Station was confirmed visible, ready, AFK/Backfill, with its real console mounted before warm-up ended. A brief startup transition through human control means pane/no-pane runs do not prove exact simulation equivalence. The two-Station matrix does not satisfy #1404's separate three-Station hardware criterion.

The additional attribution harness used a later pinned measure library containing concurrent pane changes, with no panes opened in its renderer test. It is distinct from the frozen release/native baseline executables. ExtractSchedule deferred work was corrected to count once under ExtractCommands. Windows refused kernel CPU sampling, and no GPU timestamps were obtained. The renderer residual is unattributed time.

See [performance methodology](C:/Coding/project-phoenix-v2/.phoenix/reviews/2026-09-07/performance.md), [CPU attribution](C:/Coding/project-phoenix-v2/.phoenix/reviews/2026-09-07/cpu-attribution.md) and [tooling limitations](C:/Coding/project-phoenix-v2/.phoenix/reviews/2026-09-07/profiling-tools.md) before reproducing or comparing these results.

## Review-to-plan coverage

| Review finding | Plan slice |
|---|---|
| Native startup-restore can remain suspended after phase change | R1 |
| Native/browser Session connection ownership differs | R2a/R2b |
| Native catalogue omits provenance and active packs | R3 |
| Native deferred loading transcribes World startup | A5 |
| Script callers apply six effect collections independently | A1 |
| Trigger state/handler/generation pairing relies on convention | A2 |
| Legacy System continuation knowledge lives in snapshot orchestration | A3a/A3b |
| Lobby callers repeat Ship-present/absent result application | A4 |
| Main-thread Ultralight scaling | Existing #1404 acceptance, G0 |
| Repeated unchanged HUD scripts and full copies | P2 |
| Hidden page update/raster work | P3 |
| Unattributed residual and pooled surface counters | P1/P5 |
| Full-resolution staging/memory scaling | Conditional P4 |
| Headless/renderer profile uncertainty and attribution gaps | G0/P5/P6 |

Source-traced correctness failures still need executable regressions through production adapters. Architecture findings are maintenance opportunities, not reproduced defects or measured speedups. Preserve the already-shared simulation, Boot Profiles, Admission, phone JS renderers and existing copy/upload/queue improvements.

The [implementation plan](C:/Coding/project-phoenix-v2/.worktrees/review-followup-plan/docs/plans/2026-09-07-review-followup/PLAN.md) carries the dependencies, acceptance checks, chosen policies and stopping criteria. This branch changes planning documents only.
