//! The standing regression guard for the seeded simulation RNG (issue #837).
//!
//! # Why this is its own test binary
//!
//! It needs the whole process to itself, for a reason `tests/headless_runner.rs`
//! cannot give it. `--deterministic` pins the scheduler by handing
//! `TaskPoolPlugin` a one-thread `TaskPoolOptions` — but Bevy's task pools are
//! process-global and initialised by whichever app builds first. Run alongside
//! the other headless tests this app inherits a pool some multi-threaded
//! neighbour already created, system execution order stops being fixed, and two
//! runs of the same seed drift apart in the last decimal place of a float sum.
//! Observed, not theorised: the assertions below pass under `--exact` and fail
//! unchanged in the shared binary.
//!
//! Cargo gives every integration-test file its own process, so keeping this
//! file to the one test is what makes `--deterministic` mean what it says.
//! Do not add unrelated tests here.

#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]

use bevy::prelude::*;
use project_phoenix::balance::BalanceEvent;
use project_phoenix::entity_spawner::EntityUuid;
use project_phoenix::headless::args::ticks_for_sim_seconds;
use project_phoenix::headless::report::RunTelemetry;
use project_phoenix::headless::{build_headless_app, build_report, run, HeadlessArgs};
use project_phoenix::server_app::LocalShip;
use project_phoenix::ship::shields::ShipShields;
use project_phoenix::weapons_plugin::TorpedoSystemResource;
use std::collections::{BTreeMap, HashSet};

/// The scenario's fixed inputs. `rng_coverage.toml` picks the world; the
/// destroyer is the hull whose bank ids keep the weapon labels unambiguous.
fn coverage_args(seed: u64) -> HeadlessArgs {
    let dt = 1.0 / 30.0;
    HeadlessArgs {
        world_path: "assets/worlds/rng_coverage.toml".into(),
        ship_path: "assets/entities/alliance_destroyer.toml".into(),
        dt,
        max_ticks: ticks_for_sim_seconds(90.0, dt),
        seed: Some(seed),
        deterministic: true,
        ..Default::default()
    }
}

/// Ticks after which the RNG-coverage run pauses to check its torpedo tubes.
/// Long enough for `spawn_game_start_entities` to have put the ships in the
/// world and for `ai_torpedo_load` to have driven a tube through its (3–4
/// second) load timer; short enough that this is still early in the run.
const RNG_COVERAGE_LOAD_TICK: u64 = 60 + 30 * 5;

/// Count tubes the AI has actually loaded, across every ship.
///
/// The tubes used to be armed by poking `set_volley_target` directly, because
/// nothing in an AI-crewed run ever asked for a tube to be loaded and the
/// torpedo damage chokepoint was therefore unreachable. `ai_torpedo_load` is
/// that missing order — it issues the same `SetTorpedoVolleyTarget` a human
/// console sends — so this is now an *assertion*, not a fixture: if the AI
/// stops loading tubes, the guard says so instead of silently papering over it
/// with a direct poke.
fn tubes_loaded_by_ai(app: &mut App) -> usize {
    let mut loaded = 0;
    let mut q = app.world_mut().query::<&TorpedoSystemResource>();
    let world = app.world();
    for torpedoes in q.iter(world) {
        loaded += torpedoes
            .0
            .tubes
            .iter()
            .filter(|t| t.loaded_count > 0)
            .count();
    }
    loaded
}

