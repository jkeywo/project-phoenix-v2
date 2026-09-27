# Current-source runtime matrix (#1530)

## Provenance and scope

Measured on 2026-09-27 from clean source
`fdaf31c211ed9396aaa9cc3d16e63912961d590b`. The release browser build ran
`trunk build --release` followed by `node scripts/build-client.mjs`; the native
build ran `cargo build --features ultralight --bin phoenix-host` and finished
successfully in 2m 24s. Native library mtime was refreshed without byte changes
before compilation; its dependency record named the main checkout and its
fingerprint enabled default/host/server/ultralight.

[Compact build receipts](1530-build-receipts-fdaf31c2.json) preserve original
receipt hashes and build metadata. WASM SHA-256 is
`2102dc304038244b25f0aae532b5c993f355869d567048fd318f77220c72d1e1`;
native executable SHA-256 is
`398e1c90bfb58c5e1faa6853ea47d0e8222e5393a6b7dc64bebf73936880021e`.
Every browser/mixed launch verified all served bundle hashes and the clean source
against its WASM receipt. Every native process recorded the same executable hash,
source revision and clean diff; native receipt verification occurred before launch.
The native runner's own `sourceBuildVerified: false` remains unchanged: the separate
build observation and receipt supply that attribution. Shared Cargo library
metadata was subsequently overwritten by a coordinated test before byte archival;
the original observed hashes, archived host dependency record and unchanged
executable were preserved. No later test executable is used as native provenance.

All cells use one Windows machine and loopback services, actual Phoenix transport,
real simulation peers and active Station documents. Native runs use Bevy/wgpu and
Ultralight. Browser route matrices disable Bevy rendering; the separate rendered
representative below has its own result. This is bounded functional evidence,
not mobile/internet, performance, endurance or recovery acceptance. The issue's
AFK matrix criteria do not explicitly require visual QA or a rendered route matrix.

## Browser: six cells passed

[Portable browser summary](1530-browser-source-fdaf31c2.json) includes source,
receipt/manifest/result hashes, browser version, outcomes and impairment counters.
Each cell admitted four ship and two separate GM simulation peers, launched twelve
Station documents, applied every command wave, observed both GM controls, and
captured one complete six-peer digest exchange without disagreement. The browser
gate requires one exchange; these results do not claim the mixed gate's two.

| Route | Clean duration | Impaired duration | Impaired snapshot drops |
| --- | ---: | ---: | ---: |
| Direct | 17.331 s | 25.327 s | 482 |
| Forced relay | 16.064 s | 31.363 s | 450 |
| Automatic fallback | 272.743 s | 281.247 s | 450 |

All seventeen browser links were observed on the intended route. Fallback suppressed
68 real RTC offers and exhausted the production retry ladder. The profile was 20 ms
application-frame delay, 10% snapshot sampling, seed 1530. Reliable frames retain
order and are never sampled for loss. Loaded timers exceeded the requested minimum;
the raw minimum/maximum observations are retained, not interpreted as an upper bound.

## Mixed: six cells passed

[Portable mixed summary](1530-mixed-source-fdaf31c2.json) retains exact results,
manifest hashes, native cleanup, Station/GM receipts and two post-workload matching
checkpoints per cell. Composition was two browser ships, two native ships, one
browser GM and one native GM, with six browser and six embedded Station documents.
All six adopted distinct slots and launched automatically after coordinated Ready.
Browser GM pause/resume was observed in both runtimes; the native GM applied
attributed hostility actions. Native links were relay in every route.

| Route | Profile | Duration | Matching checkpoints | Snapshot drops |
| --- | --- | ---: | --- | ---: |
| Direct | Clean | 62.648 s | 300, 600 | 0 |
| Forced relay | Clean | 66.777 s | 300, 600 | 0 |
| Automatic fallback | Clean | 214.164 s | 300, 600 | 0 |
| Direct | 20 ms / 10% | 79.691 s | 300, 600 | 454 browser / 0 relay |
| Forced relay | 20 ms / 10% | 81.082 s | 300, 600 | 433 relay |
| Automatic fallback | 20 ms / 10% | 227.844 s | 300, 600 | 436 relay |

