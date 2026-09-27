# T5 performance acceptance proposal — local measurements, ratification pending

This is the reusable plan for #1543. The user chose to decide the numerical
limits after seeing measurements. Nothing below is a ratified limit or a passing
performance acceptance result. Four ship peers, two GM peers, twelve active
Station clients and two active GM consoles remain the workload; the one-hour
mixed session remains required. The protocol checks in
`1530-six-peer-matrix.md` are separate evidence.

## Available machine provenance

Read from Windows CIM on 2026-09-27:

| Field | Observed value |
| --- | --- |
| OS | Windows 11 Home, 10.0.26200, build 26200 |
| CPU | Intel Core Ultra 9 275HX, 24 cores / 24 logical processors |
| Physical RAM | 68,112,736,256 bytes |
| GPU | NVIDIA GeForce RTX 5090 Laptop GPU, driver 32.0.16.1714 |
| Integrated GPU | Intel Graphics, driver 32.0.101.8628 |

This describes one development machine. It is not evidence for six independent
machines or for a minimum supported specification. Record the active renderer,
power mode, thermal state, display resolution, browser version and native SDK
version for every actual run; those were not measured by this inventory.

The updated inventory in `1543-measurements-2026-09-27/hardware.json` also records
High performance power mode, both display modes, Node v24.13.0 and rustc 1.95.0.
Ethernet was connected at 1 Gbps, but neither local measurement exercised it.
Thermal state and ordinary background application activity were not controlled.

## Measured loopback protocol probe

`node scripts/fleet-relay-probe.mjs` ran on this machine with Node v24.13.0
at 2026-09-27T00:13:00Z. It launched the shipped registry on loopback, admitted
four ship-role and two GM-role protocol peers over real WebSocket relay links,
froze their roster, and timed 100 synthetic frame round trips using one monotonic
clock. Raw samples, observed routes, base revision, dirty-state flag and SHA-256
hashes of the exercised sources, runner hash and tracked source patch are in
`1543-loopback-protocol-probe.json`.

| Round-trip statistic | Observed milliseconds |
| --- | --- |
| p50 | 0.7128 |
| p95 | 1.1100 |
| p99 | 1.3328 |
| Maximum | 1.3481 |

This is transport-only evidence: zero simulations, zero Station client
documents, no rendering and no authoritative command application. The source
was the dirty scale worktree based on `ea9acfb316c80a52ccbf586d07c3a5ecdfa853e4`.
It is not a comparable supported-workload baseline, a network acceptance cell,
or evidence that the proposed command/stall/recovery limits pass.

## Repeated local diagnostics (2026-09-27)

The raw evidence and reproducibility manifest are retained under
`1543-measurements-2026-09-27/`. Both instruments ran against runtime revision
`8aa756807d50a12819c2d44e6f40dd59082662d8` with the retained instrumentation patch.
The manifest pins the executable, runner, authored content and source patch.
These are repetitions of one local baseline, not a before/after comparison.

### Six simulations in one process

The ignored `six_peer_local_measurement` test ran three fresh fleets, each with
120 warm-up ticks and 600 measured ticks, paced at 60 Hz. Every fleet contained
four ship Apps, two GM Apps and twelve synthetic admitted Station sessions.
They shared one process and an in-memory mesh. No browser documents, native
surfaces or renderers were present. CommandDelay was fixed at two ticks; seed
was 1519006 and the world was `probe_fleet_six_peer.toml` with Alliance cruisers.

