//! Composite-key deterministic value derivation (issue #788).
//!
//! A pure, Bevy-free, domain-neutral answer to one question: *given several
//! independent identifiers, produce a stable pseudo-random value that is a
//! function of all of them, in order.*
//!
//! # Why this is not `crate::sim_rng`
//!
//! `crate::sim_rng::SimRng` (in the root crate) is a Bevy `Resource` carrying a master
//! seed and a fixed set of *per-call-site* streams. It answers "give me the next
//! number for this call site", which is the right shape for damage rolls and
//! uuid allocation and the wrong shape here: a recovery manoeuvre needs the
//! *same* answer every time the same (world, ship, system, transition,
//! occurrence) tuple comes round, with no sequence state at all. Nothing here
//! is stateful, nothing here is a resource, and nothing here knows what a ship
//! is — the keys are plain `u64`s and the caller decides what they mean.
//!
//! The *mixing idiom* is borrowed deliberately from `sim_rng`'s per-stream
//! derivation: FNV-1a folding into SplitMix64's finaliser, so neighbouring keys
//! land far apart in seed space. (Since #897 that module folds its FNV-1a hash
//! into `vellum_rng::Pcg32`'s stream selector rather than into a seed, but the
//! reason for the idiom is unchanged, and this module stays standalone.)
//!
//! # Why the fold is order-sensitive
//!
//! The obvious composite — `a ^ b ^ c` — collides trivially: it is commutative,
//! so `(1, 2)` and `(2, 1)` produce the same seed, and any pair of keys that
//! swap values between two fields is indistinguishable. Worse, XOR of two equal
//! keys cancels to zero. [`composite_seed`] instead folds each field through a
//! non-commutative FNV-1a byte pass and re-finalises after every field, so the
//! *position* of a key is part of its contribution.
//!
//! # Reproducibility contract
//!
//! The concrete values this module produces are pinned by fixture tests. They
//! are a contract: changing the constants or the fold order re-rolls every
//! decision ever derived from a recorded seed. Do that only deliberately.

/// The composite key a value is derived from.
///
/// Five named `u64` fields rather than a slice, because the *count* and the
/// *order* are the contract: adding a field, or passing them in a different
/// order, changes every derived value. Naming them makes a call site that gets
/// the order wrong a readable mistake rather than an invisible one.
///
/// The field names describe the *role* a caller is expected to fill, not a type
/// this module knows about: `world` is whatever identifies the run, `ship` the
/// actor, `system` the actor's subsystem, `transition` the event, `occurrence`
/// a monotonically increasing count of how many times that event has happened.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct CompositeKey {
    pub world: u64,
    pub ship: u64,
    pub system: u64,
    pub transition: u64,
    pub occurrence: u64,
}

