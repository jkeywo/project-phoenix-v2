# Default-pool perturbation and resume guards (#1400, slice 4)

The four existing determinism binaries now include a `default_pool_` parent.
Each launches exact original guard tests in fresh copies of its own executable,
sequentially. Bevy's process-global pools cannot inherit the parent's pinned
pool. The original tests and their assertions remain in place and still select
their pinned setup when run normally.

`tests/common/default_pool.rs` changes only the test fixture's `deterministic`
argument in the child. Seeds remain programmatic inputs, because the real CLI's
`--seed` also selects `--deterministic`. No production flags, scheduling edges,
authored values, or digest fixtures change.

The children reuse these cases:

| Guard | Original cases preserved | Completed App observations |
| --- | --- | --- |
| Archetype order | Duel seed 42, 900 updates; authoritative equality and the non-vacuous archetype-layout change | 4 |
| Schedule order | Duel seed 42, 900 updates; inert Input-system equality and scope-by-scope diagnostics | 4 |
| Registration order | RNG coverage seed 20260899, 300 updates; canonical order and shuffles 1/0xC0FFEE, including ship/collision preconditions and full fingerprints | 3 |
| Snapshot resume | Four exact guards: Duel seeds 8622026/8622027/8622057 with 400 updates before capture and 120 continuation frames; Combat Test seed 8622026 with its existing streamed-belt round-trip and zero-frame continuation bound; one pending script callback; two ordered callbacks due on the same tick | 12 |

The snapshot cases retain ordinary FileStore save/load, complete reconstruction,
exact captured digest, `vellum_save::verify`, and every continuation-frame
comparison. Combat Test's documented low-LOD continuation gap and the existing
ignored divergent duel seed are unchanged; this addition does not claim those
gaps are solved.

The two script guards each report the original and freshly booted App. The
pending-callback guard retains its saved identity, due tick, firing and
continuation checks. The same-tick guard restores two distinct callback
identities with their saved queue order and makes their effects noncommutative:
only the saved order produces victory. It compares the complete restored queue
and every continuation tick's digest, counters and terminal outcome, continuing
after execution to catch a replay. These are the same guards that run pinned;
the default parent changes their pool selection only. See
[Script callback order](script-callback-order-proof.md) for their bounded scope.

Every completed observation checks the actual `ComputeTaskPool` has more than
one worker, `FixedUpdate` uses `MultiThreaded`, the actual `SimRng` seed matches,
and a real mission has advanced beyond tick 0. The parent requires successful
child exit plus the exact expected report count, checks distinct process IDs,
and compares the authoritative digest at the same completed tick for each
world/seed. It retains the structured `PHOENIX_DEFAULT_PERTURBATION_RESULT`
records in its output. A one-worker default pool is an explicit environment
failure, not a passing multithreaded proof; no test overrides that allocation.

Under the coordinated compiler slot, run:

```sh
cargo test --features headless --test archetype_order_determinism --test schedule_order_determinism --test registration_order_determinism --test snapshot_resume default_pool_ -- --nocapture --test-threads=1
```

Expect four parent tests, nine exact-test child processes, and 23 completed App
observations. A zero-match child fails the parent. Preserve every failed child
and divergence diagnostic; do not re-bless a fixture, shorten a continuation
window, or change a scenario to make the default arm pass.

The earlier seven-child, 19-App version passed on the focused post-split
candidate, preserving the original pinned assertions and default-child
observations. That historical result predates the two added script guards;
the expanded nine-child proof requires validation on the integrated scheduler.
The separate default/default/pinned equivalence proof and fixed-executor work
cover other parts of slice 4 and its prerequisites. These guards add perturbation
and resume coverage; they do not establish the full #1400 PRD or authorize
parallelizing the production simulation.

## Cross-peer default-pool arms (#1400 stories 6–7)

Three additional parent tests reuse the same exact-child helper. The ordinary
pinned guards retain their scenarios, tick windows and assertions; only their
child's explicit environment selects the default pool. No runtime registration,
ordering edge or debt allowance changes.

- `lockstep_mesh::default_pool_keeps_two_crews_in_lockstep` runs the real
  two-host crew-command/combat guard, preserving per-tick digest agreement,
  admitted input and combat anti-vacuity checks. It reports both final Apps.
- `local_ship_neutrality::default_pool_keeps_a_stationless_gm_in_agreement`
  compares two ship hosts and a stationless GM with one frozen topology on
  every advancing tick. It reports all three final Apps.
- `lockstep_snapshot_transfer::default_pool_preserves_cross_peer_restore_continuation`
  retains shuffled chunk delivery, real snapshot reconciliation and 120 frames
  of compared continuation after the live 400-frame capture. It reports the
  original and restored Apps, requiring actual progression beyond capture.

Each parent launches exactly one existing guard in a fresh process and requires
its complete expected observation count: two, three and two respectively. The
shared helper verifies actual compute workers greater than one, MultiThreaded
FixedUpdate, actual seed, nonzero mission tick and equal final boundaries/digests.
The guards themselves compare intermediate states, so final convergence cannot
hide a transient divergence. Existing pinned invocations emit no new observations.

The focused SDK-enabled native run passed all three parents and their three
unchanged pinned companions. Its seven child reports used 16 compute workers
and MultiThreaded FixedUpdate: the two-crew mesh agreed at tick 1200, the three
GM/ship replicas at tick 1199, and the restore pair at tick 519. Build and all
six child-test invocations exited zero with source/artifact/dependency guards.
The first attempt exposed an observer-only seed expectation error: ordinary
Fleet activation uses the authored seed 1116, not the process bootstrap override.
The corrected observer checks WorldConfig independently of SimRng; the failed
attempt remains recorded. No mission assertions or production inputs changed.

This run used `--locked --offline --jobs 6 --features host,headless,perf,ultralight`
and frozen exact executables after one three-binary no-run build. The ordinary
headless configuration remains a separate final integration gate. To select
these parents in that configuration:

```sh
cargo test --locked --features headless --test lockstep_mesh --test local_ship_neutrality --test lockstep_snapshot_transfer -- default_pool_
```

Expect three passing parents, three exact child processes and seven completed
App reports; zero matching tests is not proof. Run the three unchanged pinned
original guards as the bounded companion check. This fills cross-peer execution
coverage, not every admitted ambiguity's semantic-order proof or a browser/
native cross-target guarantee. Retained scheduler debt remains separate.
