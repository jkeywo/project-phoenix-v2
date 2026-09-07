# Review and profiling results — 7 September 2026

The controlled captures demonstrate unnecessary repeated HUD work and support P2's revision-based reduction. They do not establish 60 Hz Station rendering or completed visual acceptance. Named renderer and headless attribution support **no functional optimization in P5/P6** on the evidence collected; Combat's occasional streaming spikes remain unresolved. P3 and the P4 presentation pilot are not adopted.

All comparative captures finished before the user released the quiet-machine window. This document records those frozen results. Subsequent functional fixes and their visual validation are separate; implementation and final-gate status remain in [PROGRESS.md](C:/Coding/project-phoenix-v2/.worktrees/review-followup-plan/docs/plans/2026-09-07-review-followup/PROGRESS.md).

## Scope and provenance

The native matrices use Combat Test and Falling Skyway, seed 42, Alliance Destroyer, 40 seconds of warmup and a 30-second observation window. Each arm has **36 selected valid captures**: three repetitions of renderer-only, chrome, one Station and two Stations per World, plus bracketing renderer controls. Rejected runs and explicit replacements remain in the raw records; the background allowance stayed at 1.0 CPU core equivalent, and valid runs had no observed competing compiler. This is bounded background activity, not an idle-machine claim.

The actual renderer was NVIDIA GeForce RTX 5090 Laptop GPU, Vulkan, driver 610.74, under Windows High performance. Viewscreen and Helm were 1920×1080 at scale 1; Tactical was 1920×1200 at scale 1.25. Real console pages were loaded and continuously acknowledged ready/AFK on Backfill. Thus one versus two Stations also changes pixel geometry, and the workload contains no measured human input. See the [hardware record](C:/Coding/project-phoenix-v2/.worktrees/review-followup-plan/.phoenix/profile/hardware-quiet.json) and [baseline report](C:/Coding/project-phoenix-v2/.worktrees/review-followup-plan/.phoenix/profile/baseline-report.md).

Frozen baseline source is `194a057e3809a8098ffe80be2ea495e73e1a8e97`; P2 source is `dc16185cf9abf2a68ac7bcbdcf3ab59d08d2c60c`. Receipts record clean source, executable/PDB hashes, build commands, content, client bundle and applicable SDK hashes.

| Artifact | Profile / features | Executable SHA-256 |
| --- | --- | --- |
| [P1 native receipt](C:/Coding/project-phoenix-v2/.worktrees/review-followup-plan/.phoenix/profile/native/build-receipt.json) | measure / host,ultralight | `a89bb76ac36044303578357c65734f7869b6fa5a7be36706a88faa75399c3130` |
| [P2 native receipt](C:/Coding/project-phoenix-v2/.worktrees/review-followup-plan/.phoenix/profile/native-p2/build-receipt.json) | measure / host,ultralight | `ad5e5a9cd74877568ebcab0625a7e6417975d408cb792b9091ad79a7a0abb0e4` |
| [Release headless receipt](C:/Coding/project-phoenix-v2/.worktrees/review-followup-plan/.phoenix/profile/headless/build-receipt.json) | release / headless | `003c23d0da63dd0d71ed103a5ecb1f6a25b840293d11d58e1014ef0069eccf74` |
| [Named profiler receipt](C:/Coding/project-phoenix-v2/.worktrees/review-followup-plan/.phoenix/profile/attribution/build-receipt.json) | attribution / host,headless,perf | `af1b78f2405bf7392cae57552d1dcdfcb2ab1712a86d136cb3628509ae3d5417` |

Shared content SHA-256 is `9fed941859163fc4742474094f09a7803c7eaf3e1574cee186a47d0375ac9814`; native SDK resources are `a1012a10d9093559d07e3493135e74efd2db70fce54b58d58f23162e2e483b9a`. P1/P2 client bundle hashes are respectively `e569327944fe212efaf89228028cc81b3b32c48230dfa58d1c96ac420eb5b957` and `2dc8dea5e677b582294c5c07efb9a1a60a23617d53b37369f3c8e95cff970754`. Native, headless and named builds are distinct profiles: subtracting their timings is not a controlled renderer-overhead measurement.

All ranges below describe individual run statistics across three repetitions, not pooled percentiles or confidence intervals. Raw links target this machine's ignored `.phoenix/profile` artifacts; large captures and proprietary SDK binaries are not checked in.

## Ultralight: P1 attribution and P2 HUD revisions

| World / condition | Worker iterations/s, P1 → P2 | HUD uploads MB/s, P1 → P2 | Repeated HUD applications/30 s, P1 → P2 |
| --- | ---: | ---: | ---: |
| Combat / chrome | 60.03–60.47 → 60.33–60.47 | 495.45–500.15 → 164.51–170.31 | 1,187–1,200 → 0 |
| Combat / one Station | 27.57–28.33 → 28.83–28.97 | 228.93–235.56 → 175.84–179.99 | 327–359 → 0 |
| Combat / two Stations | 12.30–12.50 → 11.73–12.27 | 102.30–103.96 → 80.46–83.22 | 76–83 → 0 |
| Skyway / chrome | 60.10–60.13 → 60.23–60.73 | 495.45–496.83 → 0 | 1,803–1,805 → 0 |
| Skyway / one Station | 56.47–57.77 → 59.70–60.03 | 456.47–470.57 → 0 | 1,695–1,733 → 0 |
| Skyway / two Stations | 26.70–27.87 → 25.57–32.50 | 221.74–231.69 → 0 | 802–837 → 0 |

