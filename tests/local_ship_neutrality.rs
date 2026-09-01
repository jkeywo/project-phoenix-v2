//! `LocalShip` may not reach the authoritative digest (issue #1116).
//!
//! # The hazard, stated plainly
//!
//! `LocalShip` marks "the ship whose crew is on this machine". With one host
//! that is a fixed fact about the world; with two hosts running one mission it
//! is **a different ship on each of them**. So anything the marker gates is, by
//! construction, a value the two hosts compute differently — and if that value
//! is folded into `sim_digest::world_digest`, they disagree from tick zero and
//! there is no mission to play.
//!
//! This file runs the same seeded world twice in one process, changing exactly
//! one thing between the runs — which fleet ship carries the marker — and
//! compares the fold on **every tick**. Nothing else differs: same world, same
//! seed, same scheduler, no crew, no commands, no mesh.
//!
//! # What it found when it was written
//!
//! Two sites, both fixed rather than blessed:
//!
//! * **Visual banking.** `integrate_ship_physics` lerped `ShipPhysics.roll`
//!   for the `LocalShip` alone, and `sim_digest::fold_physics` folds all eight
//!   `ShipPhysics` fields. Two hosts therefore disagreed about the fleet's roll
//!   from the first tick either crew steered. Banking now runs for every ship.
//! * **The human-seeking and detail-floor resolvers.** Both ran `With<LocalShip>`
//!   and answered from the host's own `Sessions`, so a peer's crewed hull was
//!   resolved as uncrewed here and crewed there — one host's AI operating a
//!   console another host's human was sitting at. Both now run for every ship in
//!   the fleet, answering from live `Sessions` for this host's own hull and from
//!   the frozen roster for a peer's.
//!
//! # Why it is not the archetype guard
//!
//! `LocalShip`'s other hazard is structural: a zero-sized marker on a different
//! entity groups the ships into archetypes differently, and Bevy allocates
//! archetype ids in creation order. That the digest does not care is
//! `tests/archetype_order_determinism.rs`'s claim (issue #1052), proved under a
//! deliberately stronger perturbation — the hull groups' archetypes recreated in
//! reverse. This file is about the marker's *semantics*, and the two guards are
//! complementary: #1052 says the fold is order-independent, this says the run
//! that produces it is locality-independent.
//!
//! # Why this is its own test binary
//!
//! The same reason `tests/archetype_order_determinism.rs` and
//! `tests/entity_id_minting.rs` are: `--deterministic` pins the scheduler with a
//! one-thread `TaskPoolOptions`, and Bevy's task pools are process-global,
//! created by whichever app in the process builds first. A determinism claim
//! made in a binary where forty other tests build apps is a claim about whoever
//! won that race. Do not add unrelated tests here.

#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]

use project_phoenix::command_admission::HostSlot;
use project_phoenix::headless::{build_headless_app, run, world_digest, HeadlessArgs};
use project_phoenix::lockstep::{FleetRoster, FleetShip, FleetSlotOf};
use project_phoenix::server_app::LocalShip;
use project_phoenix::sim_tick::SimTick;

/// Two crewed hulls and a hostile to shoot at — see the world's own header for
/// why a probe with combat in it is the one worth comparing.
const WORLD: &str = "assets/worlds/probe_fleet_duel.toml";

/// A two-ship world with a streaming ASTEROID field. The class
/// `probe_fleet_duel` cannot cover: asteroid streaming was the last
/// `LocalShip`-gated write folded into the digest, so a host that centred its
/// window on the single local ship loaded a different belt than its peer and
/// the two diverged on any asteroid world (issue #1116). Its two hulls sit 12
/// lattice cells apart so a per-ship window and a fleet-wide one load visibly
/// different cell sets — the reason a revert of the fix fails this test.
const ASTEROID_WORLD: &str = "assets/worlds/probe_fleet_asteroids.toml";

/// Long enough that both fleet ships have acquired the hostile, manoeuvred and
/// traded fire, so the comparison covers per-victim RNG draws and mid-run
/// projectile mints rather than two hulls coasting.
const TICKS: u64 = 1200;

const SEED: u64 = 1_116_026;

/// The two fleet slots the world's two `game_start` ship rows take.
const SLOT_ONE: HostSlot = HostSlot(1);
const SLOT_TWO: HostSlot = HostSlot(2);
/// A stationless GM simulation participant. It owns no fleet ship and therefore
/// must never receive `LocalShip`, but it still runs the exact same world.
const SLOT_GM: HostSlot = HostSlot(3);

fn args_for(world: &str) -> HeadlessArgs {
    HeadlessArgs {
        world_path: world.into(),
        ship_path: "assets/entities/alliance_cruiser.toml".into(),
        max_ticks: TICKS,
        seed: Some(SEED),
        deterministic: true,
        ..Default::default()
    }
}

