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


The refusal was traced to the native owner adapter omitting `onBeginGmJoin`
and `onRefuseGmJoin`, while `createFleetOwner` defaults the begin callback to
`false`. Its update adapter also lacked terminal `GmJoinRuntime` progress, so
starting Rust recovery alone could not finish public admission. The original
transport closure remains unexplained.

The native adapter now carries typed begin/refuse records to the shared Rust
transaction and returns its authoritative terminal progress to
`completeGmJoin`. Both owner and member options carry these callbacks, including
members later promoted to ownership. Rust retains the existing owner,
slot/operator identity and departed-slot gates. Candidate bootstrap and snapshot
transfer reuse the shared recovery path; an accepted transport never suffices
as proof of restore. The native leader case still requires a source-matched
runtime rerun after the correction.


The JavaScript owner/promoted-member regression first failed because the begin
callback did not exist, then passed with the existing fleet session tests
(90/90). The native bridge transfer regression first failed decoding the absent
record, then passed with the existing direct reconnect case (2/2):
`cargo test --features headless --test lockstep_snapshot_transfer gm_reconnect`.
These tests use real simulation restore, canonical command/GM history, digest
agreement and explicit Resume. They do not substitute for native process/relay
runtime evidence. Retained logs are
`target/1534-native-gm-bridge-red-v2.log` and
`target/1534-native-gm-bridge-green.log`; the first `red.log` instead records a
corrected test setup error before product ingress was reached.


## Deterministic same-process native GM redial probe

`fleet-native-recovery.mjs --failure gm-redial` extends the existing native
runner with a bounded redial after the six-peer healthy workload. The private
instrumented bundle retains the existing member's real bridge socket and calls
its ordinary `close()` once. It creates no new member and never rewrites
simulation state. Rust closes the old transport; ordinary production redial
must open a newer native wire generation.

The verdict requires the same live child PID/start time before and after, one
member creation, the new Rust-observed wire generation, and unchanged private
capability/operator/slot comparisons (only booleans are retained). It then
requires six identical authoritative reconnect commits, five agreed survivor
HostLosses, an explicit applied GM Resume, stable original ship identities in
fresh controls, no duplicate outgoing orders or observer errors, and two exact
matching post-commit digest checkpoints. Use the existing native invocation,
matching `--build-receipt`, `--seconds 900 --workload true` and a fresh output
directory, changing only `--failure gm-redial`. Its recovery wait is bounded at
600 seconds. No passing live redial result is established; the first
source-matched run and its failure are retained below.


### Same-handle GM continuation stream failure

The first deterministic native redial ran on clean product
`d032d604bd2f66957fd22a985a417b0490bc952c`. It reached the six-peer healthy gate,
closed GM 2's generation 0 at 19:34:44.427 UTC on 2026-09-27, and observed
native generation 1 open at 19:34:44.572. PID 29044 and its start time remained
unchanged and live; the private capability, operator and technical slot all
matched. Five survivors applied HostLoss at tick 606. The owner scheduled a
reconnect pause at 612, but emitted `owner-continuation-refused` with
`continuation-stream-gap` at 19:34:45.013. No reconnect commit occurred.

Retain `target/1534-native-gm-redial-relay-d032d604/matrix.json`, SHA-256
`a3809a3f8e289d5d862ef43b9514ae6f6ea88eea657e8ae7e71a109672e574a6`,
and its native build receipt SHA-256
`b26d304b65bbe76e63b6850b2c9b32861046d5cb41c5491c3648101dd451cc01`.
The 112.532-second run is a concrete recovery failure, not a healthy-gate miss.

The same member handle had kept numbering continuation frames while its socket
was disconnected. A later pending-candidate envelope entered the owner's
admitted journal with missing rows. Its retained journal also caused the GM
to ignore the raw private Pause/snapshot lane. The correction retires that
stale journal only at authenticated reconnect-pending, rejects candidate
continuation envelopes at the owner, and recreates the member journal from
the owner frontier only after terminal Welcome. Admitted-stream gaps still
fail; no missing simulation history is waived.

The direct and relay regressions in `fleet-gm-redial-stream.test.js` both
reproduced the exact stream-gap error before correction
(`target/1534-gm-stream-red.log`). Six focused suites then passed 113/113,
including ordinary owner handoff, continuation, fleet admission and native
adapter tests. These are protocol fixtures; corrected native runtime recovery
still needs a new source-attributed run.

## Browser replacement rerun on d032d604

The source-matched browser replacement matrix at
`target/1534-browser-replacement-d032d604` passed direct WebRTC and forced
WebSocket relay. Each admitted exactly one contender, restored at boundary 900,
refused the loser's later connected-holder challenge, and agreed at checkpoints
1200 and 1500 across the six resulting peers. Result SHA-256 values:

- `direct/result.json`: `5cb8620d7fe87d43a528a4e54652bc284b60d44c9f00d5d63b98a2f9d2c55cb1`.
- `ws-relay/result.json`: `2efdfe7d54a3b0c1032315e3da29945cd4c437cdf3a448cff28f4681879e6491`.

