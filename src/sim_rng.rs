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
//!
//! # The generator
//!
//! [`vellum_rng::Pcg32`] (issue #897), not `rand`'s `SmallRng`. Two reasons,
//! neither of them "PCG is a better generator":
//!
//! 1. **It is `Serialize`.** `SmallRng` is not, so the six stream positions
//!    could not leave the process and a snapshot could only ever record the
//!    master seed — which replays a run from the start, not from where it got
//!    to. [`SimRngState`] is what a world snapshot carries (#862).
//! 2. **A generator is part of a save format.** The fleet crate exists so the
//!    byte sequence is pinned upstream and cannot move under a dependency bump
//!    that "improved a distribution"; `vellum-rng` deliberately implements no
//!    `rand` traits so nothing can substitute itself for it silently.
//!
//! `Pcg32` has no `next_u64` and no `fill_bytes` — the crate offers one
//! 32-bit draw and bounded helpers over it. Where wider values are taken, this
//! module composes them from `next_u32` explicitly and says how; that
//! composition is a contract too.
//!
//! # What this module is no longer for (issue #907)
//!
//! It used to mint entity uuids, from a dedicated stream. It does not any
//! more. Identity is not a random variable: an id drawn from a generator is a
//! function of *draw order*, so it is reproducible within one seeded instance
//! and meaningless across two. `crate::world_id` mints ids from
//! `(namespace, tick, seq)` instead. [`SimStream::EntityUuid`] survives as a
//! declared-but-unused stream — see its own docs for why retiring it would
//! cost more than it saves.

use bevy::prelude::Resource;
use serde::{Deserialize, Serialize};
use std::sync::{Mutex, MutexGuard};
use vellum_rng::Pcg32;

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
    /// **Retired, but deliberately still declared (issue #907).**
    ///
    /// This stream used to allocate entity UUIDs. Nothing draws from it any
    /// more: ids are minted from the tick-scoped counter in `crate::world_id`,
    /// because a uuid derived from a *draw* is a function of RNG draw order,
    /// so two instances that interleave draws differently mint different ids
    /// for the same spawn — which is exactly the cross-instance hazard #894's
    /// stable-world-id-order fold could not survive.
    ///
    /// It stays in the enum rather than being deleted because retiring it is
    /// not free and buys nothing. [`SimStream::ALL`] is six long and
    /// [`SimRngState`] serialises one generator per entry, with
    /// [`SimRng::from_state`] *rejecting* a snapshot whose length disagrees;
    /// dropping the variant would invalidate every recorded snapshot and shift
    /// the fingerprint's `rng_positions`, for the sake of one unused mutex. An
    /// unused stream costs a lock nobody takes and a position that never
    /// moves — and a position that never moves is itself a useful assertion:
    /// the digest folds every stream position, so a draw appearing here again
    /// would be caught the tick it happened.
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
///
/// Serialisable because it travels with [`SimRngState`]: a restored snapshot
/// should report the provenance of the seed it was *recorded* under, not the
/// provenance of the process that loaded it.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
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
    streams: Vec<Mutex<Pcg32>>,
}

