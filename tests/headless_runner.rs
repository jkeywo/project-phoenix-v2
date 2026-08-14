//! End-to-end tests for the headless runner.
//!
//! These live in an integration test rather than a `#[cfg(test)] mod tests`
//! inside `src/headless/app.rs` for one specific reason: building a headless
//! app populates the process-global native template cache
//! (`config_cache::insert_native_config`), which is exactly the right shape for
//! a binary that runs one app per process but leaks between tests that share
//! one. Run inside the lib test binary, these would hand ~2500 unrelated unit
//! tests a populated cache where they had always seen an empty one — which
//! silently changes, for instance, the radar range the helm AI tests assert on.
//!
//! Cargo gives each integration test file its own process, which restores the
//! isolation. Keep app-building tests here; keep pure ones (`args`, `report`)
//! inline in their modules.

#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]

use bevy::prelude::*;
use project_phoenix::ai_plugin::AiHighFidelity;
use project_phoenix::balance::RunOutcome;
use project_phoenix::headless::args::ticks_for_sim_seconds;
use project_phoenix::headless::{build_headless_app, build_report, run, HeadlessArgs};
use project_phoenix::messages::GamePhase;
use project_phoenix::server_app::LocalShip;

#[test]
fn axiom_station_defence_config_parses() {
    let source =
        project_phoenix::entity_includes::resolve_from_disk("assets/entities/station_axiom.toml")
            .expect("Axiom Station template should resolve");
    let config = project_phoenix::entity_config::EntityConfig::from_toml(&source.toml)
        .expect("Axiom Station's autonomous defence should be a valid entity config");
    assert!(
        config.is_static_point_defence(),
        "Axiom Station's ownerless Tactical systems must activate static point defence"
    );
}
use project_phoenix::ship::control_source::ControlSource;
use project_phoenix::ship::state::ShipPhysics;
use project_phoenix::ship_plugin::{
    ActiveStationRatings, ShipConfigComponent, ShipSystemControlSources,
};
use project_phoenix::simmath;

fn test_args() -> HeadlessArgs {
    HeadlessArgs {
        world_path: "assets/worlds/patrol.toml".into(),
        ship_path: "assets/entities/alliance_cruiser.toml".into(),
        max_ticks: 30,
        ..Default::default()
    }
}

#[test]
fn builds_and_ticks_without_a_renderer() {
    let args = test_args();
    let mut app = build_headless_app(&args).expect("app should build");
    assert_eq!(run(&mut app, args.max_ticks), 30);
}

#[test]
fn auto_start_reaches_in_progress_with_no_players() {
    let args = test_args();
    let mut app = build_headless_app(&args).expect("app should build");
    run(&mut app, args.max_ticks);
    assert_eq!(
        app.world().resource::<State<GamePhase>>().get(),
        &GamePhase::InProgress
    );
}

/// The headline requirement: a run with nobody connected must leave the player
/// ship fully AI-crewed. Proves the boot path and the backfill path at once.
#[test]
fn player_ship_is_fully_backfilled_by_ai() {
    let args = test_args();
    let mut app = build_headless_app(&args).expect("app should build");
    run(&mut app, args.max_ticks);

    let mut q = app
        .world_mut()
        .query_filtered::<(&ShipSystemControlSources, &ActiveStationRatings), With<LocalShip>>();
    let (sources, ratings) = q
        .single(app.world())
        .expect("exactly one LocalShip should exist");

    assert!(
        !ratings.0.is_empty(),
        "player ship has no stations — ship config did not reach the lobby"
    );
    for (station, rating) in &ratings.0 {
        assert_eq!(
            rating,
            project_phoenix::ship::rating::BACKFILL_RATING,
            "station {station:?} is not backfilled"
        );
    }
    assert!(
        sources.0.entries().any(|(_, s)| *s == ControlSource::Ai),
        "no system ended up under AI control"
    );
}

/// Issue #786 wiring guard: the PLAYER ship must carry the per-system AI
/// components its own template authored.
///
/// The player ship never goes through `entities::spawner::spawn_entity` — only
/// `server_app::spawn_game_start_entities` builds it — and both Comms AI hosts
/// (`operate_comms_ai`, `operate_comms_response_ai`) are filtered
/// `With<LocalShip>`, i.e. they run ONLY on the player ship. So a missing
/// `server_app` attach meant the feature was dead in production: the hosts fell
/// back to a tick-local canonical default every tick, an authored
/// `[comms_console]` block was parsed, validated and then silently ignored, and
/// `self_fact/fact(power_rating)` was permanently ABSENT (the #779 empty-facts
/// failure mode). `RepairTargetSelector` had the same gap on the player ship —
/// less severe because `operate_repair_ai`'s host is `With<Ship>`, so
/// spawner-built NPCs already had one, but an authored `[repair.selector]` on a
/// player-class hull was still ignored.
///
/// This boots the real headless world, so it fails if either attach block is
/// removed. The `power_rating` assertion is the sharp end: it is the value the
/// selector expressions read as `self_fact(power_rating)`, and it can only be
/// right if the component was built from the ship's own `EntityConfig`.
#[test]
fn player_game_start_spawn_attaches_the_comms_and_repair_ai_components() {
    use project_phoenix::console::comms::server::{CommsResponseAiPolicy, CommsTargetSelector};
    use project_phoenix::console::repair::server::RepairTargetSelector;

    let args = test_args();
    let mut app = build_headless_app(&args).expect("app should build");
    run(&mut app, args.max_ticks);

    // `assets/entities/alliance_cruiser.toml` declares `power_rating = 90`.
    let expected_rating = Some(90.0_f32);

    let mut q = app.world_mut().query_filtered::<(
        &CommsTargetSelector,
        &CommsResponseAiPolicy,
        &RepairTargetSelector,
    ), With<LocalShip>>();
    let (comms_selector, _comms_policy, repair_selector) = q.single(app.world()).expect(
        "the player ship must carry the Comms hail selector, the Comms \
                 response policy, and the Repair selector — is the attach block \
                 still present in `spawn_game_start_entities`?",
    );

    assert_eq!(
        comms_selector.power_rating, expected_rating,
        "the Comms selector must carry the ship template's authored \
         power_rating; `self_fact(power_rating)` and `fact(power_rating)` are \
         both read off this component"
    );
    assert_eq!(
        repair_selector.power_rating, expected_rating,
        "the Repair selector must carry the same authored power_rating"
    );
}

/// Sim time is a function of tick count alone, and `HeadlessArgs::sim_seconds`
/// is the authority on the conversion.
#[test]
fn sim_time_matches_the_advertised_span() {
    let mut args = test_args();
    args.dt = 1.0 / 30.0;
    args.max_ticks = ticks_for_sim_seconds(1.0, args.dt);
    let mut app = build_headless_app(&args).expect("app should build");
    run(&mut app, args.max_ticks);
    let elapsed = app.world().resource::<Time>().elapsed_secs_f64();
    assert!(
        (elapsed - args.sim_seconds()).abs() < 1e-6,
        "expected {}s of sim time, got {elapsed}",
        args.sim_seconds()
    );
}

#[test]
fn sim_time_is_independent_of_the_tick_rate() {
    for hz in [20.0, 60.0, 144.0] {
        let mut args = test_args();
        args.dt = 1.0 / hz;
        args.max_ticks = ticks_for_sim_seconds(0.5, args.dt);
        let mut app = build_headless_app(&args).expect("app should build");
        run(&mut app, args.max_ticks);
        let elapsed = app.world().resource::<Time>().elapsed_secs_f64();
        assert!(
            (elapsed - 0.5).abs() < 1e-6,
            "{hz}Hz gave {elapsed}s of sim time, expected 0.5"
        );
    }
}

/// Runs `patrol` for `sim_secs` driven at `hz` FRAMES per sim-second and
/// returns the ship's position.
///
/// Since issue #895 the SIMULATION advances at the world's authored
/// `sim_tick_hz` inside the fixed loop regardless of this frame rate, so the
/// three drives below differ only in how many logical ticks each frame runs —
/// the residual drift is nanosecond rounding of the frame period against the
/// timestep (±1 tick per run) plus rapier, which still steps per frame until
/// #896. `patrol.toml`, not `combat_test.toml` (issue #842): the backfilled
/// player travels a deterministic, non-combat course there, which keeps the
/// measurement about the schedule rather than about chaotic combat pursuit.
fn ship_position_after(hz: f64, sim_secs: f64) -> (f32, f32) {
    let dt = 1.0 / hz;
    let args = HeadlessArgs {
        world_path: "assets/worlds/patrol.toml".into(),
        dt,
        max_ticks: ticks_for_sim_seconds(sim_secs, dt),
        deterministic: true,
        ..test_args()
    };
    let mut app = build_headless_app(&args).expect("app should build");
    run(&mut app, args.max_ticks);
    let mut q = app
        .world_mut()
        .query_filtered::<&ShipPhysics, With<LocalShip>>();
    let p = q.single(app.world()).expect("player ship should exist");
    (p.x, p.z)
}

/// The point of the fixed logical tick (issue #895): the *simulation* — not
/// just the clock — lands in the same place regardless of the FRAME rate the
/// harness drives it at. The sim itself always steps at the authored
/// `sim_tick_hz`, so the pre-#895 caveats about `HELM_AI_MAX_DT_SECS`
/// under-integration at slow rates no longer apply — every drive runs the
/// same integration step. The tolerance absorbs the ±1-tick frame-period
/// rounding and the still-frame-driven rapier (#896); the EXACT cross-rate
/// guarantee is pinned bit-for-bit by
/// `the_simulation_reaches_the_same_state_at_wildly_different_frame_rates`.
#[test]
fn simulation_state_is_rate_independent_at_or_above_30hz() {
    let at_60 = ship_position_after(60.0, 4.0);
    for (label, hz) in [("30Hz", 30.0), ("120Hz", 120.0)] {
        let p = ship_position_after(hz, 4.0);
        let drift = ((p.0 - at_60.0).powi(2) + (p.1 - at_60.1).powi(2)).sqrt();
        assert!(
            drift < 1.0,
            "{label} drifted {drift} units from the 60Hz run ({p:?} vs {at_60:?})"
        );
    }
}

/// `--deterministic` must give bit-identical runs, not merely
/// wall-clock-independent ones.
#[test]
fn deterministic_runs_reproduce_exactly() {
    assert_eq!(
        ship_position_after(60.0, 4.0),
        ship_position_after(60.0, 4.0),
        "deterministic run was not reproducible"
    );
}

/// The wiring seam for the damage ledger (issue #836, AC-1).
///
/// Every other ledger test is pure — `core::balance` folds a hand-written
/// event log, and `headless::report` hand-builds a `RunReport`. None of them
/// touch `build_headless_app`, so dropping `collect_balance_events` from its
/// schedule would leave the whole suite green while the feature emitted
/// nothing. This test is the only thing that fails in that case: it runs the
/// real app and reads the real report.
///
/// Runs on `probe_duel.toml`, not `combat_test.toml` (issue #842). This used to
/// lean on `combat_test`'s razor-tuned 100 s window — nothing dealt before ~75 s,
/// first return fire at ~90 s — which only ever held while the player sat inert
/// and let the waves close. Now the backfilled player carries default `[behaviour]`
/// doctrine (#842) and maneuvers, and `combat_test` is a chaotically-sensitive
/// scenario: a few units of player drift cascade into a completely different
/// engagement, so that hand-tuned window no longer reliably produces a two-sided
/// exchange. `probe_duel` is a purpose-built deterministic duel that trades fire
/// to a resolution in 60 s — a far more robust way to prove the same wiring: a
/// real app run that populates the real ledger with both `dealt` and `taken`.
#[test]
fn a_real_combat_run_populates_the_per_ship_damage_ledger() {
    let dt = 1.0 / 30.0;
    let args = HeadlessArgs {
        world_path: "assets/worlds/probe_duel.toml".into(),
        dt,
        max_ticks: ticks_for_sim_seconds(60.0, dt),
        deterministic: true,
        ..test_args()
    };
    let mut app = build_headless_app(&args).expect("app should build");
    run(&mut app, args.max_ticks);
    let report = build_report(&mut app, &args, 0.0);

    assert!(
        !report.damage_by_ship.is_empty(),
        "a 100s combat run produced no ledger rows — is `collect_balance_events` \
         still registered in `build_headless_app`?"
    );
    let dealt: f32 = report.damage_by_ship.values().map(|l| l.damage_dealt).sum();
    let taken: f32 = report.damage_by_ship.values().map(|l| l.damage_taken).sum();
    assert!(
        dealt > 0.0,
        "no ship dealt damage; the chokepoints are not naming an attacker: {:?}",
        report.damage_by_ship
    );
    assert!(
        taken > 0.0,
        "no ship took damage: {:?}",
        report.damage_by_ship
    );
}

/// The seed is part of the report whether or not one was asked for, so a run
/// that surprises you can always be replayed. `combat_test.toml` authors
/// `[global] seed`, which is the middle tier of the precedence chain.
#[test]
fn an_unseeded_run_still_reports_the_seed_it_used() {
    let args = HeadlessArgs {
        world_path: "assets/worlds/combat_test.toml".into(),
        max_ticks: 30,
        ..test_args()
    };
    let mut app = build_headless_app(&args).expect("app should build");
    run(&mut app, args.max_ticks);
    let report = build_report(&mut app, &args, 0.0);

    assert_eq!(report.seed, 475, "world TOML seed should have been used");
    assert_eq!(report.seed_source, "world");
    assert!(
        !args.deterministic,
        "a world seed must not pin the scheduler"
    );
}

/// Issue #838 acceptance: a world-spawned Alliance (console-suite) hull, given a
/// hostile faction and a standing Destroy doctrine via `spawn_entity`
/// `overrides`, must engage the player and trade fire until the duel resolves in
/// a destruction — not sit inert.
///
/// # What this pins down (the two gates the world-spawn path failed)
///
/// The `overrides = { faction, behaviour }` on a `spawn_entity` action was being
/// silently dropped before it reached the ECS, for two independent reasons, both
/// in the template→override→re-parse round-trip:
///
/// 1. `DoctrineObjective.text` was a *required* serde field, so an inline
///    doctrine directive (which omits display prose) made the merged config fail
///    to re-parse; `dispatch_spawn_entity` then swallowed the error and kept the
///    un-overridden template.
/// 2. `EntityConfig::ship_config` (the `[[station]]`/`[[system]]`/
///    `[power_groups]` suite) is `#[serde(skip)]`, so a plain
///    `toml::to_string(&template)` dropped every ship system; the re-parsed
///    override-config had no stations, no weapons, and nothing under AI control.
///
/// With both fixed, the destroyer keeps its Harrow faction, its Destroy
/// doctrine, and its full weapon suite; `ai_target_selection` acquires the
/// player through the nearest-hostile tier, the helm pursues, and the phasers
/// fire. This is a probe world, so the run is deterministic.
#[test]
fn world_spawned_alliance_hull_returns_fire_and_the_duel_resolves() {
    let dt = 1.0 / 30.0;
    let args = HeadlessArgs {
        world_path: "assets/worlds/probe_duel.toml".into(),
        dt,
        // Window re-blessed for issue #907's review (was 60 s). Moving the
        // game-start `NextState<GamePhase>` writers into `FixedUpdate` so the
        // player-ship mint lands on a deterministic tick (rather than the
        // frame-level `StateTransition`) shifts exactly when the duel's
        // combatants first exist by one tick relative to the old, off-tick
        // spawn — this world's seed-34 duel is combat-chaotic (see
        // `server_app::add_simulation_plugins_with`'s own registration-order
        // note on this same fragility), so that one-tick shift reorders the
        // whole run's RNG draws and the fight now settles later than the old
        // 60 s window allowed. Confirmed still resolving in a kill by 180 s;
        // widened rather than re-seeded so this world's seed-34 stays every
        // other probe's shared control condition.
        max_ticks: ticks_for_sim_seconds(180.0, dt),
        deterministic: true,
        // Re-blessed for issue #896 (see the sweep in `probe_duel.toml`). With
        // rapier moved onto the logical tick, beam line-of-sight is resolved
        // against colliders synced this tick instead of last frame's, and the
        // duel this world's default seed used to close in 18 s now settles into
        // a standoff. 34 is the seed that resolves on the new physics, pinned
        // here rather than made the world default because every other probe on
        // this world is measured against seed 3 as its control condition.
        seed: Some(34),
        ..test_args()
    };
    let mut app = build_headless_app(&args).expect("app should build");
    run(&mut app, args.max_ticks);
    let report = build_report(&mut app, &args, 0.0);

    // Both ships appear in the per-ship damage ledger, and *both* dealt damage:
    // the world-spawned hull fired on its enemy (the original bug: it dealt
    // zero), and the player returned fire. Two rows each with dealt > 0 is the
    // encoding of "both sides take damage — not a one-sided execution".
    assert_eq!(
        report.damage_by_ship.len(),
        2,
        "expected both duelists in the ledger, got {:?}",
        report.damage_by_ship
    );
    for (uuid, ledger) in &report.damage_by_ship {
        assert!(
            ledger.damage_dealt > 0.0,
            "ship {uuid} dealt no damage — one-sided: {:?}",
            report.damage_by_ship
        );
        assert!(
            ledger.damage_taken > 0.0,
            "ship {uuid} took no damage: {:?}",
            report.damage_by_ship
        );
    }
    // The aggressor (the world-spawned destroyer) does the bulk of the damage;
    // require a substantial total so a stray asteroid graze can never pass this.
    let max_dealt = report
        .damage_by_ship
        .values()
        .map(|l| l.damage_dealt)
        .fold(0.0_f32, f32::max);
    assert!(
        max_dealt > 100.0,
        "the world-spawned hull should deal real damage, peak dealt was {max_dealt}: {:?}",
        report.damage_by_ship
    );

    // The fight ends in a destruction, not a stalemate. The destroyer's Destroy
    // doctrine names the player, so the player is the one that dies → GameOver.
    assert_eq!(
        report.final_phase,
        format!("{:?}", GamePhase::GameOver),
        "the duel must resolve in a kill, ended in phase {}",
        report.final_phase
    );
    // The LocalShip's death latches the run as a defeat (#843) — reached via the
    // built-in player-death path, not a scenario trigger, so no `outcome` flag
    // is authored anywhere and the classifier must still read defeat.
    assert_eq!(
        report.outcome_report.outcome,
        RunOutcome::Defeat,
        "a run that ends in the LocalShip's death is a defeat"
    );
    // Draw/timeout margins are populated regardless (AC2): the enemy destroyer
    // survives, so its side keeps hull.
    assert!(
        report.outcome_report.enemy.remaining_hull > 0.0,
        "the surviving enemy side should report remaining hull: {:?}",
        report.outcome_report
    );
}

/// The engagement must SURVIVE the whole run — a duel that goes quiet after one
/// clash is a bug even when nobody has died yet.
///
/// # The failure this pins
///
/// `AiProfile.sensor_range` is what `ai::lod::evaluate_lod` measures an NPC
/// against: past `sensor_range * 1.2` for `LOD_DWELL_SECS`, a ship loses
/// `AiHighFidelity` and every decider that travels with it, and
/// `ai::server::simulate_low_lod_ships` dead-reckons it on its LAST heading and
/// speed for ever. No Alliance hull authored an `[ai_profile]`, so an Alliance
/// hull spawned as an NPC took the 100.0 fallback in `entities::spawner` — a
/// 120-unit demote ring INSIDE its own authored doctrine envelope. The
/// destroyer's attack-pass `escape` leg commits outward under boost (x3 of
/// `max_speed = 15`) and crossed that ring about a second before the 6 s
/// `escape_duration_secs` dwell would have turned it back around, so the
/// aggressor demoted mid-manoeuvre and coasted out of the scenario at its
/// frozen boosted speed, never to return. The player's captain then stood the
/// alert down on the honest reading — its hostile really was 500+ units away —
/// and both hulls sat inert for the rest of the run.
///
/// Asserting on TOTAL damage rather than on the kill is deliberate: the kill is
/// a balance outcome the fleet's durability ladder moves around, while "the two
/// hulls are still shooting at t = 60 s" is the invariant this world exists to
/// demonstrate. Under the freeze the pair traded 124 points in 60 s and then
/// nothing; sustained, they trade upwards of 700.
#[test]
fn the_duel_keeps_fighting_for_the_whole_run() {
    let dt = 1.0 / 30.0;
    let args = HeadlessArgs {
        world_path: "assets/worlds/probe_duel.toml".into(),
        dt,
        max_ticks: ticks_for_sim_seconds(60.0, dt),
        deterministic: true,
        ..test_args()
    };
    let mut app = build_headless_app(&args).expect("app should build");
    run(&mut app, args.max_ticks);
    let report = build_report(&mut app, &args, 0.0);

    let total_dealt: f32 = report.damage_by_ship.values().map(|l| l.damage_dealt).sum();
    assert!(
        total_dealt > 400.0,
        "the duel went quiet: only {total_dealt} damage across 60 s. A hull that \
         demotes out of high fidelity mid-manoeuvre never comes back — check that \
         every shipped hull authors an [ai_profile] sensor_range wide enough for \
         the movement doctrine it flies. Ledger: {:?}",
        report.damage_by_ship
    );

    // The aggressor's attack pass must CLOSE. `escape` is a bounded commitment
    // (`escape_duration_secs = 6`), so a hull that spends most of the run in it
    // is a hull whose state machine stopped being evaluated — the demotion
    // signature, caught here even if the shooting looked healthy early on.
    let longest_escape = report
        .damage_by_ship
        .values()
        .filter_map(|l| l.phase_seconds.get("escape").copied())
        .fold(0.0_f64, f64::max);
    assert!(
        longest_escape < 30.0,
        "a hull sat in the attack pass's `escape` leg for {longest_escape} s of a \
         60 s run; that leg's authored dwell is 6 s, so its doctrine machine \
         stopped being evaluated"
    );
}

/// **Issue #893's headless evidence.** A tactical radar reaching Destroyed
/// must make the ship STOP FIRING — not keep shooting the target it already
/// locked, which was the bug #887's A/B surfaced (both battleships in that
/// duel had `tactical-radar` Destroyed with time-to-kill still to run, and
/// nothing about their fire changed).
///
/// Runs on `probe_radar_kill.toml` rather than `probe_duel.toml`: since #893
/// also removed hit points from every shipped hull's `tactical-radar` (AC3),
/// an organic duel between shipped hulls can no longer destroy one at all, so
/// this world's `spawn_entity` override restores the destroyer's pre-#893
/// hull verbatim (a deliberately damageable test-only radar — see the world's
/// own header) to reproduce the exact scenario the decision is about.
///
/// The hostile's hull tracks its ORIGINAL system list, not just the radar
/// alone — collapsing it to "just the radar" would make destroying it read as
/// the whole ship dying (`SystemHull::is_destroyed` is "every tracked system
/// at 0"), which is a different bug (ship death) wearing this one's clothes.
#[test]
fn destroying_the_tactical_radar_stops_the_ship_firing_instead_of_shooting_its_memory() {
    use project_phoenix::damage::DamageTier;
    use project_phoenix::entity_spawner::EntitySystemHull;
    use project_phoenix::simulation::Ship;
    use project_phoenix::system_registry::tactical_radar_system_id;
    use project_phoenix::weapons_plugin::TacticalRadarSelection;

    let dt = 1.0 / 30.0;
    let args = HeadlessArgs {
        world_path: "assets/worlds/probe_radar_kill.toml".into(),
        // The world's own `[[entity]] template_path` for "player-ship" is only
        // a placeholder — `ship_path` (mirroring `--ship`/`PendingShipConfig`,
        // see `duel.toml`'s own comment on the same point) is what actually
        // selects the player's hull, and must match the world's battleship
        // fixture or the player spawns as the `test_args()` default cruiser
        // instead, which never lands real hull damage on this hostile at all.
        ship_path: "assets/entities/alliance_battleship.toml".into(),
        dt,
        max_ticks: 0, // driven by hand below
        // Re-blessed for issue #907's review (was seed 12, empirically swept
        // over 1..15 on a 60 s window). Moving the game-start
        // `NextState<GamePhase>` writers into `FixedUpdate` so the player-ship
        // mint lands on a deterministic tick shifts this combat-chaotic
        // duel's RNG draws by one tick (see the two `probe_duel.toml` re-bless
        // notes above for the same mechanism); on the new timing, seed 12's
        // fight now resolves into a full kill BEFORE the radar specifically
        // reaches Destroyed — exactly the "different bug" this test's own
        // doc comment says it deliberately does not chase, just for a
        // different seed than before. Re-swept over seeds 1..30 on a 90 s
        // window (recorded below); seed 1 destroys the hostile's tactical
        // radar at tick <=868 (~29 s) with BOTH ships still alive — the same
        // shape the original seed 12 gave, just re-timed.
        //
        // Sweep table (seed: destroyed_tick, resolved-before-radar-died):
        //   1:868,no  2:none,yes(GameOver)  3:961,no  4:1023,no  5:none,no
        //   6:1209,no  7:868,no  8:992,no  9:868,no  10:none,yes  11:961,no
        //   12:none,yes  13:none,yes  14:868,no  15:none,yes  16:868,no
        //   17:none,no  18:none,no  19:868,no  20:868,no  21:none,yes
        //   22:868,no  23:961,no  24:868,no  25:none,no  26:none,no  27:868,no
        //   28:none,yes  29:868,no
        // ("none" = radar never reached Destroyed inside 90s; "yes" = the
        // fight ended in GameOver before or at the tick the radar died —
        // both disqualify a seed for this test). Seed 1 chosen as the
        // simplest surviving candidate, not because it is otherwise special.
        seed: Some(1),
        deterministic: true,
        ..test_args()
    };
    let mut app = build_headless_app(&args).expect("app should build");
    app.finish();
    app.cleanup();

    let radar_id = tactical_radar_system_id();

    // The one ship that is not the player — the world spawns exactly one NPC.
    fn hostile_hull_tier(
        app: &mut App,
        radar_id: &project_phoenix::messages::SystemId,
    ) -> DamageTier {
        let mut q = app
            .world_mut()
            .query_filtered::<&EntitySystemHull, (With<Ship>, Without<LocalShip>)>();
        q.single(app.world())
            .expect("exactly one hostile ship in this duel")
            .0
            .tier_for(radar_id)
    }
    fn player_hull_total(app: &mut App) -> f32 {
        let mut q = app
            .world_mut()
            .query_filtered::<&EntitySystemHull, With<LocalShip>>();
        q.single(app.world())
            .expect("player hull")
            .0
            .total_current()
    }
    // Cumulative damage the player has TAKEN since the run began, per the
    // stamped balance-event ledger — never raw hull HP. Repair teams restore
    // `EntitySystemHull` HP over time on their own authored cadence
    // (`ship::repair`), independent of whether the hostile is still
    // shooting, so a flat-equality check on `player_hull_total` across a
    // window with repair running would fail even with the hostile fully
    // disarmed: HP visibly climbs from repair while `damage_taken` — a
    // monotonic ledger of landed hits, unaffected by healing — correctly
    // stays flat. `build_report` folds `telemetry.balance_events` fresh each
    // call, so calling it mid-run at two ticks and diffing is exactly "damage
    // landed between those two ticks."
    fn player_damage_taken(app: &mut App, args: &HeadlessArgs) -> f32 {
        let player_uuid = {
            let mut q = app
                .world_mut()
                .query_filtered::<&project_phoenix::entity_spawner::EntityUuid, With<LocalShip>>();
            q.single(app.world()).expect("player uuid").0.clone()
        };
        let report = build_report(app, args, 0.0);
        report
            .damage_by_ship
            .get(&player_uuid)
            .map(|l| l.damage_taken)
            .unwrap_or(0.0)
    }

    // Run until the hostile's tactical radar reaches Destroyed. The fixture's
    // hull authors real HP on it (see the world's header), so organic combat
    // — the player returning fire, exactly as `probe_duel.toml` already
    // proves it does — destroys it inside a generous 90 s window.
    let ticks_per_check = ticks_for_sim_seconds(1.0, dt).max(1);
    let max_ticks = ticks_for_sim_seconds(90.0, dt);
    let mut destroyed_at_tick: Option<u64> = None;
    let mut player_hull_at_destroy: Option<f32> = None;
    let mut tick = 0u64;
    while tick < max_ticks {
        for _ in 0..ticks_per_check.min(max_ticks - tick) {
            app.update();
            tick += 1;
            if app.world().resource::<State<GamePhase>>().get() == &GamePhase::GameOver {
                break;
            }
        }
        if hostile_hull_tier(&mut app, &radar_id) == DamageTier::Destroyed {
            destroyed_at_tick = Some(tick);
            player_hull_at_destroy = Some(player_hull_total(&mut app));
            break;
        }
        if app.world().resource::<State<GamePhase>>().get() == &GamePhase::GameOver {
            break;
        }
    }

    let destroyed_at_tick = destroyed_at_tick.unwrap_or_else(|| {
        panic!(
            "the hostile's tactical radar never reached Destroyed inside the {max_ticks}-tick \
             probe window — the fixture may need a re-bless of `[global] seed` in \
             probe_radar_kill.toml (see probe_duel.toml's own seed-sweep note for the pattern)"
        )
    });
    let player_hull_at_destroy = player_hull_at_destroy.expect("set alongside destroyed_at_tick");

    // AC1 — the SAME transition clears the standing lock.
    let hostile_lock = {
        let mut q = app
            .world_mut()
            .query_filtered::<&TacticalRadarSelection, (With<Ship>, Without<LocalShip>)>();
        q.single(app.world())
            .expect("the hostile carries a target lock")
            .0
            .clone()
    };
    assert_eq!(
        hostile_lock, None,
        "the hostile's standing lock must be cleared the moment its tactical radar \
         reaches Destroyed (tick {destroyed_at_tick})"
    );

    // AC4 (headless evidence) — run on, and the player STOPS taking damage
    // once the hostile's own weapons settle: the disarmed hostile is not
    // shooting the target it remembers. The hostile's weapon systems
    // (phaser, torpedo tubes) are still fully HP'd — this isolates "lost its
    // lock" from "lost its guns".
    //
    // The window is split in two rather than asserting flat equality straight
    // off the destroy tick: `TorpedoConfig::default().lifespan` is 20 s, so a
    // torpedo the hostile already launched (with its own captured target,
    // independent of `TacticalRadarSelection`) keeps homing and can still
    // land a hit for up to 20 s after the radar — and the lock — are gone.
    // That is not the bug #893 fixes; it is ordnance already in flight. The
    // SETTLE window absorbs that legitimate tail; only the CHECK window after
    // it is required to be perfectly flat, which is what "stopped firing"
    // actually means once every already-launched round has landed or
    // expired.
    let settle_secs = 25.0; // > the 20 s default torpedo lifespan
    let check_secs = 15.0;
    for _ in 0..ticks_for_sim_seconds(settle_secs, dt) {
        app.update();
        if app.world().resource::<State<GamePhase>>().get() == &GamePhase::GameOver {
            break;
        }
    }
    let player_hull_after_settle = player_hull_total(&mut app);
    let player_damage_taken_after_settle = player_damage_taken(&mut app, &args);
    for _ in 0..ticks_for_sim_seconds(check_secs, dt) {
        app.update();
        if app.world().resource::<State<GamePhase>>().get() == &GamePhase::GameOver {
            break;
        }
    }
    let player_hull_final = player_hull_total(&mut app);
    let player_damage_taken_final = player_damage_taken(&mut app, &args);
    assert_eq!(
        player_damage_taken_final,
        player_damage_taken_after_settle,
        "the player took {:.1} further damage in the {check_secs}s after the settle \
         window (radar destroyed at tick {destroyed_at_tick}; player hull was \
         {player_hull_at_destroy:.1} at that moment, {player_hull_after_settle:.1} after \
         the {settle_secs}s settle window, and {player_hull_final:.1} at the end — hull HP \
         alone is not asserted on here because repair teams restore it independent of \
         whether the hostile is still shooting) — a lock that survived the destruction \
         would keep shooting the remembered target, which is the bug #893 decided to fix",
        player_damage_taken_final - player_damage_taken_after_settle
    );
}