Automatic fallback failed the runner's 90-second total deadline during canonical
restore. Retain `automatic-fallback/result.json`, SHA-256
`fce5c39b078414188dc20038dafcd001dad4f5cd6edd9ba75bba679a08de5337`.
The victim's agreed loss was tick 410. Replacement 2 won slot 3 and the other
contender was refused; the winner was still in Lobby at tick 148 when the five
survivors had reached about 3411. No recovery commit or slot-claim egress was
observed, so this is not a passing recovery cell.

The runner's single deadline includes both admission and recovery. Production
first-join fallback spends 8, 16, 30 and 30 seconds on direct attempts, plus
backoff; initial admissions in this run took about 86 seconds. The replacement
was at tick 7 in the first admitted race observation and tick 148 at timeout,
leaving only about five seconds of post-admission observation. The later
connected-holder challenge would need another fallback admission ladder too.
The current total budget is therefore inadequate for this route. This evidence
does not establish that a longer wait alone fixes the absent recovery: rerun
with separately bounded admission, restore and challenge phases, retaining
phase timestamps and slot-claim observations. Keep the failed artifact and the
cell open until canonical restore, holder protection and digest agreement are
actually observed.

### Bounded replacement phases for the next run

The browser replacement runner now allocates separate deadlines to loss (90s),
concurrent admission (120s), canonical restore (90s), the connected-holder
challenge (120s), and final digest agreement (90s). The admission and challenge
budgets each cover their own production direct retry ladder before fallback.
The runner manifest records these budgets instead of implying one total 90s
fault deadline. Replacement runs also default the per-page evaluation timeout
to 180s, so that wrapper cannot truncate a 120s admission/challenge phase. An
explicit caller-supplied --timeout remains effective and is retained in the
matrix options; a smaller value can still end an evaluation early. No phase
poll renews its deadline.

Each phase retains its name, budget, start/deadline/finish times, elapsed time,
last operation and passed/failed/timed-out status in the result, including on
failure. Completed phases record their observed milestone: agreed loss tick,
winner and initial tick/phase, recovery boundary/claim sequence and peer ticks,
explicit challenger refusal, then common digest ticks. Existing race, exact
restore, connected-holder, transport-route and six-peer digest checks are
unchanged. Focused fake-clock tests exercise independent deadlines and every
phase timeout; they provide no new browser recovery acceptance. The retained
d032 automatic-fallback artifact remains inconclusive; the separately bounded
b161 rerun below provides the later passing observation.


### Same-process candidate Pause on the solo f2ed run

The sequential run at `target/1534-native-gm-redial-relay-f2ed6da8-solo`
passed its healthy baseline and triggered GM 2 redial at 20:38:03.358 UTC on
2026-09-27. Generation 1 opened at 20:38:03.501 and capability/operator/slot
comparisons remained true. Survivors applied HostLoss at 605 and entered
Transferring at the owner-authored pause boundary 610. The returning GM stayed
at tick 603 with idle GM join progress. This is a recovery stall after the
trigger, unlike the earlier concurrent-build run's unhealthy precondition. The
bounded run ended without Commit; retain its matrix SHA-256
`0b18e15e4042fff888f54fb58a86dc9af6551d3e2c26d3d765e72b6b4a9fe83f`.

The same handle retained its admitted roster/session, so the JavaScript bridge
correctly skipped fresh roster bootstrap. Rust's candidate Pause path required
GmJoinBootstrap anyway and silently ignored that Pause. The correction accepts
an owner-authenticated reconnect Pause against the exact retained local GM
identity, owner and approver; first-time joins still require their bootstrap.
No roster is adopted twice or removed. The same-handle integration regression
retains FleetRoster/FleetLockstep before Pause, then requires real canonical
command/GM history restore, digest-proven Commit, explicit Resume and continued
digest agreement. Corrected live runtime proof remains outstanding.

The first focused run after accepting that Pause reached restore and Commit but
then exposed a stale retained-session frontier: the candidate waited at tick 96
for an owner watermark of 12. Commit had called rejoin on the candidate's own
slot, which is intentionally inert. The correction rebases its existing live
peer watermarks to the proven commit tick plus delay, retaining departed-peer
exclusions, before explicit Resume. The integration regression asserts this
frontier as well as subsequent digest agreement; the failed intermediate run
is retained in `target/1534-same-handle-bootstrap-green.log`.

Focused validation of the completed correction:

- `cargo test --features headless --test lockstep_snapshot_transfer gm_reconnect`:
  3/3 pass, including the retained-process native bridge case through 24 rounds
  of continued digest agreement. Log: `target/1534-same-handle-bootstrap-green-v2.log`.
- `cargo test --lib --features headless gm_join::tests`: 23/23 pass, including
  wrong identity/owner/approver and first-time rejection, plus canonical frontier
  rebase retaining departed exclusions. Log:
  `target/1534-same-handle-bootstrap-unit-green.log`.
- The same-handle integration case first failed the missing pause hold on the
  old implementation; retain `target/1534-same-handle-bootstrap-red.log`.