| Measurement (milliseconds) | Run | p50 | p95 | p99 | Maximum |
| --- | ---: | ---: | ---: | ---: | ---: |
| Sequential six-App step | 1 | 5.851 | 7.495 | 8.345 | 10.098 |
| Sequential six-App step | 2 | 5.939 | 7.715 | 8.754 | 10.416 |
| Sequential six-App step | 3 | 5.655 | 7.380 | 8.947 | 10.143 |
| Cycle including digest checks | 1 | 6.450 | 8.297 | 9.089 | 11.275 |
| Cycle including digest checks | 2 | 6.491 | 8.487 | 9.665 | 11.530 |
| Cycle including digest checks | 3 | 6.195 | 8.075 | 9.831 | 11.249 |
| Wave start to observed command application | 1 | 40.038 | 42.830 | 42.832 | 42.832 |
| Wave start to observed command application | 2 | 40.506 | 41.520 | 41.522 | 41.522 |
| Wave start to observed command application | 3 | 39.507 | 40.590 | 40.593 | 40.593 |

Each run lasted 10.000–10.001 measured seconds, applied 80 crew commands and
seven GM grants, compared 945 NPC decisions and completed 600 all-peer digest
checks. There were zero digest mismatches and zero cycles without tick progress.
Commands were sent in five waves of sixteen: the 80 observations are correlated,
and their p99 is the sample maximum, not a robust long-tail estimate. One clock
measured from the beginning of each wave to the end of the six-App cycle that
contained application. This conservative observation includes scheduling and
the other Apps; it is not a phone-to-host latency or an individual peer's cost.
Quantiles use nearest rank. The build/run log retains the existing audio warning
and repeated-App global logger warnings; both occur outside measured cycles.

### Loopback WebSocket relay

Three sequential repetitions used twenty warm-up rounds and 200 measured rounds
across five member-to-owner-to-member links: 1,000 timed echoes per repetition.
All six protocol peers used the real shipped registry and WebSocket relay, with
zero added delay/loss. There were zero simulations and zero console documents.

| Run | Measured seconds | p50 ms | p95 ms | p99 ms | Maximum ms |
| --- | ---: | ---: | ---: | ---: | ---: |
| 1 | 16.635 | 0.515 | 0.803 | 0.950 | 1.649 |
| 2 | 16.649 | 0.515 | 0.799 | 0.952 | 1.708 |
| 3 | 16.668 | 0.489 | 0.806 | 0.946 | 1.765 |

The probe deliberately spaces echoes through a polling loop; its duration is
not a saturation-throughput result. Other agents confirmed their build and PASM
processes had stopped before these final repetitions. An earlier three-run
series overlapped PASM validation and is retained separately as
`relay-background-validation-*.json`, excluded from this table. The historical
100-echo run above used an older revision and shorter sampling configuration;
its difference from these runs is not a measured improvement.

### Interpretation and reproduction

These samples establish a reproducible local diagnostic baseline. They do not
justify tightening the proposed acceptance limits: the separate devices,
rendered viewscreens, external network profiles and hour-long workload remain
unmeasured. The proposed durations and limits below therefore remain unchanged
and explicitly await the user's decision.

```powershell
$env:CARGO_TARGET_DIR = 'C:\Coding\project-phoenix-v2\target'
$env:PHOENIX_T5_ARTIFACT_DIR = '<fresh evidence directory>'
$env:PHOENIX_T5_MEASURE_SECONDS = '10'
$env:PHOENIX_T5_MEASURE_REPETITIONS = '3'
# Refresh src/lib.rs mtime when switching a shared target between worktrees.
cargo test --features headless --test lockstep_six_peer six_peer_local_measurement -- --ignored --exact --nocapture
$env:PHOENIX_RELAY_PROBE_ROUNDS = '200'
$env:PHOENIX_RELAY_PROBE_WARMUP_ROUNDS = '20'
node scripts/fleet-relay-probe.mjs # Repeat three times; retain each JSON output.
```

This diagnostic ten-second duration is not a substitute for the proposed
ten-minute acceptance runs or the retained one-hour mixed session.

## Proposed shorter runs and repeatable profiles

Run each applicable browser/native/mixed cell from the matrix for 2 minutes of
warm-up, then 10 measured minutes, three repetitions. Use the same composed
content, seed and command schedule for baseline and candidate. The mixed
one-hour endurance run follows the shorter runs; it is not replaced by them.
Record the authored CommandDelay and hold it fixed throughout each run. These
proposals do not authorize adaptive delay or tuning a failing run into a pass.

