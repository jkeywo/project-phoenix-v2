# Six-peer recovery acceptance (#1534)

Status: implementation and automated protocol checks are partial evidence. Real
runtime results below determine which acceptance cases have actually passed.
Issue #1534 remains open until the required runtime/transport cases are covered.

## Scope and provenance

This is a local-machine acceptance record as of 2026-09-27, not a completed
matrix. Browser runs use real Chromium, WASM, rendezvous sockets and production
fleet transport, with four ship simulations, two GM simulations and twelve
active Station documents. These browser cases used `render=false`: they prove
simulation and transport behavior, not rendered viewscreen or operator feedback.
Each run used a clean integrated checkout, `trunk build --release` followed by
`node scripts/build-client.mjs`, and a receipt verifier that checked the exact
source revision and bundle file hashes before starting. These revisions differ;
a pass on an earlier revision is not a claim that later integrated changes were
rerun. The first failures are retained alongside their successful reruns.

| Source revision | Browser receipt | Receipt SHA-256 |
| --- | --- | --- |
| `b4e17e72d3a8e23cc46efff5374010e6cc98b5bf` | `target/browser-wasm-receipt-b4e17e72.json` | `cfa1c660ea620ef649a8e9f06416fab1efcb75f077a5a00748c404392ce604eb` |
| `91f382e6f516f34adf6795423ae1222132e1459f` | `target/browser-wasm-receipt-91f382e6.json` | `66684fb6743f2bf7547b821a09a840912b396700c5e6cc0e332e98a0ff55045f` |
| `d2b6a8ed855ea6a5c5b411a0ed2e4b514eb3e8c1` | `target/browser-wasm-receipt-d2b6a8ed.json` | `f28501bfd700428d23ed4cab362eb00ee47c4f291e605ca03572d6cbe69fc3db` |

All three receipts record the same application WASM SHA-256,
`33af142d48bb3be7764552f5c1483e86b19f45c8681e7c3c340b06f9d64fcd15`;
the later changes before these runs were JavaScript/harness changes. Manifests
also record the runner hashes, browser version, options and complete bundle
hashes. Raw paths below are local `target/` artifacts, not checked-in fixtures;
keep the whole named directory and its build receipt when archiving evidence.
No credentials or private continuation capabilities are needed in this guide.

## Reproduce

Use a clean integrated checkout and a matching build receipt from
`scripts/fleet-wasm-build-receipt.mjs`. The recovery runner first executes the
ordinary #1530 workload: four ship simulations, two GM simulations and twelve
active Station documents. It applies one fault only after that healthy gate.

```powershell
node scripts/fleet-browser-recovery.mjs --failure ship --out target/recovery-ship --dist dist --wasm-build-receipt target/browser-wasm-receipt.json --seconds 10 --timeout 120
```

Choose `ship`, `leader`, `gm`, `divergence` or `replacement`. Each invocation
runs direct WebRTC, forced WebSocket relay and automatic fallback; `--routes`
selects a subset. Every output directory must be fresh. Browser exceptions,
observer overflow, missing receipts and missing post-fault evidence fail the run.
The browser fault bound is 60 seconds for a host loss and 90 seconds for divergence or
replacement, after the healthy gate. No artificial delay or packet loss was
configured in the measured recovery cases. Native/mixed uses the same recovery
projection and exact hexadecimal digest checkpoints. Its initial measured
command was:

```powershell
node scripts/fleet-mixed-recovery.mjs --failure ship --out target/1534-mixed-native-ship-relay-91f382e6 --dist dist --bundle dist --binary target/debug/phoenix-host.exe --build-receipt target/native-build-receipt-91f382e6.json --wasm-build-receipt target/browser-wasm-receipt-91f382e6.json --seconds 3 --timeout 120 --deadline 295 --routes ws-relay --render
```

Use fresh output paths for new runs and pass the receipt matching the current
integrated build. The historical command above is a failed precondition record.

## Required evidence

- Loss: five surviving original slots advance; every wait-set excludes the lost
  slot; all five report the same applied loss tick; the lost ship retains its
  roster entry with empty crew (ordinary Backfill); at least two fresh common
  digest checkpoints match. GM loss has no ship Backfill transition.
- Owner loss additionally requires each survivor's local Rust continuation to
  commit at that same boundary. A transport reconnect alone cannot pass.
