//! The standing regression guard for ARCHETYPE-creation order (issue #1052,
//! the second half of #1051's root-cause investigation).
//!
//! # What this proves
//!
//! Bevy allocates archetype ids in creation order, and every query iterates
//! its matched archetypes in that order. So the order a query hands entities
//! back is an artefact of how the world happened to spawn, insert and despawn
//! — not of anything the simulation authored. Any system that draws from
//! [`SimRng`](project_phoenix::sim_rng::SimRng) once per entity **in that
//! order** therefore deals its draws out differently the moment an unrelated
//! component insert creates one extra archetype, and the authoritative digest
//! moves with it.
//!
//! That is not hypothetical. It is issue #1051, twice over:
//!
//! * #984's S7 resolver materialised `HumanSeekingHosts` mid-run and moved
//!   `duel` and `rng_coverage` (fixed in 66c3c1bd);
//! * the debug-only `HelmPhysicsWriteGuard` did the same thing behind
//!   `#[cfg(debug_assertions)]`, so DEBUG and RELEASE builds of the same
//!   commit disagreed on those two worlds (fixed in the #1051 commit).
//!
//! Both were found by bisection, months apart, because nothing checked the
//! property. This file checks the property: it runs the same seeded world
//! twice in one process, once normally and once with a deliberate mid-run
//! archetype move, and asserts the digest does not care.
//!
//! # What it does NOT claim
//!
//! It does not claim archetype order is *stable* — the perturbation genuinely
//! changes it, and [`the_perturbation_really_moves_the_archetype_layout`]
//! asserts that it does, so the comparison can never pass vacuously. The claim
//! is that the digest is INDEPENDENT of it.
//!
//! # The red state, verified by hand (issue #1052)
//!
//! With the #1052 sorts stashed out of `src/` and this file kept,
//! `the_digest_does_not_move_when_an_archetype_is_added` FAILS: clean
//! `0xa02f4cd4b97612bb`, perturbed `0xdb14eb7f767e51fe`. With the sorts in
//! place both runs are `0xa02f4cd4b97612bb` — the same number the clean run
//! produced without them, so the sorts moved nothing and only made the
//! perturbed run agree. Those digests are evidence of the red state, not a
//! contract: they move with the world's content like any other, and nothing
//! here asserts them.
//!
//! `rng_coverage.toml` was tried first and is NOT the world to guard with.
//! Under this same perturbation it stays green with and without the sorts,
//! because only one ship is ever inside its damage zone at a time — the region
//! site has no two victims to deal draws between, so the order it deals them
//! in cannot show. A guard that passes for that reason proves nothing.
//!
//! # Why this is its own test binary
//!
//! Same reason as `tests/rng_determinism.rs` and
//! `tests/registration_order_determinism.rs`: `--deterministic` (which
//! `HeadlessArgs::deterministic` selects) pins Bevy's `TaskPoolPlugin` to a
//! single thread, but task pools are process-global and initialised by
//! whichever app builds first. Sharing a binary with other headless tests
//! means inheriting a multi-threaded pool a neighbour already created, which
//! is precisely the kind of nondeterminism this guard exists to rule out.
//! Cargo gives every integration-test file its own process. Do not add
//! unrelated tests here.

#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]

use bevy::prelude::*;
use project_phoenix::entity_spawner::EntityUuid;
use project_phoenix::headless::{build_headless_app, run, HeadlessArgs};
use project_phoenix::server_app::Ship;
use project_phoenix::sim_digest::world_digest;
use project_phoenix::sim_sets::SimSet;
use project_phoenix::sim_tick::SimTick;

/// `duel.toml`: ten ships across two hostile sides, beams, blasters and
/// torpedoes all landing on several different victims in the same tick.
///
/// That last property is what makes it the right world for this guard and
/// `rng_coverage` the wrong one: an ordering bug in a per-victim draw can only
/// show when there are two victims to deal draws BETWEEN. duel is also the
/// world #1051 measured as archetype-order sensitive — both the S7 insert and
/// the debug-guard insert moved its digest — and, per the red state recorded
/// above, it still is under a raw-order damage site.
const WORLD: &str = "assets/worlds/duel.toml";

/// The window #1051's whole evidence record was taken over, so a failure here
/// is directly comparable to the numbers on that issue. Long enough that both
/// sides have closed, fired and started losing systems; short enough that the
/// guard costs a handful of seconds.
const TICKS: u64 = 900;

/// The seed #1051's whole evidence record was taken under, kept so a failure
/// here can be reproduced with the same `phoenix-headless --seed 42` command
/// the issue quotes.
const SEED: u64 = 42;

/// Late enough that the world has settled into its spawn-time archetypes, so
/// the insert is unambiguously a MID-RUN move rather than part of the spawn
/// burst — which is the whole distinction 66c3c1bd's `#[require]` fix turns on.
const PERTURB_AT_TICK: u64 = 30;

/// A component the simulation knows nothing about, inserted mid-run purely to
/// force an archetype move.
///
/// Zero-sized on purpose: it carries no state, is folded into no digest, and
/// changes no system's behaviour. The ONLY thing it can do is create a new
/// archetype — which is exactly the variable under test. #984's bisection used
/// the same trick (an unrelated ZST insert reproduced the S7 digests byte for
/// byte), and this is that throwaway probe made permanent.
#[derive(Component)]
struct ArchetypeOrderProbe;