P2 eliminates repeated successful HUD applications in all 18 runs with surfaces. Every such run first applies a HUD revision and uploads a visible full frame at 1.154–3.439 seconds, before warmup. Skyway then has no HUD state changes, applications or uploads during measurement. Combat retains dirty animation copies in addition to copies forced by new revisions. Upload MB/s is submitted pixel bytes divided by time, not measured GPU bandwidth.

Console rates are workload dependent. Combat one's console rises from 228.65–235.29 to 238.60–240.81 MB/s with faster worker cadence. Skyway one's console falls from 311.45–326.92 to 173.08–174.18 MB/s while still applying exactly 332 messages per run; clean copy decisions rise from 34–88 to 1,161–1,175. Fewer copies on a slower worker are not independently an efficiency improvement. Per-surface rows and full-copy reasons are retained in the [comparison](C:/Coding/project-phoenix-v2/.worktrees/review-followup-plan/.phoenix/profile/p1-p2-comparison/comparison.json) and [independent interpretation](C:/Coding/project-phoenix-v2/.worktrees/review-followup-plan/.phoenix/profile/p1-p2-comparison/interpretation.md).

Both arms have zero measured failed/deferred pushes, copy failures, upload deferrals, stale-epoch discards and terminal frame loss. Buffer-starved copy attempts are separate: Skyway one falls from 29–110 to zero. Observation-boundary frames upload afterward, with no measured cohort unresolved at capture close. Whole recordings can contain shutdown discards and are not globally loss-free.

Main cadence remains about 16.67 ms while the pane worker runs concurrently. P2 Combat two still spends 54.18–56.21 ms per iteration in global SDK render, about 64–67% of active worker time; pump takes 15.06–17.19 ms. These are aggregate worker costs, not per-view raster attribution or additive main-frame costs. Private-memory peaks show no uniform reduction and measure process memory, not GPU/VRAM.

The five subsequent valid [Combat crossover captures](C:/Coding/project-phoenix-v2/.worktrees/review-followup-plan/.phoenix/profile/combat-crossover) produced, in order: **P1 12.433 Hz; P2 12.433; P2 novsync 12.767; P2 12.367; P1 11.733**. Ordinary P2 lies within bracketed P1 variation, so the first matrix's slight Combat-two separation does not demonstrate a P2 pacing regression. The single novsync observation does not establish repeatable benefit; no presentation default was changed.

