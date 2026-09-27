# Mixed browser/native runtime matrix (#1530)

This runner combines two real browser ship simulations, two Windows native ship
simulations, one browser GM and one native GM. Each ship has Captain, Helm and
Engineering Station documents: six browser pages and six native Ultralight
panes. Native Bevy and the retained Ultralight fleet/GM surfaces run normally.
Browser webdriver execution defaults to WASM with Bevy rendering disabled.
`--render` requests the same SwiftShader software rendering mode as the browser
matrix runner. That mode remains unvalidated by the retained evidence below.

## Run

Build a source-matched browser bundle with
`node scripts/fleet-wasm-build-receipt.mjs build target/browser-wasm-receipt.json`
from a clean checkout. This runs release Trunk and the pure JS client build.
Add `--wasm-build-receipt target/browser-wasm-receipt.json` to the mixed runner
to verify the current browser bundle and clean source revision before launch.
The separate `--build-receipt` continues to check the native executable.
Build `phoenix-host` with `--features ultralight` using the native runtime fixes
and preserve its source/build receipt. The native bundle needs a matching
`client/index.html` and current `gui/native-fleet-peer.js`. Install the usual
`tests/smoke` Playwright dependencies and Chromium.

```powershell
node scripts/fleet-mixed-matrix.mjs --out target/mixed-clean --binary target/debug/phoenix-host.exe --bundle dist --dist dist --source . --build-receipt path/to/native-build-receipt.json
node scripts/fleet-mixed-matrix.mjs --out target/mixed-impaired --binary target/debug/phoenix-host.exe --bundle dist --dist dist --source . --build-receipt path/to/native-build-receipt.json --delay-ms 20 --loss-percent 10 --seed 1530
```

Add `--render` to either command to request browser rendering. The manifest
records the option and requested rendering mode. A configured mode alone does
not establish that the viewscreen drew successfully or passed visual acceptance.

Output directories must be new. `--routes direct`, `--routes ws-relay` or
`--routes automatic-fallback` selects one case. The default runs all three, stopping at the first failed case.
`--seconds` controls the browser command interval (default ten seconds);
`--deadline` bounds the case's polling stages (default 290 seconds). Individual
browser operations and teardown have separate bounds. Each native child also
has its own 300-second lifetime bound. Native binaries create real windows;
run one native/mixed matrix at a time on the rig.

The default native probe import is repo-relative. `--native-adapter` is an
explicit development override; `--service-script` similarly selects a service
adapter. Both selected files are hashed in the evidence.

## What a pass requires

- Four admitted ship roles, two GM roles, and six distinct adopted simulation
  slots. Native slots/roles come from the observed simulation roster and its
  accepted Rust generation acknowledgement.
- Six active browser Station documents and six active embedded Station
  documents, each with authoritative correlated command receipts.
- Browser GM pause/resume observed in both runtimes, plus at least two applied
  actions attributed to the native GM.
- At least two common outgoing digest ticks after the browser command and GM
  action interval, with matching hashes across all six peers. The browser
  observer returns the original `wasm_take_mesh_frames` result unchanged;
  it neither drains extra frames nor supplies simulated mesh data.
- Actual selected direct RTC pairs for every browser link in the direct case,
  and actual relay callbacks for native fleet links. The fleet owner has five
  browser RTC links and three native relay links. Forced relay and fallback
  require browser relay handshakes/traffic, and fallback requires real offers
  suppressed by the local service and exhaustion of the ordinary retry ladder.
- No uncaught browser/observer/native errors, queue overflow, or failed native
  bootstrap/cleanup. Failure screenshots, logs and raw events are retained.

Delay/loss acts at application-frame boundaries. Browser DataChannels delay
reliable frames and sample snapshot loss; the real local rendezvous applies
its equivalent profile to relay frames. Native fleet links carry reliable mesh
traffic; when no snapshot-class relay frames occur in the direct mixed case,
there is no relay snapshot-loss observation to claim. Direct browser snapshot
loss remains required. Other cases require observed relay snapshot drops.
Measured write delays must meet the requested minimum within one millisecond.
The configured 20 ms is not an upper latency bound: loaded browser timers may
add scheduling delay, and every observed minimum/maximum is retained. Counters
and receipts are sampled before teardown; ongoing native workload can leave a
final accepted action or a few transport frames pending at that boundary.
These counters measure local handoff, not IP packet loss or acknowledgement.

## Readiness and observation

The native GM probe runs with `deferGmReady: true`. The runner authorizes its
ordinary Ready vote only after all six peers are admitted and the browser
Station documents are seated and ready. Browser and native GM admission still
run concurrently. This matters during fallback: native relay admission can
finish while browser peers are still exhausting their RTC retries.