fn args() -> HeadlessArgs {
    args_for(WORLD)
}

/// A two-ship roster in which `local` is the slot this host projects.
///
/// No crew and no hull override: the ONE variable between the two runs below
/// must be the marker, so both rosters are otherwise byte-identical and both
/// ships boot fully AI-backfilled on both hosts.
fn roster(local: HostSlot) -> FleetRoster {
    FleetRoster::new(
        vec![FleetShip::new(SLOT_ONE), FleetShip::new(SLOT_TWO)],
        local,
    )
}

/// The exact two-ship, three-participant topology after a GM joins two ship
/// hosts. The GM contributes a lockstep watermark without inventing a third
/// player ship.
fn roster_with_gm(local: HostSlot) -> FleetRoster {
    FleetRoster::with_participants(
        vec![FleetShip::new(SLOT_ONE), FleetShip::new(SLOT_TWO)],
        vec![SLOT_ONE, SLOT_TWO, SLOT_GM],
        local,
        SLOT_ONE,
    )
    .expect("two ship hosts plus one stationless GM is a valid fleet topology")
}

/// A browser GM can be the only participant. That topology deliberately owns
/// no player ship; authored NPCs, structures, Regions and scenario state still
/// boot and run in its authoritative simulation.
fn gm_only_roster() -> FleetRoster {
    FleetRoster::with_participants(Vec::new(), vec![SLOT_GM], SLOT_GM, SLOT_GM)
        .expect("one stationless GM is a valid zero-ship topology")
}

/// Run the world with `local` as this host's ship, folding the digest after
/// every tick.
///
/// Deliberately does NOT join a lockstep session. A session would bring a
/// barrier, an input delay and a mesh, and this file is isolating one variable.
/// The roster alone is what `spawn_game_start_entities` reads to decide how
/// many player ships to build and which of them to tag.
fn run_host(local: HostSlot) -> Vec<(u64, u64)> {
    run_host_on(WORLD, local)
}

fn run_host_on(world: &str, local: HostSlot) -> Vec<(u64, u64)> {
    run_roster_on(world, roster(local))
}

fn app_with_roster(world: &str, roster: FleetRoster) -> bevy::prelude::App {
    let mut app = build_headless_app(&args_for(world)).expect("app should build");
    // Before the first update: `headless_auto_start` enters `InProgress` on the
    // first fixed step, and the ships are spawned by that transition.
    app.insert_resource(roster);
    app
}

/// Run one exact frozen topology and fold it after every tick.
fn run_roster_on(world: &str, roster: FleetRoster) -> Vec<(u64, u64)> {
    let mut app = app_with_roster(world, roster);

    let mut digests = Vec::with_capacity(TICKS as usize);
    for _ in 0..TICKS {
        run(&mut app, 1);
        let tick = app.world().resource::<SimTick>().0;
        digests.push((tick, world_digest(app.world())));
    }
    digests
}

/// How many ships carry each marker, and how many fleet slots exist — the
/// preconditions every assertion below rests on.
fn census(local: HostSlot) -> (usize, usize, Option<HostSlot>) {
    let args = args();
    let mut app = build_headless_app(&args).expect("app should build");
    app.insert_resource(roster(local));
    run(&mut app, 60);

    let fleet_ships = {
        let mut q = app.world_mut().query::<&FleetSlotOf>();
        q.iter(app.world()).count()
    };
    let local_ships = {
        let mut q = app
            .world_mut()
            .query_filtered::<(), bevy::prelude::With<LocalShip>>();
        q.iter(app.world()).count()
    };
    let tagged = {
        let mut q = app
            .world_mut()
            .query_filtered::<&FleetSlotOf, bevy::prelude::With<LocalShip>>();
        q.iter(app.world()).next().map(|slot| slot.0)
    };
    (fleet_ships, local_ships, tagged)
}

/// The precondition, asserted first because everything else is vacuous without
/// it: the world really does spawn two fleet ships, exactly one of them is
/// tagged, and it is the one the roster names.
#[test]
fn each_host_tags_its_own_fleet_ship_and_only_that_one() {
    for local in [SLOT_ONE, SLOT_TWO] {
        let (fleet_ships, local_ships, tagged) = census(local);
        assert_eq!(
            fleet_ships, 2,
            "the probe world must spawn a player ship per roster slot, or this \
             file is comparing two single-ship runs and proves nothing about a \
             fleet"
        );
        assert_eq!(
            local_ships, 1,
            "a host projects exactly one ship to its own crew — no more, and \
             not none"
        );
        assert_eq!(
            tagged,
            Some(local),
            "and it must be the slot the roster says is this host's"
        );
    }
}

