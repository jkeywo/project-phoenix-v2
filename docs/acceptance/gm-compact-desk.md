# Compact GM desk acceptance

## Automated evidence (2026-09-22)

The existing uncommitted native GM work is retained. The initial desk is now
Entity Tree, Map, Inspector and Activity. Live layout version 19 performs a
one-time reset with a private, sanitised backup; Layout can restore that backup
with retired panel IDs mapped to their consolidated replacements.

`tests/native_gm_ultralight.rs` runs the real embedded engine at 1920×1080.
Its ignored integration test passed with a native private bridge, non-zero map
canvas, state-driven session controls, a shipless native GM host, pre-start AI
backfill, the actual authored Helm iframe, authoritative takeover, a correlated
helm-thrust command in the applied GM journal, changing live speed readings,
and release/AI hand-back while paused. This is
not a Chromium substitute. It uses the Contract renderer for the simulation;
it does **not** measure Bevy/wgpu texture-upload frame times.

JavaScript tests cover tree grouping, stable row identity, empty worlds, runtime
entities, stations, slots, search and keyboard navigation; layout defaults,
backup remapping, density persistence, tab reorder without iframe replacement,
drag thresholds, docking/resize and release-before-hide; popup defaults and
confirmed-save closure; session states under repeated preset changes; and
station authority, stale frames, release refusal and timeout.

Rust tests cover native operator preference sanitisation and shared layout
fixtures, native shipless scenario ingestion and backfill, and the station
projection's actual selected-hull configuration.

## Interactive regression pass

1. Build current native Ultralight host and client bundle; launch a fresh native
   lobby, choose Host as GM and a scenario. No hull picker should appear.
2. Check the four-pane arrangement. Expand a world/faction/entity/station with
   the separate controls; use arrows, Home/End and Enter, then search. Spawn or
   remove an entity and verify selection, expansion and scroll remain stable.
3. Fill an empty player slot with AI before Start. Mark Ready; verify only
   Ready/Unready appears in the header. Force Start remains in Session.
4. Open Session and Mission through their menu categories. Change Activity's
   source filter; retain distinct action outcomes and session history. Close
   Session and verify critical connection warnings remain visible in the header.
5. Open an Inspector tool and manual save. A refused action/write must retain
   input; confirmed success closes an unpinned popup. Keep open prevents closure.
6. Reorder tabs inside a strip (including an overflowing strip), then drag out
   to float and dock again. Resize docked and floating panes and relaunch.
   Reordering must not reload the console. Toggle touch density and verify
   keyboard focus and readable selected-tab text at both densities.
7. Select a station in the tree: readings must change before takeover. Take
   Over, operate a control and inspect its journal outcome. Switch stations,
   hide the console behind another tab, and close it; each waits for release.
   Docking, resizing and application focus loss must not release. Repeat while
   paused, with a release refusal, during disconnect and after a reconnect.
8. Repeatedly change role presets in Lobby, running, paused and terminal phases.
   The header must never show both Pause and Resume. Test old-layout restoration
   explicitly and verify unrelated operator preferences remain unchanged.

## Performance acceptance — outstanding

No trustworthy pre-change 60-second GM baseline was captured before these
edits. Therefore there is no before/after FPS claim. The real-engine integration
test establishes function, not the requested 60 FPS acceptance target.

Current-tree native measurements are recorded in
[the 22 September performance report](gm-performance-2026-09-22.md): 56.6 FPS
idle, 15.9 FPS running and 11.7 FPS observing the player-slot Helm at 1080p.
These use the optimised `measure` profile, not a shipping release baseline.
Sustained controlled-state measurement remains unestablished. The report records
the active RTX 5090 Laptop GPU, driver, power mode, methodology and limitations.

Use current build receipts and existing `--frame-stats` instrumentation. Record
hardware, build profile/features, bundle hash, scenario, seed and exact layout.
Warm up each condition, then capture 60 seconds at 1920×1080: idle desk, running
mission, observed station, controlled station. Do not run Cargo/trunk concurrently.
Use an isolated operator profile and saves.

Compare frame median/p95, fixed simulation ticks, projection/serialization and
JS work, Ultralight update/render, pixel copy and texture upload separately.
Acceptance is median ≤16.7ms and p95 <25ms with one live console, approximately
60 FPS. Do not lower resolution, simulation frequency or reliable delivery to
meet it. Existing `scripts/profile-native.ps1` measures renderer/crew-pane
conditions; those results must not be relabelled as GM conditions.

Remaining performance work includes a valid sustained-control capture, release
baseline/repetitions, deeper attribution, and lazy construction of editing tools
that still initialise while closed. Closed read-only tools already coalesce
updates, but this is not a claim that every presenter is lazy.

Before any push, run all repository pre-push gates once on the integrated tree,
including PASM validate/scan/traceability and wiki link/source checks.
