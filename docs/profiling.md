# Reproducible performance captures

Use a committed checkout and a quiet machine. These scripts preserve the existing vellum-perf capture contract; they do not adopt budgets or add a CI timing gate. Native `measure` and headless `release` are separate comparison groups. Browser automation and real browser rendering require their own provenance.

Artifact builds use this checkout's private `.phoenix/profiling-target` directory. The explicit Cargo target directory overrides a shared `CARGO_TARGET_DIR`, preventing another worktree from supplying this package's executable through a stale shared fingerprint. Cargo's emitted artifact and SDK paths are frozen in the receipt after the clean revision is checked again.

## Prepare once per source revision

Build the normal browser/client bundle, then copy it with the profiling readiness shim. Production GUI files stay untouched. The shim uses ordinary participant commands to ready a Station and then acknowledge AFK, leaving the real Helm/Tactical console visible with Backfill. It reports only lifecycle and dimensions to a loopback listener, never identity tokens. The brief startup admission differs from a run with no participants, so do not claim native pane/no-pane digest parity.

Run from the checkout in PowerShell, using fresh output directories under ignored `.phoenix/`:

```powershell
./scripts/profile-prepare-client.ps1 -SourceBundle ./dist -Destination ./.phoenix/profile/client
./scripts/profile-hardware.ps1 -Output ./.phoenix/profile/hardware.json
node scripts/profile-provenance.mjs build . ./.phoenix/profile/native native ./.phoenix/profile/client
node scripts/profile-provenance.mjs build . ./.phoenix/profile/headless headless
```

A build receipt is written only after a successful Cargo build while HEAD and the clean working tree remain unchanged. The receipt pins executable, available PDB, SDK DLLs and resources, authored assets, instrumented bundle, source revision, features and Cargo profile. Cargo's build-script output selects the exact SDK used by the build; its resources are mounted in each private content directory so frozen executables do not depend on another checkout's ICU/certificate files. `measure` currently strips symbols: a null PDB hash is an explicit limitation, and named stack attribution needs a separate symbol-compatible build. Hashing occurs outside observation. Asset junctions in copied bundles refer to immutable inputs; changing them invalidates the receipt.

The hardware helper records `gpu`, `driver`, `powerMode` and `displays` using read-only Windows queries. Every display entry names its current desktop monitor identity, pixel width/height and DPI/scale; the GPU inventory can also report a panel's native mode, which is not necessarily its current desktop mode. Retain the host's actual adapter/window records alongside it. Use three explicit bridge profiles in a profile directory: `zero.toml` (1080p Viewscreen), `one.toml` (the same Viewscreen plus a Helm Station), and `two.toml` (plus Tactical). Monitor ids come from this machine's bridge setup. Check that the loaded windows match the profiles before comparisons; record a different-size Station as its actual workload.

## Native matrix

```powershell
./scripts/profile-native-matrix.ps1 -Receipt ./.phoenix/profile/native/build-receipt.json -ContentRoot . -ProfileDirectory ./.phoenix/profile/profiles -Hardware ./.phoenix/profile/hardware.json -ClientDirectory ./.phoenix/profile/client -Output ./.phoenix/profile/native-runs
```

The matrix rotates Combat Test/Falling Skyway and renderer-only/HUD-lobby/one/two Station conditions across three repetitions, with renderer controls before and after each group (36 runs). Every run uses seed 42, Alliance Destroyer, 40 seconds warm-up and 30 seconds observation. The runner refuses active compilers, saves process CPU/private memory samples, isolates saves/APPDATA/content working directory, binds delivery to loopback, and closes only its own process. The child exits itself at the capture bound; an early or forced exit remains incomplete.

`frames.json` wraps a vellum capture with completion and clock-origin metadata. Raw `native.frame` samples measure consecutive App `First` schedules, including presentation waits; `native.elapsed` aligns the observation interval and `native.fixed_ticks` counts the completed interval's fixed updates. Once-per-second workload records retain the production asset-preload gate, failed scene count and actual fullscreen monitor/physical window/scale values. These must match the profile before and throughout measurement. Preload completion describes its discovered startup assets, not every future streamed object. These are neither GPU timings nor exclusive main-thread costs. Samples are buffered, written only after the runner returns, and excluded from authoritative state. Warm-up excludes intervals crossing the boundary. Run percentiles come from raw samples, never an average of logged window percentiles.