#[test]
#[ignore]
fn scratch_seed_sweep_probe_radar_kill() {
    use project_phoenix::damage::DamageTier;
    use project_phoenix::entity_spawner::EntitySystemHull;
    use project_phoenix::simulation::Ship;
    use project_phoenix::system_registry::tactical_radar_system_id;

    let dt = 1.0 / 30.0;
    let radar_id = tactical_radar_system_id();
    let world = std::env::var("SCRATCH_WORLD")
        .unwrap_or_else(|_| "assets/worlds/probe_radar_kill.toml".into());
    let sim_secs: f64 = std::env::var("SCRATCH_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(90.0);
    let seed_lo: u64 = std::env::var("SCRATCH_SEED_LO")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);
    let seed_hi: u64 = std::env::var("SCRATCH_SEED_HI")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(30);
    let ship_path = std::env::var("SCRATCH_SHIP")
        .unwrap_or_else(|_| "assets/entities/alliance_cruiser.toml".into());
    for seed in seed_lo..=seed_hi {
        let args = HeadlessArgs {
            world_path: world.clone(),
            ship_path: ship_path.clone(),
            dt,
            max_ticks: 0,
            seed: Some(seed),
            deterministic: true,
            ..test_args()
        };
        let mut app = build_headless_app(&args).expect("app should build");
        app.finish();
        app.cleanup();
        let max_ticks = ticks_for_sim_seconds(sim_secs, dt);
        let mut destroyed_at = None;
        let mut game_over_at = None;
        for t in 0..max_ticks {
            app.update();
            let hostile_tier = {
                let mut q = app
                    .world_mut()
                    .query_filtered::<&EntitySystemHull, (With<Ship>, Without<LocalShip>)>();
                q.single(app.world()).map(|h| h.0.tier_for(&radar_id))
            };
            if destroyed_at.is_none() && matches!(hostile_tier, Ok(DamageTier::Destroyed)) {
                destroyed_at = Some(t);
            }
            if app.world().resource::<State<GamePhase>>().get() == &GamePhase::GameOver {
                game_over_at = Some(t);
                break;
            }
        }
        let hostile_cur = {
            let mut hq = app
                .world_mut()
                .query_filtered::<&EntitySystemHull, (With<Ship>, Without<LocalShip>)>();
            hq.single(app.world())
                .map(|x| x.0.total_current())
                .unwrap_or(-1.0)
        };
        let player_cur = {
            let mut pq = app
                .world_mut()
                .query_filtered::<&EntitySystemHull, With<LocalShip>>();
            pq.single(app.world())
                .map(|x| x.0.total_current())
                .unwrap_or(-1.0)
        };
        println!(
            "seed {seed}: destroyed_at={destroyed_at:?} game_over_at={game_over_at:?} \
             hostile_hull_current={hostile_cur:.1} player_hull_current={player_cur:.1}"
        );
    }
}

/// Issue #843: a scenario `game_over` action carrying `outcome = "victory"`
/// classifies the run as a victory, end to end.
///
/// `probe_victory.toml` fires a one-shot timer game-over with a declared victory
/// after 1 s — no combat, so it is a deterministic tripwire for the whole
/// outcome-declaration wiring (config parse → dispatch → `GameOverReason` →
/// classifier), independent of any combat-AI flakiness.
#[test]
fn scenario_declared_victory_classifies_as_victory() {
    let dt = 1.0 / 30.0;
    let args = HeadlessArgs {
        world_path: "assets/worlds/probe_victory.toml".into(),
        dt,
        max_ticks: ticks_for_sim_seconds(5.0, dt),
        deterministic: true,
        ..test_args()
    };
    let mut app = build_headless_app(&args).expect("app should build");
    run(&mut app, args.max_ticks);
    let report = build_report(&mut app, &args, 0.0);

    assert_eq!(
        report.final_phase,
        format!("{:?}", GamePhase::GameOver),
        "the scripted timer should have ended the run in GameOver, got {}",
        report.final_phase
    );
    assert_eq!(
        report.outcome_report.outcome,
        RunOutcome::Victory,
        "a scenario game_over with outcome=\"victory\" must classify as victory"
    );
    // Every report serialises the outcome + sides (AC1).
    let json = report.to_json();
    let parsed: serde_json::Value = serde_json::from_str(&json)
        .unwrap_or_else(|e| panic!("report is not valid JSON: {e}\n{json}"));
    assert_eq!(parsed["outcome"], "victory");
    assert!(parsed["sides"]["player"].is_object());
}

/// Issue #1025: a structure's condition degrades through an authored threshold,
/// flips its operational flag, a script hook reacts, a repair puts the condition
/// back, and the flag flips the other way — all in one real run.
///
/// `probe_infrastructure.toml` is a deliberate tripwire for the whole chain
/// rather than a fight that happens to damage something: one transfer depot,
/// spawned at an overridden 80/100, damaged 50 points at t=1 s and repaired 25
/// at t=3 s. Every link is asserted separately, because "the end state is right"
/// would pass with the flag never having moved at all.
#[test]
fn an_infrastructure_threshold_flips_its_flag_in_both_directions_in_a_real_run() {
    use project_phoenix::entity_spawner::EntityName;
    use project_phoenix::infrastructure::InfrastructureCondition;
    use project_phoenix::world::server::WorldContentRuntime;

    let dt = 1.0 / 30.0;
    let args = HeadlessArgs {
        world_path: "assets/worlds/probe_infrastructure.toml".into(),
        dt,
        max_ticks: ticks_for_sim_seconds(5.0, dt),
        deterministic: true,
        seed: Some(42),
        ..test_args()
    };
    let mut app = build_headless_app(&args).expect("app should build");

    // `first_seen[name]` is the sim-second at which each reading first went true.
    let mut first_seen: std::collections::BTreeMap<&str, f64> = std::collections::BTreeMap::new();
    let mut capable_after_offline: Option<f64> = None;
    // An unset flag reads false, so "not published yet" and "cleared" are the
    // same reading until the depot's first tick. The fall is only counted once
    // the flag has genuinely been up.
    let mut has_been_capable = false;
    for tick in 0..args.max_ticks {
        run(&mut app, 1);
        let sim_t = (tick + 1) as f64 * dt;
        let flags = &app.world().resource::<WorldContentRuntime>().flags;
        let capable = flags.flag("depot_transfer_capable");
        let offline_hook = flags.flag("depot_offline");
        let restored_hook = flags.flag("depot_restored");
        if capable {
            has_been_capable = true;
            first_seen.entry("capable").or_insert(sim_t);
        } else if has_been_capable {
            first_seen.entry("incapable").or_insert(sim_t);
        }
        if offline_hook {
            first_seen.entry("offline_hook").or_insert(sim_t);
            if capable && capable_after_offline.is_none() {
                capable_after_offline = Some(sim_t);
            }
        }
        if restored_hook {
            first_seen.entry("restored_hook").or_insert(sim_t);
        }
    }

    // ── The flag published UP before anything degraded it ──
    let capable_at = *first_seen.get("capable").unwrap_or_else(|| {
        panic!("the depot never published `depot_transfer_capable` at all: {first_seen:?}")
    });
    assert!(
        capable_at < 1.0,
        "an 80/100 depot is above its 40 % threshold, so the flag must be up from the depot's \
         first tick — first seen at {capable_at:.2} s"
    );

    // ── Degradation crossed the threshold and cleared the flag ──
    let incapable_at = *first_seen.get("incapable").unwrap_or_else(|| {
        panic!(
            "`depot_transfer_capable` never fell: 50 points of scripted damage should take an \
             80/100 depot to 30 %, below the authored 40 %. Seen: {first_seen:?}"
        )
    });
    assert!(
        (1.0..2.0).contains(&incapable_at),
        "the flag must fall just after the t=1 s damage, not before it and not much after — \
         fell at {incapable_at:.2} s"
    );

    // ── A script hook reacted to the crossing ──
    let offline_at = *first_seen.get("offline_hook").unwrap_or_else(|| {
        panic!(
            "the world's `on_flag_cleared` handler never ran — the crossing wrote the flag \
             store but never reached the trigger pipeline. Seen: {first_seen:?}"
        )
    });
    assert!(
        offline_at >= incapable_at,
        "the hook cannot fire before the crossing it reacts to ({offline_at:.2} s vs \
         {incapable_at:.2} s)"
    );
    assert!(
        offline_at - incapable_at < 0.5,
        "…and it must fire promptly after it: the crossing rides the same one-tick \
         pending_world_events bridge WaypointReached does, not an open-ended delay \
         ({offline_at:.2} s vs {incapable_at:.2} s)"
    );

    // ── The repair put the flag back, and a second hook reacted ──
    let back_up_at = capable_after_offline.unwrap_or_else(|| {
        panic!(
            "`depot_transfer_capable` never came back: repairing 25 points takes the depot to \
             55 %, above the 45 % restore point. Seen: {first_seen:?}"
        )
    });
    assert!(
        (3.0..4.0).contains(&back_up_at),
        "the flag must return just after the t=3 s repair — returned at {back_up_at:.2} s"
    );
    let restored_at = *first_seen.get("restored_hook").unwrap_or_else(|| {
        panic!(
            "the guarded `on_flag_set` handler never ran. It carries a trigger-level `when` so \
             the depot's opening publication leaves it armed; if this is missing, either the \
             restore never reached the pipeline or the guard spent the trigger early. \
             Seen: {first_seen:?}"
        )
    });
    assert!(
        restored_at >= back_up_at,
        "the restore hook cannot fire before the restore ({restored_at:.2} s vs \
         {back_up_at:.2} s)"
    );

    // ── The authored capacity is readable as a counter, unmoved by any of it ──
    assert_eq!(
        app.world()
            .resource::<WorldContentRuntime>()
            .flags
            .counter("depot_transfer_throughput"),
        40,
        "the depot's authored capacity is a world counter a script predicate can read, and it \
         is a property of the structure rather than of its condition — it must read the same \
         after the depot has been wrecked and patched as it did on arrival"
    );

    // ── And the live component agrees with the arithmetic ──
    let mut q = app
        .world_mut()
        .query::<(&EntityName, &InfrastructureCondition)>();
    let depot = q
        .iter(app.world())
        .find(|(name, _)| name.0 == "world.entity.skyhook_depot.name")
        .map(|(_, condition)| condition.0.clone())
        .expect("the depot is still in the world, carrying its condition track");
    assert_eq!(
        depot.condition(),
        55.0,
        "80 - 50 + 25 = 55, in condition points: the scripted verbs move the authored track \
         and nothing else has touched it"
    );
    assert_eq!(
        depot.capacity("depot_transfer_throughput"),
        Some(40),
        "and a consumer asking the structure directly gets the same authored answer the \
         counter carries"
    );
}

/// Issue #843: a run whose tick budget expires while combat is still live
/// classifies as a timeout, carrying the per-side margins.
///
/// A short `probe_duel` window — the same trading-fire duel, cut off at 5 s
/// before either side dies — leaves the phase `InProgress` with damage still
/// flowing in the closing window, which is exactly the timeout signal.
#[test]
fn budget_exhausted_mid_fight_classifies_as_timeout() {
    let dt = 1.0 / 30.0;
    let args = HeadlessArgs {
        world_path: "assets/worlds/probe_duel.toml".into(),
        dt,
        max_ticks: ticks_for_sim_seconds(5.0, dt),
        deterministic: true,
        ..test_args()
    };
    let mut app = build_headless_app(&args).expect("app should build");
    run(&mut app, args.max_ticks);
    let report = build_report(&mut app, &args, 0.0);

    assert_eq!(
        report.final_phase,
        format!("{:?}", GamePhase::InProgress),
        "a 5 s cutoff of a ~30 s duel should not have resolved: {}",
        report.final_phase
    );
    assert_eq!(
        report.outcome_report.outcome,
        RunOutcome::Timeout,
        "budget exhausted with combat still live is a timeout, not a draw: {:?}",
        report.outcome_report
    );
    // Both sides carry margins (AC2): the duel is two-sided, so at least one
    // side was still landing damage in the closing window.
    let closing = report.outcome_report.player.closing_damage_rate
        + report.outcome_report.enemy.closing_damage_rate;
    assert!(
        closing > 0.0,
        "a live timeout must show closing-window damage: {:?}",
        report.outcome_report
    );
}

/// Issue #838 double-despawn: a destroyed entity must emit `EntityDespawned`
/// exactly ONCE — never a second time from the reconcile sweep.
///
/// Two emitters used to fire for one kill: the weapon kill site
/// (`tick_beams_apply_damage` and siblings) despawned the entity and pushed
/// `EntityDespawned`, then the reconcile sweep (`reconcile_runtime_entities`)
/// pushed a *second* one because the kill site never cleared the uuid from the
/// `TrackedEntities` registry. The fix has every kill site call
/// `TrackedEntities::forget`, so the sweep no longer re-emits.
///
/// `probe_despawn.toml` produces exactly one kill (a battleship destroys a
/// courier, far from the uninvolved player and OUTSIDE its LOD bubble so the duel
/// stays low-LOD and single-weapon — see that world's own note), so the count of
/// `EntityDespawned` over the whole run is a direct assertion of the invariant:
/// one before the bug's second emitter, two with it.
#[test]
fn a_destroyed_entity_emits_entity_despawned_exactly_once() {
    let dt = 1.0 / 30.0;
    let args = HeadlessArgs {
        world_path: "assets/worlds/probe_despawn.toml".into(),
        dt,
        max_ticks: ticks_for_sim_seconds(90.0, dt),
        deterministic: true,
        ..test_args()
    };
    let mut app = build_headless_app(&args).expect("app should build");
    run(&mut app, args.max_ticks);
    let report = build_report(&mut app, &args, 0.0);

    let despawns = report
        .message_counts
        .get("EntityDespawned")
        .copied()
        .unwrap_or(0);
    assert_eq!(
        despawns, 1,
        "exactly one entity was destroyed, so exactly one EntityDespawned must be \
         broadcast — got {despawns}. message_counts={:?}",
        report.message_counts
    );
}

/// Issue #839 wiring guard: the production player game-start spawn path
/// (`spawn_game_start_entities`) must inject the `player` tag and the
/// `playerShip` radar icon onto the LocalShip — the hull the local player
/// actually flies — and onto nothing else. The `player_ship_identity` unit
/// test proves the helper's *output*; this proves the spawn site actually
/// *calls* it.
///
/// This boots the real headless world and reads the spawned entities, so it is
/// the only test that fails if the injection block in
/// `spawn_game_start_entities` is deleted. Without the injection the player
/// ship spawns as a plain "ship": the native radar then draws it twice and
/// every `player`-only filter misses it. A world-spawned NPC hull in the same
/// world (the patrol raider) must stay a plain ship — the identity is scoped to
/// the player, never authored in a template that NPC copies also read.
#[test]
fn player_game_start_spawn_injects_player_identity_onto_local_ship_only() {
    use project_phoenix::entity_spawner::{EntityTagsSection, RadarAppearanceSection};

    let args = test_args();
    let mut app = build_headless_app(&args).expect("app should build");
    run(&mut app, args.max_ticks);

    // The LocalShip is the player's own ship: it must carry the injected
    // player identity (player tag + playerShip icon), keeping the ship tag.
    let mut local = app
        .world_mut()
        .query_filtered::<(&EntityTagsSection, &RadarAppearanceSection), With<LocalShip>>();
    let (tags, radar) = local
        .single(app.world())
        .expect("exactly one LocalShip should exist");
    assert!(
        tags.0.iter().any(|t| t == "ship"),
        "player ship keeps the ship tag; got {:?}",
        tags.0
    );
    assert!(
        tags.0.iter().any(|t| t == "player"),
        "player ship must carry the injected `player` tag — is the injection \
         block still present in `spawn_game_start_entities`? got {:?}",
        tags.0
    );
    assert_eq!(
        radar.0.icon.as_deref(),
        Some("playerShip"),
        "player ship must carry the injected `playerShip` radar icon"
    );

    // A world-spawned NPC hull (the patrol raider) is not the player: the
    // injection is scoped to the LocalShip, so every other hull stays a plain
    // ship with no player tag.
    let mut others = app
        .world_mut()
        .query_filtered::<&EntityTagsSection, Without<LocalShip>>();
    let mut saw_npc_ship = false;
    for tags in others.iter(app.world()) {
        if tags.0.iter().any(|t| t == "ship") {
            saw_npc_ship = true;
            assert!(
                !tags.0.iter().any(|t| t == "player"),
                "a world-spawned NPC hull must NOT carry the player tag; got {:?}",
                tags.0
            );
        }
    }
    assert!(
        saw_npc_ship,
        "expected at least one world-spawned NPC ship in the patrol world"
    );
}

/// Issue #840: the balance-diagnosis-through-logging workflow needs the
/// `plog!` call sites in the balance-relevant categories to actually be
/// *reached* during a real fight. The confirmed root cause of #840 was not
/// broken emission — the parser, gate, subscriber and `EnvFilter` all lined
/// up — but the near-total *absence of call sites*: `ai`/`power` had none,
/// `weapons` had a single trace. #840 added the load-bearing ones (target
/// changes, opened/ceased fire, power energize/brownout, damage/destruction).
///
/// This test cannot assert on the emitted text — under `cargo test` no
/// `tracing` subscriber is installed, so every event short-circuits before it
/// reaches a writer and a capture-based assertion would pass vacuously (see the
/// note in `logging::macros`). What it *can* prove is that the systems now
/// carrying those `plog!` calls run to completion with a real, category-enabled
/// `LogFilterConfig` inserted (the gate branch is evaluated, not skipped), and
/// that the decisions the lines narrate actually occur: a target is acquired,
/// beams fire, and the duel resolves in a destruction. If a future change moved
/// a `plog!` into a system that panics under a populated config, or dropped the
/// systems from the schedule, this fails where the pure logging tests would
/// not. The live acceptance is a manual `--log ai=info,...` run; this is the
/// cheap automated guard that the code paths are reachable.
#[test]
fn balance_logging_systems_run_with_an_enabled_filter_and_the_duel_resolves() {
    use project_phoenix::logging::parse_log_spec;

    let dt = 1.0 / 30.0;
    let mut args = HeadlessArgs {
        world_path: "assets/worlds/probe_duel.toml".into(),
        dt,
        // Window re-blessed for issue #907's review, same reason as
        // `world_spawned_alliance_hull_returns_fire_and_the_duel_resolves`
        // above (was 60 s): the game-start writer moving into `FixedUpdate`
        // shifts this combat-chaotic duel's RNG draws by one tick, and it now
        // settles later than the old window allowed. Confirmed still
        // resolving in a kill by 180 s.
        max_ticks: ticks_for_sim_seconds(180.0, dt),
        deterministic: true,
        // Re-blessed for issue #896, same pin and same reason as
        // `world_spawned_alliance_hull_returns_fire_and_the_duel_resolves`
        // above: the "destroyed by" site this test needs reached only fires in
        // a duel that resolves, and on the new physics clock seed 34 is the one
        // that does inside the (now 180 s, issue #907) budget.
        seed: Some(34),
        ..test_args()
    };
    // The four categories the balancer reaches for, each enabled — exactly what
    // the CLI builds from `--log ai=info,weapons=info,power=info,damage=info`.
    // With this present, every `plog!` at those sites evaluates its gate against
    // a real config rather than the warn-level fallback.
    args.log = parse_log_spec("ai=info,weapons=info,power=info,damage=info")
        .expect("log spec should parse");

    let mut app = build_headless_app(&args).expect("app should build");
    run(&mut app, args.max_ticks);
    let report = build_report(&mut app, &args, 0.0);

    // The decisions the new lines narrate all happened: both duelists dealt
    // damage (targets were acquired and beams fired — the `ai` and `weapons`
    // sites), and the fight ended in a kill (the `damage` "destroyed by" site).
    let dealt: f32 = report.damage_by_ship.values().map(|l| l.damage_dealt).sum();
    assert!(
        dealt > 0.0,
        "no damage dealt — target-acquisition/weapons-fire paths (the ai/weapons \
         plog sites) were not reached: {:?}",
        report.damage_by_ship
    );
    assert_eq!(
        report.final_phase,
        format!("{:?}", GamePhase::GameOver),
        "the duel must resolve in a destruction (the damage `destroyed by` site), \
         ended in phase {}",
        report.final_phase
    );
}

/// Issue #842 AC2: a backfilled player hull acts on its own TEMPLATE doctrine —
/// it proactively engages a hostile with NO scenario objective in play.
///
/// `probe_aggressor.toml` places the player cruiser next to a passive hostile
/// destroyer whose doctrine was overridden to empty (`behaviour.doctrine = []`).
/// With no Destroy directive, that hull can only ever acquire a target through
/// the `last-attacker` tier (`ai_target_selection`), so it can never fire the
/// first shot. The only ship that can open fire is the player — and it can only
/// do so off the default `[behaviour]` doctrine now authored on the cruiser
/// template (#842), because the world declares no `add_objective`. So the
/// player's ledger row showing `damage_dealt > 0` is proof of proactive,
/// doctrine-driven engagement, not reactive return fire.
///
/// The player is identified by ledger `name_id`, not by a post-run `LocalShip`
/// query: a proactive player that gets the first shot in also draws the passive
/// destroyer's (far heavier) return fire, and a killed player is despawned by
/// run's end. The ledger, built from the balance-event log, keeps its row
/// regardless — the player keeps its template name (`entity.alliance_cruiser.name`)
/// while the world-spawned hostile is named `probe_target`.
///
/// # Why `damage_dealt > 0` is not enough on its own
///
/// That assertion holds whether or not the world's `doctrine = []` override
/// actually strips the destroyer's doctrine: the player engages either way, and
/// for a long time the override was a silent no-op (the by-`id` doctrine merge
/// treated an
/// empty array as "merge nothing in"), so the "passive" hostile carried its
/// template `destroy-hostiles` Destroy directive, out-shot the player 318 damage
/// to 29 and killed it. The test passed the whole time it was measuring nothing.
///
/// Nor does "who fired first" discriminate: the player wins that race either
/// way — it just loses the fight. So the probe's *control condition* is checked
/// directly instead. The spawned hostile must carry no doctrine at all (the
/// override applied), and the ship with combat doctrine must be the one doing
/// the damage.
#[test]
fn backfilled_player_hull_proactively_engages_on_template_doctrine() {
    use project_phoenix::entity_spawner::{BehaviourSection, EntityName};

    let dt = 1.0 / 30.0;
    let args = HeadlessArgs {
        world_path: "assets/worlds/probe_aggressor.toml".into(),
        dt,
        max_ticks: ticks_for_sim_seconds(30.0, dt),
        deterministic: true,
        ..test_args()
    };
    let mut app = build_headless_app(&args).expect("app should build");
    run(&mut app, args.max_ticks);

    // Control condition: the world spawned the hostile with `doctrine = []`, so
    // the hull it actually flies must carry no standing doctrine — no Destroy
    // directive, nothing to license an unprovoked engagement. Checked on the
    // live entity because that is where a merge that quietly kept the template's
    // list would show up.
    let mut hostile_q = app
        .world_mut()
        .query::<(&EntityName, Option<&BehaviourSection>)>();
    let hostile_doctrine_ids: Vec<String> = hostile_q
        .iter(app.world())
        .find(|(name, _)| name.0 == "probe_target")
        .map(|(_, behaviour)| {
            behaviour
                .map(|b| b.0.doctrine.iter().map(|d| d.id.clone()).collect())
                .unwrap_or_default()
        })
        .expect("the world spawns a hostile named probe_target");
    assert!(
        hostile_doctrine_ids.is_empty(),
        "probe_target kept doctrine {hostile_doctrine_ids:?} — the world's \
         `doctrine = []` override did not strip the template's list, so this \
         probe is not measuring proactive engagement at all"
    );

    let report = build_report(&mut app, &args, 0.0);

    let player = report
        .damage_by_ship
        .values()
        .find(|l| l.name_id.as_deref() != Some("probe_target"))
        .unwrap_or_else(|| {
            panic!(
                "no player ledger row — the player never engaged a soul: {:?}",
                report.damage_by_ship
            )
        });
    assert_eq!(
        player.name_id.as_deref(),
        Some("entity.alliance_cruiser.name"),
        "the non-hostile ledger row should be the player cruiser: {:?}",
        report.damage_by_ship
    );
    assert!(
        player.damage_dealt > 0.0,
        "the backfilled player dealt no damage; it did not proactively engage off \
         its template doctrine (the passive hostile cannot fire first): {:?}",
        report.damage_by_ship
    );

    // And the engagement is one-sided in the player's favour: doctrine is the
    // only asymmetry between these two hulls, so a doctrine-less hostile that
    // out-damages the player means the override leaked its Destroy directive
    // back in.
    let hostile_dealt = report
        .damage_by_ship
        .values()
        .find(|l| l.name_id.as_deref() == Some("probe_target"))
        .map(|l| l.damage_dealt)
        .unwrap_or(0.0);
    assert!(
        player.damage_dealt > hostile_dealt,
        "the doctrine-less hostile dealt {hostile_dealt} against the player's {}: \
         the passive hull is fighting back harder than the aggressor, which is \
         what a leaked template Destroy doctrine looks like",
        player.damage_dealt
    );
}

/// Issue #842 AC4: a same-hull duel between a backfilled player cruiser and a
/// world-spawned copy of the same hull is behaviourally symmetric — both sides
/// meaningfully engage and deal comparable damage across seeds.
///
/// `probe_symmetry.toml` overrides only the copy's faction, so both hulls run
/// the identical template `[behaviour]` doctrine (#842): the player through the
/// game-start spawn path, the copy through `spawn_entity`. "Comparable" is a
/// same-order-of-magnitude check, not equality — initial positions and first
/// shot differ, so exact parity is neither expected nor asserted.
#[test]
fn same_hull_duel_is_behaviourally_symmetric_across_seeds() {
    let dt = 1.0 / 30.0;
    for seed in [1_u64, 842, 20260] {
        let args = HeadlessArgs {
            world_path: "assets/worlds/probe_symmetry.toml".into(),
            dt,
            max_ticks: ticks_for_sim_seconds(45.0, dt),
            seed: Some(seed),
            deterministic: true,
            ..test_args()
        };
        let mut app = build_headless_app(&args).expect("app should build");
        run(&mut app, args.max_ticks);
        let report = build_report(&mut app, &args, 0.0);

        assert_eq!(
            report.damage_by_ship.len(),
            2,
            "seed {seed}: expected both duelists in the ledger, got {:?}",
            report.damage_by_ship
        );
        let dealt: Vec<f32> = report
            .damage_by_ship
            .values()
            .map(|l| l.damage_dealt)
            .collect();
        for d in &dealt {
            assert!(
                *d > 0.0,
                "seed {seed}: a duelist dealt no damage — not symmetric: {:?}",
                report.damage_by_ship
            );
        }
        let lo = dealt.iter().copied().fold(f32::INFINITY, f32::min);
        let hi = dealt.iter().copied().fold(0.0_f32, f32::max);
        assert!(
            hi <= lo * 10.0,
            "seed {seed}: damage dealt is lopsided ({lo} vs {hi}) — not within one \
             order of magnitude: {:?}",
            report.damage_by_ship
        );
    }
}

/// A shipped hull's only goal must actually resolve: the Requiem Courier flies
/// its `Reach` directive to the anchor the scenario gives it.
///
/// `ship_requiem_courier.toml` is a non-combat hull whose entire behaviour is
/// one `Reach` objective. It authored `directive_anchors` — the **Patrol**
/// field — on that directive; `Reach` reads the singular `directive_anchor`, so
/// the anchor resolved to `""`, `anchors.get("")` missed, the objective produced
/// no travel decision, and the courier sat on its spawn anchor for the whole
/// scenario. Nothing failed and nothing logged.
///
/// This boots `probe_reach_anchor.toml` — the dedicated test-infra world this
/// regression was re-homed onto when the `before_the_fire` world tree was
/// retired — so it still covers the whole chain the bug ran through: the
/// template's directive fields, the world's anchor table, `plan_helm_travel`'s
/// `Reach` arm, and the per-axis helm actuators. The probe carries the courier's
/// spawn and destination anchors over unchanged, so the leg and timings the
/// assertions below are tuned against are identical.
#[test]
fn requiem_courier_reaches_its_destination_anchor() {
    use project_phoenix::entity_spawner::EntityName;

    let dt = 1.0 / 30.0;
    let args = HeadlessArgs {
        world_path: "assets/worlds/probe_reach_anchor.toml".into(),
        dt,
        max_ticks: ticks_for_sim_seconds(90.0, dt),
        deterministic: true,
        ..test_args()
    };

    let mut app = build_headless_app(&args).expect("app should build");
    run(&mut app, args.max_ticks);

    // The destination the world authors for the courier, and its spawn anchor —
    // read from the `WorldConfig` the app actually loaded (the same resource
    // `simulate_low_lod_ships` reads) rather than copied out of the TOML. A
    // designer retuning either anchor would otherwise fail this test with "its
    // only goal did not resolve", which is the one thing that would not have
    // happened.
    let (destination, spawn) = {
        let world_config = app
            .world()
            .resource::<project_phoenix::world::config::WorldConfig>();
        let anchor_xz = |name: &str| {
            let a = world_config.anchors.get(name).unwrap_or_else(|| {
                panic!("probe_reach_anchor.toml must declare the `{name}` anchor")
            });
            [a[0], a[2]]
        };
        (
            anchor_xz("requiem_courier_destination"),
            anchor_xz("requiem_courier"),
        )
    };
    let start_distance =
        ((destination[0] - spawn[0]).powi(2) + (destination[1] - spawn[1]).powi(2)).sqrt();

    let mut q = app.world_mut().query::<(&EntityName, &ShipPhysics)>();
    let (x, z, forward_speed) = q
        .iter(app.world())
        .find(|(name, _)| name.0 == "entity.ship_requiem_courier.name")
        .map(|(_, physics)| (physics.x, physics.z, physics.forward_speed))
        .expect("probe_reach_anchor.toml spawns the Requiem Courier");

    let distance = ((destination[0] - x).powi(2) + (destination[1] - z).powi(2)).sqrt();
    assert!(
        distance < start_distance,
        "the courier has not moved toward its destination at all (still {distance:.1} u \
         away, started {start_distance:.1} u away, now at [{x:.1}, {z:.1}]): its only \
         goal did not resolve"
    );
    // Arrival is judged against the authored `waypoint_arrival_radius` (20 u by
    // default), so it stops just inside that ring rather than on the anchor.
    assert!(
        distance < 25.0,
        "the courier ended {distance:.1} u from its destination at [{x:.1}, {z:.1}] — \
         it neither arrived nor is holding there"
    );
    // And it stays: a completed route is an arrival, not an absence of one. Its
    // predecessor sailed straight through at cruise speed and kept going.
    assert!(
        forward_speed.abs() < 0.1,
        "the courier is still making {forward_speed:.2} u/s at its destination — it is \
         drifting past instead of holding station"
    );
}

