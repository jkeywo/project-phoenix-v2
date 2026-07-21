//! The simulation's one source of randomness.
//!
//! Before this existed every RNG consumer built its own generator inline
//! (`SmallRng::from_os_rng()` / `rand::rng()`), so two runs of the same
//! scenario diverged the moment anything took damage. [`SimRng`] replaces
//! those call sites with a Bevy resource carrying a master seed and a fixed
//! set of *derived streams* — one per call site.
//!
//! # Why per-site streams rather than one shared generator
//!
//! A single shared generator makes every consumer's sequence depend on how
//! many draws every *other* consumer happened to make first, so adding one new
//! RNG user silently reshuffles the whole simulation. Each [`SimStream`]
//! instead seeds from the master seed combined with a per-site constant
//! derived from the stream's own name, so streams are independent: adding a
//! variant to the enum cannot perturb the sequence any existing site sees.
//! (Deriving the constant from the *name* rather than the discriminant also
//! makes the enum safe to reorder.)
//!
//! # Reproducibility contract
//!
//! Same binary, same machine, same seed. Seeded runs additionally need a fixed
//! system execution order, which is why `--seed` implies `--deterministic` —
//! with the multi-threaded executor two systems drawing from different streams
//! is fine, but two *instances* of the same system are not.
//!
//! # Interior mutability
//!
//! Streams live behind per-stream `Mutex`es so call sites can take
//! `Option<Res<SimRng>>` rather than `Option<ResMut<SimRng>>`. That matters
//! for the UUID path: `world::dispatch::DispatchContext::uuid_source` is a
//! `&dyn Fn() -> String`, which cannot close over a `&mut`. Contention is nil
//! — the streams are per-site by construction, and seeded runs are
//! single-threaded anyway.

use bevy::prelude::Resource;
use rand::rngs::SmallRng;
use rand::{RngCore, SeedableRng};
use std::sync::{Mutex, MutexGuard};

/// One independent RNG stream. One variant per call site that draws numbers.
///
/// Variants are free to be added or reordered: each stream's seed comes from
/// its [`SimStream::name`], never from its position.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum SimStream {
    /// `server_app::handle_collisions` — which console absorbs collision hull damage.
    CollisionDamage,
    /// `regions::server::apply_damage_zone_damage` — damage-zone hull distribution.
    RegionDamage,
    /// `console::weapons::beam` — phaser/beam hull distribution.
    BeamDamage,
    /// `console::weapons::torpedo` — torpedo impact hull distribution.
    TorpedoDamage,
    /// `console::weapons::blaster` — blaster impact hull distribution.
    BlasterDamage,
    /// Entity UUID allocation (`assign_uuid_with`). Keyed separately from the
    /// damage streams so a scenario that spawns more entities does not shift
    /// which console the next hit lands on.
    EntityUuid,
}

impl SimStream {
    /// Every stream, in declaration order. Used to build the resource.
    pub const ALL: [SimStream; 6] = [
        SimStream::CollisionDamage,
        SimStream::RegionDamage,
        SimStream::BeamDamage,
        SimStream::TorpedoDamage,
        SimStream::BlasterDamage,
        SimStream::EntityUuid,
    ];

    /// The stable per-site constant this stream derives its seed from.
    ///
    /// Renaming a variant here re-seeds that stream (and only that stream), so
    /// treat these strings as part of the reproducibility contract.
    pub const fn name(self) -> &'static str {
        match self {
            SimStream::CollisionDamage => "collision-damage",
            SimStream::RegionDamage => "region-damage",
            SimStream::BeamDamage => "beam-damage",
            SimStream::TorpedoDamage => "torpedo-damage",
            SimStream::BlasterDamage => "blaster-damage",
            SimStream::EntityUuid => "entity-uuid",
        }
    }
}

/// Where the resolved master seed came from. Reported so a run is always
/// replayable and the provenance is obvious.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SeedSource {
    /// `--seed <N>` on the command line.
    Cli,
    /// `[global] seed` in the world TOML.
    World,
    /// Drawn from the OS because neither of the above supplied one.
    Random,
}

impl SeedSource {
    /// Lowercase tag used in the exit report.
    pub const fn as_str(self) -> &'static str {
        match self {
            SeedSource::Cli => "cli",
            SeedSource::World => "world",
            SeedSource::Random => "random",
        }
    }
}

