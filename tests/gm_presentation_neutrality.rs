//! Observing a run may not change it (issue #1472).
//!
//! # The hazard, stated plainly
//!
//! A disposable Workshop Test can be watched two ways: through the authentic
//! viewscreen of a simulated ship, or through the omniscient Game Master
//! workspace. The second is turned on by inserting `NativeGmPresentation`,
//! which makes `gm_presentation_active` true and starts the ordinary GM
//! projection systems publishing.
//!
//! Those systems read the world. If any of them also WROTE to it — a cached
//! lookup, a lazily inserted marker, a `Local` that leaks back — then an author
//! switching to the GM view to see why a hull turned would be changing the run
//! they are asking about. The bug would be invisible in the Workshop and fatal
//! in a fleet, where one peer watching the GM desk would diverge from a peer
//! watching a ship.
//!
//! So this runs the same seeded world twice in one process, changing exactly one
//! thing between the runs — whether GM projections are publishing — and compares
//! `sim_digest::world_digest` on **every tick**.
//!
//! # Why it is its own test binary
//!
//! The same reason `tests/local_ship_neutrality.rs` is, and its header says it
//! best: `--deterministic` pins the scheduler with a one-thread
//! `TaskPoolOptions`, Bevy's task pools are process-global, and a determinism
//! claim made in a binary where other tests build apps first is a claim about
//! whoever won that race. Do not add unrelated tests here.
#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]

#[path = "common/default_pool.rs"]
// The shared pool helper carries guards and observers this binary has no use
// for; the lint stays strict for every other test that includes it.
#[allow(dead_code)]
mod default_pool;

use project_phoenix::gm_projection::NativeGmPresentation;
use project_phoenix::headless::{build_headless_app, run, world_digest, HeadlessArgs};
use project_phoenix::sim_tick::SimTick;

/// The same combat probe the LocalShip guard uses: two crewed hulls and a
/// hostile, so the comparison covers per-victim RNG draws and mid-run mints
/// rather than hulls coasting.
const WORLD: &str = "assets/worlds/probe_fleet_duel.toml";
const TICKS: u64 = 600;
const SEED: u64 = 1_472_026;
/// Far enough in that the run has spawned, acquired and started shooting, so
/// the switch lands on a world with live state rather than an empty one.
const SWITCH_AT: u64 = 200;

fn args() -> HeadlessArgs {
    HeadlessArgs {
        world_path: WORLD.into(),
        ship_path: "assets/entities/alliance_cruiser.toml".into(),
        max_ticks: TICKS,
        seed: Some(SEED),
        deterministic: default_pool::deterministic(),
        ..Default::default()
    }
}

/// Fold after every tick, optionally turning the omniscient view on part-way.
fn run_observed(switch: Option<u64>) -> Vec<(u64, u64)> {
    let mut app = build_headless_app(&args()).expect("app should build");
    let mut digests = Vec::with_capacity(TICKS as usize);
    for _ in 0..TICKS {
        run(&mut app, 1);
        let tick = app.world().resource::<SimTick>().0;
        if switch == Some(tick) {
            // Exactly what the Test's view control does.
            app.world_mut().insert_resource(NativeGmPresentation);
        }
        digests.push((tick, world_digest(app.world())));
    }
    digests
}

#[test]
fn turning_the_omniscient_view_on_mid_run_does_not_move_the_digest() {
    let unobserved = run_observed(None);
    let observed = run_observed(Some(SWITCH_AT));
    assert_eq!(
        unobserved.len(),
        observed.len(),
        "both runs should reach the same tick"
    );
    assert!(
        unobserved.iter().any(|(tick, _)| *tick > SWITCH_AT),
        "the run must continue past the switch for this to prove anything"
    );
    for (left, right) in unobserved.iter().zip(observed.iter()) {
        assert_eq!(
            left, right,
            "the digest moved when the GM view was switched on at tick {SWITCH_AT}"
        );
    }
}

#[test]
fn the_omniscient_view_can_be_turned_off_again_without_moving_the_digest() {
    let unobserved = run_observed(None);
    let mut app = build_headless_app(&args()).expect("app should build");
    let mut digests = Vec::with_capacity(TICKS as usize);
    for _ in 0..TICKS {
        run(&mut app, 1);
        let tick = app.world().resource::<SimTick>().0;
        // On, then off: an author switching back to a ship must land on the
        // same run they left, not a repaired copy of it.
        if tick == SWITCH_AT {
            app.world_mut().insert_resource(NativeGmPresentation);
        }
        if tick == SWITCH_AT * 2 {
            app.world_mut().remove_resource::<NativeGmPresentation>();
        }
        digests.push((tick, world_digest(app.world())));
    }
    assert_eq!(unobserved, digests);
}