/// **The headline.** Two hosts, one mission, the marker on a different ship —
/// and the authoritative fold agrees on every tick.
///
/// A per-tick walk rather than an end-state comparison, deliberately: the two
/// runs are not being asked to *end* the same, they are being asked never to
/// diverge, and a divergence that self-corrects by tick 360 is still a fleet
/// that showed two crews different worlds in between. It also names the tick,
/// which is where a fix starts.
#[test]
fn the_digest_does_not_care_which_ship_a_host_projects() {
    let first = run_host(SLOT_ONE);
    let second = run_host(SLOT_TWO);

    assert_eq!(
        first.len(),
        TICKS as usize,
        "precondition: the run must reach the end rather than stopping at a \
         GameOver, or the comparison covers less than it claims"
    );
    if let Some((tick, mine, theirs)) = first
        .iter()
        .zip(second.iter())
        .find(|((_, a), (_, b))| a != b)
        .map(|((tick, a), (_, b))| (*tick, *a, *b))
    {
        panic!(
            "the authoritative digest diverged at tick {tick}: the host \
             projecting slot 1 folds {mine:#018x}, the host projecting slot 2 \
             folds {theirs:#018x}.\n\n\
             `LocalShip` says which ship's crew is on THIS machine, so a value \
             it gates is a value two hosts of one mission compute differently \
             from tick zero. Find the site that reads `With<LocalShip>` / \
             `Has<LocalShip>` and writes something `sim_digest` folds, and make \
             it run on `With<Ship>` (a shared mechanic) or `With<FleetSlotOf>` \
             (a fleet-wide one) — do not re-bless this comparison. See \
             `server_app::components::LocalShip` for the rule and for the two \
             sites #1116 fixed."
        );
    }

    // Anti-vacuity: the run has to actually be doing something. Two worlds that
    // never moved would agree on every tick and prove nothing.
    let distinct: std::collections::BTreeSet<u64> = first.iter().map(|(_, d)| *d).collect();
    assert!(
        distinct.len() > TICKS as usize / 2,
        "only {} distinct digests over {TICKS} ticks — the probe world is not \
         simulating anything worth comparing",
        distinct.len()
    );
}

/// A GM joining two ship hosts is a third authoritative simulation peer, not a
/// state-streaming spectator. Its only topology difference is that no fleet
/// ship carries `LocalShip` on that machine. Every folded tick must still match
/// a ship host running the same frozen roster.
#[test]
fn a_stationless_gm_joining_two_ship_hosts_matches_their_digest_every_tick() {
    let mut first_ship_host = app_with_roster(WORLD, roster_with_gm(SLOT_ONE));
    let mut second_ship_host = app_with_roster(WORLD, roster_with_gm(SLOT_TWO));
    let mut gm_host = app_with_roster(WORLD, roster_with_gm(SLOT_GM));
    let mut distinct = std::collections::BTreeSet::new();

    for _ in 0..TICKS {
        run(&mut first_ship_host, 1);
        run(&mut second_ship_host, 1);
        run(&mut gm_host, 1);

        let tick = gm_host.world().resource::<SimTick>().0;
        let gm_digest = world_digest(gm_host.world());
        distinct.insert(gm_digest);
        for (label, ship_host) in [
            ("first ship host", &first_ship_host),
            ("second ship host", &second_ship_host),
        ] {
            let ship_tick = ship_host.world().resource::<SimTick>().0;
            assert_eq!(ship_tick, tick, "{label} and GM must compare the same tick");
            let ship_digest = world_digest(ship_host.world());
            if ship_digest != gm_digest {
                let gm_stages = project_phoenix::sim_digest::digest_stages(gm_host.world());
                let first_scope = project_phoenix::sim_digest::first_divergent_scope(
                    ship_host.world(),
                    &gm_stages,
                )
                .unwrap_or("unknown");
                panic!(
                    "the stationless GM diverged from the {label} at tick {tick}: \
                     ship host {ship_digest:#018x}, GM host {gm_digest:#018x}; first \
                     divergent scope: {first_scope}. A GM's missing `LocalShip` is \
                     presentation locality only; find the host-local input that \
                     leaked into folded state instead of adding a privileged state \
                     stream"
                );
            }
        }
    }

    assert!(
        distinct.len() > TICKS as usize / 2,
        "only {} distinct GM digests over {TICKS} ticks — the comparison is vacuous",
        distinct.len()
    );
}