/// The sim-wide seeded RNG.
#[derive(Resource, Debug)]
pub struct SimRng {
    seed: u64,
    source: SeedSource,
    /// Indexed by `SimStream as usize`, built from [`SimStream::ALL`].
    streams: Vec<Mutex<SmallRng>>,
}

impl Default for SimRng {
    /// An OS-seeded instance, so an app that never configures a seed still
    /// behaves the way it did before this resource existed.
    fn default() -> Self {
        Self::random()
    }
}

impl SimRng {
    /// Build from an explicit master seed.
    pub fn new(seed: u64, source: SeedSource) -> Self {
        Self {
            seed,
            source,
            streams: SimStream::ALL
                .iter()
                .map(|s| Mutex::new(SmallRng::seed_from_u64(derive_stream_seed(seed, s.name()))))
                .collect(),
        }
    }

    /// Draw a master seed from the OS.
    ///
    /// The one sanctioned OS-entropy call in the crate: everything downstream
    /// derives from the seed this returns, and the seed is echoed in the exit
    /// report so the run can be replayed with `--seed`.
    pub fn random() -> Self {
        let mut os = SmallRng::from_os_rng();
        Self::new(os.next_u64(), SeedSource::Random)
    }

    /// The master seed. Always reported, seeded or not.
    pub fn seed(&self) -> u64 {
        self.seed
    }

    pub fn source(&self) -> SeedSource {
        self.source
    }

    /// Borrow one stream's generator.
    ///
    /// Poisoning is recovered from rather than propagated: a panic elsewhere
    /// in the frame has already failed the run, and turning that into a second
    /// panic inside the damage path buries the original.
    pub fn stream(&self, stream: SimStream) -> MutexGuard<'_, SmallRng> {
        self.streams[stream as usize]
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// A fresh v4-formatted UUID drawn from [`SimStream::EntityUuid`].
    ///
    /// Byte-identical reports depend on this: `damage_by_ship` and
    /// `entity_names` are both keyed by entity uuid, so random uuids alone
    /// would defeat every other RNG site being seeded.
    pub fn next_uuid(&self) -> String {
        let mut bytes = [0u8; 16];
        self.stream(SimStream::EntityUuid).fill_bytes(&mut bytes);
        uuid::Builder::from_random_bytes(bytes)
            .into_uuid()
            .to_string()
    }
}

/// Run `f` against `stream`, falling back to a throwaway OS-seeded generator
/// when the resource is absent.
///
/// Every simulation system takes `Option<Res<SimRng>>` rather than a bare
/// `Res`, for the same reason they take `Option<Res<LogFilterConfig>>`: a bare
/// `Res` fails Bevy parameter validation in every bare-`App` unit test in this
/// crate. Determinism plumbing must not break test fixtures.
pub fn with_stream<R>(
    sim_rng: Option<&SimRng>,
    stream: SimStream,
    f: impl FnOnce(&mut SmallRng) -> R,
) -> R {
    match sim_rng {
        Some(sim) => f(&mut sim.stream(stream)),
        None => f(&mut SmallRng::from_os_rng()),
    }
}

/// A fresh entity UUID from the seeded stream, or a random one when the
/// resource is absent. The seeded twin of `entity_loader::assign_uuid`.
pub fn assign_uuid_with(sim_rng: Option<&SimRng>) -> String {
    match sim_rng {
        Some(sim) => sim.next_uuid(),
        None => crate::entity_loader::assign_uuid(),
    }
}

/// Combine the master seed with a per-site constant.
///
/// FNV-1a over the stream name, mixed into the seed with SplitMix64's
/// finaliser so neighbouring names (which differ in one byte) land far apart
/// in seed space.
fn derive_stream_seed(master: u64, stream_name: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in stream_name.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    splitmix64(master.wrapping_add(hash))
}

