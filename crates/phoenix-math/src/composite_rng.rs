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
#[path = "composite_rng_tests.rs"]
mod tests;