/// Zero the player ship's shields — every facing's HP, max, and regen — so they
/// stay down for the rest of the run.
///
/// This is the other half of getting a torpedo onto the *player*, which is the
/// only torpedo damage the report can see (a torpedo landing on an NPC changes
/// only the invisible internal distribution of that NPC's hull). NPC torpedo
/// auto-fire refuses to launch until its target's online shield facings total
/// zero HP — the doctrine is "phasers strip the shields, torpedoes finish the
/// hull" — so with the player's shields up the escorts never throw one. The
/// world file cannot do this: the player ship is built from the `--ship`
/// template, not the scenario's `[[entity]]` block, so its `overrides` are
/// ignored. Reaching in here is the equivalent of `arm_every_torpedo_tube` for
/// the shield precondition.
fn strip_local_ship_shields(app: &mut App) -> usize {
    let mut q = app
        .world_mut()
        .query_filtered::<&mut ShipShields, With<LocalShip>>();
    let world = app.world_mut();
    let mut facings = 0;
    for mut shields in q.iter_mut(world) {
        for facing in shields.0.facings.iter_mut() {
            facing.hp = 0;
            facing.max_hp = 0;
            facing.base_max_hp = 0;
            facing.regen_per_sec = 0.0;
            facing.base_regen_per_sec = 0.0;
            facings += 1;
        }
    }
    facings
}

/// What one RNG-coverage run produced: the report itself, plus the evidence
/// that each damage chokepoint actually ran *and landed on the player*.
struct CoverageRun {
    json: String,
    /// Systems on the player ship left below `Operational`. The only place in
    /// the report where the *distribution* of hull damage across systems is
    /// visible, and therefore the only place an unseeded generator can show up.
    damaged_systems: usize,
    /// Hull damage each weapon label dealt *to the player*, keyed by label.
    ///
    /// Player-only on purpose: hull damage to an NPC is not byte-observable in
    /// the report (a hull absorbs the same total however the seed spreads it, so
    /// two runs agree on the NPC's `damage_taken` even if one chokepoint reverts
    /// to OS entropy). Hull rather than event count because every chokepoint's
    /// RNG call sits behind a `hull_damage > 0` branch — a hit fully eaten by
    /// shields proves the weapon fired but not that the seeded path ran.
    player_hull_damage_by_weapon: BTreeMap<String, f32>,
}

fn rng_coverage_run(seed: u64) -> CoverageRun {
    let args = coverage_args(seed);
    let mut app = build_headless_app(&args).expect("app should build");
    run(&mut app, RNG_COVERAGE_LOAD_TICK);
    assert!(
        tubes_loaded_by_ai(&mut app) > 0,
        "no tube in the scenario is loaded {RNG_COVERAGE_LOAD_TICK} ticks in — \
         `ai_torpedo_load` is not issuing volley orders, so the torpedo \
         chokepoint cannot fire"
    );
    assert!(
        strip_local_ship_shields(&mut app) > 0,
        "the player ship has no shield facings to strip — the NPC torpedo \
         auto-fire precondition cannot be met"
    );
    run(&mut app, args.max_ticks - RNG_COVERAGE_LOAD_TICK);

    let player_uuids: HashSet<String> = {
        let mut q = app
            .world_mut()
            .query_filtered::<&EntityUuid, With<LocalShip>>();
        q.iter(app.world()).map(|u| u.0.clone()).collect()
    };
    let mut player_hull_damage_by_weapon: BTreeMap<String, f32> = BTreeMap::new();
    for stamped in &app.world().resource::<RunTelemetry>().balance_events {
        let BalanceEvent::DamageApplied {
            weapon,
            hull_damage,
            victim,
            ..
        } = &stamped.event
        else {
            continue;
        };
        if player_uuids.contains(victim) {
            *player_hull_damage_by_weapon
                .entry(weapon.clone())
                .or_default() += hull_damage;
        }
    }

    let report = build_report(&mut app, &args, 0.0);
    CoverageRun {
        damaged_systems: report.ship.as_ref().map_or(0, |s| s.damaged_systems.len()),
        json: report.to_json(),
        player_hull_damage_by_weapon,
    }
}