- Divergence: one authenticated incoming Tick command is changed at the private
  JavaScript-to-WASM test boundary. The affected peer must report a successful
  restore and agree at two checkpoints after restore; all four ship UUIDs must
  remain unchanged in fresh admitted controls.
  A separately correlated GM direct-damage action supplies a non-idempotent
  witness: both GMs must retain exactly one actual damage-applier event through
  restore, plus its canonical journal result. An earlier retained activity tick
  rules out bounded history hiding a duplicate. Missing or delayed projection is
  awaited boundedly; ambiguity fails the gate. Outgoing command-order uniqueness
  and the focused Rust duplicate-admission tests remain additional checks.
- Replacement: two new hosts concurrently claim one disconnected fixed slot.
  One is admitted and restored, the other receives `slot-taken`. A subsequent
  challenge cannot displace the connected winner. All six authoritative slots,
  a common replacement boundary/claim sequence, one owner claim and two exact
  post-challenge checkpoints are required.

## Bounded owner continuation

The existing star transport supplies no transferable signature for an old
owner's frames. A live origin supplies its own authenticated tail. The elected
successor can replay only old-owner rows it retained from the old authenticated
link. An owner suffix held solely by another survivor remains paused with
`unverifiable-owner-suffix`; it is not imported as if its carrier were its author.
This is an explicit acceptance limitation, including for an otherwise isolated
owner failure, rather than a completed recovery guarantee.

Retained JavaScript tails and resumed egress are bounded to 4096 frames/8 MiB.
Core replay ingress is separately bounded. Missing history, unsupported structural
recovery, changed delegated membership or exceeded bounds retain refusal.
Direct media failure before the service notice preserves delegated membership
for at most the configured reclaim duration (120 seconds by default). A live
service owner never authorizes promotion; observation beyond that bound is not
a heartbeat guarantee. Only one failure at a time is in scope.

## Runtime results

Browser ship loss passed on clean integrated source `b4e17e72` with
`target/browser-wasm-receipt-b4e17e72.json`. Full invocation exited 0.
Raw results are under `target/1534-source-browser-ship-b4e17e72/`.

| Route | Lost slot | Applied Backfill tick, all five | Fresh matching checkpoints | Result |
| --- | --- | --- | --- | --- |
| Direct WebRTC | 3 | 384 | 600, 900 | Passed |
| Forced WebSocket relay | 3 | 399 | 600, 900 | Passed |
| Automatic fallback | 4 | 438 | 600, 900 | Passed |

Each route directory retains `result.json` and service diagnostics. The manifest
records the clean source revision, verified receipt and actual bundle hashes.
These runs establish isolated non-owner ship loss only; the separate owner-loss
relay result is below. Other cases remain listed explicitly under Outstanding.
Earlier source-unmatched exploratory runs diagnosed terminal RTC failure; they
do not establish completion of the current revision's matrix.

### First owner-loss probe (failure retained)

`target/1534-source-browser-leader-relay-b4e17e72/ws-relay/result.json`
failed its 60-second bound. The successor acquired the same code, but all five
Rust continuations remained held at epoch 1/generation 1, ticks 405–406; no loss
transition or post-fault checkpoint was accepted. One GM received owner-changed
while its local Begin acknowledgement was still pending, ignored that event,
and retried an obsolete epoch. Its observed counts were eleven service ready
frames, one initial join, ten fleet-state replies and ten errors. Other followers
rejoined successfully. The delayed-Begin regression reproduces this ordering;
the source-matched rerun after that fix is recorded below.

### Owner-loss relay rerun

`target/1534-source-browser-leader-relay-91f382e6/ws-relay/result.json`
passed on clean source `91f382e6` with its matching browser build receipt.
All five Rust continuations committed epoch 1/generation 3 at loss tick 379.
The same tick applied Backfill everywhere; exact fresh checkpoints 600 and 900
matched, and surviving ticks reached 915–916. This establishes the forced-relay
case after the delayed-Begin fix; direct and automatic-fallback owner loss still
require their own measured evidence.

### First divergence relay probe (failure retained)

`target/1534-source-browser-divergence-relay-d2b6a8ed/ws-relay/result.json`
failed its 90-second bound on clean source `d2b6a8ed` with the matching receipt.
The one-time direct-damage action produced exactly one 5 HP applier event on
both GMs at tick 374, attributed to journal sequence 3; the earlier retained
activity tick was 5. The harness changed one incoming SetThrust on GM slot 5 at
tick 377 from approximately 0.8 to -0.7. At checkpoint 600, the other five peers
folded `52d343f833675efe`, while GM slot 5 folded `12daa0d6dfdfacd1`.

No recovery record appeared; all peers continued past tick 3876 and the split
persisted. The adapter's electorate used ship rows only, omitting both GM-only
simulations. The focused fix uses all technical participants except departed
slots. This failure remains evidence until a fresh source-matched run proves
actual snapshot restore and two later matching checkpoints. The damage events
aged out of the bounded activity feed during the unresolved run, so the effect
retention gate also correctly refused acceptance.

