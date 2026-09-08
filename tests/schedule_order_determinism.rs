//! The standing regression guard for SCHEDULE order (issue #1346).
//!
//! # What this proves
//!
//! Bevy runs the systems of a set in the schedule's topological order, and two
//! systems with no ordering constraint between them get whichever relative
//! order that sort happens to produce. That order is a function of the WHOLE
//! graph, so it can flip when an entirely unrelated system joins the set — a
//! plugin gained a system, a feature was compiled in, a fixture added a probe.
//!
//! When the two unordered systems are a producer and a consumer of the same
//! authoritative state, the flip is a divergence. The consumer either reads
//! this tick's value or last tick's, and on a CADENCE-GATED consumer "last
//! tick's" costs a whole cadence period, not one step.
//!
//! That is not hypothetical. It is issue #1346: `handle_set_red_alert`
//! (Captain console, `SimSet::Input`) writes `ShipRedAlert`, and both
//! `operate_command_ai` and `apply_alert_change_to_stances` (Command console,
//! the same set) read it — with nothing ordering the three. The pair fell the
//! right way until #1346's Security System added two systems to `SimSet::Input`
//! and the sort came out the other way, so every AI-commanded ship in `duel`
//! held the wrong Tactical stance for a full `ai_snapshot_hz` period after each
//! alert change. `sim_sets::RedAlertApplied` is the label that fixed it.
//!
//! # How it perturbs
//!
//! By adding a system that does NOTHING — no queries, no resources, no
//! commands — to `SimSet::Input`. It cannot change the simulation by executing;
//! the only thing it can change is the shape of the graph the executor sorts,
//! which is exactly the variable under test.
//!
//! # Why this is not `archetype_order_determinism.rs`
//!
//! That guard's perturbation is a mid-run component insert, and it moves TWO
//! variables at once: the archetype layout (the one it means to test) and, by
//! adding the system that performs the insert, the schedule. It caught #1346's
//! regression for the second reason while claiming to test the first, which
//! sends the reader looking for a raw-order RNG walk that is not there. One
//! variable per guard: this file moves the schedule and nothing else.
//!
//! # Why this is its own test binary
//!
//! Same reason as `tests/rng_determinism.rs` and
//! `tests/archetype_order_determinism.rs`: `--deterministic` pins Bevy's
//! `TaskPoolPlugin` to a single thread, but task pools are process-global and
//! initialised by whichever app builds first. Cargo gives every integration
//! test file its own process. Do not add unrelated tests here.
//!
//! #1400's default-pool parent re-runs the exact guards in fresh child
//! processes, preserving their assertions and the ordinary pinned tests.

#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]

#[path = "common/default_pool.rs"]
mod default_pool;

#[test]
fn default_pool_preserves_schedule_order_guards() {
    default_pool::run_guards(&[
        (
            "the_digest_does_not_move_when_an_inert_system_joins_the_input_set",
            2,
        ),
        ("the_stage_breakdown_agrees_scope_by_scope", 2),
    ]);
}

use bevy::prelude::*;
use project_phoenix::headless::{build_headless_app, run, HeadlessArgs};
use project_phoenix::sim_digest::{digest_stages, world_digest};
use project_phoenix::sim_sets::SimSet;

/// `duel.toml`, for `archetype_order_determinism.rs`'s reasons and one more of
/// this guard's own: its ten ships are AI-commanded end to end, so every
/// station-stance and alert decision in the sim is taken by a cadence-gated
/// host — the consumers a producer/consumer order flip actually hurts.
const WORLD: &str = "assets/worlds/duel.toml";

/// Long enough for both sides to close, raise the alert and start losing
/// systems; short enough that the guard costs a handful of seconds. The same
/// window `archetype_order_determinism.rs` measures over, so a failure here is
/// directly comparable to one there.
const TICKS: u64 = 900;

/// The seed #1051's evidence record was taken under, kept for the same
/// reproducibility the sibling guard keeps.
const SEED: u64 = 42;

/// A system that does nothing at all.
///
/// No parameters on purpose: it reads nothing, writes nothing, queues no
/// command and registers no component, so it cannot touch the simulation by
/// running. All it can do is exist in the graph — which is the variable.
fn inert_probe() {}

fn args() -> HeadlessArgs {
    HeadlessArgs {
        world_path: WORLD.into(),
        max_ticks: TICKS,
        seed: Some(SEED),
        deterministic: default_pool::deterministic(),
        ..Default::default()
    }
}

/// Run the world and hand back its final digest, optionally with the inert
/// probe joining `SimSet::Input`.
fn run_world(perturbed: bool) -> u64 {
    let args = args();
    let mut app = build_headless_app(&args).expect("app should build");
    if perturbed {
        app.add_systems(FixedUpdate, inert_probe.in_set(SimSet::Input));
    }
    run(&mut app, args.max_ticks);
    default_pool::observe(&app, WORLD, SEED);
    world_digest(app.world())
}

/// The headline: an inert system joining `SimSet::Input` must not move the
/// authoritative digest.
#[test]
fn the_digest_does_not_move_when_an_inert_system_joins_the_input_set() {
    let clean = run_world(false);
    let perturbed = run_world(true);

    assert_eq!(
        clean, perturbed,
        "the authoritative digest moved when a system that does NOTHING was \
         added to SimSet::Input. The probe cannot change the simulation by \
         running, so what moved is the topological order Bevy sorts the set \
         into — which means two systems in that set read and write the same \
         authoritative state with no `.before`/`.after` between them, and the \
         sort had been settling that race in the fix's favour by luck. Find \
         the producer/consumer pair and give it an explicit ordering (the \
         `sim_sets::RedAlertApplied` label is the pattern, adopted in #1346 \
         for `handle_set_red_alert` and the two Command-console readers of \
         `ShipRedAlert`) rather than re-blessing this number."
    );
}

/// The same run, compared scope by scope, so a failure says WHERE.
///
/// Not a second assertion of the headline: `digest_stages` threads the same
/// accumulator through the same folds, so this cannot fail while the headline
/// passes. What it buys is the first divergent scope name in the panic —
/// `station-stances` for the #1346 regression — which is the difference
/// between reading this file and bisecting the sim.
#[test]
fn the_stage_breakdown_agrees_scope_by_scope() {
    let args = args();
    let mut clean = build_headless_app(&args).expect("app should build");
    let mut perturbed = build_headless_app(&args).expect("app should build");
    perturbed.add_systems(FixedUpdate, inert_probe.in_set(SimSet::Input));
    run(&mut clean, args.max_ticks);
    run(&mut perturbed, args.max_ticks);
    default_pool::observe(&clean, WORLD, SEED);
    default_pool::observe(&perturbed, WORLD, SEED);

    let mine = digest_stages(clean.world());
    let theirs = digest_stages(perturbed.world());
    let first = mine
        .iter()
        .zip(theirs.iter())
        .find(|((_, a), (_, b))| a != b)
        .map(|((name, _), _)| *name);
    assert_eq!(
        first, None,
        "an inert system joining SimSet::Input moved this scope of the fold; \
         the unordered producer/consumer pair writes state that lands there"
    );
}
