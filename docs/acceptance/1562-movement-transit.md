# Increment 18 — Movement and transit acceptance

Increment 18 is in progress under the [parallel plan](../gameplay-redesign/implementation/parallel-work.md).
The [implementation contract](../gameplay-redesign/implementation/increment-18.md)
defines the full exit. This record distinguishes integrated source from verified
gameplay; it does not close the increment.

## Integrated source checkpoints

- Existing paid movement uses canonical per-axis Direct/Assisted intent. Match
  Speed retains the selected permitted contact's dated velocity; steering remains
  independent and throttle input cancels matching. Human and ordinary AI use the
  existing admission and individual System controllers.
- `30363084` adds visible HUD controls and accepted Workshop profile editing.
  `3d737f6a` corrects actual lateral owner/publication resolution; `c0ca5827`
  fixes its sibling test's validator path.
- `153a0db3` makes sustained Boost use its actual fitted owner, Operation and paid
  work. Partial reserve can start Boost; the derived readout reports the actual
  integrator grant. No second physics owner or saved Boost timer was introduced.
- `f5f0c2e1` adds a full-host joined-crew movement/continuation probe. Its source
  is integrated; execution remains pending at this checkpoint.

The integrated client/Workshop run at `873ca2d9` passes 548 tests across 18 suites,
including Helm controls/profile editing and Boost. The client bundle builds at
protocol 17. The simulation test compile check passes after the bounded integration
fixes. Neither result establishes native gameplay or device acceptance.

## Outstanding proof and scope

The [integrated receipt](../gameplay-redesign/implementation/parallel-checkpoint.json)
records a later rebuilt suite with 343 native passes and seven failures. The
actual renamed lateral feed, four Boost cases and both movement fixture
bootstrap corrections pass. Other failures and their unexecuted corrections
remain recorded separately.

Run actual paid Direct movement, coasting, turning and ordinary AI handoff in the
full host. Compare fresh JSON restore using complete captures/digests for 360
fixed steps and exercise malformed candidate no-write cases. Execute actual
renamed-owner and Boost tests, then record the exact tested revisions and logs.

The first full-host attempt stopped at a neutral-coast grant-ratio assertion.
`e606b4c1` checks actual work on every tick, retaining the same momentum and
displacement proof. `44e9f9a2` removes an incorrect net-reserve expectation:
passive battery drain continues while propulsion emits no work. The retry passes
the movement/coasting assertions and fails at the first complete JSON comparison
after restore. The exact continuation assertion remains; its state difference
is under investigation.

Paid Dock separation is integrated as `370baf93`, with whole-candidate preflight
and compatibility joins in `fcf386de`. Explicit moving equipment uses the existing
physical pose and paid work; stationary providers release without moving their
body. The merged Dock/illumination client suites pass 30 tests, and protocol 18
client/Workshop builds pass. Six of nine new Dock cases pass in the rebuilt native
suite. `c46733dd` corrects the missing-physics lifecycle report and two legacy
tests that moved Transform without canonical ShipPhysics. Their retry is pending.

Later integrated actual-host evidence supersedes the earlier continuation
failures above. Joined Direct/coasting/turning and ordinary AI handoff pass the
complete 360-step JSON capture/digest journey (5.07 seconds). LocalJump also
passes (42.56 seconds): positive paid partial preparation, three malformed
no-write candidates, Human release during preparation, exact 360-step continuation,
one arrival/reset and fuel commitment, then admitted Logistics replenishment
with exact finite local/hold stock conservation. Runtime is through `909a47b6`,
test `7bb48028`, corrected accepted stock feeds `33b17e18`; the executable
predates subsequent typed channel defaults. Logs are
`.phoenix/helm_movement_continuation-7729ded574618af9.exe.current.log` and
`.phoenix/local_jump_continuation-18e6c4158f340b84.exe.current.log`.

Inter-system transit, formation requests, the small-craft join and their continuation
remain open. Browser/native device controls, performance, balance and
human workload across supported seat counts are unverified. No full integration
gate, push or increment exit is claimed.
