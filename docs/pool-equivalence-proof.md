# Default-pool seed equivalence (#1400, slice 4)

`tests/pool_equivalence.rs` covers the first part of the default-pool proof.
It uses #1400 slice 3's fixed-family `SingleThreaded` executor selection.
The focused post-split run passed all three fresh-process observations.

The parent launches three fresh copies of its own integration-test executable,
sequentially: default pool once, default pool again, and an explicitly
`--deterministic` boot. This avoids Bevy's process-global task-pool inheritance.
The default roles set `HeadlessArgs.seed` programmatically after CLI parsing,
leaving `deterministic` false; passing `--seed` to the CLI would instead pin them.

All roles fly the exact `native_headless_digest.rs` inputs: Combat Test, its
first available hull, seed 20260894, and 240 updates at a manual 1/60-second
frame period. The test checks that the mission left the lobby, a player ship
spawned, and the completed `SimTick` boundary agrees before comparing the real
`world_digest`. Child records include their actual pool size and fixed executor
kinds. A machine whose ordinary compute pool has only one worker fails with
an environment explanation; it cannot provide evidence of multithreaded
execution, and the test does not alter its pool allocation to hide that fact.

For a focused rerun, use the coordinated Cargo slot:

```sh
cargo test --features headless --test pool_equivalence -- --nocapture
```

The ignored child is invoked by the parent with its explicit role. Running only
that child, or selecting zero tests, is not the three-run proof. Preserve the
three emitted `PHOENIX_POOL_PROOF_RESULT` records and the actual exit status.
No digest fixture is generated or re-blessed by this test. A mismatch is evidence
to investigate, not permission to change a baseline.

This is a bounded first part of slice 4. The `archetype_order_determinism`,
`schedule_order_determinism`, `registration_order_determinism`, and
`snapshot_resume` binaries also contain fresh-process default-pool arms, as
described in [Default-pool perturbation and resume guards](default-pool-perturbations.md).
Those arms preserve the original pinned tests and passed on the same candidate. Existing Fleet and GM digest/replay tests retain their own configured execution coverage and have
not become default-pool proofs merely because this new test exists. Neither the
native renderer comparison nor cross-target guards are replaced here.
