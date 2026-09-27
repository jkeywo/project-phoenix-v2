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
  native GM 2. Both GMs continuously witness one earlier 5 HP direct damage
  effect with the shared bounded-ring reducer. Recovery requires the agreed
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
