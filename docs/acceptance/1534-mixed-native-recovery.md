# Mixed browser/native recovery observations (#1534)

`scripts/fleet-mixed-recovery.mjs` extends the real mixed matrix after its healthy
six-peer workload gate. The composition is browser ships 1/2 and GM 1, native
ships 3/4 and GM 2, with twelve active Station documents. No runtime pass is
claimed by the runner or its fixture tests alone.

## Invocation

Use the integrator's source-matched native binary and browser/native bundles,
and both build receipts. Keep them unchanged for the run. For example:

```powershell
node scripts/fleet-mixed-recovery.mjs --failure ship --routes ws-relay --render --seconds 3 --deadline 295 --binary <phoenix-host.exe> --bundle <native-dist> --dist <browser-dist> --source <source-checkout> --build-receipt <native-receipt.json> --wasm-build-receipt <wasm-receipt.json> --out <new-artifact-directory>
```

Repeat with `--failure gm`, `leader`, `replacement`, or `divergence` in new output directories.
The `ship` and `gm` cases terminate native ship 3 or native GM 2; `leader` closes
the browser owner's simulation page. `replacement` first verifies native ship 3
loss, then releases two prebooted native contenders to request the same technical
slot within 250 ms. Exactly one must be admitted and the other refused with
`slot-taken`. After restoration the loser challenges the connected winner and
must again be refused; two matching checkpoints must follow that challenge.
The observer supplies the existing `join` API's claim
argument; it does not change admission or write simulation state. Each process
has a private identity-store directory. No reconnect credential is logged.

The healthy matrix's deadline includes recovery. Defaults remain 290 seconds
for the overall case and 90 seconds after the fault. Explicit `--deadline` may
extend the overall window up to 900 seconds, and `--fault-seconds` may extend the
post-fault window up to 600 seconds; the fault window is always capped by the
remaining overall budget. The two healthy and two post-fault checkpoint gates
are unchanged. For a slow rendered rig, add `--deadline 900 --fault-seconds 600`.
Startup or automatic fallback can consume the budget and fail explicitly.
Native probes keep the default 300-second lifetime for case deadlines up to
295 seconds. An explicitly longer case gives each probe `deadline + 60` seconds,
bounded by 960 seconds, plus bounded and confirmed process cleanup. A native
probe invoked alone retains its 45-second default. Telemetry and observation
buffers remain bounded; overflow fails the run.

## Required evidence

- Six distinct admitted baseline slots and the existing healthy workload gate.
- Actual victim process/page exit, stable survivor slots, exactly one identical
  applied HostLoss per survivor, and crew removal/Backfill for a lost ship.
- Browser simulation progress and the reduced wait set. Native progress is
  measured by actual outgoing digest checkpoints; no synthetic phase or mesh
  tick is substituted for absent native diagnostics.
- Two exact 16-digit hexadecimal matching post-loss checkpoints from all five
  survivors; contradictory repeated checkpoints fail.
- For owner loss, committed continuation and the same loss watermark in every
  browser/native survivor's authoritative status.
- No repeated outgoing command order or observer/runtime error. Outgoing order
  uniqueness does not independently count every simulation side effect.
- Replacement additionally requires all six peers to report one restore boundary
  and claim sequence, one authoritative SlotClaim, the original technical slot,
  the old ship entity ID in a new post-restore command, and two fresh matching
  six-peer checkpoints.
- Divergence changes exactly one incoming authenticated `SetBoost` command on
  native GM 2. Both GMs first establish one actual 5 HP direct damage event and its journal
  order, then start the shared bounded-ring witness before injection. Each
  initial witness must contain exactly that established effect. Recovery requires the agreed
  snapshot boundary, two matching later checkpoints, original ship controls,
  no repeated outgoing orders and unchanged effect/journal evidence. A sampling
  gap or witness overflow fails the case.

Raw native events, process manifests, source/bundle/binary hashes, browser
observations, relay counters and recovery verdicts remain in the ignored output
directory. Overflow is a failure. A missing observation never becomes a pass.

## Limits

This is one-machine loopback, with rendered browser WASM and real native
Bevy/Ultralight. Its original owner is a browser; the all-native runner below
exercises native owner promotion. These runners do not claim arbitrary
packet-loss tolerance, endurance, physical-network behavior or complete
effect-count auditing beyond the single witnessed damage effect.
An old-owner suffix retained only by a non-successor has no transferable origin
proof and intentionally remains held/refused. A failed owner-continuation cell
must retain that evidence rather than weaken the provenance gate.