Native observer failures, including HTTP parser errors, non-success telemetry
responses and bounded-queue overflow, fail the mixed case. The corrected
observer emits each attributed GM action once and compact session state,
avoiding an ever-growing GET URL. The runner records the selected adapter hash.

## Evidence status

The first actual direct-capable mixed attempt on 2026-09-27 failed admission.
Native peers presented a JSON delivery stamp, while the browser host's strict
handshake parser requires the compact protocol/content/epoch field. All three
native peers received `client-stamp-missing`; the native configuration observer
confirmed a populated stamp, establishing a format mismatch rather than a
missing content manifest. The native task owns the format correction.

Artifacts remain under `target/1530-mixed-clean-1`: raw native events and
manifests, browser screenshots/state, service logs and the runner result. After
the terminal refusals, the verified ship-3 process was stopped to trigger
graceful runner cleanup; `operator-stop.json` records that early termination.
This first attempt staged GM admission. Subsequent runs restore concurrent GM
setup after the unrelated all-native identity-store collision was diagnosed.

The corrected clean direct-capable mixed cell passed on 2026-09-27 in 60.851
seconds (`target/1530-mixed-clean-2/direct`). Nine browser command waves were
applied by all six browser Station documents; the six native panes applied
33 or 34 commands each. The native GM had three attributed applied actions,
and the browser GM's pause/resume was observed in both runtimes. All six peers
reported `f80ee47c5ad6667d` at tick 300 and `7ca326a9059a54e0` at tick 600,
after the browser command/GM interval. All native children exited cleanly.

The binary was `9df9fde30250e1a511fd724a7924f65a590506ec29f8d10dac06cf4812820822`.
Its build receipt is retained in the run manifest. The browser bundle reuses
the previously recorded WASM artifact; it mirrors the content-module startup
fix and includes the reviewed stale-target transport fix in its private host
GUI copy. No newly rebuilt integrated WASM claim is made.

The first clean forced-relay case also passed (`target/1530-mixed-clean-3/ws-relay`).
Those early clean passes retain the required initial GM receipts but do not
claim complete later GM history: the original observer could exceed the HTTP
header limit with growing snapshots. Corrected-observer reruns are recorded
below.

The first automatic-fallback attempt in `target/1530-mixed-clean-3` failed.
The harness allowed its native GM to vote Ready before browser members finished
RTC fallback. The owner correctly launched the currently admitted two-peer
roster; later peers were then recovery-only. The deferred Ready coordination
above corrects that harness ordering. The original failure remains retained.

### Measured cells

All six selected route/profile combinations passed. Each requested a ten-second
browser command interval; startup, fallback retries and digest observation
account for the rest of each duration. Native commands continue until capture.

| Route | Profile | Total duration | Browser command waves | Native GM Applied |
| --- | --- | --- | --- | --- |
| direct | Clean | 93.666 s | 2 | 26 |
| ws-relay | Clean | 60.392 s | 7 | 3 |
| automatic-fallback | Clean | 223.721 s | 10 | 21 |
| direct | 20 ms / 10% snapshot sampling | 97.849 s | 6 | 24 |
| ws-relay | 20 ms / 10% snapshot sampling | 83.197 s | 8 | 26 |
| automatic-fallback | 20 ms / 10% snapshot sampling | 223.978 s | 11 | 23 |

The clean forced-relay row uses the initial observer and carries the explicit
GM history limit above. Both corrected clean cases and all three impaired cases
use the compact bounded observer. The corrected clean fallback exhausted 32
real RTC offers before relay and preserved automatic six-peer launch.

[Portable measured summary](1530-mixed-runtime-2026-09-27.json) retains the exact
source revisions, harness hashes, native build receipt, per-page routes, receipts,
post-action digest values, measured impairment counters, cleanup and original
failure references. [Complete bundle identities](1530-mixed-bundle-hashes.json)
cover every file in both served bundles. Raw result/manifest hashes identify the
local artifact directories without copying reconnect credentials into this ledger.

Each run records browser/native bundle hashes, executable hash, browser version,
source revision/diff, selected adapters, and native process hardware/SDK evidence.
An optional build receipt is retained verbatim with its hash; a binary mismatch
refuses the run. That comparison does not independently certify every source
claim in an external receipt. Browser WASM provenance must likewise be stated.

This bounded loopback runner does not establish rendered-browser, physical LAN,
internet/mobile, recovery, performance or endurance acceptance. It cannot close
#1530 by itself.