/// Issue #842 regression guard: `combat_test.toml` — a shipped defence
/// scenario — must still develop *two-sided* ship combat and resolve.
///
/// This is the tripwire whose absence let #842's clobber hide. Adding a default
/// `[behaviour]` doctrine to the player hull gave the game-start ship both
/// `LocalShip` and `BehaviourSection`, so two systems wrote the same viewscreen
/// blackboard with no defined order; the doctrine writer won and ERASED the
/// player's scenario objectives. `combat_test` then stopped developing combat
/// (player idle at full hull, zero beams, no kills) yet still "passed" every
/// other test because none asserted combat_test itself resolved. The fix merges
/// the two objective pools (see `publish_viewscreen_blackboard`); this test
/// nails combat_test down so a future reintroduced clobber fails loudly here.
///
/// The player is the cruiser (`test_args` ship_path), identified in the ledger
/// by its template `name_id` — the ledger is built from the balance-event log,
/// so the row survives even if the player is destroyed. Asserting the player
/// both `damage_dealt > 0` AND `damage_taken > 0` is what makes this a
/// *two-sided* check: a one-sided run (the bug) shows neither. AC3 rides along —
/// a player that deals damage in combat_test is pursuing the scenario's @80
/// `Destroy wave_N` objectives, not roaming on its @45 template doctrine (the
/// passive-hull probe worlds cover the pure-doctrine case separately).
///
/// Seeded so the guard samples one fixed point of the scenario rather than a
/// fresh one every time. `deterministic` alone pins the scheduler but not the
/// RNG, and `combat_test.toml` authors a `[global] seed`, so before this the
/// run was only as stable as that authored value — retuning it silently
/// re-rolled the guard.
///
/// RE-MEASURED for #960 + #936, which changed what this scenario IS: waves
/// arrive on a clock instead of on the previous wave's death, and every wave now
/// actually flies the starbase assault (the override named the display text
/// "Starbase Alpha" where name resolution wants the string id, so no wave had
/// ever run at the station). Seed 9 now resolves at 175.9 s (tick 10555) as a
/// DEFEAT — and a different defeat from the old one: the player cruiser survives
/// on 70.6/500 hull, and it is STARBASE ALPHA that dies, taking all 800 of its
/// hull. Player `damage_dealt` 106.0 and `damage_taken` 677.3 against the `> 0`
/// thresholds below; one kill in the ledger, wave 4's, on the station. The 400 s
/// budget clears the 175.9 s resolution with room and is left alone.
///
/// The two picket assertions this test used to carry are gone with the picket
/// line (#960). What replaces them is stronger and is the reason #936 exists:
/// the STARBASE's own ledger row must show damage taken. That row can only
/// exist if a Harrow resolved a named Destroy directive onto a station 700 units
/// from its spawn anchor, flew the `close-on-starbase` run-in to get inside its
/// 200-unit acquisition band, and opened fire on it — i.e. the whole assault
/// path, end to end, in a real run. Before this batch that row was never
/// present at any seed.
///
/// BUDGET RAISED 400s -> 600s for #1003 (owner-approved 2026-08-13). #1003's
/// AI power-shed/restore floors (50/25 shed, 60/35 restore) deliberately make
/// the battery exhaustion lock unreachable under AI power — that was a
/// fight-ending failure mode, and removing it means a seeded combat_test run
/// legitimately takes longer to reach GameOver than the 175.9 s this test
/// measured pre-#1003. 600 s was re-derived against the floored AI and leaves
/// headroom the same way the old 400 s did against the old 175.9 s result.
///
/// WHAT THIS TEST DOES NOT PIN. The CRUISER is not a good defender on AI
/// backfill — it dealt 106 damage in the whole run — so no run *here* clears the
/// raid. That is a statement about the AI-backfilled cruiser, not about the
/// schedule. The schedule is pinned twice over elsewhere:
/// `combat_test_spawns_its_waves_on_the_clock_in_a_real_run` below flies the
/// demo destroyer and reads the wave counter against the authored times, and
/// `world::content::tests::
/// combat_test_wave_clock_releases_eight_waves_on_schedule_then_victory` drives
/// all eight waves through the real evaluator with a scripted player.
#[test]
fn combat_test_develops_two_sided_combat_and_resolves() {
    let dt = 1.0 / 30.0;
    let args = HeadlessArgs {
        world_path: "assets/worlds/combat_test.toml".into(),
        dt,
        max_ticks: ticks_for_sim_seconds(600.0, dt),
        seed: Some(9),
        deterministic: true,
        ..test_args()
    };
    let mut app = build_headless_app(&args).expect("app should build");
    run(&mut app, args.max_ticks);
    let report = build_report(&mut app, &args, 0.0);

    assert_eq!(
        report.final_phase,
        format!("{:?}", GamePhase::GameOver),
        "combat_test did not resolve within 600s — final_phase {:?}, ship {:?}",
        report.final_phase,
        report.ship
    );

    let player = report
        .damage_by_ship
        .values()
        .find(|l| l.name_id.as_deref() == Some("entity.alliance_cruiser.name"))
        .unwrap_or_else(|| {
            panic!(
                "no player cruiser ledger row — the player never engaged: {:?}",
                report.damage_by_ship
            )
        });
    assert!(
        player.damage_dealt > 0.0,
        "the player dealt no damage — combat_test is not developing ship combat \
         (the #842 clobber dropped the player's scenario Destroy objectives): {:?}",
        report.damage_by_ship
    );
    assert!(
        player.damage_taken > 0.0,
        "the player took no damage — combat is one-sided, not a real exchange: {:?}",
        report.damage_by_ship
    );

    let kills: u64 = report.damage_by_ship.values().map(|l| l.kills).sum();
    assert!(
        kills > 0,
        "no kills across the whole run — combat never resolved to a destruction: {:?}",
        report.damage_by_ship
    );

    // A run that reached GameOver is a terminal victory or defeat, never a
    // draw/timeout (those come only from budget exhaustion) — #843. Which one
    // is seed-dependent (the scenario's victory triggers, the starbase-destroyed
    // defeat trigger and the player-death latch all feed the flag), so this
    // stays deliberately outcome-agnostic.
    assert!(
        matches!(
            report.outcome_report.outcome,
            RunOutcome::Victory | RunOutcome::Defeat
        ),
        "a resolved combat_test run must classify as victory or defeat, got {:?}",
        report.outcome_report.outcome
    );

    // #960/#936: the raid actually assaults the thing the scenario is about.
    // Wave 1 is the first ship on the clock; the starbase row is the assault.
    let ledger_names: Vec<&str> = report
        .damage_by_ship
        .values()
        .filter_map(|l| l.name_id.as_deref())
        .collect();
    assert!(
        ledger_names.contains(&"wave_1"),
        "wave_1 never engaged in a real run — got {ledger_names:?}"
    );
    let (starbase_uuid, starbase) = report
        .damage_by_ship
        .iter()
        .find(|(_, l)| l.name_id.as_deref() == Some("world.entity.starbase_alpha.name"))
        .unwrap_or_else(|| {
            panic!(
                "Starbase Alpha has no ledger row, so nothing ever shot at it. \
                 Every wave carries an `assault-starbase` Destroy override and \
                 the station is the player's own faction (#936) — if no Harrow \
                 engaged it, either the directive resolved to no entity or the \
                 run-in never got one inside its acquisition band. \
                 Ledger: {:?}",
                report.damage_by_ship
            )
        });
    assert!(
        starbase.damage_taken > 0.0,
        "Starbase Alpha took no damage — the assault this scenario is named for \
         did not happen: {starbase:?}"
    );

    // …and a HARROW dealt it. `damage_taken` alone cannot say that: collisions
    // emit `VictimKind::Ship` too, and the player's own `obj-defend` circuit
    // flies a 200-unit ring around this station, so a backfilled player that
    // merely clipped it would satisfy the assertion above while the message
    // claimed a raid. `by_pair` is keyed by victim, so a raider's row naming
    // the starbase is the assault stated as the thing it is.
    let assaulters: Vec<(&str, f32)> = report
        .damage_by_ship
        .values()
        .filter(|l| l.name_id.as_deref().is_some_and(|n| n.starts_with("wave_")))
        .filter_map(|l| {
            l.by_pair
                .get(starbase_uuid)
                .filter(|dealt| **dealt > 0.0)
                .map(|dealt| (l.name_id.as_deref().unwrap_or_default(), *dealt))
        })
        .collect();
    assert!(
        !assaulters.is_empty(),
        "Starbase Alpha took damage, but no wave's ledger credits a single point \
         of it — so what hit the station was the player's own defensive circuit \
         colliding with it, not the raid. Ledger: {:?}",
        report.damage_by_ship
    );
}

/// Issue #960: the wave CLOCK actually runs in a real sim.
///
/// This replaces the death-gated chain guard. That one asserted wave_2 could
/// only exist if wave_1's group had gone empty; there is no such implication any
/// more, and asserting the same names would now prove nothing about pacing at
/// all — a wave arrives whether or not anything died.
///
/// So this reads the schedule directly. `waves_spawned` is the world counter
/// each wave's own timer trigger increments, and the test records the sim-second
/// at which it first reaches each value, then compares that against the times
/// authored in `combat_test.toml`. Two things follow that the old test could not
/// say:
///
///   1. Waves land on the authored clock, not on a breather after a kill. A
///      re-introduced death-gate would still produce the right ORDER and the
///      right count — only the times give it away.
///   2. Waves OVERLAP. The run is sampled for a moment at which two different
///      wave groups both have a living ship, which the death-gated chain could
///      not produce by construction: it released wave N+1 only once wave N was
///      empty. This is the pacing change stated as an observable.
///
/// The DESTROYER, not the cruiser `test_args` defaults to: it is the demo hull
/// (`assets/scenarios.demo.toml`), and it sits below both `ship_power` bonus
/// gates so the run is exactly the authored eight-wave table with no extra
/// hulls to confuse the "two waves alive" sample.
///
/// The budget is 300 s — short of wave 8 at t=315 — deliberately. On seed 2 the
/// starbase falls at ~314 s, and a test that asserts on trigger dispatch has no
/// business straddling the tick the mission ends on. Waves 1-7 are the whole
/// authored cadence apart from its last entry, and the eighth link is covered by
/// the pure evaluator test in `world::content`.
#[test]
fn combat_test_spawns_its_waves_on_the_clock_in_a_real_run() {
    use project_phoenix::entity_spawner::EntityName;
    use project_phoenix::world::server::WorldContentRuntime;

    let dt = 1.0 / 30.0;
    let args = HeadlessArgs {
        world_path: "assets/worlds/combat_test.toml".into(),
        ship_path: "assets/entities/alliance_destroyer.toml".into(),
        dt,
        max_ticks: ticks_for_sim_seconds(300.0, dt),
        seed: Some(2),
        deterministic: true,
        ..test_args()
    };
    let mut app = build_headless_app(&args).expect("app should build");

    // `first_seen[n]` is the sim-second at which `waves_spawned` first read n.
    let mut first_seen: std::collections::BTreeMap<i64, f64> = std::collections::BTreeMap::new();
    let mut overlapped = false;
    for tick in 0..args.max_ticks {
        run(&mut app, 1);
        let sim_t = (tick + 1) as f64 * dt;
        let spawned = app
            .world()
            .resource::<WorldContentRuntime>()
            .flags
            .counter("waves_spawned");
        first_seen.entry(spawned).or_insert(sim_t);

        if !overlapped {
            // Distinct wave GROUPS with a living ship. Names are the authored
            // `spawn_entity` names, so `wave_5_second` folds into `wave_5` and a
            // two-hull wave is not mistaken for two waves.
            let mut q = app.world_mut().query::<&EntityName>();
            let live: std::collections::BTreeSet<String> = q
                .iter(app.world())
                .filter_map(|n| n.0.strip_prefix("wave_"))
                .filter_map(|rest| rest.split('_').next())
                .map(|n| n.to_string())
                .collect();
            if live.len() >= 2 {
                overlapped = true;
            }
        }
    }

    // The authored cadence: wave N at (N-1) x 45 s. Each reading is taken on
    // the tick after the trigger dispatched, so a one-tick lag is expected; the
    // tolerance is a second, which is far tighter than the 45 s interval and far
    // looser than the dispatch jitter.
    for (wave, authored) in (1..=7).zip([0.0, 45.0, 90.0, 135.0, 180.0, 225.0, 270.0]) {
        let at = first_seen.get(&wave).copied().unwrap_or_else(|| {
            panic!(
                "waves_spawned never reached {wave} inside the 300 s budget — \
                 the clock is not releasing waves. Seen: {first_seen:?}"
            )
        });
        assert!(
            (at - authored).abs() < 1.0,
            "wave {wave} was released at {at:.2} s, but combat_test.toml authors \
             it at {authored} s. A wave that arrives late is a wave still gated \
             on something dying. All: {first_seen:?}"
        );
    }

    assert!(
        overlapped,
        "no two waves were ever alive at once in a 300 s run. Under a clock a \
         wave arrives whether or not the last one is dead, so overlap is the \
         observable that separates a timed schedule from a death-gated chain — \
         its absence means the waves are still queueing behind each other"
    );

    // And they engaged rather than merely existing: the first three waves all
    // reach the damage ledger, which is built from the balance-event log.
    let report = build_report(&mut app, &args, 0.0);
    let ledger_names: Vec<&str> = report
        .damage_by_ship
        .values()
        .filter_map(|l| l.name_id.as_deref())
        .collect();
    for expected in ["wave_1", "wave_2", "wave_3"] {
        assert!(
            ledger_names.contains(&expected),
            "{expected} arrived on the clock but never fought anything. \
             Engaged: {ledger_names:?}"
        );
    }
}

/// Issue #960: `after_secs` is an offset from MISSION START, not from app boot.
///
/// The sibling test above starts the mission on frame one, and so does every
/// other automated driver of this scenario: headless auto-starts with nobody
/// connected, which is precisely why the boot anchor survived from #475 to here
/// without a single test noticing. The two anchors only disagree once something
/// sits between loading the world and starting the mission — a lobby.
///
/// So this run supplies one. The app is parked in `Loading` for 90 simulated
/// seconds before the mission starts. `Loading` is the phase a browser host sits
/// in while assets stream, `headless_auto_start` only fires from `Lobby`, and
/// headless registers no asset preloader (`SimPluginOptions::render` is false),
/// so nothing leaves that phase until this test writes `NextState` — from
/// outside the fixed schedule, which is also how `auto_transition_from_loading`
/// does it in the browser. The `SimSet` chain is gated on `InProgress`, so no
/// trigger is evaluated for those 90 seconds; `Time<Virtual>` and `Time<Fixed>`
/// advance through them regardless, and that gap is the whole bug.
///
/// Two observables, and the first is the one that fails against the boot anchor:
///
///   1. The first mission tick releases wave 1 and NOTHING ELSE. Anchored at
///      boot, that tick reads `elapsed_secs = 90`, which satisfies the triggers
///      authored at 0, 45 AND 90 in one dispatch batch — three waves and four
///      comms bursts landing together, `waves_spawned = 3`. At a five-minute
///      lobby the whole eight-wave raid arrives on tick one.
///   2. Wave 2 still arrives 45 s later, measured from the START, not from boot.
///      Without this the fix could "pass" by never firing anything at all.
///
/// The budget stops well short of wave 3 (t=90): what is under test is the
/// origin of the clock, and the full cadence is the sibling test's job.
#[test]
fn combat_test_wave_clock_measures_from_mission_start_not_app_boot() {
    use project_phoenix::world::server::WorldContentRuntime;

    /// Simulated seconds spent waiting to start. Chosen to sit exactly on the
    /// authored time of wave 3, so a boot-anchored run trips two thresholds
    /// rather than one.
    const LOBBY_SECS: f64 = 90.0;
    /// Mission seconds to fly after the start. Long enough to cover wave 2's
    /// authored t=45 with room to prove it was not early.
    const MISSION_SECS: f64 = 60.0;

    let dt = 1.0 / 30.0;
    let args = HeadlessArgs {
        world_path: "assets/worlds/combat_test.toml".into(),
        ship_path: "assets/entities/alliance_destroyer.toml".into(),
        dt,
        max_ticks: ticks_for_sim_seconds(LOBBY_SECS + MISSION_SECS, dt),
        seed: Some(2),
        deterministic: true,
        ..test_args()
    };
    let mut app = build_headless_app(&args).expect("app should build");

    let waves_spawned = |app: &App| {
        app.world()
            .resource::<WorldContentRuntime>()
            .flags
            .counter("waves_spawned")
    };

    // Park the app short of the mission, and burn the lobby.
    app.world_mut()
        .insert_resource(State::new(GamePhase::Loading));
    run(&mut app, ticks_for_sim_seconds(LOBBY_SECS, dt));

    assert_eq!(
        app.world().resource::<State<GamePhase>>().get(),
        &GamePhase::Loading,
        "precondition: nothing in a headless app may leave `Loading` on its own \
         - if something does, this test is no longer holding a lobby open and \
         proves nothing"
    );
    assert_eq!(
        waves_spawned(&app),
        0,
        "precondition: the `SimSet` chain is gated on `InProgress`, so no wave \
         may spawn before the mission starts"
    );

    // Start the mission the way the browser host does: `NextState` written
    // outside the fixed schedule, applied at the frame-level `StateTransition`.
    app.world_mut()
        .resource_mut::<NextState<GamePhase>>()
        .set(GamePhase::InProgress);

    // `first_seen[n]` is the MISSION-second at which `waves_spawned` first read
    // n. The first frame crosses the phase boundary, so mission time starts
    // there.
    let mut first_seen: std::collections::BTreeMap<i64, f64> = std::collections::BTreeMap::new();
    let mission_frames = ticks_for_sim_seconds(MISSION_SECS, dt);
    for frame in 0..mission_frames {
        run(&mut app, 1);
        first_seen
            .entry(waves_spawned(&app))
            .or_insert((frame + 1) as f64 * dt);

        if frame == 0 {
            assert_eq!(
                app.world().resource::<State<GamePhase>>().get(),
                &GamePhase::InProgress,
                "precondition: the mission must have started on the first frame \
                 after the `NextState` write"
            );
            assert_eq!(
                waves_spawned(&app),
                1,
                "the first mission tick released {} waves. Only wave 1 is \
                 authored at `after_secs = 0`; waves 2 and 3 are authored at 45 \
                 and 90. Releasing more than one means `on_timer` is still being \
                 measured from an anchor stamped at `Startup`, so a {LOBBY_SECS} \
                 s lobby retired {LOBBY_SECS} s of the schedule before the crew \
                 could move.",
                waves_spawned(&app)
            );
        }
    }

    let wave_2_at = first_seen.get(&2).copied().unwrap_or_else(|| {
        panic!(
            "wave 2 never arrived in {MISSION_SECS} mission-seconds, though \
             combat_test.toml authors it at 45 s. Seen: {first_seen:?}"
        )
    });
    assert!(
        (wave_2_at - 45.0).abs() < 1.0,
        "wave 2 arrived {wave_2_at:.2} mission-seconds in, but combat_test.toml \
         authors it at 45 s. The clock has to start at mission start and then \
         run at one second per second: an anchor taken anywhere else moves this \
         reading by exactly the length of the lobby. All: {first_seen:?}"
    );
}

/// Issue #943 acceptance: the player's destroyer does NOT dump its magazine
/// into the opening of `combat_test`, and what stops it is the world's own count
/// of the threat still ahead.
///
/// The run is the same demo hull, world and seed the wave-clock guard above
/// flies, sampled every tick and bucketed by the world's own remaining-threat
/// count, so what it measures is the SHAPE of the payload across the run rather
/// than one moment in it. Four things are asserted, and they are the four ways
/// the feature can fail:
///
/// 1. The scenario is PUBLISHING the measure. `mission_threat_remaining` reads 8
///    until the first wave dies — the eight-wave schedule, set by the
///    `on_world_loaded` trigger and not yet decremented.
/// 2. The ship is FIGHTING. Rounds left because the hull never got a firing
///    solution would prove nothing, so the run must also have launched.
/// 3. The opening does not eat the payload. The fleet authors
///    `min_rounds_per_threat = 0.5`, so with eight waves published the hull is
///    cleared down to a floor of four rounds (three once a volley overshoots
///    it) and no further.
/// 4. The back half is not dry. This is the failure the FIRST cut of #943
///    shipped: measured against `torpedoes_remaining` — the rounds left to
///    reload with — rather than against the rounds aboard, the reserve reads
///    three rounds short on this hull (two tubes parking `volley_max` 2 + 1 under
///    the shipped "keep the tubes loaded" doctrine), so the gate latches shut
///    after the first engagement and the parked volley is never fired at all.
///    Rounds surviving the opening is only half the acceptance criterion; they
///    have to be SPENT later.
///
/// WHAT THE `8` BUCKET NOW MEANS (#960). Under the death-gated chain the top
/// bucket was exactly "while wave 1 was alive", because nothing else could be.
/// Under the clock it is "while all eight waves are still to be fought" — wave 2
/// launches at t=45 whether or not wave 1 is dead, so the bucket can span more
/// than one engagement. That is the honest reading of the counter and the one
/// the conservation doctrine wants: it divides rounds aboard by threat REMAINING,
/// and a wave already on the field is threat remaining. The assertions below are
/// worded against the counter, not against wave 1's lifetime.
///
/// Nobody is connected, so every `FireTorpedo` in this run is AI-origin — the
/// human-origin half of the same gate is pinned by
/// `console::weapons::server_tests::
/// torpedo_conservation_holds_a_human_origin_launch_when_the_mission_is_long`,
/// which drives `handle_fire_torpedo` from a hand-written admitted command with
/// no decider in the app at all. They are the same guard: there is only one, and
/// it sits below admission where the origin is already gone.
#[test]
fn combat_test_paces_the_player_magazine_against_the_whole_eight_wave_threat() {
    use project_phoenix::weapons_plugin::TorpedoSystemResource;
    use project_phoenix::world::server::WorldContentRuntime;

    let dt = 1.0 / 30.0;
    let args = HeadlessArgs {
        world_path: "assets/worlds/combat_test.toml".into(),
        ship_path: "assets/entities/alliance_destroyer.toml".into(),
        dt,
        // The same 600 s budget the wave-clock guard's sweep used, because this
        // measures the WHOLE run and not just its opening.
        max_ticks: ticks_for_sim_seconds(600.0, dt),
        seed: Some(2),
        deterministic: true,
        ..test_args()
    };
    let mut app = build_headless_app(&args).expect("app should build");

    // Stepped a tick at a time rather than run-then-look, because both questions
    // are about the SHAPE of the whole run rather than about wherever a single
    // sample lands: a ship that empties itself and then reloads out of a
    // still-stocked magazine reads full again a few seconds later, and a ship
    // that has gone dry for the back half looks identical at the final tick to
    // one that spent its last round on the last wave.
    //
    // `trace[t]` is (rounds aboard on entering threat level `t`, lowest seen
    // while there), for every level the run actually reached.
    let mut trace: std::collections::BTreeMap<i64, (u32, u32)> = std::collections::BTreeMap::new();
    for _ in 0..args.max_ticks {
        run(&mut app, 1);
        let remaining = app
            .world()
            .resource::<WorldContentRuntime>()
            .flags
            .counter("mission_threat_remaining");
        // 0 before the first tick's `on_world_loaded` dispatch, and again once
        // the last wave is dead — neither is a wave in progress.
        if remaining <= 0 {
            continue;
        }
        let mut q = app
            .world_mut()
            .query_filtered::<&TorpedoSystemResource, With<LocalShip>>();
        // The player hull is gone once it dies.
        let Ok(torps) = q.single(app.world()) else {
            break;
        };
        let aboard = torps.0.rounds_aboard();
        trace
            .entry(remaining)
            .and_modify(|(_, low)| *low = (*low).min(aboard))
            .or_insert((aboard, aboard));
    }
    // Printed unconditionally so a retune can be MEASURED rather than guessed at:
    // libtest captures this unless the run asks for it, so
    // `cargo test --features headless --test headless_runner
    //  combat_test_paces -- --nocapture` is how you read the magazine's shape
    // before and after touching `min_rounds_per_threat`. Not having this is how
    // the reserve got retuned blind in the first place.
    println!("magazine trace by remaining threat (entered, lowest): {trace:?}");
    let report = build_report(&mut app, &args, 0.0);

    let (entered_full_threat, lowest_at_full_threat) = *trace.get(&8).expect(
        "combat_test never published `mission_threat_remaining` as 8, so the world \
                 is not declaring its eight-wave threat at all and the conservation doctrine \
                 reads an unbounded ratio that paces nothing",
    );
    let launched = report
        .message_counts
        .get("TorpedoLaunched")
        .copied()
        .unwrap_or(0);
    assert!(
        launched > 0,
        "nothing launched in the whole run, so rounds left aboard say nothing about \
         conservation — it would only mean the hull never got a shot. \
         message_counts: {:?}",
        report.message_counts
    );

    // 1. The opening does not eat the payload. The floor is what the authored
    //    reserve GUARANTEES — `min_rounds_per_threat = 0.5` x 8 waves published
    //    is 4 rounds, and volley granularity can carry it to 3 — not wherever
    //    this seed's engagements happen to stop. A bound pinned to the measured
    //    low-water mark is a bound with no margin, and a bound with no margin on
    //    this exact quantity is what broke combat_test before: the previous 1.0
    //    tuning sat exactly on its own floor, and #946 moving the engagement by
    //    one round was enough to push it under and latch the magazine shut.
    //    Losing the gate still fails here: un-gated this run bottoms out at 0.
    assert_eq!(
        entered_full_threat, 12,
        "precondition: the destroyer must start the run with its full authored \
         magazine aboard, or the low-water mark below is measuring something else"
    );
    assert!(
        lowest_at_full_threat >= 3,
        "the destroyer was down to {lowest_at_full_threat} of its 12 rounds while \
         the world still published all eight waves as threat remaining. The whole \
         point of #943 is that the opening cannot eat the payload: with the full \
         schedule still ahead, the magazine's `torpedo_conservation` guard should \
         have held fire long before this. Trace by remaining threat \
         (entered, lowest): {trace:?}"
    );

    // 2. And the hull is still SPENDING rounds after the first wave dies — the
    //    other way this feature fails, and the one that actually shipped twice.
    //
    //    A reserve measured against `torpedoes_remaining` rather than the rounds
    //    aboard reads three rounds short on this hull (its two tubes park
    //    `volley_max` 2 + 1 with the shipped "keep the tubes loaded" doctrine),
    //    which locks the gate shut after the opening and strands the parked
    //    volley for good. A reserve authored too HIGH does the same thing by a
    //    different route: the guard is a one-way latch — rounds aboard only
    //    fall, and the published threat falls only when a wave dies — so a hull
    //    that dips under its reserve stops firing at the very waves whose deaths
    //    would let it fire again.
    //
    //    Measured on this seed after the stationary-station combat retune (a
    //    firing point-defence station + LOD bubbles keeping the raid sieging it
    //    in full fidelity + an 18 s `attacked_memory_secs` that peels raiders
    //    onto the player):
    //      {8: (12, 8), 7: (8, 2), 6: (2, 2), 5: (2, 2), 4: (2, 1), 3: (1, 1), ...}
    //    — the destroyer now hunts effectively and the run clears wave after wave
    //    (a VICTORY), so rounds keep coming down across the whole schedule rather
    //    than stalling after the opening. The two invariants below are what
    //    matter, not the exact figures: the opening does not eat the payload
    //    (lowest at full threat >= 3), and the hull keeps spending after wave 1.
    //    Before the retune the stationary station was a sitting duck, the raid
    //    ignored the player to grind it down, and this stretch went dry — the
    //    exact regression this half of the test guards.
    let later_waves: Vec<_> = trace.iter().filter(|(threat, _)| **threat < 8).collect();
    assert!(
        !later_waves.is_empty(),
        "the run never cleared a single wave, so it cannot say whether the hull \
         keeps shooting for the rest of the mission. Trace: {trace:?}"
    );
    let spent_after_first_wave: u32 = later_waves
        .iter()
        .map(|(_, (entered, lowest))| entered.saturating_sub(*lowest))
        .sum();
    assert!(
        spent_after_first_wave > 0,
        "the destroyer launched nothing at all once the first wave was dead — it \
         went dry for the rest of the run while still carrying rounds. \
         Conservation is meant to SPREAD the payload across the mission, not \
         spend it on the opening and then lock the hull out of every later \
         engagement. Trace by remaining threat (entered, lowest): {trace:?}"
    );
}

/// Production-schedule guard: an AI-crewed ship actually LAUNCHES a torpedo in
/// a real run.
///
/// `ai_torpedo_auto_fire` (the decider) and `console::weapons::handle_fire_torpedo`
/// (the applier) both live in `SimSet::Physics`. Until this guard landed, the
/// production registration in `ConsoleAiPlugin` declared no edge between them
/// and the resolved order put the CONSUMER first — so the admitted
/// `FireTorpedo` sat in `AdmittedCommands` untouched until admission's
/// `clear_before_input` wiped it at the top of the next tick, and an AI-crewed
/// ship never launched a torpedo in its life. Every weapons unit test passed
/// regardless, because `torpedo_ai_test_app` adds the missing
/// `.before(handle_fire_torpedo)` edge in its own harness; only a test that
/// boots the REAL plugin set can see it. This is the same shape as #881, where
/// an AI-emitted boost silently no-opped in production.
///
/// `probe_duel.toml` is the vehicle rather than a new world: two torpedo-armed
/// AI hulls at 45 units, mutually hostile from world-load, both on the
/// unconditional default tube launch policy, resolving in ~14 sim-seconds. The
/// budget is 90 s so the assertion is about the salvo, not about the clock.
///
/// `TorpedoLaunched` is proof rather than a proxy: nobody is connected, so no
/// human `FireTorpedo` exists, and `handle_fire_torpedo` is the only system
/// that turns an admitted `FireTorpedo` into a launch (the burst rounds
/// `tick_torpedo_lifecycle` reports all descend from one). A non-zero count can
/// therefore only mean the decider's command survived to its applier in the
/// same tick.
#[test]
fn ai_crewed_ships_actually_launch_torpedoes_in_a_real_run() {
    let dt = 1.0 / 30.0;
    let args = HeadlessArgs {
        world_path: "assets/worlds/probe_duel.toml".into(),
        dt,
        max_ticks: ticks_for_sim_seconds(90.0, dt),
        // Pinned, and NOT the world's own `[global] seed` — re-blessed for the
        // fixed logical tick (issue #895) under PRD #849's policy. #895 pinned
        // 2 here against a base that predated the composed cruiser doctrine
        // (#918) and the curated player hull (#916/#917); this base is the
        // integration of the two, and needs its own measurement.
        //
        // Since #918 the helm keeps the heading its own doctrine committed
        // instead of being swung bow-on by an `ArcBearingRequest`, so a tube
        // now bears only when the doctrine's torpedo-run leg brings it round.
        // That makes launches markedly rarer in this duel than under either
        // change alone (main: 3 launches in 16 s; #895 alone: 4 in 21 s), and
        // most seeds yielded 0 or 1 across the full 90 s window under that
        // (pre-#897) generator. Swept over that window on that base:
        //   838  the world seed. Resolves in 18.7 s on beams and blasters
        //        alone — 0 launches, too fast for a tube to come round.
        //   2    #895's pick. 90 s draw, 0 launches.
        //   3, 5, 8, 6, 9, 11, 12, 17, 31, 41, 43, 47, 55, 59, 61   1 launch
        //        apiece, all of them 90 s draws.
        //   1, 4, 7, 13, 14, 19, 21, 23, 29, 34, 37, 53, 89, 144    0.
        //   10   this seed, and the only one of the ~30 sampled that both
        //        resolved (a kill at 14.8 s) and launched — the pick over the
        //        1-launch stalemates.
        //
        // RE-MEASURED for #897's generator swap (`rand`'s SmallRng ->
        // `vellum_rng::Pcg32`), which re-rolls which system every hit lands on
        // and, with it, which duels resolve at all:
        // `phoenix-headless --world assets/worlds/probe_duel.toml --seed 10
        // --hz 30 --sim-seconds 90 --deterministic --report-format json`, 3
        // runs, byte-identical. Seed 10 no longer resolves on this generator —
        // the 90 s window now ends in a draw (player hull 116/216, enemy
        // 205/205), not the 14.8 s kill the table above was measured under.
        // It still launches twice (`TorpedoLaunched: 2`), which is all this
        // test asserts — `launched > 0`, not that the duel resolves. A launch
        // inside a real duel between two torpedo-armed AI hulls is still the
        // evidence the pipeline is whole; this seed just no longer doubles as
        // a resolution guard too. Re-sweeping the other ~29 seeds above for a
        // seed that both resolves and launches under the new generator is out
        // of scope here — nothing requires this pin to do both.
        seed: Some(10),
        deterministic: true,
        ..test_args()
    };
    let mut app = build_headless_app(&args).expect("app should build");
    run(&mut app, args.max_ticks);
    let report = build_report(&mut app, &args, 0.0);

    let launched = report
        .message_counts
        .get("TorpedoLaunched")
        .copied()
        .unwrap_or(0);
    assert!(
        launched > 0,
        "no torpedo ever left a tube in a duel between two torpedo-armed AI hulls. \
         TWO causes now produce this identically, so check both. (1) The original: \
         `ai_torpedo_auto_fire`'s admitted FireTorpedo is cleared before \
         `handle_fire_torpedo` sees it — a missing production ordering edge. \
         (2) Since #872 the tubes hold until Red Alert, and since #912 a \
         backfilled Alliance captain raises it on first contact from the authored \
         `fact(hostile_contact)` / `fact(hostile_range)` rule; if that rule is \
         gone from the hull TOML, or `operate_captain_ai` stopped seeding either \
         reading, neither hull ever calls the alert and neither ever launches. \
         `RedAlertChanged` in the balance ledger separates the two: absent means \
         (2), present means (1). message_counts: {:?}",
        report.message_counts
    );
}

