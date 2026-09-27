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
retention gate correctly fails closed. The later [continuous observer](1534-effect-witness.md)
requires proven overlap before accepting evidence across ring eviction; its
first live result is recorded below.

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

### Boost-corrected divergence relay run (recurrent GM split; witness refused)

`target/1534-source-browser-divergence-relay-97a6bd01/ws-relay/result.json`
failed on clean source `97a6bd0195665fc059c8dbe34e8bad12dd029d65` after the
boost correction and continuous observer were integrated. The source-matched
receipt and every served bundle hash verified before launch. This run used
ports 18440/18441, `--seconds 10 --timeout 180 --routes ws-relay`, and the
unchanged 90-second fault bound. It ran from 14:41:35 to 14:43:27 UTC on
2026-09-27, exited 1 and completed cleanup without a reported cleanup error.

| Artifact | SHA-256 |
| --- | --- |
| `target/browser-wasm-receipt-97a6bd01.json` | `073365a7fd686b1630847116b8ee126f236247bb3e5aa85bf6cd1672b53b25a4` |
| Application WASM | `28c8e35893dc64743cdb2b8db3c8768343317c07f7496bea020c9db45e8c21ce` |
| Run `manifest.json` | `796f0e94d40fb0b5a1c6cfdc3eac79cf14e6c5136b9bd3198c83d0e57e701ce5` |
| `ws-relay/result.json` | `75c558081d769e79327bbd153be3edb1298099d849f093c4ce295528dba9d478` |
| Recovery runner | `0bbd7fd4539bc76c5264136c808bae4a77f1b7d0b8ee81aede107a883c6cc977` |
| Continuous witness helper | `942918179573a9c935d832404528c1d403e1e96ac01de308c1d8991e0a49ee81` |

The healthy six-peer and transport gates passed. Both GMs recorded the actual
one-time 5 HP direct-damage event at tick 432, journal sequence 3, with retained
earlier tick 5. The runner changed only GM-1's incoming owner SetThrust at tick
435, sequence 45, from approximately 0.8 to -0.7.

All five uninjected peers agreed at every observed checkpoint: 600, 900, 1200,
1500, 1800 and 2100. The previous additional ship-1 split did not recur. GM-1
(slot 5) alone differed. It recorded restoration at boundary 1200 from leader
1, but disagreed again at checkpoint 1500 and restored a second time at boundary
2100. The final simulation ticks were 2186; no two post-restore checkpoints
agreed. A successful restore-time fold does not prove subsequent continuation,
and this recurrent GM-only split remains unresolved. Fresh Helm commands after
the first restore used the original four ship UUIDs at ticks 1204/1205; no later
commands were sent after the second restore. The final latest-boundary identity
gate therefore failed, without establishing a ship reset.

The continuous effect witness independently failed closed on GM-1 at sample
115 (`changed-or-partially-evicted-tick`), after its last accepted observation
through tick 528 and before the first restore. The retained trace contains 114
accepted observations (67,790 bytes), including the original damage event.
GM-2's witness retained that event with no observer error through 1,989 samples.
Accepted observation intervals were at most 64.5 ms. The last accepted ring
had 127 of 128 rows, including four rows at its oldest tick 5; the next outer
sample began at tick 10 and still retained the damage event. This is consistent
with partial eviction of the oldest tick. The exact rejected ring was not
retained, so the artifact cannot identify which row changed or disappeared.
The failure proves neither a duplicate reducer effect nor exact-once effects
through recovery: the activity projection did not establish the required
continuity. Both bounded observer traces remain in `recovery.effectObserverLogs`.
No additional recovery case was run as part of that acceptance record.

### Recurrent divergence reproduced in native continuation

The six-peer injected-command regression reproduced the `97a6bd01` failure
after its workload was extended to leave boost engaged through exhaustion.
The GM restored at tick 1200, then alone differed at tick 1500:
`292f044513bfff30` versus the canonical `5049e9eb8ca69555`. The retained failing
command was `cargo test --features headless --test lockstep_six_peer
injected_gm_command_does_not_diverge_the_other_five_hosts -- --nocapture`, run
on `f78eddab` plus the regression, before the product correction. Its log is
`target/1534-boost-restore-red.log`.

