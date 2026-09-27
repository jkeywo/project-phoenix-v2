# Six-peer runtime and transport matrix

The supported workload is four ship simulation peers, two separate GM simulation
peers, three active Station clients on each ship, and ordinary Backfill elsewhere.
The twelve clients must send commands during measurement; readiness counts alone
do not establish this workload. Both GM consoles must perform recorded actions.

## Repeatable protocol checks

Run `npx vitest run tests/client/fleet-matrix.test.js` from the checkout.
`tests/client/fleet-matrix-harness.js` exports the bounded six-peer cases for
subsequent fault injection. They exercise the shipped registry, compatibility
handshake, fleet role/slot admission, Station Rating publication, readiness,
automatic launch grant, and direct/relay route selection. The fallback case
withholds all four RTC offers and observes the real retry ladder reach relay.

These are protocol tests with socket and RTC stand-ins and virtual timers.
They are neither real-browser/native workload measurements nor observations of
mobile networks. Their Station clients are published readiness/Rating inputs;
they do not run twelve client documents or the simulations.

## Real browser runner

Build the host with Trunk and then run `node scripts/build-client.mjs` so
`dist/client/index.html` exists. Install `tests/smoke` Playwright dependencies
and Chromium. The runner loads Playwright directly, with no smoke transport
fixture. Use a fresh output directory for each invocation:

```powershell
node scripts/fleet-browser-matrix.mjs --out target/browser-matrix-clean --dist dist --seconds 10 --timeout 120
node scripts/fleet-browser-matrix.mjs --out target/browser-matrix-impaired --dist dist --seconds 10 --timeout 120 --delay-ms 20 --loss-percent 10 --seed 1530
```

Each invocation runs direct, forced WebSocket relay and automatic fallback.
`--routes direct,ws-relay` selects a subset. Fallback suppresses real outgoing
RTC offers at the local rendezvous, then observes the ordinary retry ladder.
The runner launches four ship WASM documents, two GM WASM documents and twelve
full Station documents, seats Captain/Helm/Engineering, readies everyone and
waits for automatic launch. Every Station sends receipt-checked commands;
Helm also sends continuous thrust. Both GMs operate pause/resume. Final gates
require every command wave's receipt, all six live roles/slots, a complete
observed digest exchange without disagreement, and actual route traffic on
all seventeen links. Browser exceptions fail the run.

The delay/loss profile applies to application frames, not IP packets: direct
uses the actual RTCDataChannel.send boundary; relay uses the local service's
outgoing frame boundary. Reliable frames retain ordering; only snapshot frames
are sampled for loss. Seeded sampling is repeatable for a given frame sequence;
scheduling and resulting frame counts can vary. Observed write delays and drops
must support the requested profile. Counters measure local handoff, not remote
acknowledgement. Queues are bounded and overflow fails acceptance.

Artifacts include manifest/source and bundle hashes, machine/browser facts,
per-page route/receipt/digest state, GM action observations, bounded logs and
failure screenshots. Shutdown is bounded. Default webdriver execution runs the
real simulation with Bevy rendering disabled. `--render` requests software
rendering but remains unvalidated; it is not a passed rendered cell.

## Native ship membership

A built native host can join a selected ship to an existing fleet:

```powershell
phoenix-host --world assets/worlds/probe_fleet_six_peer.toml --client-dir dist --rendezvous http://127.0.0.1:8788 --origin http://localhost:8080 --fleet-code <fleet-code>
```

Use a different `--addr` for each native process. Start the local rendezvous with
`node scripts/rendezvous-dev-server.mjs --port 8788`. This serves the production
registry on loopback; it does not emulate a deployed service or a mobile network.
Native membership uses the same retained lobby bridge and fleet member protocol
as GM membership, with the `ship` role and selected hull. The native link is
WebSocket relay because this runtime has no WebRTC. Its existing GM landing
join retains the GM role. `--fleet-code` requires a selected world, built bundle,
service and Origin; it refuses `--solo`. Use an Ultralight-enabled native build
for the retained control surface.

## Runtime acceptance ledger

For each run retain revision, dirty diff (if any), complete content identity,
binary/bundle hashes, OS/CPU/RAM/GPU, native SDK and browser versions, all peer
roles, twelve Station clients, GM actions, seed, observed route per link,
impairment configuration, timestamps, and first-divergence/recovery artifacts.
An intended route is not an observed route. Native links should report relay;
a mixed session can contain direct browser links and native relay links.

| Runtime | Direct-capable links | Forced relay | Automatic fallback | Status |
| --- | --- | --- | --- | --- |
| Six browser peers, renderer disabled | All 17 links observed | All 17 links observed | Offers suppressed; real retry ladder | Short local subset passed; see evidence below |
| Six Windows native peers | Not available in native fleet transport | Five fleet links observed | Native advertises relay immediately | Clean workload and 20 ms reliable-frame delay passed; no snapshot traffic to drop |
| Mixed browser/native | Eight browser links direct, three native fleet links relay | Eleven network links relay | 32 real offers suppressed before relay | All three clean and impaired short cells passed |