### Mixed native ship-loss attempt (fault not reached)

`target/1534-mixed-native-ship-relay-91f382e6/ws-relay/result.json`
failed the healthy precondition: `post-workload six-peer digest agreement`.
Its manifest pins source `91f382e6f516f34adf6795423ae1222132e1459f`, the browser
receipt above and `target/native-build-receipt-91f382e6.json`. Composition was
two browser ships, two native ships, one browser GM and one native GM. Six peers
were admitted, the mission launched, and active Station/GM workload ran. Only
one common checkpoint (tick 300, `f84aec42538b44cd` on all six) was retained;
the required second healthy checkpoint was not reached before the deadline.
There is no `recovery` result because the hook never injected ship loss. This is
neither a recovery pass nor evidence of a product failure after native ship loss.

### Divergence relay rerun (restore occurs, later disagreement remains)

`target/1534-source-browser-divergence-relay-fdaf31c2/ws-relay/result.json`
failed after the electorate fix on source
`fdaf31c211ed9396aaa9cc3d16e63912961d590b`. Receipt
`target/browser-wasm-receipt-fdaf31c2.json` has SHA-256
`0e9db95783ac258f626f22462b76c1db8ea18e9deb1bfed3fd639d4b0285e136`;
the application WASM is
`2102dc304038244b25f0aae532b5c993f355869d567048fd318f77220c72d1e1`.
The result artifact SHA-256 is
`48cefaf4891964eddfd076854808ffbc7f5e95f8792e7d4dc7bbd7603b20e407`.
This run used ports 18440/18441, `--seconds 10 --timeout 180 --routes ws-relay`,
and the unchanged 90-second fault observation bound.

The damage baseline was again accepted: one 5 HP event on both GMs at tick 405,
journal sequence 3, with prior retained activity tick 4. The single incoming
SetThrust mutation on GM slot 6 was at tick 409. At checkpoint 600, four peers
agreed on `8c0e5fcb3137e746`, ship-1 differed (`38ad8d3dba7e61d6`) despite
receiving no injected fault, and the injected GM differed (`190b6994d0a274a8`).
All peers derived leader slot 2, recovering slots 1 and 6, and boundary 1200.
Both recovering peers recorded `recovered`; the others recorded `led` or
`witnessed`. Subsequent checkpoint 1500 had no strict majority, and all six
recorded `no-safe-leader`. No pair of post-restore agreed checkpoints passed.
The uninjected ship divergence and later split require diagnosis; a completed
snapshot transfer does not establish converged recovery.

The effect witness also became unverified before restore: at sample 19 (around
simulation tick 604), GM-1's activity ring had advanced to oldest tick 508 and
no longer retained the tick-405 damage event. GM-2 still retained it. The strict
retention gate correctly fails closed; a future bounded continuous observer
would need to prove overlap before accepting evidence across ring eviction.

### No-injection browser control (diagnostic pass)

`target/1534-source-browser-uninjected-relay-fdaf31c2/ws-relay/result.json`
passed on the same `fdaf31c2` source and receipt, using the same damage and
0.4/0.8 Helm sequence but no incoming-frame mutation. All six peers agreed at
tick 600 (`a523ef4fcf617ebc`) and tick 900 (`8fd77f9070ad5a85`); final ticks were
910–911. Both GMs retained exactly one tick-382 damage event, journal sequence 3,
with prior activity tick 5. This narrows the investigation; it is not a successful
fault recovery case. A synchronous six-app native replay of these inputs also
passed 700 tick comparisons, without the injected fault.

The browser diagnostic result SHA-256 is
`e341008284dc9efd4b044788b7515580164b003a4fa967dcdd64beac2ceae9bf`;
manifest SHA-256 is
`e265a67f36dc6181cf42d7a0a495db1a8374224080f904c1e7c778356faba8d9`.
The isolated diagnostic script hash recorded by that manifest is
`837e0d7b9c6720cb4bd65d87e729b1d26356defa0a52c7c90cdfdd78d45d8aff`.
The script reused the unchanged main matrix and receipt verifier; it did not
modify the served bundle or product source.

### First-mismatch snapshot probe (diagnostic stop)

`target/1534-source-browser-snapshot-relay-fdaf31c2/ws-relay/result.json`
used the same source-matched receipt and stopped deliberately at the first
checkpoint disagreement. At tick 600, the five uninjected peers agreed on
`cbaf846e126a5ef9`; only the injected GM differed (`b1ae0e3b8ed25bc3`). This run
did not reproduce the additional ship-1 divergence seen in the failed rerun.
It stopped before the recovery boundary, so it establishes no recovery pass.