impl CompositeKey {
    /// The five fields in their contractual order.
    fn fields(&self) -> [u64; 5] {
        [
            self.world,
            self.ship,
            self.system,
            self.transition,
            self.occurrence,
        ]
    }
}

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// SplitMix64's finaliser — the same avalanche `sim_rng` uses.
fn splitmix64(x: u64) -> u64 {
    let mut z = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// Fold one `u64` into the accumulator, FNV-1a over its little-endian bytes,
/// then re-finalise. Both halves matter: FNV-1a is non-commutative (so field
/// order survives), and the finaliser stops adjacent field values from
/// producing adjacent seeds.
fn fold(acc: u64, value: u64) -> u64 {
    let mut h = acc;
    for byte in value.to_le_bytes() {
        h ^= byte as u64;
        h = h.wrapping_mul(FNV_PRIME);
    }
    splitmix64(h)
}

/// Derive the seed for a composite key.
///
/// Deterministic, total, and dependent on every field *and* its position.
pub fn composite_seed(key: &CompositeKey) -> u64 {
    let mut h = FNV_OFFSET;
    for field in key.fields() {
        h = fold(h, field);
    }
    h
}

/// A stable `u64` for a textual identifier (a system name, a state id).
///
/// Provided so callers do not each invent their own string→`u64` mapping and
/// silently disagree. A `&str` is not a domain type: this module still knows
/// nothing about what the name refers to.
pub fn key_from_name(name: &str) -> u64 {
    let mut h = FNV_OFFSET;
    for byte in name.as_bytes() {
        h ^= *byte as u64;
        h = h.wrapping_mul(FNV_PRIME);
    }
    splitmix64(h)
}

/// The derived value as a fraction in `[0, 1)`.
///
/// Uses the top 53 bits — exactly an `f64` mantissa — so the mapping is exact
/// and never rounds to 1.0.
pub fn unit_interval(key: &CompositeKey) -> f64 {
    (composite_seed(key) >> 11) as f64 / (1u64 << 53) as f64
}

/// The derived value as a two-way choice: `+1.0` or `-1.0`.
///
/// Reads the *high* bit rather than the low one: the low bits of a SplitMix64
/// finaliser output are fine in practice, but the high bits are where its
/// avalanche is strongest, and a one-bit decision has no margin to spare.
pub fn signed_choice(key: &CompositeKey) -> f64 {
    if composite_seed(key) >> 63 == 0 {
        1.0
    } else {
        -1.0
    }
}

/// The derived value as an index in `[0, len)`. `len == 0` yields `0`.
pub fn bounded_index(key: &CompositeKey, len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    (composite_seed(key) % len as u64) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(world: u64, ship: u64, system: u64, transition: u64, occurrence: u64) -> CompositeKey {
        CompositeKey {
            world,
            ship,
            system,
            transition,
            occurrence,
        }
    }

    /// THE fixture. These literals are the reproducibility contract: every
    /// recovery orbit direction anyone ever recorded is a function of them.
    ///
    /// If you are here because this failed: changing the fold or its constants
    /// is allowed, but it re-rolls every decision derived from a recorded seed.
    /// Update the literals below only when that is what you mean to do. Pinning
    /// concrete outputs (rather than only properties) is what stops the mapping
    /// drifting silently under a refactor that still satisfies every
    /// property test in this module.
    #[test]
    fn composite_seed_is_pinned_to_its_recorded_values() {
        assert_eq!(
            composite_seed(&key(0, 0, 0, 0, 0)),
            13_569_046_481_838_298_424
        );
        assert_eq!(
            composite_seed(&key(1, 2, 3, 4, 5)),
            6_360_597_862_457_118_559
        );
        assert_eq!(
            composite_seed(&key(u64::MAX, 0, 1, 0, 1)),
            1_002_478_115_543_807_285
        );
        assert_eq!(key_from_name("helm-steering"), 3_836_562_346_148_525_846);
        assert_eq!(key_from_name(""), 14_087_677_454_934_409_008);
    }

    #[test]
    fn the_same_key_always_derives_the_same_value() {
        let k = key(7, 11, 13, 17, 19);
        assert_eq!(composite_seed(&k), composite_seed(&k));
        assert_eq!(signed_choice(&k), signed_choice(&k));
        assert_eq!(unit_interval(&k), unit_interval(&k));
    }

    /// Every field is load-bearing: change one and the value moves.
    #[test]
    fn every_field_participates_in_the_seed() {
        let base = key(1, 1, 1, 1, 1);
        let seed = composite_seed(&base);
        for mutated in [
            key(2, 1, 1, 1, 1),
            key(1, 2, 1, 1, 1),
            key(1, 1, 2, 1, 1),
            key(1, 1, 1, 2, 1),
            key(1, 1, 1, 1, 2),
        ] {
            assert_ne!(
                composite_seed(&mutated),
                seed,
                "{mutated:?} must not collide with the base key"
            );
        }
    }

    /// The naive `a ^ b ^ c` composite this replaces is commutative, so
    /// reordering the fields (or swapping two of them) collides. This fold must
    /// not: the *position* of a key is part of its contribution.
    #[test]
    fn the_fold_is_order_sensitive_where_xor_would_collide() {
        // Full reversal.
        assert_ne!(
            composite_seed(&key(1, 2, 3, 4, 5)),
            composite_seed(&key(5, 4, 3, 2, 1))
        );
        // A single adjacent swap — the case an XOR fold cannot see at all.
        assert_ne!(
            composite_seed(&key(1, 2, 3, 4, 5)),
            composite_seed(&key(2, 1, 3, 4, 5))
        );
        // Two equal keys in different fields: XOR would cancel them to the
        // same value regardless of where they sat.
        assert_ne!(
            composite_seed(&key(9, 9, 0, 0, 0)),
            composite_seed(&key(0, 0, 9, 9, 0))
        );
        // ...and a pair that XOR would cancel to zero entirely.
        assert_ne!(
            composite_seed(&key(9, 9, 0, 0, 0)),
            composite_seed(&key(0, 0, 0, 0, 0))
        );
    }

    /// Neighbouring occurrences must land far apart, or a counter that ticks
    /// 0, 1, 2, 3 would produce a visibly periodic sequence of choices.
    #[test]
    fn consecutive_occurrences_do_not_alternate_predictably() {
        let choices: Vec<f64> = (0..16)
            .map(|n| signed_choice(&key(42, 7, 3, 5, n)))
            .collect();
        let alternating: Vec<f64> = (0..16)
            .map(|n| if n % 2 == 0 { 1.0 } else { -1.0 })
            .collect();
        assert_ne!(
            choices, alternating,
            "the choice sequence must not be a period-2 alternation"
        );
        // Both outcomes must actually occur across a short run.
        assert!(choices.contains(&1.0) && choices.contains(&-1.0));
    }

    #[test]
    fn unit_interval_stays_in_range_and_spreads() {
        let mut min = f64::MAX;
        let mut max = f64::MIN;
        for n in 0..256 {
            let v = unit_interval(&key(3, 4, 5, 6, n));
            assert!((0.0..1.0).contains(&v), "{v} out of range");
            min = min.min(v);
            max = max.max(v);
        }
        assert!(
            min < 0.1 && max > 0.9,
            "expected spread, got [{min}, {max}]"
        );
    }

    #[test]
    fn signed_choice_is_only_ever_plus_or_minus_one() {
        for n in 0..64 {
            let v = signed_choice(&key(n, n * 3, n * 7, n * 11, n * 13));
            assert!(v == 1.0 || v == -1.0, "got {v}");
        }
    }

    #[test]
    fn bounded_index_stays_inside_its_range_and_tolerates_zero() {
        for n in 0..64 {
            assert!(bounded_index(&key(1, 2, 3, 4, n), 5) < 5);
        }
        assert_eq!(bounded_index(&key(1, 2, 3, 4, 5), 0), 0);
    }

    #[test]
    fn distinct_names_derive_distinct_keys() {
        assert_ne!(key_from_name("helm-steering"), key_from_name("helm-thrust"));
        // Names differing in one byte must not land adjacent.
        let a = key_from_name("recover");
        let b = key_from_name("recoves");
        assert!(a.abs_diff(b) > 1_000_000, "{a} and {b} are too close");
    }
}
