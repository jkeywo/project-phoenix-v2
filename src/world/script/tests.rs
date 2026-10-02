use super::*;

#[test]
fn init_hashing_seed_is_idempotent() {
    // Calling it repeatedly must not panic (the `Once` guard swallows the
    // second `set_hashing_seed`, which would otherwise return `Err`).
    init_hashing_seed();
    init_hashing_seed();
    init_hashing_seed();
}

#[test]
fn budgets_are_ordered_safety_limits() {
    // Sanity on the fixed constants: a per-tick aggregate must admit more
    // than a single per-call runaway, and the call cap is positive.
    const { assert!(MAX_OPS_PER_TICK > MAX_OPS_PER_CALL) };
    const { assert!(MAX_CALLS_PER_TICK > 0) };
}
