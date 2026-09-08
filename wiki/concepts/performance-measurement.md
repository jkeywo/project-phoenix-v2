---
title: Performance Measurement
type: concept
tags: [profiling, headless, renderer, performance]
sources: [src/perf/mod.rs, src/perf/native_frames.rs, src/bin/phoenix_headless.rs, examples/profile_systems.rs, examples/profile_systems/timing.rs, examples/profile_systems/gpu.rs, scripts/profile-provenance.mjs, scripts/profile-analysis.mjs, scripts/profile-native.ps1, scripts/profile-native-matrix.ps1, src/native_host/panes/surface_stats.rs, src/native_host/panes/render_geometry.rs, src/native_host/panes/frame_stats.rs, docs/profiling.md, pasm/spec/architecture/performance-measurement.yaml]
updated: 2026-09-08
---

# Performance Measurement

`src/perf/` provides Phoenix collectors around the shared `vellum-perf` capture contract. The headless runner records update durations and can write a final tick/digest companion after timing closes. `NativeFrameCapture` observes native App cadence, completed fixed ticks, asset readiness and actual window dimensions; it buffers samples until the runner returns.

The `profile_systems` example builds the production headless or native App and optionally wraps named systems. Deferred work has its own spans. Native GPU/pass evidence comes from Bevy's `RenderDiagnosticsPlugin` when the adapter supports it. System wall times can overlap, and nested GPU pass paths are not exclusive costs.

The runners and validity checks live in `scripts/profile-*`; commands and interpretation are in [docs/profiling.md](../../docs/profiling.md). Build receipts pin a clean source revision, executable, symbols, SDK resources and content. Profiling builds use a target directory private to the checkout. Samples, hardware records and raw traces stay under ignored `.phoenix/` directories.

`profile-native.ps1 -Condition three` requires live helm, tactical and engineering Station consoles on the harness's Alliance Destroyer. The optional `profile-native-matrix.ps1 -ThreeStations -Experiment scale2` brackets each experiment with native-scale runs of the same three-console profile; the original renderer/chrome/one/two matrix remains the default. The shared validator rejects an omitted Station or a console that loses readiness during observation.

Native pane observations live in `panes::surface_stats` and `pane_thread`. Copy events retain separate dirty and copied rectangles; forced/failed dirty bounds remain unknown because the pinned SDK wrapper exposes only copied bounds. `--frame-stats` adds literal per-pane rectangle lines to the Lobby log. [The attribution runbook](../../docs/acceptance/1405-surface-attribution.md) describes capture limits and compatibility with earlier schema 1 pixel-only records.

`panes::render_geometry` defines the opt-in `scale2` Console raster experiment selected by `PaneExperiments`. `panes::ultralight` uses reduced view/texture dimensions and device scale while retaining display/input geometry; Lobby and HUD stay native. Source tests do not establish real-display text legibility or a performance benefit.