`ControlState::restore_into` cleared explicit drive-write flags but marked the
restored `BoostCommand` and `ImpulseCommand` as Changed. The drive consumer's
direct-write fallback then treated those values as new commands. An exhausted
boost retains its true intent while the battery recharges, so a successful
restore could restart it on the next tick and split the recovered simulation.
Restore now rebases those two change markers to an already-consumed tick.

The corrected `cargo test --features headless --test lockstep_six_peer` passed
all four ordinary tests, with two long-running/manual cases ignored. The
boosted recovery test checks matching checkpoints 1500 and 1800, original ship
UUIDs in fresh Helm commands, and one applied canonical damage result on every
peer. Log: `target/1534-boost-restore-green.log`. These runs used the assigned
worktree and shared native target cache, refreshing `src/lib.rs` before
switching source trees. They establish native in-process continuation after
the correction; source-matched browser/native transport recovery is still
required, and the outstanding matrix below remains authoritative.

### Corrected source-matched browser recovery

Product source `da812fb07149188928561daeea2de2389644ed1c` passed injected GM
divergence on direct WebRTC and forced relay. Both cases restored at tick 1200,
then agreed at 1500 and 1800, retained all four ship identities in fresh controls,
and continuously witnessed the single actual 5 HP effect without duplicate
orders. The direct observers each took 1,658 samples; relay observers each took
1,233. Every observer ended without an error. This resolves the recurrent
post-restore GM split in those two real runtime routes.

The same product build passed GM loss on direct WebRTC, forced relay and
automatic fallback. Every case retained five equivalent survivors and agreed
at checkpoints 600/900 after one HostLoss, at ticks 376, 428 and 366 respectively.
The lost participant was a GM, so ship Backfill is not applicable.

The same build also passed owner loss on all three browser routes. Each case
committed owner continuation, applied one agreed HostLoss with Backfill (ticks
388 direct, 387 relay and 369 fallback), and retained five matching survivors
at checkpoints 600/900. The fallback used the same separately attributed
corrected runner as GM loss.

The original fallback divergence attempt and first GM fallback attempt stopped
before fault injection: the observer's 64 most recent DOM text changes had
evicted retry history. These are retained precondition failures, not recovery
passes or observed product recovery failures. The corrected GM fallback uses
the reviewed durable retry milestone recorder from runner
`45e9a1de59cf4792e13c125d439d27b432681b13`. Its recovery, replacement and effect
helpers are byte-identical to the pinned main helpers. The integrator separately
verified the original clean product source and all bundle hashes against its
original WASM receipt; the run does not relabel that receipt as the runner commit.

| Retained artifact | SHA-256 |
| --- | --- |
| `target/browser-wasm-receipt-da812fb0.json` | `9cbee6003afe510de3f914693305cc6e837c7a9cd39f783bed910c87115bdba9` |
| Application WASM | `2c3e7c095606758a2a93cbd367812375763826b3537fdf9ef760caf32e3b545f` |
| `target/1534-source-browser-divergence-other-da812fb0/direct/result.json` | `2ce565c410c4025bba1b977f82843f624aba977fddc99487fa45875c1c9844ca` |
| `target/1534-source-browser-divergence-relay-da812fb0/ws-relay/result.json` | `74ef80528aef59d1bade65fb57be794a5d334a1001f6c3e7e256b4d1208c9a98` |
| `target/1534-source-browser-gm-all-da812fb0-v2/direct/result.json` | `37eab902d7f73de379ce1aaa6a4ed17c3c17c6dc971603f7f6762a6208d48990` |
| `target/1534-source-browser-gm-all-da812fb0-v2/ws-relay/result.json` | `3c9f27e94fe748ca49236f30b0613c21c514e069914784275b7a92c3dee75530` |
| `target/1534-source-browser-gm-fallback-da812fb0-corrected/automatic-fallback/result.json` | `fa715d6c4a538fe5e51aa6a8b92949f0f2653fdc9bf0977f8256c634f081fcc1` |
| `target/1534-source-browser-leader-direct-relay-da812fb0/direct/result.json` | `fe611779c6c701e7217eb8d745f1c9e2fe5c15fccab84eede7b0b7ed3ce25f0b` |
| `target/1534-source-browser-leader-direct-relay-da812fb0/ws-relay/result.json` | `ddb085416ce6941bcfa558261daf8ac0783144ea45b9390be00e17bf4e458173` |
| `target/1534-source-browser-leader-fallback-da812fb0-corrected/automatic-fallback/result.json` | `3fbfd1bd7861221fb0792f4434378cb474668ec7d0d1f63644e7adfb2b4920b2` |
| `target/1534-browser-runner-provenance-da812fb0.json` | `a2b1cd5af1757c5932fb05e5b9ee1f3437e596606229c12471b27f89d52f6212` |

