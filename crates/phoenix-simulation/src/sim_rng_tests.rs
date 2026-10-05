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

/// Recorded before delegating stream-name hashing to vellum_digest. Literal
/// draws pin the sequence independently of the current selector implementation.
#[test]
fn seed_12345_preserves_the_first_three_draws_of_every_stream() {
    let rng = SimRng::new(12345, SeedSource::Cli);
    let cases = [
        (
            SimStream::CollisionDamage,
            [782636948, 597468250, 3304661530],
        ),
        (
            SimStream::RegionDamage,
            [3041862236, 4115658456, 1734996305],
        ),
        (SimStream::BeamDamage, [2895433044, 3826621212, 840269932]),
        (
            SimStream::TorpedoDamage,
            [2335396338, 3559883791, 644951919],
        ),
        (
            SimStream::BlasterDamage,
            [1183665535, 1278718042, 1558032525],
        ),
        (
            SimStream::BeamCycleJitter,
            [2278826355, 3730868229, 373798339],
        ),
        (
            SimStream::CommsBackfillChoice,
            [2609058382, 3464922140, 4053235755],
        ),
        (SimStream::EntityUuid, [2447351754, 1062639042, 3560280982]),
    ];
    assert_eq!(cases.len(), SimStream::ALL.len());
    for (stream, expected) in cases {
        let mut generator = rng.stream(stream);
        let actual = std::array::from_fn::<_, 3, _>(|_| generator.next_u32());
        assert_eq!(actual, expected, "{stream:?}");
    }
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
/// seed. This test pins the names directly; the literal draw regression above
/// also guards their effect on each sequence.
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
        (SimStream::BeamCycleJitter, "beam-cycle-jitter"),
        (SimStream::CommsBackfillChoice, "comms-backfill-choice"),
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
        8,
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