## All-native recovery

Start the ordinary loopback rendezvous development service separately and keep
its log. Run six actual native hosts with a retained clean source/binary build
receipt, twelve embedded Station documents and two embedded GM workspaces:

```powershell
node scripts/fleet-native-recovery.mjs --failure ship --binary <phoenix-host.exe> --bundle <native-dist> --source <clean-source-checkout> --build-receipt <native-receipt.json> --out <new-artifact-directory> --rendezvous http://127.0.0.1:18441 --origin http://127.0.0.1:18440 --seconds 900 --workload true
```

Repeat for `gm`, `leader`, `replacement` and `divergence`. Native transport is
relay only; native direct WebRTC and automatic RTC fallback are N/A. Mixed runs
still cover direct, forced relay and fallback on their browser legs. The native
runner uses the same actual recovery projection and evidence gates as the mixed
runner; it never supplies a synthetic native tick or phase. Its `healthy` record
is captured before the fault; `recoveryPassed` is the post-fault verdict.

Runner source can be a reviewed harness commit newer than the built product.
The mixed manifest records runner revision/patch separately from the explicit
product source checkout, both receipt hashes and all instrumented helper hashes.
No runtime pass follows from implementing or testing these hooks alone.

## First rendered mixed measurement

The clean source-matched `91f382e6` forced-relay native ship-loss invocation
expired at the original 295-second limit before any fault was injected. All six
peers agreed at tick 300 (`f84aec42538b44cd`); final browser ticks were 354/355/355.
Station Applied feedback and GM actions were observed without runtime errors.
All three native children confirmed exit and unchanged binary hash. Raw evidence
is in ignored `target/1534-mixed-native-ship-relay-91f382e6/`; result JSON SHA256
is `5e71f2c252637ea442025bf206d47234de09a9decea1d92d60a5646a6bc2d01b`.
This establishes a measurement-window limitation, not a recovery pass. The
extended window is opt-in and must produce its own evidence.

## Current source native ship loss

The all-native relay ship-loss case passed on product source
`da812fb07149188928561daeea2de2389644ed1c`, using clean runner
`39e89c895b26ead247074a23aafe8c5f53c18de7`. The source-matched
`cargo build --features ultralight --bin phoenix-host` completed in 3m 02s.
The retained receipt records the refreshed library timestamp, current main
dependency paths, copied executable, SDK libraries, content and bundle hashes.

The first process ran from 17:27:32 to 17:30:11 UTC on 2026-09-27. The whole
case took 160.397 seconds within its 900-second limit. All six peers completed
the twelve-Station/two-GM workload and agreed at ticks 300/600 before the fault.
Native ship 3 occupied technical slot 4 in this run; its process exit was
confirmed. All five survivors applied exactly one HostLoss at tick 603 and
reported that ship uncrewed/Backfill. They agreed at ticks 900
(`7d7d4e0afd287b74`) and 1200 (`bcc297b3707eee76`), retaining their slots and
unique outgoing command orders. All six process manifests confirm cleanup and
unchanged executable hashes; the rendezvous service also confirmed exit.

| Retained artifact | SHA-256 |
| --- | --- |
| `target/native-build-receipt-da812fb0.json` | `f93d8bdc8db2f40729ae83422cf7e04730f82643d90916ad90688ed8ceebda96` |
| Native executable | `c1ebfa3e6b3a8128e4ee357cbdcdb2c541d980cbe446b9593ed63db02c8ff9a6` |
| `target/1534-native-ship-relay-da812fb0/matrix.json` | `5bbe5fa52c8393d77c08ae0a03666200dc0011ac7152ca8d34ae4e6605b06536` |
| `target/1534-native-ship-relay-da812fb0-service/attribution.json` | `a9b3b4190fe073c640f06571766cae287cbc0c2d7bb709fe5c87053ef12f5c7a` |

The service observed 31,302 reliable relay frames, no configured impairment and
no queue overflow. The terminated victim's raw teardown stream includes an HTTP
observer connection reset after intentional termination; survivor verdict
evidence stops collecting that victim at the confirmed fault request. No
survivor runtime/observer failure occurred. This case proves native non-owner
ship loss only; the other native and mixed fault cells remain separate work.

## Native GM precondition failure and reconnect regression

