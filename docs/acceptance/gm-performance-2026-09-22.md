# Current native GM performance — 22 September 2026

Current-working-tree measurements only. The initial table predates optimisation.
These are single local samples, not a clean-release benchmark or a claim
that the performance acceptance target has passed.

## Results

Each accepted run used a fresh native process, 40 seconds of warm-up followed by
approximately 60 seconds of measurement, a 1920×1080 physical window/raster at
scale 1, and the default four-pane GM layout. All 90 assets were ready with no
failed GLBs. Raw captures completed successfully without omitted surface events.

| State | Mean FPS | Median frame | p95 frame | p99 frame |
| --- | ---: | ---: | ---: | ---: |
| Idle GM desk, before Start | 56.6 | 17.0 ms | 29.2 ms | 32.3 ms |
| Running mission, console closed | 15.9 | 63.1 ms | 85.3 ms | 97.6 ms |
| Running, observing player-slot Helm | 11.7 | 78.1 ms | 148.3 ms | 175.4 ms |
| Sustained controlled player-slot Helm | Not established | — | — | — |

The observed player console received 686 live updates during its 60.152-second
sample and remained on the same player ship in observation mode. The earlier
NPC-console sample is excluded from this table.

**Controlled is deliberately not assigned an FPS result.** Initial takeover was
confirmed, but a first attempt lost takeover, a player-ship repeat resized and
reloaded during warm-up, and the final attempt recorded loss of confirmed
ownership before sampling. The final attempt was stopped. The cause of the
ownership loss has not been established; this measurement task did not repair
or bypass station admission. Treating those runs as sustained control would be
misleading.

## Conditions and limitations

- CPU: Intel Core Ultra 9 275HX, 24 logical processors.
- Active renderer: NVIDIA GeForce RTX 5090 Laptop GPU, Vulkan, driver 610.74
  (Windows driver 32.0.16.1074). Intel Graphics 32.0.101.8628 is also installed.
- Windows High performance power plan. Attached displays reported 1920×1080 at
  60 Hz and 3840×2400 at 120 Hz; the measured window was 1920×1080.
- Scenario: `combat_test`, seed 42, standalone native GM; player slot uses the
  default Alliance Destroyer with AI backfill.
- Build: optimised Cargo `measure`, `ultralight` feature, opt-level 3, thin LTO,
  16 codegen units. **Not the shipping full-LTO `release` profile.**
- Base revision: `fa7fb3f1f9ef9d23f58b81e04e4e0fc14df9ab2b`, with the existing
  uncommitted GM desk changes. Per-run manifests record source-file hashes.
- Executable SHA-256:
  `E9506ACB2B43373DBD7B0A40A6CF1A835B55D17F6F31FA0114FCC98D66CCA7A1`.
- `examples/profile_gm.rs` uses the production native App, winit/wgpu and
  Ultralight surface stack, not the rendererless test stack. Its driver uses
  ordinary GM adapters. Preferences use a benchmark-only in-memory adapter;
  no user layout is loaded, migrated or written. No cloud/crew ingress is added.
- Existing native frame and surface observers and frame-stat logging were
  enabled. No compiler/test workload ran during accepted measurement windows.
  Other desktop activity was not isolated or quantified.
- FPS is reciprocal mean time between Bevy `First` schedules, including waits.
  Percentiles come from individual frame samples, not averaged log percentiles.
  These are not GPU execution timestamps or independent display-present events.

## Attribution available from this capture

| Counter | Idle | Running | Observed player Helm |
| --- | ---: | ---: | ---: |
| Fixed-update work, approximate ms per native frame | 3.3 | 48.7 | 69.0 |
| UI bridge pump, mean ms per worker iteration | 0.09 | 26.12 | 19.89 |
| Ultralight render, mean ms per worker iteration | 0.09 | 10.65 | 13.91 |
| Pixel copy, mean ms per worker iteration | 0.002 | 0.44 | 0.35 |
| Worker iterations per second | 60.4 | 23.4 | 24.6 |
| Surface uploads per second | 1.0 | 16.4 | 17.0 |
| Upload payload, MB per second | 0.15 | 133.24 | 101.57 |

Fixed-update means are frame-count-weighted from complete one-second reporting
intervals within each sample and include more than physics: projection and
serialization work have not been isolated from other scheduled work. Worker
phases run concurrently with the main loop and must not be added to native
frame time. Pump includes bridge evaluation/JavaScript; render is not a
per-panel attribution. Upload rate is not visible UI FPS, and low idle upload
rate is expected when the desk has no changing content.

The current running measurements miss the approximately 60 FPS / median
≤16.7 ms / p95 <25 ms acceptance target. A causal optimisation claim or a reliable
controlled-state baseline requires further work; neither is asserted here.

## Local artifacts and reproduction

Accepted artifacts are under `target/gm-performance-2026-09-22/`:

- `idle-3/`
- `running/`
- `observed-player/`

Each contains `manifest.json`, `stages.jsonl`, `frames.json`,
`frames.surfaces.json`, `summary.json` and logs. Other folders are setup,
different-target, resized or invalid-control attempts and are not table inputs.
Artifacts are local/ignored; this document retains the reported results.

Build with `cargo build --profile measure --features ultralight --example profile_gm`.
The four pinned Ultralight DLLs must be staged beside the example executable on
Windows, as for a native host. Run `scripts/profile-gm-current.ps1` with a new
`-Output` directory and `-Stage idle`, `running`, `observed` or `controlled`.
Analyse with `node scripts/profile-gm-report.mjs <output-directory>` and inspect
`stateVerified`, capture completeness, dimensions and omitted-event counts
before reporting a result. Do not resize or interact with the capture window.