/// Every ship that has not been probed yet, with its world id.
///
/// A named alias rather than the inline query type, because clippy's
/// `type_complexity` lint (this crate runs it as `-D warnings`) counts the
/// nested filter tuple against the parameter.
type UnprobedShips<'w, 's> = Query<
    'w,
    's,
    (Entity, Option<&'static EntityUuid>),
    (With<Ship>, Without<ArchetypeOrderProbe>),
>;

/// Insert [`ArchetypeOrderProbe`] on every ship once, at [`PERTURB_AT_TICK`],
/// in DESCENDING world-id order.
///
/// The descending order is the whole point, and it is what makes this a
/// permutation rather than an append. Bevy applies a `Commands` queue in the
/// order it was filled, creating one archetype per new component set as it
/// goes — so inserting on the ships from the back forwards re-creates the
/// hull groups' archetypes in reverse, and every query that matches them now
/// hands them back in that reversed order. That is the same shape as
/// `RegistrationOrder::Shuffled` in `registration_order_determinism.rs`: a
/// deliberate, deterministic permutation of an order nothing is allowed to
/// depend on.
///
/// A single insert on one ship — #984's original lever — only appends an
/// archetype at the end, which is a weaker perturbation: it moves one ship
/// relative to the rest rather than reversing the group. Both are the same
/// hazard; this one is the version a guard should hold the code to.
///
/// `Without<ArchetypeOrderProbe>` makes it idempotent, so the moves happen on
/// exactly one tick and every later tick is an ordinary tick in a world whose
/// archetype ids happen to be laid out differently.
fn perturb_archetype_order(tick: Res<SimTick>, ships: UnprobedShips, mut commands: Commands) {
    if tick.0 < PERTURB_AT_TICK {
        return;
    }
    let mut order: Vec<(String, Entity)> = ships
        .iter()
        .map(|(entity, uuid)| (uuid.map(|u| u.0.clone()).unwrap_or_default(), entity))
        .collect();
    order.sort();
    for (_, entity) in order.into_iter().rev() {
        commands.entity(entity).insert(ArchetypeOrderProbe);
    }
}

fn args() -> HeadlessArgs {
    HeadlessArgs {
        world_path: WORLD.into(),
        max_ticks: TICKS,
        seed: Some(SEED),
        deterministic: true,
        ..Default::default()
    }
}

/// Run the world and hand back `(final digest, archetype count)`.
///
/// The archetype count travels with the digest so the vacuity check below has
/// something to assert on without running the world a third time.
fn run_world(perturbed: bool) -> (u64, usize) {
    let args = args();
    let mut app = build_headless_app(&args).expect("app should build");
    if perturbed {
        // `SimSet::Input` is the head of the chain, so the insert lands before
        // anything that damages, moves or publishes this tick — the same place
        // in the step the two real regressions inserted from.
        app.add_systems(FixedUpdate, perturb_archetype_order.in_set(SimSet::Input));
    }
    run(&mut app, args.max_ticks);
    let digest = world_digest(app.world());
    let archetypes = app.world().archetypes().len();
    (digest, archetypes)
}

/// Issue #1052's headline acceptance criterion: the authoritative digest must
/// not move when an unrelated mid-run component insert re-orders the
/// archetypes underneath every query.
#[test]
fn the_digest_does_not_move_when_an_archetype_is_added() {
    let (clean, _) = run_world(false);
    let (perturbed, _) = run_world(true);

    assert_eq!(
        clean, perturbed,
        "the authoritative digest moved when a zero-sized, simulation-inert \
         component was inserted mid-run. The insert changes nothing but \
         ARCHETYPE CREATION ORDER, so some system is dealing per-entity RNG \
         draws (or ordering a state write) in raw query order. Fix the site by \
         collecting and sorting on a stable key before the walk — the pattern \
         `server_app::handle_collisions` has used since #896 and the four \
         damage sites adopted in #1052 — rather than by re-blessing this \
         number. See issue #1051 for what this costs when it is left alone: \
         debug and release builds of the same commit disagreeing on duel and \
         rng_coverage."
    );
}

/// The vacuity guard: the perturbation has to actually perturb something, or
/// the assertion above passes for the wrong reason.
///
/// A ZST insert on a live entity always creates one archetype that the clean
/// run never had (the player ship's own set, plus the probe). If this ever
/// stops holding — the marker being optimised away, the world losing its
/// `LocalShip`, the world ending before [`PERTURB_AT_TICK`] — the headline
/// test above would be comparing two identical runs and would say nothing.
#[test]
fn the_perturbation_really_moves_the_archetype_layout() {
    let (_, clean) = run_world(false);
    let (_, perturbed) = run_world(true);

    assert!(
        perturbed > clean,
        "the probe insert created no new archetype ({clean} clean vs \
         {perturbed} perturbed), so `the_digest_does_not_move_when_an_\
         archetype_is_added` is comparing two identical runs and proves \
         nothing. Check that {WORLD} still spawns a LocalShip and still runs \
         past tick {PERTURB_AT_TICK}."
    );
}