The next all-native relay run used the same `da812fb0` product receipt and
runner `f6be20ad`. It failed before the requested GM fault while waiting for
matching native digests after workload receipts. The artifact is
`target/1534-native-gm-relay-da812fb0/matrix.json`, SHA-256
`37e7db386ea1ccb5a19f0d78029bea252ae085a46ac37df26ad47ba78bdb8560`.
This is an unsuccessful precondition, not a GM recovery result.

Ship 2 had been admitted at 17:37:21.346 UTC on 2026-09-27 and remained a live
native process. Its relay link closed at 17:37:48.423; a new attempt became
ready at 17:37:48.767 and received `recovery-only` at 17:37:48.837. Five peers
applied the unrequested ship HostLoss at tick 71. The original socket closure
has no retained error reason and remains unexplained diagnostic evidence.

The subsequent refusal exposed a reproducible product defect: the same member
handle sent its original null slot claim after admission. Focused regressions
close both a direct channel and a relay socket after freezing the fleet. They
failed with `recovery-only` before the correction and pass when the redial
claims its admitted slot. They also require the original hull, one loss/claim,
no second simulation adoption, and refusal of a competing connected-slot claim.
`npx vitest run tests/client/fleet-session.test.js tests/client/host-mesh.test.js tests/client/native-fleet-peer.test.js`
passed 162 tests on the correction. Actual native GM recovery remains unproven
until a new source-matched binary and bundle execute the case successfully.

## Rebuilt source native GM loss

After the redial correction, the all-native relay GM-loss case passed on clean
product `9f2ba0036e39100c34ab76ada2eab215d89d3592` with clean runner
`cb69607e88c190206ac6f8426d063e81d939d0f1`. The matching native build finished
at 18:07:44 UTC on 2026-09-27 after 27.12 seconds; its receipt retains the
refreshed main library dependency records, archived executable and SDK hashes.
The case took 160.760 seconds within the 900-second limit. Its command was
`target/run-native-1534-current.ps1 -Failure gm -ExpectedRevision 9f2ba0036e39100c34ab76ada2eab215d89d3592`,
which invokes the documented all-native runner with workload enabled and keeps
its relay service log and counters.

All six participants completed the twelve-Station/two-GM healthy workload and
agreed at ticks 300/600. The runner then terminated native GM 2 in slot 6.
All five survivors applied one HostLoss at tick 603 and agreed at ticks 900
(`c6f9a0be9ced1ac1`) and 1200 (`b411b8fe6a84cd1f`), with unchanged survivor
slots and no repeated outgoing orders. Ship Backfill is not applicable to GM
loss. All six process manifests confirm cleanup, unchanged binary hashes and
no process failure; the service also confirmed cleanup. Raw HTTP observer
connection resets occur during process teardown, after the collected verdict.
The service saw 31,113 reliable relay frames, no snapshots, no impairment and
no queue overflow. This run proves native GM loss; it does not establish the
cause of the earlier spontaneous socket closure or directly inject a redial.

| Retained artifact | SHA-256 |
| --- | --- |
| `target/browser-wasm-receipt-9f2ba003.json` | `eeb9437fcb25690ed95489e462b527ff612638d73cc20a216a289da1c8557d13` |
| `target/native-build-receipt-9f2ba003.json` | `75a801b14dd0962a675fdcdf7ff4cf263c9fa6f809b92ab051877305382e0242` |
| Native executable | `9c2928b6658cd894e06af4c6f0963b2f34fb9ba40f98911a842ab5a54dfa3887` |
| `target/1534-native-gm-relay-9f2ba003/matrix.json` | `edbfa930adc67299a571b75a10443d1839ccc7f255c4c8e2ad73da863cea9d88` |
| `target/1534-native-gm-relay-9f2ba003-service/attribution.json` | `590acdea3bdd2ed4ed00636c69d18c841ea7ce297367f860c5e73c59ba4737df` |

## Native leader precondition failure

The next `9f2ba003` native relay run, using clean harness `4c24b4f4`, failed
before its requested leader fault. GM 1 had been admitted at 18:26:33.530 UTC
on 2026-09-27; its link closed at 18:27:00.904, redial became ready at
18:27:01.271, and `gm-join-refused` followed at 18:27:01.410. The case ended
at the healthy matching-digest gate after 66.892 seconds, with no recovery
transaction injected. Retained artifact:
`target/1534-native-leader-relay-9f2ba003/matrix.json`, SHA-256
`911b2318f939131725aefd9bb31c1db7d5c58046ab449e02ec05805be41b7713`.
This is a failed precondition and leaves native leader recovery unproven.