## Optimisation baseline and attribution

The subsequent quiet `baseline-verified` run (unchanged production code,
per-frame log writes suppressed) measured **14.04 FPS**, median **64.73 ms**,
p95 **123.28 ms**. Its player Helm stayed live and observing, with 763 updates
over the measured minute; continuous selection, geometry and iframe checks
reported no failures. This harness revision omitted the surface-capture
installer, so it supplies cadence/workload evidence only, not complete raster
attribution or acceptance. That installer is restored in the current harness.
`baseline-quiet` is excluded because compilation overlapped its sample.

The separate `baseline-named` trace uses that same unchanged runtime binary.
Across its 85-second diagnostic window, `publish_local_projection` accumulated
40.59 s / 5,098 calls and `publish_station_projection` 12.18 s / 5,099 calls,
versus 1,110 presentation frames. `feed_projections` accumulated 3.66 s.
These are system wall spans, not mutually exclusive CPU totals. The diagnostic
window is intentionally distinct from the 60-second acceptance interval.

The current harness adds buffered tick/time/entity/ship samples, optional
`-Diagnostic` named-system spans, and continuous console checks. Acceptance
must use the non-diagnostic mode and reject missing/truncated telemetry,
resizes, reloads, stale readings, wrong selection and non-real-time simulation.
Optimised measurements below do not yet establish 60 FPS acceptance.

`optimised-pass1` is an **excluded exploratory capture**, because PASM validation
overlapped it. Its outer-window cadence was 37.24 FPS (median 26.68 ms,
p95 31.71 ms), but the console received only 798 updates/minute (median update
interval 74 ms). The entity channel delivered about 1.36 million UTF-16 code
units per update; the Station channel about 182 thousand. This identified
closed Inspector schema traffic as the next target rather than establishing
acceptance. The launcher now refuses active PASM/uv validation as well as builds.

### Selected-console projection pass

`optimised-pass2` is a complete, uncontended measure-profile sample: **60.00 FPS**
outer-window cadence, median **16.36 ms**, p95 **20.38 ms**, over 60.15 seconds.
Continuous health checks reported no failure; simulation advanced at its normal
60 Hz in real time, with the same player-slot Helm observed throughout.
It is **not a performance acceptance pass**: the embedded console delivered
1,667 measured changing readings (about 27.7/s), with median interval 35 ms and
p95 42 ms. The reporter now distinguishes `sampleValid` from `accepted` and
checks console cadence as well as outer-window cadence.

The entity payload fell from the exploratory pass's ~1.36 million to ~15,171
UTF-16 code units/update, and the station payload to ~89,769. Worker means were
10.12 ms bridge/JavaScript, 24.78 ms Ultralight rendering, and 0.81 ms copying.
Rendering remains the dominant measured cost; these concurrent worker times
must not be added to the outer window's frame time.

The new headless regression compares authoritative digests and GM journal
contents at matching ticks with presentation enabled/disabled and changing
subscriptions. It passes, as does selected-detail/topology/world-invalidation
coverage. This does not establish sustained station takeover, which remains
separate from this performance workload.

### Incremental painting pass — target still unmet

`optimised-pass3` is a complete, uncontended measure-profile run, with 40 seconds
of warm-up and 60 seconds of measurement at 1920×1080. The outer window achieved
**60.00 FPS**, median **16.21 ms**, p95 **20.92 ms**. The observed player Helm
remained live, but delivered only **1,981 changing readings/minute (~33/s)**,
median interval **30 ms**, p95 **36 ms**. It therefore **fails acceptance**.
Simulation continued in real time at its existing fixed rate.

Worker mean spans were **6.25 ms** bridge/JavaScript, **22.96 ms** render,
**0.80 ms** copy, and **30.27 ms** total. A separate diagnostic capture measured
the map animation callback at ~1.64 ms (previously ~5.96 ms) and the console
animation callback at ~1.50 ms. These callbacks execute inside the render span;
do not count them twice or describe that entire span as pure raster time.

`opaque-staging` tested opaque offscreen staging canvases under the same workload:
outer **60.00 FPS**, median **16.29 ms**, p95 **20.60 ms**; Helm **1,965 changing
readings/minute**, interval median **30 ms**, p95 **36 ms**. Worker render averaged
**23.21 ms**, total **30.48 ms**. No useful improvement was established, so the
staging experiment was removed; direct drawing and the full-resolution grid
cache remain. Diagnostic-only hiding of the console reduced render time to
~15.52 ms; hiding the map reduced it to ~20.93 ms. Neither is an admissible fix.

The remaining measured bottleneck is the embedded render/animation pipeline,
not outer-window cadence or pixel copying. No renderer replacement, resolution
reduction, simulation-rate reduction or mission simplification was made.
The three successful acceptance captures, idle/closed regression captures and
shipping-release verification remain **outstanding**, not waived. The current
harness additionally records world/mount generations; the captures above predate
that addition and instead checked iframe identity and selection continuously.

Latest targeted JavaScript validation: 254 tests across nine suites passed.
The 15 GM projection unit tests and hidden-surface pumping/reveal regression
also passed; formatting and affected wiki link/source checks passed.
Native measure builds and browser WASM checking passed; headless projection
regressions compare matching-tick digests and journals, but do not prove a
non-empty station-command journal. Full pre-push gates have not been run and
nothing has been pushed. This is measured progress, not completion of the plan.