Commands use `node scripts/fleet-browser-recovery.mjs --failure <case>`, fresh
`--out`, pinned `--dist`, `--seconds 10 --timeout 180`, the named `--routes` and
ports 18440/18441. Original main runs pass `--wasm-build-receipt`; the separately
attributed corrected runner uses the integrator's external verification above.
Browser rendering is disabled in these bounded simulation/transport cases.

### Fallback observer baseline sequencing

The first `9f2ba003` automatic-fallback divergence attempt ran from 18:11:35 to
18:16:20 UTC on 2026-09-27 and failed before injection. Both continuous
observers rejected `backdated-history` at sample 4. The retained ring had a
bootstrap connection row at tick 5099; the requested GM damage and action rows
then arrived at game tick 396. This records an observer precondition failure,
not a duplicate effect or failed divergence recovery. Artifact:
`target/1534-source-browser-divergence-fallback-9f2ba003/automatic-fallback/result.json`,
SHA-256 `969eadcc6e379a777056758a93931bd2969f9eb06ef020fc302147f19c62270a`.

The harness now establishes the actual matching two-GM effect and journal
baseline before installing either continuous witness. It checks that each
initial witness contains exactly the established effect and journal order,
then injects divergence. Later backdated target effects, missing continuity or a
duplicate effect still fails; unrelated late combat rows may be inserted while
the retained history suffix and sorted-ring eviction bounds remain valid.
Native/mixed observers publish an initial
baseline separately and must publish their first continuous projection before
injection. This is a harness-only sequencing change; a corrected runtime run
must establish its own result against the pinned product receipt.

### Corrected fallback effect continuity

The corrected v2 attempt on `9f2ba003` actually restored boundary 1200 and
agreed across six peers at 1500/1800/2100/2400/2700. It still failed its effect
verdict because ordinary hostile damage at ticks 527/1007 arrived after
bootstrap connection rows at 5110/5109. The target GM effect and its journal
entry remained present once. Preserve
`target/1534-source-browser-divergence-fallback-9f2ba003-corrected-v2/automatic-fallback/result.json`
as a failed observer result, not a recovery pass.

The reviewed `c2246356` observer accepts unrelated late rows while preserving
exact target counts, later target-effect backdating rejection, retained suffix
continuity, full-capacity eviction and the bound that a removed row cannot be
newer than the surviving oldest row. The final v4 run passed against the same
independently verified `9f2ba003` product receipt: boundary 1200 restored, all
six peers agreed at 1500/1800, zero duplicate command orders, stable ship
identities, and one target effect continuously witnessed by both GMs in
1,243/1,244 error-free samples. It suppressed 68 actual RTC offers before relay.
Browser and service cleanup completed.

Artifact:
`target/1534-source-browser-divergence-fallback-9f2ba003-corrected-v4/automatic-fallback/result.json`,
SHA-256 `3b1ba56b9750c469fb8776f1780c585038718901dc5ef5ed5ce08d8b59dd67af`.
Runner/product attribution:
`target/1534-browser-runner-provenance-9f2ba003-c2246356.json`.
This separately attributed harness revision does not relabel the product receipt.

### Fresh-Lobby replacement restore failure