- PASM validate, scan and traceability all exit 0 on the final contract; logs
  are `target/1534-same-handle-bootstrap-pasm-*-v3.log`.

These targeted checks are complemented by the corrected six-process
active-workload runtime observation below.


### Corrected native GM redial PASS on b16122a0

On 2026-09-27, the six-process native relay case passed its healthy admission
and active-workload baseline, same-process GM reconnect, and post-Resume digest
gates in 171.423 seconds. Product and runner both used clean source
`b16122a02414ed67cd5d396d0bed889ae275cc82`.

GM 2 retained PID 10360 and its process start time, capability, operator and
slot while transport generation 0 closed and generation 1 opened. The agreed
loss was tick 604; all six peers reported the same reconnect Commit at tick
609. Explicit Resume was accepted and applied. All six subsequently agreed at
tick 900 (`abc485a58080e5c1`) and tick 1200 (`7b7ff12876ec618e`), with no
repeated command orders. The result records no failure and sets admission,
workload and recovery verdicts to true.

Retained evidence under the main checkout's `target/` (SHA-256):

- `1534-native-gm-redial-relay-b16122a0-corrected/matrix.json`:
  `016567084e7f38f05ffd42d5eb08568f0eb79b96fcfb1e87b96e1c371f4561eb`.
- `native-build-receipt-b16122a0.json`:
  `f56f4940768927552c47ce396b6452f14eff1c36d9ec618a91c134041c5ef731`.
  It records clean source, the refreshed library and dependency attribution,
  content/bundle/SDK hashes, and the successful native build. Its archived
  `native-build-b16122a0-provenance/phoenix-host.exe` was independently hashed:
  `7f5a4f2ed618663369ef4423c55358da41708faf3dd6c1ff52af7aedb305ffc2`.
- `1534-native-gm-redial-relay-b16122a0-corrected-service/attribution.json`:
  `4ddf01fffd131868f71163753a5916e1d6b9661d076d1168bc8deb34a58ecbd9`.
  It pins the same source and receipt, records service exit 0 and confirmed
  cleanup, and reports 37,923 reliable frames seen/written with no cancellation
  or queue overflow. Delay and loss were both zero.

This passes the bounded all-native GM socket-redial cell on loopback relay.
It does not establish the other unrun native/mixed fault cells or impaired
transport coverage. The earlier failed artifacts remain diagnostic evidence.


### Browser replacement automatic fallback PASS on b16122a0

The corrected phase-bounded browser run passed on 2026-09-27 from 21:45:14.376
through 21:52:55.277 UTC. Its manifest and source-matched WASM receipt name
`b16122a02414ed67cd5d396d0bed889ae275cc82` with an empty source patch;
the manifest's 1,662 bundle hashes match the receipt exactly. The per-page
evaluation timeout was 180 seconds.

Ship 2, slot 2, was lost at agreed tick 440. Both contenders started their
claim in the same recorded millisecond; only `replacement-1` received slot 2,
and `replacement-2` was refused with `slot-taken`. All six resulting peers
reported canonical restore boundary 2700, claim sequence 1, and InProgress.
The outcome confirms exactly one claim and a restored roster. The loser's
later challenge was also refused with `slot-taken`, preserving the holder.

Every phase passed within its independently recorded deadline:

| Phase | Observed seconds | Budget seconds |
| --- | ---: | ---: |
| Agreed loss and Backfill | 0.278 | 90 |
| Concurrent admission | 85.782 | 120 |
| Canonical restore | 17.298 | 90 |
| Connected-holder challenge | 85.058 | 120 |
| Final digest verification | 0.009 | 90 |

The final check used checkpoints collected after challenge start at tick 2735,
including while its fallback admission ladder was pending; 0.009 seconds is
verification time, not the time needed to simulate new checkpoints. All six
agreed at 3000 (`e741e0f4dfc3581f`), 3300 (`3cfc6414c3d1fcba`), and
3600/3900/4200/4500. The result records no telemetry overflow and confirms the
fallback route. The service suppressed 80 RTC offers; the winner sent four
offers, opened relay, and retained no selected direct connection. Artificial
relay delay/loss were zero, with no dropped snapshots or queue overflow.

Retained evidence under the main checkout's `target/` (SHA-256):

- `1534-browser-replacement-b16122a0-fallback/automatic-fallback/result.json`:
  `2d59fe4fe566ac8f484d6cf8ffde185cb9dff1fbf4e53999320251e049f3ed00`.
- `1534-browser-replacement-b16122a0-fallback/manifest.json`:
  `74be1bd5bf12598a27d46fe3d0d787d052d2df8967bc6d7a89f47e42b8e1f85e`.
- `browser-wasm-receipt-b16122a0.json`:
  `cc247306f53280ca0f0b168d83c5a8e2b73465b0b7b1fbb1515df4e193cdf377`.

This closes the observed browser automatic-fallback replacement gap while
retaining the d032 shared-budget timeout as historical evidence. It is a
single-machine, browser-only loopback run with real WASM simulation and Bevy
rendering disabled. It establishes no native/mixed replacement, added relay
impairment, physical-network, endurance or performance acceptance.