`summary.json` retains diagnostic results and refuses comparative status for missing provenance, changed inputs, incomplete capture, runtime/content errors, observed compilation, excess background CPU, or a Station not visibly ready/AFK/Backfill before observation and live through its end. The default background allowance is one CPU core; an individual runner can set `-MaxBackgroundCpuCores` explicitly. Process polling is sampled evidence, so short activity between polls can be missed. Keep raw logs and process evidence with conclusions.

Pane-thread iterations, main-frame upload work and global SDK update/render work have different denominators. The ordinary frame-stats log is retained, but rounded log windows do not establish exact event totals or run p99. The #1405 surface collector supplies that additional evidence; until present, mark event volume/age unavailable rather than deriving false precision. Likewise, native App cadence says nothing about GPU duration or frame presentation on every monitor.

## Headless matrix

```powershell
./scripts/profile-headless.ps1 -Receipt ./.phoenix/profile/headless/build-receipt.json -ContentRoot . -Hardware ./.phoenix/profile/hardware.json -Output ./.phoenix/profile/headless-runs
```

This runs both worlds three times for 60 simulation seconds, seed 42/Alliance Destroyer, excluding the first 300 sampled updates. It stores existing `sim.tick`/`sim.run` series and run reports plus a companion final tick/digest computed after timing has closed. Matching final digests corroborate repeated continuation, not every intermediate state. The summary refuses mismatched continuations or contention. Compare update counts and logical ticks separately when changing pacing.

Keep SDK binaries, copied bundles, raw captures and local hardware/profile files out of Git. Commit the repeatable commands, compact findings and limitations. Accept an optimization only when its change exceeds bracketing-control variation and correctness/input checks still pass. A documented no-change decision is a valid profiling result.

## Named CPU and supported GPU attribution

The `profile_systems` Cargo example uses the production headless/native App builders with Bevy's development-dependency debug names and an `attribution` profile retaining symbols. It wraps existing systems without adding schedule dependencies and records bounded raw wall spans, including deferred work separately. The control mode uses the same executable and workload with wrappers and GPU diagnostics disabled. This symbol-compatible example is a separate comparison group from the ordinary native/headless binaries.

```powershell
node scripts/profile-provenance.mjs build . ./.phoenix/profile/attribution attribution
./scripts/profile-systems.ps1 -Receipt ./.phoenix/profile/attribution/build-receipt.json -ContentRoot . -Hardware ./.phoenix/profile/hardware.json -Runtime headless -Mode wrapped -World combat_test -Output ./.phoenix/profile/named-combat
./scripts/profile-systems.ps1 -Receipt ./.phoenix/profile/attribution/build-receipt.json -ContentRoot . -Hardware ./.phoenix/profile/hardware.json -Runtime native -Mode wrapped -World falling_skyway -Profile ./.phoenix/profile/profiles/zero.toml -Output ./.phoenix/profile/named-skyway-render
```

For each world/runtime, run control → wrapped → control three times, rotating world order. Headless excludes the first 300 updates and writes the final continuation digest; compare it with both controls. Native observes the 40–70 second window using the exact App-cadence collector's monotonic origin. Exclude spans crossing either edge when comparing complete intervals. Native update tags are the main counter observed at span start, not causal render-frame ids. Retain raw spans and report the combined CPU-wrapper/GPU-query overhead against bracketing controls.

Wrapped native runs also install Bevy 0.18's `RenderDiagnosticsPlugin`. `elapsed_gpu` paths, when actually present, report GPU timestamp milliseconds; `elapsed_cpu` paths are CPU wall milliseconds, and pipeline counters are counts. Unsupported adapters yield no GPU result. Bevy delivers the latest completed render batch asynchronously, so these are delivered diagnostic samples, not coverage of every rendered frame. All same-path rows in a delivered batch are retained, and a path filling its retained history is flagged as potentially truncated. Parent/pass paths are nested and cannot be summed as exclusive GPU work. Likewise, system durations overlap across threads; ExtractSchedule's deferred work runs inside ExtractCommands and must not be counted twice.
