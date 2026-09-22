# Native Ultralight tuning — 22 September 2026

## Outcome and limits

The retained production change avoids identical DOM writes in the player
station hero bar. Exploratory native player-Helm output increased from 26.4
to 35.7 displayed frames/s with SDK automatic workers. Four workers gave
37.3. This is **not 60 FPS acceptance**. The outer Bevy loop remains near
60 FPS while the embedded UI paints more slowly.

Renderer-worker tuning and split refresh/paint timing were measured through
a local vellum prototype, not a published dependency revision. A reusable,
backwards-compatible vellum API is prepared separately on
`codex/ultralight-render-settings`, based on `606b0c0`; Phoenix's committed pin
is unchanged. No active local Cargo override belongs in a commit.

All rows are single exploratory samples, not three-repeat release acceptance.
No shipping release verification or matched-tick digest comparison was run.
The simulation and reliable command paths were not modified. Mission entities
were not removed, raster resolution was not reduced, and no tick-rate changes
were made. Dynamic entity counts vary during each run and between runs; these
are not identical entity-by-entity replay benchmarks.

## Workloads and provenance

- Source base: `e11aff16`, with working-tree harness and hero-bar changes.
- Build: optimised `measure`, `ultralight`, real CPU-rendered SDK surfaces.
- CPU: Intel Core Ultra 9 275HX; GPU selected by wgpu: RTX 5090 Laptop,
  Vulkan, NVIDIA 610.74 / Windows driver 32.0.16.1074. High performance plan.
- Scenario: `combat_test`, seed 42, selected Alliance Destroyer player hull.
- Each valid capture: fresh process, 40 seconds warm-up, 60 seconds measured,
  no concurrent compilation/tests, buffered telemetry and no per-frame log IO.
- GM observation: standalone native GM, player-slot Helm observed.
- Player: ordinary native participant claims Helm and readies; full-screen
  console at 1920×1080, including its station rail.
- Three screens: normal ship host, GM four-pane desk (no embedded observer),
  ordinary player Helm and the 3-D viewscreen simultaneously. GM on
  `DISPLAY5@1920x1080`, viewscreen on `DISPLAY3@1920x1080`, player on
  `DISPLAY2@1920x1200` at existing 125% scale. The player's CSS viewport is
  1536×960, but its physical surface remains 1920×1200.

Raw captures, executable/source hashes, markers, actual window geometry,
SDK split timings and reports are under ignored
`target/ultralight-tuning-2026-09-22/`. `three-monitor-hardware.json` records
the fresh OS display inventory. Each row below names its capture directory.
The reporter rejects missing live readings, pauses, reloads, changed geometry,
incorrect roles and truncated telemetry; all listed rows have no rejection
reasons. They fail the displayed-UI 60 FPS target.

## Measured displayed UI rate

| Capture | Change / workers | UI FPS | UI-thread mean ms |
|---|---|---:|---:|
| `gm-auto` | GM observation, automatic | 33.7 | 29.35 |
| `gm-2` | GM observation, 2 | 40.0 | 24.85 |
| `gm-4` | GM observation, 4 | 40.1 | 24.74 |
| `player-auto-ready` | Player, original hero bar, automatic | 26.4 | 37.84 |
| `player-2-ready` | Player, original hero bar, 2 | 22.9 | 43.68 |
| `player-4-ready` | Player, original hero bar, 4 | 26.2 | 38.14 |
| `player-auto-quiet-hero` | Conditional hero writes, automatic | 35.7 | 27.85 |
| `player-4-quiet-hero` | Conditional hero writes, 4 | 37.3 | 26.56 |
| `multi-auto-run` | Three screens, conditional hero writes, automatic | 35.4 both | 28.26 |
| `multi-4` | Same three screens, 4 | 37.4 player / 37.4 GM | 26.75 |

Outer frame medians were 16.25–16.76 ms and p95 19.93–22.56 ms in these
retained-configuration samples. That does not establish embedded UI cadence.
Player callbacks sometimes exceed displayed frames because several updates
can precede a paint. The report counts uploads separately for each visible
GM/console surface and includes zero for a stalled visible surface.

Actual total ship counts during measurement: GM 3–4, player 4–5, three-screen
3–5, including mission NPC ships. Mission entity counts: GM 115–224, player
116–256, three-screen 71–73. The measured player is explicitly Helm on the
selected Alliance Destroyer, not an arbitrary first station. Fixed ticks and
simulation time pass the existing real-time/rate checks throughout.

## Attribution and rejected experiment

In `multi-4`, mean shared UI-thread costs were:

| Phase | ms |
|---|---:|
| SDK update / timers | 6.90 |
| Bridge pump | 4.78 |
| Display refresh / animation callbacks | 2.34 |
| SDK paint | 11.05 |
| Pixel copy | 1.67 |

These are sequential owner-thread spans, not per-view raster costs or summed
worker CPU time. Refresh plus paint equals the existing render span. Global
SDK rendering remains shared by all views. Four workers are an opt-in tuning
candidate, not a universal default: two workers helped GM but hurt player.

Removing the player's timer-backed rAF shim was tested and **reverted**:
`multi-4-display-clock` gave 35.7 FPS / 28.02 ms, while
`player-4-display-clock` fell to 25.9 FPS / 38.68 ms. Work moved into refresh
callbacks; it did not disappear. The final source retains the existing parent
and child scheduling contract. Those two capture binaries are rejected
experiments, not the final source; rebuild before using the harness again.

Remaining dominant costs are JS/bridge work and SDK paint, not pixel copying.
The runtime already owns a dedicated UI thread. No unsafe cross-thread SDK
access, second renderer, or GPU-driver replacement was introduced.

## Reproduction and validation

Build the harness with `cargo build --profile measure --features ultralight
--example profile_gm`. Build the client bundle with
`node scripts/build-client.mjs`. Use a new output directory for every run:

```powershell
powershell -NoProfile -File scripts/profile-gm-current.ps1 -Stage player -Output target/player-capture
powershell -NoProfile -File scripts/profile-gm-current.ps1 -Stage multi -BridgeProfile <three-role-profile.toml> -Output target/multi-capture
node scripts/profile-gm-report.mjs target/multi-capture
```

An experimental wrapper build additionally needs `-UltralightExperiment`,
`-RendererThreads <count>` and `-WrapperSource <compiled-runtime.rs>`.
It must emit `ultralight.csv`; passing those flags to the ordinary pinned
wrapper does not enable tuning and is rejected when timing evidence is absent.
Re-inventory monitor identities rather than copying this machine's profile.

Validation: 104 targeted Vitest tests passed (hero bar, native pane scripts,
iframe scheduler, native display-rate accounting); Rust formatting passed;
measure harness builds passed. The new vellum API passed 38 SDK-free unit
tests, one doctest and its real-SDK animation/dirty-pixel regression with four
workers and both render paths. SDK-enabled targeted clippy passed in vellum.
Phoenix's final `cargo check --offline --locked --features ultralight --example
profile_gm` passed against the unchanged pinned wrapper. PASM validate, scan
and traceability passed in both repositories (with warnings); affected wiki
sources and local links resolve. Full pre-push gates and release acceptance
remain required before publishing; no push is part of this pass.

Incomplete setup attempts are excluded: player runs without Ready had hidden
consoles; the first three-monitor launcher stalled while serialising an
extended PowerShell string before opening the app. Neither is performance
evidence. The launcher now records the profile as plain file text.
