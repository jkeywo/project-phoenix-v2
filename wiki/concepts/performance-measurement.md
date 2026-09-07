---
title: Performance Measurement
type: concept
tags: [profiling, headless, renderer, performance]
sources: [src/perf/mod.rs, src/perf/native_frames.rs, src/bin/phoenix_headless.rs, examples/profile_systems.rs, examples/profile_systems/timing.rs, examples/profile_systems/gpu.rs, scripts/profile-provenance.mjs, scripts/profile-analysis.mjs, docs/profiling.md, pasm/spec/architecture/performance-measurement.yaml]
updated: 2026-09-07
---

# Performance Measurement

`src/perf/` provides Phoenix collectors around the shared `vellum-perf` capture contract. The headless runner records update durations and can write a final tick/digest companion after timing closes. `NativeFrameCapture` observes native App cadence, completed fixed ticks, asset readiness and actual window dimensions; it buffers samples until the runner returns.

The `profile_systems` example builds the production headless or native App and optionally wraps named systems. Deferred work has its own spans. Native GPU/pass evidence comes from Bevy's `RenderDiagnosticsPlugin` when the adapter supports it. System wall times can overlap, and nested GPU pass paths are not exclusive costs.

The runners and validity checks live in `scripts/profile-*`; commands and interpretation are in [docs/profiling.md](../../docs/profiling.md). Build receipts pin a clean source revision, executable, symbols, SDK resources and content. Profiling builds use a target directory private to the checkout. Samples, hardware records and raw traces stay under ignored `.phoenix/` directories.