fn splitmix64(x: u64) -> u64 {
    let mut z = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn first_draws(seed: u64) -> Vec<u64> {
        let rng = SimRng::new(seed, SeedSource::Cli);
        SimStream::ALL
            .iter()
            .map(|s| rng.stream(*s).next_u64())
            .collect()
    }

    #[test]
    fn the_same_seed_reproduces_every_stream() {
        assert_eq!(first_draws(12345), first_draws(12345));
    }

    #[test]
    fn different_seeds_diverge() {
        assert_ne!(first_draws(1), first_draws(2));
    }

    /// Streams must be independent, not offsets into one sequence — otherwise
    /// adding an RNG consumer reshuffles unrelated parts of the simulation.
    #[test]
    fn streams_are_independent_of_one_another() {
        let draws = first_draws(99);
        let unique: std::collections::HashSet<_> = draws.iter().collect();
        assert_eq!(unique.len(), draws.len(), "streams share a sequence");
    }

    /// The regression guard for the "adding a stream must not shift the
    /// others" property: a stream's seed is a function of its *name* and the
    /// master seed alone, so a hypothetical new variant — wherever it is
    /// declared — cannot move an existing one.
    #[test]
    fn a_streams_seed_depends_only_on_its_name_and_the_master_seed() {
        let rng = SimRng::new(7, SeedSource::Cli);
        for stream in SimStream::ALL {
            // Rebuilt from the name alone — no reference to the variant's
            // position, so declaring a new variant anywhere cannot move it.
            let expected = SmallRng::seed_from_u64(derive_stream_seed(7, stream.name())).next_u64();
            assert_eq!(rng.stream(stream).next_u64(), expected, "{stream:?}");
        }
        // A name that does not exist yet stands in for a future call site.
        assert_ne!(
            derive_stream_seed(7, "some-future-site"),
            derive_stream_seed(7, SimStream::BeamDamage.name())
        );
    }

    /// The name strings are the reproducibility contract, pinned literally.
    ///
    /// A stream's seed is derived from its [`SimStream::name`], so renaming one
    /// re-seeds it: every seed anyone ever recorded stops reproducing the run
    /// it was recorded from, silently, with the report still claiming that
    /// seed. Nothing else in the build notices — a rename compiles, and every
    /// other test here is written against `name()` rather than against a
    /// literal, so it would pass too.
    ///
    /// If you are here because this test failed: changing a name is allowed,
    /// but it is a deliberate act that invalidates recorded seeds. Update the
    /// literal below only when that is what you mean to do.
    #[test]
    fn stream_names_are_pinned_to_their_recorded_strings() {
        for (stream, expected) in [
            (SimStream::CollisionDamage, "collision-damage"),
            (SimStream::RegionDamage, "region-damage"),
            (SimStream::BeamDamage, "beam-damage"),
            (SimStream::TorpedoDamage, "torpedo-damage"),
            (SimStream::BlasterDamage, "blaster-damage"),
            (SimStream::EntityUuid, "entity-uuid"),
        ] {
            assert_eq!(
                stream.name(),
                expected,
                "{stream:?} was renamed — that re-seeds it and invalidates every recorded seed"
            );
        }
        // Anti-vacuity: the list above has to cover the enum, or a newly added
        // variant would go unpinned and be free to be renamed later.
        assert_eq!(
            SimStream::ALL.len(),
            6,
            "a stream was added — pin its name above too"
        );
    }

    #[test]
    fn uuids_are_seeded_valid_and_unique() {
        let a = SimRng::new(4242, SeedSource::Cli);
        let b = SimRng::new(4242, SeedSource::World);
        let first: Vec<String> = (0..4).map(|_| a.next_uuid()).collect();
        let second: Vec<String> = (0..4).map(|_| b.next_uuid()).collect();
        assert_eq!(first, second, "uuid stream must follow the master seed");
        assert_ne!(
            first[0], first[1],
            "uuids must still be unique within a run"
        );
        let parsed = uuid::Uuid::parse_str(&first[0]).expect("valid uuid");
        assert_eq!(parsed.get_version_num(), 4, "v4 formatting is preserved");
    }

    #[test]
    fn seed_and_provenance_are_reported_back() {
        let rng = SimRng::new(88, SeedSource::World);
        assert_eq!(rng.seed(), 88);
        assert_eq!(rng.source().as_str(), "world");
        assert_eq!(SeedSource::Cli.as_str(), "cli");
        assert_eq!(SeedSource::Random.as_str(), "random");
    }
}