| Profile | Controlled added delay per direction | Random packet loss |
| --- | --- | --- |
| Local baseline | 0 ms | 0% |
| Delayed | 50 ms | 0% |
| Impaired | 100 ms, uniform jitter ±20 ms | 1% |

These are proposed test inputs, not descriptions of the user's LAN or mobile
networks. Record the shaping tool/version, seed, interface and observed counters.
WebSocket relay has TCP retransmission and head-of-line delay: packet loss must
be applied below the WebSocket, not simulated by discarding reliable game frames.
Keep ordinary internet, hotspot and separate mobile-network observations in
separate ledger rows. Loopback does not establish any of those rows.

## Proposed numerical limits for discussion

| Measurement | Local baseline | Delayed | Impaired |
| --- | --- | --- | --- |
| Command input to authoritative application, p95 | ≤250 ms | ≤450 ms | ≤750 ms |
| Command input to authoritative application, p99 | ≤500 ms | ≤900 ms | ≤1,500 ms |
| Unplanned stall time / measured duration | ≤0.5% | ≤1% | ≤3% |
| Longest unplanned stall outside fault injection | ≤1 s | ≤2 s | ≤5 s |
| One departed peer to survivor progress, p95 | ≤5 s | ≤8 s | ≤15 s |
| Deliberate divergence to verified recovery, p95 | ≤10 s | ≤15 s | ≤30 s |

All rows additionally require zero unexplained digest mismatches, zero duplicated
canonical actions and exactly one winner for each replacement-slot race. Numeric
values are starting proposals; measured distributions and identified causes must
be shown to the user before any acceptance decision. Do not hard-code these
values into product behavior or unrelated CI timing gates.

Measure input-to-application with a correlation ID and the originating clock;
do not subtract unsynchronized clocks on different devices. Track the application
tick and a correlated return observation, documenting whether the reported value
includes the return trip. Exclude deliberate GM pause from unplanned stalls,
but record both. For recovery, retain the loss/divergence detection, restore
commit and resumed-progress timestamps and digest/history evidence.

## Required evidence for a ratification request

For both comparable baseline and candidate, retain commit SHA plus dirty patch,
composed content ID/epoch, bundle and binary hashes, seed, runtime versions,
per-peer hardware and role, actual link routes, twelve active client counts and
their command totals, both GM consoles' actions, shaping provenance, raw samples,
p50/p95/p99/max summaries and first-divergence artifacts. Report changes against
baseline beside absolute values. A baseline from a different build profile,
content set or network profile is not a comparable baseline.

Current measurement ledger: one bounded browser/loopback run with four ship
simulations, two GM simulations, twelve active Station documents and two GM
console documents is recorded below. Its receipt bound, host tick cadence and
ship-loss recovery observations are real but do not establish the proposed
application-latency or unplanned-stall limits. Rendered, native, mixed, external
network, repeatability and one-hour evidence remain outstanding. User
ratification remains pending.

## Trace capture and reducer

`scripts/fleet-performance-observer.mjs` is an opt-in browser page observer for
the existing real matrix clients. After `observeBrowser` has installed its
`__matrixEvidence` array, install `installBrowserPerformanceObserver` with a
unique document clock ID and peer label. Call `window.__fleetPerformance.input`
with the command's existing correlation immediately before dispatch. The
observer timestamps that input and the real `ActionFeedback` `Applied` receipt
with that document's `performance.now()` clock. The receipt duration includes
the return delivery and is therefore a conservative **upper bound** on input to
authoritative application. A refusal is not an applied sample. Read the events
before closing the page. This hook does not alter or inject transport frames.

