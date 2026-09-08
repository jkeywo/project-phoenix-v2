# Observed phase timing (#1400 slice 5)

The existing non-recording `phoenix-headless --perf-capture <PATH>` path now
owns a live span collector outside the App. `src/perf/phase_trace.rs` observes
Bevy's actual FixedUpdate/system spans; `src/perf/phase.rs` reduces their paired
intervals and feeds the same Recorder used by the harness tick sampler.
Native compilation, real coverage, CLI capture and measured/unmeasured state and
census parity passed on the preserved declaration-stage source. The phase
implementation is unchanged in the post-split candidate; this is bounded
correctness evidence, not a comparative timing baseline.

No clock-reading ECS Resource/system, scheduling edge, RNG operation or digest
field is added. Ordinary runs retain LogPlugin and instantiate no collector.
The headless build feature enables the already-pinned Bevy ECS crate's trace
support and real debug names, including release builds. This is a compile-time
observability cost of headless; ordinary browser/native builds without headless
retain their previous features. It does not enable Bevy's unrelated renderer
tracing or LogPlugin trace-error layers, and adds no new command-line flag.

## Capture and attribution

The profiler installs one process-global subscriber before App construction
creates cached system spans. Its worker-visible layer pairs span entries/exits
by span identity and thread. A separate formatting layer retains normal
headless `--log`/`RUST_LOG` filtering and stderr output; an error-only log filter
cannot suppress the collector's INFO spans. A previously installed global logger
is an explicit setup error. There is no temporary thread-local dispatcher and
no collector handle stored in the World.

The collector records raw names first, then reads the initialized schedule
AFTER ordinary app.update(). It never eagerly initializes queries/components
before Startup, registers a probe, or reorders systems. Per-frame reduction is
outside the `sim.tick` bracket. The actual FixedUpdate span can include its
first ordinary initialization cost; invocation IDs identify those executions,
not inferred game tick numbers or rendered frames.

Only unique full system names with one unambiguous phase are attributed.
Duplicate schedule names, duplicate live spans, composed children not present in
the schedule, and nested/auxiliary work remain explicitly unattributed. Ambiguity
is retained at entry, so a temporary duplicate closing before drain cannot make
its earlier work appear unambiguous. The outer FixedMain facilitator is not
mistaken for a nested FixedUpdate system. A changed schedule, unsupported nested
FixedUpdate, unbalanced spans or undrained buffer overflow fails the capture
instead of producing a plausible incomplete pass.

For each invocation the reducer preserves:

- Total observed FixedUpdate elapsed time.
- Each observed phase's interval union, first-entry-to-last-exit envelope and
  supplied interval count. Parallel/nested overlap is counted once in the union.
- Unattributed work and time outside the union of all observed intervals.

Missing phases are absent, not zero samples. The union is observed execution
wall time, not CPU time; the envelope includes internal gaps. Neither is an
inclusive SimSet boundary measurement. Unobserved time includes conditions,
scheduler/deferred work and coverage gaps, so it is not pure executor overhead.
Tracing callbacks and synchronization impose observer overhead; captures are
instrumented evidence, not the cost of an uninstrumented run. Compare coverage
and the same build/configuration before interpreting changes as performance.

The Recorder receives `sim.fixed.elapsed`, `sim.phase.<phase>.observed`, optional
`sim.fixed.unattributed.observed`, and `sim.fixed.unobserved`. Per-invocation
coverage, envelopes, interval counts, unknown names, and the schedule/name census
are written beside a file capture as `<PATH>.phases.json`; with `PATH=-`, coverage
goes to stderr and the original capture remains on stdout. Keep both artifacts.
No baseline is changed or automatically adopted. The new metrics retain the
existing unknown-metric comparison policy; less observed work may mean poorer
coverage, not a faster simulation.

The existing `sim.tick` remains the app.update() bracket; zero/one/multiple
FixedUpdate invocations in a frame are separate phase samples. `sim.run` remains
the full run and therefore includes collection/reduction overhead. The existing
console-latency bridge remains the only source of admission-to-Broadcast latency.
Recording/replay command paths retain their previous behavior; this producer is
attached only to the existing non-recording perf capture path.

## Prepared validation

Six fabricated reducer tests cover gaps, overlap, nesting, duplicates, input
order, missing samples, unknown attribution and atomic malformed-batch refusal.
Five producer tests cover conservative name attribution, worker/nested pairing,
outer-facilitator handling, temporary duplicates, and unbalanced span refusal.
No test asserts a real duration threshold.

`tests/phase_profiling.rs` launches fresh own-executable processes for an ordinary
pinned mission, the same profiled mission, and default-executor coverage. It
compares actual authoritative seed/digest/tick and schedule membership/ambiguity
census for measured versus unmeasured pinned runs. The default arm proves actual
phase coverage with the ordinary multi-worker executor, not default-pool digest
stability (that remains slice 4). All use Combat Test, Destroyer, seed 20260894
and 240 frames at 1/60 seconds; both parity arms retain the existing console
latency pipeline. Error-only formatting must still yield every phase. A short
real `phoenix-headless` invocation also checks the existing CLI path writes both
capture and coverage files.

With the integrator's sequential Cargo slot, run the focused checks:

```sh
cargo check --features headless --tests
cargo test --features headless --lib perf::phase
cargo test --features headless --test phase_profiling -- --nocapture
```

The targeted phase unit tests and real producer/CLI integration passed on the
preserved declaration-stage source, including the locked Bevy ECS dependency
edge. Later typed RNG/mint changes were checked separately; final integrated
validation remains the integrator's responsibility. No timing baseline is
generated by these tests.