Read-only snapshot exports completed for all six peers at ticks 620–621.
Comparisons use equal simulation ticks: ship-1, ship-2 and ship-4 had the same
snapshot digest at tick 620; ship-3 and GM-2 matched at tick 621. On the injected
GM at tick 620, the owner's thrust was -0.7 rather than 0.8, with corresponding
position and velocity differences; the other three ship physics rows matched.
This confirms the intended injection reached authoritative state. It does not
identify the intermittent additional divergence from the earlier run, whose
artifact did not retain a first-mismatch snapshot or full incoming history.

The result SHA-256 is
`ae04fed2adc405db2576931b24e4bb2723578630b0d407388ec887a6d3a74622`;
manifest SHA-256 is
`38863f47782a3a8f66d9caa864099cb28f5a0d2d062e68c5e06aa2bad68cbf79`.
The isolated observer script SHA-256 is
`72c9195151a318d5fa7f418fe855c6f5daa526d2d0645d9e72a22605eafb9b1c`.
The result records each exported snapshot's hash. The served bundle and
product source were unchanged; exports used the existing snapshot API.

### Continued snapshot probe (additional ship divergence reproduced)

`target/1534-source-browser-continuation-relay-fdaf31c2/ws-relay/result.json`
continued past an expected GM-only disagreement, with bounded authenticated
incoming command history. It reproduced the extra uninjected ship-1 split at
checkpoint 600: ship-1 `0c685a916658bea6`, injected GM-1 `89d18b4b91341370`,
and the other four peers `d5f7ddcc1b022fdb`. All six snapshots were captured;
the four ship snapshots share tick 604, while the GMs share tick 605. The probe
then stopped deliberately, before restoration, preserving that first additional
split. Result SHA-256:
`4d23043d8e608da4e873fb131710b754d9226239109a56e048b4a8f858fb420c`;
manifest SHA-256:
`5e1e4d4289d8afda3a7a9c7ef8f5bf06724c2ed16c21b9d37b0c4078cd25944b`.

The equal-tick snapshots identify a concrete locality defect. The owner's
thrust was 0.8 on both ship-1 and a canonical replica. Its boost was inactive
with battery 0.08166661 on ship-1, but active with battery 1.0 on the replica;
its forward speeds were 12.900043 and 28.0 respectively. `tick_boost` only
advanced `LocalShip`, using local crew sessions and the local Helm display
cache. Other replicas therefore kept boosting after the owning host exhausted
its battery. The focused correction advances every ship from its admitted
actuator inputs. A source-matched live recovery rerun is still required after
that correction; this diagnostic is not an acceptance pass.

Before that correction, a synchronous six-app diagnostic did restore the
injected GM at boundary 1200 and agree at checkpoint 1500 after fresh Helm
inputs. Its workload did not sustain boost through exhaustion, so that passing
core diagnostic could not exclude the captured live defect. The added sustained
boost regression explicitly observes activation, exhaustion and recharge on
all six replicas.

## Outstanding

| Case | Browser direct | Browser forced relay | Browser automatic fallback | Native/mixed |
| --- | --- | --- | --- | --- |
| Non-owner ship loss | Passed at `b4e17e72` | Passed at `b4e17e72` | Passed at `b4e17e72` | Healthy gate failed before injection; no recovery evidence |
| Owner loss | Untested | Failed at `b4e17e72`; passed at `91f382e6` | Untested | Untested |
| GM loss | Untested | Untested | Untested | Untested |
| Divergence restore and exact-once reducer effect | Untested | Failed at `d2b6a8ed`; `fdaf31c2` restores but later diverges | Untested | Untested |
| Two replacement contenders and connected-holder challenge | Untested | Untested | Untested | Untested |

The participant-electorate fix passed seven focused Rust recovery tests and a
WASM configuration check; these are not substitute runtime evidence. The `fdaf31c2` live rerun remains failed for the reasons above. The owner-suffix proof limitation above remains
unresolved even if the ordinary owner-loss cells pass. Native hosts have no
WebRTC; native-involving routes require relay, with browser legs exercising
direct or automatic fallback where applicable. No native-only recovery case,
rendered-browser recovery, physical/mobile/internet recovery, impaired-network
recovery or complete recovery-feedback observation is established here.

The replacement race has focused helper tests but no runtime result yet. Final
integration gates and the final source-matched matrix remain the integration
task's responsibility. Issue #1534 must remain open while these gaps remain.
