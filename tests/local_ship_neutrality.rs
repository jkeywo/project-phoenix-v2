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

/// A two-ship world that opens a WEIGHTED Comms decision (issue #1343).
///
/// The class the two worlds above cannot cover: neither of them contains a
/// single `open_comms` call, so `sim_digest::fold_comms_scope` takes its
/// everything-empty arm on every tick and the comms half of the fold — the
/// unmanned consoles' running weighted decisions included — was never entered by
/// this guard at all. See that world's own header for why its pool authors ONE
/// weighted option.
const COMMS_CHOICE_WORLD: &str = "assets/worlds/probe_fleet_comms_choice.toml";

/// Long enough that both fleet ships have acquired the hostile, manoeuvred and
/// traded fire, so the comparison covers per-victim RNG draws and mid-run
/// projectile mints rather than two hulls coasting.
const TICKS: u64 = 1200;

const SEED: u64 = 1_116_026;

/// The two fleet slots the world's two `game_start` ship rows take.
const SLOT_ONE: HostSlot = HostSlot(1);
const SLOT_TWO: HostSlot = HostSlot(2);

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
    let args = args_for(world);
    let mut app = build_headless_app(&args).expect("app should build");
    // Before the first update: `headless_auto_start` enters `InProgress` on the
    // first fixed step, and the ships are spawned by that transition.
    app.insert_resource(roster(local));

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

// ── The Comms class (issue #1343) ────────────────────────────────────────────

/// What one run of [`COMMS_CHOICE_WORLD`] did, beyond its per-tick digests.
struct CommsRun {
    digests: Vec<(u64, u64)>,
    /// Every fleet slot that held a running weighted wait at any point — the
    /// evidence that the walk really is fleet-wide rather than local-hull-only.
    waiting_slots: Vec<u32>,
    /// How the corridor was answered, by the end.
    granted: i64,
    held: i64,
}

/// Run the comms probe with `local` as this host's ship, sampling the fold and
/// the unmanned consoles' schedule after every tick.
fn run_comms_host(local: HostSlot) -> CommsRun {
    use project_phoenix::comms::server::CommsRuntime;
    use project_phoenix::world::server::WorldContentRuntime;

    let args = args_for(COMMS_CHOICE_WORLD);
    let mut app = build_headless_app(&args).expect("app should build");
    app.insert_resource(roster(local));

    let mut digests = Vec::with_capacity(TICKS as usize);
    let mut waiting_slots = std::collections::BTreeSet::new();
    for _ in 0..TICKS {
        run(&mut app, 1);
        let tick = app.world().resource::<SimTick>().0;
        digests.push((tick, world_digest(app.world())));
        for key in app
            .world()
            .resource::<CommsRuntime>()
            .pending_ai_responses
            .keys()
        {
            waiting_slots.insert(key.host.0);
        }
    }

    let flags = &app.world().resource::<WorldContentRuntime>().flags;
    CommsRun {
        digests,
        waiting_slots: waiting_slots.into_iter().collect(),
        granted: flags.counter("corridor_granted"),
        held: flags.counter("corridor_held"),
    }
}

/// **The Comms class.** The same neutrality claim on a world that opens a
/// WEIGHTED Backfill decision — the scope neither probe above reaches.
///
/// Two folded things #1343 added hang on this: `CommsRuntime::pending_ai_responses`
/// (walked by `sim_digest::fold_comms_scope`) and the position of
/// `SimStream::CommsBackfillChoice` (`SimRngState` is folded whole). Both used to
/// be written by a `With<LocalShip>` host, which is a different hull on each host
/// of a fleet — so the schedule a peer folded, and the number of draws it had
/// taken, depended on which crew was sitting where. The fix walks every
/// `FleetSlotOf` hull in slot order and gates only the EMISSION of the admitted
/// response on the local marker, exactly as a human officer's press replicates.
#[test]
fn the_digest_does_not_care_which_ship_a_host_projects_through_a_backfill_decision() {
    let first = run_comms_host(SLOT_ONE);
    let second = run_comms_host(SLOT_TWO);

    // Anti-vacuity, and the sharp end of the claim: BOTH hulls must have held a
    // wait of their own. One slot here means the walk saw one hull — which is
    // the `LocalShip` gating this test exists to forbid.
    assert_eq!(
        first.waiting_slots,
        vec![SLOT_ONE.0, SLOT_TWO.0],
        "every fleet hull's Comms console decides on every host; a schedule \
         holding only one slot's wait was computed from `LocalShip`"
    );
    assert_eq!(
        first.waiting_slots, second.waiting_slots,
        "and the same two slots whichever hull this host projects"
    );

    // …and the decision actually RESOLVED, so the comparison covers arm → fold →
    // draw → admitted answer → `on_pick`, not just an armed wait.
    assert_eq!(
        (first.granted, first.held),
        (1, 0),
        "the corridor must be granted exactly once and never held: index 0 is \
         the stand-by and carries `ai_weight = 0`, so an unmanned bridge is \
         forbidden it outright"
    );
    assert_eq!(
        (second.granted, second.held),
        (first.granted, first.held),
        "and answered identically whichever hull this host projects"
    );

    assert_eq!(
        first.digests.len(),
        TICKS as usize,
        "precondition: the run must reach the end rather than stopping early"
    );
    if let Some((tick, mine, theirs)) = first
        .digests
        .iter()
        .zip(second.digests.iter())
        .find(|((_, a), (_, b))| a != b)
        .map(|((tick, a), (_, b))| (*tick, *a, *b))
    {
        panic!(
            "the authoritative digest diverged at tick {tick} on a world holding \
             a WEIGHTED Comms decision: the host projecting slot 1 folds \
             {mine:#018x}, the host projecting slot 2 folds {theirs:#018x}.\n\n\
             `CommsRuntime::pending_ai_responses` and the `CommsBackfillChoice` \
             stream's position are both folded, so the walk that writes them must \
             run over every `FleetSlotOf` hull in slot order — never \
             `With<LocalShip>`. Only the EMISSION of the admitted \
             `RespondToMessage` is this host's business. See \
             `console::comms::server::operate_comms_response_ai`."
        );
    }
}
