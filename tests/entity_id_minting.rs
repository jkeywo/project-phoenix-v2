//! Issue #907's acceptance: every simulation id is minted from a
//! `(namespace, tick, seq)` counter, and two **separate instances** on the same
//! seed and the same inputs give the same entity the same id.
//!
//! # Why this is its own test binary
//!
//! The same reason `tests/rng_determinism.rs` and `tests/replay_simulation.rs`
//! are. `--deterministic` pins the scheduler by handing `TaskPoolPlugin` a
//! one-thread `TaskPoolOptions`, but Bevy's task pools are **process-global**
//! and created by whichever app in the process builds first. Dropped into
//! `tests/headless_runner.rs`, these builds would race forty-odd other tests
//! over who fixes the pool, and a claim about spawn ORDER made under a
//! scheduler somebody else chose is not a claim about anything.
//!
//! # Why two instances and not two calls
//!
//! This is the whole point of #907, and it is the distinction the previous
//! scheme failed on. Ids used to come off `SimStream::EntityUuid`, which made
//! two *calls* on one seeded generator agree — the old
//! `sim_rng::uuids_are_seeded_valid_and_unique` asserted exactly that and
//! passed — while two *instances* that interleaved draws differently still
//! minted different ids for the same spawn. Every assertion below therefore
//! builds two independent `App`s and compares them.

#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]

use bevy::prelude::*;
use project_phoenix::entity_spawner::EntityUuid;
use project_phoenix::headless::{build_headless_app, run, world_digest, HeadlessArgs};
use project_phoenix::server_app::AsteroidUuid;
use project_phoenix::world_id::{IdNamespace, WorldId, WorldIdMint};
use std::collections::BTreeSet;

/// Ticks each instance runs. Well past the auto-start countdown, so
/// `spawn_game_start_entities` has run and the world is populated rather than
/// empty — an empty world would make every equality below vacuous, which is
/// what the spawn-count preconditions guard against.
const TICKS: u64 = 240;

/// A spawn-heavy scenario, deliberately: `combat_test.toml` puts several ships
/// in the world and keeps them shooting, so projectile ids are minted mid-run
/// as well as at start-up.
fn args(seed: u64) -> HeadlessArgs {
    HeadlessArgs {
        world_path: "assets/worlds/combat_test.toml".into(),
        ship_path: "assets/entities/alliance_destroyer.toml".into(),
        max_ticks: TICKS,
        seed: Some(seed),
        deterministic: true,
        ..Default::default()
    }
}

/// What one instance produced: its entity ids, its asteroid ids, and the digest
/// of its whole authoritative state.
struct Instance {
    entity_ids: BTreeSet<String>,
    asteroid_ids: BTreeSet<String>,
    digest: u64,
}

fn run_instance(seed: u64) -> Instance {
    let args = args(seed);
    let mut app = build_headless_app(&args).expect("app should build");
    run(&mut app, args.max_ticks);

    let entity_ids = {
        let mut q = app.world_mut().query::<&EntityUuid>();
        q.iter(app.world()).map(|u| u.0.clone()).collect()
    };
    let asteroid_ids = {
        let mut q = app.world_mut().query::<&AsteroidUuid>();
        q.iter(app.world()).map(|u| u.0.clone()).collect()
    };
    let digest = world_digest(app.world());
    Instance {
        entity_ids,
        asteroid_ids,
        digest,
    }
}

/// AC5: two runs on the same seed and the same commands mint identical ids for
/// identical entities — verified, not assumed.
#[test]
fn two_instances_mint_identical_entity_ids() {
    let a = run_instance(9_072_026);
    let b = run_instance(9_072_026);

    assert!(
        a.entity_ids.len() > 1,
        "precondition: the scenario must have spawned entities, or the equality \
         below is a comparison of two empty sets. Got {:?}",
        a.entity_ids
    );
    assert_eq!(
        a.entity_ids, b.entity_ids,
        "two instances on the same seed must give the same entity the same id"
    );
}

/// AC1: those ids are *minted*, not uuids that happen to agree.
///
/// Worth asserting separately from the equality above: two instances would also
/// agree if ids were derived from an authored name, and that scheme would break
/// the moment two entities shared one. This pins the shape the fold depends on.
#[test]
fn every_entity_id_is_a_tick_scoped_mint() {
    let a = run_instance(9_072_027);
    for id in &a.entity_ids {
        let parsed = WorldId::parse(id)
            .unwrap_or_else(|| panic!("entity id {id:?} is not a minted world id"));
        assert_eq!(
            parsed.namespace,
            IdNamespace::Entity,
            "an EntityUuid must carry the Entity namespace, not {:?} ({id})",
            parsed.namespace
        );
        assert_eq!(
            id,
            &parsed.render(),
            "the rendering must round-trip exactly"
        );
        // And it is still a real uuid, which is not decoration: `AiWorldEntity`
        // holds a `uuid::Uuid` and comms uses uuid-parseability to tell an
        // entity from a synthetic sender. See `world_id`'s module docs.
        assert_eq!(
            uuid::Uuid::parse_str(id)
                .expect("a minted id must parse as a uuid")
                .get_version_num(),
            8,
            "mints are version 8; version 4 is what an asteroid carries"
        );
    }
}