/// The standing regression guard for every RNG site (issue #837, AC-1).
///
/// Two seeded runs of a damage-heavy scenario must produce byte-identical
/// reports. The comparison is whole-report rather than a spot-check: the
/// report's `damage_by_ship` and `entity_names` are keyed by entity uuid and
/// its `damaged_systems` reflects which system each hit landed on, so *any*
/// unseeded generator in the damage or spawn path shows up here.
///
/// # Why the scenario is purpose-built
///
/// This guard used to run on `combat_test.toml`, and it was a fiction. Over the
/// tested window that scenario produced beam traffic and nothing else — no
/// torpedoes, no blasters, no damage zone, and `damaged_systems` stayed empty —
/// so four of the five chokepoints could be reverted to `rand::rng()` with the
/// whole suite still green. `rng_coverage.toml` exists to make the claim true:
/// it fires all five inside 90 sim-seconds and lands every one of them *on the
/// player*, which is the only place the report records the per-system
/// distribution the seed decides.
///
/// The `assert!`s below are the part that stops the guard rotting back into a
/// comparison of two identical empty reports. Each names one chokepoint by the
/// weapon label its balance events carry, and those labels are unique *within
/// this scenario*: `lash` and `spike` are the RNG-coverage escort's phaser and
/// blaster, `lance` its torpedo tube (the player destroyer's own phaser bank is
/// `omni` and its tubes are `fore`/`aft`, so no torpedo label can be a beam in
/// disguise), and `collision` / `region` are the two fixed environmental
/// labels. Reusing any of those ids on another hull in this world would quietly
/// break the classification — SILENTLY, attributing one chokepoint's damage to
/// another and passing while proving nothing, which is why
/// `assets/entities/test/rng_coverage_lancer.toml` says so at the top of the
/// file rather than trusting this note to be found. Every check is against hull
/// damage dealt *to the player* — see
/// [`CoverageRun::player_hull_damage_by_weapon`] for why NPC damage would not
/// prove anything.
///
/// `wall_seconds` is passed as 0.0 so the derived timing fields
/// (`ticks_per_second`, `speedup_vs_realtime`) are constants rather than
/// measurements — otherwise no two reports could ever match byte for byte.
#[test]
fn two_runs_with_the_same_seed_produce_byte_identical_reports() {
    let first = rng_coverage_run(20260720);

    // One entry per seeded chokepoint: (SimStream variant, weapon labels that
    // can only have come from it in this scenario). `lance` is the escort's
    // torpedo landing on the player; the player's own `fore`/`aft` tubes only
    // ever hit the NPCs, so they are not listed — an NPC hit would not be
    // observable and so would not guard anything.
    let chokepoints: [(&str, &[&str]); 5] = [
        ("BeamDamage", &["lash"]),
        ("BlasterDamage", &["spike"]),
        ("TorpedoDamage", &["lance"]),
        ("CollisionDamage", &["collision"]),
        ("RegionDamage", &["region"]),
    ];
    for (stream, labels) in chokepoints {
        let dealt: f32 = labels
            .iter()
            .filter_map(|l| first.player_hull_damage_by_weapon.get(*l))
            .sum();
        assert!(
            dealt > 0.0,
            "no hull damage to the player from {labels:?} — the {stream} \
             chokepoint never ran against the player, so this test cannot detect \
             it regressing. Observed player hull damage: {:?}",
            first.player_hull_damage_by_weapon
        );
    }
    assert!(
        first.damaged_systems > 0,
        "the player ship finished with every system Operational, so the report \
         carries no record of *which* system each hit picked — the one thing \
         the seeded distribution decides:\n{}",
        first.json
    );
    assert!(
        first.json.contains("\"seed\": 20260720"),
        "the report must echo the seed it ran with:\n{}",
        first.json
    );
    assert!(
        first.json.contains("\"seed_source\": \"cli\""),
        "an explicit --seed must be reported as cli-sourced:\n{}",
        first.json
    );
    assert!(
        first.json.contains("\"damage_by_ship\": {\""),
        "the run took no damage, so identical reports prove nothing:\n{}",
        first.json
    );

    let second = rng_coverage_run(20260720);
    assert_eq!(
        first.json, second.json,
        "two runs with seed 20260720 diverged — some RNG site is still unseeded"
    );
}