/// GM-only is the M1 tracer topology: one full simulation participant, zero
/// player ships. Two fresh boots with the same authored seed must traverse the
/// same digest ledger, proving that removing the local player projection did
/// not remove or randomise the authoritative world.
#[test]
fn a_gm_only_zero_ship_session_boots_repeatably() {
    let (fleet_ships, local_ships, non_fleet_entities) = {
        let mut app = build_headless_app(&args()).expect("GM-only app should build");
        app.insert_resource(gm_only_roster());
        run(&mut app, 60);
        let fleet_ships = {
            let mut query = app.world_mut().query::<&FleetSlotOf>();
            query.iter(app.world()).count()
        };
        let local_ships = {
            let mut query = app
                .world_mut()
                .query_filtered::<(), bevy::prelude::With<LocalShip>>();
            query.iter(app.world()).count()
        };
        let non_fleet_entities = {
            let mut query = app.world_mut().query_filtered::<
                &project_phoenix::entities::spawner::EntityUuid,
                bevy::prelude::Without<FleetSlotOf>,
            >();
            query.iter(app.world()).count()
        };
        (fleet_ships, local_ships, non_fleet_entities)
    };
    assert_eq!(
        fleet_ships, 0,
        "a GM-only roster must not invent a player ship"
    );
    assert_eq!(
        local_ships, 0,
        "a stationless GM must not receive `LocalShip`"
    );
    assert!(
        non_fleet_entities > 0,
        "the GM-only tracer must still boot authored NPC, structure, Region, or scenario entities"
    );

    let first = run_roster_on(WORLD, gm_only_roster());
    let second = run_roster_on(WORLD, gm_only_roster());

    assert_eq!(
        first.len(),
        TICKS as usize,
        "the first GM-only boot completed"
    );
    assert_eq!(
        second.len(),
        TICKS as usize,
        "the second GM-only boot completed"
    );
    assert_eq!(
        first, second,
        "two GM-only boots of the same authored world and seed must match on every tick"
    );

    let distinct: std::collections::BTreeSet<u64> =
        first.iter().map(|(_, digest)| *digest).collect();
    assert!(
        distinct.len() > TICKS as usize / 2,
        "only {} distinct GM-only digests over {TICKS} ticks — the world did not run",
        distinct.len()
    );
}

/// **The asteroid class.** The same neutrality claim, on a world with a
/// streaming belt — the site the headline test above cannot reach, because
/// `probe_fleet_duel` has no asteroids (issue #1116).
///
/// Asteroid streaming used to centre its ring-buffer window on the single
/// `LocalShip`, and a rock's position folds into `sim_digest` (it is what a
/// collision resolves against). `LocalShip` is a different ship on each host of
/// a fleet, so the two hosts loaded different belts and diverged from tick zero
/// on ANY asteroid world — `combat_test.toml`, the demo, included. The fix
/// drives the window off `fleet_stream_centre` (the mean over every
/// `FleetSlotOf` ship), identical on every host. Revert it and this test fails.
#[test]
fn the_digest_does_not_care_which_ship_a_host_projects_on_an_asteroid_world() {
    // Precondition: the belt actually streams. A field that failed to load would
    // make this two empty windows agreeing about nothing.
    let rocks = {
        let mut app = build_headless_app(&args_for(ASTEROID_WORLD)).expect("app should build");
        app.insert_resource(roster(SLOT_ONE));
        run(&mut app, 120);
        let mut q = app
            .world_mut()
            .query::<&project_phoenix::server_app::Asteroid>();
        q.iter(app.world()).count()
    };
    assert!(
        rocks > 50,
        "the probe belt must stream rocks around the fleet — got {rocks}, so the \
         comparison below would be about two empty windows"
    );

    let first = run_host_on(ASTEROID_WORLD, SLOT_ONE);
    let second = run_host_on(ASTEROID_WORLD, SLOT_TWO);

    assert_eq!(
        first.len(),
        TICKS as usize,
        "precondition: the run must reach the end rather than stopping early"
    );
    if let Some((tick, mine, theirs)) = first
        .iter()
        .zip(second.iter())
        .find(|((_, a), (_, b))| a != b)
        .map(|((tick, a), (_, b))| (*tick, *a, *b))
    {
        panic!(
            "the authoritative digest diverged at tick {tick} on an ASTEROID \
             world: the host projecting slot 1 folds {mine:#018x}, the host \
             projecting slot 2 folds {theirs:#018x}.\n\n\
             Asteroid streaming must be driven by FLEET-WIDE geometry — \
             `asteroids::lifecycle::fleet_stream_centre` over every \
             `FleetSlotOf` ship — not the single `LocalShip`. A window centred \
             on the local ship loads a different belt on each host, and rock \
             positions fold into `sim_digest`, so the two hosts split from tick \
             zero. See `update_asteroid_window`."
        );
    }

    let distinct: std::collections::BTreeSet<u64> = first.iter().map(|(_, d)| *d).collect();
    assert!(
        distinct.len() > TICKS as usize / 2,
        "only {} distinct digests over {TICKS} ticks — the asteroid probe world \
         is not simulating anything worth comparing",
        distinct.len()
    );
}