/// Issue #792, the whole doctrine end-to-end: the Harrow battleship actually
/// FIRES its artillery in a real run.
///
/// Every unit test above the line asserts one link — the machine reaches `hold`,
/// the host publishes the leg, the planner solves an intercept, the bolt is
/// ballistic. None of them can see the chain snap in production, and there are
/// several places it could: the helm doctrine could park the hull outside its own
/// bank's arc or range, the artillery bank's `[[system]]` declaration could go
/// missing so the bank is never AI-operable, or `tick_blaster_auto_fire`'s
/// admitted `ChargeBlasterStart` could be wiped before `handle_fire_blaster` saw
/// it — the exact failure #791 found on the torpedo path, where every unit test
/// passed because the harness supplied the edge the production schedule did not.
///
/// `BlasterFired` is proof rather than a proxy: nobody is connected, so no human
/// `ChargeBlasterStart` exists, and this hull's only blaster is the bow artillery
/// piece #792 authored.
#[test]
fn the_harrow_battleship_fires_its_artillery_in_a_real_run() {
    let dt = 1.0 / 30.0;
    let args = HeadlessArgs {
        world_path: "assets/worlds/duel.toml".into(),
        side_a: vec!["cruiser".into()],
        side_b: vec!["ship_harrow_warhawk".into()],
        dt,
        max_ticks: ticks_for_sim_seconds(120.0, dt),
        seed: Some(792),
        deterministic: true,
        ..test_args()
    };
    let mut app = build_headless_app(&args).expect("app should build");
    run(&mut app, args.max_ticks);
    let report = build_report(&mut app, &args, 0.0);

    let fired = report
        .message_counts
        .get("BlasterFired")
        .copied()
        .unwrap_or(0);
    assert!(
        fired > 0,
        "the battleship never fired its bow artillery in a real duel — the chain \
         from `hold_artillery_position` through the bank's `[[system]]` \
         declaration to `handle_fire_blaster` is broken somewhere no unit test \
         can see. message_counts: {:?}",
        report.message_counts
    );
}

/// Issue #792's blocking defect, end-to-end: the battleship must actually come
/// to rest on its authored holding radius in a real run, with the impulse drive
/// production attaches to it present and attached.
///
/// [`the_harrow_battleship_fires_its_artillery_in_a_real_run`] above proves the
/// FIRE chain and nothing about the standoff — `duel.toml` anchors side B 55
/// units from the player, so the bank fires by luck of geometry and the run-in is
/// never flown at all. The unit tests, for their part, all measured a hull posed
/// at or near its holding radius.
///
/// Between those two there was no coverage of the one geometry that matters, and
/// a defect lived in it. `entities::spawner` attaches an `ImpulseConfigResource`
/// to every hull declaring a `[helm_console]`, taking parse defaults of engage
/// 200 / cancel 40 — a window the authored 180-unit hold band sits entirely
/// inside — and the impulse autopilot in `integrate_ship_physics` replaces
/// commanded throttle with `thrust = 1.0` for as long as the drive runs. So the
/// hull entered `hold` at 180, commanded `SetThrust{0.0}` every tick, and flew
/// straight through its own gun line to the drive's 40-unit release range.
///
/// `probe_artillery_standoff.toml` is built for exactly this: the battleship
/// starts 300 units out, so it flies a real run-in from beyond its own envelope,
/// and its doctrine is replaced wholesale by the scenario without a `use_impulse`
/// — the shape `duel.toml` and `combat_test.toml`'s `assault-starbase` waves
/// both use (since #892 that is most of the wave list, not just wave 8), and the one
/// that makes a doctrine-level fix worthless. The stopping distance is the
/// assertion because it is where the defect is a NUMBER: ~180 when the doctrine
/// is flown, ~40 when the drive is flying the hull.
#[test]
fn the_harrow_battleship_takes_up_its_artillery_standoff_in_a_real_run() {
    use project_phoenix::entity_config::EntityConfig;
    use project_phoenix::server_app::{Ship, ShipImpulse};
    use project_phoenix::ship_plugin::ImpulseConfigResource;

    /// The battleship's range to the player, and its own speed.
    fn standoff(app: &mut App) -> (f32, f32) {
        let player = *app
            .world_mut()
            .query_filtered::<&ShipPhysics, With<LocalShip>>()
            .single(app.world())
            .expect("exactly one player ship");
        let npc = *app
            .world_mut()
            .query_filtered::<&ShipPhysics, (With<Ship>, Without<LocalShip>)>()
            .single(app.world())
            .expect("exactly one NPC — the battleship");
        let (dx, dz) = (npc.x - player.x, npc.z - player.z);
        ((dx * dx + dz * dz).sqrt(), npc.forward_speed)
    }

    // The hull's own authored band, so this asserts against content rather than
    // against numbers restated here.
    let hull = EntityConfig::from_toml(
        project_phoenix::entity_includes::resolve_from_disk(
            "assets/entities/ship_harrow_warhawk.toml",
        )
        .expect("ship_harrow_warhawk must resolve")
        .toml
        .as_str(),
    )
    .expect("the shipped battleship hull must parse");
    let param = |name: &str| -> f32 {
        hull.helm_console
            .as_ref()
            .and_then(|hc| hc.steering_ai.as_ref())
            .and_then(|ai| ai.param.get(name).copied())
            .unwrap_or_else(|| panic!("the shipped battleship must author `{name}`"))
    };
    let max_artillery_range = param("max_artillery_range");
    let artillery_hold_range = param("artillery_hold_range");

    let dt = 1.0 / 30.0;
    let args = HeadlessArgs {
        world_path: "assets/worlds/probe_artillery_standoff.toml".into(),
        dt,
        max_ticks: ticks_for_sim_seconds(60.0, dt),
        seed: Some(792),
        deterministic: true,
        ..test_args()
    };
    let mut app = build_headless_app(&args).expect("app should build");

    // One second in: spawned, promoted, and still well outside its own envelope.
    // Without this the whole run could pass by starting where it means to finish.
    run(&mut app, ticks_for_sim_seconds(1.0, dt));
    let (start_range, _) = standoff(&mut app);
    assert!(
        start_range > max_artillery_range,
        "precondition: the battleship must begin OUTSIDE its artillery envelope \
         ({max_artillery_range}), or no run-in is flown; got {start_range}"
    );

    // The drive that discards the doctrine really is on this hull in production.
    // If a future change stops attaching it, this test would go on passing while
    // no longer proving anything, so the presence is asserted rather than assumed.
    let mut drive = app
        .world_mut()
        .query_filtered::<(&ImpulseConfigResource, &ShipImpulse), (With<Ship>, Without<LocalShip>)>(
        );
    let (impulse_cfg, _) = drive
        .single(app.world())
        .expect("the spawner attaches an impulse drive to every [helm_console] hull");
    assert!(
        impulse_cfg.engage_distance < start_range
            && impulse_cfg.cancel_distance < artillery_hold_range,
        "precondition: this run only exercises the defect while the hold band \
         ({artillery_hold_range}) sits inside the drive's cruise window \
         (engage {}, cancel {})",
        impulse_cfg.engage_distance,
        impulse_cfg.cancel_distance
    );

    // Fly the run-in out. 120 units at 9 units/s is ~14 s; the rest is settling
    // time, so what is read below is where the hull STOPS, not where it is.
    run(&mut app, ticks_for_sim_seconds(35.0, dt));
    let (settled_range, settled_speed) = standoff(&mut app);
    assert!(
        settled_speed.abs() < 0.1,
        "the battleship must come to rest, got {settled_speed} units/s"
    );
    assert!(
        (settled_range - artillery_hold_range).abs() < artillery_hold_range * 0.1,
        "the battleship must stop on its authored holding radius \
         ({artillery_hold_range}); got {settled_range}. A reading near the impulse \
         drive's cancel distance is the autopilot having overridden the hold's \
         own `SetThrust{{0.0}}` all the way in"
    );

    // ...and it STAYS there. "Holds station" is a claim about a range that stops
    // changing, so it needs two readings separated by real flown time.
    run(&mut app, ticks_for_sim_seconds(20.0, dt));
    let (held_range, _) = standoff(&mut app);
    assert!(
        (held_range - settled_range).abs() < 1.0,
        "the firing position must be HELD: {held_range} vs {settled_range}"
    );
}

/// Issue #793, the close defence end-to-end: the battleship's opportunistic
/// launchers actually put a round in the air in a real run.
///
/// The unit tests above the line each assert one link — the shipped guard reads
/// its own tube's facts, the decider admits a launch at that tube, the hull
/// declares the tube as a system. None of them can see the chain snap in
/// production, and the whole armament is authored content: a `[[system]]` block
/// missing its magazine entry switches BOTH the loader and the launcher off
/// silently, an unloadable tube never reaches `loaded`, and a doctrine that never
/// puts an enemy inside a 90-degree cone with its arc down would leave every unit
/// test passing over a weapon that has never fired.
///
/// The assertion is per-SHIP and per-WEAPON rather than on the run's
/// `TorpedoLaunched` count, because both duelists carry tubes and an aggregate
/// message count cannot say whose round it was. `WeaponFired` carries the
/// shooter's uuid and the tube's own id, so the battleship's ledger names the
/// launcher — which is also why the tube ids are checked against this hull's beam
/// and blaster ids first: `fore` on the battleship's ledger is only proof while
/// `fore` is not also the name of one of its guns.
#[test]
fn the_harrow_battleship_takes_its_close_defence_opportunities_in_a_real_run() {
    use project_phoenix::entity_config::EntityConfig;

    let hull = EntityConfig::from_toml(
        project_phoenix::entity_includes::resolve_from_disk(
            "assets/entities/ship_harrow_warhawk.toml",
        )
        .expect("ship_harrow_warhawk must resolve")
        .toml
        .as_str(),
    )
    .expect("the shipped battleship hull must parse");
    let tube_ids: Vec<String> = hull
        .torpedoes
        .as_ref()
        .expect("the battleship carries close-defence launchers")
        .tubes
        .iter()
        .map(|t| t.id.clone())
        .collect();
    let wc = hull
        .weapons_console
        .as_ref()
        .expect("hull declares [weapons_console]");
    for gun in wc
        .phaser_banks
        .iter()
        .map(|b| b.id.clone())
        .chain(wc.blaster_banks.iter().map(|b| b.id.clone()))
    {
        assert!(
            !tube_ids.contains(&gun),
            "precondition: `{gun}` names both a gun and a tube on this hull, so a \
             ledger row under that name could not attribute a launch"
        );
    }

    let dt = 1.0 / 30.0;
    let args = HeadlessArgs {
        world_path: "assets/worlds/duel.toml".into(),
        side_a: vec!["cruiser".into()],
        side_b: vec!["ship_harrow_warhawk".into()],
        dt,
        max_ticks: ticks_for_sim_seconds(120.0, dt),
        seed: Some(792),
        deterministic: true,
        ..test_args()
    };
    let mut app = build_headless_app(&args).expect("app should build");
    run(&mut app, args.max_ticks);
    let report = build_report(&mut app, &args, 0.0);

    // Side B is the battleship — the duel harness names its ships `side_b_<n>`.
    let battleship = report
        .damage_by_ship
        .values()
        .find(|l| {
            l.name_id
                .as_deref()
                .is_some_and(|n| n.starts_with("side_b"))
        })
        .unwrap_or_else(|| {
            panic!(
                "the battleship must appear in the ledger: {:?}",
                report.damage_by_ship
            )
        });
    // The assertion names the FORE tube specifically, because that is the only
    // launcher this duel reaches. The battleship holds its artillery lead, so the
    // cruiser spends the engagement inside the bow cone and never crosses the
    // stern one: the run's ledger is
    // `{"bow_artillery": 2, "fore": 1, "port": 2, "starboard": 2}`. Summing both
    // tubes instead would state coverage this test does not have — a permanently
    // dead AFT chain would still pass on the fore tube's round. The aft launcher
    // is pinned at unit level instead, by
    // `shipped_warhawk_launchers_decide_independently_through_a_downed_arc`,
    // which puts a target dead astern; no reachable duel scenario does.
    assert!(
        tube_ids.iter().any(|id| id == "fore"),
        "precondition: this hull must still carry a `fore` launcher for the \
         assertion below to name — its tubes are {tube_ids:?}"
    );
    let launches = battleship.shots_fired.get("fore").copied().unwrap_or(0);
    assert!(
        launches > 0,
        "the battleship never took a close-defence opportunity with its FORE \
         launcher in a real duel — the chain from the authored \
         `[[torpedoes.tubes]] ai` guard through the `torpedo-magazine` / \
         `torpedo-tube-*` declarations to `handle_fire_torpedo` is broken \
         somewhere no unit test can see. Its ledger: {:?}",
        battleship.shots_fired
    );
}

/// Issue #844 AC: an asymmetric duel driven from `--side-a`/`--side-b` runs
/// `duel.toml` to a classified annihilation, with side-tagged ledgers and side
/// aggregates.
///
/// A courier (side A, the player) against a battleship (side B) is a mismatch:
/// under seed 1 the courier is run down and dies, latching the run as a Defeat
/// via the built-in player-death path. (A fast courier can also evade a slow
/// battleship indefinitely — that is a legitimate timeout under other seeds —
/// so this pins the seed that resolves.) Both combatants appear in the per-ship
/// ledger (side A = Federation, side B = Harrow), and the side aggregates in
/// `outcome_report` carry the enemy's surviving hull.
#[test]
fn asymmetric_duel_ends_in_annihilation_with_side_tagged_ledgers() {
    let dt = 1.0 / 30.0;
    let args = HeadlessArgs {
        world_path: "assets/worlds/duel.toml".into(),
        side_a: vec!["courier".into()],
        side_b: vec!["battleship".into()],
        dt,
        max_ticks: ticks_for_sim_seconds(90.0, dt),
        seed: Some(1),
        deterministic: true,
        ..test_args()
    };
    let mut app = build_headless_app(&args).expect("app should build");
    run(&mut app, args.max_ticks);
    let report = build_report(&mut app, &args, 0.0);

    assert_eq!(
        report.final_phase,
        format!("{:?}", GamePhase::GameOver),
        "a courier vs battleship duel must annihilate one side, ended {:?} / ship {:?}",
        report.final_phase,
        report.ship
    );
    // The player courier is the one that dies → the player-death latch reads
    // Defeat (no scenario side_a trigger is authored).
    assert_eq!(
        report.outcome_report.outcome,
        RunOutcome::Defeat,
        "the weaker player side should be annihilated: {:?}",
        report.outcome_report
    );
    // Both duelists appear, side-tagged: two rows, both having traded fire.
    assert_eq!(
        report.damage_by_ship.len(),
        2,
        "expected both duelists in the ledger, got {:?}",
        report.damage_by_ship
    );
    let total_dealt: f32 = report.damage_by_ship.values().map(|l| l.damage_dealt).sum();
    assert!(
        total_dealt > 0.0,
        "no damage dealt: {:?}",
        report.damage_by_ship
    );
    // Side aggregates are populated: the surviving enemy battleship keeps hull.
    assert!(
        report.outcome_report.enemy.remaining_hull > 0.0,
        "the surviving enemy side should report remaining hull: {:?}",
        report.outcome_report
    );

    // The report serialises both side aggregates (AC1).
    let json = report.to_json();
    let parsed: serde_json::Value = serde_json::from_str(&json)
        .unwrap_or_else(|e| panic!("report is not valid JSON: {e}\n{json}"));
    assert!(parsed["sides"]["player"].is_object());
    assert!(parsed["sides"]["enemy"].is_object());
    assert!(parsed["damage_by_ship"].is_object());
}

/// Issue #844 AC: a duel cut off before either side is annihilated ends in a
/// timeout/draw carrying per-side margins — not a false victory/defeat.
///
/// A cruiser-vs-destroyer duel is a live fight for tens of seconds; a 5 s budget
/// leaves it `InProgress` with damage still flowing, which the classifier reads
/// as a timeout, with both sides' margins populated.
#[test]
fn a_duel_cut_short_ends_in_timeout_with_populated_margins() {
    let dt = 1.0 / 30.0;
    let args = HeadlessArgs {
        world_path: "assets/worlds/duel.toml".into(),
        side_a: vec!["cruiser".into()],
        side_b: vec!["destroyer".into()],
        dt,
        max_ticks: ticks_for_sim_seconds(5.0, dt),
        deterministic: true,
        ..test_args()
    };
    let mut app = build_headless_app(&args).expect("app should build");
    run(&mut app, args.max_ticks);
    let report = build_report(&mut app, &args, 0.0);

    assert_eq!(
        report.final_phase,
        format!("{:?}", GamePhase::InProgress),
        "a 5 s cutoff should not have resolved: {}",
        report.final_phase
    );
    assert_eq!(
        report.outcome_report.outcome,
        RunOutcome::Timeout,
        "budget exhausted with combat still live is a timeout: {:?}",
        report.outcome_report
    );
    // Both sides carry margins (remaining hull on each side of the two-sided
    // fight).
    assert!(
        report.outcome_report.player.remaining_hull > 0.0
            && report.outcome_report.enemy.remaining_hull > 0.0,
        "both sides should still have hull mid-fight: {:?}",
        report.outcome_report
    );
}

/// Issue #844: an unknown ship name is a clean, informative error naming every
/// path that was tried.
#[test]
fn an_unknown_side_ship_is_a_clean_error() {
    let args = HeadlessArgs {
        world_path: "assets/worlds/duel.toml".into(),
        side_a: vec!["nonesuch".into()],
        ..test_args()
    };
    let err = build_headless_app(&args).unwrap_err().to_string();
    assert!(
        err.contains("nonesuch") && err.contains("alliance_nonesuch.toml"),
        "error should list the tried paths, got: {err}"
    );
}

#[test]
fn missing_world_file_is_a_clean_error() {
    let args = HeadlessArgs {
        world_path: "assets/worlds/does_not_exist.toml".into(),
        ..test_args()
    };
    let err = build_headless_app(&args).unwrap_err().to_string();
    assert!(err.contains("could not read world"), "got: {err}");
}

/// Issue #973, end to end: a world naming a `template_path` that does not
/// resolve must ABORT THE BUILD, not build happily and spawn one entity fewer.
///
/// The old behaviour is what made #954's hull relocation so hard to see — the
/// scenario ran to completion, spawned no hostiles, and the only signal was an
/// `entity template not found in cache` log line. Headless is the authoritative
/// host (its loader reaches the filesystem, so absence is a fact), so the
/// composition gate that already runs before anything spawns now refuses.
#[test]
fn a_world_naming_an_unresolvable_template_fails_the_build() {
    let world = std::env::temp_dir().join("phoenix_973_unresolvable_template_world.toml");
    std::fs::write(
        &world,
        "[[entity]]\n\
         template_path = \"assets/entities/definitely_not_a_hull.toml\"\n\
         name = \"ghost\"\n\
         transform = { position = [0.0, 0.0, 0.0] }\n",
    )
    .expect("write world fixture");

    let args = HeadlessArgs {
        world_path: world.to_string_lossy().into_owned(),
        ..test_args()
    };
    let err = build_headless_app(&args)
        .err()
        .map(|e| e.to_string())
        .unwrap_or_else(|| {
            panic!(
                "a world whose template does not resolve must not build: it would \
                 run to completion with the entity silently missing"
            )
        });

    assert!(
        err.contains("unresolvable-template"),
        "the abort must name the category so the log says what to fix, got: {err}"
    );
    assert!(
        err.contains("definitely_not_a_hull.toml") && err.contains("ghost"),
        "and must name the template and the entity, got: {err}"
    );
    assert!(
        err.contains("activation blocked"),
        "it goes through the same atomic-activation gate as every other \
         composition error, got: {err}"
    );

    std::fs::remove_file(&world).ok();
}

// ── `ShipPhysics` writer disjointness (issues #699, #886) ────────────────────
//
// `simulate_low_lod_ships` (`src/ai/server.rs`) writes `ShipPhysics.x/z/yaw`
// directly instead of going through helm intent + `integrate_ship_physics`.
// That is sanctioned — see the writer-policy table on `ShipPhysics`
// (`src/ship/state.rs`) — but the whole justification rests on one property:
//
//     No entity is ever moved by both the low-LOD substitute and the
//     admitted/integrated helm path in the same tick.
//
// Today that holds structurally: `simulate_low_lod_ships` filters
// `(With<Ship>, Without<AiHighFidelity>)` and `integrate_ship_physics` filters
// `With<AiHighFidelity>`. Until now it was asserted only in a comment. Two
// writers of `ShipPhysics` is the hazard issue #699 exists for, and the LOD
// machinery has already produced six *silent* bugs (#692, #693, #695, #785,
// #786, #882/#883) from components missed on one of its two spawn routes, so
// the property is pinned here rather than trusted.
//
// These tests are deliberately *access-level*, not behavioural. They read the
// `FilteredAccessSet` Bevy itself derived from each system as the production
// plugins registered it, and ask Bevy's own disjointness prover
// (`FilteredAccessSet::get_conflicts` — the machinery that decides whether two
// systems may run in parallel) whether the two can ever be handed the same
// entity. That is strictly stronger than spawning ships and watching them move:
// it holds for every archetype that could ever exist, not just the ones a
// fixture happened to create, so it cannot be fooled by a test ship missing the
// component that actually breaks the feature in production.
//
// They run against the real headless app, so the filters exercised are the ones
// actually registered — not a copy of the query signatures re-typed in a
// fixture, which would keep passing after someone widened the real filter.

/// A system in the real app that takes *mutable* access to `ShipPhysics`,
/// together with the access set the schedule recorded for it.
struct PhysicsWriter {
    schedule: String,
    /// Only meaningful when built with the `debug` feature — bevy compiles
    /// system names out otherwise — so never assert on it. It is here to make a
    /// CI failure legible.
    name: String,
    access: bevy::ecs::query::FilteredAccessSet,
}

/// Every mutable-`ShipPhysics` system in every schedule of `app`.
///
/// Must be called on an app that has been built but never run: `Schedule`
/// initialization moves systems out of the graph into a private executable and
/// takes their recorded access with them, so this is the only window in which
/// the access set is reachable through public API.
///
/// Bevy observers are not part of any schedule and so never appear here —
/// `handle_slow_zone_speed_clamp` (`src/regions/server.rs`) is one, and is
/// covered by the writer-policy table rather than by this scan.
fn ship_physics_writers(app: &mut App) -> Vec<PhysicsWriter> {
    use bevy::ecs::schedule::Schedules;

    let physics_id = app.world_mut().register_component::<ShipPhysics>();
    let mut schedules = app
        .world_mut()
        .remove_resource::<Schedules>()
        .expect("a built app always carries a Schedules resource");

    let mut writers = Vec::new();
    for (label, schedule) in schedules.iter_mut() {
        let label = format!("{label:?}");
        // Populates `SystemWithAccess::access` without building the schedule,
        // which is what would move the systems out of reach.
        schedule.graph_mut().systems.initialize(app.world_mut());
        let systems = &schedule.graph().systems;
        for (key, system, _conditions) in systems.iter() {
            let entry = systems
                .get(key)
                .expect("key was just yielded by this container");
            if entry
                .access
                .combined_access()
                .has_component_write(physics_id)
            {
                writers.push(PhysicsWriter {
                    schedule: label.clone(),
                    name: system.name().to_string(),
                    access: entry.access.clone(),
                });
            }
        }
    }
    app.world_mut().insert_resource(schedules);
    writers
}

/// Access set of a hypothetical system that writes `ShipPhysics` on exactly the
/// ships selected by `F`. Conflict-testing a real writer against this asks
/// "can that system ever be handed a ship matching `F`?".
fn ship_physics_probe<F>(app: &mut App) -> bevy::ecs::query::FilteredAccessSet
where
    F: bevy::ecs::query::QueryFilter + Send + Sync + 'static,
{
    let mut probe = IntoSystem::into_system(|_q: Query<&mut ShipPhysics, F>| {});
    probe.initialize(app.world_mut())
}

/// Splits the writers into those that can be handed a high-fidelity ship and
/// those that can be handed a low-LOD one. A writer with no `AiHighFidelity`
/// filter appears in both.
fn classify_ship_physics_writers(
    app: &mut App,
    writers: &[PhysicsWriter],
) -> (Vec<usize>, Vec<usize>) {
    let touches_high_fi = ship_physics_probe::<With<AiHighFidelity>>(app);
    let touches_low_lod = ship_physics_probe::<Without<AiHighFidelity>>(app);

    let high_fi = (0..writers.len())
        .filter(|&i| !writers[i].access.get_conflicts(&touches_high_fi).is_empty())
        .collect();
    let low_lod = (0..writers.len())
        .filter(|&i| !writers[i].access.get_conflicts(&touches_low_lod).is_empty())
        .collect();
    (high_fi, low_lod)
}

