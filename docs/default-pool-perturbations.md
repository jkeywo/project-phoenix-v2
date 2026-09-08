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
| Snapshot resume | Duel seeds 8622026/8622027/8622057 with 400 updates before capture and 120 continuation frames; Combat Test seed 8622026 with its existing streamed-belt round-trip and zero-frame continuation bound | 8 |

The snapshot cases retain ordinary FileStore save/load, complete reconstruction,
exact captured digest, `vellum_save::verify`, and every continuation-frame
comparison. Combat Test's documented low-LOD continuation gap and the existing
ignored divergent duel seed are unchanged; this addition does not claim those
gaps are solved.

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

Expect four parent tests, seven exact-test child processes, and 19 completed App
observations. A zero-match child fails the parent. Preserve every failed child
and divergence diagnostic; do not re-bless a fixture, shorten a continuation
window, or change a scenario to make the default arm pass.

These guards passed on the focused post-split candidate, preserving the original
pinned assertions and the default-child observations.
The separate default/default/pinned equivalence proof and fixed-executor work
cover other parts of slice 4 and its prerequisites. These guards add perturbation
and resume coverage; they do not establish the full #1400 PRD or authorize
parallelizing the production simulation.