**P3 stop decision:** all measured hidden lobby rows apply zero messages and copy zero pixels, leaving no attributed hidden-work reduction to justify adoption. The held implementation is not adopted. **P4:** neither the single novsync pilot nor these memory samples justify adopting cadence or render-scale changes. The proposed 30 Hz service policy remains unadopted; this two-Station rig does not satisfy the original three-Station hardware acceptance in [#1404](https://github.com/jkeywo/project-phoenix-v2/issues/1404)/[#1405](https://github.com/jkeywo/project-phoenix-v2/issues/1405).

## Native 3D renderer: P5

All 18 named control/wrapped/control runs are valid. Main means remain about 16.67 ms. `prepare_windows` takes 10.10–10.29 ms per execution in Combat and 12.95–12.97 in Skyway, but includes swapchain acquisition/presentation waiting. It is not an equivalent CPU computation budget. Combat mesh allocation takes 1.54–1.75 ms, StandardMaterial preparation 0.425–0.430 ms and material bind groups 0.246–0.253 ms. Parallel system wall spans overlap and must not be summed into an exclusive frame budget.

Supported GPU timestamps were obtained: opaque-3D mean durations range 0.206–0.470 ms in Combat and 0.140–0.197 ms in Skyway; transparent-3D means are 0.018–0.042 and 0.023–0.029 ms; bloom means are 0.096–0.120 and 0.108–0.124 ms. These asynchronous pass samples are not a complete presented-frame GPU total or a VRAM measurement.

**Decision: no renderer functional change.** Recurring mesh preparation is measured, but no avoidable asset regeneration is attributed to the critical path. Trail-mesh mutation, LOD prefetch lifetimes and marker discovery are bounded follow-up candidates requiring affected asset IDs/bytes and correctness evidence before optimization. See [renderer attribution and raw run links](C:/Coding/project-phoenix-v2/.worktrees/review-followup-plan/.phoenix/profile/systems-native-runs/renderer-attribution-analysis.md).

## Headless simulation: P6

The six release 60-second runs retain 3,301 measured updates each. Combat mean/p99 is 1.350–1.385 / 2.125–2.218 ms; Skyway is 1.112–1.138 / 1.553–1.636 ms. All finish at tick 3600 with matching per-World digests: Combat `d3af44b6242a4d02`, Skyway `69d4fd9cbded1475`.

Eighteen valid named control/wrapped/control runs reproduce those digests and attribute Combat's six repeatable first-minute spikes to `update_asteroid_window`, followed by FixedLast `sync_authoritative_model_markers`. At those exact ticks, streaming bodies take 13.68–17.27 ms and marker bodies 3.31–5.14 ms. Mean named body-wall sums per update are much smaller for Phoenix Physics (Combat 0.153–0.160 ms), Publish (0.075–0.079), PublishAggregate (0.044–0.047) and Rapier's three phases (0.025–0.027). These categories are distinct; deferred work and observer bookkeeping are reported separately.

The bounded 120-second Combat continuation has three valid repeats, 7,201 raw samples each and matching tick-7200 digest `8b50cca91c63d28b`. In the second minute, mean update is 1.576–1.615 ms, p99 2.172–2.383 ms, maximum 19.423–21.415 ms, with 3–4 updates per run above 16.67 ms. Later spikes remain repeatable by simulation tick; no named spans were captured after tick 3600, so assigning them the same streaming mechanism is an inference.

**Decision: no Physics/publication/Rapier functional change.** Ordinary work has headroom on this machine, but streaming tails are not fixed. Whole-cache cloning during rock spawning and repeated model-marker sidecar resolution are concrete future investigation candidates, not measured sub-operation shares. The named observer's extra Skyway spike at tick 2348 is absent from controls and remains identified as likely bookkeeping/allocation overhead. See [named analysis](C:/Coding/project-phoenix-v2/.worktrees/review-followup-plan/.phoenix/profile/named-headless-analysis.md), [release summaries](C:/Coding/project-phoenix-v2/.worktrees/review-followup-plan/.phoenix/profile/headless-runs/summary.json) and [120-second summaries](C:/Coding/project-phoenix-v2/.worktrees/review-followup-plan/.phoenix/profile/headless-120-runs/summary.json).

## Architecture, native/WASM reuse and remaining acceptance

The review's architecture work deepens ownership of complete script effects, trigger pairing/generations, Power/Control continuation, lobby result application and World materialization. Native/WASM reuse work shares startup-restore policy, connection ownership and catalogue projection. These are implementation/correctness outcomes, not measured performance gains; their issue-specific evidence and remaining integration gates belong to the [implementation ledger](C:/Coding/project-phoenix-v2/.worktrees/review-followup-plan/docs/plans/2026-09-07-review-followup/PROGRESS.md).

During the final on-screen test the user reported uneven cooldown bars and radar updates stopping when the ship was stationary. The final displayed build was **P1**, so that observation predates P2. The investigation found an iframe animation scheduler coverage gap and binary-width cooldown rendering despite existing authoritative durations. Both corrections passed independent review and focused tests. A later real native combat run showed changing phaser and blaster cooldown fractions from authoritative data. The controlled SDK radar checks did not reproduce the original stationary freeze, so the scheduler correction alone is not presented as its confirmed cure.

The deeper review found a separate concrete data-loss path: native queue coalescing treats partial BlackboardUpdate and SimState messages as complete replacements. A repair-only batch can replace the common radar/Helm batch, and a later entity delta can discard another contact's final movement. The native correction is integrated at af0b4609 and independently reviewed; all 182 focused pane tests pass, including real producer-to-bus regressions, privacy withdrawal and retry ordering. A fresh SDK executable at 2ce2cce6 then demonstrated 70 seconds of stationary Helm with changing real contacts and advancing visible radar canvases on both Stations. Its [functional report](C:/Coding/project-phoenix-v2/.worktrees/review-followup-plan/.phoenix/final-native-combat-02/analysis.md) records provenance, manual setup corrections and diagnostic limits. The frozen comparative results do not measure these later client or delivery corrections.

P2 first paint, duplicate suppression and steady upload reduction are evidenced. A later all-AI functional run on the corrected 2ce2cce6 SDK records eight phaser and sixteen blaster countdowns with decreasing interior fractions reaching zero, alongside changing HUD heading, hull and alert state. The [analysis](C:/Coding/project-phoenix-v2/.worktrees/review-followup-plan/.phoenix/final-native-all-ai-01-analysis/report.md) rejects inactive or stale Station documents. A separate [physical-window exercise](C:/Coding/project-phoenix-v2/.worktrees/review-followup-plan/.phoenix/final-native-visual-01/visuals) shows partially filled weapon bars and later empty states, updated HUD text/red alert and F9 reveal/hide. A phaser remains blocked during its active beam even when its post-beam cooldown timer is zero; the display preserves that authoritative readiness rule. These observations verify fractional presentation, not continuous animation smoothness or a console frame rate.

A real mission ending exposed missing Station terminal delivery and empty HUD ending text. The lifecycle dispatch/ordering correction is integrated at 08b8598d, with eight focused Rust checks passing; the old WASM reproduces both faults in a real renderer regression. Updated SDK/browser terminal acceptance and recovery remain pending. A main-frame cadence near 60 Hz is neither console 60 Hz nor an input-latency result. No issue closure, three-Station acceptance, GPU-memory result or general percentage speedup is asserted here.