/// Renders the writer inventory for an assertion message. Without the `debug`
/// feature the names are placeholders, so the classification carries the
/// information.
fn describe_ship_physics_writers(
    writers: &[PhysicsWriter],
    high_fi: &[usize],
    low_lod: &[usize],
) -> String {
    writers
        .iter()
        .enumerate()
        .map(|(i, w)| {
            let kind = match (high_fi.contains(&i), low_lod.contains(&i)) {
                (true, false) => "high-fidelity ships only",
                (false, true) => "low-LOD ships only",
                (true, true) => "unfiltered (can touch every ship)",
                (false, false) => "matches no ship at all (?)",
            };
            format!("  - [{}] {} - {kind}", w.schedule, w.name)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The property `simulate_low_lod_ships`' direct `ShipPhysics` write depends on:
/// the low-LOD substitute and the helm integrator can never be handed the same
/// entity, so no ship is ever moved twice in a tick.
#[test]
fn low_lod_and_helm_ship_physics_writers_can_never_share_an_entity() {
    let mut app = build_headless_app(&test_args()).expect("app should build");
    let writers = ship_physics_writers(&mut app);
    let (high_fi, low_lod) = classify_ship_physics_writers(&mut app, &writers);

    let only_low_lod: Vec<usize> = low_lod
        .iter()
        .copied()
        .filter(|i| !high_fi.contains(i))
        .collect();
    let only_high_fi: Vec<usize> = high_fi
        .iter()
        .copied()
        .filter(|i| !low_lod.contains(i))
        .collect();

    let inventory = describe_ship_physics_writers(&writers, &high_fi, &low_lod);

    assert_eq!(
        only_low_lod.len(),
        1,
        "expected exactly ONE system writing ShipPhysics only on low-LOD ships \
         (`simulate_low_lod_ships`, filtered `Without<AiHighFidelity>`). That system \
         writes ShipPhysics.x/z/yaw directly instead of going through helm intent, and \
         the only thing that makes it safe is that the helm integrator can never see \
         the same entity. A count of 0 means its `Without<AiHighFidelity>` filter was \
         widened or removed, so it now dead-reckons ships the helm path is also \
         integrating: the ship advances twice per tick and every distance, arrival and \
         intercept calculation downstream is silently wrong — nothing panics. Restore \
         the filter, or move the system onto helm intent components and delete its row \
         from the writer-policy table on `ShipPhysics` (src/ship/state.rs). \
         ShipPhysics writers found:\n{inventory}"
    );
    assert_eq!(
        only_high_fi.len(),
        1,
        "expected exactly ONE system writing ShipPhysics only on high-fidelity ships \
         (`integrate_ship_physics`, filtered `With<AiHighFidelity>`). A count of 0 means \
         the helm integrator's filter was widened, so it now also integrates the ships \
         `simulate_low_lod_ships` is dead-reckoning: both writers advance the same \
         ShipPhysics in one tick and the ship travels at roughly double speed along a \
         heading neither system chose. See the writer-policy table on `ShipPhysics` \
         (src/ship/state.rs). ShipPhysics writers found:\n{inventory}"
    );

    let low = &writers[only_low_lod[0]];
    let helm = &writers[only_high_fi[0]];
    assert!(
        low.access.get_conflicts(&helm.access).is_empty(),
        "the low-LOD ShipPhysics substitute and the helm integrator are no longer \
         provably disjoint: Bevy's own access prover says they can be handed the same \
         entity, so one ship can be moved by both in a single tick. That is the \
         two-writer hazard issue #699 exists for, and it fails silently — the \
         simulation keeps running, the ship just is not where anything thinks it is. \
         Either restore the `With<AiHighFidelity>` / `Without<AiHighFidelity>` split \
         that keeps them apart, or stop writing ShipPhysics directly from the low-LOD \
         path. ShipPhysics writers found:\n{inventory}"
    );
}

/// Reconciles the scan against the writer-policy table on `ShipPhysics`
/// (`src/ship/state.rs`). A new system that mutates `ShipPhysics` fails here
/// until it is either given a disjoint filter or written into that table.
#[test]
fn ship_physics_writer_inventory_matches_the_policy_table() {
    let mut app = build_headless_app(&test_args()).expect("app should build");
    let writers = ship_physics_writers(&mut app);
    let (high_fi, low_lod) = classify_ship_physics_writers(&mut app, &writers);
    let inventory = describe_ship_physics_writers(&writers, &high_fi, &low_lod);

    // The scheduled writers named by the policy table: the helm integrator
    // (`integrate_ship_physics`), the low-LOD substitute
    // (`simulate_low_lod_ships`), the collision responder (`handle_collisions`)
    // and blaster recoil (`tick_blaster_system`). The table's fifth entry,
    // `handle_slow_zone_speed_clamp`, is an observer and so is in no schedule —
    // see `ship_physics_writers`.
    assert_eq!(
        writers.len(),
        4,
        "the number of scheduled systems writing ShipPhysics changed. Every writer \
         beyond the helm integrator has to be a correction layered on top of it rather \
         than a competing integrator, and has to be documented in the writer-policy \
         table on `ShipPhysics` (src/ship/state.rs). Two systems integrating the same \
         ship is the bug class issue #699 exists for, and it is silent: nothing panics, \
         the ship simply moves further than everything downstream believes it did. If \
         you added a writer, prefer helm intent components; if it genuinely must write \
         directly, add its row to the table and update this count. ShipPhysics writers \
         found:\n{inventory}"
    );

    // Exactly two of them are unfiltered corrections (collision response and
    // blaster recoil): they deliberately apply to every ship, high-LOD and
    // low-LOD alike, and their safety argument is that they are one-shot
    // corrections rather than integrators — not filter disjointness.
    let unfiltered = (0..writers.len())
        .filter(|i| high_fi.contains(i) && low_lod.contains(i))
        .count();
    assert_eq!(
        unfiltered, 2,
        "expected exactly two unfiltered ShipPhysics correction writers (collision \
         response and blaster recoil). A change here means a correction grew an \
         `AiHighFidelity` filter, or an integrator lost one — either way the set of \
         ships that get moved twice per tick has changed. Reconcile with the \
         writer-policy table on `ShipPhysics` (src/ship/state.rs). ShipPhysics writers \
         found:\n{inventory}"
    );
}

// ── #871: NPC hulls are stationed ships with nobody connected ────────────────

/// Locate the one world-spawned NPC ship in a booted world: `Ship`, not
/// `LocalShip`, and carrying its own station/system config.
fn npc_ship_with_stations(
    app: &mut App,
) -> (
    ShipConfigComponent,
    ShipSystemControlSources,
    ActiveStationRatings,
) {
    let mut q = app.world_mut().query_filtered::<(
        &ShipConfigComponent,
        &ShipSystemControlSources,
        &ActiveStationRatings,
    ), (With<project_phoenix::server_app::Ship>, Without<LocalShip>)>(
    );
    let mut found: Vec<_> = q
        .iter(app.world())
        .filter(|(cfg, _, _)| !cfg.0.stations.is_empty())
        .map(|(cfg, cs, ar)| (cfg.clone(), cs.clone(), ar.clone()))
        .collect();
    assert_eq!(
        found.len(),
        1,
        "patrol.toml spawns exactly one NPC ship; found {}",
        found.len()
    );
    found.remove(0)
}

/// AC: an unmanned NPC hull reports `Backfill` on every station, and every
/// system it carries is AI-operated and closed to human input — i.e. it behaves
/// exactly as it did when its systems were ownerless `ai_only` declarations.
///
/// This boots the real world, so it covers the whole path: TOML → `EntityConfig`
/// → `entities::spawner::spawn_entity` → `ship::rating::seed_boot_ratings`.
#[test]
fn an_unmanned_npc_hull_is_fully_backfilled_on_every_station() {
    let args = test_args();
    let mut app = build_headless_app(&args).expect("app should build");
    run(&mut app, args.max_ticks);

    let (config, sources, ratings) = npc_ship_with_stations(&mut app);

    assert!(
        !config.0.stations.is_empty(),
        "the NPC hull must declare crew stations — that is what #871 gave it"
    );
    for station in &config.0.stations {
        assert_eq!(
            ratings.0.get(&station.id).map(String::as_str),
            Some(project_phoenix::ship::rating::BACKFILL_RATING),
            "NPC station {:?} must report Backfill with nobody connected; got {:?}",
            station.id,
            ratings.0
        );
    }

    for system in &config.0.systems {
        let policy = sources.0.policy_for(&system.id);
        assert!(
            policy.operate_ai,
            "NPC system {:?} (owner {:?}, ai_only {}) must be AI-operated on an \
             unmanned hull",
            system.id, system.station, system.ai_only
        );
        assert!(
            !policy.accept_human_input,
            "NPC system {:?} must not accept human input while backfilled",
            system.id
        );
    }

    // AC: `ai_only` survives only on ownerless systems. Everything a station
    // owns dropped the flag.
    for system in &config.0.systems {
        if system.station.is_some() {
            assert!(
                !system.ai_only,
                "station-owned system {:?} must not rely on ai_only",
                system.id
            );
        }
    }
}

/// AC: a human can take an NPC hull's Tactical seat and be admitted to its
/// systems, exactly as at a backfilled player seat.
///
/// Both halves matter and the first is the mutation guard: while the seat is
/// unmanned it reports `Backfill`, the AI holds the systems, and the human is
/// REFUSED. Regress the spawner to the old blanket "set every declared system to
/// Ai" seed and the ratings map is empty, so the Backfill assertion fails before
/// the join is even attempted.
///
/// The join itself is the production seam: `ship::rating::apply_rating` (what
/// `handle_station_rating_change` calls) followed by the real admission
/// predicate `command_admission::is_command_authorized`, evaluated against the
/// NPC ship's OWN config and control sources.
#[test]
fn a_human_can_take_an_npc_hull_tactical_seat() {
    use project_phoenix::command_admission::is_command_authorized;
    use project_phoenix::lobby::Sessions;
    use project_phoenix::messages::{StationId, SystemControlPayload, SystemId};

    let args = test_args();
    let mut app = build_headless_app(&args).expect("app should build");
    run(&mut app, args.max_ticks);

    let (config, mut sources, ratings) = npc_ship_with_stations(&mut app);
    let tactical = StationId("tactical".into());
    let radar = SystemId("tactical-radar".into());
    let payload = SystemControlPayload::SetTarget {
        uuid: "some-contact".into(),
    };

    assert!(
        config.0.station(&tactical).is_some(),
        "the NPC hull must declare a Tactical seat"
    );
    assert!(
        config
            .0
            .systems_for_station(&tactical)
            .any(|s| s.id == radar),
        "the Tactical seat must own `tactical-radar` — the system #887 needs \
         declared on NPC hulls to route AI target selection through admission"
    );

    // A human token that has claimed the Tactical seat.
    let mut sessions = Sessions(project_phoenix::session::SessionManager::new());
    sessions
        .0
        .register("player-token".into(), "Rook".into())
        .expect("registration succeeds");
    sessions
        .0
        .set_station("player-token", Some(tactical.clone()));

    // ── Unmanned: Backfill, AI holds the radar, the human is refused ─────────
    assert_eq!(
        ratings.0.get(&tactical).map(String::as_str),
        Some(project_phoenix::ship::rating::BACKFILL_RATING),
        "the seat must boot on Backfill; an empty ratings map means the spawner \
         regressed to the all-Ai seed and never applied a station rating at all"
    );
    assert!(
        is_command_authorized("ai:npc", &radar, &payload, &sources, &sessions, &config.0, None),
        "the backfilled seat's AI must hold the radar before the human sits down"
    );
    assert!(
        !is_command_authorized(
            "player-token",
            &radar,
            &payload,
            &sources,
            &sessions,
            &config.0,
            None
        ),
        "a backfilled seat must refuse human input — that is what claiming it changes"
    );

    // ── Join the seat: exactly what `handle_station_rating_change` does ──────
    project_phoenix::ship::rating::apply_rating(&config.0, &tactical, "Std", &mut sources.0);

    assert!(
        is_command_authorized(
            "player-token",
            &radar,
            &payload,
            &sources,
            &sessions,
            &config.0,
            None
        ),
        "a human holding the NPC hull's Tactical seat must be admitted to its radar"
    );
    assert!(
        !is_command_authorized("ai:npc", &radar, &payload, &sources, &sessions, &config.0, None),
        "the AI must stand down from a seat a human has taken"
    );

    // A token that holds no seat on this ship is still refused, so admission is
    // gating on station tenure rather than merely on `accept_human_input`.
    assert!(
        !is_command_authorized(
            "some-other-token",
            &radar,
            &payload,
            &sources,
            &sessions,
            &config.0,
            None
        ),
        "only the seat's holder may drive its systems"
    );

    // Seats nobody claimed are untouched by the join.
    let engineering = StationId("engineering".into());
    for system in config.0.systems_for_station(&engineering) {
        assert!(
            sources.0.policy_for(&system.id).operate_ai,
            "unclaimed seat {:?} keeps system {:?} on AI",
            engineering,
            system.id
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Issue #875 — the first COMPOSED shipped hull, end to end
// ─────────────────────────────────────────────────────────────────────────────

/// **The composed player destroyer boots, backfills, and flies.**
///
/// `alliance_destroyer.toml` is the first shipped hull assembled from the
/// fragment library — its Captain doctrine, its five target selectors, its
/// ship-level policies and all three of its travel axes arrive through
/// `includes`. Nothing else in this file spawns one, and unit tests over the
/// resolved document cannot say that the RESOLVED hull survives the real boot
/// path: template cache → include resolution → spawn → station ratings →
/// backfill → the shared AI tick → the planner → physics.
///
/// The movement assertion is the sharp end. A composed hull whose three travel
/// axes failed to compose would still load, still validate, still backfill and
/// still report every station crewed — and would sit motionless, because a
/// policy that resolves no verb emits nothing and the throttle coasts. That is
/// the #779 failure shape at hull scale, and only ticking the real app catches
/// it.
#[test]
fn the_composed_player_destroyer_boots_backfilled_and_flies() {
    let dt = 1.0 / 30.0;
    let args = HeadlessArgs {
        world_path: "assets/worlds/probe_duel.toml".into(),
        ship_path: "assets/entities/alliance_destroyer.toml".into(),
        dt,
        max_ticks: ticks_for_sim_seconds(8.0, dt),
        deterministic: true,
        ..test_args()
    };
    let mut app = build_headless_app(&args).expect("the composed destroyer must build an app");
    run(&mut app, args.max_ticks);

    {
        let mut q = app
            .world_mut()
            .query_filtered::<(&ShipSystemControlSources, &ActiveStationRatings), With<LocalShip>>(
            );
        let (sources, ratings) = q.single(app.world()).expect("exactly one LocalShip");
        assert_eq!(
            ratings.0.len(),
            4,
            "the destroyer's four authored seats must survive composition: {:?}",
            ratings.0
        );
        for (station, rating) in &ratings.0 {
            assert_eq!(
                rating,
                project_phoenix::ship::rating::BACKFILL_RATING,
                "station {station:?} is not backfilled"
            );
        }
        assert!(
            sources.0.entries().any(|(_, s)| *s == ControlSource::Ai),
            "no system ended up under AI control"
        );
    }

    let start = {
        let mut q = app
            .world_mut()
            .query_filtered::<&ShipPhysics, With<LocalShip>>();
        let p = q.single(app.world()).expect("one LocalShip");
        (p.x, p.z)
    };
    run(&mut app, ticks_for_sim_seconds(12.0, dt));
    let end = {
        let mut q = app
            .world_mut()
            .query_filtered::<&ShipPhysics, With<LocalShip>>();
        let p = q.single(app.world()).expect("one LocalShip");
        (p.x, p.z)
    };
    let travelled = ((end.0 - start.0).powi(2) + (end.1 - start.1).powi(2)).sqrt();
    assert!(
        travelled > 20.0,
        "the composed destroyer moved {travelled:.1} units in 12 s of a live          engagement. Its authored max_speed is 15, so a hull whose travel axes          actually composed covers far more — a motionless ship is what a policy          that resolves no verb looks like."
    );
}

/// **A hull's own backfilled Navigation must not cancel its own doctrine.**
///
/// The regression this pins is the one `probe_duel.toml` structurally cannot
/// see. `pass_under_navigation_orders` stands the authored manoeuvre down under
/// a cleared Navigation waypoint, which is right for a captain's redirection
/// (PRD #774 stories 10/11) — but `NavigationWaypoint` has TWO writers, and the
/// second is the ship's own `operate_navigation_ai`. On a hull that declares a
/// navigation system (every Alliance hull) with a mission `Destroy` objective
/// that NAMES its target, that operator waypoints the ship onto the very entity
/// the pass is attacking, once, anchored — and `NavigationWaypoint::set` is
/// idempotent for an anchored target that merely moves, so the clearance latches
/// for the whole engagement. Keyed on presence alone, the stand-down therefore
/// fired on every tick of every such mission and the class doctrine never flew.
///
/// `probe_duel.toml` authors an UNTARGETED Destroy, and the nav operator's
/// objective arm guards on `!target.is_empty()`, so no waypoint is ever set
/// there and the whole hazard is invisible. Hence a world of its own.
///
/// The two unit tests over `pass_under_navigation_orders` are pure-function
/// tests: they can say what the precedence decides given inputs, and cannot say
/// which inputs a real composed hull actually presents. Only a live tick can.
#[test]
fn a_targeted_destroy_objective_does_not_cancel_the_hulls_own_attack_pass() {
    use project_phoenix::navigation_plugin::NavigationWaypoint;
    use project_phoenix::ship::helm_ai::HelmPassSurface;

    let dt = 1.0 / 30.0;
    let args = HeadlessArgs {
        world_path: "assets/worlds/probe_targeted_pass.toml".into(),
        ship_path: "assets/entities/alliance_destroyer.toml".into(),
        dt,
        max_ticks: ticks_for_sim_seconds(40.0, dt),
        deterministic: true,
        ..test_args()
    };
    let mut app = build_headless_app(&args).expect("the composed destroyer must build an app");

    // The measurement is the hull's HEADING against its own frozen escape
    // heading, sampled per tick across the escape dwell.
    //
    // A range trace was tried first and does not discriminate: a stood-down hull
    // still carries 15 units/s of momentum out of the merge and still opens
    // plenty of daylight before it can turn, so both behaviours look alike for
    // several seconds. What ONLY the pass arm does is fly `escape_heading_rad`
    // — the heading the host froze at the merge — and deliberately stop looking
    // at the target for the authored dwell. A hull whose own Navigation stood
    // the doctrine down flies ordinary travel toward the anchored waypoint
    // instead, i.e. curls straight back onto the ship it just passed, and its
    // heading walks away from the frozen one until it is flying the reciprocal.
    let mut waypoint_onto_target_ticks = 0usize;
    let mut escape_ticks = 0usize;
    let mut min_range = f32::MAX;
    let mut worst_escape_heading_error = 0.0f32;

    for _ in 0..args.max_ticks {
        run(&mut app, 1);
        let mut q = app.world_mut().query_filtered::<(
            &ShipPhysics,
            &NavigationWaypoint,
            Option<&HelmPassSurface>,
        ), With<LocalShip>>();
        let Ok((physics, waypoint, pass)) = q.single(app.world()) else {
            continue;
        };
        // The waypoint's own snapshot is both the hazard's proof and the range
        // reference: an ANCHORED waypoint mirrors its parent entity's transform,
        // so if this is present the ship's Navigation really did waypoint the
        // objective's named target, and its position is that target's.
        let Some(snapshot) = waypoint.snapshot() else {
            continue;
        };
        if snapshot.source_uuid.is_none() {
            continue;
        }
        waypoint_onto_target_ticks += 1;
        min_range = min_range
            .min(((physics.x - snapshot.x).powi(2) + (physics.z - snapshot.z).powi(2)).sqrt());

        // `escape` and `escape_heading_rad` are both published by the ship's own
        // Steering POLICY, which the planner never writes — so they say the
        // doctrine reached its commitment and what it committed to, and the
        // hull's actual yaw says whether the planner let it fly.
        let Some(pass) = pass.filter(|p| p.escape) else {
            continue;
        };
        escape_ticks += 1;
        let error = (physics.yaw - pass.escape_heading_rad + std::f32::consts::PI)
            .rem_euclid(std::f32::consts::TAU)
            - std::f32::consts::PI;
        worst_escape_heading_error = worst_escape_heading_error.max(error.abs());
    }

    // 1. The hazard is live: the ship's own Navigation DID clear its helm to an
    //    anchored waypoint on the objective's target. Without this the rest of
    //    the assertions would pass vacuously on a world that never reproduced
    //    the state at all — which is exactly how `probe_duel` passes.
    assert!(
        waypoint_onto_target_ticks > 0,
        "no anchored Navigation waypoint was ever cleared, so this world is not          reproducing the two-writer state the regression lives in"
    );

    // 2. The doctrine actually flew: the hull committed to a run and merged, and
    //    its own policy reached the commitment leg.
    assert!(
        min_range < 40.0,
        "closest approach was {min_range:.1} units — the hull never committed to          a run at all"
    );
    assert!(
        escape_ticks > 0,
        "the hull's own Steering policy never reached the escape leg"
    );

    // 3. THE regression assertion. One radian is far outside the authored
    //    steering deadband and far inside the reciprocal a redirected hull ends
    //    up flying, so it separates the two behaviours without pinning the
    //    doctrine's exact tracking response.
    assert!(
        worst_escape_heading_error < 1.0,
        "the hull drifted {worst_escape_heading_error:.2} rad off the escape          heading it froze at the merge, over {escape_ticks} escape-leg ticks          (closest approach {min_range:.1}). Holding that heading IS the          commitment; a hull whose own backfilled Navigation stood its doctrine          down turns back onto its waypoint and ends up flying the reciprocal."
    );
}

/// **The destroyer's attack pass is a LOOP: it passes, breaks off, and passes
/// again (issue #937).**
///
/// Every other probe of `movement_attack_pass.toml` stops at the first leg.
/// `a_targeted_destroy_objective_does_not_cancel_the_hulls_own_attack_pass`
/// above asserts one escape happens and that its frozen heading is held;
/// `the_composed_player_destroyer_boots_backfilled_and_flies` asserts the hull
/// moves at all. A destroyer that ran in ONCE and then never re-opened the range
/// satisfied both, and that is precisely what shipped: in a live engagement the
/// hull merged and then ground along at contact range beside the ship it had
/// just rammed — the "orbits at ~range 5 instead of making attack runs" report.
///
/// The cause was not the doctrine and not the posture gate. `inbound` hands off
/// to `escape` on a closest-approach detector whose load-bearing conjunct is
/// `fact(closing_rate) < param(closing_rate_epsilon)`, and `closing_rate` is the
/// radial component of the RELATIVE velocity, reconstructed from both ships'
/// `(yaw, forward_speed)`. `build_world_snapshot` published every entity's
/// heading straight off its render `Transform`, which carries the negation
/// `sync_ship_position` applies — so every target's velocity came back mirrored
/// and the detector read "still closing" for a hull that had already flown past.
/// The unit half of that is
/// `ai::server::tests::the_snapshot_publishes_headings_in_the_simulations_own_convention`;
/// only a live run can say what it cost the doctrine.
///
/// The measurement is the four-phase SEQUENCE, in order, because no single
/// scalar separates the two behaviours: a jammed hull still has a tiny closest
/// approach, still publishes a pass surface, and still spends the engagement at
/// red alert. What only a cycling hull does is re-open the range past its own
/// authored `commit_range` after a merge and then close again.
#[test]
fn the_composed_destroyer_passes_breaks_off_and_passes_again() {
    use project_phoenix::server_app::Ship;
    use project_phoenix::ship::helm_ai::HelmPassSurface;
    use project_phoenix::ship::state::ShipRedAlert;

    // The authored trigger range this hull commits a run inside of, read off the
    // shipped template through the real (include-resolving) load path rather
    // than restated here, so a retune in TOML retunes this test with it.
    let commit_range = project_phoenix::entity_includes::load_entity_config(
        "assets/entities/alliance_destroyer.toml",
    )
    .expect("the shipped destroyer resolves")
    .helm_console
    .as_ref()
    .and_then(|h| h.steering_ai.as_ref())
    .and_then(|ai| ai.param.get("commit_range"))
    .copied()
    .expect("the composed steering axis authors `commit_range`");

    let dt = 1.0 / 30.0;
    let args = HeadlessArgs {
        world_path: "assets/worlds/probe_attack_pass_cycle.toml".into(),
        ship_path: "assets/entities/alliance_destroyer.toml".into(),
        dt,
        max_ticks: ticks_for_sim_seconds(60.0, dt),
        deterministic: true,
        ..test_args()
    };
    let mut app = build_headless_app(&args).expect("the composed destroyer must build an app");

    // The phases, advanced strictly in order — reaching phase N is a claim that
    // all of 1..N happened, in sequence, in one run.
    //
    //   1  merged     — closed to inside a fifth of `commit_range`: a real pass,
    //                   not a long-range exchange.
    //   2  broke off  — its own Steering policy reached the commitment leg AFTER
    //                   that merge. Read off `HelmPassSurface`, which the policy
    //                   publishes, so this says the DOCTRINE committed rather
    //                   than that the hull happened to drift outward.
    //   3  re-opened  — past `commit_range`, i.e. all the way back outside its
    //                   own run-in trigger. This is THE phase the jam could not
    //                   reach: a hull stuck at contact range never gets here.
    //   4  re-passed  — merged again. The loop closed.
    let mut phase = 0u8;
    let mut surface_ticks = 0usize;
    let mut red_ticks = 0usize;
    let mut min_range = f32::MAX;
    let mut max_range_since_merge = 0.0f32;
    let mut range_ticks = 0usize;
    let mut contact_ticks = 0usize;
    let merge_range = commit_range / 5.0;

    for _ in 0..args.max_ticks {
        run(&mut app, 1);
        // The one hostile this world spawns. It dies partway through a healthy
        // run, which is why every phase transition is latched rather than
        // sampled at the end.
        let hostile = {
            let mut q = app
                .world_mut()
                .query_filtered::<&ShipPhysics, (With<Ship>, Without<LocalShip>)>();
            q.iter(app.world()).next().map(|p| (p.x, p.z))
        };
        let mut q = app.world_mut().query_filtered::<(
            &ShipPhysics,
            Option<&ShipRedAlert>,
            Option<&HelmPassSurface>,
        ), With<LocalShip>>();
        let Ok((physics, alert, pass)) = q.single(app.world()) else {
            continue;
        };
        if alert.is_some_and(|a| a.0) {
            red_ticks += 1;
        }
        let escaping = pass.is_some_and(|p| p.escape);
        if pass.is_some() {
            surface_ticks += 1;
        }
        let Some((hx, hz)) = hostile else { continue };
        let range = ((physics.x - hx).powi(2) + (physics.z - hz).powi(2)).sqrt();
        range_ticks += 1;
        min_range = min_range.min(range);
        if range < merge_range {
            contact_ticks += 1;
        }
        if phase > 0 {
            max_range_since_merge = max_range_since_merge.max(range);
        }
        phase = match phase {
            0 if range < merge_range => 1,
            1 if escaping => 2,
            2 if range > commit_range => 3,
            3 if range < merge_range => 4,
            p => p,
        };
    }

    // Liveness first: without it every phase claim below could be vacuous on a
    // world where the two hulls never met.
    assert!(
        range_ticks > 0 && red_ticks > 0 && surface_ticks > 0,
        "the engagement never happened: {range_ticks} ticks with a hostile in \
         world, {red_ticks} at red alert, {surface_ticks} publishing a pass \
         surface"
    );
    assert!(
        phase >= 1,
        "closest approach was {min_range:.1} units against a {merge_range:.0}-unit \
         merge threshold — the hull never made a pass at all"
    );
    assert!(
        phase >= 2,
        "the hull merged at {min_range:.1} units and its Steering policy never \
         reached the commitment leg. Closest approach with no escape is the \
         signature of a closest-approach detector that cannot fire."
    );
    // THE regression assertion. A destroyer that merges and then hangs on its
    // target reaches phase 2 on a lucky tick and stops; only a hull that really
    // broke off gets all the way back outside its own run-in trigger.
    assert!(
        phase >= 3,
        "after breaking off, the hull only ever re-opened to \
         {max_range_since_merge:.1} units against its own authored \
         `commit_range` of {commit_range:.0} ({contact_ticks} of {range_ticks} \
         ticks spent inside {merge_range:.0} units). It committed to the outward \
         heading and did not get out — which is what grinding along at contact \
         range looks like from the outside."
    );
    assert_eq!(
        phase, 4,
        "the hull passed, broke off and re-opened to {max_range_since_merge:.1} \
         units, then never came back in. The doctrine is a LOOP; a hull that \
         leaves and does not return has stopped attacking."
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Issue #876 — the other two composed player hulls, end to end
// ─────────────────────────────────────────────────────────────────────────────

/// One tick's reading of a composed hull's own doctrine, taken off the ship
/// rather than inferred from where it ended up.
///
/// `HelmPassSurface` is published by the ship's own Steering POLICY (the planner
/// never writes it), so the leg booleans say which leg the DOCTRINE reached;
/// physics says whether the planner then flew it. `ShipRedAlert` is the switch
/// `posture` is seeded from, so sampling the two together is what makes "when
/// clear / at red alert" a measurement rather than a hope.
struct DoctrineSample {
    red_alert: bool,
    surface: Option<project_phoenix::ship::helm_ai::HelmPassSurface>,
    self_pos: [f32; 2],
    self_yaw: f32,
    hostile_pos: Option<[f32; 2]>,
}

/// Tick the app once and read the LocalShip's doctrine surface, its physics and
/// the one hostile it is fighting.
///
/// The hostile is "the ship that is not the LocalShip" — `probe_duel.toml`
/// spawns exactly one — and it is needed because every geometric claim below is
/// about the RELATIVE geometry the doctrine is flying, never a world position.
fn sample_doctrine(app: &mut App) -> Option<DoctrineSample> {
    use project_phoenix::ship::helm_ai::HelmPassSurface;
    use project_phoenix::ship::state::ShipRedAlert;

    run(app, 1);
    let (self_pos, self_yaw, red_alert, surface) = {
        let mut q = app.world_mut().query_filtered::<(
            &ShipPhysics,
            Option<&ShipRedAlert>,
            Option<&HelmPassSurface>,
        ), With<LocalShip>>();
        let (physics, alert, pass) = q.single(app.world()).ok()?;
        (
            [physics.x, physics.z],
            physics.yaw,
            alert.is_some_and(|a| a.0),
            pass.cloned(),
        )
    };
    let hostile_pos = {
        let mut q = app
            .world_mut()
            .query_filtered::<&ShipPhysics, Without<LocalShip>>();
        q.iter(app.world()).map(|p| [p.x, p.z]).next()
    };
    Some(DoctrineSample {
        red_alert,
        surface,
        self_pos,
        self_yaw,
        hostile_pos,
    })
}

/// World bearing from one planar point to another, in the same frame
/// `ShipPhysics::yaw` is expressed in (forward is `-Z`, starboard `+X`).
fn bearing_to(from: [f32; 2], to: [f32; 2]) -> f32 {
    simmath::atan2(to[0] - from[0], -(to[1] - from[1]))
}

fn wrap_pi(a: f32) -> f32 {
    (a + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI
}

/// **The composed player battleship takes a predictive firing position, and only
/// once the alert is up (issue #876 AC2).**
///
/// `alliance_battleship.toml` takes Engines, Steering and Impulse from
/// `fragments/ai/movement_artillery.toml` and tunes the envelope by `param`
/// alone.
///
/// The discriminating metric is the LEAD. Holding station at range looks much
/// like ordinary doctrine travel at `maintain_range = 38` — both end up slow and
/// pointed roughly at the enemy. What only `hold_artillery_position` does is
/// point the bow at where the target WILL BE when this hull's bolt arrives, so
/// against a moving target the bow sits off the live bearing by a real angle.
/// A hull flying ordinary travel drives that error into its deadband instead.
#[test]
fn the_composed_player_battleship_holds_a_leading_gun_line_only_at_red_alert() {
    let dt = 1.0 / 30.0;
    let args = HeadlessArgs {
        world_path: "assets/worlds/probe_duel.toml".into(),
        ship_path: "assets/entities/alliance_battleship.toml".into(),
        dt,
        max_ticks: ticks_for_sim_seconds(45.0, dt),
        deterministic: true,
        ..test_args()
    };
    let mut app = build_headless_app(&args).expect("the composed battleship must build an app");

    let mut clear_ticks = 0usize;
    let mut hold_ticks = 0usize;
    let mut hold_while_clear = 0usize;
    let mut defensive_ring_ticks = 0usize;
    let mut worst_lead = 0.0f32;
    let mut lead_ticks = 0usize;
    let mut lead_speed = 0.0f32;
    let mut hold_speed = f32::NAN;
    let mut deadband = f32::NAN;

    for _ in 0..args.max_ticks {
        let Some(s) = sample_doctrine(&mut app) else {
            continue;
        };
        if !s.red_alert {
            clear_ticks += 1;
        }
        let Some(pass) = s.surface else {
            continue;
        };
        if pass.recover {
            defensive_ring_ticks += 1;
        }
        if !pass.artillery_hold {
            continue;
        }
        hold_ticks += 1;
        lead_speed = pass.artillery_lead_speed;
        hold_speed = pass.artillery_hold_speed;
        deadband = pass.tracking_deadband_rad;
        if !s.red_alert {
            hold_while_clear += 1;
        }
        if let Some(hostile) = s.hostile_pos {
            let error = wrap_pi(bearing_to(s.self_pos, hostile) - s.self_yaw).abs();
            worst_lead = worst_lead.max(error);
            if error > deadband {
                lead_ticks += 1;
            }
        }
    }

    assert!(
        clear_ticks > 0,
        "the alert was up from the first tick, so this run never observed the \
         doctrine's defensive half at all"
    );
    // The posture gate, as an INVARIANT rather than as this test's mutation
    // canary: the gate's own proof is
    // `authored_ai_pins::the_artillery_doctrine_rests_defensive_until_red_alert`,
    // which fails by name when the guard is removed. In this world the hostile
    // spawns at the band's own inner edge, so a hull with the gate mutated out
    // still cannot reach `hold` before its captain calls the alert — which is
    // exactly why the guard needs a unit truth table and not only a live run.
    assert_eq!(
        hold_while_clear, 0,
        "the battleship established its firing position on {hold_while_clear} \
         ticks with the alert DOWN. The doctrine holds position at standoff range \
         until the captain presses it."
    );
    assert!(
        hold_ticks > 100,
        "only {hold_ticks} ticks on the gun line in 45 s of a live engagement. A \
         composed hull whose travel axes failed to compose still loads, still \
         backfills and still reports every station crewed — it just publishes no \
         leg and flies ordinary doctrine travel."
    );
    assert!(
        defensive_ring_ticks > 0,
        "the hull never once held its standoff ring, so `shadow` resolved no yaw \
         verb: the six recovery scalars the fragment authors did not reach the \
         host and the defensive leg is flying nothing."
    );
    // The hull's OWN armament reached the host: the lead is solved at the flight
    // speed of this ship's longest-reaching blaster, which is a reading of the
    // weapon rather than an authored copy of its speed.
    assert!(
        lead_speed > 0.0,
        "the artillery hold published a lead speed of {lead_speed}, so the \
         intercept degenerates to aiming at the live position — this hull's \
         blaster bank did not reach the planner"
    );
    assert_eq!(
        hold_speed, 0.0,
        "the authored hold throttle did not reach the host: an artillery platform \
         that keeps closing is not an artillery platform"
    );
    // THE DISCRIMINATOR, and it is a MAJORITY rather than a maximum: one tick of
    // large bearing error proves nothing (a shove from a collision turns a
    // station-keeping hull too), but a bow that sits outside its own authored
    // tracking deadband for most of the phase is a bow that is deliberately not
    // on the target. The threshold is `tracking_deadband_rad` as the host
    // published it, so a designer retuning this hull's steering response retunes
    // the test with it — and it is exactly the error an ordinary tracking leg
    // drives to zero and holds.
    //
    // Deliberately NOT a speed assertion. That `artillery_hold_speed` reached the
    // host is asserted above, off the surface, which is the authored claim; the
    // hull's MEASURED speed is not, because a shove — a collision, a torpedo
    // detonation — gives a station-keeping ship real momentum it never commanded,
    // and a run of this world contains several.
    assert!(
        lead_ticks * 2 > hold_ticks,
        "the battleship's bow sat outside its own {deadband:.3} rad tracking          deadband on only {lead_ticks} of {hold_ticks} holding ticks (worst          {worst_lead:.3} rad). Pointing at the INTERCEPT rather than at the ship          is the whole of what `hold_artillery_position` does; a hull tracking the          live bearing drives that error into the deadband and stays there."
    );
}

/// **The composed player cruiser flies a broadside ring and breaks it for a
/// torpedo run, and neither leg before the alert is up (issue #876 AC1).**
///
/// `alliance_cruiser.toml` takes Engines and Steering from
/// `fragments/ai/movement_broadside_orbit.toml` and tunes them with three
/// `param` keys and nothing else.
///
/// Every claim below is read off `HelmPassSurface`, which the ship's own Steering
/// POLICY publishes by resolving its `yaw` channel — so `combat_orbit` and
/// `torpedo_bearing` say which leg the DOCTRINE reached, not where the hull
/// happened to end up. That distinction is the whole point of measuring here: a
/// range or speed assertion cannot tell a doctrine apart from ordinary travel
/// that arrived at a similar place, and this hull's previous, WITHDRAWN attempt
/// held a plausible-looking range while Channel 3 flew it.
///
/// Two probes, because the two legs dominate in opposite worlds and a test that
/// only saw one of them would pass on half a doctrine:
///
/// * `probe_duel` — an ACTIVE hostile that closes and shoots. The ring is what
///   the hull flies almost throughout, which is where "works its arcs in a
///   tightened orbit at red alert" is actually demonstrated.
/// * `probe_aggressor` — a PASSIVE hostile. Nothing ever strips its shields, so
///   the torpedo run is entered on the hull's own loaded battery and held; this
///   is where the run leg, and the tube bearing it exists to buy, are visible.
#[test]
fn the_composed_player_cruiser_rings_its_target_and_breaks_off_to_bear_its_tubes() {
    use project_phoenix::ship::helm_ai::HelmPassSurface;
    use project_phoenix::ship::state::ShipRedAlert;

    struct Legs {
        clear: usize,
        orbit: usize,
        run: usize,
        orbit_while_clear: usize,
        run_while_clear: usize,
        surface_ticks: usize,
        /// How many ships dealt any damage over the sampled window, and how
        /// much in total. Not a doctrine claim — the liveness precondition the
        /// leg counts are only meaningful against (see below).
        sides_dealing: usize,
        damage_dealt: f32,
    }

    let sample = |world: &str, secs: f64| -> Legs {
        let dt = 1.0 / 30.0;
        let args = HeadlessArgs {
            world_path: world.into(),
            dt,
            max_ticks: ticks_for_sim_seconds(secs, dt),
            // Pinned explicitly so a future re-bless of `probe_duel.toml`
            // cannot silently move the window these leg counts were measured
            // on — it happens to be that world's own `[global] seed` again
            // today, as it was before #923, but the two are free to diverge.
            // Re-measured for #897's generator swap: the previous pin (838)
            // rings for only 277 of 1351 sampled frames on this generator,
            // well under the floor below, while 3 rings for 1332 of them with
            // both hulls trading fire. See `probe_duel.toml` for the per-seed
            // table.
            seed: Some(3),
            deterministic: true,
            ..test_args()
        };
        let mut app = build_headless_app(&args).expect("the composed cruiser must build an app");
        let mut l = Legs {
            clear: 0,
            orbit: 0,
            run: 0,
            orbit_while_clear: 0,
            run_while_clear: 0,
            surface_ticks: 0,
            sides_dealing: 0,
            damage_dealt: 0.0,
        };
        for _ in 0..args.max_ticks {
            run(&mut app, 1);
            let mut q = app.world_mut().query_filtered::<(
                Option<&ShipRedAlert>,
                Option<&HelmPassSurface>,
            ), With<LocalShip>>();
            let Ok((alert, pass)) = q.single(app.world()) else {
                continue;
            };
            let red = alert.is_some_and(|a| a.0);
            if !red {
                l.clear += 1;
            }
            let Some(pass) = pass else { continue };
            l.surface_ticks += 1;
            if pass.combat_orbit {
                l.orbit += 1;
                if !red {
                    l.orbit_while_clear += 1;
                }
            }
            if pass.torpedo_bearing {
                l.run += 1;
                if !red {
                    l.run_while_clear += 1;
                }
            }
        }
        let report = build_report(&mut app, &args, 0.0);
        l.sides_dealing = report
            .damage_by_ship
            .values()
            .filter(|d| d.damage_dealt > 0.0)
            .count();
        l.damage_dealt = report.damage_by_ship.values().map(|d| d.damage_dealt).sum();
        l
    };

    let duel = sample("assets/worlds/probe_duel.toml", 45.0);
    let aggressor = sample("assets/worlds/probe_aggressor.toml", 30.0);

    // LIVENESS, first — every leg count below is meaningless without it. A duel
    // that stalls at standoff parks the hull in a wide holding pattern that the
    // Steering policy still publishes as `combat_orbit`, so `duel.orbit` alone
    // can be satisfied by two ships circling each other and never shooting. This
    // is the trap the pre-#895 seed 838 fell into. Requiring BOTH hulls to have
    // actually landed damage over the window pins the ring assertion to a real
    // exchange, whatever seed a future re-bless picks.
    assert_eq!(
        duel.sides_dealing, 2,
        "only {} of the two duelists landed any damage across 45 s ({:.0} total) \
         — the ring counts below would be measuring a standoff, not a fight",
        duel.sides_dealing, duel.damage_dealt
    );

    // The surface exists at all. A hull whose travel axes failed to compose still
    // loads, still backfills and still reports every station crewed — it just
    // publishes no leg and flies ordinary doctrine travel, which is precisely the
    // state this hull was left in before AC1.
    assert!(
        duel.surface_ticks > 100 && aggressor.surface_ticks > 100,
        "the cruiser published a pass surface on only {} / {} ticks: its Steering \
         policy is not resolving a yaw verb, so no leg of the doctrine is being \
         flown at all",
        duel.surface_ticks,
        aggressor.surface_ticks
    );

    // THE RING. Asserted in the duel, where an active hostile keeps the alert up
    // and the ring is the leg the engagement is actually fought on.
    //
    // RE-BLESSED at issue #907, from `> 300` to `> 250` (observed: 280). The
    // cause is identity, not doctrine: a hull's orbit DIRECTION is drawn from
    // `composite_rng` keyed on, among other things, the ship's own uuid
    // (`helm_ai`'s `ORBIT_DIRECTION_MEMORY`), and #907 replaced randomly-minted
    // uuids with tick-scoped ones. Different ids, different derived directions,
    // different duel geometry, slightly fewer ticks on the ring. Nothing about
    // the leg changed — the liveness assertion above still requires BOTH hulls
    // to have landed damage, and the pass surface is still published on >100
    // ticks, so what this number cannot silently become is a standoff.
    assert!(
        duel.orbit > 250,
        "only {} ticks on the fighting ring across 45 s of a live duel — the hull \
         is not flying a broadside orbit",
        duel.orbit
    );

    // THE TORPEDO RUN. Asserted in the aggressor probe, where the hull carries a
    // loaded battery it never gets to spend, so the leg is entered on its own
    // readiness and held. This is the leg whose ABSENCE let Channel 3 overwrite
    // the ring solution every tick, which is what withdrew this doctrine the first
    // time round.
    assert!(
        aggressor.run > 100,
        "the cruiser never broke its ring to bring its tubes to bear ({} run \
         ticks). With no torpedo leg of its own the hull is dragged bow-on by \
         `ArcBearingRequest` instead, which is a facing the doctrine did not \
         choose and cannot see",
        aggressor.run
    );

    // THE POSTURE GATE, on both legs and in both worlds. Neither half of the
    // aggressive doctrine may be flown with the captain stood down.
    for (name, l) in [("duel", &duel), ("aggressor", &aggressor)] {
        assert!(
            l.clear > 0,
            "{name}: the alert was up from the first tick, so this run never \
             observed the doctrine's defensive half at all"
        );
        assert_eq!(
            (l.orbit_while_clear, l.run_while_clear),
            (0, 0),
            "{name}: the cruiser flew its fighting ring on {} ticks and its \
             torpedo run on {} ticks with the alert DOWN. Both belong to the \
             pressed half of the doctrine; when clear this hull holds a standoff \
             ring and nothing else",
            l.orbit_while_clear,
            l.run_while_clear
        );
    }
}

/// **The composed cruiser's ring flies the facing its OWN doctrine solved, in a
/// real run, with Channel 3 asking for a different one (issue #918).**
///
/// This is #918's no-sawtooth pin at the level the defect was measured on. The
/// two probes are the pair the doctrine test above uses and for the same reason:
/// `probe_duel` is where the ring dominates, `probe_aggressor` is the world the
/// original measurement came from.
///
/// Measured as the admitted yaw against the yaw the ship's own planner solved
/// this tick, decoded exactly as `ai_helm_steering` decodes it. Deliberately NOT
/// as a range: a hull carries momentum either way, so a radius that looks steady
/// says nothing about whether the steering command was the doctrine's, and this
/// doctrine's withdrawn first attempt held a plausible radius while Channel 3
/// flew it.
///
/// ## The control was re-measured after issue #896's freeze fix (this batch)
///
/// The doc comment this replaces asserted `requested > 0` on the duel probe —
/// a standing arc-bearing request against the ring — pinned to a pre-#896
/// count (367 of 1351 ticks under seed 3) that the world header had flagged
/// as un-remeasured. Re-measured on the fixed tree (the `[ai_profile]` blocks
/// that stop an Alliance hull losing AI fidelity mid-manoeuvre): 346 ring
/// ticks, `requested == 0`, `overwritten == 0`, worst error `0.0`, in BOTH
/// probes. That is not the control rotting into a false pass; it is a real
/// change to this doctrine's own geometry:
///
///   * The two phaser banks' `auto_arc_deg = 180` abut exactly on the ring's
///     beam line, so at least one bank is always `Ready` there — the phaser
///     family can never qualify for a request while the hull holds the ring
///     (see the "not changed" note on `alliance_cruiser.toml`).
///   * The three torpedo tubes (fore/aft, `fire_arc_deg = 90`) genuinely
///     cannot bear on the ring — but this hull authors its OWN
///     `hold_torpedo_bearing` leg (`movement_broadside_orbit.toml`), which now
///     reliably claims a loaded, ready tube and breaks the ring to fire it
///     BEFORE Channel 3 ever observes a loaded-but-out-of-arc emitter. Under
///     the freeze fix combat sustains long enough for that leg to actually
///     fire (`TorpedoLaunched: 4` in this run) — pre-fix the hull could lose
///     AI fidelity mid-ring and never complete the leg, which is one way a
///     stale loaded tube could have stood in front of Channel 3 instead.
///
/// So a standing arc-bearing request is no longer an expected phenomenon for
/// THIS composed doctrine at all — the doctrine now resolves its own bearing
/// needs before Channel 3 is asked. The control this test needs is not "a
/// request stood at some point" but "this was a live, contested duel and not
/// an idle scenario with nothing to decline" — otherwise `overwritten == 0`
/// would be trivially true of a hull with no target. That control is now
/// asserted directly off the same run's report: nonzero damage dealt and at
/// least one torpedo launch, which only happen if both hulls were actively
/// fighting across the window.
#[test]
fn the_composed_cruisers_ring_is_not_overwritten_by_an_arc_bearing_request() {
    use project_phoenix::ai::decode_steering_from_facing;
    use project_phoenix::ship::helm_ai::HelmPassSurface;
    use project_phoenix::ship::helm_planner::HelmMotionPlan;
    use project_phoenix::ship_plugin::{LastHelmInput, PendingArcBearingRequest};

    struct Ring {
        ticks: usize,
        overwritten: usize,
        requested: usize,
        worst: f32,
        dealt: f32,
        torpedoes_launched: u64,
    }

    let sample = |world: &str, secs: f64| -> Ring {
        let dt = 1.0 / 30.0;
        let args = HeadlessArgs {
            world_path: world.into(),
            dt,
            max_ticks: ticks_for_sim_seconds(secs, dt),
            deterministic: true,
            ..test_args()
        };
        let mut app = build_headless_app(&args).expect("the composed cruiser must build an app");
        let mut r = Ring {
            ticks: 0,
            overwritten: 0,
            requested: 0,
            worst: 0.0,
            dealt: 0.0,
            torpedoes_launched: 0,
        };
        for _ in 0..args.max_ticks {
            run(&mut app, 1);
            let mut q = app.world_mut().query_filtered::<(
                Entity,
                &HelmPassSurface,
                &LastHelmInput,
                Option<&PendingArcBearingRequest>,
            ), With<LocalShip>>();
            let Ok((ship, pass, last, pending)) = q.single(app.world()) else {
                continue;
            };
            let standing = pending.and_then(|p| p.target).is_some();
            if !pass.combat_orbit {
                continue;
            }
            let steering = last.steering;
            let Some(plan) = app
                .world()
                .resource::<HelmMotionPlan>()
                .ships
                .get(&ship)
                .copied()
            else {
                continue;
            };
            r.ticks += 1;
            if standing {
                r.requested += 1;
            }
            let solved = decode_steering_from_facing(plan.motion.desired_facing_local.to_array());
            let error = (steering - solved).abs();
            r.worst = r.worst.max(error);
            if error > 1e-3 {
                r.overwritten += 1;
            }
        }
        // Full-run report, taken from this same app/seed after every tick has
        // played out — the liveness control below reads off it.
        let report = build_report(&mut app, &args, 0.0);
        r.dealt = report.damage_by_ship.values().map(|l| l.damage_dealt).sum();
        r.torpedoes_launched = report
            .message_counts
            .get("TorpedoLaunched")
            .copied()
            .unwrap_or(0);
        r
    };

    let duel = sample("assets/worlds/probe_duel.toml", 45.0);
    let aggressor = sample("assets/worlds/probe_aggressor.toml", 30.0);

    // The control: the duel probe is a genuinely live, contested engagement —
    // both damage exchanged and a torpedo actually launched — so `overwritten
    // == 0` below is proof the ring held under real combat pressure, not a
    // vacuous read of an idle scenario with nothing to decline.
    assert!(
        duel.dealt > 0.0 && duel.torpedoes_launched > 0,
        "the duel probe did not read as live combat (dealt={}, torpedoes_launched={}) \
         across {} ring ticks — this probe proves nothing about the ring holding \
         under contested fire, because there was no contested fire to hold under",
        duel.dealt,
        duel.torpedoes_launched,
        duel.ticks
    );

    // The request itself is now expected to be absent on this composed
    // doctrine: the phasers' abutting 180-degree arcs mean a bank is always
    // ready on the ring, and the hull's own torpedo-bearing leg now reliably
    // claims a loaded tube before Channel 3 ever sees it out of arc. A
    // standing request here would mean one of those two guarantees broke.
    assert_eq!(
        duel.requested, 0,
        "the cruiser's Weapons had a standing arc-bearing request on {} of {} ring \
         ticks — either a phaser bank went unready with the other out of arc, or \
         the hull's own torpedo-bearing leg stopped claiming a loaded tube before \
         Channel 3 saw it. Both are regressions from the composed doctrine this \
         probe pins",
        duel.requested, duel.ticks
    );

    // The `duel` floor was re-blessed from 300 to 250 at issue #907 (observed:
    // 280) for the reason recorded at the sibling assertion in
    // `the_composed_player_cruiser_rings_its_target_and_breaks_off_to_bear_its_tubes`:
    // orbit direction is derived from the ship's uuid, and #907 changed how
    // uuids are minted. It is a floor on "did this run measure the leg at all",
    // not a doctrine assertion, and 250 still says yes.
    for (name, ring, min_ticks) in [("duel", &duel, 250), ("aggressor", &aggressor, 50)] {
        assert!(
            ring.ticks >= min_ticks,
            "{name}: only {} ticks were flown on the fighting ring — this run did not \
             measure the leg the issue is about",
            ring.ticks
        );
        assert_eq!(
            ring.overwritten, 0,
            "{name}: {} of {} ring ticks were flown at a yaw the ship's own doctrine \
             did not solve (worst {}). That is the sawtooth: an arc-bearing request \
             overwriting the ring tangent after the planner had already solved it",
            ring.overwritten, ring.ticks, ring.worst
        );
    }
}

/// Issue #891 stage 2, end to end in the REAL app: the production schedule
/// hands every AI host a live world-flag chain. A flag-gated Captain doctrine
/// on the fully-backfilled player ship holds Red Alert down while the
/// scenario's flag store is clear, and raises it on the ticks after the flag
/// appears — the same base `WorldContentRuntime` store a world trigger's
/// `set_flag` action writes, so this is the "scenarios influence doctrine
/// without surrendering progression authority" arc (#774 US11) with only the
/// trigger's own firing elided.
///
/// Unit fixtures register the host systems by hand; what this adds is the
/// wiring proof that the SHIPPED schedule passes the chain — a host whose
/// `runtime`/`layers` params were dropped would go green everywhere except
/// here.
#[test]
fn a_world_flag_drives_a_backfilled_doctrine_in_a_real_run() {
    use project_phoenix::console::captain::server::CaptainAiPolicy;
    use project_phoenix::ship::state::ShipRedAlert;

    let args = test_args();
    let mut app = build_headless_app(&args).expect("app should build");
    // Boot to InProgress with the crew backfilled.
    run(&mut app, 30);

    // Swap the shipped Captain doctrine for one whose ONLY question is the
    // world flag: raise on `flag(battle_stations)`, stand down otherwise.
    let ship = app
        .world_mut()
        .query_filtered::<Entity, With<LocalShip>>()
        .single(app.world())
        .expect("exactly one LocalShip");
    app.world_mut().entity_mut(ship).insert(CaptainAiPolicy(
        project_phoenix::ai::policy::AiPolicy {
            params: project_phoenix::world::flags::AiParams::new(),
            rules: vec![
                project_phoenix::ai::policy::AiPolicyRule {
                    priority: 10,
                    channel: project_phoenix::entities::config::CAPTAIN_RED_ALERT_CHANNEL.into(),
                    when: project_phoenix::world::flags::parse_predicate("flag(battle_stations)")
                        .expect("guard parses"),
                    verb: project_phoenix::ai::policy::AiPolicyVerb::SetRedAlert(true),
                },
                project_phoenix::ai::policy::AiPolicyRule {
                    priority: 0,
                    channel: project_phoenix::entities::config::CAPTAIN_RED_ALERT_CHANNEL.into(),
                    when: project_phoenix::world::flags::parse_predicate("true")
                        .expect("guard parses"),
                    verb: project_phoenix::ai::policy::AiPolicyVerb::SetRedAlert(false),
                },
            ],
            idle: false,
            machine: None,
        },
    ));

    let red_alert = |app: &mut App| -> bool {
        let mut q = app
            .world_mut()
            .query_filtered::<Option<&ShipRedAlert>, With<LocalShip>>();
        q.single(app.world())
            .expect("exactly one LocalShip")
            .map(|r| r.0)
            .unwrap_or(false)
    };

    // Flag CLEAR → the guard reads false every tick and the alert stays (or
    // goes) down, whatever the shipped doctrine did during boot.
    run(&mut app, 60);
    assert!(
        !red_alert(&mut app),
        "with the scenario flag clear the flag-gated doctrine must hold the \
         alert down"
    );

    // Set the flag on the LIVE base store — exactly what a world trigger's
    // `set_flag` action does — and the SAME guard raises the alert.
    app.world_mut()
        .resource_mut::<project_phoenix::world::server::WorldContentRuntime>()
        .flags
        .set_flag("battle_stations");
    run(&mut app, 60);
    assert!(
        red_alert(&mut app),
        "once the scenario sets the flag the same doctrine must raise Red Alert \
         through the production schedule's flag chain"
    );
}

/// **A hull's phaser reach is its authored `beam_range`, whatever the radar
/// slot is doing, in a real engagement (issue #955).**
///
/// This test used to assert the opposite (#923): that the cruiser's effective
/// reach `beam_range × ModifierSlot::RadarRange` *climbed to* nominal at combat
/// stations, and dipped below it the rest of the time. That assertion was
/// pinning a coupling that should never have existed — reach is a property of
/// the gun, and power buys DAMAGE (`ModifierSlot::PhaserDamage`), not distance.
/// #955 removed the multiplication from every firing and reach-reporting path,
/// so the old claim is now false by construction and the honest claim is its
/// inverse: reach must not move at all.
///
/// It is asserted through the SHIPPED schedule rather than by hand. The reading
/// is the production reach fact — `entity_direct_fire_range`, published on
/// `AiWorldEntity::direct_fire_range` and consumed by the standoff-ring
/// doctrine — which is exactly the projection that carried the multiplier
/// before #955. `longest_usable_direct_fire_range` is a max over the ONLINE
/// banks, so its value is always either 0 (every bank down) or one of the
/// authored per-bank ranges; anything else means a multiplier crept back in.
/// A damaged or destroyed bank therefore reads as an authored number or as
/// zero, never as two thirds of one, which is what makes the set membership
/// robust in a live duel rather than brittle.
///
/// The control is the same one the old test carried, kept for the same reason:
/// the `RadarRange` slot must actually MOVE across the run, or the run proves
/// nothing about decoupling, because a slot pinned at 1.0 satisfies an
/// invariance claim trivially. It moves because the cruiser takes hits —
/// `apply_radar_damage_modifiers` drives the same slot off the tactical radar's
/// damage tier. It is deliberately NOT a power group any more: #955 removed
/// the sensors red-alert rule along with the coupling it paid for, and #952
/// retired the `sensors` power group outright in favour of `shields` — so
/// nothing the reactor does touches this slot, for the whole run or ever.
#[test]
fn a_cruisers_phaser_reach_never_leaves_its_authored_beam_range_in_a_live_duel() {
    use project_phoenix::messages::ModifierSlot;
    use project_phoenix::modifiers::ShipModifiers;

    // The authored numbers this test is about, read off the shipped hull rather
    // than restated — a retune of a bank retunes the assertion with it.
    let hull = project_phoenix::entity_includes::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .expect("the shipped cruiser composes");
    let authored_ranges: Vec<f32> = hull
        .weapons_console
        .as_ref()
        .map(|wc| wc.phaser_banks.iter().map(|b| b.beam_range).collect())
        .unwrap_or_default();
    assert!(
        !authored_ranges.is_empty() && authored_ranges.iter().all(|r| *r > 0.0),
        "the cruiser must author at least one phaser bank with a positive \
         beam_range, or there is no reach for this test to pin: {authored_ranges:?}"
    );
    let longest = authored_ranges
        .iter()
        .copied()
        .fold(0.0f32, |a, b| if b > a { b } else { a });

    let dt = 1.0 / 30.0;
    let args = HeadlessArgs {
        world_path: "assets/worlds/probe_duel.toml".into(),
        dt,
        max_ticks: ticks_for_sim_seconds(45.0, dt),
        deterministic: true,
        ..test_args()
    };
    let mut app = build_headless_app(&args).expect("probe_duel must build an app");

    // Every distinct reach the production fact reported, and every distinct
    // radar multiplier the slot held, across the whole run. The LocalShip is
    // spawned by `spawn_game_start_entities` once the run begins, so its uuid is
    // resolved lazily rather than before the first tick.
    let mut local_uuid: Option<uuid::Uuid> = None;
    let mut reaches: Vec<f32> = Vec::new();
    let mut radar_mults: Vec<f32> = Vec::new();
    let mut saw_longest = false;
    // The radar slot is DRIVEN here, at two points in the run, rather than
    // waited on.
    //
    // Until issue #952 it moved by itself, and by accident: the modifier table
    // starts empty, so the first sample read x1.0 and every later one read the
    // x0.667 the `sensors` power group wrote — two distinct values, and the
    // control below passed on an ordering artefact rather than on anything the
    // duel did. #952 retired that power group, and on an Alliance hull nothing
    // else in a duel can move this slot: their radar systems deliberately carry
    // no `[[hull.system_hull]]` entry, so `apply_radar_damage_modifiers` writes
    // a constant 0.0 however hard the ship is hit, and `probe_duel` authors no
    // dampening region.
    //
    // Writing it here under `RegionEffect` — a real producer of this slot —
    // makes the control assert what it always claimed to: that reach holds
    // while the slot moves underneath it. It also retires the seed dependence
    // the old comment had to warn about.
    let dampen_at = args.max_ticks / 3;
    let amplify_at = (args.max_ticks * 2) / 3;
    let drive_radar_slot = |app: &mut App, bonus: f32| {
        use project_phoenix::messages::ModifierSource;
        use project_phoenix::modifiers::Modifier;
        let mut q = app
            .world_mut()
            .query_filtered::<&mut ShipModifiers, With<LocalShip>>();
        let world = app.world_mut();
        if let Ok(mut mods) = q.single_mut(world) {
            mods.add_or_update(Modifier {
                source: ModifierSource::RegionEffect {
                    uuid: uuid::Uuid::from_u128(0x952),
                },
                slot: ModifierSlot::RadarRange,
                bonus,
            });
        }
    };
    for tick in 0..args.max_ticks {
        run(&mut app, 1);
        if tick == dampen_at {
            drive_radar_slot(&mut app, -0.5);
        } else if tick == amplify_at {
            drive_radar_slot(&mut app, 1.0);
        }

        if local_uuid.is_none() {
            let mut uuid_q = app
                .world_mut()
                .query_filtered::<&project_phoenix::entity_spawner::EntityUuid, With<LocalShip>>();
            local_uuid = uuid_q
                .single(app.world())
                .ok()
                .and_then(|u| uuid::Uuid::parse_str(&u.0).ok());
        }
        let Some(local_uuid) = local_uuid else {
            continue;
        };

        let mut mods_q = app
            .world_mut()
            .query_filtered::<&ShipModifiers, With<LocalShip>>();
        if let Ok(mods) = mods_q.single(app.world()) {
            let mult = mods.get(&ModifierSlot::RadarRange);
            if !radar_mults.iter().any(|m| (m - mult).abs() < 1e-6) {
                radar_mults.push(mult);
            }
        }

        let snapshot = app
            .world()
            .resource::<project_phoenix::ai::server::WorldSnapshot>();
        let Some(me) = snapshot.entities.iter().find(|e| e.uuid == local_uuid) else {
            continue;
        };
        let reach = me.direct_fire_range;
        if (reach - longest).abs() < 1e-3 {
            saw_longest = true;
        }
        if !reaches.iter().any(|r| (r - reach).abs() < 1e-6) {
            reaches.push(reach);
        }
    }

    for reach in &reaches {
        let legal = *reach == 0.0 || authored_ranges.iter().any(|a| (a - reach).abs() < 1e-3);
        assert!(
            legal,
            "the cruiser reported an effective direct-fire reach of {reach:.3} during a \
             live duel, which is neither zero (every bank offline) nor any authored \
             beam_range in {authored_ranges:?}. Something is scaling reach again — \
             #955 made reach a property of the gun, and power buys PhaserDamage \
             instead. All observed reaches: {reaches:?}"
        );
    }
    assert!(
        saw_longest,
        "the cruiser never once reported its longest authored reach ({longest:.2}) across \
         45 s of a live duel. Every observed value: {reaches:?}. Either the guns are \
         offline for the whole run or the reach fact is not being published at all, and \
         either way the invariance above passed vacuously"
    );
    // The control: the slot must actually have MOVED, or the invariance above
    // is satisfied trivially by a slot pinned at x1.0. Driven from the loop
    // (see the note there) rather than waited on, so this is a statement about
    // the harness now and not about the seed — it fails only if the write
    // stopped landing.
    assert!(
        radar_mults.len() > 2,
        "`ModifierSlot::RadarRange` held {} distinct value(s) ({radar_mults:?}) across \
         the run, but the loop writes a dampening and then an amplifying region \
         effect onto the cruiser, so it should have held three (nominal, crushed, \
         doubled). Whatever this probe measured, it did not measure reach holding \
         steady while the radar slot moved underneath it",
        radar_mults.len()
    );
}

/// **A sustained red-alert fight never reaches the exhaustion lock (#1003).**
///
/// The end-to-end half of the shed ladder. `modifiers::power_system::tick` slams
/// every group to `GROUP_LEVEL_MIN` and LOCKS the reactor the instant the
/// battery reaches 0, and nothing unlocks it until the charge climbs back to the
/// hull's `emergency_threshold` — a ship that fights hard enough to flatten its
/// own battery loses its drive, its guns and its shield regeneration at once,
/// for as long as the recovery takes. The authored SHED floors
/// (`min_reserve_helm` = 50, `min_reserve_weapons` = 25) exist so that an AI
/// crew cannot walk a ship into that on its own decisions: at 50 the helm
/// elevation is shed (total 7, a slow drain), at 25 the weapons elevation
/// follows (total 6, which every shipped reactor authors as a POSITIVE rate),
/// and the charge oscillates across the band between the weapons shed floor and
/// its RESTORE floor (`min_restore_weapons` = 35, with `min_restore_helm` = 60
/// one rung up). The gap between each pair is what makes that oscillation a
/// decision taken seconds apart instead of a per-tick flip.
///
/// A unit test on the shipped hull already walks the ladder rung by rung
/// (`console_ai::server::tests::the_shipped_power_policy_sheds_one_group_at_each_authored_floor`
/// and its no-lock sibling). This is the one that puts the claim in front of a
/// real fight: two hulls under their own AI crews, red alert raised by their own
/// captains, thrust commanded by their own helm doctrine, damage taken and
/// systems degrading — every input the ladder reads driven by the simulation
/// rather than by the test.
///
/// `probe_duel` is the vehicle because its default seed deliberately does NOT
/// resolve inside 60 s (see that world's own seed sweep): a duel that ends early
/// stops draining, and the seed that keeps both ships shooting for the whole
/// window is exactly the one this probe wants.
///
/// The control is the drain itself. A run in which no battery ever fell under
/// the helm floor would satisfy "never locked" without exercising a single step
/// of the ladder, so the probe insists that at least one reactor crossed it.
#[test]
fn neither_reactor_reaches_the_exhaustion_lock_across_a_seeded_duel() {
    use project_phoenix::entity_config::{POWER_HELM_RESERVE_PARAM, POWER_WEAPONS_RESERVE_PARAM};
    use project_phoenix::ship::power::{PowerAiPolicy, PowerConfigResource, ShipPowerSystem};
    use project_phoenix::simulation::Ship;
    use std::collections::BTreeMap;

    let dt = 1.0 / 30.0;
    let args = HeadlessArgs {
        world_path: "assets/worlds/probe_duel.toml".into(),
        dt,
        max_ticks: 0, // driven by hand below
        deterministic: true,
        ..test_args()
    };
    let mut app = build_headless_app(&args).expect("probe_duel must build an app");
    app.finish();
    app.cleanup();

    // Per-ship: lowest battery percentage seen, the set of ship-wide allocation
    // totals seen, and each hull's own authored SHED floors.
    //
    // No `Default`: a derived `min_pct` would be 0.0, which reads as "this
    // reactor flattened" and would quietly invert the `min_pct > 0.0` assertion
    // below for any ship constructed that way. Every construction site below
    // seeds `f64::MAX` explicitly.
    struct Reading {
        min_pct: f64,
        totals: Vec<u8>,
        floors: Option<(f64, f64)>,
    }
    let mut readings: BTreeMap<Entity, Reading> = BTreeMap::new();

    // Built once and reused: `run` takes the world back between samples, but a
    // `QueryState` only borrows it while it is iterating.
    let mut reactors = app.world_mut().query_filtered::<(
        Entity,
        &ShipPowerSystem,
        Option<&PowerConfigResource>,
        Option<&PowerAiPolicy>,
    ), With<Ship>>();

    // (entity, locked, battery pct, commanded total, authored shed floors)
    type SampledReactor = (Entity, bool, f64, u8, Option<(f64, f64)>);

    let total_ticks = ticks_for_sim_seconds(60.0, dt);
    for tick in 0..total_ticks {
        run(&mut app, 1);

        let sampled: Vec<SampledReactor> = reactors
            .iter(app.world())
            .map(|(e, power, cfg, policy)| {
                let capacity = cfg.map(|c| c.0.capacity).unwrap_or(0.0);
                let pct = if capacity > 0.0 {
                    (power.0.battery_charge / capacity) as f64 * 100.0
                } else {
                    f64::NAN
                };
                let floors = policy.and_then(|p| {
                    Some((
                        p.0.params.get(POWER_HELM_RESERVE_PARAM)?,
                        p.0.params.get(POWER_WEAPONS_RESERVE_PARAM)?,
                    ))
                });
                (e, power.0.locked(), pct, power.0.commanded_total(), floors)
            })
            .collect();

        for (ship, locked, pct, total, floors) in sampled {
            assert!(
                !locked,
                "{ship} hit the reactor exhaustion lock at tick {tick} \
                 ({:.1} s into the duel) with the battery at {pct:.2} %. The AI \
                 shed ladder is supposed to make that unreachable: helm is given \
                 back at `min_reserve_helm` and weapons at `min_reserve_weapons`, \
                 and the total those leave is one the hull's own `rates` recharge \
                 from. Totals this ship held before the lock: {:?}",
                tick as f64 * dt,
                readings
                    .get(&ship)
                    .map(|r| r.totals.clone())
                    .unwrap_or_default()
            );
            let entry = readings.entry(ship).or_insert(Reading {
                min_pct: f64::MAX,
                totals: Vec::new(),
                floors: None,
            });
            if pct.is_finite() {
                entry.min_pct = entry.min_pct.min(pct);
            }
            if !entry.totals.contains(&total) {
                entry.totals.push(total);
            }
            if entry.floors.is_none() {
                entry.floors = floors;
            }
        }

        if app.world().resource::<State<GamePhase>>().get() == &GamePhase::GameOver {
            break;
        }
    }

    assert!(
        readings.len() >= 2,
        "the duel should have carried two reactors; sampled {}",
        readings.len()
    );

    // The control: the ladder must actually have been walked. At least one
    // reactor has to have crossed its own helm floor, or "never locked" is a
    // statement about a fight that never spent any power.
    let crossed_helm = readings.values().any(|r| match r.floors {
        Some((helm, _)) => r.min_pct < helm,
        None => false,
    });
    let summary: Vec<String> = readings
        .iter()
        .map(|(ship, r)| {
            format!(
                "{ship}: min {:.2} %, totals {:?}, floors {:?}",
                r.min_pct, r.totals, r.floors
            )
        })
        .collect();
    assert!(
        crossed_helm,
        "no reactor in the duel ever fell under its own `min_reserve_helm`, so \
         the no-lock assertion above passed without a single step of the shed \
         ladder being exercised. Readings: {summary:?}"
    );
    for (ship, r) in &readings {
        assert!(
            r.min_pct > 0.0,
            "{ship} reached {:.2} % — that is the lock. Readings: {summary:?}",
            r.min_pct
        );
    }
}

// ── The fixed logical tick (issue #895) ─────────────────────────────────────

/// The logical tick counts fixed steps at the authored `[global] sim_tick_hz`,
/// not rendered frames: the same virtual span covers the same number of ticks
/// whatever the frame rate, and a frame whose accumulated time never reaches
/// the timestep advances the counter not at all.
#[test]
fn the_logical_tick_follows_the_authored_rate_not_the_frame_rate() {
    use project_phoenix::sim_tick::SimTick;

    let ticks_after = |dt: f64, frames: u64| -> u64 {
        let args = HeadlessArgs {
            world_path: "assets/worlds/patrol.toml".into(),
            dt,
            max_ticks: frames,
            deterministic: true,
            ..test_args()
        };
        let mut app = build_headless_app(&args).expect("app should build");
        run(&mut app, args.max_ticks);
        app.world().resource::<SimTick>().0
    };

    // `patrol.toml` authors no `sim_tick_hz`, so the serde default 60 Hz
    // applies. 121 frames at 60 fps = 120 stepping frames (the first update
    // establishes the time baseline with a zero delta) = exactly 120 ticks —
    // `build_headless_app` feeds `TimeUpdateStrategy` and `Time<Fixed>` the
    // identical `Duration`, so the accumulator never drifts a nanosecond.
    assert_eq!(
        ticks_after(1.0 / 60.0, 121),
        120,
        "at one frame per authored tick, every stepping frame is one tick"
    );

    // The same two virtual seconds at HALF the frame rate: 61 frames at
    // 30 fps still cover ~120 logical ticks (±1 for nanosecond rounding of
    // the 1/30 s frame against the 1/60 s timestep — the two are not exact
    // Duration multiples).
    let at_30fps = ticks_after(1.0 / 30.0, 61);
    assert!(
        (119..=120).contains(&at_30fps),
        "two virtual seconds at 30 fps must still cover ~120 logical ticks \
         (the authored 60 Hz), got {at_30fps} — the sim is stepping per frame \
         again"
    );
}

/// Command admission is per LOGICAL TICK, not per frame (issue #895
/// integration risk #1): inbound messages are drained once per frame in
/// `PreUpdate`, so if `admit_system_commands` still ran per frame, a frame
/// with zero fixed steps would clear-and-refill `AdmittedCommands` between
/// ticks, and a command could be wiped before any consumer saw it.
///
/// Drives the real backfilled app at FOUR frames per tick and pins both
/// halves: a frame that runs no step leaves the admitted set untouched, and
/// the injected command is admitted on a tick boundary. The `ai:` probe token
/// is unregistered, so it routes to the LocalShip, whose backfilled
/// helm-thrust system `operate_ai`s — the same admission path every AI
/// command takes (AGENTS.md #6).
#[test]
fn command_admission_moves_with_the_logical_tick_not_the_frame() {
    use project_phoenix::lobby::InboundMessage;
    use project_phoenix::messages::{AdmittedCommands, ClientMessage, SystemControlPayload};
    use project_phoenix::sim_tick::SimTick;

    const PROBE_TOKEN: &str = "ai:admission-probe";

    let dt = 1.0 / 60.0;
    let args = HeadlessArgs {
        world_path: "assets/worlds/patrol.toml".into(),
        dt,
        max_ticks: 30,
        deterministic: true,
        ..test_args()
    };
    let mut app = build_headless_app(&args).expect("app should build");
    // Boot far enough that the game is InProgress and the ship is backfilled.
    run(&mut app, args.max_ticks);

    // Re-pace the harness to a QUARTER tick per frame, off the app's own
    // timestep so the ratio is exact.
    let period = app.world().resource::<Time<Fixed>>().timestep();
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(period / 4));

    let probe_admitted = |app: &mut App| -> bool {
        let mut q = app
            .world_mut()
            .query_filtered::<&AdmittedCommands, With<LocalShip>>();
        q.single(app.world())
            .map(|a| {
                a.0.iter()
                    .any(|c| c.response_token.as_deref() == Some(PROBE_TOKEN))
            })
            .unwrap_or(false)
    };

    app.world_mut()
        .resource_mut::<bevy::ecs::message::Messages<InboundMessage>>()
        .write(InboundMessage {
            token: PROBE_TOKEN.into(),
            msg: ClientMessage::ControlSystem {
                target: project_phoenix::ship::system_registry::helm_thrust_system_id(),
                payload: SystemControlPayload::SetThrust { value: 0.5 },
            },
        });

    let mut admitted_on_a_tick = false;
    let mut cleared_on_a_later_tick = false;
    for _ in 0..16 {
        let tick_before = app.world().resource::<SimTick>().0;
        let was_admitted = probe_admitted(&mut app);
        app.update();
        let tick_after = app.world().resource::<SimTick>().0;
        let is_admitted = probe_admitted(&mut app);

        if tick_after == tick_before {
            // No fixed step ran: admission must not have touched the set —
            // neither wiping a command a consumer has not seen, nor admitting
            // one mid-tick.
            assert_eq!(
                is_admitted, was_admitted,
                "a frame with no fixed step changed AdmittedCommands — \
                 admission is running per frame again"
            );
        } else if is_admitted {
            admitted_on_a_tick = true;
        } else if admitted_on_a_tick && !is_admitted {
            // The next tick's admission pass cleared the consumed command.
            cleared_on_a_later_tick = true;
        }
    }
    assert!(
        admitted_on_a_tick,
        "the probe command was never admitted — with 16 quarter-period frames \
         at least three ticks ran, and the message must survive the frames \
         between them"
    );
    assert!(
        cleared_on_a_later_tick,
        "the admitted probe command was never cleared by a later tick's \
         admission pass — AdmittedCommands is per-tick state"
    );
}

/// A wide, cheap fingerprint of a whole run's end state, for the frame-rate
/// invariance assertion below.
///
/// Extracted to `project_phoenix::headless::fingerprint` (issue #899) so
/// `tests/registration_order_determinism.rs` can reuse the exact same struct
/// and extraction logic instead of carrying a second copy — see that module
/// for the field-by-field rationale.
use project_phoenix::headless::fingerprint::{fingerprint, RunFingerprint};

/// Issue #895's headline acceptance: the SAME end state at two very different
/// frame rates, given the same seed and commands — verified, not assumed.
///
/// Both runs cover exactly 240 logical ticks of the same seeded world; one is
/// driven a frame per tick (a 60 Hz display), the other a frame per FOUR
/// ticks (a heavily loaded host at 15 fps). The frame periods are derived
/// from the app's own timestep by integer `Duration` arithmetic, so both
/// accumulate the identical number of steps with zero rounding drift, and the
/// end states are asserted BIT-equal: with the whole simulation on the fixed
/// tick there is nothing frame-coupled left to diverge.
///
/// # The two worlds, and why it took two issues to get here
/// `patrol.toml` flies a deterministic non-contact course: no collision ever
/// fires, so rapier feeds nothing back into ship state and what is under test
/// is exactly the schedule. That was all #895 could assert, because rapier was
/// still driven once per rendered FRAME — anything downstream of the physics
/// pipeline diverged between these two drives for a reason #895 could not fix,
/// and this test's own docs said so and named the follow-up.
///
/// #896 moved rapier onto the logical tick, so the second world is now here:
/// `rng_coverage.toml` puts the player inside an asteroid belt and makes it fly,
/// which means real contacts, real collision damage, and real draws on
/// `SimStream::CollisionDamage`. That is the case that fails without #896 and
/// passes with it, and [`RunFingerprint::collisions`] plus the precondition
/// below are what stop it quietly degrading into a second copy of the
/// no-contact run.
#[test]
fn the_simulation_reaches_the_same_state_at_wildly_different_frame_rates() {
    let per_tick = frame_pacing_end_state("assets/worlds/patrol.toml", 240, 1);
    let per_four = frame_pacing_end_state("assets/worlds/patrol.toml", 60, 4);
    assert_eq!(
        per_tick.tick, 240,
        "precondition: one frame per tick for 240 frames is 240 ticks"
    );
    assert_eq!(
        per_four.tick, 240,
        "precondition: one frame per FOUR ticks for 60 frames is 240 ticks"
    );
    assert!(
        !per_tick.ships.is_empty(),
        "precondition: the fingerprint must cover at least one ship — an \
         empty slice would make the comparison below vacuous"
    );
    assert_eq!(
        per_tick, per_four,
        "the same 240 logical ticks of the same seeded world must land in the \
         BIT-identical state whatever the frame rate — a difference means \
         something in the sim still advances per frame"
    );
}

/// Frames the collision-bearing pacing runs cover, at one logical tick each.
///
/// Long enough that the backfilled player is up to speed and well into the belt
/// `rng_coverage.toml` wraps around the spawn point — the precondition below
/// fails loudly if it ever stops being long enough, rather than passing on an
/// empty collision list.
const COLLIDING_INVARIANCE_TICKS: u64 = 900;

/// Issue #896's headline acceptance, and the case #895 had to leave open: the
/// same claim as the test above, in a world where **ships actually collide**.
///
/// With rapier stepping in `PostUpdate` off the frame clock, these two drives
/// were not running the same physics at all — the 4-ticks-per-frame run stepped
/// the solver a quarter as often, over four times the distance each step, so it
/// hit different rocks at different speeds and this test failed on the collision
/// list, the hull totals and the `CollisionDamage` stream position together.
/// With physics on the logical tick and its results consumed in world-id order,
/// the two runs are the same simulation and agree bit for bit.
#[test]
fn a_colliding_world_reaches_the_same_state_at_wildly_different_frame_rates() {
    let world = "assets/worlds/rng_coverage.toml";
    let per_tick = frame_pacing_end_state(world, COLLIDING_INVARIANCE_TICKS, 1);
    let per_four = frame_pacing_end_state(world, COLLIDING_INVARIANCE_TICKS / 4, 4);

    assert_eq!(
        (per_tick.tick, per_four.tick),
        (COLLIDING_INVARIANCE_TICKS, COLLIDING_INVARIANCE_TICKS),
        "precondition: both drives must cover the same number of logical ticks"
    );
    assert!(
        !per_tick.collisions.is_empty(),
        "precondition: no collision was applied in {COLLIDING_INVARIANCE_TICKS} \
         ticks of {world}, so this is a second no-contact run and proves \
         nothing about physics. Ships: {:?}",
        per_tick.ships
    );
    assert_eq!(
        per_tick, per_four,
        "a colliding world must reach the BIT-identical state whatever the \
         frame rate — a difference means physics is still following the frame \
         clock, or its contacts are still being consumed in an order the \
         simulation does not choose"
    );
}

/// Drive `frames` frames of `ticks_per_frame` logical ticks each through
/// `world`, and fingerprint the world it leaves behind.
fn frame_pacing_end_state(world: &str, frames: u64, ticks_per_frame: u32) -> RunFingerprint {
    frame_pacing_end_state_with(world, frames, ticks_per_frame, false)
}

/// [`frame_pacing_end_state`], with `physics_last` — the registration-order
/// knob issue #896's digest AC turns on. See `SimPluginOptions::physics_last`.
fn frame_pacing_end_state_with(
    world: &str,
    frames: u64,
    ticks_per_frame: u32,
    physics_last: bool,
) -> RunFingerprint {
    let dt = 1.0 / 60.0;
    let args = HeadlessArgs {
        world_path: world.into(),
        dt,
        max_ticks: 0, // driven by hand below
        seed: Some(42),
        deterministic: true,
        physics_last,
        ..test_args()
    };
    let mut app = build_headless_app(&args).expect("app should build");
    app.finish();
    app.cleanup();
    // Establish the time baseline (zero delta, no steps)…
    app.update();
    // …then re-pace off the app's own timestep, exactly.
    let period = app.world().resource::<Time<Fixed>>().timestep();
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
        period * ticks_per_frame,
    ));
    for _ in 0..frames {
        app.update();
    }

    fingerprint(&mut app)
}

/// Issue #896, AC-4: the same colliding run, with the physics plugin registered
/// **after** every simulation system instead of before them, reaches the same
/// state.
///
/// Registration order is the thing a schedule falls back on when nothing else
/// decides: two sets that merely coexist in `FixedUpdate` can be interleaved
/// either way round, and which way you get is a function of the order the
/// `add_plugins` calls happened to appear in `add_simulation_plugins_with`.
/// That is not a property anyone would notice changing, and it would silently
/// change what a collision resolves against — physics stepping *before* this
/// tick's `sync_ship_position` reads last tick's positions.
///
/// `register_physics` therefore declares both edges it needs
/// (`SyncBackend.after(SimSet::Physics)`, `Writeback.before(SimSet::Damage)`),
/// and this is the assertion that they, and not the call order, are what holds
/// the tick together. It runs on the collision-bearing world for the same
/// reason as the test above: in a world with no contacts, physics could be
/// scheduled anywhere at all and nothing downstream would notice.
#[test]
fn a_colliding_run_is_the_same_with_physics_registered_last() {
    let world = "assets/worlds/rng_coverage.toml";
    let physics_first = frame_pacing_end_state_with(world, COLLIDING_INVARIANCE_TICKS, 1, false);
    let physics_last = frame_pacing_end_state_with(world, COLLIDING_INVARIANCE_TICKS, 1, true);

    assert!(
        !physics_first.collisions.is_empty(),
        "precondition: the run applied no collisions, so registration order \
         could not have mattered either way"
    );
    assert_eq!(
        physics_first, physics_last,
        "the same run diverged when the physics plugin was registered last — \
         the tick's physics ordering is coming from the order the plugins were \
         added, not from the explicit set edges in `register_physics`"
    );
}

/// Issue #896, AC-1: rapier steps once per LOGICAL tick, whatever the frame
/// pacing — the property the whole slice rests on.
///
/// Measured rather than inferred, and measured out of rapier itself. A
/// kinematic probe body is dropped into the world with a known velocity and no
/// collider, so the only thing that moves it is the solver integrating it: the
/// distance it has travelled after the run divided by its speed IS the time
/// rapier simulated. At the 60 Hz tick, 240 ticks must integrate exactly four
/// seconds of it, from a host running one tick per frame and from a host
/// running four.
///
/// With physics in `PostUpdate` on `TimestepMode::Fixed { dt }`, that number
/// was the FRAME count times `dt` — four seconds against one for the two drives
/// below. That is the gap the colliding invariance test then sees the
/// consequences of: the same 240 logical ticks, with physics having advanced
/// the world by four times as much in one of them.
#[test]
fn rapier_steps_once_per_logical_tick_at_any_frame_rate() {
    use bevy_rapier3d::prelude::{RigidBody, Velocity};
    use project_phoenix::sim_tick::SimTick;

    /// The probe's speed along +X, in units per simulated second. Chosen for
    /// exactness in binary rather than realism — nothing else touches it.
    const PROBE_SPEED: f32 = 8.0;

    #[derive(Component)]
    struct StepProbe;

    /// `(logical ticks, seconds of physics rapier integrated)` after `frames`
    /// frames of `ticks_per_frame` logical ticks each.
    fn drive(frames: u64, ticks_per_frame: u32) -> (u64, f32) {
        let args = HeadlessArgs {
            world_path: "assets/worlds/patrol.toml".into(),
            dt: 1.0 / 60.0,
            max_ticks: 0,
            seed: Some(42),
            deterministic: true,
            ..test_args()
        };
        let mut app = build_headless_app(&args).expect("app should build");
        app.finish();
        app.cleanup();
        app.update();

        // Well clear of the scenario, with no collider, so nothing but the
        // integrator can affect where it ends up.
        app.world_mut().spawn((
            StepProbe,
            Transform::from_xyz(0.0, 5_000.0, 0.0),
            RigidBody::KinematicVelocityBased,
            Velocity::linear(Vec3::X * PROBE_SPEED),
        ));

        let period = app.world().resource::<Time<Fixed>>().timestep();
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            period * ticks_per_frame,
        ));
        for _ in 0..frames {
            app.update();
        }

        let travelled = app
            .world_mut()
            .query_filtered::<&Transform, With<StepProbe>>()
            .single(app.world())
            .expect("the probe body should still be there")
            .translation
            .x;
        (app.world().resource::<SimTick>().0, travelled / PROBE_SPEED)
    }

    let (ticks_a, seconds_a) = drive(240, 1);
    let (ticks_b, seconds_b) = drive(60, 4);

    assert_eq!(
        (ticks_a, ticks_b),
        (240, 240),
        "precondition: both drives must cover 240 logical ticks"
    );
    assert!(
        (seconds_a - 4.0).abs() < 1e-3,
        "240 logical ticks at 60 Hz is four seconds of simulation, but rapier \
         integrated {seconds_a}s of it"
    );
    assert_eq!(
        seconds_a, seconds_b,
        "the same 240 logical ticks must step physics by the same amount \
         whatever the frame pacing — a difference means rapier is back on the \
         frame clock"
    );
}

/// Issue #896, AC-2: the build that claims determinism runs rapier's broadphase
/// serially, on every target.
///
/// A parallel broadphase does not hand contacts to the narrow phase in the same
/// order a serial one does. The wasm build cannot have one (the browser runtime
/// is single-threaded), so a native build with `features = ["parallel"]` and a
/// wasm build without it are running measurably different physics — and the
/// difference is invisible to native-only testing, because any two native
/// instances agree with each other perfectly. It would first surface in real
/// P2P, between a browser and anything else.
///
/// This reads the manifest rather than the running solver because that is where
/// the decision lives and where it would be undone: `parallel` is a cargo
/// feature, so no assertion inside a native test process can observe its
/// absence. The AC allows the feature back for a build that does *not* claim
/// determinism; what it does not allow is the two targets drifting apart
/// without anyone recording the choice, and re-adding the feature has to walk
/// past this test to do it.
///
/// The manifest is parsed as TOML (the `toml` crate is already a regular
/// dependency, so it is on the classpath for tests too) rather than scanned
/// line by line, because a per-line `starts_with("bevy_rapier3d")` only
/// catches the inline-table form (`bevy_rapier3d = { ... }`). The full
/// `[dependencies.bevy_rapier3d]` table form, and a `features = [...]` array
/// spread across several lines, both start their `bevy_rapier3d` line with
/// whitespace or a bracket instead, and a plain substring scan would walk
/// straight past them. Parsing means every one of those shapes lands in the
/// same `toml::Value::Table`, and the check below finds `bevy_rapier3d`'s
/// `features` list under any of them.
#[test]
fn the_deterministic_build_runs_rapier_serially() {
    let manifest = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"))
        .expect("the crate manifest should be readable");
    let doc: toml::Value =
        toml::from_str(&manifest).expect("the crate manifest should be valid TOML");

    /// Recursively walk every table in the document looking for a
    /// `bevy_rapier3d` dependency entry (inline table or full
    /// `[dependencies.bevy_rapier3d]` table — both parse to the same
    /// `toml::Value::Table` shape) and assert its `features` list, if any,
    /// does not contain `"parallel"`.
    fn check(value: &toml::Value, path: &str) {
        let Some(table) = value.as_table() else {
            return;
        };
        for (key, entry) in table {
            let sub_path = format!("{path}.{key}");
            if key == "bevy_rapier3d" {
                let enables_parallel = entry
                    .get("features")
                    .and_then(|f| f.as_array())
                    .is_some_and(|features| {
                        features.iter().any(|f| f.as_str() == Some("parallel"))
                    });
                assert!(
                    !enables_parallel,
                    "a bevy_rapier3d dependency at `{sub_path}` enables the \
                     `parallel` feature:\n  {entry}\nA parallel broadphase \
                     orders contacts differently from the serial one wasm is \
                     stuck with, so the two targets would no longer be \
                     running the same simulation. If this is deliberate, it \
                     belongs in a build that does not claim determinism."
                );
            } else {
                check(entry, &sub_path);
            }
        }
    }
    check(&doc, "");
}

// ── Command log (issue #898) ──────────────────────────────────────────────────

/// Option A's load-bearing claim, at run scale: a headless run is crewed
/// entirely by AI, and it records **nothing**.
///
/// The two assertions are a pair. The AI is demonstrably issuing orders — every
/// tick's `AdmittedCommands`, summed over the run — and none of them reached
/// the log, because none of them crossed a network boundary. What makes the
/// omission safe is the other half of the contract, proved in
/// `tests/rng_determinism.rs`: two runs of this same class of scenario on one
/// seed produce byte-identical reports, so a replay re-derives every one of
/// those decisions rather than needing them written down. Logging them as well
/// would apply each order twice.
///
/// This is also the standing guard against the mistake that would break the
/// whole design: if some future AI decider ever routed its orders through
/// `InboundMessage` instead of `emit_ai_command`, they would start being
/// recorded, the replay would double-count them, and this test would go red
/// first.
///
/// # Why the emissions are counted, not sampled
///
/// The "the AI is demonstrably issuing orders" half used to read
/// `AdmittedCommands` once, after the run, and require it non-empty. That
/// asserts on ONE tick's buffer — the last one — and whether that tick held
/// anything is a parity question: the buffer is cleared and refilled every
/// tick, and the deciders run on the AI cadence (every
/// `sim_tick_hz / ai_tick_hz`-th tick, so every second tick at the shipped
/// rates). Land the run's final tick between decisions and the buffer is
/// legitimately empty and the test fails with nothing wrong; equally, it would
/// keep passing if the AI had fallen silent for every tick but the last.
///
/// A `FixedLast` probe accumulating each tick's buffer answers the question
/// actually being asked — *did this run's AI issue orders at all?* — and is
/// indifferent to where the run happens to stop. `FixedLast` because that is
/// after the whole `SimSet` chain has run for the tick and before the next
/// tick's admission clears the buffer.
#[test]
fn an_ai_crewed_run_records_no_commands() {
    use project_phoenix::command_admission::CommandLog;
    use project_phoenix::messages::AdmittedCommands;

    /// Every command that sat in any ship's `AdmittedCommands`, summed over
    /// every tick of the run.
    #[derive(Resource, Default)]
    struct EmissionTally(usize);

    fn tally_emissions(mut tally: ResMut<EmissionTally>, ships: Query<&AdmittedCommands>) {
        tally.0 += ships.iter().map(|a| a.0.len()).sum::<usize>();
    }

    let args = HeadlessArgs {
        max_ticks: 200,
        ..test_args()
    };
    let mut app = build_headless_app(&args).expect("app should build");
    app.init_resource::<EmissionTally>()
        .add_systems(FixedLast, tally_emissions);
    run(&mut app, args.max_ticks);

    let emitted = app.world().resource::<EmissionTally>().0;
    assert!(
        emitted > 0,
        "no ship held an admitted command on ANY tick of the run, so the AI \
         emitted nothing at all and an empty log would prove nothing"
    );

    let log = app.world().resource::<CommandLog>();
    assert!(
        log.is_empty(),
        "an AI-crewed run must record no commands — the log carries the \
         network boundary, and the simulation re-derives everything else. \
         The AI issued {emitted} order(s) across the run, none of which \
         belonged in the log. Recorded: {:?}",
        log.entries()
    );
}

/// A run's log is replayable in principle: every command that crossed the
/// boundary is in it, in order, stamped with the tick it applied on, and those
/// ticks never go backwards.
///
/// Driving a whole replay from this is #901's job. What is checked here is the
/// property such a driver depends on and cannot repair — that the record is
/// complete and monotonic — against the real production wiring rather than a
/// bare-`App` fixture. `red-alert` is the target because every shipped hull
/// declares it `automated`, so it answers to an `ai:` token in a run with
/// nobody connected; the point being made is about the *boundary*, not about
/// who was on the far side of it.
#[test]
fn a_runs_log_records_every_boundary_command_in_tick_order() {
    use project_phoenix::command_admission::ai_emit::AI_BACKFILL_TOKEN;
    use project_phoenix::command_admission::CommandLog;
    use project_phoenix::lobby::InboundMessage;
    use project_phoenix::messages::{ClientMessage, SystemControlPayload, SystemId};
    use project_phoenix::sim_tick::SimTick;

    fn send(app: &mut App, active: bool) {
        app.world_mut()
            .resource_mut::<Messages<InboundMessage>>()
            .write(InboundMessage {
                token: AI_BACKFILL_TOKEN.into(),
                msg: ClientMessage::ControlSystem {
                    target: SystemId("red-alert".into()),
                    payload: SystemControlPayload::SetRedAlert { active },
                },
            });
    }

    let args = HeadlessArgs {
        max_ticks: 200,
        ..test_args()
    };
    let mut app = build_headless_app(&args).expect("app should build");
    // Far enough in that the auto-start countdown has put the run InProgress,
    // which is what admission is gated on.
    run(&mut app, 120);
    assert_eq!(
        app.world().resource::<State<GamePhase>>().get(),
        &GamePhase::InProgress,
        "precondition: admission only runs InProgress"
    );

    // Three commands across three separate ticks, plus two in one tick, so the
    // record has to get both the across-tick and the within-tick order right.
    let mut expected: Vec<u64> = Vec::new();
    for active in [true, false, true] {
        expected.push(app.world().resource::<SimTick>().0);
        send(&mut app, active);
        run(&mut app, 1);
    }
    let paired_tick = app.world().resource::<SimTick>().0;
    send(&mut app, false);
    send(&mut app, true);
    expected.push(paired_tick);
    expected.push(paired_tick);
    run(&mut app, 1);

    let log = app.world().resource::<CommandLog>();
    let ticks: Vec<u64> = log.entries().iter().map(|e| e.tick).collect();
    assert_eq!(
        ticks, expected,
        "every boundary command must be recorded, stamped with the tick it \
         applied on — one entry per command, in arrival order"
    );
    assert!(
        log.ticks_are_monotonic(),
        "recorded ticks must never go backwards, or a replay could not apply \
         them against a clock that only advances"
    );
    assert_eq!(
        log.for_tick(paired_tick).count(),
        2,
        "two commands admitted in one tick both belong to that tick"
    );
    let alternating: Vec<bool> = log
        .entries()
        .iter()
        .map(|e| match &e.payload {
            SystemControlPayload::SetRedAlert { active } => *active,
            other => panic!("unexpected payload in the log: {other:?}"),
        })
        .collect();
    assert_eq!(
        alternating,
        vec![true, false, true, false, true],
        "the log preserves the order the commands arrived in, not just their \
         count"
    );
}

/// A second round starts a fresh log (issue #898 review).
///
/// `ReturnToLobby` from `GameOver` puts everyone back in the lobby for another
/// round (`lobby::handler::handle_return_to_lobby`), and `OnEnter(InProgress)`
/// runs again. Without the reset hung on that chain, round two appends to round
/// one's log and the pair "master seed + command log" stops describing a single
/// run — silently, because `SimTick` keeps counting and the merged log stays
/// perfectly monotonic. Nothing downstream can detect it afterwards, so the
/// guard belongs at the boundary.
///
/// Drives the real `add_simulation_plugins_with` wiring rather than a fixture:
/// the reset is registered in `server_app`'s `OnEnter` chain, and a fixture that
/// registered it by hand would prove only that the system works, not that
/// production calls it.
#[test]
fn a_second_round_starts_a_fresh_command_log() {
    use project_phoenix::command_admission::ai_emit::AI_BACKFILL_TOKEN;
    use project_phoenix::command_admission::{CommandLog, PendingCommands};
    use project_phoenix::lobby::InboundMessage;
    use project_phoenix::messages::{ClientMessage, SystemControlPayload, SystemId};

    let args = HeadlessArgs {
        max_ticks: 200,
        ..test_args()
    };
    let mut app = build_headless_app(&args).expect("app should build");
    run(&mut app, 150);
    assert_eq!(
        app.world().resource::<State<GamePhase>>().get(),
        &GamePhase::InProgress,
        "precondition: the first round is under way"
    );

    // Round one takes a command across the boundary.
    app.world_mut()
        .resource_mut::<Messages<InboundMessage>>()
        .write(InboundMessage {
            token: AI_BACKFILL_TOKEN.into(),
            msg: ClientMessage::ControlSystem {
                target: SystemId("red-alert".into()),
                payload: SystemControlPayload::SetRedAlert { active: true },
            },
        });
    run(&mut app, 1);
    assert_eq!(
        app.world().resource::<CommandLog>().len(),
        1,
        "precondition: round one recorded the command, so there is something \
         for round two to inherit"
    );

    // GameOver → Lobby → InProgress: the shape `ReturnToLobby` produces.
    // Driven through `NextState` because the phases are what the reset hangs
    // on; the lobby handler's own path to them is its test's business.
    for phase in [GamePhase::GameOver, GamePhase::Lobby, GamePhase::InProgress] {
        app.world_mut()
            .resource_mut::<NextState<GamePhase>>()
            .set(phase.clone());
        app.update();
        assert_eq!(
            app.world().resource::<State<GamePhase>>().get(),
            &phase,
            "the run must actually reach {phase:?}"
        );
    }

    assert!(
        app.world().resource::<CommandLog>().is_empty(),
        "a second round must start from an empty log — otherwise seed + log \
         describes two runs at once, and a replay would apply round one's \
         commands to round two's world. Inherited: {:?}",
        app.world().resource::<CommandLog>().entries()
    );
    assert!(
        app.world().resource::<PendingCommands>().is_empty(),
        "and no command from round one may still be queued for a future tick"
    );
}

// ── The native template preload walks subdirectories (issue #954) ────────────

/// The headless preload must find a template that lives in a SUBDIRECTORY of
/// `assets/entities/`, and must still refuse to cache the fragment tree.
///
/// Both halves are load-bearing and neither is obvious from the code:
///
/// * **Subdirectories are in.** A world naming a template the preload skipped
///   used to spawn nothing and not fail the build: it logged `entity template
///   not found in cache` and the run continued with a quietly emptier world.
///   That is exactly what happened when #954 first moved the three-weapon
///   escort to `assets/entities/test/rng_coverage_lancer.toml`:
///   `rng_coverage.toml` lost both hostiles, and the only symptom was three of
///   the five RNG chokepoints silently going quiet.
///
///   Issue #973 closed both halves of that — the build now fails validation on
///   an unresolvable `template_path`, and the spawn path resolves through
///   `resolve_entity_via` (cache, then the host loader) so it can no longer see
///   less than the validator did. This walk still decides what the *cache*
///   holds, which is what the AI-declaration manifest, marker validation and
///   the strict-parse gate all read, and what a browser preload has to mirror.
/// * **`fragments/` is out.** Nothing under it is spawnable — they are the
///   partial documents hulls compose FROM. Caching them would offer the world
///   loader entities that are not entities.
///
/// Asserted against the process-global native config cache, which any
/// `build_headless_app` in this binary populates from `assets/entities`.
#[test]
fn the_preload_caches_subdirectory_templates_but_never_the_fragment_tree() {
    let args = HeadlessArgs {
        world_path: "assets/worlds/rng_coverage.toml".into(),
        max_ticks: 0,
        ..test_args()
    };
    // The app is not what this asserts on — BUILDING it is what runs the
    // preload, and the preload's product is the process-global cache below.
    let _app = build_headless_app(&args).expect("app should build");

    let cache = project_phoenix::config_cache::get_config_cache();

    assert!(
        cache.contains_key("assets/entities/test/rng_coverage_lancer.toml"),
        "the RNG-coverage escort lives one directory down and `rng_coverage.toml` \
         names it by that path; a preload that only reads the top level leaves it \
         unspawnable with no build error. Cached keys under assets/entities/test/: \
         {:?}",
        cache
            .keys()
            .filter(|k| k.starts_with("assets/entities/test/"))
            .collect::<Vec<_>>()
    );
    assert!(
        cache.contains_key("assets/entities/alliance_destroyer.toml"),
        "precondition: the top-level walk still works, so the assertion above is \
         about recursion and not about the cache being populated at all"
    );

    let fragments: Vec<&String> = cache
        .keys()
        .filter(|k| k.starts_with("assets/entities/fragments/"))
        .collect();
    assert!(
        fragments.is_empty(),
        "no fragment may be cached as a spawnable template: {fragments:?}"
    );
}

/// Issue #968: a hull must not be able to drive INSIDE the `huge` asteroid class
/// (collider radius 12, added in #947), and must keep flying its authored route
/// afterwards instead of leaving the map.
///
/// `probe_huge_rock.toml` holds the reported geometry still, twice over: a
/// player destroyer looping a two-anchor patrol whose straight line runs through
/// a belt with a `huge` rock exactly on it, and — 2 km east, far enough to stay
/// LOD-demoted for the whole run — a Harrow picket doing the same on the
/// dead-reckoned path. The world carries no hostiles at all, so nothing here is
/// seed-sensitive combat: either the ships go round their rocks and keep
/// shuttling, or they do not.
///
/// # What the numbers mean
///
/// The instrumented `combat_test` run this issue was diagnosed from measured
/// **6.5 units** of penetration into a radius-12 rock by the high-fidelity
/// player hull, **3.7** by the dead-reckoned pickets, and a wreck strafing out
/// of the scenario at a fixed 8.77 u/s afterwards, never to return. Three causes,
/// all fixed: hazard severity was normalised by CENTRE distance, so the biggest
/// obstacles pushed the least (`ai::core::hazard_threat_fraction`); the collision
/// response only de-overlapped on the tick its 1 Hz damage cooldown let a hit
/// through (`server_app::handle_collisions`); and the demoted path assessed
/// hazards as a dimensionless point and answered them by facing radially away
/// rather than steering around (`ai::server::low_lod_avoid_yaw`).
///
/// Against the code as it stands the picket clears every rock by 3.7 units and
/// the player by 3.2, so both bounds below are met with room; against the code
/// before the fix the picket ends ticks 3.7 units INSIDE a collider. The
/// player's own leg is the weaker half of the guard — a clean head-on patrol
/// approach was survivable even before — which is why the picket is here.
///
/// # What the bounds are, and why they are two different numbers
///
/// The residual penetration tolerance is two simulation steps of travel, not
/// zero: rapier publishes a contact the tick AFTER the transforms that produced
/// it, so the first frame of a touch is always visible. Each ship's step is
/// derived from ITS OWN authored `max_speed` rather than from a shared
/// constant — the destroyer tops out at 15 u/s (0.25 per 60 Hz step) and the
/// Harrow picket at 12.5 (0.208), and quietly measuring the picket against the
/// destroyer's figure would have given it 20% more slack than its own hull
/// earns.
///
/// The picket carries a SECOND, much tighter bound: a strictly positive
/// clearance. The low-LOD leg is the half of the run this probe mainly exists
/// for — the player's head-on approach was survivable even before the fix — and
/// it has 3.7 units of room, so a penetration tolerance of 0.42 is not guarding
/// it at all: reverting the severity ramp to linear would still pass. "Never
/// touches a rock" is the claim the low-LOD path can actually be held to, so
/// that is the claim asserted.
#[test]
fn a_hull_never_ends_a_tick_inside_a_huge_asteroid() {
    use bevy::prelude::Transform;
    use project_phoenix::entities::spawner::HelmConsoleSection;
    use project_phoenix::entity_spawner::ColliderSection;
    use project_phoenix::server_app::Ship;

    /// Two ticks of travel at a hull's OWN authored top speed — the window in
    /// which rapier has not yet published a contact the transforms already
    /// imply.
    fn tolerated_penetration(max_speed: f32, dt: f32) -> f32 {
        2.0 * max_speed * dt
    }

    let dt = 1.0 / 60.0;
    let args = HeadlessArgs {
        world_path: "assets/worlds/probe_huge_rock.toml".into(),
        ship_path: "assets/entities/alliance_destroyer.toml".into(),
        dt,
        max_ticks: ticks_for_sim_seconds(210.0, dt),
        deterministic: true,
        ..test_args()
    };
    let mut app = build_headless_app(&args).expect("app should build");

    // The belt, read once: these are `[[entity]]` placements, so they never move
    // and never stream in or out.
    let rocks: Vec<(f32, f32, f32)> = {
        app.update();
        let mut q = app
            .world_mut()
            .query_filtered::<(&Transform, &ColliderSection), Without<Ship>>();
        q.iter(app.world())
            .map(|(t, c)| (t.translation.x, t.translation.z, c.0.radius))
            .collect()
    };
    assert_eq!(
        rocks.len(),
        8,
        "probe precondition: the world places eight rocks across its two belts and \
         the test must be measuring against all of them, got {rocks:?}"
    );
    assert_eq!(
        rocks.iter().filter(|(_, _, r)| *r == 12.0).count(),
        2,
        "probe precondition: each belt must contain the `huge` class this issue is \
         about, got radii {:?}",
        rocks.iter().map(|(_, _, r)| *r).collect::<Vec<_>>()
    );

    let mut worst_gap = f32::MAX;
    let mut worst_picket_gap = f32::MAX;
    let mut picket_travelled = 0.0_f32;
    let mut picket_min_z = f32::MAX;
    let mut picket_last: Option<(f32, f32)> = None;
    let mut reached_far = 0_u32;
    let mut reached_near = 0_u32;
    let mut at_far = false;
    let mut at_near = false;
    let mut furthest_from_lane = 0.0_f32;

    for _ in 1..args.max_ticks {
        app.update();

        let mut q = app
            .world_mut()
            .query_filtered::<(&ShipPhysics, &ColliderSection), With<LocalShip>>();
        let Some((physics, collider)) = q.iter(app.world()).next() else {
            panic!("the player destroyer must survive a world with no hostiles in it");
        };
        let (x, z, hull_radius) = (physics.x, physics.z, collider.0.radius);

        for (rx, rz, rr) in &rocks {
            let gap = ((rx - x).powi(2) + (rz - z).powi(2)).sqrt() - rr - hull_radius;
            worst_gap = worst_gap.min(gap);
        }

        // The dead-reckoned picket, on its own lane 2 km east through its own
        // huge rock. It is the only other ship in the world, and it is never the
        // `LocalShip`.
        let mut picket_q = app
            .world_mut()
            .query_filtered::<(&ShipPhysics, &ColliderSection), (With<Ship>, Without<LocalShip>)>();
        if let Some((pp, pc)) = picket_q.iter(app.world()).next() {
            for (rx, rz, rr) in &rocks {
                let gap = ((rx - pp.x).powi(2) + (rz - pp.z).powi(2)).sqrt() - rr - pc.0.radius;
                worst_picket_gap = worst_picket_gap.min(gap);
            }
            if let Some((lx, lz)) = picket_last {
                picket_travelled += ((pp.x - lx).powi(2) + (pp.z - lz).powi(2)).sqrt();
            }
            picket_last = Some((pp.x, pp.z));
            picket_min_z = picket_min_z.min(pp.z);
        }

        // Anchor arrivals, edge-triggered, so a ship loitering at one end counts
        // once rather than every tick it sits there. `WAYPOINT_ARRIVAL_RADIUS` is
        // 20; 25 keeps the count about *getting there*, not about the exact tick.
        let near_far = ((x - 0.0).powi(2) + (z + 240.0).powi(2)).sqrt() < 25.0;
        let near_near = ((x - 0.0).powi(2) + (z - 40.0).powi(2)).sqrt() < 25.0;
        if near_far && !at_far {
            reached_far += 1;
        }
        if near_near && !at_near {
            reached_near += 1;
        }
        at_far = near_far;
        at_near = near_near;

        // How far the hull ever strayed from the anchor pair's own bounding box.
        let off_lane = x.abs().max((z - 40.0).max(-240.0 - z).max(0.0));
        furthest_from_lane = furthest_from_lane.max(off_lane);
    }

    // Each hull's own authored top speed, read off the entity rather than
    // restated here, so the tolerances below are that hull's and not a shared
    // guess (issue #968 review). Read after the run rather than before it: the
    // player ship is a `spawn_on = "game_start"` placement and does not exist
    // at world load.
    let player_top_speed = {
        let mut q = app
            .world_mut()
            .query_filtered::<&HelmConsoleSection, With<LocalShip>>();
        q.iter(app.world())
            .next()
            .expect("the player destroyer authors a helm console")
            .0
            .max_speed
    };
    let picket_top_speed = {
        let mut q = app
            .world_mut()
            .query_filtered::<&HelmConsoleSection, (With<Ship>, Without<LocalShip>)>();
        q.iter(app.world())
            .next()
            .expect("the picket authors a helm console")
            .0
            .max_speed
    };
    assert!(
        picket_top_speed < player_top_speed,
        "probe precondition: the two hulls must have DIFFERENT authored top \
         speeds, else deriving a per-hull tolerance proves nothing (player \
         {player_top_speed}, picket {picket_top_speed})"
    );
    let player_tolerance = tolerated_penetration(player_top_speed, dt as f32);
    let picket_tolerance = tolerated_penetration(picket_top_speed, dt as f32);

    assert!(
        worst_gap >= -player_tolerance,
        "the hull ended a tick {:.2} units inside a collider — more than the \
         {player_tolerance:.2} units two simulation steps at its own authored \
         {player_top_speed} u/s can carry it, so it was being driven through the \
         rock rather than caught on the way in (issue #968)",
        -worst_gap
    );

    // Crossing the belt once could be luck; doing it repeatedly, in both
    // directions, is the ship still flying its route. The far anchor is on the
    // other side of the rock, so every arrival there is a traverse.
    assert!(
        reached_far >= 2 && reached_near >= 2,
        "the ship must keep shuttling through the belt — reached the far anchor \
         {reached_far} time(s) and the near anchor {reached_near} time(s) in 210 s"
    );

    // The dead-reckoned half of the report. A demoted hull used to assess hazards
    // as a dimensionless point with the parse-default buffer, which is short of
    // what it actually needs by its own radius; `combat_test`'s two pickets drove
    // into their belt on every pass because of it.
    assert!(
        picket_travelled > 600.0,
        "probe precondition: the picket must actually be flying its route for its \
         clearance to say anything — it covered only {picket_travelled:.0} units"
    );
    assert!(
        worst_picket_gap >= -picket_tolerance,
        "the dead-reckoned picket ended a tick {:.2} units inside a collider, more \
         than the {picket_tolerance:.2} units two steps at its own authored \
         {picket_top_speed} u/s could carry it (issue #968)",
        -worst_picket_gap
    );
    // And the real bound on this leg. The picket clears every rock by 4.7 units,
    // so the penetration tolerance above is not guarding it — reverting the
    // severity ramp to linear would still satisfy it. A dead-reckoned hull that
    // steers AROUND obstacles never touches one at all, and that is the claim
    // worth pinning: it is the half of the run this probe mainly exists for.
    assert!(
        worst_picket_gap > 0.0,
        "the dead-reckoned picket touched a rock (closest approach {:.2} units \
         of clearance). It has 3.7 units of room when the avoidance is working, \
         so any contact at all means the low-LOD path stopped clearing its belt \
         (issue #968)",
        worst_picket_gap
    );
    // And it must go THROUGH the belt, not stop at it. Not clearing a rock and
    // being pinned against one are the same dead mission; an avoidance that only
    // pushes the hull back the way it came turns the first failure into the
    // second, which is exactly what a radial escape heading did here before
    // `low_lod_avoid_yaw` started steering AROUND obstacles. The belt sits at
    // z = 0 and the far anchor at z = -200.
    assert!(
        picket_min_z < -50.0,
        "the picket never got past its own belt — its southernmost point was \
         z = {picket_min_z:.0}, with the rocks at z = 0. A hull pinned against a \
         rock has lost its mission just as surely as one that flew through it \
         (issue #968)"
    );

    // The other half of the report: a ship that wanders off is a mission that
    // silently ends. The reported hull left its scenario by ~2,600 units.
    assert!(
        furthest_from_lane < 250.0,
        "the ship strayed {furthest_from_lane:.0} units outside its own patrol \
         box; the failure this probe guards against is a hull leaving the map and \
         never coming back"
    );
}

// ── Named mission deadlines (issue #1024, parent #851) ───────────────────────

/// `probe_deadlines.toml`, driven for twenty mission seconds: a deadline that is
/// **slipped** fires at its new tick and never at its old one, a deadline that is
/// **cancelled** never fires at all, and the control deadline fires untouched.
///
/// This is the whole slice on one run — `[[deadline]]` parse, `on_deadline`
/// pairing, arming onto the EXISTING `pending_callbacks` queue, `ctx.deadlines`
/// inspection, slip/cancel re-keying that queue, and firing on a `SimTick`. The
/// probe world's own header carries the authored timeline it is asserted
/// against.
///
/// Every assertion is on a tick, never on a wall-clock reading: `due_tick` is
/// the arm tick plus a whole number of sim ticks, so two peers running this
/// world at the same `sim_tick_hz` reach every one of these states on the same
/// tick.
#[test]
fn a_slipped_deadline_fires_at_its_new_tick_and_a_cancelled_one_never_fires() {
    use project_phoenix::sim_tick::SimTick;
    use project_phoenix::world::deadlines::DeadlineState;
    use project_phoenix::world::server::{WorldContentRuntime, WorldScriptRuntime};

    let dt = 1.0 / 60.0;
    let args = HeadlessArgs {
        world_path: "assets/worlds/probe_deadlines.toml".into(),
        ship_path: "assets/entities/alliance_destroyer.toml".into(),
        dt,
        max_ticks: ticks_for_sim_seconds(20.0, dt),
        seed: Some(1024),
        deterministic: true,
        ..test_args()
    };
    let mut app = build_headless_app(&args).expect("app should build");

    // `(tick, window state, window fire count)` for every step after arming, so
    // the span between the OLD due tick and the new one can be inspected whole
    // rather than sampled at one hopeful moment.
    let mut trace: Vec<(u64, DeadlineState, i64)> = Vec::new();
    let mut warning_fired_at: Option<u64> = None;
    for _ in 0..args.max_ticks {
        run(&mut app, 1);
        let tick = app.world().resource::<SimTick>().0;
        let runtime = app.world().resource::<WorldContentRuntime>();
        if !runtime.deadlines.armed {
            continue;
        }
        let window = runtime
            .deadlines
            .get("window_opens")
            .expect("the authored window is armed");
        trace.push((tick, window.state, runtime.flags.counter("window_fired")));
        if warning_fired_at.is_none() && runtime.flags.counter("warning_fired") > 0 {
            warning_fired_at = Some(tick);
        }
    }

    let runtime = app.world().resource::<WorldContentRuntime>();
    let flags = &runtime.flags;
    let deadlines = &runtime.deadlines;

    // AC1: every authored deadline is armed under its own id, carrying its
    // authored label and visibility flag.
    assert_eq!(deadlines.records.len(), 3, "three authored deadlines");
    let warning = deadlines
        .get("first_warning")
        .expect("the control deadline")
        .clone();
    assert!(
        !warning.visible,
        "first_warning is authored invisible and stays that way"
    );
    assert_eq!(deadlines.get("window_opens").map(|d| d.visible), Some(true));
    assert_eq!(
        deadlines.get("window_opens").map(|d| d.label.as_str()),
        Some("world.probe_deadlines.deadline.window_opens.label"),
        "the crew-facing label is a strings.csv id, never English"
    );

    // The arm tick, derived from the control deadline nothing touched: 4 s at
    // 60 Hz. Everything below is stated relative to it.
    let arm_tick = warning.due_tick - 240;

    // AC4: script read the deadline back mid-run — 8 s left two seconds into a
    // ten-second deadline, and 13 s immediately after a five-second slip.
    assert_eq!(
        flags.counter("window_remaining_at_adjust"),
        8,
        "ctx.deadlines.remaining reported the live countdown"
    );
    assert_eq!(
        flags.counter("window_remaining_after_slip"),
        13,
        "and the same call read back its own slip"
    );
    assert_eq!(
        flags.counter("stabiliser_reads_cancelled"),
        1,
        "a cancel is visible to the rest of the call that made it"
    );

    // AC3, the load-bearing half: the slipped deadline is due 15 s in, and it is
    // still PENDING and unfired across every tick of the span its ORIGINAL due
    // tick falls in.
    assert_eq!(
        deadlines.get("window_opens").map(|d| d.due_tick),
        Some(arm_tick + 900),
        "10 s authored + a 5 s slip = tick 900 at 60 Hz"
    );
    let old_due = arm_tick + 600;
    let new_due = arm_tick + 900;
    let straddle: Vec<_> = trace
        .iter()
        .filter(|(tick, ..)| *tick >= old_due && *tick < new_due)
        .collect();
    assert!(
        !straddle.is_empty(),
        "precondition: the run covered the span between the old and new due ticks"
    );
    for (tick, state, fires) in &straddle {
        assert_eq!(
            *state,
            DeadlineState::Pending,
            "tick {tick}: a slipped deadline must not fire at its old time"
        );
        assert_eq!(*fires, 0, "tick {tick}: and must not have run its handler");
    }

    // …and it DOES fire, exactly once, at the new tick.
    assert_eq!(
        flags.counter("window_fired"),
        1,
        "the slipped deadline fired exactly once"
    );
    assert_eq!(
        flags.counter("window_reads_fired"),
        1,
        "and its own handler read its state as fired while running"
    );
    assert_eq!(
        deadlines.get("window_opens").map(|d| d.state),
        Some(DeadlineState::Fired)
    );
    let first_fire = trace
        .iter()
        .find(|(_, state, _)| *state == DeadlineState::Fired)
        .map(|(tick, ..)| *tick)
        .expect("the window fired inside the run");
    assert!(
        first_fire >= new_due && first_fire <= new_due + 1,
        "it fired ON its new tick ({new_due}), not merely near it: {first_fire}"
    );

    // AC3, the other half: a cancelled deadline never fires, and its authored
    // due tick (12 s) passed inside this run.
    assert!(
        arm_tick + 720 < args.max_ticks,
        "precondition: the run outlasts the cancelled deadline's authored tick"
    );
    assert_eq!(
        deadlines.get("stabiliser_failure").map(|d| d.state),
        Some(DeadlineState::Cancelled)
    );
    assert_eq!(
        flags.counter("stabiliser_fired"),
        0,
        "a cancelled deadline never runs its handler"
    );

    // The control: untouched, fired once, on its authored tick.
    assert_eq!(flags.counter("warning_fired"), 1);
    assert_eq!(
        warning_fired_at,
        // `advance_sim_tick` runs in `FixedLast`, so a step that reads tick N
        // inside the fixed schedule leaves `SimTick` at N+1 for this test to
        // read after the frame. The fire happened ON `due_tick`; the observation
        // of it is one tick later, and saying so is more honest than widening
        // the assertion to a band.
        Some(warning.due_tick + 1),
        "the invisible control deadline fired on its authored tick"
    );
    assert_eq!(warning.state, DeadlineState::Fired);

    // AC2, made concrete: the deferred work IS the existing queue, and nothing
    // stale is left on it. A slip that failed to retract, or a cancel that only
    // marked the record, would leave a `ScheduledCall` sitting here.
    let queued = app
        .world()
        .resource::<WorldScriptRuntime>()
        .pending_callbacks
        .len();
    assert_eq!(
        queued, 0,
        "every armed call either fired or was retracted — no stale deferred work"
    );

    // AC5's server half: the visible deadlines — and only those — reach the
    // captain blackboard with a server-computed countdown. (The panel that
    // renders them is covered by tests/client/console-state.test.js.)
    let mut q = app
        .world_mut()
        .query::<&project_phoenix::server_app::ShipSystemBlackboards>();
    let published: Vec<_> = q
        .iter(app.world())
        .filter_map(|bbs| {
            bbs.0
                .values()
                .find_map(|bb| match bb {
                    project_phoenix::messages::SystemBlackboard::Captain(c) => Some(c),
                    _ => None,
                })
                .filter(|c| !c.deadlines.is_empty())
                .map(|c| c.deadlines.clone())
        })
        .collect();
    let published = published
        .first()
        .expect("the local ship publishes its visible deadlines");
    assert_eq!(
        published.iter().map(|d| d.id.as_str()).collect::<Vec<_>>(),
        vec!["window_opens", "stabiliser_failure"],
        "only the visible deadlines are published, in authored order"
    );
    assert_eq!(published[0].state, "fired");
    assert_eq!(published[1].state, "cancelled");
    assert_eq!(
        published[1].remaining_secs, -1,
        "a cancelled deadline reports no countdown rather than a stale one"
    );
}

// ── Issue #1026: the stabilise operation, end to end in a real run ──────────

/// Read the operating ship's operations record out of a live app.
///
/// Found by the component rather than by name: exactly one entity in
/// `probe_stabilise.toml` authors an `[operations]` table, and a lookup that
/// went through `EntityName` would be testing name resolution rather than the
/// operation.
fn live_operations(
    app: &mut bevy::prelude::App,
) -> Option<project_phoenix::operations::ShipOperations> {
    app.world_mut()
        .query::<&project_phoenix::operations::ShipOperations>()
        .iter(app.world())
        .next()
        .cloned()
}

/// Move the operating ship to `x` by writing its `ShipPhysics`, which is what
/// helm moves. Writing the `Transform` directly would be undone by
/// `sync_ship_position` on the next tick.
fn move_operator_to(app: &mut bevy::prelude::App, x: f32) {
    let entity = app
        .world_mut()
        .query_filtered::<bevy::prelude::Entity, bevy::prelude::With<project_phoenix::operations::ShipOperations>>()
        .iter(app.world())
        .next()
        .expect("the probe world spawns one operating ship");
    app.world_mut()
        .get_mut::<project_phoenix::ship::state::ShipPhysics>(entity)
        .expect("the operating ship is a ship")
        .x = x;
}

fn stabilise_args(dt: f64, seconds: f64) -> HeadlessArgs {
    HeadlessArgs {
        world_path: "assets/worlds/probe_stabilise.toml".into(),
        dt,
        max_ticks: ticks_for_sim_seconds(seconds, dt),
        deterministic: true,
        seed: Some(42),
        ..test_args()
    }
}

/// **Issue #1026.** A scripted stabilise operation opens, holds, completes, and
/// the completion lifts the depot back over its own operational threshold —
/// every link asserted separately, because "the depot ended up capable" would
/// pass with the operation never having run at all.
#[test]
fn a_scripted_stabilise_operation_runs_to_completion_and_restores_the_depot() {
    use project_phoenix::infrastructure::InfrastructureCondition;
    use project_phoenix::operations::HoldState;
    use project_phoenix::world::server::WorldContentRuntime;

    let dt = 1.0 / 60.0;
    let args = stabilise_args(dt, 6.0);
    let mut app = build_headless_app(&args).expect("app should build");

    let mut first_seen: std::collections::BTreeMap<&str, f64> = std::collections::BTreeMap::new();
    let mut max_progress: f32 = 0.0;
    for tick in 0..args.max_ticks {
        run(&mut app, 1);
        let sim_t = (tick + 1) as f64 * dt;
        if let Some(ops) = live_operations(&mut app) {
            if let Some(hold) = ops.active.as_ref() {
                max_progress = max_progress.max(hold.progress());
                match hold.state() {
                    HoldState::Holding => {
                        first_seen.entry("holding").or_insert(sim_t);
                    }
                    HoldState::Completed => {
                        first_seen.entry("completed").or_insert(sim_t);
                    }
                    other => panic!(
                        "the operation ended as {other:?} at {sim_t:.2} s — nothing in this \
                         world interrupts it"
                    ),
                }
            }
        }
        let flags = &app.world().resource::<WorldContentRuntime>().flags;
        if flags.flag("depot_transfer_capable") {
            first_seen.entry("capable").or_insert(sim_t);
        }
        if flags.flag("depot_restored") {
            first_seen.entry("restored_hook").or_insert(sim_t);
        }
    }

    // ── The script effect opened the hold ──
    let opened_at = *first_seen.get("holding").unwrap_or_else(|| {
        panic!("the scripted `stabilise` effect never opened a hold at all: {first_seen:?}")
    });
    assert!(
        (1.0..1.5).contains(&opened_at),
        "the hold must open just after the t=1 s `on_timer`, not before it and not much after — \
         opened at {opened_at:.2} s"
    );

    // ── It completed after its authored duration of ELIGIBLE ticks ──
    let completed_at = *first_seen.get("completed").unwrap_or_else(|| {
        panic!(
            "the hold never completed: three authored seconds at 60 Hz is 180 eligible ticks, \
             and nothing in this world interrupts it. Seen: {first_seen:?}"
        )
    });
    assert!(
        (completed_at - opened_at - 3.0).abs() < 0.2,
        "it must complete three seconds after it opened — the authored duration, counted in \
         eligible ticks ({opened_at:.2} s to {completed_at:.2} s)"
    );
    assert_eq!(
        max_progress, 1.0,
        "and the published progress reaches the top"
    );

    // ── The completion moved the target's condition, through #1025's queue ──
    let condition = app
        .world_mut()
        .query::<&InfrastructureCondition>()
        .iter(app.world())
        .next()
        .map(|c| c.0.condition())
        .expect("the depot carries its condition track");
    assert_eq!(
        condition, 55.0,
        "30 authored points plus the operation's authored 25 — paid ONCE, on completion, into \
         the queue tick_infrastructure_condition drains"
    );

    // ── …which crossed the depot's threshold and flipped its flag ──
    let capable_at = *first_seen.get("capable").unwrap_or_else(|| {
        panic!(
            "`depot_transfer_capable` never came back: 55/100 is above the depot's 45 % restore \
             point, and the operation is what took it there. Seen: {first_seen:?}"
        )
    });
    assert!(
        (capable_at - completed_at).abs() < 0.1,
        "the flag flips on the tick the operation completes, not a tick later: tick_operations \
         is ordered BEFORE tick_infrastructure_condition precisely so the payoff lands on the \
         tick it was earned ({completed_at:.2} s vs {capable_at:.2} s)"
    );
    assert!(
        capable_at > opened_at,
        "…and the depot spawned BELOW its threshold, so this is the operation's crossing rather \
         than a flag that was up all along"
    );

    // ── A scenario hook reacted to the crossing ──
    let hook_at = *first_seen.get("restored_hook").unwrap_or_else(|| {
        panic!(
            "the world's `on_flag_set` handler never ran — the crossing wrote the flag store but \
             never reached the trigger pipeline. Seen: {first_seen:?}"
        )
    });
    assert!(
        hook_at >= capable_at && hook_at - capable_at < 0.5,
        "the hook fires promptly after the crossing, on the same one-tick pending_world_events \
         bridge #1025's rides ({capable_at:.2} s vs {hook_at:.2} s)"
    );
}

/// **Issue #1026.** Flying the operating ship off station stalls the hold and
/// banks nothing; flying it back resumes it from where it stood, and it still
/// completes.
///
/// The interruption arrives through the ship's REAL position — the same input
/// helm moves — rather than through a flag something else set, which is the
/// whole claim the ECS adapter makes.
#[test]
fn flying_the_operator_off_station_stalls_the_hold_and_returning_resumes_it() {
    use project_phoenix::operations::{HoldState, Ineligibility};

    let dt = 1.0 / 60.0;
    let mut app = build_headless_app(&stabilise_args(dt, 12.0)).expect("app should build");

    // Past the t=1 s start, with eligible hold banked.
    run(&mut app, 130);
    let opened = live_operations(&mut app)
        .and_then(|o| o.active)
        .unwrap_or_else(|| panic!("precondition: the scripted effect opens the hold by 130 ticks"));
    assert_eq!(opened.state(), HoldState::Holding);
    let banked = opened.elapsed_ticks();
    assert!(banked > 0, "precondition: it has banked eligible ticks");

    // ── Off station ──
    move_operator_to(&mut app, 60_000.0);
    run(&mut app, 120);
    let stalled = live_operations(&mut app)
        .and_then(|o| o.active)
        .expect("the hold survives");
    assert_eq!(
        stalled.state(),
        HoldState::Stalled(Ineligibility::OutOfRange),
        "leaving the authored range stalls the operation rather than ending it — it is exactly \
         the thing helm is there to fix"
    );
    assert_eq!(
        stalled.elapsed_ticks(),
        banked,
        "…and the ticks already held are not lost. Progress that decayed would make a brief \
         drift as expensive as never having started."
    );
    assert!(
        stalled.stalled_ticks() > 0,
        "the stall is counted, so a later slice can budget it"
    );

    // ── Back on station ──
    move_operator_to(&mut app, 200.0);
    run(&mut app, 240);
    let resumed = live_operations(&mut app)
        .and_then(|o| o.active)
        .expect("the hold survives");
    assert_eq!(
        resumed.state(),
        HoldState::Completed,
        "flying back resumes the hold from where it stood: the operation needs its authored \
         seconds of ELIGIBLE time, not of wall clock"
    );
}

/// **Issue #1026.** The operating ship publishes its hold on the wire, under
/// the operations blackboard channel rather than onto an existing system's.
#[test]
fn the_operating_ship_publishes_its_hold_under_the_operations_blackboard_key() {
    use project_phoenix::messages::SystemBlackboard;

    let dt = 1.0 / 60.0;
    let mut app = build_headless_app(&stabilise_args(dt, 3.0)).expect("app should build");
    run(&mut app, 130);

    let key = project_phoenix::messages::SystemId(
        project_phoenix::operations::OPERATIONS_BLACKBOARD_KEY.to_string(),
    );
    let published: Vec<_> = app
        .world_mut()
        .query::<&project_phoenix::server_app::ShipSystemBlackboards>()
        .iter(app.world())
        .filter_map(|boards| match boards.0.get(&key) {
            Some(SystemBlackboard::Operations(bb)) => Some(bb.clone()),
            _ => None,
        })
        .collect();
    let bb = published
        .first()
        .expect("the operating ship publishes an operations blackboard");
    assert_eq!(
        bb.capabilities
            .iter()
            .map(|c| c.verb.as_str())
            .collect::<Vec<_>>(),
        vec!["stabilise"],
        "the hull's authored verbs reach the console"
    );
    let active = bb.active.as_ref().expect("the live hold is published");
    assert_eq!(active.state, "holding");
    assert!(
        active.progress > 0.0 && active.progress < 1.0,
        "progress is published mid-hold, not only at the ends: {}",
        active.progress
    );
    assert_eq!(
        active.verb_label, "operation.verb.stabilise",
        "no English crosses the wire — the console resolves this id"
    );
    assert_eq!(
        active.target_name.as_deref(),
        Some("world.probe_stabilise.entity.skyhook.name"),
        "and the target is named by its own string id"
    );
}
