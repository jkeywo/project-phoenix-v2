pub use phoenix_sim_contracts::sim_rng::*;

/// A throwaway OS-seeded generator, for unit tests that need *a* generator and
/// do not care which numbers come out of it.
///
/// The fixture twin of [`with_stream`]'s `None` arm, and deliberately
/// `cfg(test)`: production code must reach a generator through a named
/// [`SimStream`], and a `pub fn` handing out unseeded generators would be
/// exactly the hole that closes. A fixture that *does* care about the sequence
/// should build a `SimRng` with a literal seed instead of calling this.
#[cfg(test)]
#[allow(clippy::disallowed_methods)]
pub fn unseeded_test_rng() -> Pcg32 {
    Pcg32::seeded(rand::random::<u64>(), 0)
}

#[cfg(test)]
#[path = "sim_rng_live_stream_tests.rs"]
mod live_restore_tests;

#[cfg(test)]
use vellum_rng::Pcg32;
