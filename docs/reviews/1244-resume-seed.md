# Resume seed recovery (#1244, #1449 B2)

On 8 September 2026 the unchanged ignored reproducer passed against
`349dea207b6a27cec81b696fc84dd1b029aea730`:

```powershell
cargo test --features headless --test snapshot_resume the_bounded_duel_resumes_on_the_seed_that_still_diverges -- --ignored --exact --nocapture
```

Exit 0: **1 passed, 0 failed, 0 ignored**, 87 filtered out. Cargo rebuilt
the test and ran `target/debug/deps/snapshot_resume-81fefa07cc3eb2ae.exe`.
No current failure was available to diagnose; this does not attribute recovery
to a particular intervening fix.

The change restores seed **8,622,033** (`SEED + 7`) to the ordinary four-seed
sweep, removes the obsolete ignore, and increases the default-pool sweep's
expected observations from six to eight. The named original reproducer remains
individually runnable, including its historical name.

The existing helper captures after **400 App updates**, round-trips through
storage, restores into a fresh App, checks exact digest equality immediately
after restore and `vellum_save::verify`, then compares live and resumed digest
after **every continuation frame 1–120 inclusive**. No payload, save version,
digest, assertion or observation bound changed.

After that test-only change:

```powershell
cargo test --features headless --test snapshot_resume -- the_bounded_duel_resumes_across_several_seeds the_bounded_duel_resumes_on_the_seed_that_still_diverges default_pool_preserves_snapshot_resume_guards
```

Exit 0: **3 passed, 0 failed, 0 ignored**, 85 filtered out, 8.71 seconds.
The default-pool parent launches exact guards in fresh processes, requires more
than one compute worker and `MultiThreaded` execution, and verifies eight
live/resumed observations for the four-seed sweep. It also retains the existing
Combat Test and script-callback continuation guards.

An independent read-only review returned **PASS**, confirming restore-instant
and per-frame comparisons and the fresh-process default-pool proof. The reviewer
ran no gates. These are targeted results, not a claim that the final integrated
pre-push gate has run.