/// Everything about a [`SimRng`] that can leave the process: the master seed,
/// its provenance, and every stream's *exact position*.
///
/// This is the shape a world snapshot stores (#862). The seed alone would only
/// let a run be replayed from tick zero; the stream states are what let one be
/// resumed from where it got to, which is the whole point of capturing a live
/// world. Ordering is [`SimStream::ALL`]'s, and the length is checked on the
/// way back in — a snapshot written before a stream was added must be rejected
/// rather than silently mapped onto the wrong call sites.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SimRngState {
    pub seed: u64,
    pub source: SeedSource,
    /// One generator per [`SimStream`], in [`SimStream::ALL`] order.
    pub streams: Vec<Pcg32>,
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
                .map(|s| Mutex::new(stream_generator(seed, s.name())))
                .collect(),
        }
    }

    /// Draw a master seed from the OS.
    ///
    /// The one sanctioned OS-entropy call in the crate: everything downstream
    /// derives from the seed this returns, and the seed is echoed in the exit
    /// report so the run can be replayed with `--seed`. `rand` is the entropy
    /// source because `vellum-rng` has none by design — it seeds from numbers
    /// you hand it, and where those come from is the caller's business.
    ///
    /// Issue #903: `rand::random` is banned crate-wide in `clippy.toml` so a
    /// new draw elsewhere in the sim fails the build; this is the one site
    /// the ban is scoped away from, since it IS the sanctioned OS-entropy call
    /// the module doc above claims.
    #[allow(clippy::disallowed_methods)]
    pub fn random() -> Self {
        Self::new(rand::random::<u64>(), SeedSource::Random)
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
    pub fn stream(&self, stream: SimStream) -> MutexGuard<'_, Pcg32> {
        self.streams[stream as usize]
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    // `next_uuid()` composed a v4 uuid from four `next_u32` draws off
    // `SimStream::EntityUuid`. It is gone (issue #907), and with it the whole
    // idea of deriving identity from the RNG: a draw-derived id is stable
    // within one seeded instance but is a function of *draw order*, so adding
    // any unrelated draw upstream reshuffles every subsequent entity id, and
    // two instances that interleave draws differently disagree about which
    // ship is which. `crate::world_id` mints ids from `(namespace, tick, seq)`
    // instead. The stream itself stays declared — see `SimStream::EntityUuid`.

    /// Capture the seed, its provenance, and every stream's exact position.
    ///
    /// The snapshot half of the contract with #862. Taking this does not
    /// disturb the generators — a captured world keeps running.
    ///
    /// Every guard is taken BEFORE any stream is cloned: the six locks are
    /// collected into a `Vec` first, and only then is each one cloned. A
    /// version that instead locked-and-cloned one stream at a time (as this
    /// used to) would drop each guard before taking the next, so a draw
    /// landing on another thread between two of those locks would make the
    /// resulting `SimRngState` a torn snapshot — six positions that never
    /// coexisted in any single instant of the live run. Holding all six at
    /// once rules that out.
    ///
    /// That said, holding every guard makes a capture *coherent*, not
    /// *meaningful* — it does not by itself make an arbitrary mid-run moment
    /// a sensible thing to resume from. Callers should still capture at a
    /// tick boundary (outside `SimSet`, between `FixedUpdate` steps), where
    /// "all six streams right now" corresponds to a well-defined point every
    /// system agrees on, rather than mid-tick where some systems for this
    /// frame have drawn and others have not yet run.
    pub fn state(&self) -> SimRngState {
        let guards: Vec<MutexGuard<'_, Pcg32>> =
            SimStream::ALL.iter().map(|s| self.stream(*s)).collect();
        SimRngState {
            seed: self.seed,
            source: self.source,
            streams: guards.iter().map(|g| (**g).clone()).collect(),
        }
    }

    /// Rebuild from a captured [`SimRngState`], resuming every stream at the
    /// position it was captured at.
    ///
    /// Returns `None` when the snapshot does not carry one generator per
    /// [`SimStream`]. That is the case where a save predates a stream being
    /// added, and the only safe answer is to refuse: mapping a short list onto
    /// the enum by position would hand one call site another's sequence.
    pub fn from_state(state: SimRngState) -> Option<Self> {
        if state.streams.len() != SimStream::ALL.len() {
            return None;
        }
        Some(Self {
            seed: state.seed,
            source: state.source,
            streams: state.streams.into_iter().map(Mutex::new).collect(),
        })
    }
}

/// Run `f` against `stream`, falling back to a throwaway OS-seeded generator
/// when the resource is absent.
///
/// Every simulation system takes `Option<Res<SimRng>>` rather than a bare
/// `Res`, for the same reason they take `Option<Res<LogFilterConfig>>`: a bare
/// `Res` fails Bevy parameter validation in every bare-`App` unit test in this
/// crate. Determinism plumbing must not break test fixtures.
///
/// Issue #903: the `None` arm draws OS entropy, which is why the fn carries
/// the `disallowed_methods` allow — a bare-`App` fixture that never inserted
/// `SimRng` has no run to reproduce, so this is sanctioned, not a hole.
#[allow(clippy::disallowed_methods)]
pub fn with_stream<R>(
    sim_rng: Option<&SimRng>,
    stream: SimStream,
    f: impl FnOnce(&mut Pcg32) -> R,
) -> R {
    match sim_rng {
        Some(sim) => f(&mut sim.stream(stream)),
        // Same shape as the real thing — the same stream selector, over a
        // throwaway OS-drawn master. Deliberately *not* seeded: a fixture that
        // never inserted the resource has no run to reproduce, and quietly
        // giving it a fixed seed would make every such test agree by accident.
        None => f(&mut stream_generator(rand::random::<u64>(), stream.name())),
    }
}

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

// `assign_uuid_with(Option<&SimRng>)` was the call sites' entry point to
// `next_uuid`. Its replacement is `crate::world_id::mint_id_with`, which has
// the same `Option<Res<_>>`-friendly shape for the same bare-`App` reason and
// takes an `IdNamespace` instead of nothing — because namespace membership has
// to come from the same value as the fold's sort key (issue #907).

/// The generator for one named stream of `master`.
///
/// The master seed goes in as PCG's *seed* and the name-derived constant as
/// its *stream selector*, which is a stronger form of the independence this
/// module has always claimed than the old arrangement could offer. Seeding
/// alone gives every stream the same increment, so the six were one sequence
/// entered at six points and could in principle collide onto each other;
/// selecting the stream gives each its own increment, so they are disjoint by
/// construction. `Pcg32::seeded` runs the master through SplitMix64 before it
/// becomes state, which is what stops a typed-in `--seed 1` starting from
/// almost-zero state.
fn stream_generator(master: u64, stream_name: &str) -> Pcg32 {
    Pcg32::seeded(master, stream_selector(stream_name))
}