The direct mixed cell measured native reliable relay delay but no relay snapshots;
its snapshot loss occurred on browser Station DataChannels. Relay/fallback snapshot
loss occurred on browser Station delivery through the service. These are application
frame observations, not packet loss. No cell reported queue overflow or cleanup
failure. All eighteen native child exits across these cells were observed.

## Native: two relay cells passed

[Portable native summary](1530-native-source-fdaf31c2.json) records four native
ship processes, two separate native GM processes, twelve embedded Station documents,
five observed fleet relay links, exact accepted slots, and two matching post-action
checkpoints per cell. Clean completed in 108.309 s with 53–54 Applied commands per
Station and 27 Applied actions per separate GM. Delayed completed in 125.338 s with
66 Applied commands per Station and 33/34 Applied GM actions. Both matched all six
peers at ticks 300 and 600, had no runtime/observer faults, and confirmed all child
and service exits.

Both used a separate GM's ordinary **Force Start**. The native owner retains an
extra local GM operator; this is four ship peers plus two stationless GM peers,
not exactly two GM operator rows or automatic all-ready launch.

Delayed transport recorded **18,961 reliable frames seen, delayed and written**,
20.0384–36.8297 ms observed write delay, no cancellation/overflow/pending frames,
and **zero snapshot frames and zero snapshot drops**. Its configured 10% snapshot
sampling therefore exercised no native loss. All-native direct WebRTC and RTC
retry fallback are **N/A** because this runtime advertises relay immediately.

## Rendered representative

A clean forced-relay mixed cell with `--render --deadline 600` **passed in
456.709 seconds**. [Portable rendered summary](1530-rendered-source-fdaf31c2.json)
records the requested SwiftShader mode, exact manifests/receipts and matching
six-peer digests at tick 300 (`495ee158bc3219b0`) and 600 (`1531eaf825adcf1f`).
The six browser Stations each applied all 20 command waves; six native Stations
each applied 378 commands. The native GM recorded 191 Applied outcomes from 192
requests at capture; its continuous driver could leave the final request pending.
Browser GM pause/resume was observed in both runtimes. All three native child
exits were confirmed, with unchanged executable hashes and no runtime faults.

This establishes one functional cell with browser rendering enabled, not a full
rendered route/profile matrix, visual correctness assessment or performance
threshold. Its longer bound changes no workload or digest requirement. The earlier
295-second rendered mixed recovery attempt on `91f382e6` timed out during its
healthy gate before any fault injection; this new #1530 cell does not relabel that
#1534 attempt as a recovery pass.

## Reproduction and retained evidence

Run the existing browser and mixed matrix commands described in their ledgers,
adding `--wasm-build-receipt target/browser-wasm-receipt-fdaf31c2.json`; mixed
also uses `--build-receipt target/native-build-receipt-fdaf31c2.json`. Commands
used ten-second browser command intervals, routes `direct,ws-relay,automatic-fallback`,
and new output folders. Browser timeout was 180 seconds per operation; mixed
case deadline was 295 seconds. Impaired invocations added
`--delay-ms 20 --loss-percent 10 --seed 1530`. Browser ports were 18430/18431;
mixed ports 18450/18451. Native matrices used port 18471, a 300-second bound,
and `--workload true`; their attribution JSON retains service arguments and receipt.
The rendered representative adds `--render --deadline 600 --routes ws-relay`
to the clean mixed invocation.

Full local artifacts are under `C:/Coding/project-phoenix-v2/target/`:
`1530-source-browser-{clean,impaired}-fdaf31c2`,
`1530-source-mixed-{clean,impaired}-fdaf31c2`, and
`1530-source-native-{clean,delayed}-fdaf31c2`, and
`1530-source-mixed-rendered-fdaf31c2`. Checked-in compact summaries
retain result/manifest hashes without copying private reconnect stores or large
raw logs. Earlier failed attempts and artifact-only runs remain separately
identified in the runtime ledgers and are not relabelled as current-source passes.

Recovery remains #1534; sustained performance/endurance remains #1543 and duration
ratification #1090. No recovery injection occurred in these #1530 cells.