On product `9f2ba003`, both real browser replacement races admitted exactly one
contender and refused the other with `slot-taken`. Direct lost slot 3 at tick
425 and emitted one owner claim at tick 451; relay lost slot 4 at tick 406 and
claimed at tick 426. Both six-peer groups stopped at tick 901. The admitted
replacement remained in Lobby without a completed restore while the five
survivors held in progress. The 90-second canonical restore deadline expired;
neither route reached the later connected-holder challenge.

| Retained result | SHA-256 |
| --- | --- |
| `target/1534-source-browser-replacement-direct-relay-9f2ba003/direct/result.json` | `7ae5eb0243a2fd8d60e11952bec4254e8414519996c05dde0935ff97b92244e9` |
| `target/1534-source-browser-replacement-direct-relay-9f2ba003/ws-relay/result.json` | `63331a14e9c7606c059d873ee1e2c7bca28a0f6aef8ebfa041f93e61256f0e12` |

The regression in `tests/lockstep_slot_recovery.rs` now replaces the lost host
with a real native Contract app still in Lobby, rather than an automatically
started headless app. Before correction it stalled without any committed
restore, reproducing the runtime failure (`target/1534-fresh-lobby-replacement-red.log`).
The trusted slot-recovery arm now enables the existing gated canonical
GameStart bootstrap: only the elected leader's accepted record can stage the
saved identities and request `InProgress`. An already-booted reconnect skips
bootstrap; the readiness and connected-holder gates remain unchanged.

`cargo test --features headless --test lockstep_slot_recovery` passed all six
tests after the correction (`target/1534-fresh-lobby-replacement-green.log`).
Both ordinary and fresh-Lobby replacements commit the record and retain more
than sixty shared post-restore digest checks. This is integration evidence;
later source-matched browser race results are recorded in
`1534-mixed-native-recovery.md`; native/mixed races remain untested.

## Outstanding

| Case | Browser direct | Browser forced relay | Browser automatic fallback | Native/mixed |
| --- | --- | --- | --- | --- |
| Non-owner ship loss | Passed at `b4e17e72` | Passed at `b4e17e72` | Passed at `b4e17e72` | Native relay passed at `da812fb0`; mixed healthy gate previously expired before injection |
| Owner loss | Passed at `da812fb0` | Passed at `da812fb0`; prior evidence retained | Passed at `da812fb0` with attributed milestone recorder | Native healthy precondition failed at `9f2ba003`; no owner fault injected |
| GM loss | Passed at `da812fb0` | Passed at `da812fb0` | Passed at `da812fb0` with attributed milestone recorder | Native relay passed at `9f2ba003`; earlier precondition failure retained |
| Divergence restore and exact-once reducer effect | Passed at `da812fb0` | Passed at `da812fb0`; prior failures retained above | Passed at `9f2ba003` with attributed `c2246356` witness; earlier observer failures retained | Untested |
| Two replacement contenders and connected-holder challenge | Passed at `d032d604`; earlier restore failure retained | Passed at `d032d604`; earlier restore failure retained | Passed at `b16122a0` with separate phase deadlines; earlier shared-budget timeout retained | Untested |

The participant-electorate fix passed seven focused Rust recovery tests and a
WASM configuration check; these are not substitute runtime evidence. The `97a6bd01` live rerun remains failed for the reasons above. The owner-suffix proof limitation above remains
unresolved even if the ordinary owner-loss cells pass. Native hosts have no
WebRTC; native-involving routes require relay, with browser legs exercising
direct or automatic fallback where applicable. The native-only ship-loss pass
is detailed in `1534-mixed-native-recovery.md`; no rendered-browser recovery,
physical/mobile/internet recovery, impaired-network
recovery or complete recovery-feedback observation is established here.

The replacement race runtime failures and subsequent integration regression are
retained above; corrected browser races now pass all three routes, with
source attribution and bounds in `1534-mixed-native-recovery.md`. Final
integration gates and the final source-matched matrix remain the integration
task's responsibility. Issue #1534 must remain open while these gaps remain.