Run each cell with LAN conditions, then reproducible delay/loss. Preserve the
profile and observed impairment counters beside results; use separate evidence
for ordinary internet, mobile hotspot and separate mobile networks. Do not call
a controlled impairment a real mobile observation. The one-hour mixed run and
shorter-run durations/limits remain governed by #1543 and #1090.

### Browser evidence, 2026-09-27

[Retained browser summary](1530-browser-runtime-2026-09-27.json) records the
actual cases, artifact hashes, runtime bounds, command counts and impairment
observations. The six route/profile combinations passed on Windows with
Chromium 147.0.7727.15. Each command interval was ten wall-clock seconds, plus
startup/fallback and waiting for the first complete digest exchange. These are
short local checks, not performance or endurance results.

The bundle reused the existing WASM artifact (SHA-256 recorded), with a fresh
client build and an exact scratch-bundle mirror of the server module-readiness
fix. No matching Rust build receipt was available, so this is artifact-level
evidence, not proof of a newly rebuilt integrated revision. Host/client build
stamp admission succeeded. Hashes of every retained bundle file, including
authored data and presentation assets, are retained separately; GPU/SDK
evidence is not applicable to this unrendered
browser subset. Main dist was not edited.

The first real-browser attempt exposed a production boot race: Trunk could
start content loading before the independent content-fetch module evaluated.
The fix explicitly imports and awaits that module. The regression fails with
`registerContentFetch is not a function` against the original bundle and passes
with the fix. Original failure artifacts remain under
`target/1530-browser-real-4` and `target/1530-browser-real-5`; later harness
errors and their artifacts are also retained, rather than overwritten.

### Native and mixed evidence

The [native runtime ledger](1530-native-runtime.md) records six real Windows
processes, twelve embedded Station documents, two GM process workloads and
common post-action digest ticks. Its clean relay workload passed in 107.192
seconds with six distinct slots, applied Station commands, attributed GM
outcomes and clean child shutdown. Native has no RTC ladder; its five fleet
links are relay links.

The corrected-observer native impaired run passed in 138.310 seconds: all
twelve Station documents applied 72–73 commands, the two native GMs retained
37/36 attributed Applied actions, and all six agreed at ticks 300 and 600.
The service delayed 19,037 reliable frames; observed writes took 20.045–57.953
ms. No snapshot-class traffic occurred, so the configured 10% snapshot-loss
profile exercised no native loss. See the [native impaired summary](1530-native-impaired-result.json).

The [mixed runtime ledger](1530-mixed-runtime.md) records two browser ships,
two native ships, one browser GM and one native GM, with six browser and six
embedded native Station documents. All three routes passed with clean transport
and with 20 ms application-frame delay plus 10% snapshot-loss sampling. Each
gate requires exact adopted roles/slots, active Station receipts,
both GM workloads and two matching post-action digest ticks across all six
runtimes. Run against an Ultralight binary and built host/client bundles:

```powershell
node scripts/fleet-mixed-matrix.mjs --out target/mixed-clean --binary target/debug/phoenix-host.exe --bundle dist --dist dist --source .
node scripts/fleet-mixed-matrix.mjs --out target/mixed-impaired --binary target/debug/phoenix-host.exe --bundle dist --dist dist --source . --delay-ms 20 --loss-percent 10 --seed 1530
```

The first mixed attempt found and retained a native/browser stamp-format
mismatch. The first mixed fallback attempt exposed premature Ready voting in
the harness; its failed artifacts remain, and it is not a fallback pass.
Early clean native/mixed runs retain the required GM receipts but do not claim
a complete later GM activity history: large observer snapshots could exceed
the HTTP header limit. Corrected compact telemetry passed representative clean
and all impaired mixed cases. The [mixed measured summary](1530-mixed-runtime-2026-09-27.json)
retains route, receipt, digest, build and counter evidence plus both original
failures. Actual direct snapshot drops and relay snapshot drops were observed
in the respective mixed impaired cases; native-only delay has the narrower
limit stated above.

Rendered-browser, physical/mobile/internet, recovery and endurance acceptance
remain separate and outstanding. Browser rows also retain their artifact-level
WASM provenance limit until a source-matched build is measured.
This evidence does not close #1530.

## Operator feedback controls

`tests/client/fleet-health.test.js` drives retry/fallback and disconnect
diagnostics from the same protocol fixture into the shared browser/native
renderer, preserves keyboard focus, and admits a fifth ship and third GM with
the support warning. Its slow/restoring-peer projection case tests rendering;
the actual delayed-runtime projection and native surface observations remain
unverified operator-surface acceptance. The native retained surface is available
through F9 during play; browser fleet warnings remain outside the hidden lobby.

Actual-runtime #1535 feedback acceptance remains unverified.