/// The stable per-site constant a stream's identity is derived from: FNV-1a
/// over the stream name.
///
/// Derived from the *name* rather than the enum discriminant so declaration
/// order is not part of the contract — adding or reordering a variant cannot
/// move an existing stream. PCG shifts the selector left one bit to build an
/// odd increment, so the identity is really the low 63 bits of this hash; two
/// names would have to agree in all of those and differ only in the top bit to
/// collide, which the six pinned names do not.
fn stream_selector(stream_name: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in stream_name.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    fn first_draws(seed: u64) -> Vec<u32> {
        let rng = SimRng::new(seed, SeedSource::Cli);
        SimStream::ALL
            .iter()
            .map(|s| rng.stream(*s).next_u32())
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
            let expected = stream_generator(7, stream.name()).next_u32();
            assert_eq!(rng.stream(stream).next_u32(), expected, "{stream:?}");
        }
        // A name that does not exist yet stands in for a future call site.
        assert_ne!(
            stream_selector("some-future-site"),
            stream_selector(SimStream::BeamDamage.name())
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

    /// `uuids_are_seeded_valid_and_unique` used to live here and drove
    /// `next_uuid`. Identity left this module in issue #907; what replaced that
    /// test is `world_id`'s own suite plus
    /// `tests/entity_id_minting.rs::two_instances_mint_identical_entity_ids`,
    /// which asserts the stronger property the old test could not: two
    /// *separate* instances agree, not just two calls on one master seed.
    ///
    /// What this module still owes the retired stream is that it stays
    /// untouched. The digest folds every stream position, so a draw reappearing
    /// on `EntityUuid` would show up as a divergence; this asserts it locally
    /// too, where the failure names the cause instead of a tick number.
    #[test]
    fn the_retired_entity_uuid_stream_is_never_drawn_from() {
        let rng = SimRng::new(4242, SeedSource::Cli);
        let before = rng.state();
        // Exercise the streams that ARE live; the retired one must not move.
        for stream in [
            SimStream::CollisionDamage,
            SimStream::RegionDamage,
            SimStream::BeamDamage,
            SimStream::TorpedoDamage,
            SimStream::BlasterDamage,
        ] {
            with_stream(Some(&rng), stream, |g| g.next_u32());
        }
        let after = rng.state();
        let idx = SimStream::EntityUuid as usize;
        assert_eq!(
            before.streams[idx], after.streams[idx],
            "SimStream::EntityUuid is retired (issue #907) — nothing may draw from it"
        );
        assert_ne!(
            before.streams[SimStream::BeamDamage as usize],
            after.streams[SimStream::BeamDamage as usize],
            "the live streams must actually have moved, or this proves nothing"
        );
    }

    /// The property `SmallRng` could not offer and #862 is waiting on: the six
    /// stream positions survive a trip out of the process and back.
    ///
    /// Round-tripped through RON specifically, because that is the text format
    /// `vellum-save`'s stores move — a state that only round-trips in memory
    /// would prove nothing about the snapshot. Each stream is advanced a
    /// *different* number of times first, so a restore that merely re-seeded
    /// from the master (or mapped the list onto the enum by the wrong index)
    /// cannot pass by accident.
    #[test]
    fn stream_positions_survive_serialisation_and_restore() {
        let live = SimRng::new(31337, SeedSource::World);
        for (i, stream) in SimStream::ALL.iter().enumerate() {
            for _ in 0..=i {
                live.stream(*stream).next_u32();
            }
        }
        // Capture BEFORE the reference draws, so `expected` is what the live
        // generators go on to produce *from the captured positions*.
        let text = ron::to_string(&live.state()).expect("state should serialise");
        let expected: Vec<u32> = SimStream::ALL
            .iter()
            .map(|s| live.stream(*s).next_u32())
            .collect();

        let restored = SimRng::from_state(ron::from_str(&text).expect("state should parse"))
            .expect("a full-length state should restore");

        assert_eq!(restored.seed(), 31337, "the master seed round-trips");
        assert_eq!(
            restored.source(),
            SeedSource::World,
            "so does its provenance"
        );
        let continued: Vec<u32> = SimStream::ALL
            .iter()
            .map(|s| restored.stream(*s).next_u32())
            .collect();
        assert_eq!(
            continued, expected,
            "a restored run must continue every stream where it left off"
        );

        // Anti-vacuity: replaying from the seed alone lands somewhere else, so
        // the assertion above really is about the captured *positions*.
        let from_seed_only = SimRng::new(31337, SeedSource::World);
        let restarted: Vec<u32> = SimStream::ALL
            .iter()
            .map(|s| from_seed_only.stream(*s).next_u32())
            .collect();
        assert_ne!(
            restarted, expected,
            "the streams were never advanced — this test proves nothing"
        );
    }

    /// A snapshot written before a stream existed must be refused, not mapped
    /// onto the enum by position: a short list would silently hand one call
    /// site another's sequence.
    #[test]
    fn a_state_that_does_not_cover_every_stream_is_refused() {
        let mut state = SimRng::new(5, SeedSource::Cli).state();
        state.streams.pop();
        assert!(SimRng::from_state(state).is_none());
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