`scripts/fleet-performance-evidence.mjs` reduces event traces from browser,
native, or mixed runners. A producer must supply `format`,
`expectedTickMs`, `provenance` (revision, composed content identity, runtime,
artifact hashes and network profile) and events carrying `peer`, a unique
monotonic `clock`, `ms`, `kind` and a correlation where applicable. For example,
the native probe can emit a real `authoritative_applied` event with its applied
tick, and a host tick observer can emit every successive `tick`. Fault runs can
mark `fault`, `loss_detected`, `restore_commit`, `progress_resumed` and an
agreed `digest_verified` at their actual observation boundaries. Record those
events on one observer clock for each interval being reported. Keep the
per-peer hardware, browser/native versions, shaping counters, raw source and
bundle hashes in the same retained provenance as the matrix artifact. The
reducer checks the revision shape and SHA-256 hash values; it cannot establish
that supplied provenance matches a binary without the runner's separate
artifact verification.

The reducer subtracts timestamps only for the same clock and peer. It reports
input-to-application as unavailable when the input document and authoritative
host use different clocks, while retaining the independently measured receipt
bound. Tick gaps require a contiguous tick sequence; planned GM pause overlap
is removed from unplanned excess. Stall duration, excess and fraction are
reported per clock and peer; the fleet-facing fraction is the **worst peer's**
fraction, never a sum diluted by healthy peers. Recovery summaries retain detection,
progress and digest verification as separate intervals and list restore commits
without subtracting another peer's timestamp. Missing events remain visible
as incomplete pairs. Run it with
`node scripts/fleet-performance-evidence.mjs trace.json summary.json` after
capturing a real workload trace. The browser lane below used these tools; native
and mixed traces remain outstanding.

`scripts/fleet-performance-capture.mjs` connects those instruments to the
actual browser matrix and its existing ship-loss recovery gate. It requires a
clean, source-matched browser WASM build receipt and a fresh output directory
outside the source checkout, so evidence creation does not invalidate that
receipt.
The default is one direct-route case: the matrix's one-second admission and
active-command gate, ten additional measured seconds of twelve correlated
Station command streams, then one ship departure with up to 60 seconds for the
five survivors to prove loss handling and matching digest progress. Use
`--measure-seconds` (1–120) and `--fault-seconds` (1–180) to bound a different
capture; the ordinary matrix `--routes`, `--render` and impairment options
remain available. For a built checkout:

```powershell
node scripts/fleet-performance-capture.mjs --out <fresh-directory> `
  --dist dist --wasm-build-receipt <source-matched-receipt.json> --render
```

Each case retains the matrix manifest/result, `performance-trace.json` and
`performance-summary.json`. The trace includes the exact source revision and
patch, content identity and world hash, complete bundle hash inventory, build
receipt, runtime/hardware summary and observed route/impairment counters. The
host tick timestamps are taken as production mesh egress is drained in the
host document, and the reducer rejects skipped or duplicate tick observations
instead of calling an incomplete series a stall result. The fault timestamp is
recorded on each survivor before the browser victim is closed, so its recovery
interval includes the close operation. This is a bounded browser/loopback
measurement lane only. Native, mixed, external-network, one-hour and human
ratification evidence remain separate and unmeasured until actually run.

## Bounded browser observation (2026-09-27)

Application bundle revision `b16122a02414ed67cd5d396d0bed889ae275cc82`
had a clean source tree and a verified source-and-bundle receipt at
`target/browser-wasm-receipt-b16122a0.json` (SHA-256
`cc247306f53280ca0f0b168d83c5a8e2b73465b0b7b1fbb1515df4e193cdf377`).
The capture wrapper fix was a separate, reviewed `0a334c35` overlay, SHA-256
`09dfcc5482654de15f43647aaea7ca0313fa20a63543a32bb2863da62931f2d9`;
the trace records this as its runner hash, rather than claiming the wrapper was
part of the clean application revision. The final ignored local evidence is in
`target/1543-performance-b16122a0-run3/`, copied byte-for-byte from the fresh
temporary run output. Its manifest records Chromium 147.0.7727.15, Node
v24.13.0, Windows 10.0.26200, Intel Core Ultra 9 275HX, 68,112,736,256 bytes
RAM, content `phoenix-base@1:probe_fleet_six_peer`, world hash, seed 1530 and
the full bundle hash inventory. The observed route was direct WebRTC at the
actual `RTCDataChannel.send` boundary on one machine; there was zero added
application-frame delay/loss and no IP-level shaping.

The rendered attempt used
`node scripts/fleet-performance-capture.mjs --out C:\Users\jkeyw\AppData\Local\Temp\phoenix-1543-b16122a0-run2 --dist dist --wasm-build-receipt C:\Coding\project-phoenix-v2\target\browser-wasm-receipt-b16122a0.json --render`.
Its temporary output is retained at that path. It failed the matrix's
90-second digest-sample wait after six hosts and twelve clients launched. All
clients had two Applied receipts, but the hosts recorded no digest samples.
The original wrapper then masked the matrix failure with an undefined manifest
status; `0a334c35` corrects that return path. Run 2 remains a diagnostic failure,
with no latency or recovery sample. It does not establish a rendered performance
result or an underlying product defect.

The successful non-render run used this command from the clean application
checkout; the overlay changed only its relative imports to run from `target`:

```powershell
node target/fleet-performance-capture-overlay-0a334c35.mjs `
  --out C:\Users\jkeyw\AppData\Local\Temp\phoenix-1543-b16122a0-run3 `
  --dist dist --wasm-build-receipt target/browser-wasm-receipt-b16122a0.json