/// The scoped exception, asserted rather than left as a comment: asteroid ids
/// are NOT minted here. `deterministic_cell_uuid` derives them from the rock's
/// cell coordinates (constraint 8), which is why they are cross-instance
/// identical without being tick-scoped — a rock respawning after the player
/// leaves and returns has to come back with the id it had.
///
/// Since a mint is uuid-shaped too, this is a sharper claim than it looks: the
/// version nibble is the whole of the difference, and getting it wrong would
/// have the fold invent a tick and a sequence for a rock.
#[test]
fn asteroid_ids_stay_cell_derived_and_still_agree_across_instances() {
    let a = run_instance(9_072_028);
    let b = run_instance(9_072_028);
    assert_eq!(
        a.asteroid_ids, b.asteroid_ids,
        "cell-derived asteroid ids must agree across instances"
    );
    for id in &a.asteroid_ids {
        assert!(
            WorldId::parse(id).is_none(),
            "asteroid id {id:?} parsed as a tick-scoped mint — if asteroids have \
             been migrated, this test and the digest's fallback docs both need \
             rewriting rather than deleting"
        );
    }
}

/// The reason #907 exists at all: with ids stable across instances, the #901
/// digest is cross-instance comparable for a spawn-heavy world, not merely
/// stable within one seeded run.
///
/// This is the claim `pasm/spec/architecture/deterministic-simulation.yaml`
/// used to carry as an OPEN HAZARD.
#[test]
fn the_digest_is_comparable_across_two_instances() {
    let a = run_instance(9_072_029);
    let b = run_instance(9_072_029);
    assert_eq!(
        a.digest, b.digest,
        "two instances on the same seed must fold to the same digest"
    );

    // And the digest is not a constant that would agree with anything: a
    // different seed must move it, or the equality above proves nothing.
    let c = run_instance(9_072_030);
    assert_ne!(
        a.digest, c.digest,
        "a different seed reached the same digest — the fold is not reading the \
         run at all, so the cross-instance equality is vacuous"
    );
}

/// In-tick sequence stability: several spawns landing on one tick get
/// `seq = 0, 1, 2, …` in the order the schedule ran them, and the *next* tick
/// starts over at 0 rather than continuing.
///
/// Driven through the resource directly rather than through a scenario, because
/// what is being asserted is the counter's contract; which systems spawn in
/// which order on a given tick is #895/#896/#899's ground, and a scenario test
/// would be asserting theirs as well as this one's.
#[test]
fn a_multi_spawn_tick_numbers_its_spawns_in_order() {
    let mint = WorldIdMint::default();

    mint.begin_tick(11);
    let first: Vec<WorldId> = (0..3).map(|_| mint.mint(IdNamespace::Entity)).collect();
    assert_eq!(
        first,
        vec![
            WorldId::new(IdNamespace::Entity, 11, 0),
            WorldId::new(IdNamespace::Entity, 11, 1),
            WorldId::new(IdNamespace::Entity, 11, 2),
        ]
    );
    // The rendering is a distinct claim from the tuple, so assert it too rather
    // than only comparing tuples to tuples.
    assert_eq!(first[2].render(), "00000000-0000-8000-8000-000b00000002");

    // A namespace minted alongside them counts on its own axis, so a tick that
    // fires a torpedo does not shift the id of the ship spawned next to it.
    assert_eq!(
        mint.mint(IdNamespace::Projectile),
        WorldId::new(IdNamespace::Projectile, 11, 0)
    );

    mint.begin_tick(12);
    assert_eq!(
        mint.mint(IdNamespace::Entity),
        WorldId::new(IdNamespace::Entity, 12, 0),
        "the sequence must reset with the tick, not run monotonically"
    );
}

/// The Bevy wiring, end to end: an app built the way the runtime builds one
/// carries the mint, and the mint tracks the tick.
#[test]
fn a_built_app_mints_against_its_own_tick() {
    let args = args(9_072_031);
    let mut app = build_headless_app(&args).expect("app should build");
    run(&mut app, 30);

    let tick = app
        .world()
        .resource::<project_phoenix::sim_tick::SimTick>()
        .0;
    assert!(tick > 0, "precondition: the run must have stepped");
    assert_eq!(
        app.world().resource::<WorldIdMint>().tick(),
        tick.saturating_sub(1),
        "the mint adopts the tick in FixedFirst, so after N completed steps it \
         holds the index of the last step that ran"
    );
}