```

The matrix passed with four ship and two GM WASM simulations, twelve Station
documents and two GM console documents (18 browser documents total), and its
ship-loss digest recovery gate passed. Bevy viewscreen rendering was disabled.
Over ten measured seconds, twenty command waves produced 240 correlated Applied
receipts and zero incomplete pairs. Same-document input-to-**Applied receipt**
duration, which includes return delivery and is an upper bound on application
time, was p50 **101.8 ms**, p95 **136.3 ms**, p99 **141.7 ms**, maximum
**141.9 ms** (nearest rank). The authoritative application timestamp is on a
different clock, so exact input-to-application latency is unavailable.

Pre-fault host tick observations span 10.436–10.471 seconds per host. Against a
60 Hz expected tick interval, the worst host's excess-time fraction was **60.43%**
and its longest single excess was **92.3 ms**. These are cadence deficits, not
measured unplanned-stall time: steady slow ticks contribute excess. The
separately retained `direct/pre-fault-stalls.json` excludes each host's fault
window; the full-run stall summary includes the deliberate fault and must not
be used for acceptance.

Ship 2 loss was applied at tick 560. Five survivors proved Backfill and matching
digests at ticks 600 and 900. Across their same-page clocks, fault marker to
loss detection was p50 **15.626 s**, p95/maximum **15.651 s**; to resumed
progress p50 **16.601 s**, p95/maximum **16.605 s**; to verified agreement
p50 **28.153 s**, p95/maximum **28.153 s**. These intervals start before victim
close and include browser close plus observation delay. They are not a
divergence-recovery measurement or a p95 estimate across independent runs.

SHA-256 of the retained `manifest.json` is
`40fd5aa754d7805c0442cc7ac5cbcafe80449555589e8416e4f1a1ac99ffd529`;
`direct/result.json` is
`d4778f3488043f704932f347818bc0d1b10e48e2135cc8d4d2b78d8e6817d74e`;
`direct/performance-trace.json` is
`9f31377743007ada20205f5f2487dcc0bfa8ee1ba70620b1a19754a6ed3694e3`;
`direct/performance-summary.json` is
`5c1ad1fe68894a0e26f7f913ce46d7318c712c756e8289892fc18c513ca5b9d4`;
and `direct/pre-fault-stalls.json` is
`5d8db3ebfdc03ca6b4db0389c26977ac044ec9a5982b5d719a32e447a003f44b`.
This is one short browser/loopback sample on one machine. Rendered, native,
mixed, mobile, separate-device, repeatability and one-hour evidence remain
pending, as does the user's numerical ratification.
