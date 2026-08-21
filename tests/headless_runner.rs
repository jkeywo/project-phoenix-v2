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

    let cruiser: project_phoenix::entity_config::EntityConfig =
        project_phoenix::entity_includes::load_entity_config(
            "assets/entities/alliance_cruiser.toml",
        )
        .expect("the cruiser template must compose and parse");
    let expected_rating = cruiser.power_rating.map(|r| r as f32);

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

/// A world's `player-ship` overrides apply to the hull the LOBBY picked
/// (defect found in #1036).
///
/// `spawn_game_start_entities` replaces the world row's config with the
/// lobby-selected template, because the row's `template_path` is only a
/// placeholder for position and identity. It used to replace it WHOLESALE —
/// discarding the `[entity.overrides.*]` the same row authored, after the
/// composition validator had already merged and approved them. Every world's
/// player-ship tuning was therefore decorative.
///
/// `tests/fixtures/worlds/player_ship_override.toml` is built to make the two
/// authorities visibly disagree: its row names the **Destroyer** and tunes
/// `hold-station` to a priority neither hull authors, and this boots it with
/// `--ship` on the **Cruiser**. So one run answers both halves — the tuning
/// survived the swap, and it was merged onto the picked hull rather than onto
/// the placeholder.
///
/// The last two assertions double as the no-override control: every field the
/// override does not name comes back exactly as the Cruiser template authored
/// it, which is the property the shipped worlds' unmoved digests rest on.
#[test]
fn a_worlds_player_ship_overrides_apply_to_the_lobby_selected_hull() {
    use project_phoenix::entity_config::{DoctrineObjective, EntityConfig};
    use project_phoenix::entity_spawner::BehaviourSection;

    fn doctrine<'a>(pool: &'a [DoctrineObjective], id: &str) -> &'a DoctrineObjective {
        pool.iter()
            .find(|d| d.id == id)
            .unwrap_or_else(|| panic!("a `{id}` doctrine must be present"))
    }

    let args = HeadlessArgs {
        world_path: "tests/fixtures/worlds/player_ship_override.toml".into(),
        // The lobby's pick, and deliberately NOT the hull the world's row names.
        ship_path: "assets/entities/alliance_cruiser.toml".into(),
        ..test_args()
    };
    let mut app = build_headless_app(&args).expect("app should build");
    run(&mut app, args.max_ticks);

    let mut q = app
        .world_mut()
        .query_filtered::<&BehaviourSection, With<LocalShip>>();
    let flown = q
        .single(app.world())
        .expect("exactly one LocalShip should exist")
        .0
        .doctrine
        .clone();

    assert_eq!(
        doctrine(&flown, "hold-station").base_priority,
        77.0,
        "the world tuned its player ship and the tuning has to reach the hull \
         the crew actually fly — both templates author 20.0, so this number can \
         only have come from the world's `[entity.overrides.*]`"
    );

    let cruiser: EntityConfig = project_phoenix::entity_includes::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .expect("the cruiser template must compose and parse");
    let cruiser_doctrine = cruiser
        .behaviour
        .as_ref()
        .expect("the cruiser authors [behaviour]")
        .doctrine
        .clone();
    assert_eq!(
        doctrine(&flown, "destroy-hostiles"),
        doctrine(&cruiser_doctrine, "destroy-hostiles"),
        "…merged onto the LOBBY's hull: an untouched doctrine comes back exactly \
         as the Cruiser authored it, not as the placeholder Destroyer did (whose \
         entry differs in text, target_speed and maintain_range)"
    );
    assert_eq!(
        DoctrineObjective {
            base_priority: 77.0,
            ..doctrine(&cruiser_doctrine, "hold-station").clone()
        },
        *doctrine(&flown, "hold-station"),
        "…and the override edited the Cruiser's own entry IN PLACE: one field \
         changed, every other field of it untouched"
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
        // Re-blessed twice. First for issue #907's review (was seed 12,
        // swept 1..15 on a 60 s window): moving the game-start
        // `NextState<GamePhase>` writers into `FixedUpdate` shifted this
        // combat-chaotic duel's RNG draws by one tick and seed 12's fight
        // resolved into a full kill before the radar specifically died; seed
        // 1 replaced it. Then again when the power fixes (5672b09a seeding
        // the AI power decider's thrust fact from the real throttle, and
        // c8a13b9a's brownout-advisory timing) re-timed the duel once more:
        // on THAT timing seed 1's hostile dies whole at tick 961 — radar
        // never reaching Destroyed on its own — which is again the
        // "different bug" this test deliberately does not chase. Re-swept
        // over seeds 1..40 on the same 90 s window (recorded below); seed 2
        // destroys the hostile's tactical radar at tick 868 (~29 s) with
        // both ships alive, the same shape both previous picks gave.
        //
        // Sweep table (seed: outcome@tick; "hostile-gone" = the whole ship
        // died before its radar specifically reached Destroyed, which
        // disqualifies the seed):
        //   1:hostile-gone@961   2:destroyed@868   3:destroyed@868
        //   4:destroyed@868      5:hostile-gone@930 6:destroyed@868
        //   7:destroyed@868      8:destroyed@837   9:hostile-gone@930
        //   10:destroyed@868    11:destroyed@930  12:destroyed@868
        //   13:destroyed@868    14:destroyed@868  15:hostile-gone@930
        //   16:destroyed@868    17:hostile-gone@961 18:hostile-gone@930
        //   19..24:destroyed@868 25:hostile-gone@930 26:destroyed@868
        //   27..29:hostile-gone@930 30..32:destroyed@868 33/34:hostile-gone@930
        //   35:destroyed@930    36:destroyed@868  37:destroyed@930
        //   38:hostile-gone@930 39:destroyed@930  40:destroyed@868
        // Seed 2 chosen as the simplest surviving candidate, not because it
        // is otherwise special. The sweep harness is
        // `scratch_seed_sweep_probe_radar_kill` below.
        //
        // RE-BLESSED A THIRD TIME at issue #1053, seed 2 -> seed 5. The
        // over-cap bleed stopped a helm power shed deleting an over-cap
        // velocity in one tick, so hulls hold speed through a shed and are
        // harder to hit — this whole duel resolves LATER. Seed 2's radar no
        // longer reaches Destroyed at all inside the 90 s window (it did at
        // 868), which trips the same "never reached Destroyed" panic the two
        // earlier re-blesses were about. Nothing about the decision under test
        // moved; the fight it is observed in did.
        //
        // Re-swept 1..40 on the same 90 s window and the same battleship. The
        // whole distribution slid ~300 ticks later — the earliest destroy is
        // now 1174 against the old 837:
        //   1:1188      2:none      3:1188      4:none      5:1174
        //   6:1174      7:1174      8:1795      9:1188      10:1188
        //   11:1188     12:none     13:none     14:1174     15:1174
        //   16:1174     17:1807     18:1188     19:1315     20:1188
        //   21:1188     22:none     23:none     24:none     25:1174
        //   26:none     27:none     28:none     29:1174     30:1174
        //   31:1295     32:1174     33:none     34:1315     35:1188
        //   36:1315     37:1315     38:none     39:1174     40:1188
        // ("none" = the radar never reached Destroyed inside the window.)
        // Seed 5 is the earliest clean destroy, at tick 1174 (~39 s), leaving
        // the full 25 s settle + 15 s check window inside the run.
        seed: Some(5),
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

/// Issue #1028: four civilians on authored lanes, one order each, walked all
/// the way through the compliance machine in a real run — and one of them left
/// alone long enough to prove the lane itself is being flown.
///
/// `probe_civilian_traffic.toml` is a deliberate tripwire for the whole chain
/// rather than a scenario that happens to contain traffic. Every outcome the
/// machine can produce is on screen at once, and the two a console must be able
/// to tell apart — `refused` (declined, carried on) and `non_compliant`
/// (agreed, then stuck) — are produced by two different craft on the same tick,
/// because a probe that produced only one of them would pass with the two
/// folded together.
#[test]
fn civilian_orders_walk_the_compliance_machine_while_the_lane_keeps_being_flown() {
    use project_phoenix::civilian::{CivilianTraffic, ComplianceState, REASON_UNABLE};
    use project_phoenix::entity_spawner::{BehaviourSection, EntityName};

    const KESTREL: &str = "world.entity.hauler_kestrel.name";
    const WREN: &str = "world.entity.hauler_wren.name";
    const TEAL: &str = "world.entity.hauler_teal.name";
    const GULL: &str = "world.entity.hauler_gull.name";
    const ROUTE_ID: &str = project_phoenix::civilian::CIVILIAN_ROUTE_OBJECTIVE_ID;

    let dt = 1.0 / 30.0;
    let args = HeadlessArgs {
        world_path: "assets/worlds/probe_civilian_traffic.toml".into(),
        dt,
        // Long enough for the Wren — the one craft nobody diverts — to fly a
        // whole leg of its circuit, sit out the authored dwell at the northern
        // anchor, and set off on the next leg.
        max_ticks: ticks_for_sim_seconds(30.0, dt),
        deterministic: true,
        seed: Some(42),
        ..test_args()
    };
    let mut app = build_headless_app(&args).expect("app should build");

    // Every compliance state each craft passed through, in order and without
    // repeats: the SEQUENCE is the assertion, not the endpoint.
    let mut seen: std::collections::BTreeMap<String, Vec<ComplianceState>> = Default::default();
    let mut legs: std::collections::BTreeMap<String, usize> = Default::default();
    let mut dwelled = false;
    for _ in 0..args.max_ticks {
        run(&mut app, 1);
        let now = app
            .world()
            .resource::<project_phoenix::sim_tick::SimTick>()
            .0;
        let mut q = app
            .world_mut()
            .query::<(&EntityName, &CivilianTraffic, &BehaviourSection)>();
        for (name, traffic, behaviour) in q.iter(app.world()) {
            let row = seen.entry(name.0.clone()).or_default();
            if row.last() != Some(&traffic.0.compliance()) {
                row.push(traffic.0.compliance());
            }
            let leg = legs.entry(name.0.clone()).or_insert(0);
            *leg = (*leg).max(traffic.0.leg());
            // The authored 3 s dwell at the northern anchor, observed as the one
            // thing it is: a craft that KEEPS its lane directive, at zero
            // throttle. Read inside the loop because by the end it is over.
            if name.0 == WREN && traffic.0.is_dwelling(now) {
                dwelled = behaviour.0.doctrine.iter().any(|d| {
                    d.id == ROUTE_ID
                        && d.directive_kind.as_deref() == Some("Patrol")
                        && d.target_speed == 0.0
                });
            }
        }
    }

    // ── AC7: route progress. The Wren refuses its only order and is otherwise
    // left alone, so it is the craft that proves the lane is real.
    assert_eq!(
        legs.get(WREN).copied(),
        Some(1),
        "the Wren must have reached the northern anchor and moved on to the next \
         leg — the PatrolCursor IS the leg pointer, so a leg that never advances \
         is a lane nobody is flying. Legs seen: {legs:?}"
    );
    assert!(
        dwelled,
        "…and the authored 3 s dwell at that anchor must show up as the lane \
         directive held at zero throttle, rather than as the craft being taken \
         off its lane and put back on it"
    );

    // ── AC4/AC6: the four outcomes, each its own sequence.
    assert_eq!(
        seen.get(KESTREL).map(Vec::as_slice),
        Some(
            [
                ComplianceState::Unordered,
                ComplianceState::Received,
                ComplianceState::Acknowledged,
                ComplianceState::Complying,
                // The hold at t = 12 s: a second order, on a craft already
                // complying with the first.
                ComplianceState::Received,
                ComplianceState::Acknowledged,
                ComplianceState::Complying,
            ]
            .as_slice()
        ),
        "the Kestrel takes its divert, then its hold, and every intermediate \
         state is visible rather than skipped: {seen:?}"
    );
    assert_eq!(
        seen.get(WREN).map(Vec::as_slice),
        Some(
            [
                ComplianceState::Unordered,
                ComplianceState::Received,
                ComplianceState::Refused,
            ]
            .as_slice()
        ),
        "the Wren RECEIVES the order like everyone else — an uncooperative craft \
         still hears you — and then refuses straight out of `received`, never \
         acknowledging its way to complying: {seen:?}"
    );
    assert_eq!(
        seen.get(TEAL).map(Vec::as_slice),
        Some(
            [
                ComplianceState::Unordered,
                ComplianceState::Received,
                ComplianceState::Acknowledged,
                ComplianceState::Complying,
            ]
            .as_slice()
        ),
        "the Teal takes its dock order: {seen:?}"
    );
    assert_eq!(
        seen.get(GULL).map(Vec::as_slice),
        Some(
            [
                ComplianceState::Unordered,
                ComplianceState::Received,
                ComplianceState::Acknowledged,
                ComplianceState::NonCompliant,
            ]
            .as_slice()
        ),
        "the Gull agrees to dock at a berth this world does not have and lands in \
         `non_compliant` — a DIFFERENT state from the Wren's refusal, which is \
         the distinction the whole vocabulary exists for: {seen:?}"
    );

    // ── The state each craft is left in, and the directive behind it.
    let mut q = app
        .world_mut()
        .query::<(&EntityName, &CivilianTraffic, &BehaviourSection)>();
    let rows: std::collections::BTreeMap<String, (String, Option<String>, Option<String>)> = q
        .iter(app.world())
        .map(|(name, traffic, behaviour)| {
            let entry = behaviour.0.doctrine.iter().find(|d| d.id == ROUTE_ID);
            (
                name.0.clone(),
                (
                    traffic.0.route().unwrap_or_default().to_string(),
                    entry.and_then(|d| d.directive_kind.clone()),
                    traffic.0.reason().map(str::to_string),
                ),
            )
        })
        .collect();

    assert_eq!(
        rows.get(KESTREL).map(|r| (r.0.as_str(), r.1.as_deref())),
        Some(("storm_detour", None)),
        "a complied divert BECOMES the craft's own lane, and the later hold then \
         takes its directive away entirely — a held craft is flown by the \
         existing no-objective arm, not by a stop command this slice invented: \
         {rows:?}"
    );
    assert_eq!(
        rows.get(WREN).map(|r| (r.0.as_str(), r.1.as_deref())),
        Some(("depot_run", Some("Patrol"))),
        "a refusal is a decision, so the Wren is still flying its own circuit: \
         {rows:?}"
    );
    assert_eq!(
        rows.get(TEAL).map(|r| (r.0.as_str(), r.1.as_deref())),
        Some(("depot_run", Some("Dock"))),
        "the Teal is under a Dock directive — the one addition to the shared \
         directive vocabulary — while the lane it will return to is untouched \
         underneath: {rows:?}"
    );
    assert_eq!(
        rows.get(GULL).map(|r| (r.1.as_deref(), r.2.as_deref())),
        Some((None, Some(REASON_UNABLE))),
        "a stuck craft stops where it is and says why, rather than wandering back \
         onto its lane as if nothing happened: {rows:?}"
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

// ── Scripted entity removal (issue #1033, parent #851) ───────────────────────

/// The two names `probe_destroy.toml` addresses by their authored `[[entity]]`
/// name; the storm bands are script-spawned and carry plain identifiers.
const DESTROY_TENDER: &str = "world.probe_destroy.entity.tender.name";
const DESTROY_SKYHOOK: &str = "world.probe_destroy.entity.skyhook.name";

/// Whether an entity with this `EntityName` is still in the world.
fn named_entity_present(app: &mut bevy::prelude::App, name: &str) -> bool {
    app.world_mut()
        .query::<&project_phoenix::entities::spawner::EntityName>()
        .iter(app.world())
        .any(|entity_name| entity_name.0 == name)
}

/// The status of the objective with this id, which must exist.
fn objective_status(
    app: &bevy::prelude::App,
    id: &str,
) -> project_phoenix::core::messages::ObjectiveStatus {
    app.world()
        .resource::<project_phoenix::world::server::ObjectiveManagerRes>()
        .0
        .sorted_snapshots()
        .into_iter()
        .find(|o| o.id == id)
        .unwrap_or_else(|| panic!("the world never posted objective '{id}'"))
        .status
}

/// **Issue #1033.** `probe_destroy.toml` driven for twelve mission seconds: a
/// script destroys a structure at a named deadline, and every consequence a
/// combat kill would have follows from it.
///
/// This is the whole slice on one real run — the effect, the chaining
/// `WorldEvent::Destroyed`, the `on_destroyed` that rides it, the group
/// `on_all_destroyed` that needs the LAST member to go, the deferred
/// `ctx.schedule.in_seconds(n).destroy_entity(…)` form, and an operation whose
/// target is destroyed out from under it. The probe world's own header carries
/// the authored timeline this is asserted against.
///
/// Nothing in this world shoots at anything: the tender and the player share a
/// faction and the storm bands are regions. So every destruction observed here
/// is a scripted one, which is what makes the assertions mean what they say.
#[test]
fn a_scripted_destroy_chains_its_triggers_and_ends_the_operation_on_its_target() {
    use project_phoenix::core::messages::ObjectiveStatus;
    use project_phoenix::operations::{HoldState, Ineligibility, OperationVerb};
    use project_phoenix::world::server::WorldContentRuntime;

    let dt = 1.0 / 60.0;
    let args = HeadlessArgs {
        world_path: "assets/worlds/probe_destroy.toml".into(),
        dt,
        max_ticks: ticks_for_sim_seconds(12.0, dt),
        deterministic: true,
        seed: Some(42),
        ..test_args()
    };
    let mut app = build_headless_app(&args).expect("app should build");

    // Everything is read tick by tick and asserted on ORDER, not on frame
    // arithmetic. The mission clock anchors on the first `InProgress` tick rather
    // than at frame zero, so an assertion pinned to an absolute frame is really an
    // assertion about how long the lobby took — while the causal order is what the
    // slice actually claims. `first[…]` is the sim-second each reading first went
    // true.
    let mut first: std::collections::BTreeMap<&str, f64> = std::collections::BTreeMap::new();
    // Was the tender ever genuinely mid-operation? A hold that never opened would
    // "fail on target gone" for free.
    let mut held_before_collapse = false;
    // Was there a window where exactly one band was down and the group had not
    // fired? Without it, "fires at the end" is satisfied by "fires on the first".
    let mut group_silent_at_one_down = false;
    // The bands are SPAWNED at t=0 rather than authored as `[[entity]]` blocks, so
    // "absent" reads true for the first few frames too. A removal is only counted
    // once the band has genuinely been there — the same not-published-yet guard
    // `an_infrastructure_threshold_flips_its_flag_in_both_directions_in_a_real_run`
    // carries, and for the same reason: without it every band reads retired on
    // frame one and the ordering assertions below pass vacuously.
    let (mut band_a_seen, mut band_b_seen) = (false, false);

    for tick in 0..args.max_ticks {
        run(&mut app, 1);
        let sim_t = (tick + 1) as f64 * dt;

        let skyhook = named_entity_present(&mut app, DESTROY_SKYHOOK);
        let band_a = named_entity_present(&mut app, "storm_band_a");
        let band_b = named_entity_present(&mut app, "storm_band_b");
        let state = operations_named(&mut app, DESTROY_TENDER)
            .and_then(|ops| ops.active)
            .map(|hold| hold.state());
        let flags = &app.world().resource::<WorldContentRuntime>().flags;
        let cleared = flags.counter("band_cleared");

        if skyhook {
            if state == Some(HoldState::Holding) {
                held_before_collapse = true;
            }
        } else {
            first.entry("skyhook_gone").or_insert(sim_t);
        }
        if flags.counter("skyhook_lost") > 0 {
            first.entry("skyhook_lost_hook").or_insert(sim_t);
        }
        if state == Some(HoldState::Failed(Ineligibility::TargetGone)) {
            first.entry("hold_failed_target_gone").or_insert(sim_t);
        }
        band_a_seen |= band_a;
        band_b_seen |= band_b;
        if band_a_seen && !band_a {
            first.entry("band_a_gone").or_insert(sim_t);
        }
        if band_b_seen && !band_b {
            first.entry("band_b_gone").or_insert(sim_t);
        }
        if cleared > 0 {
            first.entry("band_cleared").or_insert(sim_t);
        }
        if band_a_seen && !band_a && band_b && cleared == 0 {
            group_silent_at_one_down = true;
        }
    }

    let at = |key: &str| -> f64 {
        *first
            .get(key)
            .unwrap_or_else(|| panic!("'{key}' never happened in this run: {first:?}"))
    };

    // ── The scripted destruction happened, and the operation was live for it ──
    assert!(
        held_before_collapse,
        "precondition: the tender must be MID-operation while the skyhook still \
         stands — otherwise the TargetGone below is free. Seen: {first:?}"
    );
    let gone_at = at("skyhook_gone");

    // ── It chained. This is the acceptance the whole slice turns on ──
    let hook_at = at("skyhook_lost_hook");
    assert!(
        hook_at - gone_at < 0.2,
        "the chained `on_destroyed` must fire off the scripted removal essentially \
         at once — its Destroyed event rides `new_events` into the SAME tick's next \
         chaining pass. Destroyed at {gone_at:.2} s, hook at {hook_at:.2} s"
    );
    assert_eq!(
        app.world()
            .resource::<WorldContentRuntime>()
            .flags
            .counter("skyhook_lost"),
        1,
        "exactly once — a destroy must not chain its event twice"
    );
    assert_eq!(
        objective_status(&app, "obj-hold-skyhook"),
        ObjectiveStatus::Failed,
        "the chained handler's effect landed: a scripted destruction drives mission \
         state exactly as a kill does"
    );

    // ── The operation settled on its own reason ──
    // `TargetGone`, not `OutOfRange`: the tender never moved, and waiting cannot
    // bring a skyhook back.
    let failed_at = at("hold_failed_target_gone");
    assert!(
        failed_at >= gone_at && failed_at - gone_at < 0.5,
        "an operation whose target is destroyed mid-flight must end promptly, \
         through the SAME eligibility spine a hull shot out from under a tug goes \
         through — with nothing in the destroy path knowing an operation was \
         running. Destroyed at {gone_at:.2} s, settled at {failed_at:.2} s"
    );
    assert_eq!(
        hold_of(&mut app, DESTROY_TENDER).state(),
        HoldState::Failed(Ineligibility::TargetGone),
        "…and it STAYS failed: a settled operation does not quietly resume"
    );
    assert_eq!(
        hold_of(&mut app, DESTROY_TENDER).verb(),
        OperationVerb::Stabilise
    );

    // ── The group fired on the LAST member, not the first ──
    let band_a_at = at("band_a_gone");
    let band_b_at = at("band_b_gone");
    let cleared_at = at("band_cleared");
    assert!(
        band_a_at < band_b_at,
        "precondition: the bands must be retired one at a time ({band_a_at:.2} s, \
         {band_b_at:.2} s)"
    );
    assert!(
        group_silent_at_one_down,
        "one of two members down is not the whole group: there must be a window \
         where band A is gone, band B is up, and `on_all_destroyed` has NOT fired. \
         Without it, a group that fired on the first kill would pass the assertion \
         below just as well. Seen: {first:?}"
    );
    assert!(
        cleared_at >= band_b_at && cleared_at - band_b_at < 0.2,
        "the group fires when the LAST member goes — the storm-band teardown this \
         effect exists for. Last band at {band_b_at:.2} s, group at {cleared_at:.2} s"
    );
    assert_eq!(
        app.world()
            .resource::<WorldContentRuntime>()
            .flags
            .counter("band_cleared"),
        1,
        "exactly once"
    );
    assert_eq!(
        objective_status(&app, "obj-clear-band"),
        ObjectiveStatus::Completed,
        "…and its handler's effect landed"
    );

    // ── The deferred form did the first band's work ──
    // Authored as `on_timer(6)` scheduling two seconds out, so band A goes at
    // t≈8 — after the t=5 collapse and before the t=10 immediate destroy. Had the
    // deferred call been dropped, `band_a_gone` would never have happened at all
    // and `at()` would have failed above.
    assert!(
        band_a_at > gone_at,
        "the DEFERRED `in_seconds(2).destroy_entity` fires after the collapse it \
         was scheduled behind — same action, same queue, no new machinery \
         ({band_a_at:.2} s vs {gone_at:.2} s)"
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
    // (`simulate_low_lod_ships`), the collision responder (`handle_collisions`),
    // blaster recoil (`tick_blaster_system`), the tow rig (`move_towed_targets`,
    // issue #1027), the tractor rig (`move_coupled_target`, issue #1156) and the
    // dock controller (`tick_dock`, issue #1159). The table's remaining entry,
    // `handle_slow_zone_speed_clamp`, is an observer and so is in no schedule —
    // see `ship_physics_writers`.
    assert_eq!(
        writers.len(),
        7,
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

    // Exactly five of them are unfiltered corrections (collision response,
    // blaster recoil, the tow rig, the tractor rig and the dock controller): they
    // deliberately apply to every ship, high-LOD and low-LOD alike, and their
    // safety argument is that they are one-shot corrections rather than
    // integrators — not filter disjointness. The tow, the tractor and the dock are
    // unfiltered on purpose: a demoted freighter under tow is exactly the case
    // that has to keep working, and dead reckoning it away from the rig would drag
    // it out of the operator's wake; the dock likewise places only its own hull
    // onto the mate pose as a last-writer correction.
    let unfiltered = (0..writers.len())
        .filter(|i| high_fi.contains(i) && low_lod.contains(i))
        .count();
    assert_eq!(
        unfiltered, 5,
        "expected exactly five unfiltered ShipPhysics correction writers (collision \
         response, blaster recoil, the tow rig, the tractor rig and the dock controller). \
         A change here means a correction grew an `AiHighFidelity` filter, or an integrator \
         lost one — either way the set of ships that get moved twice per tick has changed. \
         Reconcile with the writer-policy table on `ShipPhysics` (src/ship/state.rs). \
         ShipPhysics writers found:\n{inventory}"
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
        let mut q = app.world_mut().query_filtered::<(
            &ShipSystemControlSources,
            &ActiveStationRatings,
            &ShipConfigComponent,
        ), With<LocalShip>>();
        let (sources, ratings, config) = q.single(app.world()).expect("exactly one LocalShip");
        // `seed_boot_ratings` inserts one entry per authored station with no
        // auxiliary filter, so the raw total counts the auxiliary stations
        // (Navigation, Comms, Command) too. The real invariant is the number of
        // *crewable* (non-auxiliary) seats — auxiliary stations are data-authored
        // directors/hosted tabs, not seats a human can take — so a future
        // auxiliary addition must not be silently accepted here as a new seat.
        let crewable = config.0.stations.iter().filter(|s| !s.auxiliary).count();
        assert_eq!(
            crewable, 4,
            "the destroyer's four crewable seats must survive composition: {:?}",
            ratings.0
        );
        assert_eq!(
            ratings.0.len(),
            7,
            "four crewable seats + three auxiliary stations (navigation, comms, \
             command) each boot a rating: {:?}",
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
    //
    // RE-BLESSED AGAIN at issue #1053, from `> 250` to `> 200` (observed: 228).
    // `probe_duel` is where #1053 was MEASURED — `probe_hostile` is the hull
    // that was seen going 67.5 -> 54.0 in a single tick — so it is the world
    // the over-cap bleed bites hardest in. A hull that sheds helm power at
    // flank now keeps its speed for half a second instead of losing it
    // instantly, which means a higher average speed through the shed and a
    // wider turn out of it, and a wider turn spends fewer ticks inside the
    // ring's tangent band. Same character of re-bless as #907's, and the same
    // guards still stand between this number and a standoff: both hulls land
    // damage, and the pass surface is published on >100 ticks.
    assert!(
        duel.orbit > 200,
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
    // 280) and from 250 to 200 at issue #1053 (observed: 228), both for the
    // reasons recorded at the sibling assertion in
    // `the_composed_player_cruiser_rings_its_target_and_breaks_off_to_bear_its_tubes`:
    // orbit direction is derived from the ship's uuid and #907 changed how uuids
    // are minted; #1053 stopped an over-cap velocity being deleted in one tick,
    // and `probe_duel` is the world that fix was measured in. It is a floor on
    // "did this run measure the leg at all", not a doctrine assertion, and 200
    // still says yes — `overwritten == 0` below is the assertion that matters
    // and it is unmoved by either re-bless.
    for (name, ring, min_ticks) in [("duel", &duel, 200), ("aggressor", &aggressor, 50)] {
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

/// A helm power level is a DECISION, not a strobe: once the channel moves it
/// must hold the new level for a while before moving again.
///
/// The regression this pins is the AI-thrust-burst defect. `thrust` is a
/// CONTINUOUS fact — `plan_helm_travel` hands out analogue throttles, and the
/// decel ramps sweep smoothly through the whole range — so a helm rule whose
/// HOLD and ELEVATE both read one bare `thrust_threshold` flips the channel on
/// consecutive AI decision arms for as long as the throttle DWELLS near that
/// number. Each flip is a ×1.25/×1.0 `MaxSpeed` swing, which is a ship visibly
/// thrusting in bursts instead of holding a constant burn. The fix is the
/// authored `thrust_release_threshold` — the thrust axis's mirror of the
/// battery axis's `min_reserve_*`/`min_restore_*` pair.
///
/// `rng_coverage` is the vehicle because its two lancers fly a geometry that
/// parks a throttle right on 0.70 and dithers there: before the band,
/// `lancer_alpha` changed helm level 18 times across this window, SIXTEEN of
/// them inside 38 ticks, with the battery at 57-59 % — nowhere near its 50 %
/// shed floor, so the battery ladder was provably not the thing moving it.
///
/// The assertion is on the GAP between changes rather than on their count, and
/// deliberately so: a ship that genuinely changes its travel intention should be
/// free to re-decide, and bounding the total would fight that.
///
/// WHERE THE BOUND COMES FROM, measured on both sides. The defect reversed the
/// channel every 2 to 4 ticks — one or two AI decision arms at the authored
/// 30 Hz, against a 60 Hz sim tick. The banded tree's two CLOSEST changes in
/// this same run are 28 and 16 ticks, and both are legitimate single-axis
/// decisions rather than chatter, which the recorded facts prove:
///
///   * tick 39, gap 28, throttle 0.5911 — the throttle genuinely collapsed
///     past the 0.6 RELEASE floor, a 0.16 move from the 0.7497 it elevated on.
///   * tick 1423, gap 16, throttle 0.7483, reserve 49.83 % — the throttle is
///     still high, so this is the BATTERY axis: the reserve crossed
///     `min_reserve_helm` (50 %) and the shed ladder did its documented job.
///
/// So the bound is set to catch reversals within a handful of AI arms — the
/// strobe's signature — and deliberately NOT to police the ladder's legitimate
/// steps, whose timing belongs to the battery and the doctrine and may fall
/// close behind an elevate. Ten ticks is five decision arms: comfortably above
/// the defect's 2-to-4 and comfortably below the banded tree's 16. Do not
/// tighten it towards 16 without re-reading the two changes above; they are
/// correct behaviour and a tighter bound would forbid them.
#[test]
fn a_helm_power_level_holds_instead_of_strobing_at_the_thrust_threshold() {
    use project_phoenix::entity_spawner::EntityName;
    use project_phoenix::messages::PowerGroupId;
    use project_phoenix::modifiers::power_system::HELM_POWER_GROUP;
    use project_phoenix::ship::power::ShipPowerSystem;
    use project_phoenix::simulation::Ship;
    use std::collections::BTreeMap;

    /// Minimum ticks a helm level must hold before it may move again. Anything
    /// shorter is chatter rather than a decision — see the bound's derivation in
    /// this test's doc comment before changing it.
    const MIN_DWELL_TICKS: u64 = 10;

    let args = HeadlessArgs {
        world_path: "assets/worlds/rng_coverage.toml".into(),
        max_ticks: 0, // driven by hand below
        seed: Some(42),
        deterministic: true,
        ..test_args()
    };
    let mut app = build_headless_app(&args).expect("rng_coverage must build an app");
    app.finish();
    app.cleanup();

    let helm = PowerGroupId(HELM_POWER_GROUP.into());
    /// One recorded helm level change, carrying the two facts that could have
    /// caused it. A bare (from, to, tick) says a level moved; the throttle and
    /// the reserve beside it say which axis moved it, which is the whole
    /// difference between a decision and the strobe.
    #[derive(Debug)]
    #[allow(dead_code)] // read only through the Debug rendering in the message
    struct Change {
        from: u8,
        to: u8,
        tick: u64,
        gap: Option<u64>,
        thrust: f32,
        battery_pct: f32,
    }
    struct Seen {
        name: String,
        level: u8,
        changed_at: Option<u64>,
        changes: Vec<Change>,
    }
    let mut seen: BTreeMap<Entity, Seen> = BTreeMap::new();

    let mut reactors = app.world_mut().query_filtered::<(
        Entity,
        &ShipPowerSystem,
        Option<&EntityName>,
        Option<&project_phoenix::ship::helm::ThrustInput>,
        Option<&project_phoenix::ship::power::PowerConfigResource>,
    ), With<Ship>>();

    for tick in 0..1800u64 {
        run(&mut app, 1);

        let sampled: Vec<(Entity, u8, String, f32, f32)> = reactors
            .iter(app.world())
            .map(|(e, power, name, thrust, cfg)| {
                let capacity = cfg.map(|c| c.0.capacity).unwrap_or(0.0);
                let pct = if capacity > 0.0 {
                    (power.0.battery_charge / capacity) * 100.0
                } else {
                    f32::NAN
                };
                (
                    e,
                    power.0.level_for(&helm),
                    name.map(|n| n.0.clone()).unwrap_or_else(|| format!("{e}")),
                    thrust.map(|t| t.0).unwrap_or(f32::NAN),
                    pct,
                )
            })
            .collect();

        for (ship, level, name, thrust, battery_pct) in sampled {
            let entry = seen.entry(ship).or_insert_with(|| Seen {
                name: name.clone(),
                level,
                changed_at: None,
                changes: Vec::new(),
            });
            if entry.level == level {
                continue;
            }
            let gap = entry.changed_at.map(|prev| tick - prev);
            entry.changes.push(Change {
                from: entry.level,
                to: level,
                tick,
                gap,
                thrust,
                battery_pct,
            });
            entry.level = level;
            entry.changed_at = Some(tick);
        }
    }

    // The control: a run in which no helm level ever moved would satisfy the
    // dwell assertion without exercising the rule whose text this fix changed.
    let total_changes: usize = seen.values().map(|s| s.changes.len()).sum();
    assert!(
        total_changes > 0,
        "no ship in rng_coverage moved its helm power level at all, so the dwell \
         assertion below passed without the helm rules ever resolving. Ships \
         sampled: {:?}",
        seen.values().map(|s| &s.name).collect::<Vec<_>>()
    );

    for entry in seen.values() {
        for change in &entry.changes {
            let Some(gap) = change.gap else { continue };
            assert!(
                gap >= MIN_DWELL_TICKS,
                "{} strobed its helm power level: {} -> {} at tick {}, only {gap} \
                 ticks after the previous change (minimum dwell is \
                 {MIN_DWELL_TICKS}), with the throttle at {:.4} and the reserve \
                 at {:.1} %. That is the AI-thrust-burst defect — the helm \
                 channel flipping on consecutive AI decision arms because the \
                 HOLD and the ELEVATE read one bare `thrust_threshold` while the \
                 throttle dithers across it. Every change this ship made: {:?}",
                entry.name,
                change.from,
                change.to,
                change.tick,
                change.thrust,
                change.battery_pct,
                entry.changes
            );
        }
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

// ── The commitments ledger with a live consumer (issue #1029, parent #851) ───

/// `probe_commitments.toml`, driven for fifteen mission seconds: a promise is
/// recorded, a **real dialogue node offers an option that exists only because
/// of it**, keeping the promise writes a campaign flag that an `on_flag_set`
/// trigger reacts to, and a deadline breaks the promise nobody kept.
///
/// The live-consumer assertion is the load-bearing one, and it is made against
/// `CommsRuntime::active_dialogues` — the projected node the comms pipeline
/// actually built and would have sent a console — rather than against anything
/// the script says about itself. One authored node fn is opened twice: while
/// `safe_passage` is owed it offers two responses, and after it has been kept it
/// offers one. That is the gate opening AND closing, from the ledger.
///
/// The probe world's own header carries the authored timeline this is asserted
/// against.
#[test]
fn a_promise_gates_a_dialogue_option_and_its_resolution_writes_a_campaign_flag() {
    use project_phoenix::comms::server::CommsRuntime;
    use project_phoenix::world::commitments::CommitmentState;
    use project_phoenix::world::server::WorldContentRuntime;

    let dt = 1.0 / 60.0;
    let args = HeadlessArgs {
        world_path: "assets/worlds/probe_commitments.toml".into(),
        ship_path: "assets/entities/alliance_destroyer.toml".into(),
        dt,
        max_ticks: ticks_for_sim_seconds(15.0, dt),
        seed: Some(1029),
        deterministic: true,
        ..test_args()
    };
    let mut app = build_headless_app(&args).expect("app should build");

    // Every dialogue this run opened, in the order it opened, recorded as the
    // response texts the node actually offered. Sampled every tick because a
    // thread opened at t=4 is still open at t=12 — the pair is what the gate is
    // read off, and it has to be collected as it happens rather than at the end.
    let mut offered: Vec<Vec<String>> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for _ in 0..args.max_ticks {
        run(&mut app, 1);
        let comms = app.world().resource::<CommsRuntime>();
        let mut fresh: Vec<(String, Vec<String>)> = comms
            .active_dialogues
            .iter()
            .filter(|(id, _)| !seen.contains(*id))
            .map(|(id, d)| {
                (
                    id.clone(),
                    d.current_node
                        .responses
                        .iter()
                        .map(|r| r.text.clone())
                        .collect(),
                )
            })
            .collect();
        // `active_dialogues` is a `HashMap`, so a tick that opened two threads
        // could hand them back in either order. Sorting by the minted message id
        // makes the sample deterministic; in this world only one thread ever
        // opens per tick, so this is belt to that brace.
        fresh.sort_by(|a, b| a.0.cmp(&b.0));
        for (id, texts) in fresh {
            seen.insert(id);
            offered.push(texts);
        }
    }

    let runtime = app.world().resource::<WorldContentRuntime>();
    let flags = &runtime.flags;
    let ledger = &runtime.commitments;

    // AC1: both promises are on the books under their own ids, carrying the
    // party, the terms and the stated resolution condition.
    assert_eq!(ledger.records.len(), 2, "two promises were made");
    let passage = ledger
        .get("safe_passage")
        .expect("the id is the lookup key");
    assert_eq!(
        passage.made_to, "skyway_strike_committee",
        "the party is a committee, not a hull — and it is stored as the script \
         wrote it rather than resolved to a UUID"
    );
    assert_eq!(
        passage.terms, "world.probe_commitments.commitment.safe_passage.terms",
        "the terms are a strings.csv id, never English"
    );
    assert_eq!(
        passage.resolves_when, "world.probe_commitments.commitment.safe_passage.resolves",
        "and what would count as keeping it is authored data, not an implication \
         of whichever handler settles it"
    );
    assert_eq!(
        flags.counter("unmade_reads_unknown"),
        1,
        "a promise that was never made reads as unknown — the guard against \
         recording a duplicate, and the answer that is NOT 'broken'"
    );
    assert_eq!(flags.counter("passage_reads_open"), 1);

    // AC5, THE LIVE CONSUMER: one authored node fn, opened twice, offering a
    // different option list each time because the ledger moved between them.
    assert_eq!(
        offered.len(),
        2,
        "the probe opens the committee channel twice; got {offered:?}"
    );
    assert_eq!(
        offered[0],
        vec![
            "world.probe_commitments.comms.stall".to_string(),
            "world.probe_commitments.comms.honour_word".to_string(),
        ],
        "while the promise is OWED the node offers the option that exists only \
         because the captain gave their word"
    );
    assert_eq!(
        offered[1],
        vec!["world.probe_commitments.comms.stall".to_string()],
        "and once it is KEPT that option is gone — the gate closes as well as \
         opens, from the same authored node"
    );

    // AC3: keeping writes a campaign flag, and an `on_flag_set` trigger that
    // knows nothing about commitments reacts to it.
    assert_eq!(
        ledger.get("safe_passage").map(|c| c.state),
        Some(CommitmentState::Kept)
    );
    assert_eq!(
        flags.counter("commitment.safe_passage.kept"),
        1,
        "resolution writes the campaign flag through the ordinary flag path"
    );
    assert_eq!(
        flags.counter("kept_flag_chained"),
        1,
        "and an on_flag_set trigger authored against it fired — the consequence \
         reaching beyond the scene the promise was made in"
    );
    assert_eq!(
        flags.counter("chain_reads_kept"),
        1,
        "by the time that trigger runs, the ledger it can read agrees"
    );

    // The deadline composition: the ledger carries no timer, so the promise
    // nobody kept is broken by a `[[deadline]]` handler.
    assert_eq!(
        flags.counter("records_broken_by_deadline"),
        1,
        "the deadline's handler found the promise still open and settled it"
    );
    assert_eq!(
        ledger.get("surface_records").map(|c| c.state),
        Some(CommitmentState::Broken)
    );
    assert_eq!(flags.counter("commitment.surface_records.broken"), 1);
    assert_eq!(flags.counter("broken_flag_chained"), 1);
    assert_eq!(
        flags.counter("chain_reads_broken"),
        1,
        "a broken promise is distinguishable from an unresolved one all the way \
         out to the handler that reacts to it"
    );

    // The two outcomes never wrote each other's flag.
    assert_eq!(flags.counter("commitment.safe_passage.broken"), 0);
    assert_eq!(flags.counter("commitment.surface_records.kept"), 0);

    // Both promises were settled on ticks, not on a wall clock, and both stamps
    // are after the tick they were made on.
    for record in &ledger.records {
        let resolved = record
            .resolved_at_tick
            .expect("both promises were settled by the end of the run");
        assert!(
            resolved > record.made_at_tick,
            "{} was settled on tick {resolved}, after the tick {} it was made on",
            record.id,
            record.made_at_tick
        );
    }
    assert_eq!(
        ledger.open().count(),
        0,
        "nothing is still owed at the end of this run"
    );
}

// ── Issue #1027: the four remaining verbs, end to end in a real run ──────────

/// The named operator's operations record, or `None`.
///
/// By `EntityName` rather than by component, because `probe_operations.toml`
/// runs five operators at once and "the first one the query yielded" would be a
/// different ship on a different day.
fn operations_named(
    app: &mut bevy::prelude::App,
    name: &str,
) -> Option<project_phoenix::operations::ShipOperations> {
    app.world_mut()
        .query::<(
            &project_phoenix::entities::spawner::EntityName,
            &project_phoenix::operations::ShipOperations,
        )>()
        .iter(app.world())
        .find(|(entity_name, _)| entity_name.0 == name)
        .map(|(_, ops)| ops.clone())
}

/// The named entity's live hold, which must exist.
fn hold_of(app: &mut bevy::prelude::App, name: &str) -> project_phoenix::operations::OperationHold {
    operations_named(app, name)
        .unwrap_or_else(|| panic!("{name} carries no operations record"))
        .active
        .unwrap_or_else(|| panic!("{name} is running no operation"))
}

/// The named entity's world position, read off its transform.
fn position_of(app: &mut bevy::prelude::App, name: &str) -> bevy::prelude::Vec3 {
    app.world_mut()
        .query::<(
            &project_phoenix::entities::spawner::EntityName,
            &bevy::prelude::Transform,
        )>()
        .iter(app.world())
        .find(|(entity_name, _)| entity_name.0 == name)
        .map(|(_, transform)| transform.translation)
        .unwrap_or_else(|| panic!("{name} is not in this world"))
}

/// The named structure's infrastructure condition in points.
fn condition_of(app: &mut bevy::prelude::App, name: &str) -> f32 {
    app.world_mut()
        .query::<(
            &project_phoenix::entities::spawner::EntityName,
            &project_phoenix::infrastructure::InfrastructureCondition,
        )>()
        .iter(app.world())
        .find(|(entity_name, _)| entity_name.0 == name)
        .map(|(_, condition)| condition.0.condition())
        .unwrap_or_else(|| panic!("{name} carries no condition track"))
}

/// The named structure's live level for a named capacity.
fn capacity_of(app: &mut bevy::prelude::App, name: &str, capacity: &str) -> i64 {
    app.world_mut()
        .query::<(
            &project_phoenix::entities::spawner::EntityName,
            &project_phoenix::infrastructure::InfrastructureCondition,
        )>()
        .iter(app.world())
        .find(|(entity_name, _)| entity_name.0 == name)
        .and_then(|(_, condition)| condition.0.capacity(capacity))
        .unwrap_or_else(|| panic!("{name} carries no {capacity} capacity"))
}

/// Move the named ship by writing its `ShipPhysics`, which is what helm moves.
/// Writing the `Transform` directly would be undone by `sync_ship_position`.
fn move_named_to(app: &mut bevy::prelude::App, name: &str, position: bevy::prelude::Vec3) {
    let entity = app
        .world_mut()
        .query::<(
            bevy::prelude::Entity,
            &project_phoenix::entities::spawner::EntityName,
        )>()
        .iter(app.world())
        .find(|(_, entity_name)| entity_name.0 == name)
        .map(|(entity, _)| entity)
        .unwrap_or_else(|| panic!("{name} is not in this world"));
    let mut physics = app
        .world_mut()
        .get_mut::<project_phoenix::ship::state::ShipPhysics>(entity)
        .unwrap_or_else(|| panic!("{name} is not a ship"));
    physics.x = position.x;
    physics.y = position.y;
    physics.z = position.z;
}

const TUG: &str = "world.probe_operations.entity.tug.name";
const HULK: &str = "world.probe_operations.entity.hulk.name";
const OUTRIDER: &str = "world.probe_operations.entity.outrider.name";
const CONVOY: &str = "world.probe_operations.entity.convoy.name";
const TENDER: &str = "world.probe_operations.entity.tender.name";
const BERTH: &str = "world.probe_operations.entity.berth.name";
const SISTER: &str = "world.probe_operations.entity.sister.name";
const DEPOT_CLEAR: &str = "world.probe_operations.entity.depot_clear.name";
const PACKHORSE: &str = "world.probe_operations.entity.packhorse.name";
const DEPOT_STORM: &str = "world.probe_operations.entity.depot_storm.name";
const THROUGHPUT: &str = "depot_transfer_throughput";

fn operations_args(dt: f64, seconds: f64) -> HeadlessArgs {
    HeadlessArgs {
        world_path: "assets/worlds/probe_operations.toml".into(),
        dt,
        max_ticks: ticks_for_sim_seconds(seconds, dt),
        deterministic: true,
        seed: Some(42),
        ..test_args()
    }
}

/// **Issue #1027, tow.** The load's position becomes the tug's rig and stays
/// there as the tug moves; a tow that stalls lets go.
#[test]
fn a_towed_hulk_rides_the_tugs_rig_and_a_stalled_tow_lets_it_go() {
    use project_phoenix::operations::{HoldState, Ineligibility, OperationVerb};

    let dt = 1.0 / 60.0;
    let mut app = build_headless_app(&operations_args(dt, 20.0)).expect("app should build");
    run(&mut app, 130);

    let hold = hold_of(&mut app, TUG);
    assert_eq!(hold.verb(), OperationVerb::Tow);
    assert_eq!(
        hold.state(),
        HoldState::Holding,
        "the scripted tow opened just after the t=1 s timer"
    );

    // The rig, not the hulk's own last position: it spawned 80 units to
    // starboard of the tug and is now 120 astern of it.
    let offset = position_of(&mut app, HULK) - position_of(&mut app, TUG);
    assert!(
        (offset.length() - 120.0).abs() < 1.0,
        "the load rides the authored 120-unit tow offset, not wherever it happened to be — got a \
         separation of {}",
        offset.length()
    );

    // The tug moves; the load moves with it.
    move_named_to(&mut app, TUG, bevy::prelude::Vec3::new(900.0, 0.0, -600.0));
    run(&mut app, 10);
    let carried = position_of(&mut app, HULK) - position_of(&mut app, TUG);
    assert!(
        (carried.length() - 120.0).abs() < 1.0,
        "…and goes on riding it after the tug moves a kilometre, which is the whole of what a tow \
         is. Got {}",
        carried.length()
    );
    assert!(
        position_of(&mut app, HULK).distance(bevy::prelude::Vec3::new(80.0, 0.0, 0.0)) > 500.0,
        "the hulk really did travel — a test that only compared the two positions would pass with \
         both of them sitting at the origin"
    );

    // Out of range: the tow stalls, and the towline parts.
    let parted_at = position_of(&mut app, HULK);
    move_named_to(
        &mut app,
        TUG,
        bevy::prelude::Vec3::new(90_000.0, 0.0, 90_000.0),
    );
    run(&mut app, 30);
    assert_eq!(
        hold_of(&mut app, TUG).state(),
        HoldState::Stalled(Ineligibility::OutOfRange),
        "flying beyond the authored range stalls the tow rather than ending it"
    );
    assert!(
        position_of(&mut app, HULK).distance(parted_at) < 200.0,
        "…and a stalled tow has LET GO: the load stays roughly where the towline parted rather \
         than being yanked ninety kilometres across the map to wherever the tug got to"
    );
}

/// **Issue #1027, escort.** The hold advances against an escortee that is
/// travelling under its own power, and ends — terminally — when it gets past
/// the authored separation limit.
#[test]
fn an_escort_holds_on_a_moving_convoy_and_fails_when_it_is_lost() {
    use project_phoenix::operations::{HoldState, Ineligibility, OperationVerb};

    let dt = 1.0 / 60.0;
    let mut app = build_headless_app(&operations_args(dt, 20.0)).expect("app should build");
    run(&mut app, 70);

    let opened = hold_of(&mut app, OUTRIDER);
    assert_eq!(opened.verb(), OperationVerb::Escort);
    let convoy_start = position_of(&mut app, CONVOY);
    // Fifteen seconds. A bulk hauler makes about five units a second, so this
    // is the window in which its travel becomes unmistakable rather than
    // arguable — the assertion below is the point of the test and it should not
    // be scraping its own threshold.
    run(&mut app, 900);

    let held = hold_of(&mut app, OUTRIDER);
    assert_eq!(held.state(), HoldState::Holding);
    assert!(
        held.elapsed_ticks() > opened.elapsed_ticks() + 800,
        "the escort banked eligible time throughout — progress ran from {} to {} ticks",
        opened.elapsed_ticks(),
        held.elapsed_ticks()
    );
    let travelled = position_of(&mut app, CONVOY).distance(convoy_start);
    assert!(
        travelled > 50.0,
        "…and the escortee really was MOVING while it did: it covered {travelled:.1} units on its \
         own lane. An escort that only held station on something parked would pass a proximity \
         test without ever exercising the thing escort is for."
    );

    // The convoy runs for it, past the authored separation limit.
    move_named_to(
        &mut app,
        CONVOY,
        bevy::prelude::Vec3::new(200_000.0, 0.0, 200_000.0),
    );
    run(&mut app, 10);
    assert_eq!(
        hold_of(&mut app, OUTRIDER).state(),
        HoldState::Failed(Ineligibility::Separated),
        "past the separation limit the relationship is OVER, not stalled — a hold that stalled \
         here would sit waiting for a convoy that has gone, and the crew would never be told"
    );
}

/// **Issue #1027, transfer.** The load moves between two infrastructure
/// entities, and a second delivery into a depot the first one filled stalls on
/// the destination's capacity rather than moving anything.
#[test]
fn a_transfer_moves_a_capacity_between_two_depots_and_stalls_when_the_far_end_is_full() {
    use project_phoenix::operations::{HoldState, Ineligibility};
    use project_phoenix::world::server::WorldContentRuntime;

    let dt = 1.0 / 60.0;
    let mut app = build_headless_app(&operations_args(dt, 20.0)).expect("app should build");

    // Before: forty aboard the tender, twenty in a depot whose ceiling is forty.
    run(&mut app, 30);
    assert_eq!(capacity_of(&mut app, TENDER, THROUGHPUT), 40);
    assert_eq!(capacity_of(&mut app, BERTH, THROUGHPUT), 20);

    // The first delivery opens at t=2 s and needs six authored seconds.
    run(&mut app, 540);
    assert_eq!(
        hold_of(&mut app, TENDER).state(),
        HoldState::Completed,
        "the first transfer ran to term"
    );
    assert_eq!(
        (
            capacity_of(&mut app, TENDER, THROUGHPUT),
            capacity_of(&mut app, BERTH, THROUGHPUT),
        ),
        (20, 40),
        "twenty berths moved off the tender and into the depot — BOTH ends, on the same tick. A \
         transfer that only credited the destination would create capacity out of nothing."
    );
    assert_eq!(
        app.world()
            .resource::<WorldContentRuntime>()
            .flags
            .counter(THROUGHPUT),
        40,
        "…and the world-flag counter a scenario predicate reads was RE-published, rather than \
         still carrying the number the depot was authored with. That mirror is why the move goes \
         through tick_infrastructure_condition instead of onto the component."
    );

    // The second delivery opens at t=12 s against a depot now at its ceiling.
    run(&mut app, 660);
    let blocked = hold_of(&mut app, TENDER);
    assert_eq!(
        blocked.state(),
        HoldState::Stalled(Ineligibility::CapacityUnavailable),
        "a start is only refused for a capability the hull lacks, so the second transfer OPENS — \
         and then stalls, because the depot is full. That is the readable behaviour: the crew are \
         alongside with cargo and nowhere to put it."
    );
    assert_eq!(
        blocked.elapsed_ticks(),
        0,
        "and it banks nothing while it waits"
    );
    assert_eq!(
        (
            capacity_of(&mut app, TENDER, THROUGHPUT),
            capacity_of(&mut app, BERTH, THROUGHPUT),
        ),
        (20, 40),
        "…and moves nothing. The tender still has twenty aboard, so this refusal is a fact about \
         the DESTINATION rather than about the source — which is the half a one-ended check would \
         have missed."
    );
}

/// **Issue #1027, field-repair.** Condition is paid per tick rather than on
/// completion, and a storm band stretches the work — measured against a control
/// tender doing the identical job in clear space.
#[test]
fn a_field_repair_pays_as_it_works_and_a_storm_band_halves_it() {
    use project_phoenix::operations::{HoldState, OperationVerb, ProgressRate};

    let dt = 1.0 / 60.0;
    let mut app = build_headless_app(&operations_args(dt, 30.0)).expect("app should build");
    run(&mut app, 70);

    assert_eq!(hold_of(&mut app, SISTER).verb(), OperationVerb::FieldRepair);
    assert_eq!(
        hold_of(&mut app, PACKHORSE).rate(),
        ProgressRate::percent(50),
        "the tender parked inside the band is working at the rate its capability authored for a \
         slow zone — through the ship's REAL region membership, not a flag something else set"
    );
    assert_eq!(
        hold_of(&mut app, SISTER).rate(),
        ProgressRate::FULL,
        "…and the one six radii clear of it is not"
    );

    // Part way through: BOTH have already paid, which is what makes this
    // field-repair rather than stabilise.
    run(&mut app, 300);
    let clear_mid = condition_of(&mut app, DEPOT_CLEAR);
    let storm_mid = condition_of(&mut app, DEPOT_STORM);
    assert!(
        clear_mid > 41.0 && !hold_of(&mut app, SISTER).is_settled(),
        "five seconds into a twenty-second repair the depot is ALREADY better off ({clear_mid}), \
         with the hold still running. A crew pulled off here keep what they did — which is the \
         whole difference between a repair and a stabilise."
    );
    assert!(
        storm_mid > 40.0 && storm_mid < clear_mid,
        "the tender in the band has done real work too, just less of it: {storm_mid} against \
         {clear_mid}"
    );

    // Let the clear repair run to term.
    run(&mut app, 900);
    assert_eq!(
        hold_of(&mut app, SISTER).state(),
        HoldState::Completed,
        "twenty authored seconds of eligible time"
    );
    let clear_total = condition_of(&mut app, DEPOT_CLEAR) - 40.0;
    assert!(
        (clear_total - 40.0).abs() < 1.5,
        "two points a second for twenty seconds is forty points: got {clear_total}"
    );

    let storm_hold = hold_of(&mut app, PACKHORSE);
    assert!(
        !storm_hold.is_settled(),
        "…while the one in the band is still going, because the band STRETCHED it rather than \
         cancelling it. That is the storm mechanic in one assertion."
    );
    assert_eq!(
        storm_hold.stalled_ticks(),
        0,
        "and it never stalled once, so no authored stall budget would ever have ended it — a \
         slowed operation is being held, just badly"
    );
    let storm_total = condition_of(&mut app, DEPOT_STORM) - 40.0;
    assert!(
        (storm_total - clear_total / 2.0).abs() < 2.0,
        "the payout tracks the rate exactly: half speed, half the repair. {storm_total} against \
         {clear_total} in clear space. If these matched, parking in a storm would be free."
    );
}

/// **Issue #1027, capacity as cost.** A tender holding a field-repair has a
/// repair team committed for the duration and released when it settles — and
/// the team never leaves the hull.
#[test]
fn a_field_repair_commits_one_of_the_tenders_own_teams_without_moving_it() {
    use project_phoenix::messages::TeamSlot;

    let dt = 1.0 / 60.0;
    let mut app = build_headless_app(&operations_args(dt, 30.0)).expect("app should build");
    run(&mut app, 130);

    let ops = operations_named(&mut app, SISTER).expect("the sister carries an operations record");
    assert_eq!(
        ops.committed_repair_teams(),
        1,
        "the running field-repair holds the capability's authored team count"
    );

    let slots = app
        .world_mut()
        .query::<(
            &project_phoenix::entities::spawner::EntityName,
            &project_phoenix::console::repair::server::ShipRepairTeams,
        )>()
        .iter(app.world())
        .find(|(name, _)| name.0 == SISTER)
        .map(|(_, teams)| teams.0.slots().to_vec())
        .expect("the sister carries a repair roster");
    assert!(
        slots.iter().all(|slot| matches!(slot, TeamSlot::Idle)),
        "THE TEAMS NEVER LEAVE THE HULL. Nothing is dispatched, nothing travels, and the console \
         goes on showing idle teams — the commitment is a reservation the dispatchers honour, not \
         a trip. Got {slots:?}"
    );

    // Let it finish, and the team comes back.
    run(&mut app, 1_400);
    let settled = operations_named(&mut app, SISTER).expect("the record survives");
    assert!(settled.active.as_ref().is_some_and(|h| h.is_settled()));
    assert_eq!(
        settled.committed_repair_teams(),
        0,
        "released on completion, without a release step anyone had to remember to write: the \
         commitment is derived from the live hold, so a settled hold commits nothing"
    );
}

/// **Issue #1027.** All five verbs reach the wire under the operations
/// blackboard channel, each carrying its own progress and rate.
#[test]
fn every_operating_ship_publishes_its_verb_and_its_rate_on_the_wire() {
    use project_phoenix::messages::SystemBlackboard;

    let dt = 1.0 / 60.0;
    let mut app = build_headless_app(&operations_args(dt, 10.0)).expect("app should build");
    run(&mut app, 200);

    let key = project_phoenix::messages::SystemId(
        project_phoenix::operations::OPERATIONS_BLACKBOARD_KEY.to_string(),
    );
    let mut published: Vec<(String, u16)> = app
        .world_mut()
        .query::<&project_phoenix::server_app::ShipSystemBlackboards>()
        .iter(app.world())
        .filter_map(|boards| match boards.0.get(&key) {
            Some(SystemBlackboard::Operations(bb)) => bb
                .active
                .as_ref()
                .map(|active| (active.verb.clone(), active.rate_percent)),
            _ => None,
        })
        .collect();
    published.sort();
    assert_eq!(
        published,
        vec![
            ("escort".to_string(), 100),
            ("field_repair".to_string(), 50),
            ("field_repair".to_string(), 100),
            ("tow".to_string(), 100),
            ("transfer".to_string(), 100),
        ],
        "four verbs across five ships reach the console, and the one in the storm reports the \
         rate the band is holding it to. A bar that crawled with no number beside it would read \
         as a bug rather than as the storm."
    );
}

// ── Issue #1035: the strike gates a depot and un-assists a repair ────────────

const T_STRUCK: &str = "world.probe_strike.entity.tender_struck.name";
const D_STRUCK: &str = "world.probe_strike.entity.depot_struck.name";
const T_WORKING: &str = "world.probe_strike.entity.tender_working.name";
const D_WORKING: &str = "world.probe_strike.entity.depot_working.name";
const S_STRUCK: &str = "world.probe_strike.entity.sister_struck.name";
const S_CLEAR: &str = "world.probe_strike.entity.sister_clear.name";
const RIG_STRUCK: &str = "world.probe_strike.entity.rig_struck.name";
const RIG_CLEAR: &str = "world.probe_strike.entity.rig_clear.name";

fn strike_args(dt: f64, seconds: f64) -> HeadlessArgs {
    HeadlessArgs {
        world_path: "assets/worlds/probe_strike.toml".into(),
        dt,
        max_ticks: ticks_for_sim_seconds(seconds, dt),
        deterministic: true,
        seed: Some(42),
        ..test_args()
    }
}

/// The operations blackboard the console would render for the named ship, as
/// `(state, reason)` — the exact pair `ph-operation-panel.js` resolves through
/// `t()`.
fn operations_readout(app: &mut bevy::prelude::App, name: &str) -> (String, Option<String>) {
    use project_phoenix::messages::SystemBlackboard;
    let key = project_phoenix::operations::operations_blackboard_key();
    app.world_mut()
        .query::<(
            &project_phoenix::entities::spawner::EntityName,
            &project_phoenix::server_app::ShipSystemBlackboards,
        )>()
        .iter(app.world())
        .find(|(entity_name, _)| entity_name.0 == name)
        .and_then(|(_, boards)| match boards.0.get(&key) {
            Some(SystemBlackboard::Operations(bb)) => bb
                .active
                .as_ref()
                .map(|active| (active.state.clone(), active.reason.clone())),
            _ => None,
        })
        .unwrap_or_else(|| panic!("{name} publishes no operations blackboard"))
}

/// **Issue #1035.** `probe_strike.toml` driven for thirty mission seconds: the
/// strike refuses a transfer at the depot its people staff, un-assists a repair
/// on the rig they work, and lets go of both when it is settled — each claim
/// measured against a control that differs in exactly one authored word.
///
/// Every number the run is asserted against comes off a comparison rather than
/// off arithmetic in this file. "The strike slowed the repair" is `struck <
/// clear` at the same instant on two rigs that spawned identical; "the strike
/// refused the transfer" is one depot empty and its twin filled by the same
/// cargo on the same tick.
#[test]
fn a_strike_refuses_a_depot_transfer_and_unassists_a_repair_until_it_is_settled() {
    use project_phoenix::operations::{HoldState, Ineligibility, OperationVerb};
    use project_phoenix::world::server::WorldContentRuntime;

    let dt = 1.0 / 60.0;
    let mut app = build_headless_app(&strike_args(dt, 30.0)).expect("app should build");

    // ── The world opens with the dispute already under way (AC1) ────────────
    //
    // Sampled on the mission's opening ticks — after the register is armed and
    // well before the t=1 s handler opens anything, so what is being read is
    // the world's authored state and not the consequence of a script.
    run(&mut app, 20);
    {
        let register = &app.world().resource::<WorldContentRuntime>().workforce;
        assert!(
            register.on_strike("probe_workers"),
            "the strike is AUTHORED, not scripted on tick one — it was happening before \
             anybody arrived"
        );
        assert!(!register.on_strike("probe_operator"));
        assert_eq!(register.disposition("probe_workers"), Some(20));
        assert_eq!(
            register.disposition("probe_operator"),
            Some(60),
            "each side carries its own opinion of the crew, and they are different"
        );
        let flags = &app.world().resource::<WorldContentRuntime>().flags;
        assert!(
            flags.flag("workforce.probe_workers.on_strike"),
            "…mirrored into an ordinary world flag, which is how a script condition reads \
             it and how a later slice chains a trigger off the settlement"
        );
        assert!(!flags.flag("workforce.probe_operator.on_strike"));
        assert_eq!(flags.counter("workforce.probe_workers.disposition"), 20);
    }

    // ── AC2: the transfer is REFUSED, in words, and its twin is not ─────────
    run(&mut app, 80); // t ≈ 1.7 s — both transfers opened at t=1.
    assert_eq!(
        hold_of(&mut app, T_STRUCK).state(),
        HoldState::Failed(Ineligibility::WorkStopped),
        "the delivery came back refused rather than stalling, timing out, or quietly \
         doing nothing"
    );
    assert_eq!(
        operations_readout(&mut app, T_STRUCK),
        (
            "failed".to_string(),
            Some("operation.refused.work_stopped".to_string())
        ),
        "and the console has the REASON, as a strings.csv id — the crew can tell why they \
         are blocked without reading the world file"
    );
    assert_eq!(
        hold_of(&mut app, T_WORKING).state(),
        HoldState::Holding,
        "the control: same hull, same verb, same cargo, same interrupt rule, a depot whose \
         people are at work — so the refusal is about the STRIKE and not about transfers"
    );

    // ── AC3: the repair is measurably degraded, against a live control ──────
    let struck = hold_of(&mut app, S_STRUCK);
    let clear = hold_of(&mut app, S_CLEAR);
    assert_eq!(struck.verb(), OperationVerb::FieldRepair);
    assert_eq!(
        struck.state(),
        HoldState::Holding,
        "still working — the ship's own team can \
         climb the spine; they are simply on their own"
    );
    assert_eq!(
        (struck.rate().as_percent(), clear.rate().as_percent()),
        (40, 100),
        "at the rate the CAPABILITY authored, not one this slice invented at the call site"
    );
    assert!(
        struck.elapsed_ticks() * 2 < clear.elapsed_ticks(),
        "…so it has banked well under half the control's time: {} against {}",
        struck.elapsed_ticks(),
        clear.elapsed_ticks()
    );
    assert!(
        condition_of(&mut app, RIG_STRUCK) < condition_of(&mut app, RIG_CLEAR),
        "and the WORK is degraded, not just the bar: the payout is scaled by the same rate, \
         so the two cannot come apart"
    );

    // The working transfer lands. Its depot fills; the struck one does not.
    run(&mut app, 400); // t ≈ 8.3 s
    assert_eq!(hold_of(&mut app, T_WORKING).state(), HoldState::Completed);
    assert_eq!(
        (
            capacity_of(&mut app, D_WORKING, "working_depot_load"),
            capacity_of(&mut app, D_STRUCK, "struck_depot_load"),
        ),
        (20, 0),
        "the goods moved at the depot whose people were there, and nowhere else"
    );

    // ── AC4: the settlement, through the flag lever a dialogue will pull ────
    run(&mut app, 180); // t ≈ 11.3 s — the t=10 s handler has set the flag.
    {
        let runtime = app.world().resource::<WorldContentRuntime>();
        assert!(
            !runtime.workforce.on_strike("probe_workers"),
            "the settlement went through `ctx.effects.settle_strike`, chained off an \
             ordinary `on_flag_set` — the same seam #1036's negotiation pulls"
        );
        assert_eq!(
            runtime.workforce.disposition("probe_workers"),
            Some(70),
            "and the other half of a side's state moved with it"
        );
        assert_eq!(runtime.flags.counter("strike_settled_handled"), 1);
        assert!(
            !runtime.flags.flag("workforce.probe_workers.on_strike"),
            "the mirror followed the register rather than drifting from it"
        );
        assert_eq!(
            runtime.flags.counter("workforce.probe_workers.disposition"),
            70
        );
    }
    assert_eq!(
        hold_of(&mut app, S_STRUCK).rate(),
        project_phoenix::operations::ProgressRate::FULL,
        "the assisted rate is back on the next tick, with no restoration path anywhere: \
         the rule simply stopped firing"
    );

    // ── AC4, the other bite: the same delivery, to the same depot ───────────
    run(&mut app, 500); // t ≈ 19.6 s — re-opened at t=12, six seconds to run.
    assert_eq!(
        hold_of(&mut app, T_STRUCK).state(),
        HoldState::Completed,
        "the depot that refused a delivery eight seconds ago has taken one"
    );
    assert_eq!(
        capacity_of(&mut app, D_STRUCK, "struck_depot_load"),
        20,
        "and the cargo is really in it — reversibility is the goods moving, not a flag"
    );
}

// ── The dossier projection (issue #1030, parent #851) ───────────────────────

/// `probe_dossier.toml`, driven for four mission seconds: four subjects on the
/// crew's intelligence channel, and — sitting in the same world, on a live
/// track, at 31 of 100 points — the one condition the scenario is keeping back.
///
/// The load-bearing assertion is the negative one, and it is made against the
/// published payload rather than against the projection's own return value:
/// this is the whole path a console would read, and the withheld number is on
/// no fact of any subject anywhere in it.
///
/// The probe world's own header carries the roster the subjects are asserted
/// against and the two gates it authors.
#[test]
fn the_dossier_channel_carries_what_the_crew_know_and_not_what_they_do_not() {
    use project_phoenix::dossier::{
        dossier_blackboard_key, FACT_COMMITMENT_OPEN, FACT_COMMS, FACT_CONDITION, FACT_FACTION,
    };
    use project_phoenix::messages::{DossierBlackboard, DossierValue, SystemBlackboard};
    use project_phoenix::server_app::ShipSystemBlackboards;

    let dt = 1.0 / 60.0;
    let args = HeadlessArgs {
        world_path: "assets/worlds/probe_dossier.toml".into(),
        ship_path: "assets/entities/alliance_destroyer.toml".into(),
        dt,
        max_ticks: ticks_for_sim_seconds(4.0, dt),
        seed: Some(1030),
        deterministic: true,
        ..test_args()
    };
    let mut app = build_headless_app(&args).expect("app should build");
    run(&mut app, args.max_ticks);

    // Read off the local ship's own blackboard rather than off the wire:
    // `broadcast_blackboard_updates` is diffed, so a picture that stopped
    // changing is deliberately not re-sent.
    let mut q = app
        .world_mut()
        .query_filtered::<&ShipSystemBlackboards, With<LocalShip>>();
    let blackboards = q
        .iter(app.world())
        .next()
        .expect("the local ship publishes");
    let bb: DossierBlackboard = match blackboards.0.get(&dossier_blackboard_key()) {
        Some(SystemBlackboard::Dossiers(bb)) => bb.clone(),
        other => panic!("expected a dossier blackboard, got {other:?}"),
    };

    let by_name = |id: &str| {
        bb.subjects
            .iter()
            .find(|d| d.name == id)
            .unwrap_or_else(|| panic!("{id} is a subject; the roster was {:?}", names(&bb)))
            .clone()
    };
    fn names(bb: &DossierBlackboard) -> Vec<&str> {
        bb.subjects.iter().map(|d| d.name.as_str()).collect()
    }
    fn labels(d: &project_phoenix::messages::DossierSnapshot) -> Vec<&str> {
        d.facts.iter().map(|f| f.label.as_str()).collect()
    }

    // THE ROSTER. Four subjects through two doors, and the marker beacon —
    // hailable by nobody, publishing nothing — is not one of them.
    let mut roster = names(&bb);
    roster.sort_unstable();
    assert_eq!(
        roster,
        vec![
            "world.probe_dossier.entity.sealed_depot.name",
            "world.probe_dossier.entity.skyway_control.name",
            "world.probe_dossier.entity.skyway_depot.name",
            "world.probe_dossier.entity.strike_committee.name",
        ],
        "a dossier exists for what the crew can already observe, and for nothing else"
    );
    assert!(
        bb.subjects.windows(2).all(|w| w[0].uuid <= w[1].uuid),
        "and the list is UUID-ordered, never archetype-ordered"
    );

    // THE PUBLISHED STRUCTURE. On the roster through the infrastructure door
    // alone, so it carries no comms row — which is what proves the two doors
    // are independent rather than one door with two names.
    let depot = by_name("world.probe_dossier.entity.skyway_depot.name");
    assert_eq!(
        labels(&depot),
        vec![
            FACT_CONDITION,
            "world.probe_dossier.threshold.transfer_capable.label",
            "world.probe_dossier.capacity.berths.label",
        ],
        "condition, the labelled flag and the labelled capacity — and the UNLABELLED \
         capacity is published data that is not published prose"
    );
    assert!(
        !labels(&depot).contains(&"depot_transfer_throughput"),
        "a machine id in the author's namespace never becomes a row label"
    );
    assert_eq!(
        depot.summary, "entity.depot_transfer.target.description",
        "the sheet's one line is the entity's own authored description, which the \
         target info panel already shows"
    );
    assert_eq!(depot.facts[2].value, DossierValue::Count(4));

    // THE ONE BEING KEPT BACK. Still a subject — the crew can hail it — and its
    // real, live, degraded condition is on no fact at all.
    let sealed = by_name("world.probe_dossier.entity.sealed_depot.name");
    assert_eq!(
        labels(&sealed),
        vec![FACT_COMMS],
        "hailable, and that is the whole file"
    );
    for subject in &bb.subjects {
        for fact in &subject.facts {
            assert!(
                !matches!(fact.value, DossierValue::Fraction(f) if (f - 0.31).abs() < 1e-3),
                "the withheld 31/100 reached {} as {:?}",
                subject.name,
                fact.label
            );
        }
    }

    // AFFILIATION resolves through the faction's own authored label, so no
    // English crosses the wire even for a faction's name.
    let control = by_name("world.probe_dossier.entity.skyway_control.name");
    assert_eq!(control.facts[0].label, FACT_FACTION);
    assert_eq!(
        control.facts[0].value,
        DossierValue::Text("faction.federation.display_name".into())
    );

    // THE PROMISE, on the sheet of the party it was made to and nobody else's.
    let committee = by_name("world.probe_dossier.entity.strike_committee.name");
    assert_eq!(
        labels(&committee),
        vec![FACT_FACTION, FACT_COMMS, FACT_COMMITMENT_OPEN]
    );
    assert_eq!(
        committee.facts[2].value,
        DossierValue::Text("world.probe_dossier.commitment.safe_passage.terms".into()),
        "the row carries the terms the crew gave, by string id, under the label for \
         the state the promise is in"
    );
    for subject in &bb.subjects {
        if subject.name != "world.probe_dossier.entity.strike_committee.name" {
            assert!(
                !labels(subject).contains(&FACT_COMMITMENT_OPEN),
                "{} was promised nothing and its sheet says so",
                subject.name
            );
        }
    }

    // #1031's seam: carried on every subject, written by nothing in this slice.
    assert!(bb.subjects.iter().all(|d| d.evidence.is_empty()));
}

// ── Falling Skyway: the skeleton world and Act 1 (issue #1034, parent #852) ──

/// The four civilian craft `falling_skyway.toml` puts on lanes at mission start.
const SKYWAY_TRAFFIC: [&str; 4] = [
    "world.falling_skyway.entity.convoy_meridian.name",
    "world.falling_skyway.entity.hauler_lark.name",
    "world.falling_skyway.entity.hauler_pell.name",
    "world.falling_skyway.entity.shuttle_wick.name",
];

/// The status of the objective with this id, or `None` while the world has yet
/// to post it. The panicking [`objective_status`] is the right read once a run
/// is over; a per-tick sample has to survive the ticks before `on_world_loaded`
/// has been through the trigger pipeline.
fn objective_status_opt(
    app: &bevy::prelude::App,
    id: &str,
) -> Option<project_phoenix::core::messages::ObjectiveStatus> {
    app.world()
        .resource::<project_phoenix::world::server::ObjectiveManagerRes>()
        .0
        .sorted_snapshots()
        .into_iter()
        .find(|o| o.id == id)
        .map(|o| o.status)
}

/// Every named ship's `(x, z)` this tick, keyed by its authored `EntityName`.
fn ship_positions(app: &mut bevy::prelude::App) -> std::collections::BTreeMap<String, (f32, f32)> {
    app.world_mut()
        .query::<(
            &project_phoenix::entities::spawner::EntityName,
            &project_phoenix::ship::state::ShipPhysics,
        )>()
        .iter(app.world())
        .map(|(name, physics)| (name.0.clone(), (physics.x, physics.z)))
        .collect()
}

/// **Issue #1034.** The Falling Skyway skeleton, driven past the end of Act 1:
/// the world loads, its traffic is already flying, its clock is on the captain's
/// panel, its three objectives resolve, and the run reaches an Act-1-complete
/// state.
///
/// Every reading is taken tick by tick and asserted on ORDER and STATE, never on
/// frame arithmetic. The mission clock anchors on the first `InProgress` tick
/// rather than at frame zero, so an assertion pinned to an absolute frame is
/// really an assertion about how long the lobby took — while the causal order is
/// what the scenario actually claims. The one number this test does read is the
/// world's own authored `due_secs`, asserted to fall inside the run, so
/// lengthening Act 1 in the TOML (the #1044 tuning pass) does not silently turn
/// this into a test of an act that never finished.
#[test]
fn falling_skyway_runs_traffic_a_countdown_and_three_objectives_to_act_1_complete() {
    use project_phoenix::civilian::CivilianTraffic;
    use project_phoenix::core::messages::ObjectiveStatus;
    use project_phoenix::entities::spawner::EntityName;
    use project_phoenix::infrastructure::InfrastructureCondition;
    use project_phoenix::world::config::WorldConfig;
    use project_phoenix::world::deadlines::DeadlineState;
    use project_phoenix::world::server::WorldContentRuntime;

    const SKYHOOK: &str = "world.falling_skyway.entity.skyhook.name";

    let dt = 1.0 / 30.0;
    let sim_seconds = 100.0;
    let args = HeadlessArgs {
        world_path: "assets/worlds/falling_skyway.toml".into(),
        // The mission's authored hull, and the one its six stations are the
        // small-crew set of.
        ship_path: "assets/entities/alliance_destroyer.toml".into(),
        dt,
        max_ticks: ticks_for_sim_seconds(sim_seconds, dt),
        deterministic: true,
        seed: Some(42),
        ..test_args()
    };
    let mut app = build_headless_app(&args).expect("the scenario world must load and build");

    // AC1 precondition: the act's own clock fits inside this run. Read from the
    // authored config rather than restated here.
    let authored_due: i64 = app
        .world()
        .resource::<WorldConfig>()
        .deadlines
        .iter()
        .find(|d| d.id == "skyway_survey_due")
        .expect("the world authors the act-boundary deadline")
        .due_secs;
    assert!(
        (authored_due as f64) < sim_seconds - 5.0,
        "Act 1 closes at t={authored_due} s, which this {sim_seconds} s run does not \
         cover — retune the run, not the assertions below"
    );

    // `first[…]` is the sim-second at which each reading first went true, so
    // every ordering claim below is read off one pass rather than sampled at a
    // hopeful moment.
    let mut first: std::collections::BTreeMap<&str, f64> = std::collections::BTreeMap::new();
    // Where each craft was on the first tick it existed, and whether it has
    // since moved. "Traffic is moving at mission start" is a claim about the
    // opening of the run, and it is only checkable while the run is opening.
    let mut opening: std::collections::BTreeMap<String, (f32, f32)> = Default::default();
    let mut moved_by: std::collections::BTreeMap<String, f64> = Default::default();
    let mut lift_capable_seen = false;
    let mut skyhook_condition_at_close: Option<f32> = None;

    for tick in 0..args.max_ticks {
        run(&mut app, 1);
        let sim_t = (tick + 1) as f64 * dt;

        for (name, position) in ship_positions(&mut app) {
            let start = *opening.entry(name.clone()).or_insert(position);
            let travelled =
                ((position.0 - start.0).powi(2) + (position.1 - start.1).powi(2)).sqrt();
            if travelled > 40.0 {
                moved_by.entry(name).or_insert(sim_t);
            }
        }

        let runtime = app.world().resource::<WorldContentRuntime>();
        let flags = &runtime.flags;
        // The head is certified to lift on arrival; only once that has genuinely
        // been up does its fall mean anything (the same not-published-yet guard
        // the #1025 probe carries).
        if flags.flag("skyhook_lift_capable") {
            lift_capable_seen = true;
            first.entry("lift_capable").or_insert(sim_t);
        } else if lift_capable_seen {
            first.entry("lift_lost").or_insert(sim_t);
        }
        for (flag, key) in [
            ("tether_slipped", "tether_slipped"),
            ("a1_lane_open", "lane_open"),
            ("a1_priority_set", "priority_set"),
            ("act1_complete", "act1_complete"),
        ] {
            if flags.counter(flag) > 0 {
                first.entry(key).or_insert(sim_t);
            }
        }
        if objective_status_opt(&app, "obj-a1-corridor") == Some(ObjectiveStatus::Completed) {
            first.entry("corridor_objective").or_insert(sim_t);
        }
        if objective_status_opt(&app, "obj-a1-triage") == Some(ObjectiveStatus::Completed) {
            first.entry("triage_objective").or_insert(sim_t);
        }
        if objective_status_opt(&app, "obj-a1-survey") == Some(ObjectiveStatus::Completed) {
            first.entry("survey_objective").or_insert(sim_t);
            if skyhook_condition_at_close.is_none() {
                skyhook_condition_at_close = app
                    .world_mut()
                    .query::<(&EntityName, &InfrastructureCondition)>()
                    .iter(app.world())
                    .find(|(name, _)| name.0 == SKYHOOK)
                    .map(|(_, condition)| condition.0.condition());
            }
        }
    }

    let at = |key: &str| -> f64 {
        *first
            .get(key)
            .unwrap_or_else(|| panic!("'{key}' never happened in this run: {first:?}"))
    };

    // ── AC3: the corridor was already working when the crew arrived ──
    // Present from the opening tick, on authored lanes, and under way — not
    // spawned by a trigger and not waiting to be told.
    let traffic: Vec<String> = app
        .world_mut()
        .query::<(&EntityName, &CivilianTraffic)>()
        .iter(app.world())
        .filter(|(_, traffic)| traffic.0.route().is_some())
        .map(|(name, _)| name.0.clone())
        .collect();
    let mut on_lanes = traffic.clone();
    on_lanes.sort();
    assert_eq!(
        on_lanes,
        SKYWAY_TRAFFIC.map(String::from).to_vec(),
        "four civilian craft fly this world's authored lanes"
    );
    for name in SKYWAY_TRAFFIC {
        assert!(
            opening.contains_key(name),
            "{name} must exist from the opening tick — traffic the mission spawns \
             later is traffic the crew watched arrive"
        );
        let under_way = *moved_by.get(name).unwrap_or_else(|| {
            panic!(
                "{name} never left its spawn point; the lane is authored but nobody is flying it"
            )
        });
        assert!(
            under_way < at("lane_open"),
            "{name} must already be under way before anything the crew does resolves \
             ({under_way:.1} s vs the corridor objective at {:.1} s)",
            at("lane_open")
        );
    }

    // ── AC4: the clock is on the captain's panel ──
    // Both authored deadlines are visible, and both reach the captain blackboard
    // with a server-computed countdown. (The panel that renders them is covered
    // by tests/client/console-state.test.js.)
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
        .expect("the crew's own ship publishes the mission's visible deadlines");
    // Every deadline the world authors `visible`, in authored order — read from
    // the config rather than restated here, so a later act adding its own clock
    // (the storm sweep, #1037) extends the panel without rewriting this
    // assertion into a list of ids nobody maintains.
    let authored_visible: Vec<String> = app
        .world()
        .resource::<WorldConfig>()
        .deadlines
        .iter()
        .filter(|d| d.visible)
        .map(|d| d.id.clone())
        .collect();
    assert_eq!(
        published.iter().map(|d| d.id.clone()).collect::<Vec<_>>(),
        authored_visible,
        "the captain's countdown carries exactly the deadlines the world authored \
         visible, in authored order"
    );
    assert_eq!(
        published[..2]
            .iter()
            .map(|d| d.id.as_str())
            .collect::<Vec<_>>(),
        vec!["tether_slip", "skyway_survey_due"],
        "…and Act 1's two lead it"
    );
    assert_eq!(
        published[1].label, "world.falling_skyway.deadline.skyway_survey_due.label",
        "the crew-facing label is a strings.csv id, never English"
    );

    // ── AC5, the causal chain: a deadline moves a condition track, the track
    // crosses an authored threshold, and the threshold resolves an objective ──
    let slipped = at("tether_slipped");
    let lost = at("lift_lost");
    assert!(
        at("lift_capable") < slipped,
        "precondition: the head arrives CERTIFIED, so losing that is an event \
         rather than the opening state ({:.1} s vs {slipped:.1} s)",
        at("lift_capable")
    );
    assert!(
        lost >= slipped && lost - slipped < 1.0,
        "the six points the tether slip spends must take the head under its own \
         45 % lift line promptly — slip at {slipped:.1} s, line crossed at {lost:.1} s"
    );
    assert!(
        at("priority_set") >= lost && at("priority_set") - lost < 0.5,
        "…and the chained `on_flag_cleared` handler rides the same one-tick \
         pending_world_events bridge, rather than an open-ended delay"
    );
    assert_eq!(
        at("triage_objective"),
        at("priority_set"),
        "the triage objective resolves in the handler that recorded the priority"
    );

    // ── AC5, the other resolution: the lane proves itself ──
    assert_eq!(
        at("corridor_objective"),
        at("lane_open"),
        "the corridor objective resolves when the lead hauler clears the gate — \
         #1028's route machinery, not a timer"
    );

    // ── AC1/AC5: the act closes, deterministically, on its own deadline ──
    let closed = at("act1_complete");
    assert_eq!(
        at("survey_objective"),
        closed,
        "the survey report and the act boundary are the same beat"
    );
    for earlier in ["corridor_objective", "triage_objective"] {
        assert!(
            at(earlier) < closed,
            "'{earlier}' resolved at {:.1} s, at or after the act closed at \
             {closed:.1} s — Act 1 must end with its work already accounted for",
            at(earlier)
        );
    }
    let runtime = app.world().resource::<WorldContentRuntime>();
    assert_eq!(
        runtime.flags.counter("act1_complete"),
        1,
        "the Act-1-complete state is reached exactly once"
    );
    assert_eq!(
        runtime.flags.counter("act"),
        2,
        "…and the act counter the later slices hang their content on has advanced"
    );
    assert_eq!(
        runtime.deadlines.get("skyway_survey_due").map(|d| d.state),
        Some(DeadlineState::Fired),
        "the act boundary is the authored deadline firing, not a script counting frames"
    );
    for id in ["obj-a1-survey", "obj-a1-corridor", "obj-a1-triage"] {
        assert_eq!(
            objective_status(&app, id),
            ObjectiveStatus::Completed,
            "every Act 1 objective resolves in a clean run; a partial one would read \
             Failed here rather than staying Pending"
        );
    }

    // ── AC2: the structures the act is about, as authored data ──
    // The skyhook is six points down on where it started and still standing:
    // this act damages the head, it does not lose it (#1040 owns the collapse).
    assert_eq!(
        skyhook_condition_at_close,
        Some(42.0),
        "48 authored - 6 spent by the tether slip, in condition points"
    );
    let mut structures = app
        .world_mut()
        .query::<(&EntityName, &InfrastructureCondition)>();
    let tracks: std::collections::BTreeMap<String, (f32, Vec<String>)> = structures
        .iter(app.world())
        .map(|(name, condition)| {
            (
                name.0.clone(),
                (
                    condition.0.condition(),
                    condition
                        .0
                        .capacities()
                        .iter()
                        .map(|c| c.id.clone())
                        .collect(),
                ),
            )
        })
        .collect();
    assert_eq!(
        tracks
            .get("world.falling_skyway.entity.depot_ladder_a.name")
            .map(|(condition, _)| *condition),
        Some(62.0),
        "the working rung of the ladder is authored, published and untouched by Act 1"
    );
    assert_eq!(
        tracks
            .get("world.falling_skyway.entity.depot_ladder_b.name")
            .map(|(condition, _)| *condition),
        Some(34.0)
    );
    assert_eq!(
        tracks
            .get(SKYHOOK)
            .map(|(_, capacities)| capacities.clone())
            .unwrap_or_default(),
        vec![
            "skyhook_transfer_berths".to_string(),
            "skyhook_climber_load".to_string()
        ],
        "the head carries its authored capacities, berths first"
    );
    let flags = &app.world().resource::<WorldContentRuntime>().flags;
    assert_eq!(
        flags.counter("skyhook_transfer_berths"),
        2,
        "TWO berths, and this world fields three parties who each need one — the \
         scenario's central pressure is authored data a script predicate can read"
    );
    assert!(
        flags.flag("depot_a_pumping"),
        "the depot chain's two rungs own SEPARATE flags: A is above its line…"
    );
    assert!(
        !flags.flag("depot_b_pumping"),
        "…and B is authored below its own, so the triage picture is on the panel \
         before anybody says a word about it"
    );
}

// ── Evidence entries with provenance (issue #1031, parent #851) ──────────────

/// `probe_evidence.toml`, driven for eight mission seconds: the same subject
/// learned about twice, by two genuinely different routes, and both entries on
/// one fact sheet in the order the crew got them.
///
/// This is the whole slice end to end. The scan half comes from a `[[deadline]]`
/// handler; the testimony half comes from a **real dialogue `on_pick`**, run
/// because the ship's own AI-backfilled Comms officer answered an open thread
/// through the ordinary admitted `RespondToMessage` path — not from a timer
/// standing in for a player's finger. Every assertion is made against the
/// PUBLISHED blackboard, which is the payload a console would render.
///
/// The two no-ops are asserted here rather than in a unit test because both are
/// properties of the live applier: a duplicate append leaves one line stamped at
/// the first tick, and an append to a name no entity answers to is dropped with
/// a warning while the rest of the handler still runs.
///
/// The probe world's own header carries the authored timeline.
#[test]
fn a_scan_and_a_dialogue_admission_both_land_on_one_fact_sheet_with_their_provenance() {
    use project_phoenix::dossier::{dossier_blackboard_key, FACT_CONDITION};
    use project_phoenix::messages::{DossierBlackboard, SystemBlackboard};
    use project_phoenix::server_app::ShipSystemBlackboards;
    use project_phoenix::world::server::WorldContentRuntime;

    let dt = 1.0 / 60.0;
    let args = HeadlessArgs {
        world_path: "assets/worlds/probe_evidence.toml".into(),
        ship_path: "assets/entities/alliance_destroyer.toml".into(),
        dt,
        max_ticks: ticks_for_sim_seconds(8.0, dt),
        seed: Some(1031),
        deterministic: true,
        ..test_args()
    };
    let mut app = build_headless_app(&args).expect("app should build");
    run(&mut app, args.max_ticks);

    // Both routes ran. The survey handler's flag is the one that proves the two
    // no-ops inside it cost it nothing: a raise would have discarded the call
    // and this counter with it.
    {
        let flags = &app.world().resource::<WorldContentRuntime>().flags;
        assert_eq!(
            flags.counter("survey_ran"),
            1,
            "the survey handler ran to completion — a duplicate append and an \
             unknown subject are no-ops, not raises"
        );
        assert_eq!(
            flags.counter("foreman_pressed"),
            1,
            "and the Comms officer answered the thread with response 0, so the \
             on_pick ran"
        );
        assert_eq!(flags.counter("foreman_let_go"), 0);
    }

    // The store: three appends, two entries, one subject.
    {
        let log = &app.world().resource::<WorldContentRuntime>().evidence;
        assert_eq!(
            log.entries.len(),
            2,
            "three appends, one of them a duplicate and one of them addressed to \
             nothing: {:?}",
            log.entries
        );
        let scan = &log.entries[0];
        let testimony = &log.entries[1];
        assert_eq!(scan.text, "world.probe_evidence.evidence.stress_fracture");
        assert_eq!(
            testimony.text,
            "world.probe_evidence.evidence.foreman_admission"
        );
        assert_eq!(
            scan.subject_uuid, testimony.subject_uuid,
            "both are about the hook — a finding is filed under what it is ABOUT, \
             not under who said it"
        );
        assert!(
            scan.gathered_at_tick < testimony.gathered_at_tick,
            "the survey came back before the foreman talked ({} vs {})",
            scan.gathered_at_tick,
            testimony.gathered_at_tick
        );
    }

    // The payload a console reads.
    let mut q = app
        .world_mut()
        .query_filtered::<&ShipSystemBlackboards, With<LocalShip>>();
    let blackboards = q
        .iter(app.world())
        .next()
        .expect("the local ship publishes");
    let bb: DossierBlackboard = match blackboards.0.get(&dossier_blackboard_key()) {
        Some(SystemBlackboard::Dossiers(bb)) => bb.clone(),
        other => panic!("expected a dossier blackboard, got {other:?}"),
    };

    let hook = bb
        .subjects
        .iter()
        .find(|d| d.name == "world.probe_evidence.entity.skyway_hook.name")
        .expect("the hook is a subject through the infrastructure door");

    // AC1/AC4/AC6: both entries, in gather order, each carrying its own
    // provenance and the tick it was learned on.
    assert_eq!(
        hook.evidence
            .iter()
            .map(|e| (e.text.as_str(), e.provenance.as_str()))
            .collect::<Vec<_>>(),
        vec![
            ("world.probe_evidence.evidence.stress_fracture", "scan"),
            (
                "world.probe_evidence.evidence.foreman_admission",
                "dialogue"
            ),
        ],
        "the crew can say how they know, one entry at a time"
    );
    assert!(
        hook.evidence.iter().all(|e| e.gathered_at_tick > 0),
        "each is stamped with the tick the handler that wrote it ran on"
    );

    // The separation the panel renders: what they were handed is still in
    // `facts`, what they went and got is in `evidence`, and neither list leaked
    // into the other.
    assert_eq!(
        hook.facts
            .iter()
            .map(|f| f.label.as_str())
            .collect::<Vec<_>>(),
        vec![FACT_CONDITION, "world.probe_evidence.capacity.berths.label"],
        "the baseline facts are exactly what the world authored"
    );
    assert!(
        !hook
            .facts
            .iter()
            .any(|f| f.label.starts_with("world.probe_evidence.evidence.")),
        "a finding never becomes a fact row"
    );

    // Nobody else's file grew an entry — including the foreman, whose testimony
    // this was.
    for subject in &bb.subjects {
        if subject.name != "world.probe_evidence.entity.skyway_hook.name" {
            assert!(
                subject.evidence.is_empty(),
                "{} was not what any of this was about",
                subject.name
            );
        }
    }
}

/// **Issue #1035, in the scenario rather than in a probe.** The strike the
/// skeleton world left as prose is authored state, and the crew's own hull
/// meets both of its bites: a transfer stood up at Ladder Depot B comes back
/// refused in words, and a field-repair on the head runs at the unassisted rate
/// while the same repair on the depot the strike does not touch runs at full.
///
/// The ship is moved by hand to each structure in turn. That is the honest
/// analogue of helm flying it there — the two operations are authored 500 and
/// 400 units of range, the mission opens the crew a kilometre off both, and
/// closing is helm's job rather than this test's subject.
#[test]
fn the_skyway_strike_refuses_depot_b_and_leaves_the_head_repair_unassisted() {
    use project_phoenix::entities::spawner::EntityName;
    use project_phoenix::infrastructure::InfrastructureCondition;
    use project_phoenix::operations::{
        HoldState, Ineligibility, OperationVerb, PendingOperationStart, ProgressRate,
        ShipOperations,
    };
    use project_phoenix::ship::state::ShipPhysics;
    use project_phoenix::world::server::WorldContentRuntime;

    const SKYHOOK: &str = "world.falling_skyway.entity.skyhook.name";
    const DEPOT_B: &str = "world.falling_skyway.entity.depot_ladder_b.name";
    const DEPOT_A: &str = "world.falling_skyway.entity.depot_ladder_a.name";

    let dt = 1.0 / 30.0;
    let args = HeadlessArgs {
        world_path: "assets/worlds/falling_skyway.toml".into(),
        ship_path: "assets/entities/alliance_destroyer.toml".into(),
        dt,
        max_ticks: ticks_for_sim_seconds(120.0, dt),
        deterministic: true,
        seed: Some(42),
        ..test_args()
    };
    let mut app = build_headless_app(&args).expect("the scenario world must load and build");
    run(&mut app, 10);

    // ── AC1: two sides, in the world, with the workers already out ──────────
    {
        let register = &app.world().resource::<WorldContentRuntime>().workforce;
        assert_eq!(
            register
                .records
                .iter()
                .map(|r| (r.id.clone(), r.on_strike))
                .collect::<Vec<_>>(),
            vec![
                ("skyway_workers".to_string(), true),
                ("havelock_operations".to_string(), false),
            ],
            "the worker and corporate sides are both in the world from the first tick, \
             and only one of them has walked out"
        );
        assert_eq!(register.disposition("skyway_workers"), Some(25));
        assert_eq!(register.disposition("havelock_operations"), Some(55));
        let flags = &app.world().resource::<WorldContentRuntime>().flags;
        assert!(flags.flag("workforce.skyway_workers.on_strike"));
        assert!(!flags.flag("workforce.havelock_operations.on_strike"));
    }

    // Which structures each side staffs — the other half of the pairing, read
    // off the live condition tracks rather than off the file.
    let staffed: std::collections::BTreeMap<String, Option<String>> = app
        .world_mut()
        .query::<(&EntityName, &InfrastructureCondition)>()
        .iter(app.world())
        .map(|(name, condition)| (name.0.clone(), condition.0.workforce().map(str::to_string)))
        .collect();
    assert_eq!(
        staffed.get(SKYHOOK).cloned().flatten(),
        Some("skyway_workers".to_string())
    );
    assert_eq!(
        staffed.get(DEPOT_B).cloned().flatten(),
        Some("skyway_workers".to_string())
    );
    assert_eq!(
        staffed.get(DEPOT_A).cloned().flatten(),
        Some("havelock_operations".to_string()),
        "the rung that still works is the rung whose crews are still there — which is why \
         A pumps and B does not, and it is authored rather than implied"
    );

    // The crew's hull, and the two verbs it works the skyway with. Found by its
    // capability table because the player row carries no authored name: its
    // config comes from the lobby-selected template wholesale.
    let (ship, ship_uuid) = app
        .world_mut()
        .query::<(
            bevy::prelude::Entity,
            &project_phoenix::entities::spawner::EntityUuid,
            &ShipOperations,
        )>()
        .iter(app.world())
        .map(|(entity, uuid, _)| (entity, uuid.0.clone()))
        .next()
        .expect("the destroyer authors an [operations] table");
    let uuid_of = |app: &bevy::prelude::App, name: &str| {
        app.world()
            .resource::<WorldContentRuntime>()
            .name_to_uuid
            .get(name)
            .cloned()
            .unwrap_or_else(|| panic!("{name} is not in this world"))
    };
    let move_ship = |app: &mut bevy::prelude::App, to: bevy::prelude::Vec3| {
        let mut physics = app
            .world_mut()
            .get_mut::<ShipPhysics>(ship)
            .expect("a ship");
        physics.x = to.x;
        physics.y = to.y;
        physics.z = to.z;
    };
    // The same queue a scripted `ctx.effects.transfer(…)` fills — the applier
    // resolves the names and `tick_operations` decides, which is the whole path
    // a console's `start_operation` reaches by a different door.
    let order = |app: &mut bevy::prelude::App, verb, target: &str| {
        let target_uuid = uuid_of(app, target);
        app.world_mut()
            .resource_mut::<WorldContentRuntime>()
            .pending_operation_starts
            .push(PendingOperationStart {
                ship_uuid: ship_uuid.clone(),
                verb,
                target_uuid,
            });
    };

    // ── AC2: a transfer at Ladder Depot B is refused, with the reason ───────
    move_ship(&mut app, bevy::prelude::Vec3::new(1180.0, 0.0, 300.0));
    order(&mut app, OperationVerb::Transfer, DEPOT_B);
    run(&mut app, 4);
    let refused = app
        .world()
        .get::<ShipOperations>(ship)
        .and_then(|ops| ops.active.clone())
        .expect("the transfer opened");
    assert_eq!(refused.verb(), OperationVerb::Transfer);
    assert_eq!(
        refused.state(),
        HoldState::Failed(Ineligibility::WorkStopped),
        "alongside, in range, powered — and refused, because nobody down there is \
         authorising anything"
    );
    assert_eq!(
        refused.state().reason().map(|r| r.string_id()),
        Some("operation.refused.work_stopped"),
        "the crew are told WHY on the operations panel, in words, without reading the \
         world file"
    );

    // ── AC3: the head repair runs, and runs un-assisted ─────────────────────
    move_ship(&mut app, bevy::prelude::Vec3::new(200.0, 0.0, 0.0));
    order(&mut app, OperationVerb::FieldRepair, SKYHOOK);
    run(&mut app, 4);
    let unassisted = app
        .world()
        .get::<ShipOperations>(ship)
        .and_then(|ops| ops.active.clone())
        .expect("the repair opened");
    assert_eq!(unassisted.verb(), OperationVerb::FieldRepair);
    assert_eq!(
        unassisted.state(),
        HoldState::Holding,
        "the ship's own team \
         can still work the spine — they are simply on their own"
    );
    assert_eq!(
        unassisted.rate().as_percent(),
        35,
        "at the unassisted rate the hull's capability authors. The same job on a structure \
         the strike does not touch runs at {}%, which is the measurement",
        ProgressRate::FULL.as_percent()
    );

    // The control, in the same world on the same tick: Ladder Depot A is worked
    // by people who are not out, so the identical capability runs at full rate.
    move_ship(&mut app, bevy::prelude::Vec3::new(620.0, 0.0, -180.0));
    order(&mut app, OperationVerb::FieldRepair, DEPOT_A);
    run(&mut app, 4);
    let assisted = app
        .world()
        .get::<ShipOperations>(ship)
        .and_then(|ops| ops.active.clone())
        .expect("the control repair opened");
    assert_eq!(assisted.rate(), ProgressRate::FULL);
}

// ── The science scan (issue #1032, parent #851) ──────────────────────────────

/// `probe_scan.toml` — the world's own header carries what each contact is for.
const SCAN_WORLD: &str = "assets/worlds/probe_scan.toml";
/// The published, decaying structure every reading is taken of.
const SCAN_DEPOT: &str = "world.probe_scan.entity.skyway_depot.name";
/// The structure the scenario keeps off the wire, sitting at a real 31/100.
const SCAN_SEALED: &str = "world.probe_scan.entity.sealed_depot.name";
/// A named contact with no condition track of any kind.
const SCAN_BEACON: &str = "world.probe_scan.entity.lonely_beacon.name";

/// The `ai:` probe token every scan below is asked for under.
///
/// Unregistered, so admission routes it to the LocalShip, whose backfilled
/// `sensors` system `operate_ai`s — the same admission path every AI command
/// takes and the same one a human on the captain's console would take
/// (AGENTS.md rule 6). Nothing downstream can tell which sent it, which is the
/// point.
const SCAN_TOKEN: &str = "ai:scan-probe";

fn scan_args(dt: f64, seconds: f64) -> HeadlessArgs {
    HeadlessArgs {
        world_path: SCAN_WORLD.into(),
        ship_path: "assets/entities/alliance_destroyer.toml".into(),
        dt,
        max_ticks: ticks_for_sim_seconds(seconds, dt),
        deterministic: true,
        seed: Some(1032),
        ..test_args()
    }
}

/// The minted UUID of the entity carrying `name`.
fn scan_uuid_named(app: &mut App, name: &str) -> String {
    use project_phoenix::entities::spawner::{EntityName, EntityUuid};
    let mut q = app.world_mut().query::<(&EntityName, &EntityUuid)>();
    let found = q
        .iter(app.world())
        .find(|(n, _)| n.0 == name)
        .map(|(_, uuid)| uuid.0.clone());
    found.unwrap_or_else(|| panic!("the probe world spawns '{name}'"))
}

/// Put the scanning ship at `x` by writing its `ShipPhysics`, which is what helm
/// moves. Writing the `Transform` directly would be undone by
/// `sync_ship_position` on the next tick.
///
/// Range is the crew's own lever on how good a reading they get, so the test
/// pulls it the way the crew would rather than editing an authored band.
fn place_scanner_at(app: &mut App, x: f32) {
    let entity = app
        .world_mut()
        .query_filtered::<Entity, With<LocalShip>>()
        .iter(app.world())
        .next()
        .expect("the probe world spawns a local ship");
    app.world_mut()
        .get_mut::<ShipPhysics>(entity)
        .expect("the local ship is a ship")
        .x = x;
}

/// Ask for a scan through the real admission path, and give it the ticks to
/// arrive: the message is drained per frame in `PreUpdate`, admitted before
/// `SimSet::Input`, and consumed in `SimSet::Modifiers` of the same tick.
fn ask_for_scan(app: &mut App, uuid: &str) {
    use project_phoenix::lobby::InboundMessage;
    use project_phoenix::messages::{ClientMessage, SystemControlPayload};

    app.world_mut()
        .resource_mut::<bevy::ecs::message::Messages<InboundMessage>>()
        .write(InboundMessage {
            token: SCAN_TOKEN.into(),
            msg: ClientMessage::ControlSystem {
                target: project_phoenix::ship::system_registry::sensors_system_id(),
                payload: SystemControlPayload::ScanTarget {
                    uuid: uuid.to_string(),
                },
            },
        });
    run(app, 2);
}

/// The scan channel as the local ship publishes it — the payload a console
/// renders, never the component behind it.
fn published_scan(app: &mut App) -> project_phoenix::messages::ScanBlackboard {
    use project_phoenix::messages::SystemBlackboard;
    use project_phoenix::server_app::ShipSystemBlackboards;

    let mut q = app
        .world_mut()
        .query_filtered::<&ShipSystemBlackboards, With<LocalShip>>();
    let boards = q
        .iter(app.world())
        .next()
        .expect("the local ship publishes");
    match boards
        .0
        .get(&project_phoenix::science::scan_blackboard_key())
    {
        Some(SystemBlackboard::Scan(bb)) => bb.clone(),
        other => panic!("expected a scan blackboard, got {other:?}"),
    }
}

/// **Issue #1032, AC1 and AC2 — the whole point of the slice, end to end.**
///
/// Two scans of the same structure from the same place, a few seconds apart,
/// through the ordinary admitted command path. The depot is failing under its
/// own authored `decay_per_sec` the entire time, and the second reading says so:
/// a lower condition, and an operational flag that has dropped in between.
///
/// **No content is edited between the two readings.** Nothing in this test, this
/// world, the entity template or `strings.csv` describes either result — the
/// numbers are the depot's condition track and the words are the labels its own
/// capacity and threshold authored. That is
/// `pasm/spec/design/simulation-differentiation.yaml`'s "sensors reveal state
/// rather than scenario text", asserted rather than claimed.
#[test]
fn two_scans_of_a_failing_structure_report_it_failing_with_no_content_edited() {
    let dt = 1.0 / 60.0;
    let mut app = build_headless_app(&scan_args(dt, 10.0)).expect("app should build");
    // Far enough in that the game is InProgress and the ship is backfilled.
    run(&mut app, 60);
    let depot = scan_uuid_named(&mut app, SCAN_DEPOT);

    // 80 units off the depot at x = 600: inside the destroyer's authored
    // detailed band, which reads whole percent and counts berths.
    place_scanner_at(&mut app, 520.0);
    ask_for_scan(&mut app, &depot);
    let first = published_scan(&mut app)
        .reading
        .expect("the first scan comes back");

    assert_eq!(first.subject_uuid, depot);
    assert_eq!(first.subject_name, SCAN_DEPOT);
    assert_eq!(first.band, "detailed");
    assert!(
        first.condition_fraction > 0.45 && first.condition_fraction <= 0.62,
        "the depot spawned at 62 points and has been shedding four a second since: {}",
        first.condition_fraction
    );
    assert_eq!(
        first.flags,
        vec![(
            "world.probe_scan.threshold.transfer_capable.label".to_string(),
            true
        )],
        "its transfer arm is still in tolerance, reported under the LABEL the depot \
         authored for the flag rather than under any word this slice invented"
    );
    assert_eq!(
        first.capacities,
        vec![("world.probe_scan.capacity.berths.label".to_string(), 4)],
        "the detailed band counts berths — and only the LABELLED capacity, so the \
         depot's unlabelled throughput stays published data and not published prose"
    );

    // Five more seconds of the storm. Nobody scripts anything; the structure is
    // simply still failing.
    run(&mut app, ticks_for_sim_seconds(5.0, dt));
    place_scanner_at(&mut app, 520.0);
    ask_for_scan(&mut app, &depot);
    let second = published_scan(&mut app)
        .reading
        .expect("the second scan comes back");

    assert_eq!(
        second.band, first.band,
        "same suite, same range, same fidelity"
    );
    assert!(
        second.condition_fraction < first.condition_fraction - 0.1,
        "the reading MOVED with the track it is derived from: {} then {}",
        first.condition_fraction,
        second.condition_fraction
    );
    assert_eq!(
        second.flags,
        vec![(
            "world.probe_scan.threshold.transfer_capable.label".to_string(),
            false
        )],
        "…and the depot dropped through its own 40 % threshold in between, so the \
         same row now reads as failed"
    );
    assert!(
        second.taken_at_tick > first.taken_at_tick,
        "each reading is stamped with the tick it was taken on — it is a reading, \
         not a live gauge"
    );
}

/// **AC5.** The same structure, seconds apart, read from twice the range: a
/// rounder number and a capacity list the coarse band does not claim to know.
///
/// Both bands are the destroyer's own shipped `[scan]` ladder, unmodified by
/// this world — the gate is authored data, and the lever the crew pull is helm.
#[test]
fn the_same_structure_reads_coarser_from_further_out() {
    let dt = 1.0 / 60.0;
    let mut app = build_headless_app(&scan_args(dt, 8.0)).expect("app should build");
    run(&mut app, 60);
    let depot = scan_uuid_named(&mut app, SCAN_DEPOT);

    // 200 units out: past the detailed band's 120 and inside the coarse band's
    // 260.
    place_scanner_at(&mut app, 400.0);
    ask_for_scan(&mut app, &depot);
    let far = published_scan(&mut app).reading.expect("a coarse reading");

    // …and straight back in to 80, on the very next scan.
    place_scanner_at(&mut app, 520.0);
    ask_for_scan(&mut app, &depot);
    let close = published_scan(&mut app)
        .reading
        .expect("a detailed reading");

    assert_eq!(far.band, "coarse");
    assert_eq!(close.band, "detailed");
    assert_eq!(
        far.condition_step, 0.25,
        "the coarse band reports to the nearest quarter, and says so"
    );
    assert_eq!(close.condition_step, 0.01);
    assert_eq!(
        far.condition_fraction % 0.25,
        0.0,
        "the coarse reading lands on a quarter: {}",
        far.condition_fraction
    );
    assert!(
        far.capacities.is_empty(),
        "the coarse band authors report_capacities = false — a sweep that rough \
         does not pretend to count berths"
    );
    assert_eq!(
        close.capacities.len(),
        1,
        "…while closing to 80 units buys the crew the count back"
    );
    assert_eq!(
        far.flags.len(),
        1,
        "both bands still resolve the operational flag, because the ladder says so"
    );
}

/// **AC1's refusal half, and the leak rule.**
///
/// A structure the scenario keeps off the wire and a beacon with no condition
/// track at all are refused with the **same** reason. That identity is the
/// load-bearing part: an error distinguishing "nothing to read" from "something
/// I am not telling you" would betray the secret by its shape, which is the leak
/// #1030 closed and this slice must not reopen.
///
/// The withheld 31 points are then hunted for across the whole published
/// channel, the way `probe_dossier`'s test hunts them across every fact.
#[test]
fn a_withheld_structure_and_a_bare_beacon_are_refused_with_the_same_reason() {
    let dt = 1.0 / 60.0;
    let mut app = build_headless_app(&scan_args(dt, 8.0)).expect("app should build");
    run(&mut app, 60);
    let sealed = scan_uuid_named(&mut app, SCAN_SEALED);
    let beacon = scan_uuid_named(&mut app, SCAN_BEACON);
    let depot = scan_uuid_named(&mut app, SCAN_DEPOT);

    // Close enough that range is not what refuses any of the three.
    place_scanner_at(&mut app, 520.0);
    ask_for_scan(&mut app, &depot);
    assert!(
        published_scan(&mut app).reading.is_some(),
        "precondition: from here the suite reads a published structure fine"
    );

    place_scanner_at(&mut app, 520.0);
    ask_for_scan(&mut app, &sealed);
    let withheld = published_scan(&mut app);
    assert_eq!(
        withheld.refusal.as_deref(),
        Some("scan.refusal.no_readable_condition")
    );
    assert!(
        withheld.reading.is_none(),
        "a refusal replaces the previous reading rather than leaving one on screen \
         beside a complaint about a different contact"
    );

    place_scanner_at(&mut app, 520.0);
    ask_for_scan(&mut app, &beacon);
    let bare = published_scan(&mut app);
    assert_eq!(
        bare.refusal, withheld.refusal,
        "the same answer, exactly — the shape of the refusal must not tell the crew \
         that the sealed depot is keeping something back"
    );

    // The number itself, hunted for across the whole published channel.
    let json = serde_json::to_string(&published_scan(&mut app)).expect("serialises");
    assert!(
        !json.contains("31"),
        "the sealed depot's real 31 of 100 points reached the wire: {json}"
    );
}

/// Past the coarsest authored band the suite returns nothing, and says which
/// gate stopped it — a refusal the crew can act on with helm, rather than an
/// empty readout they have to interpret.
#[test]
fn a_scan_from_past_the_last_band_is_refused_as_out_of_range() {
    let dt = 1.0 / 60.0;
    let mut app = build_headless_app(&scan_args(dt, 8.0)).expect("app should build");
    run(&mut app, 60);
    let depot = scan_uuid_named(&mut app, SCAN_DEPOT);

    // The origin is 600 units off the depot; the destroyer's coarsest band
    // reaches 260.
    place_scanner_at(&mut app, 0.0);
    ask_for_scan(&mut app, &depot);
    let bb = published_scan(&mut app);
    assert_eq!(bb.refusal.as_deref(), Some("scan.refusal.out_of_range"));
    assert!(bb.reading.is_none());
    assert!(
        bb.capable,
        "…and the hull still reports that it HAS a survey suite, so the console says \
         'too far' rather than 'you cannot do this'"
    );
}

// ── Negotiation, and the other way it ends (issue #1036, parent #852) ────────

/// The workers' entity — the party every promise in this act is made to, and
/// the file the casualty finding lands on.
const SKYWAY_COMMITTEE: &str = "world.falling_skyway.entity.strike_committee.name";
/// The operator's security cutter — the other open channel.
const SKYWAY_CUTTER: &str = "world.falling_skyway.entity.havelock_cutter.name";
/// The rung the dispute is about.
const SKYWAY_DEPOT_B: &str = "world.falling_skyway.entity.depot_ladder_b.name";
/// The finding the evidence branch reads. Filed by the operator's own file, and
/// what `ctx.dossier.holds` is asked about.
const SKYWAY_FILE: &str = "world.falling_skyway.evidence.ladder_b_maintenance_file";
/// Unregistered, so a submission routes to the LocalShip through the ordinary
/// admission path — the same door `command_admission_moves_with_the_logical_tick`
/// uses for a helm command.
const SKYWAY_TOKEN: &str = "ai:skyway-comms";
/// Every test in this group drives the scenario at the #1035 test's timestep.
const SKYWAY_DT: f64 = 1.0 / 30.0;

/// Build Falling Skyway and step it to the act-2 boundary, with the destroyer
/// stood off the ladder transit leg where both parties are inside the hull's
/// authored 1000-unit comms range.
///
/// Stepped rather than jumped: the survey deadline is what OPENS both threads,
/// so a test that shortcut the act boundary would be testing a hand-built
/// situation instead of the one the mission produces. The hull is then moved by
/// hand, which is the honest analogue of helm flying the act-2 objective's own
/// Reach directive — the same substitution the #1035 test makes, and for its
/// reason: closing the range is helm's job, not this test's subject.
fn skyway_at_act_two() -> (bevy::prelude::App, bevy::prelude::Entity) {
    use project_phoenix::world::server::WorldContentRuntime;

    let args = HeadlessArgs {
        world_path: "assets/worlds/falling_skyway.toml".into(),
        ship_path: "assets/entities/alliance_destroyer.toml".into(),
        dt: SKYWAY_DT,
        max_ticks: ticks_for_sim_seconds(300.0, SKYWAY_DT),
        deterministic: true,
        seed: Some(42),
        ..test_args()
    };
    let mut app = build_headless_app(&args).expect("the scenario world must load and build");

    // THE COMMS OFFICER IS A PERSON IN THESE TESTS, and saying so is setup
    // rather than cheating. The destroyer merges `fragments/ai/fleet_baseline`,
    // whose Comms policy answers any open thread with its first response — on a
    // real destroyer Tactical (which is where the comms overlay lives) is one of
    // the four seats a crew hold, and this hull is fully AI-backfilled only
    // because a headless run has nobody in it. Dropping the response policy off
    // the LocalShip is the closest thing this harness has to seating somebody:
    // the thread then waits for the `RespondToMessage` this test submits, which
    // is the same admitted command a human's finger produces.
    //
    // It cannot be authored in the world file instead — see the note in
    // `havelock_offer`: the player hull's config comes from the lobby selection
    // wholesale, so an override on the `player-ship` row is discarded.
    // Stepped far enough for the game to start and the hull to spawn first; the
    // threads this matters for do not open until the act boundary, 90 s away.
    run(&mut app, 10);
    let ship = app
        .world_mut()
        .query_filtered::<bevy::prelude::Entity, With<LocalShip>>()
        .iter(app.world())
        .next()
        .expect("the crew's hull");
    app.world_mut()
        .entity_mut(ship)
        .remove::<project_phoenix::comms_plugin::CommsResponseAiPolicy>();

    // The survey falls due at an authored 90 s; the limit is generous so the
    // tuning pass (#1044) can lengthen the act without touching this test.
    let limit = ticks_for_sim_seconds(150.0, SKYWAY_DT);
    for _ in 0..limit {
        run(&mut app, 1);
        if skyway_flag(&app, "act") == 2 {
            break;
        }
    }
    assert_eq!(
        skyway_flag(&app, "act"),
        2,
        "the survey deadline must open act 2"
    );
    let _ = std::any::type_name::<WorldContentRuntime>();

    skyway_move(&mut app, ship, bevy::prelude::Vec3::new(900.0, 0.0, 40.0));
    // Two ticks for the comms range pass to see the new position, so a pick is
    // never refused by a stale range flag.
    run(&mut app, 2);
    (app, ship)
}

fn skyway_move(app: &mut bevy::prelude::App, ship: bevy::prelude::Entity, to: bevy::prelude::Vec3) {
    let mut physics = app
        .world_mut()
        .get_mut::<ShipPhysics>(ship)
        .expect("a ship");
    physics.x = to.x;
    physics.y = to.y;
    physics.z = to.z;
}

fn skyway_flag(app: &bevy::prelude::App, name: &str) -> i64 {
    app.world()
        .resource::<project_phoenix::world::server::WorldContentRuntime>()
        .flags
        .counter(name)
}

/// Every inbox message from `sender`, oldest first.
fn skyway_messages(
    app: &bevy::prelude::App,
    sender: &str,
) -> Vec<project_phoenix::messages::CommsMessage> {
    let uuid = app
        .world()
        .resource::<project_phoenix::world::server::WorldContentRuntime>()
        .name_to_uuid
        .get(sender)
        .cloned()
        .unwrap_or_else(|| panic!("{sender} is not in this world"));
    app.world()
        .resource::<project_phoenix::comms::server::CommsInboxRes>()
        .0
        .messages()
        .into_iter()
        .filter(|m| m.sender_uuid == uuid)
        .collect()
}

/// The live, unanswered node on `sender`'s thread.
fn skyway_open_node(
    app: &bevy::prelude::App,
    sender: &str,
) -> project_phoenix::messages::CommsMessage {
    skyway_messages(app, sender)
        .into_iter()
        .rfind(|m| m.selected_response.is_none() && !m.responses.is_empty())
        .unwrap_or_else(|| panic!("no open dialogue node from {sender}"))
}

fn skyway_options(msg: &project_phoenix::messages::CommsMessage) -> Vec<String> {
    msg.responses.iter().map(|r| r.text.clone()).collect()
}

/// Submit `text`'s response on `sender`'s open node through the ordinary
/// admitted `RespondToMessage` path, and step far enough for the pick's effects
/// and its follow-up node to land.
///
/// Addressed by TEXT rather than by index on purpose: the whole point of this
/// tree is that which options are offered depends on the world, so a hard-coded
/// index would be asserting the opposite of what the slice is for.
fn skyway_pick(app: &mut bevy::prelude::App, sender: &str, text: &str) {
    use project_phoenix::lobby::InboundMessage;
    use project_phoenix::messages::{ClientMessage, SystemControlPayload};

    let msg = skyway_open_node(app, sender);
    let index = msg
        .responses
        .iter()
        .position(|r| r.text == text)
        .unwrap_or_else(|| {
            panic!(
                "{sender}'s open node offers no '{text}'; it offers {:?}",
                skyway_options(&msg)
            )
        });
    app.world_mut()
        .resource_mut::<bevy::ecs::message::Messages<InboundMessage>>()
        .write(InboundMessage {
            token: SKYWAY_TOKEN.into(),
            msg: ClientMessage::ControlSystem {
                target: project_phoenix::system_registry::comms_system_id(),
                payload: SystemControlPayload::RespondToMessage {
                    message_id: msg.id.clone(),
                    response_index: index,
                },
            },
        });
    run(app, 4);
    assert!(
        skyway_messages(app, sender)
            .iter()
            .any(|m| m.id == msg.id && m.selected_response == Some(index)),
        "the pick '{text}' was refused rather than recorded"
    );
}

/// Order one operation on the crew's hull through the queue a scripted
/// `ctx.effects.transfer(…)` fills — the #1035 test's route, which is where the
/// reasoning for using it lives.
fn skyway_order(
    app: &mut bevy::prelude::App,
    ship: bevy::prelude::Entity,
    verb: project_phoenix::operations::OperationVerb,
    target: &str,
) -> project_phoenix::operations::OperationHold {
    use project_phoenix::operations::{PendingOperationStart, ShipOperations};
    use project_phoenix::world::server::WorldContentRuntime;

    let ship_uuid = app
        .world()
        .get::<project_phoenix::entities::spawner::EntityUuid>(ship)
        .expect("the hull carries a uuid")
        .0
        .clone();
    let target_uuid = app
        .world()
        .resource::<WorldContentRuntime>()
        .name_to_uuid
        .get(target)
        .cloned()
        .unwrap_or_else(|| panic!("{target} is not in this world"));
    app.world_mut()
        .resource_mut::<WorldContentRuntime>()
        .pending_operation_starts
        .push(PendingOperationStart {
            ship_uuid,
            verb,
            target_uuid,
        });
    run(app, 4);
    app.world()
        .get::<ShipOperations>(ship)
        .and_then(|ops| ops.active.clone())
        .expect("the operation opened")
}

/// One structure's live condition, read off the entity rather than off the file.
fn skyway_condition(app: &mut bevy::prelude::App, name: &str) -> f32 {
    use project_phoenix::entities::spawner::EntityName;
    use project_phoenix::infrastructure::InfrastructureCondition;

    let mut q = app
        .world_mut()
        .query::<(&EntityName, &InfrastructureCondition)>();
    q.iter(app.world())
        .find(|(n, _)| n.0 == name)
        .map(|(_, c)| c.0.condition())
        .unwrap_or_else(|| panic!("{name} has no condition track"))
}

/// **AC1/AC2/AC3/AC4/AC7 — Path A, end to end.** The committee are talked round,
/// both promises land on the books with the workers as the party, and the strike
/// clears: #1035's two bites let go, through the one settlement lever, at the
/// pace the tree authored.
///
/// The evidence branch is asserted here in its NEGATIVE form — a crew who never
/// went and looked are not offered the line about the file, and must therefore
/// give both promises to get the vote called. Its positive form is the test
/// below.
#[test]
fn the_negotiation_settles_the_skyway_strike_and_the_ledger_carries_both_promises() {
    use project_phoenix::operations::{HoldState, Ineligibility, OperationVerb, ProgressRate};
    use project_phoenix::world::commitments::CommitmentState;
    use project_phoenix::world::server::WorldContentRuntime;

    let (mut app, ship) = skyway_at_act_two();

    // Both channels open on the act boundary, and neither is behind the other.
    assert!(
        !skyway_messages(&app, SKYWAY_COMMITTEE).is_empty(),
        "the committee hail the destroyer when act 2 opens"
    );
    assert!(
        !skyway_messages(&app, SKYWAY_CUTTER).is_empty(),
        "and so does the operator's cutter — the force-open path is reachable \
         without saying a word to the workers"
    );

    skyway_pick(
        &mut app,
        SKYWAY_COMMITTEE,
        "world.falling_skyway.comms.listen",
    );

    // The terms node, as read by a crew who have gathered nothing.
    let terms = skyway_open_node(&app, SKYWAY_COMMITTEE);
    assert_eq!(
        terms.body, "world.falling_skyway.comms.committee_terms",
        "the committee state their two demands"
    );
    assert_eq!(
        skyway_options(&terms),
        vec![
            "world.falling_skyway.comms.promise_passage".to_string(),
            "world.falling_skyway.comms.promise_records".to_string(),
            "world.falling_skyway.comms.stall".to_string(),
        ],
        "no maintenance file on the crew's sheet, so no line about it — and no \
         vote to call yet either"
    );

    skyway_pick(
        &mut app,
        SKYWAY_COMMITTEE,
        "world.falling_skyway.comms.promise_passage",
    );
    let after_one = skyway_options(&skyway_open_node(&app, SKYWAY_COMMITTEE));
    assert!(
        !after_one.contains(&"world.falling_skyway.comms.promise_passage".to_string()),
        "a promise already on the books is not offered a second time"
    );
    assert!(
        !after_one.contains(&"world.falling_skyway.comms.call_the_vote".to_string()),
        "ONE promise is not enough for a crew who did not do the evidence work"
    );

    skyway_pick(
        &mut app,
        SKYWAY_COMMITTEE,
        "world.falling_skyway.comms.promise_records",
    );
    let ready = skyway_open_node(&app, SKYWAY_COMMITTEE);
    assert_eq!(
        ready.body, "world.falling_skyway.comms.committee_ready",
        "with two pieces of ground the committee's own line changes"
    );
    assert!(
        skyway_options(&ready).contains(&"world.falling_skyway.comms.call_the_vote".to_string())
    );

    // AC2: both promises on the books, open, with the striking workers as the
    // party.
    {
        let ledger = &app.world().resource::<WorldContentRuntime>().commitments;
        for id in ["skyway_safe_passage", "skyway_surface_records"] {
            let promise = ledger
                .get(id)
                .unwrap_or_else(|| panic!("{id} is not on the books"));
            assert_eq!(
                promise.state,
                CommitmentState::Open,
                "given, not yet settled — keeping it is the transfer window's business"
            );
            assert_eq!(
                promise.made_to, SKYWAY_COMMITTEE,
                "the striking workers are the party"
            );
        }
    }

    skyway_pick(
        &mut app,
        SKYWAY_COMMITTEE,
        "world.falling_skyway.comms.call_the_vote",
    );
    assert_eq!(skyway_flag(&app, "skyway_settled_by_negotiation"), 1);
    assert_eq!(
        skyway_flag(&app, "skyway_strike_settled"),
        0,
        "the floor has to vote first — the settlement is not on the pick"
    );

    // The PACE of this ending, measured rather than asserted: seven seconds
    // after the vote is called the rung is still stopped. The other ending has
    // finished by then (see the test below).
    run(&mut app, ticks_for_sim_seconds(7.0, SKYWAY_DT));
    assert_eq!(
        skyway_flag(&app, "strike_resolved"),
        0,
        "a floor vote takes longer than a boarding party"
    );

    run(&mut app, ticks_for_sim_seconds(20.0, SKYWAY_DT));
    assert_eq!(skyway_flag(&app, "strike_resolved"), 1);
    assert_eq!(
        objective_status(&app, "obj-a2-line"),
        project_phoenix::core::messages::ObjectiveStatus::Completed
    );
    assert_eq!(
        skyway_flag(&app, "skyway_worker_corroboration_closed"),
        0,
        "nothing about talking to them closes the route to talking to them"
    );
    {
        let register = &app.world().resource::<WorldContentRuntime>().workforce;
        assert!(
            !register.on_strike("skyway_workers"),
            "the workers are back on the rung"
        );
    }

    // AC3: #1035's two effects, reversed. The transfer that came back refused
    // in words now stands up, and the head repair comes off the unassisted rate.
    skyway_move(&mut app, ship, bevy::prelude::Vec3::new(1180.0, 0.0, 300.0));
    let transfer = skyway_order(&mut app, ship, OperationVerb::Transfer, SKYWAY_DEPOT_B);
    assert_ne!(
        transfer.state(),
        HoldState::Failed(Ineligibility::WorkStopped),
        "nobody down there is refusing to authorise it now"
    );
    assert_eq!(transfer.state(), HoldState::Holding);

    skyway_move(&mut app, ship, bevy::prelude::Vec3::new(200.0, 0.0, 0.0));
    let repair = skyway_order(
        &mut app,
        ship,
        OperationVerb::FieldRepair,
        "world.falling_skyway.entity.skyhook.name",
    );
    assert_eq!(
        repair.rate(),
        ProgressRate::FULL,
        "assisted again: the people who work the spine are back on it"
    );
}

/// **AC4's positive form.** The same tree, asked by a crew who went and got the
/// maintenance file, offers a line it offers nobody else — and that line is
/// worth a promise: one commitment plus the file calls the vote, where an
/// empty-handed crew needed two.
///
/// The branch reads the run's own evidence log through `ctx.dossier.holds`, so
/// what makes the difference is a finding on a fact sheet rather than a flag set
/// beside one.
#[test]
fn the_skyway_maintenance_file_opens_a_line_in_the_negotiation_nobody_else_gets() {
    use project_phoenix::dossier::evidence::EvidenceProvenance;
    use project_phoenix::world::server::WorldContentRuntime;

    let (mut app, _ship) = skyway_at_act_two();

    // The operator hands over their own file, which is this scenario's joke: it
    // is what makes the workers listen to the crew.
    let offer = skyway_open_node(&app, SKYWAY_CUTTER);
    assert!(
        skyway_options(&offer).contains(&"world.falling_skyway.comms.ask_for_file".to_string()),
        "the file is on the table before any authorisation is"
    );
    skyway_pick(
        &mut app,
        SKYWAY_CUTTER,
        "world.falling_skyway.comms.ask_for_file",
    );

    {
        let log = &app.world().resource::<WorldContentRuntime>().evidence;
        let entry = log
            .entries
            .iter()
            .find(|e| e.text == SKYWAY_FILE)
            .expect("the file is on the crew's sheet");
        assert_eq!(
            entry.provenance,
            EvidenceProvenance::Records,
            "a document comparison — not a scan, and not somebody's word"
        );
    }
    assert!(
        !skyway_options(&skyway_open_node(&app, SKYWAY_CUTTER))
            .contains(&"world.falling_skyway.comms.ask_for_file".to_string()),
        "and it is not offered twice"
    );

    skyway_pick(
        &mut app,
        SKYWAY_COMMITTEE,
        "world.falling_skyway.comms.listen",
    );
    let terms = skyway_options(&skyway_open_node(&app, SKYWAY_COMMITTEE));
    assert!(
        terms.contains(&"world.falling_skyway.comms.show_file".to_string()),
        "THE branch: this option exists only because the crew hold the finding — \
         the node offers {terms:?}"
    );

    skyway_pick(
        &mut app,
        SKYWAY_COMMITTEE,
        "world.falling_skyway.comms.show_file",
    );
    skyway_pick(
        &mut app,
        SKYWAY_COMMITTEE,
        "world.falling_skyway.comms.promise_passage",
    );
    let ready = skyway_open_node(&app, SKYWAY_COMMITTEE);
    assert_eq!(ready.body, "world.falling_skyway.comms.committee_ready");
    assert!(
        skyway_options(&ready).contains(&"world.falling_skyway.comms.call_the_vote".to_string()),
        "the file plus ONE promise carries it, where two promises were needed \
         without it — the evidence work is a promise the captain did not have to \
         make"
    );

    skyway_pick(
        &mut app,
        SKYWAY_COMMITTEE,
        "world.falling_skyway.comms.call_the_vote",
    );
    run(&mut app, ticks_for_sim_seconds(25.0, SKYWAY_DT));
    assert_eq!(skyway_flag(&app, "strike_resolved"), 1);
    {
        let ledger = &app.world().resource::<WorldContentRuntime>().commitments;
        assert!(
            ledger.get("skyway_surface_records").is_none(),
            "and the promise the file bought is one the captain never had to give"
        );
    }
}

/// **AC5/AC6 — Path B, end to end.** The operator clears the picket: faster than
/// the vote, reachable without negotiating, it costs people, and what it shuts
/// is said out loud rather than latched in silence.
#[test]
fn forcing_the_skyway_picket_open_is_faster_and_the_bill_arrives_on_a_console() {
    use project_phoenix::core::messages::ObjectiveStatus;
    use project_phoenix::world::server::WorldContentRuntime;

    let (mut app, _ship) = skyway_at_act_two();

    let before = skyway_condition(&mut app, SKYWAY_DEPOT_B);
    skyway_pick(
        &mut app,
        SKYWAY_CUTTER,
        "world.falling_skyway.comms.force_now",
    );

    // The order's own consequences, on the tick it is given.
    assert_eq!(skyway_flag(&app, "skyway_forced_open"), 1);
    assert_eq!(
        skyway_flag(&app, "skyway_force_casualties"),
        2,
        "read off the workers' disposition as it stood when the order was given \
         — 25, dug in, nobody having talked to them"
    );
    // AC5: the campaign remembers, on both sides of the same act.
    assert_eq!(skyway_flag(&app, "relationship.skyway_workers.damaged"), 1);
    assert_eq!(
        skyway_flag(&app, "relationship.havelock_operations.favoured"),
        1
    );
    {
        let register = &app.world().resource::<WorldContentRuntime>().workforce;
        assert_eq!(register.disposition("skyway_workers"), Some(5));
        assert_eq!(register.disposition("havelock_operations"), Some(70));
    }

    // AC6: the closure is LEGIBLE — a flag a later mission reads, an objective
    // gone red on the crew's own list, and the workers saying why.
    assert_eq!(skyway_flag(&app, "skyway_worker_corroboration_closed"), 1);
    assert_eq!(
        objective_status(&app, "obj-a2-corroborate"),
        ObjectiveStatus::Failed,
        "the corroboration objective fails where the crew can see it"
    );
    assert!(
        skyway_messages(&app, SKYWAY_COMMITTEE)
            .iter()
            .any(|m| m.body == "world.falling_skyway.comms.committee_signs_off"),
        "and the workers say it themselves before the channel goes quiet"
    );

    // AC5's pace: settled seven seconds after the order, where the vote had not
    // carried by then.
    run(&mut app, ticks_for_sim_seconds(7.0, SKYWAY_DT));
    assert_eq!(
        skyway_flag(&app, "strike_resolved"),
        1,
        "a boarding party is faster than a floor vote"
    );
    {
        let register = &app.world().resource::<WorldContentRuntime>().workforce;
        assert!(!register.on_strike("skyway_workers"));
    }

    // The casualties, resolved ON SCREEN rather than in a flag: the cutter
    // reports what it cost, the finding lands on the workers' file, and the rung
    // itself takes the damage.
    assert!(
        skyway_messages(&app, SKYWAY_CUTTER)
            .iter()
            .any(|m| m.body == "world.falling_skyway.comms.force_report_hurt"),
        "the outcome comes back in words, on the channel that asked for it"
    );
    {
        let log = &app.world().resource::<WorldContentRuntime>().evidence;
        assert!(
            log.entries
                .iter()
                .any(|e| e.text == "world.falling_skyway.evidence.picket_cleared_hurt"),
            "and onto the crew's own record of what happened"
        );
    }
    let after = skyway_condition(&mut app, SKYWAY_DEPOT_B);
    assert!(
        (before - after - 8.0).abs() < 0.001,
        "four condition points a casualty, on a track the operations panel is \
         already showing: {before} -> {after}"
    );
}

/// **AC5's "real risk".** The casualty count is a function of the situation the
/// order was given in — how dug in the workers are, and whether the captain gave
/// the picket ten minutes on the open channel first — and it is the same number
/// every time for the same seed and the same choices.
#[test]
fn the_skyway_casualty_count_is_read_off_the_ground_the_order_was_given_on() {
    // Talked down to first: they are further dug in, and it goes worse.
    let mut app = skyway_at_act_two().0;
    skyway_pick(
        &mut app,
        SKYWAY_COMMITTEE,
        "world.falling_skyway.comms.dismiss",
    );
    skyway_pick(
        &mut app,
        SKYWAY_CUTTER,
        "world.falling_skyway.comms.force_now",
    );
    assert_eq!(skyway_flag(&app, "skyway_force_casualties"), 3);

    // The same run again from a fresh app on the same seed: the risk is resolved
    // from state, so it does not move.
    let mut repeat = skyway_at_act_two().0;
    skyway_pick(
        &mut repeat,
        SKYWAY_COMMITTEE,
        "world.falling_skyway.comms.dismiss",
    );
    skyway_pick(
        &mut repeat,
        SKYWAY_CUTTER,
        "world.falling_skyway.comms.force_now",
    );
    assert_eq!(
        skyway_flag(&repeat, "skyway_force_casualties"),
        3,
        "deterministic per seed, because it is read rather than rolled"
    );

    // The captain's lever: ten minutes on the open channel takes a step off the
    // count, and costs time doing it.
    let mut warned = skyway_at_act_two().0;
    skyway_pick(
        &mut warned,
        SKYWAY_CUTTER,
        "world.falling_skyway.comms.force_warned",
    );
    assert_eq!(skyway_flag(&warned, "skyway_force_casualties"), 1);
    run(&mut warned, ticks_for_sim_seconds(7.0, SKYWAY_DT));
    assert_eq!(
        skyway_flag(&warned, "strike_resolved"),
        0,
        "warning them first is slower than not"
    );
    run(&mut warned, ticks_for_sim_seconds(8.0, SKYWAY_DT));
    assert_eq!(skyway_flag(&warned, "strike_resolved"), 1);
    assert!(
        skyway_messages(&warned, SKYWAY_CUTTER)
            .iter()
            .any(|m| m.body == "world.falling_skyway.comms.force_report_one"),
        "one casualty is its own report, not the same one with a number in it"
    );
}

// ── The radiation storm and the Act-2 rescue (issue #1037, parent #852) ──────

/// `probe_storm.toml` — the world's own header carries the authored timeline
/// every assertion below is read against.
const STORM_WORLD: &str = "assets/worlds/probe_storm.toml";
const STORM_TUG: &str = "world.probe_storm.entity.tug_storm.name";
const STORM_HULK: &str = "world.probe_storm.entity.hulk_storm.name";
const CLEAR_TUG: &str = "world.probe_storm.entity.tug_clear.name";
const CLEAR_HULK: &str = "world.probe_storm.entity.hulk_clear.name";
const STORM_LOITERER: &str = "world.probe_storm.entity.loiterer.name";
const STORM_CROSSER: &str = "world.probe_storm.entity.crosser.name";
const STORM_BYSTANDER: &str = "world.probe_storm.entity.bystander.name";

fn storm_args(dt: f64, seconds: f64) -> HeadlessArgs {
    HeadlessArgs {
        world_path: STORM_WORLD.into(),
        dt,
        max_ticks: ticks_for_sim_seconds(seconds, dt),
        deterministic: true,
        seed: Some(42),
        ..test_args()
    }
}

/// How many region entities are in the world right now, counted by the shape
/// section every authored region carries — the same component region membership
/// is computed from. A retired band that left one behind would go on jamming,
/// slowing and burning hulls with nothing on radar to explain it.
fn region_entity_count(app: &mut bevy::prelude::App) -> usize {
    app.world_mut()
        .query::<&project_phoenix::entities::spawner::RegionShapeSection>()
        .iter(app.world())
        .count()
}

/// The named ship's live modifier cache — what a band actually does to a crew's
/// instruments, read off the ship rather than off the region.
fn modifiers_of(
    app: &mut bevy::prelude::App,
    name: &str,
) -> project_phoenix::modifiers::ShipModifiers {
    app.world_mut()
        .query::<(
            &project_phoenix::entities::spawner::EntityName,
            &project_phoenix::modifiers::ShipModifiers,
        )>()
        .iter(app.world())
        .find(|(entity_name, _)| entity_name.0 == name)
        .map(|(_, modifiers)| modifiers.clone())
        .unwrap_or_else(|| panic!("{name} carries no modifier cache"))
}

/// The named ship's remaining hull as a fraction of its maximum.
fn hull_fraction_of(app: &mut bevy::prelude::App, name: &str) -> f32 {
    app.world_mut()
        .query::<(
            &project_phoenix::entities::spawner::EntityName,
            &project_phoenix::entities::spawner::EntitySystemHull,
        )>()
        .iter(app.world())
        .find(|(entity_name, _)| entity_name.0 == name)
        .map(|(_, hull)| {
            let max = hull.0.total_max();
            if max <= 0.0 {
                0.0
            } else {
                hull.0.total_current() / max
            }
        })
        .unwrap_or_else(|| panic!("{name} carries no hull"))
}

/// **Issue #1037, AC1–AC3 and AC7.** A radiation band spawned on a named
/// deadline degrades the crew's instruments, stretches a tow to half rate, and
/// retires — all of it an authored region template plus an authored schedule,
/// with no hazard system anywhere.
///
/// The comparison is the point: `tug_storm` and `tug_clear` carry the same hull,
/// the same capability, the same duration, the same payout and the same
/// interrupt rule, and differ only in which side of a band they are parked on.
/// A single tow would pass whatever the rate did.
#[test]
fn a_storm_band_degrades_instruments_and_halves_a_tow_until_it_is_retired() {
    use project_phoenix::messages::{FlagKind, ModifierSlot};
    use project_phoenix::operations::{HoldState, OperationVerb, ProgressRate};
    use project_phoenix::world::server::WorldContentRuntime;

    let dt = 1.0 / 60.0;
    let args = storm_args(dt, 34.0);
    let mut app = build_headless_app(&args).expect("the probe world must load and build");

    // AC2's precondition, and it can only be checked before the deadline fires:
    // the band is NOT authored into the world, it arrives.
    run(&mut app, 30);
    assert_eq!(
        region_entity_count(&mut app),
        0,
        "the storm is spawned by the schedule, not placed by the file — a band that was \
         already in the world at half a second has nothing to do with a deadline"
    );

    // Everything below is read tick by tick and asserted on ORDER and STATE,
    // never on frame arithmetic. `first[…]` is the sim-second each reading first
    // went true.
    let mut first: std::collections::BTreeMap<&str, f64> = std::collections::BTreeMap::new();
    // Was the storm tow ever genuinely running at the band's rate? Without this,
    // the "returns to full" reading below is satisfied by a tow that was never
    // slowed at all.
    let mut storm_tow_seen_slowed = false;
    // Did the clear tow ever leave full rate? It is the control, and a control
    // that wobbled would explain the difference by itself.
    let mut clear_tow_ever_slowed = false;
    let mut band_seen = false;

    for tick in 0..args.max_ticks {
        run(&mut app, 1);
        let sim_t = (tick + 1) as f64 * dt;

        // The TOW band by its own spawned name, not the region count: the dwell
        // band is up for the whole of this run, so a count would never fall to
        // zero and the retirement below would never be observed at all.
        let tow_band = named_entity_present(&mut app, "storm_band_tow");
        if tow_band {
            band_seen = true;
            first.entry("band_present").or_insert(sim_t);
        } else if band_seen {
            first.entry("tow_band_retired").or_insert(sim_t);
        }

        let storm = operations_named(&mut app, STORM_TUG).and_then(|ops| ops.active);
        let clear = operations_named(&mut app, CLEAR_TUG).and_then(|ops| ops.active);
        if let Some(hold) = &storm {
            if hold.rate() == ProgressRate::percent(50) {
                storm_tow_seen_slowed = true;
                first.entry("storm_tow_slowed").or_insert(sim_t);
            }
            if storm_tow_seen_slowed && hold.rate() == ProgressRate::FULL {
                first.entry("storm_tow_back_to_full").or_insert(sim_t);
            }
            if hold.state() == HoldState::Completed {
                first.entry("storm_tow_done").or_insert(sim_t);
            }
        }
        if let Some(hold) = &clear {
            if hold.rate() != ProgressRate::FULL {
                clear_tow_ever_slowed = true;
            }
            if hold.state() == HoldState::Completed {
                first.entry("clear_tow_done").or_insert(sim_t);
            }
        }

        let flags = &app.world().resource::<WorldContentRuntime>().flags;
        if flags.counter("storm_hulk_recovered") > 0 {
            first.entry("storm_hulk_recovered").or_insert(sim_t);
        }
        if flags.counter("clear_hulk_recovered") > 0 {
            first.entry("clear_hulk_recovered").or_insert(sim_t);
        }

        // AC3's other half, the instruments — sampled while the band is up, off
        // the OPERATOR's own cache, so this is what a console would render
        // rather than what the region file says.
        if tow_band && !first.contains_key("degraded_readings") {
            let inside = modifiers_of(&mut app, STORM_TUG);
            if inside.get(&ModifierSlot::RadarRange) < 1.0
                && inside.has_flag(&FlagKind::CommsJammed)
                && inside.has_flag(&FlagKind::SensorBlind)
            {
                first.entry("degraded_readings").or_insert(sim_t);
            }
        }
    }

    let at = |key: &str| -> f64 {
        *first
            .get(key)
            .unwrap_or_else(|| panic!("'{key}' never happened in this run: {first:?}"))
    };

    // ── AC1/AC2: the band arrived on its named deadline ──
    let arrived = at("band_present");
    assert!(
        (1.5..3.5).contains(&arrived),
        "the band arrives on the authored `storm_front` deadline (t=2 s), not whenever a \
         spawner felt like it — got {arrived:.2} s"
    );

    // ── AC3: inside a band, the crew's picture is worse ──
    let degraded = at("degraded_readings");
    assert!(
        degraded >= arrived && degraded - arrived < 0.5,
        "radar range, comms and sensors all degrade essentially as the band arrives \
         ({arrived:.2} s vs {degraded:.2} s)"
    );
    let clear_side = modifiers_of(&mut app, CLEAR_TUG);
    assert!(
        (clear_side.get(&ModifierSlot::RadarRange) - 1.0).abs() < 1e-6
            && !clear_side.has_flag(&FlagKind::CommsJammed)
            && !clear_side.has_flag(&FlagKind::SensorBlind),
        "…and the control three kilometres away sees perfectly well, which is what makes \
         the degradation a fact about the BAND"
    );

    // ── AC3: the tow is stretched, not stopped ──
    assert!(
        storm_tow_seen_slowed,
        "the tow inside the band must run at the 50 % its capability authors for a slow \
         zone. Seen: {first:?}"
    );
    assert!(
        !clear_tow_ever_slowed,
        "…and the identical tow in clear space must never leave full rate. If both \
         wobbled, the difference below is about something else."
    );
    let clear_done = at("clear_tow_done");
    let storm_done = at("storm_tow_done");
    assert!(
        storm_done > clear_done + 4.0,
        "the storm tow finishes MEASURABLY later than its control off the same 12-second \
         capability: {storm_done:.2} s against {clear_done:.2} s. If these matched, \
         working in a storm would be free."
    );

    // ── AC2: retirement, and its live consequence ──
    let retired = at("tow_band_retired");
    let back_to_full = at("storm_tow_back_to_full");
    assert!(
        retired > arrived,
        "precondition: the band has to have been there before it can go"
    );
    assert!(
        back_to_full >= retired && back_to_full - retired < 0.5,
        "when the band is retired the tow returns to full rate at once — with nothing in \
         the destroy path knowing an operation was running. Retired at {retired:.2} s, \
         full rate at {back_to_full:.2} s"
    );
    assert!(
        storm_done > retired,
        "precondition for the reading above: the tow was still running when its band was \
         retired ({storm_done:.2} s vs {retired:.2} s)"
    );

    // ── The completion SIGNAL: the payout crosses the authored threshold ──
    // This is the whole mechanism the Falling Skyway rescue is read through.
    for (flag, done) in [
        ("clear_hulk_recovered", "clear_tow_done"),
        ("storm_hulk_recovered", "storm_tow_done"),
    ] {
        let (raised, completed) = (at(flag), at(done));
        assert!(
            raised >= completed && raised - completed < 0.2,
            "a completed tow pays its `condition_on_complete` into the towed craft, the \
             payout is queued for the one system that owns condition edges, and the craft's \
             own threshold flag comes up a tick later — which is what a scenario hangs a \
             handler off, there being no 'operation completed' trigger to use instead. \
             {done} at {completed:.3} s, {flag} at {raised:.3} s"
        );
    }
    let (storm_condition, clear_condition) = (
        condition_of(&mut app, STORM_HULK),
        condition_of(&mut app, CLEAR_HULK),
    );
    assert!(
        storm_condition > 70.0 && clear_condition > 70.0,
        "both hulks are carried from 30 of 100 to 75 by the authored 45-point payout — got \
         {storm_condition} and {clear_condition}"
    );

    // ── The group is SILENT while the other band is still up ──
    // One of two is not the whole storm, and a group that fired here would
    // satisfy a fires-at-the-end assertion just as well as the real thing.
    assert_eq!(
        app.world()
            .resource::<WorldContentRuntime>()
            .flags
            .counter("sweep_complete"),
        0,
        "the tow band has been retired and the dwell band has not — the sweep is not over, \
         and `on_all_destroyed` must not have fired"
    );
    assert_eq!(
        region_entity_count(&mut app),
        1,
        "exactly the one band that has not reached its own retirement is left"
    );
    assert_eq!(hold_of(&mut app, STORM_TUG).verb(), OperationVerb::Tow);
}

/// **Issue #1037, AC2 and AC3's tuning target.** A band is survivable to cross
/// and fatal to live in, and when the last one retires the corridor is clear
/// with nothing left behind.
///
/// Three identical Alliance Couriers, one band, and the only difference between
/// them is dwell: the crosser is flown out at the authored crossing time (the
/// honest analogue of helm taking a ship through), the loiterer stays, and the
/// bystander is three kilometres away and proves nothing else in this world
/// hurts anybody.
#[test]
fn a_storm_band_is_survivable_to_cross_and_fatal_to_live_in() {
    use project_phoenix::core::messages::ObjectiveStatus;
    use project_phoenix::world::server::WorldContentRuntime;

    let dt = 1.0 / 60.0;
    // The dwell band retires at t=100; the run gives it eight seconds' grace.
    let args = storm_args(dt, 108.0);
    let mut app = build_headless_app(&args).expect("the probe world must load and build");

    // A 520-unit band crossed at an Alliance Courier's IN-BAND speed. The band
    // arrives at t=3, so the crossing ends 39 seconds later, at t=42.
    //
    // MOVED BY THE SLOW-ZONE SIGN FIX, from 32.0, and the derivation moved with
    // it in two ways. It used to read "a 520-unit band crossed at a destroyer's
    // 18 units a second is 29 seconds inside it" — which borrowed a different
    // hull's number, and took that hull's speed in CLEAR SPACE for its speed
    // inside a band. `region_radiation_band.toml`'s `slow_zone` authored a
    // POSITIVE `thrust_modifier`, so the band was in fact making ships 60%
    // FASTER, and 29 seconds was neither hull's crossing under either reading.
    //
    // The ship flown out here is a courier, so this is a courier's crossing:
    // 22 u/s clear x 0.6 in-band = 13.2 u/s, and 520 / 13.2 = 39 s. The
    // destroyer the mission actually fields crosses the same band in 48 s at
    // 10.8 u/s, and `region_radiation_band.toml` carries that arithmetic — the
    // two are consistent rather than interchangeable, which is the thing the
    // old constant got wrong.
    //
    // Both hulls still land inside the `0.5..0.85` band asserted below under
    // their OWN crossing. MEASURED in this world: the loiterer dies at t=88.6
    // having been in the band since t=3, so 200 points buy 85.6 seconds and the
    // band bills 2.34 hull points a second. A courier's 39-second crossing is
    // therefore 91 points and it comes out on ~54% of 200; a destroyer's
    // 48-second crossing is 112 points and it comes out on ~63% of 300. The
    // substitution the world header makes for run length is still faithful, and
    // it is now faithful on purpose rather than by accident.
    const CROSSING_ENDS_AT: f64 = 42.0;
    let mut crosser_hull_on_exit = 1.0f32;
    let mut loiterer_gone_at: Option<f64> = None;
    let mut sweep_complete_at: Option<f64> = None;
    let mut regions_after_sweep: Option<usize> = None;
    let mut flown_out = false;

    for tick in 0..args.max_ticks {
        run(&mut app, 1);
        let sim_t = (tick + 1) as f64 * dt;

        if !flown_out && sim_t >= CROSSING_ENDS_AT {
            crosser_hull_on_exit = hull_fraction_of(&mut app, STORM_CROSSER);
            move_named_to(
                &mut app,
                STORM_CROSSER,
                bevy::prelude::Vec3::new(0.0, 0.0, -6600.0),
            );
            flown_out = true;
        }
        if loiterer_gone_at.is_none() && !named_entity_present(&mut app, STORM_LOITERER) {
            loiterer_gone_at = Some(sim_t);
        }
        let swept = app
            .world()
            .resource::<WorldContentRuntime>()
            .flags
            .counter("sweep_complete");
        if swept > 0 && sweep_complete_at.is_none() {
            sweep_complete_at = Some(sim_t);
            regions_after_sweep = Some(region_entity_count(&mut app));
        }
    }

    // ── The tuning target, both halves, on one hull ──
    let died_at = loiterer_gone_at.expect(
        "the hull that never left the band must be destroyed by it — if it survived, \
         'lingering is dangerous' is not true of these numbers",
    );
    assert!(
        died_at > CROSSING_ENDS_AT,
        "precondition: it must outlive a crossing, or the crosser's survival is luck \
         rather than tuning (died at {died_at:.1} s, a crossing ends at \
         {CROSSING_ENDS_AT:.1} s)"
    );
    assert!(
        named_entity_present(&mut app, STORM_CROSSER),
        "the SAME hull, flown out after a crossing's worth of exposure, is alive at the end \
         of the run. Being caught by a band is not death; living in one is."
    );
    assert!(
        (0.5..0.85).contains(&crosser_hull_on_exit),
        "a crossing has to COST something and not nearly everything: the crosser came out \
         on {:.0}% hull, and the band is tuned for somewhere between a half and four \
         fifths",
        crosser_hull_on_exit * 100.0
    );
    assert!(
        (hull_fraction_of(&mut app, STORM_BYSTANDER) - 1.0).abs() < 1e-3,
        "the bystander three kilometres clear is untouched — without it, 'the band killed \
         the loiterer' is satisfied by 'something in this world kills couriers'"
    );

    // ── AC2: the sweep completes and leaves nothing behind ──
    let swept_at =
        sweep_complete_at.expect("`on_all_destroyed` must fire when the LAST band retires");
    assert!(
        swept_at > died_at,
        "precondition: the last band outlived the hull it killed, so the death above is the \
         band's work and not the retirement's ({swept_at:.1} s vs {died_at:.1} s)"
    );
    assert_eq!(
        regions_after_sweep,
        Some(0),
        "when the group fires, every band is gone: a sweep that leaked a region entity would \
         go on jamming and burning hulls with nothing on radar to explain it"
    );
    assert_eq!(region_entity_count(&mut app), 0, "…and stays gone");
    assert_eq!(
        app.world()
            .resource::<WorldContentRuntime>()
            .flags
            .counter("sweep_complete"),
        1,
        "exactly once"
    );
    assert_eq!(
        objective_status(&app, "obj-sweep-clear"),
        ObjectiveStatus::Completed
    );
}

// ── Falling Skyway, Act 2: the storm and the rescue (issue #1037) ────────────

const SKYWAY_WORLD: &str = "assets/worlds/falling_skyway.toml";
const SKYWAY_LYRA: &str = "world.falling_skyway.entity.lyra_ascending.name";
/// The three craft the sweep schedule actually moves. `shuttle_wick` works the
/// depot ladder east of the corridor and is deliberately left alone.
const SKYWAY_CORRIDOR_TRAFFIC: [&str; 3] = [
    "world.falling_skyway.entity.convoy_meridian.name",
    "world.falling_skyway.entity.hauler_lark.name",
    "world.falling_skyway.entity.hauler_pell.name",
];
const SKYWAY_LADDER_SHUTTLE: &str = "world.falling_skyway.entity.shuttle_wick.name";
/// The three bands, by the names the Act-2 script spawns them under.
const SKYWAY_BANDS: [&str; 3] = ["storm_band_one", "storm_band_two", "storm_band_three"];

fn skyway_args(dt: f64, seconds: f64) -> HeadlessArgs {
    HeadlessArgs {
        world_path: SKYWAY_WORLD.into(),
        // The mission's authored hull, and the one that carries the `tow`
        // capability the rescue is made of.
        ship_path: "assets/entities/alliance_destroyer.toml".into(),
        dt,
        max_ticks: ticks_for_sim_seconds(seconds, dt),
        deterministic: true,
        seed: Some(42),
        ..test_args()
    }
}

/// The authored `due_secs` of one of this world's deadlines, read from the
/// config rather than restated in the test — the tuning pass (#1044) moves
/// these numbers and nothing here should have to be edited with them.
fn skyway_deadline_secs(app: &bevy::prelude::App, id: &str) -> i64 {
    app.world()
        .resource::<project_phoenix::world::config::WorldConfig>()
        .deadlines
        .iter()
        .find(|d| d.id == id)
        .unwrap_or_else(|| panic!("the world authors the '{id}' deadline"))
        .due_secs
}

/// The named civilian's traffic state — the route it is currently flying and
/// how far through the compliance machine its standing order has got.
fn civilian_state_of(
    app: &mut bevy::prelude::App,
    name: &str,
) -> project_phoenix::civilian::CivilianState {
    app.world_mut()
        .query::<(
            &project_phoenix::entities::spawner::EntityName,
            &project_phoenix::civilian::CivilianTraffic,
        )>()
        .iter(app.world())
        .find(|(entity_name, _)| entity_name.0 == name)
        .map(|(_, traffic)| traffic.0.clone())
        .unwrap_or_else(|| panic!("{name} is not civilian traffic in this world"))
}

/// **Issue #1037, AC1/AC2/AC4/AC6.** Act 2 driven end to end with nobody at the
/// consoles: the storm sweeps the corridor in three bands and clears, the
/// traffic gets out of its way and survives, and the rescue that nobody ran
/// FAILS — loudly, with an on-screen consequence and campaign state written.
///
/// The failure branch is the one an unattended run produces, and that is the
/// honest default: opening an operation is a crew verb, so a backfilled bridge
/// never tows anybody. The companion test below drives the same act with a crew
/// that does.
#[test]
fn falling_skyway_act_2_sweeps_the_corridor_and_fails_the_rescue_nobody_ran() {
    use project_phoenix::civilian::ComplianceState;
    use project_phoenix::core::messages::ObjectiveStatus;
    use project_phoenix::world::server::WorldContentRuntime;

    let dt = 1.0 / 30.0;
    let probe = build_headless_app(&skyway_args(dt, 1.0)).expect("the world must load");
    // The act's own clock decides the run length. Six seconds past the close, so
    // lengthening Act 2 in the TOML does not silently turn this into a test of an
    // act that never finished.
    let closes_at = skyway_deadline_secs(&probe, "storm_passed_due") as f64;
    drop(probe);

    let args = skyway_args(dt, closes_at + 6.0);
    let mut app = build_headless_app(&args).expect("the scenario world must load and build");

    let mut first: std::collections::BTreeMap<String, f64> = std::collections::BTreeMap::new();
    let mut seen: std::collections::BTreeSet<String> = Default::default();
    // How far every diverted craft was from each band on the tick that band
    // arrived. The claim is that they RESPOND to the schedule, and that is a
    // fact about where they were when the weather turned up.
    let mut clearance: std::collections::BTreeMap<String, f32> = Default::default();
    let mut compliance_reached: std::collections::BTreeMap<String, bool> = Default::default();

    for tick in 0..args.max_ticks {
        run(&mut app, 1);
        let sim_t = (tick + 1) as f64 * dt;

        for band in SKYWAY_BANDS {
            if named_entity_present(&mut app, band) {
                // On the FIRST tick a band exists, measure how far every craft
                // the forecast moved is from it. That is the moment the claim is
                // about: a schedule the traffic responded to is one where nobody
                // is standing under the weather when it turns up.
                if seen.insert(band.to_string()) {
                    let centre = position_of(&mut app, band);
                    for name in SKYWAY_CORRIDOR_TRAFFIC {
                        let craft = position_of(&mut app, name);
                        let range =
                            ((craft.x - centre.x).powi(2) + (craft.z - centre.z).powi(2)).sqrt();
                        clearance.insert(format!("{band} :: {name}"), range);
                    }
                }
                first.entry(format!("{band}_up")).or_insert(sim_t);
            } else if seen.contains(band) {
                first.entry(format!("{band}_gone")).or_insert(sim_t);
            }
        }
        if !named_entity_present(&mut app, SKYWAY_LYRA) {
            first.entry("lyra_gone".to_string()).or_insert(sim_t);
        }

        for name in SKYWAY_CORRIDOR_TRAFFIC {
            let state = civilian_state_of(&mut app, name);
            if state.compliance() == ComplianceState::Complying {
                compliance_reached.insert(name.to_string(), true);
                first.entry(format!("{name}_complying")).or_insert(sim_t);
            }
        }

        let flags = &app.world().resource::<WorldContentRuntime>().flags;
        for flag in [
            "a2_front_warned",
            "a2_rescue_resolved",
            "a2_lyra_lost",
            "act2_complete",
        ] {
            if flags.counter(flag) > 0 {
                first.entry(flag.to_string()).or_insert(sim_t);
            }
        }
    }

    let at = |key: &str| -> f64 {
        *first
            .get(key)
            .unwrap_or_else(|| panic!("'{key}' never happened in this run: {first:?}"))
    };

    // ── AC1/AC2: three bands, each on its own deadline, each retired ──
    // Read against the world's authored `due_secs` rather than against numbers
    // restated here.
    for (band, deadline) in SKYWAY_BANDS.iter().zip([
        "storm_band_one_due",
        "storm_band_two_due",
        "storm_band_three_due",
    ]) {
        let due = skyway_deadline_secs(&app, deadline) as f64;
        let up = at(&format!("{band}_up"));
        assert!(
            (up - due).abs() < 2.0,
            "{band} arrives on its authored deadline ({due} s), not on a timer this test \
             knows about — got {up:.1} s"
        );
        let gone = at(&format!("{band}_gone"));
        assert!(
            gone > up,
            "{band} is RETIRED, which is what makes this a sweep rather than a wall \
             (up at {up:.1} s, gone at {gone:.1} s)"
        );
    }
    assert!(
        at("storm_band_one_up") < at("storm_band_two_up")
            && at("storm_band_two_up") < at("storm_band_three_up"),
        "the bands arrive in order, stepping down the corridor: {first:?}"
    );
    assert_eq!(
        region_entity_count(&mut app),
        0,
        "the corridor is CLEAR when the sweep is over — a leaked region entity would go \
         on jamming, slowing and burning hulls with nothing on radar to explain it"
    );

    // ── AC4: the traffic responded to the schedule ──
    for name in SKYWAY_CORRIDOR_TRAFFIC {
        assert!(
            compliance_reached.get(name).copied().unwrap_or(false),
            "{name} must take the divert order the forecast issues — the sweep is \
             announced through #1028's ordinary compliance machine, and a craft that \
             never answered would be flying the lane into a band"
        );
        assert!(
            at(&format!("{name}_complying")) < at("storm_band_one_up"),
            "…and be under way BEFORE the first band, which is what the 45 seconds of \
             forecast are for"
        );
        assert!(
            named_entity_present(&mut app, name),
            "{name} survives the storm, which is the whole point of moving it"
        );
    }
    // Every craft against every band, on the tick that band arrived. 260 units
    // is `region_radiation_band.toml`'s own authored radius.
    for (pair, range) in &clearance {
        assert!(
            *range > 260.0,
            "{pair}: the craft was {range:.0} units from the band's centre when it \
             arrived, which is inside its authored 260-unit radius. Traffic is routed \
             AROUND a sweep; it does not fly into one and hope."
        );
    }
    assert_eq!(
        clearance.len(),
        SKYWAY_BANDS.len() * SKYWAY_CORRIDOR_TRAFFIC.len(),
        "every craft was sampled against every band"
    );
    assert_eq!(
        civilian_state_of(&mut app, SKYWAY_LADDER_SHUTTLE)
            .route()
            .map(str::to_string),
        Some("ladder_run".to_string()),
        "the ladder shuttle is never diverted: it works east of the corridor and no band \
         comes near it. Ordering it off a lane it was safe on would be theatre."
    );
    assert_eq!(
        app.world()
            .resource::<WorldContentRuntime>()
            .flags
            .counter("a2_traffic_lost"),
        0,
        "no civilian craft is lost to the storm"
    );
    assert_eq!(
        objective_status(&app, "obj-a2-shelter"),
        ObjectiveStatus::Completed
    );

    // ── AC5/AC6: the rescue nobody ran fails, and says so ──
    let resolved = at("a2_rescue_resolved");
    let lost = at("lyra_gone");
    assert!(
        (resolved - skyway_deadline_secs(&app, "lyra_clear_due") as f64).abs() < 2.0,
        "the rescue resolves on its own visible deadline"
    );
    assert!(
        lost >= resolved - 0.2 && lost - resolved < 1.0,
        "…and with nobody having towed her, the band has her at that beat (resolved at \
         {resolved:.1} s, gone at {lost:.1} s)"
    );
    assert_eq!(
        objective_status(&app, "obj-a2-rescue"),
        ObjectiveStatus::Failed,
        "a rescue nobody ran is a FAILED objective, not a quietly missing one"
    );
    assert_eq!(
        objective_status(&app, "obj-a2-loss-report"),
        ObjectiveStatus::Active,
        "…and the consequence is on the panel: somebody has to tell Control, and it sits \
         there until they do"
    );
    let flags = &app.world().resource::<WorldContentRuntime>().flags;
    assert_eq!(
        (
            flags.counter("skyway_lyra_lost"),
            flags.counter("skyway_lyra_recovered"),
            flags.counter("skyway_storm_passed"),
        ),
        (1, 0, 1),
        "the campaign state is WRITTEN. Exactly one of lost/recovered is always set, so \
         a later act reads a fact rather than an absence."
    );
    assert_eq!(
        (flags.counter("act"), flags.counter("act2_complete")),
        (3, 1),
        "and the act closes on its own clock whatever the crew got done"
    );
}

/// **Issue #1037, AC3/AC5.** The same act with a crew that starts the tow before
/// the weather arrives: the rescue is COMPLETABLE, the storm makes it cost more,
/// and the craft is still there when the band that would have taken her passes.
///
/// The tow is opened through the same queue a console's `StartOperation` and a
/// scripted `ctx.effects.tow(…)` both land in — the applier resolves the names
/// and `tick_operations` decides, which is the only place range, power and
/// capability are tested.
#[test]
fn falling_skyway_act_2_rescue_lands_when_the_crew_start_before_the_band() {
    use project_phoenix::core::messages::ObjectiveStatus;
    use project_phoenix::operations::{
        HoldState, OperationVerb, PendingOperationStart, ProgressRate, ShipOperations,
    };
    use project_phoenix::ship::state::ShipPhysics;
    use project_phoenix::world::server::WorldContentRuntime;

    let dt = 1.0 / 30.0;
    let probe = build_headless_app(&skyway_args(dt, 1.0)).expect("the world must load");
    let front_at = skyway_deadline_secs(&probe, "storm_front_due") as f64;
    let band_at = skyway_deadline_secs(&probe, "storm_band_one_due") as f64;
    let clear_by = skyway_deadline_secs(&probe, "lyra_clear_due") as f64;
    drop(probe);

    // Six seconds past the rescue's own deadline: long enough to see the Lyra
    // survive the beat that would otherwise have taken her.
    let args = skyway_args(dt, clear_by + 6.0);
    let mut app = build_headless_app(&args).expect("the scenario world must load and build");
    run(&mut app, 10);

    // The crew's hull, found by its capability table: the player row carries no
    // authored name, because its config comes from the lobby-selected template
    // wholesale.
    let (ship, ship_uuid) = app
        .world_mut()
        .query::<(
            bevy::prelude::Entity,
            &project_phoenix::entities::spawner::EntityUuid,
            &ShipOperations,
        )>()
        .iter(app.world())
        .map(|(entity, uuid, _)| (entity, uuid.0.clone()))
        .next()
        .expect("the destroyer authors an [operations] table");
    let lyra_uuid = app
        .world()
        .resource::<WorldContentRuntime>()
        .name_to_uuid
        .get(SKYWAY_LYRA)
        .cloned()
        .expect("the Lyra is an authored entity of this world");

    // The crew set off with ten seconds to spare before the first band — early
    // enough to bank real progress in clear air, late enough that the weather
    // catches the tow mid-flight, which is the case the act is actually about.
    // A crew who waited for the band could not finish at all: 24 authored
    // seconds at half rate is 48, and by then the deadline is 47 away.
    let start_at = band_at - 10.0;
    assert!(
        start_at > front_at,
        "precondition: the crew cannot start before the forecast tells them she is there \
         ({front_at} s, {start_at} s)"
    );
    let mut opened = false;
    let mut rate_before_band: Option<ProgressRate> = None;
    let mut rate_under_band: Option<ProgressRate> = None;
    let mut completed_at: Option<f64> = None;
    let mut recovered_at: Option<f64> = None;

    for tick in 0..args.max_ticks {
        let sim_t = tick as f64 * dt;
        if !opened && sim_t >= start_at {
            // Alongside. Helm's job, done by hand here for the reason every
            // operations test in this file moves a ship by hand: this is a test
            // of the rescue, not of station-keeping.
            let drift = position_of(&mut app, SKYWAY_LYRA);
            let mut physics = app
                .world_mut()
                .get_mut::<ShipPhysics>(ship)
                .expect("the crew's hull is a ship");
            physics.x = drift.x + 40.0;
            physics.y = drift.y;
            physics.z = drift.z + 40.0;
            app.world_mut()
                .resource_mut::<WorldContentRuntime>()
                .pending_operation_starts
                .push(PendingOperationStart {
                    ship_uuid: ship_uuid.clone(),
                    verb: OperationVerb::Tow,
                    target_uuid: lyra_uuid.clone(),
                });
            opened = true;
        }
        run(&mut app, 1);
        let sim_t = (tick + 1) as f64 * dt;

        if let Some(hold) = app
            .world()
            .get::<ShipOperations>(ship)
            .and_then(|ops| ops.active.clone())
        {
            if sim_t > start_at + 1.0 && sim_t < band_at - 1.0 {
                rate_before_band = Some(hold.rate());
            }
            if sim_t > band_at + 1.0 && !hold.is_settled() {
                rate_under_band = Some(hold.rate());
            }
            if hold.state() == HoldState::Completed && completed_at.is_none() {
                completed_at = Some(sim_t);
            }
        }
        if recovered_at.is_none()
            && app
                .world()
                .resource::<WorldContentRuntime>()
                .flags
                .counter("a2_lyra_recovered")
                > 0
        {
            recovered_at = Some(sim_t);
        }
    }

    // ── AC3: the storm made the work cost more, without stopping it ──
    assert_eq!(
        rate_before_band,
        Some(ProgressRate::FULL),
        "a tow run in clear air runs at full rate — the crew who set off early are the \
         crew the weather has not reached yet"
    );
    assert_eq!(
        rate_under_band,
        Some(ProgressRate::percent(50)),
        "…and once the band is on top of them the SAME tow runs at the 50 % the hull's \
         capability authors for a slow zone. Stretched, not stopped."
    );

    // ── AC5: it completes, and the completion is what the mission reads ──
    let done = completed_at.expect(
        "a tow started twenty-five seconds before the first band must COMPLETE inside \
         the rescue's deadline — if it cannot, the act is unwinnable rather than hard",
    );
    let recovered = recovered_at.expect("…and its payout must raise the craft's own flag");
    assert!(
        done < clear_by,
        "the rescue lands before its visible deadline ({done:.1} s against {clear_by} s)"
    );
    assert!(
        recovered >= done && recovered - done < 0.5,
        "the flag comes up off the completion's payout, a tick behind it ({done:.1} s, \
         {recovered:.1} s)"
    );
    assert!(
        condition_of(&mut app, SKYWAY_LYRA) > 50.0,
        "…because the authored 45-point payout carried her over her own half-way line \
         from 30, which is what `skyway_lyra_under_control` means"
    );

    // ── AC5/AC6: she is still there, and the record says so ──
    assert!(
        named_entity_present(&mut app, SKYWAY_LYRA),
        "the deadline that takes her when nobody tows has passed, and she is still in the \
         world: the failure branch is guarded on the recovery, not on the clock alone"
    );
    assert_eq!(
        objective_status(&app, "obj-a2-rescue"),
        ObjectiveStatus::Completed
    );
    assert!(
        objective_status_opt(&app, "obj-a2-loss-report").is_none(),
        "and the consequence objective is never posted at all"
    );
    let flags = &app.world().resource::<WorldContentRuntime>().flags;
    assert_eq!(
        (
            flags.counter("skyway_lyra_recovered"),
            flags.counter("skyway_lyra_lost"),
        ),
        (1, 0),
        "exactly one of the two campaign flags is written, and it is the other one this \
         time"
    );
}

// ── The scan-versus-dossier diff (issue #1038, parent #852) ──────────────────

/// `probe_scandiff.toml` — four rungs authored identically except for the one
/// number under test. The world's own header carries what each is for.
const SCANDIFF_WORLD: &str = "assets/worlds/probe_scandiff.toml";
/// Under its recorded standard, and read. The diff.
const DIFF_FRAYED: &str = "world.probe_scandiff.entity.ladder_frayed.name";
/// Over its recorded standard, and read. The derivation proof.
const DIFF_SOUND: &str = "world.probe_scandiff.entity.ladder_sound.name";
/// Under its recorded standard, and never read. The control.
const DIFF_UNREAD: &str = "world.probe_scandiff.entity.ladder_unread.name";
/// Read while its record still held up, and slipping under it afterwards.
const DIFF_SLIPPING: &str = "world.probe_scandiff.entity.ladder_slipping.name";
/// What the operator's file claims, on every rung, as a BRIEFING entry.
const DIFF_CLAIM: &str = "world.probe_scandiff.evidence.record_certified";
/// What a crew who put the two documents side by side write down.
const DIFF_FINDING: &str = "world.probe_scandiff.evidence.record_contradicted";

/// Every finding on `subject_uuid`'s file, oldest first, as `(text, provenance)`.
fn diff_file(app: &App, subject_uuid: &str) -> Vec<(String, String)> {
    app.world()
        .resource::<project_phoenix::world::server::WorldContentRuntime>()
        .evidence
        .for_subject(subject_uuid)
        .map(|e| (e.text.clone(), e.provenance.as_str().to_string()))
        .collect()
}

fn diff_flag(app: &App, name: &str) -> i64 {
    app.world()
        .resource::<project_phoenix::world::server::WorldContentRuntime>()
        .flags
        .counter(name)
}

/// The tick a finding about `subject_uuid` was gathered on.
fn diff_gathered_at(app: &App, subject_uuid: &str, text: &str) -> u64 {
    app.world()
        .resource::<project_phoenix::world::server::WorldContentRuntime>()
        .evidence
        .for_subject(subject_uuid)
        .find(|e| e.text == text)
        .unwrap_or_else(|| panic!("{subject_uuid} has no finding {text}"))
        .gathered_at_tick
}

fn diff_now_tick(app: &App) -> u64 {
    app.world()
        .resource::<project_phoenix::sim_tick::SimTick>()
        .0
}

/// Move the scanner alongside the structure at `x` and take a reading through
/// the ordinary admitted path, then give the mirrored flag its one-tick bridge
/// into the trigger pipeline: `collect_world_events` drains at the top of the
/// NEXT tick's `SimSet::Physics`, exactly as an infrastructure crossing does.
fn scan_from_alongside(app: &mut App, x: f32, uuid: &str) {
    place_scanner_at(app, x - 80.0);
    ask_for_scan(app, uuid);
    run(app, 4);
}

/// **Issue #1038, AC2/AC3/AC7 — the whole truth table in one run.**
///
/// Four rungs, one authored record, one comparison function. Which of them the
/// crew end up with a finding about is decided by exactly two things, and the
/// world holds all four combinations of them at once:
///
/// * FRAYED against SOUND is the DERIVATION. Identical authoring, identical
///   record, scanned by the same test from the same distance — and only the one
///   whose `condition` sits under the standard its file claims produces a
///   finding. No copy is edited between them; the difference is a number.
/// * FRAYED against UNREAD is the ACT. Identical in every authored respect.
///   Only the one the crew pointed something at produces a finding, so a run
///   that never scans and a run that does are different runs.
/// * SLIPPING is the two facts arriving in the OTHER order, which is what a
///   scripted reveal could not do. The scan comes back while the record is
///   still defensible and files nothing; the finding lands later, with nobody
///   scanning anything, because the condition crossed the claim.
#[test]
fn the_diff_falls_out_of_the_condition_and_the_crews_own_reading_and_nothing_else() {
    let dt = 1.0 / 60.0;
    let args = HeadlessArgs {
        world_path: SCANDIFF_WORLD.into(),
        ship_path: "assets/entities/alliance_destroyer.toml".into(),
        dt,
        max_ticks: ticks_for_sim_seconds(30.0, dt),
        deterministic: true,
        seed: Some(1038),
        ..test_args()
    };
    let mut app = build_headless_app(&args).expect("app should build");
    // Far enough in that the game is InProgress and the hull is backfilled.
    run(&mut app, 60);

    let frayed = scan_uuid_named(&mut app, DIFF_FRAYED);
    let sound = scan_uuid_named(&mut app, DIFF_SOUND);
    let unread = scan_uuid_named(&mut app, DIFF_UNREAD);
    let slipping = scan_uuid_named(&mut app, DIFF_SLIPPING);

    // The paperwork is in the crew's hands before anybody looks at anything, on
    // every rung, and it is the same claim on each.
    assert_eq!(diff_flag(&app, "record_filed"), 1);
    for subject in [&frayed, &sound, &unread, &slipping] {
        assert_eq!(
            diff_file(&app, subject),
            vec![(DIFF_CLAIM.to_string(), "briefing".to_string())],
            "every rung starts with the operator's claim and nothing else"
        );
    }
    assert_eq!(diff_flag(&app, "records_diff_found"), 0);

    // ── The rung that cannot be what its file says ───────────────────────────
    scan_from_alongside(&mut app, 600.0, &frayed);
    assert_eq!(
        diff_flag(&app, "records_diff_found"),
        1,
        "the crew looked, and the two documents disagree"
    );
    assert_eq!(
        diff_file(&app, &frayed),
        vec![
            (DIFF_CLAIM.to_string(), "briefing".to_string()),
            (DIFF_FINDING.to_string(), "records".to_string()),
        ],
        "the finding is filed under RECORDS — two documents side by side, not a \
         sensor return and not somebody's word"
    );

    // ── THE DERIVATION. The same act, on a rung whose file is true ───────────
    scan_from_alongside(&mut app, 1200.0, &sound);
    assert_eq!(
        diff_flag(
            &app,
            &project_phoenix::science::scanned_flag("ladder_sound")
        ),
        1,
        "precondition: the crew DID read this one — the act happened"
    );
    assert_eq!(
        diff_flag(&app, "records_diff_found"),
        1,
        "…and found nothing, because 78 points can carry what the file claims. \
         The only difference from the frayed rung is the authored condition."
    );
    assert_eq!(
        diff_file(&app, &sound),
        vec![(DIFF_CLAIM.to_string(), "briefing".to_string())],
    );

    // ── The other ordering: read first, wrong afterwards ─────────────────────
    scan_from_alongside(&mut app, 1800.0, &slipping);
    let slipping_read_tick = diff_now_tick(&app);
    assert_eq!(
        diff_flag(
            &app,
            &project_phoenix::science::scanned_flag("ladder_slipping")
        ),
        1,
        "precondition: the reading came back"
    );
    assert_eq!(
        diff_flag(&app, "records_diff_found"),
        1,
        "…while the rung still met its recorded standard, so there was nothing \
         to write down at the time"
    );

    // Nobody scans anything from here on. The rung simply keeps failing under
    // its own authored decay until it crosses the claim.
    run(&mut app, ticks_for_sim_seconds(8.0, dt));
    assert_eq!(
        diff_flag(&app, "records_diff_found"),
        2,
        "the comparison completed itself when the OTHER half moved — which a \
         flag flip hung off the player pressing scan could not do"
    );
    assert_eq!(
        diff_file(&app, &slipping),
        vec![
            (DIFF_CLAIM.to_string(), "briefing".to_string()),
            (DIFF_FINDING.to_string(), "records".to_string()),
        ],
    );
    assert!(
        diff_gathered_at(&app, &slipping, DIFF_FINDING) > slipping_read_tick,
        "and it is stamped after the crew read it, not at the moment they did"
    );

    // ── THE CONTROL. Same condition as the frayed rung, never looked at ──────
    assert_eq!(
        diff_flag(
            &app,
            &project_phoenix::science::scanned_flag("ladder_unread")
        ),
        0,
        "precondition: nobody ever pointed anything at this one"
    );
    assert_eq!(
        diff_flag(&app, "ladder_unread_meets_record"),
        0,
        "precondition: its record does not hold up either — #1025 says so off \
         the live track, whether or not anybody asks"
    );
    assert_eq!(
        diff_file(&app, &unread),
        vec![(DIFF_CLAIM.to_string(), "briefing".to_string())],
        "…and its file is as clean as the sound rung's. The discrepancy is \
         DISCOVERABLE, not automatic."
    );
    assert_eq!(
        diff_flag(&app, "records_diff_found"),
        2,
        "two findings for the whole run: one rung read while wrong, one read \
         before it went wrong, and two rungs that produced nothing"
    );
}

/// The published fact sheet — the payload a tactical console renders — carries
/// both halves of the contradiction, apart, and each says where it came from.
///
/// AC1 and AC4 are the same panel: the recorded facts a competent crew can read,
/// and the finding that they do not survive a look. The condition rows beside
/// them are the projection's, off the same published snapshot the scan read.
#[test]
fn the_recorded_claim_and_the_finding_that_breaks_it_are_both_on_the_fact_sheet() {
    use project_phoenix::dossier::{dossier_blackboard_key, FACT_CONDITION};
    use project_phoenix::messages::{DossierBlackboard, DossierValue, SystemBlackboard};
    use project_phoenix::server_app::ShipSystemBlackboards;

    let dt = 1.0 / 60.0;
    let args = HeadlessArgs {
        world_path: SCANDIFF_WORLD.into(),
        ship_path: "assets/entities/alliance_destroyer.toml".into(),
        dt,
        max_ticks: ticks_for_sim_seconds(30.0, dt),
        deterministic: true,
        seed: Some(1038),
        ..test_args()
    };
    let mut app = build_headless_app(&args).expect("app should build");
    run(&mut app, 60);
    let frayed = scan_uuid_named(&mut app, DIFF_FRAYED);
    scan_from_alongside(&mut app, 600.0, &frayed);

    let mut q = app
        .world_mut()
        .query_filtered::<&ShipSystemBlackboards, With<LocalShip>>();
    let blackboards = q
        .iter(app.world())
        .next()
        .expect("the local ship publishes");
    let bb: DossierBlackboard = match blackboards.0.get(&dossier_blackboard_key()) {
        Some(SystemBlackboard::Dossiers(bb)) => bb.clone(),
        other => panic!("expected a dossier blackboard, got {other:?}"),
    };
    let rung = bb
        .subjects
        .iter()
        .find(|d| d.name == DIFF_FRAYED)
        .expect("the rung is a subject through the infrastructure door");

    // What the crew were HANDED, then what they FOUND, in the order they got
    // them, each under its own provenance.
    assert_eq!(
        rung.evidence
            .iter()
            .map(|e| (e.text.as_str(), e.provenance.as_str()))
            .collect::<Vec<_>>(),
        vec![(DIFF_CLAIM, "briefing"), (DIFF_FINDING, "records")],
        "the crew can say which of these two they were told and which they worked out"
    );

    // And the machine-readable half of the same claim, as a fact row off the
    // live track: the standard the file says this rung meets, saying it does not.
    assert_eq!(
        rung.facts
            .iter()
            .map(|f| f.label.as_str())
            .collect::<Vec<_>>(),
        vec![
            FACT_CONDITION,
            "world.probe_scandiff.threshold.certified_load.label"
        ],
        "the panel shows the condition and the recorded standard beside it"
    );
    assert!(
        matches!(rung.facts[1].value, DossierValue::Flag(false)),
        "the standard the record claims is NOT met: {:?}",
        rung.facts[1].value
    );
}

/// The two rows the operator's own file puts on Ladder B's dossier at world
/// load — what the crew are HANDED, before anybody looks at anything.
const SKYWAY_RECORD: [&str; 2] = [
    "world.falling_skyway.evidence.ladder_b_record_inspection",
    "world.falling_skyway.evidence.ladder_b_record_reinforced",
];

/// Where `name` is standing right now.
fn skyway_position(app: &mut bevy::prelude::App, name: &str) -> bevy::prelude::Vec3 {
    use project_phoenix::entities::spawner::EntityName;
    let mut q = app.world_mut().query::<(&EntityName, &Transform)>();
    q.iter(app.world())
        .find(|(n, _)| n.0 == name)
        .map(|(_, t)| t.translation)
        .unwrap_or_else(|| panic!("{name} is not in this world"))
}

/// Fly to Ladder Depot B, take a science scan of it, and come back.
///
/// The whole beat, through the two real doors: helm closes the range (moved by
/// hand here for `skyway_at_act_two`'s stated reason — flying is not this test's
/// subject), and the reading is asked for through the ordinary admitted
/// `ScanTarget` path on the `sensors` system. Nothing in this helper touches an
/// evidence log, a flag or a dossier; everything that follows is the scenario's.
fn skyway_scan_ladder_b(app: &mut bevy::prelude::App, ship: bevy::prelude::Entity) {
    let station = {
        let physics = app.world().get::<ShipPhysics>(ship).expect("a ship");
        bevy::prelude::Vec3::new(physics.x, physics.y, physics.z)
    };
    let rung = skyway_position(app, SKYWAY_DEPOT_B);
    skyway_move(app, ship, rung + bevy::prelude::Vec3::new(60.0, 0.0, 0.0));
    run(app, 2);

    let uuid = scan_uuid_named(app, SKYWAY_DEPOT_B);
    ask_for_scan(app, &uuid);
    // The mirrored flag's one-tick bridge into the trigger pipeline, then the
    // handler's own effects.
    run(app, 4);

    skyway_move(app, ship, station);
    run(app, 2);
}

/// **Issue #1038 in the scenario, AC1/AC4/AC5 — and the arc it opens.**
///
/// The crew are handed the operator's file on Ladder B with the mission and go
/// and look at the rung. What comes back cannot carry what the file says it was
/// signed off to carry, and the crew write that down: an entry under RECORDS
/// provenance on the rung's own sheet, and a campaign flag a later mission can
/// read.
///
/// The last assertion is what the evidence is FOR. #1036's committee settle for
/// two pieces of ground out of three, and one of the three is the crew having
/// already read the file — so a survey saves the captain a promise. The
/// negotiation reads the evidence log itself rather than a flag beside it, which
/// is why this route lights that line without #1036 knowing it exists.
#[test]
fn scanning_ladder_b_against_its_own_maintenance_record_opens_the_evidence_route() {
    let (mut app, ship) = skyway_at_act_two();
    let rung = scan_uuid_named(&mut app, SKYWAY_DEPOT_B);

    // Before: the paperwork is on the sheet, and nothing else is.
    assert_eq!(
        diff_file(&app, &rung)
            .iter()
            .map(|(text, provenance)| (text.as_str(), provenance.as_str()))
            .collect::<Vec<_>>(),
        vec![
            (SKYWAY_RECORD[0], "briefing"),
            (SKYWAY_RECORD[1], "briefing"),
        ],
        "a competent crew can read the operator's claim before they act on it"
    );
    assert_eq!(skyway_flag(&app, "skyway_records_diff_found"), 0);

    skyway_scan_ladder_b(&mut app, ship);

    assert_eq!(
        skyway_flag(&app, "skyway_records_diff_found"),
        1,
        "the campaign flag a later mission reads: THIS crew found it"
    );
    assert_eq!(
        diff_file(&app, &rung)
            .iter()
            .map(|(text, provenance)| (text.as_str(), provenance.as_str()))
            .collect::<Vec<_>>(),
        vec![
            (SKYWAY_RECORD[0], "briefing"),
            (SKYWAY_RECORD[1], "briefing"),
            (SKYWAY_FILE, "records"),
        ],
        "…and the finding sits under the claim it contradicts, filed as the \
         records comparison it is"
    );

    // The arc. #1036's committee offer a line that only exists for a crew who
    // have read the file, and it is the evidence log they ask — so this route
    // opens it without the negotiation knowing this slice exists.
    skyway_pick(
        &mut app,
        SKYWAY_COMMITTEE,
        "world.falling_skyway.comms.listen",
    );
    assert!(
        skyway_options(&skyway_open_node(&app, SKYWAY_COMMITTEE))
            .contains(&"world.falling_skyway.comms.show_file".to_string()),
        "the committee's terms offer the line the survey earned: {:?}",
        skyway_options(&skyway_open_node(&app, SKYWAY_COMMITTEE))
    );
}

/// **AC6/AC7's control run.** The same mission, played by a crew who never
/// pointed anything at the rung.
///
/// Nothing is written — no finding, no flag — and the difference is not that the
/// mission stalls. The act still opened, the committee are still talking, and
/// the settlement is still there to be reached: it costs BOTH promises instead
/// of one, which is what "changes the available endings rather than blocking
/// progress" means in this scenario.
#[test]
fn a_crew_who_never_scan_the_rung_find_nothing_and_are_blocked_by_nothing() {
    let (mut app, _ship) = skyway_at_act_two();
    let rung = scan_uuid_named(&mut app, SKYWAY_DEPOT_B);

    assert_eq!(
        skyway_flag(&app, "skyway_records_diff_found"),
        0,
        "no campaign state is written by a crew who did not do the work"
    );
    assert_eq!(
        skyway_flag(
            &app,
            &project_phoenix::science::scanned_flag("depot_ladder_b")
        ),
        0,
        "precondition: nobody read the rung"
    );
    assert_eq!(
        diff_file(&app, &rung)
            .iter()
            .map(|(text, _)| text.as_str())
            .collect::<Vec<_>>(),
        vec![SKYWAY_RECORD[0], SKYWAY_RECORD[1]],
        "the operator's claim is still on the sheet — it is the FINDING that is \
         absent, and the crew are looking at an unchallenged document"
    );

    // The mission runs on. The evidence line is gone, and the two promises that
    // reach the same settlement are not.
    skyway_pick(
        &mut app,
        SKYWAY_COMMITTEE,
        "world.falling_skyway.comms.listen",
    );
    let options = skyway_options(&skyway_open_node(&app, SKYWAY_COMMITTEE));
    assert!(
        !options.contains(&"world.falling_skyway.comms.show_file".to_string()),
        "nothing to show them: {options:?}"
    );
    assert!(
        options.contains(&"world.falling_skyway.comms.promise_passage".to_string())
            && options.contains(&"world.falling_skyway.comms.promise_records".to_string()),
        "…and the road to the settlement is open, at a price: {options:?}"
    );

    skyway_pick(
        &mut app,
        SKYWAY_COMMITTEE,
        "world.falling_skyway.comms.promise_passage",
    );
    skyway_pick(
        &mut app,
        SKYWAY_COMMITTEE,
        "world.falling_skyway.comms.promise_records",
    );
    skyway_pick(
        &mut app,
        SKYWAY_COMMITTEE,
        "world.falling_skyway.comms.call_the_vote",
    );
    assert_eq!(
        skyway_flag(&app, "skyway_settled_by_negotiation"),
        1,
        "the offer is on the floor"
    );

    // The floor votes on #1036's own clock. What matters here is only that it
    // votes at all for a crew who found nothing.
    run(&mut app, ticks_for_sim_seconds(27.0, SKYWAY_DT));
    assert_eq!(
        skyway_flag(&app, "strike_resolved"),
        1,
        "the strike ends for a crew who never found the diff — it cost them the \
         promise the evidence would have saved, and nothing else"
    );
}

/// The contradiction is visible on the panel a crew actually read, in both
/// halves: the recorded claim as a gathered BRIEFING row, and the standard it
/// claims as a live condition fact off #1025's own published snapshot.
#[test]
fn the_ladder_b_panel_shows_the_recorded_standard_failing_beside_the_claim() {
    use project_phoenix::dossier::{dossier_blackboard_key, FACT_CONDITION};
    use project_phoenix::messages::{DossierBlackboard, DossierValue, SystemBlackboard};
    use project_phoenix::server_app::ShipSystemBlackboards;

    let (mut app, ship) = skyway_at_act_two();
    skyway_scan_ladder_b(&mut app, ship);

    let mut q = app
        .world_mut()
        .query_filtered::<&ShipSystemBlackboards, With<LocalShip>>();
    let blackboards = q
        .iter(app.world())
        .next()
        .expect("the local ship publishes");
    let bb: DossierBlackboard = match blackboards.0.get(&dossier_blackboard_key()) {
        Some(SystemBlackboard::Dossiers(bb)) => bb.clone(),
        other => panic!("expected a dossier blackboard, got {other:?}"),
    };
    let rung = bb
        .subjects
        .iter()
        .find(|d| d.name == SKYWAY_DEPOT_B)
        .expect("Ladder Depot B is a subject through the infrastructure door");

    let labels: Vec<&str> = rung.facts.iter().map(|f| f.label.as_str()).collect();
    assert!(
        labels.contains(&FACT_CONDITION)
            && labels.contains(&"world.falling_skyway.threshold.certified_load.label"),
        "the panel carries the condition and the standard the record claims: {labels:?}"
    );
    let standard = rung
        .facts
        .iter()
        .find(|f| f.label == "world.falling_skyway.threshold.certified_load.label")
        .expect("the recorded standard is a row");
    assert!(
        matches!(standard.value, DossierValue::Flag(false)),
        "34 of 100 cannot be a rung signed off at the certified load standard: {:?}",
        standard.value
    );
    assert_eq!(
        rung.evidence
            .iter()
            .map(|e| (e.text.as_str(), e.provenance.as_str()))
            .collect::<Vec<_>>(),
        vec![
            (SKYWAY_RECORD[0], "briefing"),
            (SKYWAY_RECORD[1], "briefing"),
            (SKYWAY_FILE, "records"),
        ],
        "what they were told, then what they worked out, in that order"
    );
}

// ── The worker who corroborates the record (issue #1039, parent #852) ────────

/// The witness's own hull — the sender whose thread is the whole slice.
const SKYWAY_RIGGER: &str = "world.falling_skyway.entity.rigger_tacket.name";
/// What she says, filed under DIALOGUE provenance onto the rung's sheet.
const SKYWAY_ACCOUNT: &str = "world.falling_skyway.evidence.ladder_b_worker_account";

/// Whether the rigger has ever been on the channel at all — the read the two
/// negative runs are built on, where the claim is that nothing was ever sent.
fn rigger_called(app: &bevy::prelude::App) -> bool {
    !skyway_messages(app, SKYWAY_RIGGER).is_empty()
}

/// Ladder B's fact sheet as `(text, provenance)` pairs, oldest first — the panel
/// payload, read through the same helper #1038's tests use.
fn ladder_b_sheet(app: &mut bevy::prelude::App) -> Vec<(String, String)> {
    let rung = scan_uuid_named(app, SKYWAY_DEPOT_B);
    diff_file(app, &rung)
}

/// Talk the committee round to a vote with whatever ground the crew already
/// hold. A crew carrying the maintenance file settle on one promise; a crew
/// carrying nothing give both. Either way the strike ends by NEGOTIATION, which
/// is the gate this slice reads — and which of the two roads got there is
/// #1036's business rather than this one's.
fn skyway_negotiate_to_a_vote(app: &mut bevy::prelude::App) {
    skyway_pick(app, SKYWAY_COMMITTEE, "world.falling_skyway.comms.listen");
    let options = skyway_options(&skyway_open_node(app, SKYWAY_COMMITTEE));
    if options.contains(&"world.falling_skyway.comms.show_file".to_string()) {
        skyway_pick(
            app,
            SKYWAY_COMMITTEE,
            "world.falling_skyway.comms.show_file",
        );
        skyway_pick(
            app,
            SKYWAY_COMMITTEE,
            "world.falling_skyway.comms.promise_passage",
        );
    } else {
        skyway_pick(
            app,
            SKYWAY_COMMITTEE,
            "world.falling_skyway.comms.promise_passage",
        );
        skyway_pick(
            app,
            SKYWAY_COMMITTEE,
            "world.falling_skyway.comms.promise_records",
        );
    }
    skyway_pick(
        app,
        SKYWAY_COMMITTEE,
        "world.falling_skyway.comms.call_the_vote",
    );
}

/// **Issue #1039, AC1/AC3/AC4/AC5/AC6/AC7 — the beat end to end.**
///
/// A crew who went and read the rung and then talked the strike down get one of
/// the people off that rung on an open channel, and what she says lands on the
/// SAME fact sheet as the document she is talking about, under a different
/// provenance. They finish able to say both what the structure is and who knew,
/// which is the sentence the confrontation is built on and the reason it unlocks
/// here and nowhere else.
///
/// The order is asserted causally throughout: she is silent while either gate is
/// open, and the beat she calls on is the one that closes the second.
#[test]
fn a_worker_corroborates_the_record_for_a_crew_who_read_it_and_talked_them_down() {
    use project_phoenix::core::messages::ObjectiveStatus;
    use project_phoenix::world::commitments::CommitmentState;
    use project_phoenix::world::server::WorldContentRuntime;

    let (mut app, ship) = skyway_at_act_two();

    // NEITHER GATE HELD YET, and she is not on the channel.
    assert_eq!(skyway_flag(&app, "skyway_records_diff_found"), 0);
    assert_eq!(skyway_flag(&app, "skyway_settled_by_negotiation"), 0);
    assert!(
        !rigger_called(&app),
        "nobody calls a crew who have not done anything yet"
    );

    // ── The evidence gate, on its own: the crew go and read the rung ─────────
    skyway_scan_ladder_b(&mut app, ship);
    assert_eq!(
        skyway_flag(&app, "skyway_records_diff_found"),
        1,
        "precondition: #1038's finding is on the sheet"
    );
    run(&mut app, ticks_for_sim_seconds(3.0, SKYWAY_DT));
    assert!(
        !rigger_called(&app),
        "ONE gate is not the gate: the strike is still on, and nobody on that \
         picket is talking to a destroyer about anything yet"
    );

    // ── The settlement gate: the floor carries the vote ──────────────────────
    skyway_negotiate_to_a_vote(&mut app);
    assert_eq!(skyway_flag(&app, "skyway_settled_by_negotiation"), 1);
    assert_eq!(
        skyway_flag(&app, "skyway_forced_open"),
        0,
        "precondition: nobody cleared that picket for them"
    );

    // AC1: the contact exists, and she is a real sender on a real hull rather
    // than a voice — the pick below is range-gated against where she is sitting.
    assert!(
        rigger_called(&app),
        "with both halves in place she calls, on the beat the second one lands"
    );
    let hail = skyway_open_node(&app, SKYWAY_RIGGER);
    assert_eq!(hail.body, "world.falling_skyway.comms.rigger_hails");
    assert_eq!(
        skyway_options(&hail),
        vec![
            "world.falling_skyway.comms.rigger_ask".to_string(),
            "world.falling_skyway.comms.rigger_later".to_string(),
        ],
        "the ask is index 0 — what a backfilled Tactical seat does with a thread \
         the crew already earned"
    );

    // The sheet before she speaks: what they were told, then what they worked
    // out. #1038's three rows and nothing else.
    assert_eq!(
        ladder_b_sheet(&mut app)
            .iter()
            .map(|(t, p)| (t.as_str(), p.as_str()))
            .collect::<Vec<_>>(),
        vec![
            (SKYWAY_RECORD[0], "briefing"),
            (SKYWAY_RECORD[1], "briefing"),
            (SKYWAY_FILE, "records"),
        ],
    );

    // ── AC4: the corroboration ───────────────────────────────────────────────
    skyway_pick(
        &mut app,
        SKYWAY_RIGGER,
        "world.falling_skyway.comms.rigger_ask",
    );

    // AC6: campaign state — the mirror of `skyway_worker_corroboration_closed`,
    // so a later mission reads a fact on either branch rather than an absence.
    assert_eq!(skyway_flag(&app, "skyway_worker_corroboration_obtained"), 1);
    assert_eq!(skyway_flag(&app, "skyway_worker_corroboration_closed"), 0);

    // AC4 proper: a FOURTH row on the same rung's sheet, visibly distinct from
    // the records comparison above it. One panel, one subject, and the crew can
    // say which of the four they were handed, which they worked out, and which
    // somebody told them to their face.
    assert_eq!(
        ladder_b_sheet(&mut app)
            .iter()
            .map(|(t, p)| (t.as_str(), p.as_str()))
            .collect::<Vec<_>>(),
        vec![
            (SKYWAY_RECORD[0], "briefing"),
            (SKYWAY_RECORD[1], "briefing"),
            (SKYWAY_FILE, "records"),
            (SKYWAY_ACCOUNT, "dialogue"),
        ],
        "the account is filed under the RUNG — a finding goes under what it is \
         about, not under who said it — and under its own provenance"
    );

    // ── AC5: what two entries on one sheet unlock ────────────────────────────
    assert_eq!(
        skyway_flag(&app, "skyway_confront_unlocked"),
        1,
        "the records comparison AND somebody's word: the confrontation is on the \
         table for the Act-3 scene"
    );
    assert_eq!(
        objective_status(&app, "obj-a2-corroborate"),
        ObjectiveStatus::Completed,
        "the optional objective #1036 posted goes GREEN — the mirror of the red \
         one the force-open produces"
    );
    assert_eq!(
        objective_status_opt(&app, "obj-a3-confront"),
        Some(ObjectiveStatus::Active),
        "and the crew are told what they have earned, on the panel, without this \
         slice authoring #1043's scene"
    );

    // Her own ask, answered. The promise is made to HER and not to the
    // committee: a captain who puts her account to the operator has to know
    // whose name is on it.
    let account = skyway_open_node(&app, SKYWAY_RIGGER);
    assert_eq!(account.body, "world.falling_skyway.comms.rigger_account");
    skyway_pick(
        &mut app,
        SKYWAY_RIGGER,
        "world.falling_skyway.comms.rigger_protect",
    );
    {
        let ledger = &app.world().resource::<WorldContentRuntime>().commitments;
        let promise = ledger
            .get("skyway_protect_witness")
            .expect("the captain's promise to the witness is on the books");
        assert_eq!(promise.state, CommitmentState::Open);
        assert_eq!(promise.made_to, SKYWAY_RIGGER);
    }
    assert_eq!(skyway_flag(&app, "skyway_witness_unprotected"), 0);
}

/// **AC2 — the force-open path, and the closure said out loud.**
///
/// The SAME crew, holding the SAME finding, who let Havelock clear the line
/// instead of talking. Nobody calls them, and the only difference between this
/// run and the one above is one dialogue pick.
///
/// The closure is asserted as legible rather than merely absent: #1036 fails the
/// objective on the crew's own list, the committee say why on the channel they
/// will not answer again, and the campaign flag carries it forward. This slice
/// deliberately adds no second announcement — the route it would announce is one
/// the crew were already told they had lost — so what is asserted here is that
/// all three of those still hold with the corroboration authored behind them.
#[test]
fn forcing_the_picket_open_leaves_nobody_willing_to_corroborate_and_says_so() {
    use project_phoenix::core::messages::ObjectiveStatus;

    let (mut app, ship) = skyway_at_act_two();

    // The evidence gate is SATISFIED, so the only thing separating this run from
    // the one above is how the dispute ended.
    skyway_scan_ladder_b(&mut app, ship);
    assert_eq!(skyway_flag(&app, "skyway_records_diff_found"), 1);

    skyway_pick(
        &mut app,
        SKYWAY_CUTTER,
        "world.falling_skyway.comms.force_now",
    );
    // The dispute ENDS. This is not the run where nothing happened.
    run(&mut app, ticks_for_sim_seconds(10.0, SKYWAY_DT));
    assert_eq!(skyway_flag(&app, "strike_resolved"), 1);
    assert_eq!(skyway_flag(&app, "skyway_settled_by_negotiation"), 0);

    // Long enough that a slow route would have shown up.
    run(&mut app, ticks_for_sim_seconds(20.0, SKYWAY_DT));
    assert!(
        !rigger_called(&app),
        "the workers watched security come over the rail; none of them is getting \
         on a channel to help afterwards"
    );
    assert_eq!(skyway_flag(&app, "skyway_worker_corroboration_obtained"), 0);
    assert_eq!(
        skyway_flag(&app, "skyway_confront_unlocked"),
        0,
        "half a case is not a case: the crew hold the record and nobody's word"
    );
    assert_eq!(
        objective_status_opt(&app, "obj-a3-confront"),
        None,
        "and no objective promises them a confrontation they cannot have"
    );

    // LEGIBLY, not silently — all three of #1036's surfaces, with this slice's
    // content in the world.
    assert_eq!(skyway_flag(&app, "skyway_worker_corroboration_closed"), 1);
    assert_eq!(
        objective_status(&app, "obj-a2-corroborate"),
        ObjectiveStatus::Failed,
        "the route goes red on the crew's own list rather than latching quietly"
    );
    assert!(
        skyway_messages(&app, SKYWAY_COMMITTEE)
            .iter()
            .any(|m| m.body == "world.falling_skyway.comms.committee_signs_off"),
        "and the workers say why, on the channel they will not answer again"
    );
    // Ladder B's sheet carries what a reading can prove and nothing a person
    // said, which is the whole cost of this branch in one assertion.
    assert!(
        ladder_b_sheet(&mut app)
            .iter()
            .all(|(text, _)| text != SKYWAY_ACCOUNT),
        "nothing on the rung's file came from anybody's mouth"
    );
}

/// **AC3 — negotiated, and nothing to ask about.**
///
/// The crew talk the strike down without ever pointing anything at the rung.
/// Nobody calls: a witness corroborating a document the crew have not read is an
/// exposition scene, and there is no question to put to her.
///
/// Then the other half of the same claim, and it is what separates this branch
/// from the force-open one: the route is NOT shut. The objective is still on the
/// list, no campaign flag says otherwise, and the moment the crew go and do the
/// survey — with nothing settled on that beat, nobody picking anything on it,
/// and no timer that knows a witness exists — she calls. Which is the
/// registration working from its other side.
#[test]
fn a_crew_who_settled_the_strike_but_never_read_the_rung_hear_nothing_yet() {
    use project_phoenix::core::messages::ObjectiveStatus;

    let (mut app, ship) = skyway_at_act_two();

    skyway_negotiate_to_a_vote(&mut app);
    assert_eq!(skyway_flag(&app, "skyway_settled_by_negotiation"), 1);
    run(&mut app, ticks_for_sim_seconds(25.0, SKYWAY_DT));
    assert_eq!(
        skyway_flag(&app, "strike_resolved"),
        1,
        "precondition: the rung is moving again, and it was talked into moving"
    );

    assert_eq!(
        skyway_flag(&app, "skyway_records_diff_found"),
        0,
        "precondition: nobody read the rung"
    );
    assert!(
        !rigger_called(&app),
        "she has nothing to corroborate — the crew are holding an unchallenged \
         document"
    );
    assert_eq!(skyway_flag(&app, "skyway_worker_corroboration_obtained"), 0);
    assert_eq!(skyway_flag(&app, "skyway_confront_unlocked"), 0);
    assert_eq!(
        objective_status_opt(&app, "obj-a3-confront"),
        None,
        "nothing is unlocked by half a case"
    );

    // NOT SHUT, which is the difference from the other negative run.
    assert_eq!(skyway_flag(&app, "skyway_worker_corroboration_closed"), 0);
    assert_eq!(
        objective_status(&app, "obj-a2-corroborate"),
        ObjectiveStatus::Active,
        "the crew can still go and do the work; nothing has told them otherwise"
    );

    // ── The other order ──────────────────────────────────────────────────────
    // The settlement is a long way behind them. The only new fact is a reading
    // of a rung, and it is what puts her on the channel — so the pair of
    // conditions is a pair rather than a sequence.
    skyway_scan_ladder_b(&mut app, ship);
    assert_eq!(skyway_flag(&app, "skyway_records_diff_found"), 1);
    assert!(
        rigger_called(&app),
        "the second half arriving second reaches the same beat"
    );

    skyway_pick(
        &mut app,
        SKYWAY_RIGGER,
        "world.falling_skyway.comms.rigger_ask",
    );
    assert_eq!(skyway_flag(&app, "skyway_worker_corroboration_obtained"), 1);
    assert_eq!(skyway_flag(&app, "skyway_confront_unlocked"), 1);
    assert_eq!(
        ladder_b_sheet(&mut app)
            .iter()
            .map(|(t, p)| (t.as_str(), p.as_str()))
            .collect::<Vec<_>>(),
        vec![
            (SKYWAY_RECORD[0], "briefing"),
            (SKYWAY_RECORD[1], "briefing"),
            (SKYWAY_FILE, "records"),
            (SKYWAY_ACCOUNT, "dialogue"),
        ],
    );
}

/// Refusing the witness cover costs the crew nothing they already have, and it
/// is written down. A captain who would not give her the promise is a fact about
/// this crew, and #1043's scene should be able to read it.
#[test]
fn refusing_the_witness_cover_keeps_the_corroboration_and_records_the_refusal() {
    use project_phoenix::world::server::WorldContentRuntime;

    let (mut app, ship) = skyway_at_act_two();
    skyway_scan_ladder_b(&mut app, ship);
    skyway_negotiate_to_a_vote(&mut app);

    skyway_pick(
        &mut app,
        SKYWAY_RIGGER,
        "world.falling_skyway.comms.rigger_ask",
    );
    skyway_pick(
        &mut app,
        SKYWAY_RIGGER,
        "world.falling_skyway.comms.rigger_no_promise",
    );

    assert_eq!(skyway_flag(&app, "skyway_witness_unprotected"), 1);
    assert_eq!(
        skyway_flag(&app, "skyway_confront_unlocked"),
        1,
        "she said it before she asked for anything, so refusing her does not \
         un-say it"
    );
    {
        let ledger = &app.world().resource::<WorldContentRuntime>().commitments;
        assert!(
            ledger.get("skyway_protect_witness").is_none(),
            "and nothing the captain did not promise is on the books"
        );
    }
}

/// **`probe_corroborate.toml` — the whole matrix in one run.**
///
/// Five sites, authored line for line the same, differing only in how their
/// dispute ended and in what their crew had read. The world's own header carries
/// the timeline every assertion below is read against.
///
/// * BOTH against FORCED is the settlement gate. Identical records, identical
///   witness, identical everything — and the only difference is which flag the
///   dispute ended on.
/// * BOTH against UNHEARD is the evidence gate. Identical settlements, and the
///   only difference is whether anything was ever filed about the rung.
/// * BOTH against LATE is the ORDER, and it is the corner a scripted reveal
///   could not produce: Late's witness calls on a beat whose only new fact is a
///   document, with nothing settled and nobody choosing anything.
///
/// The unlock is read at the end for all five, so its three refusals are
/// distinguished too: Forced holds a records comparison and nobody's word, while
/// Unheard and Quiet hold neither.
#[test]
fn corroboration_opens_on_two_gates_and_on_neither_of_them_alone() {
    let dt = 1.0 / 60.0;
    let args = HeadlessArgs {
        world_path: "assets/worlds/probe_corroborate.toml".into(),
        ship_path: "assets/entities/alliance_destroyer.toml".into(),
        dt,
        max_ticks: ticks_for_sim_seconds(15.0, dt),
        deterministic: true,
        seed: Some(1039),
        ..test_args()
    };
    let mut app = build_headless_app(&args).expect("app should build");

    // Sampled tick by tick, so the ORDER below is causal rather than arithmetic:
    // what is asserted is which fact was in the world when each thread opened,
    // never which frame it happened on.
    let mut called_both: Option<u64> = None;
    let mut called_late: Option<u64> = None;
    let mut late_filed: Option<u64> = None;
    for tick in 0..args.max_ticks {
        run(&mut app, 1);
        if called_both.is_none() && diff_flag(&app, "called_both") == 1 {
            called_both = Some(tick);
        }
        if called_late.is_none() && diff_flag(&app, "called_late") == 1 {
            called_late = Some(tick);
        }
        if late_filed.is_none() && diff_flag(&app, "late_filed") == 1 {
            late_filed = Some(tick);
        }
    }

    // The world ran its whole clock: three sweeps and one unlock pass.
    assert_eq!(diff_flag(&app, "records_filed"), 1);
    assert_eq!(diff_flag(&app, "ground_set"), 1);
    assert_eq!(diff_flag(&app, "swept"), 3, "asked three times, not once");
    assert_eq!(diff_flag(&app, "unlocks_read"), 1);

    // ── Which witnesses called ───────────────────────────────────────────────
    assert_eq!(diff_flag(&app, "called_both"), 1);
    assert_eq!(diff_flag(&app, "called_late"), 1);
    assert_eq!(
        diff_flag(&app, "called_forced"),
        0,
        "the dispute ended, and it ended the other way: nobody is talking"
    );
    assert_eq!(
        diff_flag(&app, "called_unheard"),
        0,
        "talked down, and nothing on the sheet to ask about"
    );
    assert_eq!(diff_flag(&app, "called_quiet"), 0);

    // ── The order, and what it proves ────────────────────────────────────────
    let both_at = called_both.expect("the both-halves witness called");
    let late_at = called_late.expect("the late-record witness called");
    let filed_at = late_filed.expect("the late record was filed");
    assert!(
        both_at < filed_at,
        "the first site opened on a settlement, before the late document existed \
         ({both_at} vs {filed_at})"
    );
    assert!(
        filed_at < late_at,
        "and the second opened AFTER its document landed, on a beat nothing was \
         settled and nobody chose anything on ({filed_at} vs {late_at})"
    );

    // ── The picks the backfilled Comms officer made ──────────────────────────
    assert_eq!(diff_flag(&app, "corroborated_both"), 1);
    assert_eq!(diff_flag(&app, "corroborated_late"), 1);
    for silent in ["forced", "unheard", "quiet"] {
        assert_eq!(
            diff_flag(&app, &format!("corroborated_{silent}")),
            0,
            "no thread, no pick, no account from the {silent} site"
        );
    }

    // ── The fact sheets ──────────────────────────────────────────────────────
    let both_sheet = {
        let uuid = scan_uuid_named(&mut app, "world.probe_corroborate.entity.rung_both.name");
        diff_file(&app, &uuid)
    };
    assert_eq!(
        both_sheet,
        vec![
            (
                "world.probe_corroborate.evidence.record_both".to_string(),
                "records".to_string()
            ),
            (
                "world.probe_corroborate.evidence.account_both".to_string(),
                "dialogue".to_string()
            ),
        ],
        "one sheet, two provenances: what a reading proved and what somebody said"
    );
    let forced_sheet = {
        let uuid = scan_uuid_named(&mut app, "world.probe_corroborate.entity.rung_forced.name");
        diff_file(&app, &uuid)
    };
    assert_eq!(
        forced_sheet,
        vec![(
            "world.probe_corroborate.evidence.record_forced".to_string(),
            "records".to_string()
        )],
        "the forced site holds exactly what an instrument can prove, and nothing \
         anybody was willing to say"
    );
    for empty in ["rung_unheard", "rung_quiet"] {
        let name = format!("world.probe_corroborate.entity.{empty}.name");
        let uuid = scan_uuid_named(&mut app, &name);
        assert!(
            diff_file(&app, &uuid).is_empty(),
            "{empty} was never read and never spoken about"
        );
    }

    // ── The unlock, read off the sheets rather than off the picks ────────────
    assert_eq!(diff_flag(&app, "unlocked_both"), 1);
    assert_eq!(diff_flag(&app, "unlocked_late"), 1);
    assert_eq!(
        diff_flag(&app, "unlocked_forced"),
        0,
        "a records comparison on its own does not unlock it"
    );
    assert_eq!(diff_flag(&app, "unlocked_unheard"), 0);
    assert_eq!(diff_flag(&app, "unlocked_quiet"), 0);
}

// ── Issue #1041: the tactical restraint lever, and the choice it enables ─────

const RESTRAINT: &str = "assets/worlds/probe_restraint.toml";

const R_ENFORCER: &str = "world.probe_restraint.entity.enforcer.name";
const R_DISABLED: &str = "world.probe_restraint.entity.claimant_disabled.name";
const R_DESTROYED: &str = "world.probe_restraint.entity.claimant_destroyed.name";

fn restraint_args(dt: f64, seconds: f64) -> HeadlessArgs {
    HeadlessArgs {
        world_path: RESTRAINT.into(),
        dt,
        max_ticks: ticks_for_sim_seconds(seconds, dt),
        deterministic: true,
        seed: Some(42),
        ..test_args()
    }
}

/// The sim time of the named ship's last shot, or `None` if it has never fired.
///
/// Read off `RecentCombatActivity`, the same per-entity record the captain's own
/// stand-down policy reads. A test that watched a beam component would be
/// watching one weapon family; this watches "did this hull discharge anything".
fn last_shot_secs(app: &mut bevy::prelude::App, name: &str) -> Option<f32> {
    app.world_mut()
        .query::<(
            &project_phoenix::entities::spawner::EntityName,
            &project_phoenix::ship::combat_activity::RecentCombatActivity,
        )>()
        .iter(app.world())
        .find(|(entity_name, _)| entity_name.0 == name)
        .and_then(|(_, activity)| activity.last_weapon_fired)
}

fn restraint_flag(app: &bevy::prelude::App, name: &str) -> bool {
    app.world()
        .resource::<project_phoenix::world::server::WorldContentRuntime>()
        .flags
        .flag(name)
}

fn restraint_counter(app: &bevy::prelude::App, name: &str) -> i64 {
    app.world()
        .resource::<project_phoenix::world::server::WorldContentRuntime>()
        .flags
        .counter(name)
}

fn is_in_world(app: &mut bevy::prelude::App, name: &str) -> bool {
    app.world_mut()
        .query::<&project_phoenix::entities::spawner::EntityName>()
        .iter(app.world())
        .any(|entity_name| entity_name.0 == name)
}

fn is_holding_fire(app: &mut bevy::prelude::App, name: &str) -> bool {
    app.world_mut()
        .query::<(
            &project_phoenix::entities::spawner::EntityName,
            &project_phoenix::ship::state::ShipWeaponsHold,
        )>()
        .iter(app.world())
        .find(|(entity_name, _)| entity_name.0 == name)
        .map(|(_, hold)| hold.0)
        .unwrap_or_else(|| panic!("{name} is not in the world"))
}

/// Put the hull the crew fly under a weapons hold, as the admitted
/// `SetWeaponsHold` command would leave it.
fn hold_the_local_ships_fire(app: &mut bevy::prelude::App) {
    let mut q = app
        .world_mut()
        .query_filtered::<&mut project_phoenix::ship::state::ShipWeaponsHold, With<LocalShip>>();
    let mut hold = q
        .single_mut(app.world_mut())
        .expect("the headless run flies one local ship");
    hold.0 = true;
}

/// **Issue #1041, the lever.** A weapons hold suppresses an always-armed hull's
/// fire, and releasing it gives the fire back.
///
/// Every claim is a BEFORE and an AFTER on the SAME hull, in one run, with
/// nothing else about the world moving: the picket stays hostile, stays in
/// range and keeps its target throughout. What changes between the samples is
/// one boolean, applied through the same state a captain's console writes.
///
/// The subject is a Harrow patrol boat deliberately. Its gun line authors
/// `min_alert_to_fire = 0` — always armed, no captain to call an alert — so it
/// is the hull a hold has to beat the hard way. An implementation that seeded a
/// plain `0.0` for a held ship would satisfy every Alliance hull and leave this
/// one shooting.
#[test]
fn a_weapons_hold_silences_an_always_armed_hull_and_releasing_it_gives_the_fire_back() {
    let dt = 1.0 / 60.0;
    let mut app = build_headless_app(&restraint_args(dt, 24.0)).expect("app should build");

    // ── Free: it shoots ─────────────────────────────────────────────────────
    //
    // Asserted first, and the whole test rests on it: a hull that never fired
    // would pass every "it did not fire" assertion below for the wrong reason.
    run(&mut app, ticks_for_sim_seconds(3.5, dt));
    let before_hold = last_shot_secs(&mut app, R_ENFORCER)
        .expect("the always-armed picket opens fire on its own");

    // ── Held: it stops ──────────────────────────────────────────────────────
    //
    // The order lands at t=4. Sampled well after it, and compared against the
    // instant of the last shot rather than against a shot count — the claim is
    // "nothing has been fired since", which is what a stopped gun means.
    run(&mut app, ticks_for_sim_seconds(3.5, dt));
    assert!(
        restraint_flag(&app, "enforcer_ordered_to_hold"),
        "precondition: the scenario has issued the hold"
    );
    assert!(is_holding_fire(&mut app, R_ENFORCER));
    let during_hold = last_shot_secs(&mut app, R_ENFORCER).expect("it fired before it was held");
    assert_eq!(
        during_hold, before_hold,
        "held, the picket has not discharged a weapon since the order — through its \
         OWN authored `fact(red_alert) >= param(min_alert_to_fire)` gate, with no new \
         doctrine vocabulary and no Rust branch on who is flying"
    );

    // ── Released: it shoots again ───────────────────────────────────────────
    //
    // The half that makes the middle sample mean something. A lever that could
    // not be released would be a ship that had disarmed itself.
    run(&mut app, ticks_for_sim_seconds(6.0, dt));
    assert!(restraint_flag(&app, "enforcer_released"));
    assert!(!is_holding_fire(&mut app, R_ENFORCER));
    let after_release = last_shot_secs(&mut app, R_ENFORCER).expect("still firing");
    assert!(
        after_release > during_hold,
        "released, the same hull is shooting again — {after_release} > {during_hold}"
    );
}

/// **Issue #1041, AC3.** The hold is readable by scenario script, and an
/// authored party reacts to it.
///
/// The reaction is chained off the MIRROR flag rather than off a Rust hook, the
/// shape issue #1035 established for `workforce.<id>.on_strike`: the component
/// stays authoritative, the flag is a mirror of it, and the scenario reads the
/// mirror. Nothing in `probe_restraint.toml` touches ship state.
#[test]
fn the_crews_own_hold_is_visible_to_the_scenario_and_something_answers_it() {
    let dt = 1.0 / 60.0;
    let mut app = build_headless_app(&restraint_args(dt, 24.0)).expect("app should build");

    // Before the order: no flag, and nobody has noticed anything. The named
    // picket the scenario DID order to hold at t=4 already carries its own
    // mirror, which is the second key this system writes.
    run(&mut app, ticks_for_sim_seconds(8.0, dt));
    assert!(!restraint_flag(&app, "weapons_hold.own_ship"));
    assert!(!restraint_flag(&app, "operator_saw_restraint"));
    assert!(
        restraint_flag(
            &app,
            &project_phoenix::ship::state::weapons_hold_flag(R_ENFORCER)
        ),
        "a NAMED ship mirrors under its authored name, which is the key a scenario \
         asking about one specific hull would use"
    );

    // The crew's captain calls the hold. Written as the state the admitted
    // `SetWeaponsHold` command produces rather than pushed as that command,
    // because a headless run has no console session to send one from — the
    // command path itself is pinned by `console::captain::server`'s own tests.
    // What is under test HERE is everything downstream of the state.
    hold_the_local_ships_fire(&mut app);
    run(&mut app, ticks_for_sim_seconds(1.0, dt));
    assert!(
        restraint_flag(&app, "weapons_hold.own_ship"),
        "the hull the crew fly mirrors under a ROLE key, because a world's player \
         ship is not required to declare a reference name — `falling_skyway.toml` \
         gives its own player entry an id and no name"
    );
    assert!(
        restraint_flag(&app, "operator_saw_restraint"),
        "an authored party reacted to the hold — through `on_flag_set`, which only \
         fires because the mirror emits a real transition event"
    );
    assert_eq!(
        restraint_counter(&app, "operator_restraint_notices"),
        1,
        "once, on the transition — not once per tick the hold is up"
    );
}

/// **Issue #1041, the choice.** Both branches of the disable-or-destroy
/// interaction, and the campaign flags each writes.
///
/// Two claimants spawned identical, taken out of the fight two different ways,
/// in one run — so the flags are read against a control rather than against an
/// expectation. Neither branch invents a combat state: DISABLED is a condition
/// track crossing the threshold that owns its gun mount (#1025), at which point
/// this issue's own lever makes the silence real; DESTROYED is the ordinary
/// `WorldEvent::Destroyed` chain (#1033).
#[test]
fn a_claimant_can_be_disabled_or_destroyed_and_each_writes_its_own_flags() {
    let dt = 1.0 / 60.0;
    let mut app = build_headless_app(&restraint_args(dt, 24.0)).expect("app should build");

    // Both are in the world, whole, and neither branch has been taken.
    run(&mut app, ticks_for_sim_seconds(12.0, dt));
    assert!(is_in_world(&mut app, R_DISABLED));
    assert!(is_in_world(&mut app, R_DESTROYED));
    assert!(!restraint_flag(&app, "claimant_disabled"));
    assert!(!restraint_flag(&app, "claimant_destroyed"));

    // ── DISABLED (t=14) ─────────────────────────────────────────────────────
    run(&mut app, ticks_for_sim_seconds(3.0, dt));
    assert!(
        restraint_flag(&app, "claimant_disabled"),
        "the condition crossing chained into the disable handler"
    );
    assert!(
        is_in_world(&mut app, R_DISABLED),
        "…and the claimant is STILL THERE. That is the whole point of the branch: it \
         is out of the fight and it is not dead."
    );
    assert!(
        is_holding_fire(&mut app, R_DISABLED),
        "…silenced through the restraint lever rather than through a new combat state"
    );
    assert!(
        restraint_flag(&app, "restraint_shown"),
        "the campaign records which way the crew took it out"
    );
    assert!(!restraint_flag(&app, "claimant_destroyed"));

    // ── DESTROYED (t=18) — the control ──────────────────────────────────────
    run(&mut app, ticks_for_sim_seconds(4.0, dt));
    assert!(restraint_flag(&app, "claimant_destroyed"));
    assert!(
        !is_in_world(&mut app, R_DESTROYED),
        "the other claimant is gone — same hull, same authored condition track, \
         different ending"
    );
    assert!(
        !restraint_flag(&app, "restraint_shown"),
        "…and the campaign's answer to 'how did you take it out?' moved with it. One \
         question, two answers, written by two handlers."
    );
    assert!(
        restraint_flag(&app, "claimant_disabled"),
        "the disabled claimant's own flag is untouched by its twin's ending"
    );
}

/// **Issue #1041, in the mission it was written for.** Falling Skyway's Havelock
/// picket arrives, arms nothing, and provokes nobody.
///
/// The restraint interaction is a CHOICE, which means a run in which the crew
/// make no choice must be indistinguishable from one in which the picket is not
/// there at all — no faction turns hostile, no branch is taken, and Act 1 runs
/// its own clock undisturbed. This is the guard for that: the picket is the only
/// armed hull the mission fields and the only way it can matter is if somebody
/// shoots at it first.
#[test]
fn falling_skyways_picket_sits_there_until_somebody_starts_something() {
    let dt = 1.0 / 60.0;
    let args = HeadlessArgs {
        world_path: "assets/worlds/falling_skyway.toml".into(),
        dt,
        max_ticks: ticks_for_sim_seconds(20.0, dt),
        deterministic: true,
        seed: Some(42),
        ..test_args()
    };
    let mut app = build_headless_app(&args).expect("app should build");
    run(&mut app, ticks_for_sim_seconds(20.0, dt));

    const PICKET: &str = "world.falling_skyway.entity.havelock_enforcer.name";
    assert!(is_in_world(&mut app, PICKET), "the picket is on station");
    assert!(
        !is_holding_fire(&mut app, PICKET),
        "…weapons-free, which is the state a hull nobody has ordered anything is in"
    );
    assert!(
        restraint_flag(&app, "havelock_enforcer_guns_online"),
        "its mount is online: the condition threshold arms UP, so `on_flag_cleared` \
         is a crossing the crew have to cause"
    );
    for untaken in [
        "picket_engaged",
        "havelock_enforcer_disabled",
        "havelock_enforcer_destroyed",
        "restraint_shown",
        "havelock_saw_restraint",
    ] {
        assert!(
            !restraint_flag(&app, untaken),
            "`{untaken}` is unset in a run where nobody chose anything — every one of \
             these flags is a consequence of a crew's decision, and an Act 1 that set \
             them by itself would be authoring the choice rather than offering it"
        );
    }
    assert!(
        last_shot_secs(&mut app, PICKET).is_none(),
        "and it has not fired a shot: the Harrow are neutral until provoked, which is \
         the premise the whole lever rests on"
    );
}
// ── Falling Skyway, Act 3: the collapse and the epilogue (issue #1040) ───────

/// The structure the act is about, by the name the world authors it under.
const SKYWAY_HEAD: &str = "world.falling_skyway.entity.skyhook.name";
/// The two craft the collapse handler spawns into the debris.
const SKYWAY_LIGHTER: &str = "world.falling_skyway.entity.head_lighter.name";
const SKYWAY_POD: &str = "world.falling_skyway.entity.head_pod.name";
/// The three warning rungs, in the order the ladder crosses them, paired with
/// the objective each one posts and the dialogue body each one speaks. Read as a
/// table by every assertion below, so "the warnings fire in order on all three
/// surfaces" is one loop rather than three copies of one.
const SKYWAY_WARNINGS: [(&str, &str, &str); 3] = [
    (
        "a3_warning_one",
        "obj-a3-tether-1",
        "world.falling_skyway.comms.head_warns_one",
    ),
    (
        "a3_warning_two",
        "obj-a3-tether-2",
        "world.falling_skyway.comms.head_warns_two",
    ),
    (
        "a3_warning_three",
        "obj-a3-tether-3",
        "world.falling_skyway.comms.head_warns_three",
    ),
];

/// One deadline as the CAPTAIN'S PANEL sees it — off the published blackboard
/// rather than off the runtime table, because "the crew can see it coming" is a
/// claim about the wire.
fn skyway_captain_deadline(
    app: &mut bevy::prelude::App,
    id: &str,
) -> project_phoenix::messages::DeadlineSnapshot {
    let mut q = app
        .world_mut()
        .query::<&project_phoenix::server_app::ShipSystemBlackboards>();
    q.iter(app.world())
        .find_map(|bbs| {
            bbs.0
                .values()
                .find_map(|bb| match bb {
                    project_phoenix::messages::SystemBlackboard::Captain(c) => Some(c),
                    _ => None,
                })
                .and_then(|c| c.deadlines.iter().find(|d| d.id == id).cloned())
        })
        .unwrap_or_else(|| panic!("the captain's countdown carries no '{id}'"))
}

/// The crew's own hull and its uuid.
fn skyway_crew_hull(app: &mut bevy::prelude::App) -> (bevy::prelude::Entity, String) {
    let ship = app
        .world_mut()
        .query_filtered::<bevy::prelude::Entity, With<LocalShip>>()
        .iter(app.world())
        .next()
        .expect("the crew's hull is in the world");
    let uuid = app
        .world()
        .get::<project_phoenix::entities::spawner::EntityUuid>(ship)
        .expect("the hull carries a uuid")
        .0
        .clone();
    (ship, uuid)
}

/// Queue one operation on the crew's hull through the queue a scripted
/// `ctx.effects.stabilise(…)` fills — the route every operations test in this
/// file uses, and the only place range, power and capability are decided.
fn skyway_start_op(
    app: &mut bevy::prelude::App,
    ship_uuid: &str,
    verb: project_phoenix::operations::OperationVerb,
    target: &str,
) {
    use project_phoenix::operations::PendingOperationStart;
    use project_phoenix::world::server::WorldContentRuntime;

    let target_uuid = app
        .world()
        .resource::<WorldContentRuntime>()
        .name_to_uuid
        .get(target)
        .cloned()
        .unwrap_or_else(|| panic!("{target} is not in this world"));
    app.world_mut()
        .resource_mut::<WorldContentRuntime>()
        .pending_operation_starts
        .push(PendingOperationStart {
            ship_uuid: ship_uuid.to_string(),
            verb,
            target_uuid,
        });
}

/// The floor crossing the world's authored strain rate reaches, derived from the
/// projection deadline rather than restated: `falling_skyway.toml`'s Act-3 band
/// authors `skyhook_failure_due` four seconds past it and says so. The tuning
/// pass (#1044) moves the deadline and this follows it.
fn skyway_projected_floor(app: &bevy::prelude::App) -> f64 {
    skyway_deadline_secs(app, "skyhook_failure_due") as f64 - 4.0
}

/// **Issue #1040, AC1/AC2/AC4/AC5/AC6/AC7.** Act 3 driven end to end with nobody
/// at the consoles: the head warns three times on three surfaces, crosses its
/// authored structural floor, is REMOVED FROM THE WORLD, and the mission carries
/// on into a survivor-rescue epilogue that reaches its own resolution.
///
/// The neglect branch is the one an unattended run produces, and that is the
/// honest default: opening an operation is a crew verb, so a backfilled bridge
/// never stabilises anything. The companion tests below drive the same act with
/// a crew that reacts, and the epilogue with a crew that tows.
#[test]
fn falling_skyway_act_3_warns_three_times_then_the_head_falls_into_a_playable_epilogue() {
    use project_phoenix::core::messages::ObjectiveStatus;
    use project_phoenix::world::server::WorldContentRuntime;

    let dt = SKYWAY_DT;
    let probe = build_headless_app(&skyway_args(dt, 1.0)).expect("the world must load");
    let projected = skyway_deadline_secs(&probe, "skyhook_failure_due") as f64;
    drop(probe);

    // The act's own clock decides the run length: the projection, plus the
    // epilogue's authored window, plus a margin. Lengthening Act 3 in the TOML
    // must not silently turn this into a test of an act that never finished.
    let args = skyway_args(dt, projected + 100.0);
    let mut app = build_headless_app(&args).expect("the scenario world must load and build");

    let mut first: std::collections::BTreeMap<String, f64> = Default::default();
    // What each surface said on the tick each warning fired. Sampled AT THE
    // EVENT rather than at the end of the run, because "the crew were told"
    // is a claim about the moment, and an inbox can be cleared afterwards.
    let mut at_warning: std::collections::BTreeMap<String, (String, usize, String, i64)> =
        Default::default();
    let mut condition_at: std::collections::BTreeMap<String, f32> = Default::default();
    let mut bodies_seen: std::collections::BTreeSet<String> = Default::default();

    for tick in 0..args.max_ticks {
        run(&mut app, 1);
        let sim_t = (tick + 1) as f64 * dt;

        for message in skyway_messages(&app, SKYWAY_HEAD) {
            bodies_seen.insert(message.body.clone());
        }

        for (counter, objective, _body) in SKYWAY_WARNINGS {
            if skyway_flag(&app, counter) > 0 && !first.contains_key(counter) {
                first.insert(counter.to_string(), sim_t);
                condition_at.insert(counter.to_string(), condition_of(&mut app, SKYWAY_HEAD));
                let snapshot = skyway_captain_deadline(&mut app, "skyhook_failure_due");
                at_warning.insert(
                    counter.to_string(),
                    (
                        format!("{:?}", objective_status_opt(&app, objective)),
                        skyway_messages(&app, SKYWAY_HEAD).len(),
                        snapshot.state.clone(),
                        snapshot.remaining_secs,
                    ),
                );
            }
        }
        for flag in [
            "a3_watch_open",
            "a3_floor_crossed",
            "a3_head_lost",
            "a3_epilogue_open",
            "a3_epilogue_resolved",
        ] {
            if skyway_flag(&app, flag) > 0 {
                first.entry(flag.to_string()).or_insert(sim_t);
            }
        }
        if !named_entity_present(&mut app, SKYWAY_HEAD) {
            first.entry("head_gone".to_string()).or_insert(sim_t);
        }
    }

    let at = |key: &str| -> f64 {
        *first
            .get(key)
            .unwrap_or_else(|| panic!("'{key}' never happened in this run: {first:?}"))
    };

    // ── AC2: three warnings, in ladder order, each on all three surfaces ──
    let mut previous = at("a3_watch_open");
    let mut previous_condition = f32::MAX;
    for (counter, objective, body) in SKYWAY_WARNINGS {
        let fired = at(counter);
        assert!(
            fired > previous,
            "the warnings fire IN ORDER — {counter} at {fired:.1} s must follow the beat \
             before it at {previous:.1} s: {first:?}"
        );
        let condition = condition_at[counter];
        assert!(
            condition < previous_condition,
            "…and each one is a LOWER rung of the same authored ladder: {counter} fired on \
             {condition} points, which is not below the {previous_condition} of the rung \
             before it"
        );
        let (objective_state, message_count, deadline_state, remaining) = &at_warning[counter];

        // SURFACE ONE: the objectives list.
        assert_eq!(
            objective_state, "Some(Active)",
            "{counter} must post {objective} on the crew's own list, naming what happens \
             next — got {objective_state}"
        );
        // SURFACE TWO: the captain's countdown, still counting toward failure.
        assert_eq!(
            deadline_state, "pending",
            "{counter}: the captain's projected-failure countdown must still be live when \
             a warning fires"
        );
        assert!(
            *remaining > 0,
            "{counter}: …and counting DOWN to something that has not happened yet, not \
             sitting at {remaining}"
        );
        // SURFACE THREE: the people on the structure, on an open channel.
        assert!(
            bodies_seen.contains(body),
            "{counter} must put the gang on the head on the channel: '{body}' never reached \
             the inbox. Seen: {bodies_seen:?}"
        );
        assert!(
            *message_count > 0,
            "{counter}: the head's thread is open when the warning lands"
        );

        previous = fired;
        previous_condition = condition;
    }
    // The rungs the crew let go stay RED, which is what lets a crew say which
    // warning they ignored.
    assert_eq!(
        objective_status(&app, "obj-a3-tether-1"),
        ObjectiveStatus::Failed
    );
    assert_eq!(
        objective_status(&app, "obj-a3-tether-2"),
        ObjectiveStatus::Failed
    );
    assert_eq!(
        objective_status(&app, "obj-a3-tether-3"),
        ObjectiveStatus::Failed
    );

    // ── AC1: the authored floor triggers the collapse, and the entity GOES ──
    let crossed = at("a3_floor_crossed");
    let gone = at("head_gone");
    assert!(
        (crossed - skyway_projected_floor(&app)).abs() < 2.0,
        "the head crosses its floor when the authored strain rate says it will \
         ({crossed:.1} s against the world's own projection)"
    );
    assert!(
        gone >= crossed && gone - crossed < 1.0,
        "…and crossing it REMOVES THE STRUCTURE, on the same beat: crossed at \
         {crossed:.1} s, gone at {gone:.1} s"
    );
    assert!(
        !named_entity_present(&mut app, SKYWAY_HEAD),
        "the skyhook is not in the world at the end of a run that lost it — this is a \
         structural loss on screen, not a score penalty"
    );
    assert!(
        (at("a3_head_lost") - crossed).abs() < 0.5,
        "the consequences are chained off the REMOVAL's own Destroyed event, in the same \
         tick — a scripted destruction chains exactly as a kill does"
    );
    assert_eq!(
        skyway_captain_deadline(&mut app, "skyhook_failure_due").state,
        "cancelled",
        "the captain's countdown is called off once the thing it was counting to has \
         happened; a panel counting down to a structure that is already gone is worse \
         than no panel"
    );

    // ── AC4: the consequences are authored, on screen and derived from state ──
    let flags = &app.world().resource::<WorldContentRuntime>().flags;
    assert_eq!(
        (
            flags.counter("skyway_skyhook_lost"),
            flags.counter("skyway_skyhook_held"),
        ),
        (1, 0),
        "exactly one of the two fate flags is written, so a later mission reads a fact \
         rather than an absence"
    );
    assert_eq!(
        (
            flags.counter("skyway_transfer_capacity_lost"),
            flags.counter("skyhook_transfer_berths"),
        ),
        (1, 0),
        "the corridor's only two transfer berths left the world with the structure \
         that carried them — the named flag says so to the next act, and the \
         capacity's own mirrored counter says so to any predicate that asks. A \
         structure that has stopped ticking cannot correct its own last reading."
    );
    // The count is READ off authored state, never rolled. In an unattended run
    // #1036's comms backfill answers the committee (settling the strike, so a
    // FULL shift is back on the head) and answers the head's own channel (so the
    // gang were told, and braced): six, less one, is five.
    assert_eq!(
        (
            flags.counter("skyway_strike_settled"),
            flags.counter("skyway_head_told"),
            flags.counter("skyway_head_cleared"),
        ),
        (1, 1, 0),
        "precondition: the state the casualty count is derived FROM"
    );
    assert_eq!(
        flags.counter("skyway_head_casualties"),
        5,
        "…and the count is that state, arithmetically: a full shift of six because the \
         strike settled, less one because somebody answered when they called"
    );
    assert_eq!(
        objective_status(&app, "obj-a3-head-report"),
        ObjectiveStatus::Active,
        "somebody has to tell Control, and it sits on the panel until they do"
    );

    // ── AC5: the mission BRANCHES rather than ending ──
    let opened = at("a3_epilogue_open");
    assert!(
        opened > gone && opened - gone < 4.0,
        "the epilogue opens a beat behind the collapse ({gone:.1} s, {opened:.1} s)"
    );
    assert!(
        at("a3_epilogue_resolved") > opened,
        "…and reaches its own resolution: an epilogue that never resolves is not a branch, \
         it is a loose end"
    );
    assert_eq!(
        objective_status(&app, "obj-a3-survivors"),
        ObjectiveStatus::Failed,
        "a rescue nobody ran fails, exactly as the Lyra's did — the epilogue is playable, \
         which means it is also losable"
    );
    let flags = &app.world().resource::<WorldContentRuntime>().flags;
    assert_eq!(
        (
            flags.counter("skyway_survivors_recovered"),
            flags.counter("skyway_survivors_lost"),
        ),
        (0, 2),
        "and the epilogue writes its own campaign state"
    );

    // ── AC6: this is NOT the hard-fail path ──
    assert_eq!(
        app.world().resource::<State<GamePhase>>().get(),
        &GamePhase::InProgress,
        "losing the skyhook is a worse ending, not a game over: the run is still going"
    );
    assert!(
        app.world()
            .resource::<project_phoenix::simulation::GameOverReason>()
            .0
            .is_none(),
        "…and nothing latched a game-over reason. The crew's hull dying is the other \
         path entirely, and it writes that reason and none of these flags."
    );
}

/// **Issue #1040, AC3.** THE PLAYTEST. The final warning window is long enough
/// that a crew who react to the LAST rung still save the structure — driven on
/// the tick that rung actually fires, not at a time this test knows.
///
/// The margin it measures is the tuned value the world file records: the
/// stabilise is authored at 18 seconds against a 30-second window, and what is
/// left over is what a crew get to notice, decide and give the order in.
#[test]
fn falling_skyway_act_3_a_crew_who_react_to_the_last_warning_save_the_head() {
    use project_phoenix::core::messages::ObjectiveStatus;
    use project_phoenix::operations::{HoldState, OperationVerb, ShipOperations};
    use project_phoenix::world::server::WorldContentRuntime;

    let dt = SKYWAY_DT;
    let probe = build_headless_app(&skyway_args(dt, 1.0)).expect("the world must load");
    let projected = skyway_deadline_secs(&probe, "skyhook_failure_due") as f64;
    drop(probe);

    let args = skyway_args(dt, projected + 20.0);
    let mut app = build_headless_app(&args).expect("the scenario world must load and build");
    run(&mut app, 10);
    let (ship, ship_uuid) = skyway_crew_hull(&mut app);
    // Where Act 3's own approach objective sends a helm, and 247 units off the
    // head — inside the destroyer's authored 500-unit stabilise range.
    let station = bevy::prelude::Vec3::new(180.0, 0.0, 170.0);

    let mut ordered_at: Option<f64> = None;
    let mut completed_at: Option<f64> = None;

    for tick in 0..args.max_ticks {
        // Helm holding station, done by hand for the reason every operations
        // test in this file moves a ship by hand: this is a test of the warning
        // window, not of station-keeping.
        if ordered_at.is_some() && completed_at.is_none() {
            skyway_move(&mut app, ship, station);
        }
        run(&mut app, 1);
        let sim_t = (tick + 1) as f64 * dt;

        // ON THE TICK THE LAST WARNING FIRES. Not at an authored second this
        // test restates — a crew react to the warning, so the test does too.
        if ordered_at.is_none() && skyway_flag(&app, "a3_warning_three") > 0 {
            skyway_move(&mut app, ship, station);
            skyway_start_op(&mut app, &ship_uuid, OperationVerb::Stabilise, SKYWAY_HEAD);
            ordered_at = Some(sim_t);
        }
        if completed_at.is_none() {
            if let Some(hold) = app
                .world()
                .get::<ShipOperations>(ship)
                .and_then(|ops| ops.active.clone())
            {
                if hold.state() == HoldState::Completed {
                    completed_at = Some(sim_t);
                }
            }
        }
    }

    let ordered = ordered_at.expect("the third warning must fire in this run at all");
    let done = completed_at.expect(
        "a stabilise opened on the last warning must COMPLETE — if it cannot, the final \
         warning is decoration and the act is unwinnable rather than hard",
    );
    let floor = skyway_projected_floor(&app);

    // ── AC3: the margin, measured rather than asserted ──
    let slack = floor - done;
    assert!(
        slack >= 10.0,
        "the final warning window must leave real room: the order went in at {ordered:.1} s, \
         the work landed at {done:.1} s, and the head would have crossed its floor at \
         {floor:.1} s — {slack:.1} s of margin, against the 12 the world file records"
    );
    assert!(
        named_entity_present(&mut app, SKYWAY_HEAD),
        "…and the structure is STILL THERE, past the tick that would otherwise have taken it"
    );
    assert!(
        condition_of(&mut app, SKYWAY_HEAD) >= 42.0,
        "because the authored 22-point payout carried the head back over the FIRST rung's \
         42 % restore line from the last one — one run stands the whole ladder down, which \
         is what makes a late reaction a save rather than a stay of execution"
    );

    // ── The other ending, on the same surfaces ──
    let flags = &app.world().resource::<WorldContentRuntime>().flags;
    assert_eq!(
        (
            flags.counter("skyway_skyhook_held"),
            flags.counter("skyway_skyhook_lost"),
        ),
        (1, 0),
        "exactly one of the two fate flags is written, and it is the other one this time"
    );
    assert_eq!(
        (
            flags.counter("a3_floor_crossed"),
            flags.counter("a3_epilogue_open"),
        ),
        (0, 0),
        "the floor was never crossed, so there is no collapse and no epilogue — the two \
         branches are exclusive"
    );
    assert_eq!(
        objective_status(&app, "obj-a3-tether-3"),
        ObjectiveStatus::Completed,
        "the rung the crew acted on completes"
    );
    for ignored in ["obj-a3-tether-1", "obj-a3-tether-2"] {
        assert_eq!(
            objective_status(&app, ignored),
            ObjectiveStatus::Failed,
            "…and the two they let go stay red. Saving the structure does not un-ignore a \
             warning, and the panel is the record of which ones went by."
        );
    }
    assert_eq!(
        objective_status(&app, "obj-a3-head"),
        ObjectiveStatus::Completed
    );
    assert_eq!(
        skyway_captain_deadline(&mut app, "skyhook_failure_due").state,
        "cancelled",
        "and the countdown is called off, because the projection has been beaten"
    );
}

/// **Issue #1040, AC5.** The epilogue is PLAYABLE, and playable means winnable:
/// a crew who lose the head and then go and do the work pull both craft out of
/// the debris inside the epilogue's own window.
///
/// Both rescues run through the tow the Lyra's did, against condition thresholds
/// the payout is sized to cross — so the epilogue reuses the act before it
/// rather than inventing a second rescue mechanic.
#[test]
fn falling_skyway_act_3_epilogue_is_completable_by_a_crew_that_tows_both_craft() {
    use project_phoenix::core::messages::ObjectiveStatus;
    use project_phoenix::operations::{HoldState, OperationVerb, ShipOperations};
    use project_phoenix::world::server::WorldContentRuntime;

    let dt = SKYWAY_DT;
    let probe = build_headless_app(&skyway_args(dt, 1.0)).expect("the world must load");
    let projected = skyway_deadline_secs(&probe, "skyhook_failure_due") as f64;
    drop(probe);

    let args = skyway_args(dt, projected + 100.0);
    let mut app = build_headless_app(&args).expect("the scenario world must load and build");
    run(&mut app, 10);
    let (ship, ship_uuid) = skyway_crew_hull(&mut app);

    // The two craft, taken in the order the collapse handler spawns them.
    let mut queue = vec![SKYWAY_LIGHTER, SKYWAY_POD];
    let mut towing: Option<&str> = None;
    let mut recovered: Vec<(String, f64)> = Vec::new();

    for tick in 0..args.max_ticks {
        // Alongside whatever is on the line, every tick — a tow is run from
        // close aboard and the crew are holding it. Done by hand for the reason
        // every operations test in this file moves a ship by hand.
        if let Some(target) = towing {
            let alongside = position_of(&mut app, target);
            skyway_move(
                &mut app,
                ship,
                bevy::prelude::Vec3::new(alongside.x + 40.0, alongside.y, alongside.z + 40.0),
            );
        }
        run(&mut app, 1);
        let sim_t = (tick + 1) as f64 * dt;

        let settled = app
            .world()
            .get::<ShipOperations>(ship)
            .and_then(|ops| ops.active.clone())
            .map(|hold| hold.state() == HoldState::Completed)
            .unwrap_or(true);

        // Take the next craft the moment the line is free and there is one in
        // the world to take.
        if settled {
            if let Some(next) = queue.first().copied() {
                if named_entity_present(&mut app, next) {
                    if towing == Some(next) {
                        recovered.push((next.to_string(), sim_t));
                        queue.remove(0);
                        towing = None;
                    } else {
                        let alongside = position_of(&mut app, next);
                        skyway_move(
                            &mut app,
                            ship,
                            bevy::prelude::Vec3::new(
                                alongside.x + 40.0,
                                alongside.y,
                                alongside.z + 40.0,
                            ),
                        );
                        skyway_start_op(&mut app, &ship_uuid, OperationVerb::Tow, next);
                        towing = Some(next);
                    }
                }
            }
        }
    }

    assert_eq!(
        recovered.len(),
        2,
        "both craft must be recoverable inside the epilogue's authored window — an \
         epilogue that cannot be completed is a cut scene, not a branch. Got {recovered:?}"
    );
    let flags = &app.world().resource::<WorldContentRuntime>().flags;
    assert_eq!(
        (
            flags.counter("skyway_skyhook_lost"),
            flags.counter("a3_epilogue_resolved"),
        ),
        (1, 1),
        "precondition: this is the branch that lost the head, and its epilogue closed"
    );
    assert_eq!(
        (
            flags.counter("skyway_survivors_recovered"),
            flags.counter("skyway_survivors_lost"),
        ),
        (2, 0),
        "both craft are on the record as recovered"
    );
    assert_eq!(
        objective_status(&app, "obj-a3-survivors"),
        ObjectiveStatus::Completed,
        "and the epilogue's own mandatory objective completes"
    );
    for craft in [SKYWAY_LIGHTER, SKYWAY_POD] {
        assert!(
            named_entity_present(&mut app, craft),
            "{craft} is still in the world: the debris takes what is still adrift when the \
             window closes, and neither of these was"
        );
    }
}

/// **Issue #1040, AC6.** Ship destruction is a DIFFERENT path, and the two are
/// told apart by what they write rather than by tone.
///
/// The crew's hull dies the way the mission already makes possible — living in a
/// radiation band it was warned about — and the run ends: the engine latches its
/// own game-over reason, and neither of Act 3's fate flags is written by
/// anybody, because the act the collapse belongs to was never reached.
#[test]
fn falling_skyway_losing_the_ship_is_a_hard_fail_and_writes_none_of_the_head_s_flags() {
    use project_phoenix::world::server::WorldContentRuntime;

    let dt = SKYWAY_DT;
    let probe = build_headless_app(&skyway_args(dt, 1.0)).expect("the world must load");
    let band_at = skyway_deadline_secs(&probe, "storm_band_one_due") as f64;
    drop(probe);

    let args = skyway_args(dt, band_at + 60.0);
    let mut app = build_headless_app(&args).expect("the scenario world must load and build");
    run(&mut app, 10);
    let (ship, _) = skyway_crew_hull(&mut app);

    let mut rng = vellum_rng::Pcg32::seeded(1040, 0);
    let mut spent = false;
    let mut over_at: Option<f64> = None;
    for tick in 0..args.max_ticks {
        // Standing in the front, at the anchor the world authors the first band
        // over. The band arrives on its own authored deadline and burns whatever
        // is under it; helm is doing the one thing every warning in this mission
        // tells a crew not to do.
        if over_at.is_none() {
            skyway_move(&mut app, ship, bevy::prelude::Vec3::new(0.0, 0.0, -760.0));
        }
        run(&mut app, 1);
        let sim_t = (tick + 1) as f64 * dt;

        // A destroyer that has already taken a beating. The hull is spent by
        // hand rather than shot off, because what this test is about is what the
        // DEATH writes, not how a hull gets to zero — and it is spent once the
        // band is overhead, so the ship's own repair sweep has no hundred and
        // forty seconds to undo it. The last twelve points are taken by
        // `region_radiation_band.toml`'s own authored damage, through the
        // ordinary region path that latches the reason.
        if !spent && sim_t > band_at + 1.0 {
            let mut hull = app
                .world_mut()
                .get_mut::<project_phoenix::entity_spawner::EntitySystemHull>(ship)
                .expect("the crew's hull has systems");
            let spend = hull.0.total_current() - 12.0;
            hull.0.apply_damage(spend, &mut rng);
            spent = true;
        }
        if over_at.is_none()
            && app.world().resource::<State<GamePhase>>().get() == &GamePhase::GameOver
        {
            over_at = Some(sim_t);
            break;
        }
    }

    let over = over_at.expect(
        "a spent hull living in an authored radiation band must die — this is the mission's \
         own hard-fail path, not a hypothetical one",
    );
    assert!(
        over > band_at,
        "…and it dies to the band, which does not exist before its deadline ({band_at} s, \
         {over:.1} s)"
    );
    // The OUTCOME half of the latch, not the reason string: `on_game_over_enter`
    // takes the string on its way out to the clients, and what survives in the
    // record is the verdict. That verdict is the engine's own — no scenario
    // handler wrote it, and no scenario handler could.
    assert_eq!(
        app.world()
            .resource::<project_phoenix::simulation::GameOverReason>()
            .1,
        Some(project_phoenix::balance::Outcome::Defeat),
        "the hard fail latches the ENGINE's own defeat"
    );
    let flags = &app.world().resource::<WorldContentRuntime>().flags;
    assert_eq!(
        (
            flags.counter("skyway_skyhook_lost"),
            flags.counter("skyway_skyhook_held"),
            flags.counter("a3_epilogue_open"),
        ),
        (0, 0, 0),
        "and it writes NONE of the collapse branch's state. The two endings are \
         distinguishable by what is in the record, not by how they read: a crew who lost \
         their ship did not lose the skyhook, and a crew who lost the skyhook are still \
         flying."
    );
    assert!(
        named_entity_present(&mut app, SKYWAY_HEAD),
        "the head is still standing when the ship goes — which is the whole distinction"
    );
}

// ── The collapse mechanism, end to end (issue #1040) ─────────────────────────

const PROBE_COLLAPSE_WORLD: &str = "assets/worlds/probe_collapse.toml";
const PROBE_HOOK_SAVED: &str = "world.probe_collapse.entity.hook_saved.name";
const PROBE_HOOK_LOST: &str = "world.probe_collapse.entity.hook_lost.name";
const PROBE_SURVIVOR: &str = "world.probe_collapse.entity.survivor.name";

/// **Issue #1040, AC1/AC2/AC3/AC5.** The whole collapse mechanism in one
/// twenty-five-second run: a ladder of authored thresholds warns three times as
/// two identical heads walk down toward their floor, the one with a tender that
/// ACTS ON THE THIRD WARNING is saved, the one without it is taken out of the
/// world by `destroy_entity`, and the removal branches into a rescue that
/// completes.
///
/// The two heads are the assertion. They are the same template on the same
/// strain beat with the same ladder shape, so nothing about their fates can be
/// coincidence — the only thing that differs is one `ctx.effects.stabilise(…)`
/// on one `on_flag_cleared` handler.
#[test]
fn a_condition_floor_collapses_a_structure_and_a_reaction_inside_the_last_window_prevents_it() {
    use project_phoenix::core::messages::ObjectiveStatus;
    use project_phoenix::world::server::WorldContentRuntime;

    let dt = 1.0 / 60.0;
    let args = HeadlessArgs {
        world_path: PROBE_COLLAPSE_WORLD.into(),
        ship_path: "assets/entities/alliance_cruiser.toml".into(),
        dt,
        max_ticks: ticks_for_sim_seconds(45.0, dt),
        deterministic: true,
        seed: Some(42),
        ..test_args()
    };
    let mut app = build_headless_app(&args).expect("the probe world must load and build");

    let mut first: std::collections::BTreeMap<String, f64> = Default::default();
    let mut saved_at_warning_three: Option<f32> = None;
    let mut lost_at_warning_three: Option<f32> = None;

    for tick in 0..args.max_ticks {
        run(&mut app, 1);
        let sim_t = (tick + 1) as f64 * dt;

        for flag in [
            "saved_warning_one",
            "saved_warning_two",
            "saved_warning_three",
            "saved_op_started",
            "saved_stood_down",
            "saved_floor_crossed",
            "lost_warning_one",
            "lost_warning_two",
            "lost_warning_three",
            "lost_stood_down",
            "lost_floor_crossed",
            "lost_head_gone",
            "projection_fired",
            "epilogue_open",
            "rescue_started",
            "survivor_recovered",
            "epilogue_resolved",
        ] {
            if app
                .world()
                .resource::<WorldContentRuntime>()
                .flags
                .counter(flag)
                > 0
            {
                first.entry(flag.to_string()).or_insert(sim_t);
            }
        }
        if saved_at_warning_three.is_none() && first.contains_key("saved_warning_three") {
            saved_at_warning_three = Some(condition_of(&mut app, PROBE_HOOK_SAVED));
            lost_at_warning_three = Some(condition_of(&mut app, PROBE_HOOK_LOST));
        }
        if !named_entity_present(&mut app, PROBE_HOOK_LOST) {
            first.entry("lost_gone".to_string()).or_insert(sim_t);
        }
    }

    let at = |key: &str| -> f64 {
        *first
            .get(key)
            .unwrap_or_else(|| panic!("'{key}' never happened in this run: {first:?}"))
    };
    let never = |key: &str| {
        assert!(
            !first.contains_key(key),
            "'{key}' must NEVER happen in this run, and it did at {:.2} s",
            first[key]
        );
    };

    // ── AC2: the ladder warns three times, in order, on BOTH heads ──
    for side in ["saved", "lost"] {
        let one = at(&format!("{side}_warning_one"));
        let two = at(&format!("{side}_warning_two"));
        let three = at(&format!("{side}_warning_three"));
        assert!(
            one < two && two < three,
            "{side}: the rungs fire in ladder order ({one:.2}, {two:.2}, {three:.2})"
        );
        assert!(
            (two - one - (three - two)).abs() < 0.5,
            "{side}: …and evenly, because the strain rate and the rung spacing are both \
             authored — a scenario derives its warning window from that arithmetic"
        );
    }
    assert!(
        (at("saved_warning_three") - at("lost_warning_three")).abs() < 0.1,
        "the two heads reach their last warning together: they are the same template on \
         the same beat, and everything after this point is the tender"
    );
    assert_eq!(
        (saved_at_warning_three, lost_at_warning_three),
        (Some(12.0), Some(12.0)),
        "…on the same authored condition, which is what makes the ladder a reading of the \
         structure rather than a timer beside it"
    );

    // ── AC3: the reaction lands inside the final window and prevents it ──
    assert!(
        (at("saved_op_started") - at("saved_warning_three")).abs() < 0.1,
        "the tender opens its stabilise ON the third warning — the event a crew would be \
         reacting to, not a second this file also has to keep in step"
    );
    let stood_down = at("saved_stood_down");
    let floor = at("lost_floor_crossed");
    assert!(
        stood_down < floor,
        "and it lands BEFORE the floor the other head crosses ({stood_down:.2} s against \
         {floor:.2} s). If the final window is ever tuned shorter than the operation, \
         this is the assertion that says so."
    );
    assert!(
        named_entity_present(&mut app, PROBE_HOOK_SAVED),
        "the saved head is still in the world"
    );
    assert!(
        condition_of(&mut app, PROBE_HOOK_SAVED) >= 30.0,
        "…back over its own top rung's restore line, which is what standing the ladder \
         down means"
    );
    never("saved_floor_crossed");
    never("lost_stood_down");

    // ── AC1: the floor removes the structure, and the removal chains ──
    let gone = at("lost_gone");
    assert!(
        gone >= floor && gone - floor < 0.5,
        "crossing the floor REMOVES the head, on the same beat ({floor:.2} s, {gone:.2} s)"
    );
    assert!(
        !named_entity_present(&mut app, PROBE_HOOK_LOST),
        "…and it is not in the world afterwards"
    );
    assert!(
        (at("lost_head_gone") - floor).abs() < 0.5,
        "the consequences are chained off the removal's own Destroyed event, which is what \
         makes a scripted collapse indistinguishable from any other loss downstream"
    );
    assert_eq!(
        objective_status(&app, "obj-probe-lost"),
        ObjectiveStatus::Failed
    );
    assert_eq!(
        app.world()
            .resource::<WorldContentRuntime>()
            .flags
            .counter("probe_lost_berths"),
        0,
        "…and the berths the head carried are gone from the flag store with it, \
         rather than left saying two forever"
    );
    assert_eq!(
        objective_status(&app, "obj-probe-saved"),
        ObjectiveStatus::Completed
    );
    never("projection_fired");

    // ── AC5: the removal branches into a rescue, and the rescue completes ──
    assert!(
        at("epilogue_open") > gone,
        "the epilogue opens after the collapse, not beside it"
    );
    assert!(
        at("survivor_recovered") > at("rescue_started"),
        "the recovery is read off the tow's own payout crossing the craft's authored \
         threshold — the only completion signal an operation leaves behind"
    );
    assert!(
        at("epilogue_resolved") > at("survivor_recovered"),
        "…and the epilogue resolves on its own clock, after the work"
    );
    assert_eq!(
        objective_status(&app, "obj-probe-survivors"),
        ObjectiveStatus::Completed
    );
    assert!(
        named_entity_present(&mut app, PROBE_SURVIVOR),
        "the craft that was recovered is still in the world; the debris takes only what is \
         still adrift when the window closes"
    );
}

// ── Falling Skyway, Act 3: the transfer window (issue #1042, parent #852) ────

/// The authority that runs the window, and the one console surface its whole
/// arithmetic lands on.
const WINDOW_CONTROL: &str = "world.falling_skyway.entity.skyway_control.name";
/// The head. Its lift certification is the whole of its contribution.
const WINDOW_HEAD: &str = "world.falling_skyway.entity.skyhook.name";
/// The rung that still works.
const WINDOW_DEPOT_A: &str = "world.falling_skyway.entity.depot_ladder_a.name";
/// Where the act's own objective sends helm, and — not by accident — inside
/// Control's authored 800-unit comms range, which is what makes the window's
/// hail land.
const WINDOW_STATION: bevy::prelude::Vec3 = bevy::prelude::Vec3::new(180.0, 0.0, 170.0);

/// Mission seconds elapsed in this run.
fn window_now(app: &bevy::prelude::App) -> f64 {
    app.world()
        .resource::<bevy::prelude::Time>()
        .elapsed_secs_f64()
}

/// Step in blocks until the run reaches `secs`. Coarse on purpose: the beats
/// this group asserts ON are found tick by tick, and everything between them is
/// the mission running.
fn window_run_to(app: &mut bevy::prelude::App, secs: f64) {
    while window_now(app) < secs {
        run(app, 10);
    }
}

/// Skyway Control's manifest as the live condition track carries it — every
/// authored capacity id against its current level.
fn window_manifest(app: &mut bevy::prelude::App) -> std::collections::BTreeMap<String, i64> {
    use project_phoenix::entities::spawner::EntityName;
    use project_phoenix::infrastructure::InfrastructureCondition;

    let mut q = app
        .world_mut()
        .query::<(&EntityName, &InfrastructureCondition)>();
    let found = q
        .iter(app.world())
        .find(|(name, _)| name.0 == WINDOW_CONTROL)
        .map(|(_, condition)| {
            condition
                .0
                .capacities()
                .iter()
                .map(|c| (c.id.clone(), c.level))
                .collect::<std::collections::BTreeMap<String, i64>>()
        });
    found.expect("Skyway Control carries the window manifest")
}

/// One structure's published capacity level, read off its own live track.
fn window_capacity_of(app: &mut bevy::prelude::App, entity: &str, id: &str) -> i64 {
    use project_phoenix::entities::spawner::EntityName;
    use project_phoenix::infrastructure::InfrastructureCondition;

    let mut q = app
        .world_mut()
        .query::<(&EntityName, &InfrastructureCondition)>();
    let found = q
        .iter(app.world())
        .find(|(name, _)| name.0 == entity)
        .and_then(|(_, condition)| condition.0.capacity(id));
    found.unwrap_or_else(|| panic!("{entity} publishes no capacity '{id}'"))
}

/// What the crew can actually READ: Control's fact sheet, as `(label, count)`
/// pairs off the dossier blackboard the tactical panel renders.
///
/// This is the AC that matters most about the manifest — a number the
/// simulation holds and no console shows is not a number the crew have — so it
/// is read through the published projection rather than off the component.
fn window_panel_rows(app: &mut bevy::prelude::App) -> Vec<(String, i64)> {
    use project_phoenix::dossier::dossier_blackboard_key;
    use project_phoenix::messages::{DossierBlackboard, DossierValue, SystemBlackboard};
    use project_phoenix::server_app::ShipSystemBlackboards;

    let mut q = app
        .world_mut()
        .query_filtered::<&ShipSystemBlackboards, With<LocalShip>>();
    let blackboards = q
        .iter(app.world())
        .next()
        .expect("the crew own ship publishes its blackboards");
    let bb: DossierBlackboard = match blackboards.0.get(&dossier_blackboard_key()) {
        Some(SystemBlackboard::Dossiers(bb)) => bb.clone(),
        other => panic!("expected a dossier blackboard, got {other:?}"),
    };
    bb.subjects
        .iter()
        .find(|s| s.name == WINDOW_CONTROL)
        .map(|s| {
            s.facts
                .iter()
                .filter_map(|f| match f.value {
                    DossierValue::Count(n) => Some((f.label.clone(), n)),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Run one external operation on `target` from alongside, holding station for
/// `seconds` — long enough that the authored duration has expired and the payout
/// has landed.
///
/// The ship is pinned by hand for the length of the job, and that is the honest
/// analogue of helm holding position rather than a shortcut: an operation
/// re-tests its authored range every tick, and the mission's own Reach
/// directives would otherwise fly the hull off the job mid-run. Which is helm's
/// problem, and not this test's subject.
fn window_operation(
    app: &mut bevy::prelude::App,
    ship: bevy::prelude::Entity,
    ship_uuid: &str,
    verb: project_phoenix::operations::OperationVerb,
    target: &str,
    alongside: bevy::prelude::Vec3,
    seconds: f64,
) {
    use project_phoenix::operations::PendingOperationStart;
    use project_phoenix::world::server::WorldContentRuntime;

    let target_uuid = app
        .world()
        .resource::<WorldContentRuntime>()
        .name_to_uuid
        .get(target)
        .cloned()
        .unwrap_or_else(|| panic!("{target} is not in this world"));
    skyway_move(app, ship, alongside);
    run(app, 2);
    app.world_mut()
        .resource_mut::<WorldContentRuntime>()
        .pending_operation_starts
        .push(PendingOperationStart {
            ship_uuid: ship_uuid.to_string(),
            verb,
            target_uuid,
        });
    let blocks = ticks_for_sim_seconds(seconds, SKYWAY_DT) / 10;
    for _ in 0..blocks {
        skyway_move(app, ship, alongside);
        run(app, 10);
    }
}

/// A 30-second field repair on `target`, from alongside.
fn window_field_repair(
    app: &mut bevy::prelude::App,
    ship: bevy::prelude::Entity,
    ship_uuid: &str,
    target: &str,
    alongside: bevy::prelude::Vec3,
) {
    window_operation(
        app,
        ship,
        ship_uuid,
        project_phoenix::operations::OperationVerb::FieldRepair,
        target,
        alongside,
        34.0,
    );
}

/// Catch the tether: one 18-second stabilise on the skyhook head, run from the
/// station-keeping berth #1040's own approach objective points helm at.
///
/// This is the Act-3 work a crew who are paying attention do, and it is what
/// keeps the head CERTIFIED as well as standing — a stabilise pays a lump into
/// the condition track, and the transfer window reads that track's lift
/// threshold. Unlike a field repair it carries no `work_stoppage` interrupt, so
/// it pays in full whether or not the strike was ever settled.
fn window_stabilise_head(
    app: &mut bevy::prelude::App,
    ship: bevy::prelude::Entity,
    ship_uuid: &str,
) {
    window_operation(
        app,
        ship,
        ship_uuid,
        project_phoenix::operations::OperationVerb::Stabilise,
        WINDOW_HEAD,
        WINDOW_STATION,
        22.0,
    );
}

/// Hold the head up until the tether's projection has fallen due, then leave the
/// ship on station.
///
/// The strain #1040 authors walks the head down 2 points every 10 seconds for as
/// long as the watch is open, and it opens the moment the storm clears. A crew
/// who want anything to lift through the transfer window have to catch it more
/// than once — a single stabilise buys about a hundred seconds before the head
/// falls back under its lift line — so this runs them until the projection
/// resolves the act, which is what a crew who never stopped watching look like.
fn window_hold_the_tether(
    app: &mut bevy::prelude::App,
    ship: bevy::prelude::Entity,
    ship_uuid: &str,
    until: f64,
) {
    // The last one has to LAND before the projection: an 18-second job started
    // at t-10 is a job the act resolves out from under.
    while window_now(app) < until - 24.0 {
        window_stabilise_head(app, ship, ship_uuid);
    }
    window_run_to(app, until + 3.0);
    skyway_move(app, ship, WINDOW_STATION);
}

/// The crew's own hull and its minted uuid.
fn window_ship(app: &mut bevy::prelude::App) -> (bevy::prelude::Entity, String) {
    use project_phoenix::entities::spawner::EntityUuid;
    let mut q = app
        .world_mut()
        .query_filtered::<(bevy::prelude::Entity, &EntityUuid), With<LocalShip>>();
    let found = q
        .iter(app.world())
        .next()
        .map(|(entity, uuid)| (entity, uuid.0.clone()));
    found.expect("the crew hull")
}

/// One row off the captain's countdown, by deadline id.
fn window_countdown(
    app: &mut bevy::prelude::App,
    id: &str,
) -> Option<project_phoenix::messages::DeadlineSnapshot> {
    let mut q = app
        .world_mut()
        .query::<&project_phoenix::server_app::ShipSystemBlackboards>();
    let rows: Vec<_> = q
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
    rows.first()
        .and_then(|list| list.iter().find(|d| d.id == id).cloned())
}

/// Pull one of the act's booking seams: ask for a lift for that claimant.
///
/// A flag write AND the `WorldEvent::FlagSet` that makes it an event, which is
/// the same pair every other writer of a world flag emits — the script applier
/// for `ctx.flags.x = 1`, the infrastructure mirror for a threshold crossing.
/// Writing the store alone would move the number and chain nothing, which is
/// the bug this pairing exists to prevent.
///
/// Cleared first so a second ask on the same seam is a fresh EDGE. That is what
/// a captain asking twice looks like, and `on_flag_set` fires on transitions.
fn window_book(app: &mut bevy::prelude::App, seam: &str) {
    use project_phoenix::world::server::WorldContentRuntime;
    let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
    runtime.flags.clear_flag(seam);
    runtime.flags.set_flag(seam);
    runtime
        .pending_world_events
        .push(project_phoenix::world::content::WorldEvent::FlagSet {
            name: seam.to_string(),
            origin_layer: None,
        });
    run(app, 4);
}

/// Tick until the window opens, and return the mission time it opened at.
fn window_open_time(app: &mut bevy::prelude::App, give_up_at: f64) -> f64 {
    while window_now(app) < give_up_at {
        run(app, 1);
        if skyway_flag(app, "skyway_window_open") == 1 {
            return window_now(app);
        }
    }
    panic!("the transfer window never opened by {give_up_at} s");
}

/// **Issue #1042, AC6 — the claim that is about EVERY run, asserted from the
/// authored numbers rather than from one of them.**
///
/// A headless run can only ever show that one mission state is short. What the
/// slice actually promises is that no mission state is sufficient, and that is
/// arithmetic over what the world authors: the ladder's two rungs are the
/// chain's ceiling, nothing the crew can do raises either, and the three claims
/// together are larger than the pair of them. The companion tests below prove a
/// crew can REACH that ceiling; this one proves reaching it is not enough.
#[test]
fn no_state_of_falling_skyway_covers_all_three_claimants() {
    use project_phoenix::operations::{OperationVerb, ShipOperations};

    let mut app = build_headless_app(&skyway_args(SKYWAY_DT, 2.0)).expect("the world must load");
    run(&mut app, 30);

    // The supply side, as the world authors it: the head's two published
    // numbers and the two rungs' throughput, read off the live tracks on the
    // mission's opening tick.
    let berths = window_capacity_of(&mut app, WINDOW_HEAD, "skyhook_transfer_berths");
    let climber = window_capacity_of(&mut app, WINDOW_HEAD, "skyhook_climber_load");
    let rung_a = window_capacity_of(&mut app, WINDOW_DEPOT_A, "depot_a_fuel_lift");
    let rung_b = window_capacity_of(&mut app, SKYWAY_DEPOT_B, "depot_b_fuel_lift");
    let head_ceiling = berths * climber;
    let ladder_ceiling = rung_a + rung_b;
    let best_possible = head_ceiling.min(ladder_ceiling);

    // The demand side, off Control's manifest.
    let manifest = window_manifest(&mut app);
    let demand = manifest["skyway_claim_committee"]
        + manifest["skyway_claim_havelock"]
        + manifest["skyway_claim_convoy"];

    assert!(
        best_possible < demand,
        "THE WHOLE ACT: the best the chain can ever do is {best_possible} (the head offers \
         {berths} berths at {climber} a climber, the ladder puts in {rung_a} + {rung_b}), and \
         the three claims come to {demand}. If this ever passes, the window is a puzzle \
         rather than a choice"
    );

    // …and that ceiling is a CEILING rather than today's reading, because
    // nothing the crew fly can put anything into a rung. A real transfer moves
    // an authored `[transfer]` cargo between two ends carrying the same capacity
    // id, and the mission's hull authors none — its `transfer` verb stands a
    // berth up, it does not deliver into one.
    let (ship, _) = window_ship(&mut app);
    let transfer_terms_exist = app
        .world()
        .get::<ShipOperations>(ship)
        .expect("the destroyer authors an [operations] table")
        .capabilities
        .capabilities
        .iter()
        .any(|c| c.verb == OperationVerb::Transfer && c.transfer.is_some());
    assert!(
        !transfer_terms_exist,
        "the crew's hull must carry no [transfer] terms: a hull that could deliver cargo \
         into a rung could raise the ladder's ceiling, and the assertion above would stop \
         being about every run"
    );

    // A structural double-check on the pairing the claims are tuned for: ANY
    // TWO fit, and that is what makes the window a choice rather than a
    // disaster. #1043 enforces it by reading these numbers.
    let mut claims = [
        manifest["skyway_claim_committee"],
        manifest["skyway_claim_havelock"],
        manifest["skyway_claim_convoy"],
    ];
    claims.sort();
    assert!(
        claims[1] + claims[2] <= best_possible,
        "the two most expensive claims must fit inside the best the chain can do, or \
         'capacity for two' is not what the numbers say: {} + {} against {best_possible}",
        claims[1],
        claims[2]
    );
}

/// **Issue #1042, AC1/AC2/AC3/AC4/AC5/AC7 — the window end to end, for a crew
/// who did everything the mission had to offer.**
///
/// The strike talked down at the table, Ladder B mended back over its own line,
/// and the tether caught and held all the way to its projection — so this crew
/// reach the window with a certified head and both rungs pumping, which is the
/// most the chain can give. It is still not enough, Control says so when the
/// window opens, the three figures are on Control's fact sheet while it is open,
/// and the window shuts on its own deadline.
///
/// KEEPING THE HEAD IS PART OF "EVERYTHING" NOW, and that is the interlock
/// between the two halves of Act 3 (#1040 and this slice): the strain walks the
/// head down through its lift certification long before the window opens, so a
/// crew who stop watching arrive with nothing to lift however well they did in
/// Acts 1 and 2.
#[test]
fn a_crew_who_did_everything_still_reach_the_window_short() {
    use project_phoenix::core::messages::ObjectiveStatus;

    let (mut app, ship) = skyway_at_act_two();
    let (_, ship_uuid) = window_ship(&mut app);

    // ── Act 2's work: the strike ends at the table ──────────────────────────
    skyway_negotiate_to_a_vote(&mut app);
    assert_eq!(skyway_flag(&app, "skyway_settled_by_negotiation"), 1);
    // The committee put it to the floor; the rung starts moving twenty seconds
    // later, which is #1036's authored pace and not this test's to shortcut.
    let settled_by = window_now(&app) + 24.0;
    window_run_to(&mut app, settled_by);
    assert_eq!(
        skyway_flag(&app, "skyway_strike_settled"),
        1,
        "precondition: the workers are back before anything gets repaired at full rate"
    );

    // ── Act 2's other work: mend the rung the dispute was about ─────────────
    window_field_repair(
        &mut app,
        ship,
        &ship_uuid,
        SKYWAY_DEPOT_B,
        bevy::prelude::Vec3::new(1180.0, 0.0, 300.0),
    );
    assert_eq!(
        skyway_flag(&app, "depot_b_pumping"),
        1,
        "a full-rate repair must put the rung back over its own line. Mending it while the \
         strike was still on would not have been enough, which is the pairing #1035 authored"
    );

    // ── Act 3's work: hold the tether up until its projection resolves ──────
    let projection = skyway_deadline_secs(&app, "skyhook_failure_due") as f64;
    let watch_opens = skyway_deadline_secs(&app, "storm_passed_due") as f64;
    window_run_to(&mut app, watch_opens + 4.0);
    assert_eq!(
        skyway_flag(&app, "skyway_tether_watch"),
        1,
        "precondition: #1040's Act-3 watch is open and the head is under load"
    );
    window_hold_the_tether(&mut app, ship, &ship_uuid, projection);
    assert_eq!(
        skyway_flag(&app, "skyway_skyhook_held"),
        1,
        "this crew kept the structure"
    );
    assert_eq!(skyway_flag(&app, "skyway_skyhook_lost"), 0);
    assert_eq!(
        skyway_flag(&app, "skyhook_lift_capable"),
        1,
        "…and kept it CERTIFIED, which is a stronger claim than kept it standing: a head \
         caught at the last rung is a head that holds and cannot lift"
    );

    // ── AC1: the countdown is on the panel BEFORE the window opens ──────────
    let opens_at = skyway_deadline_secs(&app, "skyway_transfer_window") as f64;
    let closes_at = skyway_deadline_secs(&app, "skyway_window_closes") as f64;
    window_run_to(&mut app, opens_at - 6.0);
    assert_eq!(
        skyway_flag(&app, "skyway_window_open"),
        0,
        "the window is not open until its deadline says so"
    );
    let pending = window_countdown(&mut app, "skyway_transfer_window")
        .expect("the transfer window is a VISIBLE deadline on the captain's countdown");
    assert!(
        pending.remaining_secs > 0 && pending.remaining_secs <= 8,
        "…and it is counting DOWN: {} s left six seconds out",
        pending.remaining_secs
    );
    assert_eq!(
        pending.label, "world.falling_skyway.deadline.transfer_window.label",
        "the crew-facing label is a strings.csv id, never English"
    );
    // Nothing has been published onto the manifest yet: a figure the mission has
    // not earned is not a figure on a panel.
    let before = window_manifest(&mut app);
    assert_eq!(before["skyway_window_available"], 0);
    assert_eq!(before["skyway_window_demand"], 0);
    assert_eq!(before["skyway_window_shortfall"], 0);

    // ── AC1: it opens ON the deadline ───────────────────────────────────────
    let opened = window_open_time(&mut app, opens_at + 10.0);
    assert!(
        (opened - opens_at).abs() < 1.5,
        "the window opens on its authored deadline, not near it: {opened:.1} s against an \
         authored {opens_at:.0} s"
    );

    // ── AC2: the capacity is COMPUTED, and this crew earned all of it ───────
    let berths = window_capacity_of(&mut app, WINDOW_HEAD, "skyhook_transfer_berths");
    let climber = window_capacity_of(&mut app, WINDOW_HEAD, "skyhook_climber_load");
    let rung_a = window_capacity_of(&mut app, WINDOW_DEPOT_A, "depot_a_fuel_lift");
    let rung_b = window_capacity_of(&mut app, SKYWAY_DEPOT_B, "depot_b_fuel_lift");
    let supply = skyway_flag(&app, "skyway_window_supply");
    assert_eq!(
        supply,
        (berths * climber).min(rung_a + rung_b),
        "a crew with a certified head, both rungs pumping and nobody out get the whole \
         chain: the smaller of what the head lifts and what the ladder puts in it"
    );
    assert_eq!(
        supply,
        rung_a + rung_b,
        "…and with the head certified the LADDER is what binds, which is why mending the \
         rung was worth doing"
    );

    // ── AC3/AC4: both figures, and the shortfall, on the panel and out loud ─
    let demand = skyway_flag(&app, "skyway_window_demand_total");
    let short = skyway_flag(&app, "skyway_window_shortfall_at_open");
    assert_eq!(short, demand - supply);
    assert!(
        short > 0,
        "THE ACCEPTANCE CRITERION: even this crew are short. supply {supply}, demand \
         {demand}"
    );
    let manifest = window_manifest(&mut app);
    assert_eq!(manifest["skyway_window_available"], supply);
    assert_eq!(manifest["skyway_window_demand"], demand);
    assert_eq!(manifest["skyway_window_shortfall"], short);
    assert_eq!(
        manifest["skyway_window_committed"], 0,
        "nothing has been put on the ribbon yet"
    );

    // …and the same three, on a console, through the projection the tactical
    // dossier panel renders.
    let rows: std::collections::BTreeMap<String, i64> =
        window_panel_rows(&mut app).into_iter().collect();
    assert_eq!(
        rows.get("world.falling_skyway.capacity.window_available.label"),
        Some(&supply),
        "capacity AVAILABLE has to be readable from a console, not inferred: Control's \
         sheet carried {rows:?}"
    );
    assert_eq!(
        rows.get("world.falling_skyway.capacity.window_demand.label"),
        Some(&demand),
        "…and so does capacity DEMANDED"
    );
    assert_eq!(
        rows.get("world.falling_skyway.capacity.window_shortfall.label"),
        Some(&short)
    );

    // AC4: the shortfall is STATED, in mission copy, at the moment it opens.
    let said = skyway_messages(&app, WINDOW_CONTROL)
        .last()
        .map(|m| m.body.clone())
        .expect("Control hails the crew when the window opens");
    assert_eq!(
        said, "world.falling_skyway.comms.window_opens_short",
        "a crew who did everything get the 'short by a workable margin' body, and it says \
         they can put two up and not three"
    );

    // The act's objective is on the list while the window is open.
    assert_eq!(
        objective_status(&app, "obj-a3-window"),
        ObjectiveStatus::Active
    );
    assert_eq!(
        objective_status(&app, "obj-a3-window-ready"),
        ObjectiveStatus::Completed,
        "the run-up beat resolves when the thing it was a run-up to arrives"
    );

    // The duration is on the panel for as long as the window lasts — the second
    // half of AC1, and the reason the pair of deadlines is authored rather than
    // one deadline and a hidden timer.
    let running = window_countdown(&mut app, "skyway_window_closes")
        .expect("the closing deadline is visible too");
    assert!(
        running.remaining_secs > 0
            && (running.remaining_secs as f64) <= (closes_at - opens_at) + 2.0,
        "with the window open, the closing deadline IS its duration: {} s of an authored \
         {} s window",
        running.remaining_secs,
        closes_at - opens_at
    );

    // ── The ledger the choice scene has to obey (issue #1043's AC1) ─────────
    //
    // A lift asked for early, and it lands well inside the window.
    let committee_claim = manifest["skyway_claim_committee"];
    window_book(&mut app, "skyway_book_committee");
    assert_eq!(skyway_flag(&app, "skyway_window_lifts_started"), 1);
    assert_eq!(
        window_manifest(&mut app)["skyway_window_available"],
        supply - committee_claim,
        "booking a lift spends the window's lift, on the panel, the moment it is booked"
    );
    assert_eq!(
        window_manifest(&mut app)["skyway_window_committed"],
        committee_claim
    );
    let lands_by = window_now(&app) + 28.0;
    window_run_to(&mut app, lands_by);
    assert_eq!(
        skyway_flag(&app, "skyway_window_served_committee"),
        1,
        "a climber that reaches the top inside the window LANDS"
    );
    assert_eq!(skyway_flag(&app, "skyway_window_lifts_landed"), 1);

    // A second one, booked with twelve seconds left — so it is on the ribbon
    // when the window shuts.
    window_run_to(&mut app, closes_at - 12.0);
    let convoy_claim = manifest["skyway_claim_convoy"];
    window_book(&mut app, "skyway_book_convoy");
    assert_eq!(skyway_flag(&app, "skyway_window_lifts_started"), 2);
    assert_eq!(
        window_manifest(&mut app)["skyway_window_committed"],
        committee_claim + convoy_claim
    );

    // And a third, which the ARITHMETIC refuses: what is left of the window is
    // less than this claimant asked for. This is the constraint #1043's dialogue
    // is required to be enforced by rather than to hard-code.
    let left = window_manifest(&mut app)["skyway_window_available"];
    assert!(
        left < manifest["skyway_claim_havelock"],
        "precondition: the window has {left} left against a {} claim",
        manifest["skyway_claim_havelock"]
    );
    window_book(&mut app, "skyway_book_havelock");
    assert_eq!(
        skyway_flag(&app, "skyway_window_refused_short"),
        1,
        "three claimants and lift for two: the third is refused by the numbers"
    );
    assert_eq!(
        skyway_flag(&app, "skyway_window_lifts_started"),
        2,
        "…and refusing is refusing — nothing was booked"
    );

    // ── AC5: it shuts on schedule, and the lift on the ribbon stands down ───
    window_run_to(&mut app, closes_at + 3.0);
    assert_eq!(skyway_flag(&app, "skyway_window_closed"), 1);
    assert_eq!(skyway_flag(&app, "skyway_window_open"), 0);
    assert_eq!(
        window_manifest(&mut app)["skyway_window_available"],
        0,
        "the lift on offer goes with the window"
    );
    // THE AUTHORED CLOSE RULE: abort, not complete and not partial.
    window_run_to(&mut app, closes_at + 18.0);
    assert_eq!(
        skyway_flag(&app, "skyway_window_stood_down_convoy"),
        1,
        "a lift still on the ribbon when the window shuts is STOOD DOWN — the authored \
         rule, not an accident of what happened to be running"
    );
    assert_eq!(skyway_flag(&app, "skyway_window_served_convoy"), 0);
    assert_eq!(skyway_flag(&app, "skyway_window_lifts_aborted"), 1);
    assert_eq!(
        window_manifest(&mut app)["skyway_window_committed"],
        committee_claim,
        "…and the manifest ends the act saying what actually went up, not what was booked"
    );

    // The OTHER refusal — asking after the shutters are down — is not reachable
    // here, and that is a fact about the act rather than a gap in the test: a
    // claimant's seam is ONE ask, and all three have now been spent. It is
    // `probe_window.toml`'s rung, which fields a spare claimant for the purpose.
    assert_eq!(skyway_flag(&app, "skyway_window_refused_shut"), 0);
    assert_eq!(skyway_flag(&app, "skyway_window_refused_short"), 1);
    assert_eq!(
        objective_status(&app, "obj-a3-window"),
        ObjectiveStatus::Completed
    );
    assert_eq!(
        skyway_messages(&app, WINDOW_CONTROL)
            .last()
            .map(|m| m.body.clone()),
        Some("world.falling_skyway.comms.window_closes".to_string())
    );
}

/// **Issue #1042, AC2/AC7 — the same window, a different mission behind it.**
///
/// This crew caught the tether and nothing else. The strike is still on and
/// Ladder B is still under its own line, so the ladder delivers one rung's worth
/// instead of two: the supply figure is DIFFERENT from the run above, the
/// shortfall is bigger, and both are non-zero. Capacity is a function of the
/// mission, and two missions produce two numbers.
///
/// It is also the control on the OTHER pairing. A stabilise carries no
/// `work_stoppage` interrupt where a field repair does, so this crew keep the
/// head certified with the picket still up — proving that what the ladder is
/// short of here is the RUNG and not the catch.
#[test]
fn a_crew_who_saved_the_head_and_nothing_else_reach_the_window_shorter() {
    let (mut app, ship) = skyway_at_act_two();
    let (_, ship_uuid) = window_ship(&mut app);

    let projection = skyway_deadline_secs(&app, "skyhook_failure_due") as f64;
    let watch_opens = skyway_deadline_secs(&app, "storm_passed_due") as f64;
    window_run_to(&mut app, watch_opens + 4.0);
    window_hold_the_tether(&mut app, ship, &ship_uuid, projection);

    assert_eq!(
        skyway_flag(&app, "skyhook_lift_capable"),
        1,
        "the head is held and still certified to lift, with nobody having settled anything"
    );
    assert_eq!(
        skyway_flag(&app, "skyway_strike_settled"),
        0,
        "…and nothing else was done: the workers are still out"
    );
    assert_eq!(
        skyway_flag(&app, "depot_b_pumping"),
        0,
        "…and the rung is still under its own line"
    );

    skyway_move(&mut app, ship, WINDOW_STATION);
    let opens_at = skyway_deadline_secs(&app, "skyway_transfer_window") as f64;
    window_run_to(&mut app, opens_at - 4.0);
    let _ = window_open_time(&mut app, opens_at + 10.0);

    let rung_a = window_capacity_of(&mut app, WINDOW_DEPOT_A, "depot_a_fuel_lift");
    let supply = skyway_flag(&app, "skyway_window_supply");
    let demand = skyway_flag(&app, "skyway_window_demand_total");
    let short = skyway_flag(&app, "skyway_window_shortfall_at_open");

    assert_eq!(
        supply, rung_a,
        "ONE rung delivers. Ladder A is above its line and its people never walked out; \
         Ladder B fails both halves of the same rule"
    );
    assert!(short > 0, "and it is short: {supply} against {demand}");
    assert_eq!(
        window_manifest(&mut app)["skyway_window_available"],
        supply,
        "the panel says the same thing"
    );
    assert_eq!(
        skyway_messages(&app, WINDOW_CONTROL)
            .last()
            .map(|m| m.body.clone()),
        Some("world.falling_skyway.comms.window_opens_short".to_string()),
        "there is lift in this window and it is not enough — the same news as the run \
         above, told against a different number"
    );
}

/// **Issue #1042, AC2/AC4 — the third outcome, and the one an unattended bridge
/// produces.**
///
/// Nobody mended anything and nobody caught the tether, so #1040's strain walks
/// the head to its structural floor and `destroy_entity` takes it out of the
/// world a little over twenty seconds before this window opens. THE SKYHOOK
/// FELL, so there is no lift — whatever the ladder is doing, and Ladder A is
/// pumping perfectly well throughout.
///
/// That is the skyhook's fate entering the window's arithmetic at full strength,
/// and it arrives through one flag this act already read before the collapse
/// slice existed: `skyhook_lift_capable`. Nothing here names a collapse flag, a
/// warning rung or a debris anchor. The two halves of Act 3 meet at a number.
///
/// Supply zero is a third distinct reading off a third distinct mission, and the
/// crew are told about it in its own words rather than in the ones written for a
/// window that has something in it.
#[test]
fn a_crew_who_mended_nothing_reach_a_window_with_no_lift_in_it_at_all() {
    let probe = build_headless_app(&skyway_args(SKYWAY_DT, 1.0)).expect("the world must load");
    let opens_at = skyway_deadline_secs(&probe, "skyway_transfer_window") as f64;
    drop(probe);

    let mut app =
        build_headless_app(&skyway_args(SKYWAY_DT, opens_at + 12.0)).expect("the world must load");
    run(&mut app, 10);
    let (ship, _) = window_ship(&mut app);
    window_run_to(&mut app, opens_at - 20.0);
    // On station, so the hail reaches them. Nothing else about this run is
    // steered: it is the mission running with nobody at the consoles.
    skyway_move(&mut app, ship, WINDOW_STATION);
    let _ = window_open_time(&mut app, opens_at + 10.0);

    assert_eq!(
        skyway_flag(&app, "skyway_skyhook_lost"),
        1,
        "precondition: nobody caught the tether, so #1040's floor took the head"
    );
    assert!(
        !named_entity_present(&mut app, WINDOW_HEAD),
        "…and it is out of the world, not merely broken"
    );
    assert_eq!(
        skyway_flag(&app, "skyhook_lift_capable"),
        0,
        "a head that is gone is certified for nothing"
    );
    assert_eq!(
        skyway_flag(&app, "depot_a_pumping"),
        1,
        "…while the working rung went on working, which is what makes this a fact about \
         the HEAD rather than about the ladder"
    );
    assert_eq!(
        skyway_flag(&app, "skyway_window_supply"),
        0,
        "a chain delivers what its worst link delivers, and this one is not certified to \
         lift anything at all"
    );
    let demand = skyway_flag(&app, "skyway_window_demand_total");
    assert_eq!(
        skyway_flag(&app, "skyway_window_shortfall_at_open"),
        demand,
        "every unit of it is short"
    );
    let manifest = window_manifest(&mut app);
    assert_eq!(manifest["skyway_window_available"], 0);
    assert_eq!(manifest["skyway_window_shortfall"], demand);
    assert_eq!(
        skyway_messages(&app, WINDOW_CONTROL)
            .last()
            .map(|m| m.body.clone()),
        Some("world.falling_skyway.comms.window_opens_dead".to_string()),
        "a window with nothing in it gets its own body: the crew are not read a line \
         about picking two"
    );
}

/// **Issue #1042 — the mechanism under the scene.**
///
/// `probe_window.toml` driven for forty mission seconds. Its own header carries
/// the four chains, the timeline and what each rung isolates; what this test
/// asserts is that the four chains price DIFFERENTLY from the same function,
/// that the `min` binds on both sides across the matrix, and that the window's
/// ledger grants, refuses twice for two different reasons, lands one climber
/// and stands the other down.
///
/// It is also the only test of `ctx.effects.adjust_capacity` as a verb: every
/// register below starts at an authored zero and is written by script, so a run
/// where they stay at zero is a run where the new host function did nothing.
#[test]
fn probe_window_prices_four_chains_and_stands_down_what_it_cannot_finish() {
    let dt = 1.0 / 60.0;
    let args = HeadlessArgs {
        world_path: "assets/worlds/probe_window.toml".into(),
        ship_path: "assets/entities/alliance_cruiser.toml".into(),
        dt,
        max_ticks: ticks_for_sim_seconds(40.0, dt),
        deterministic: true,
        seed: Some(1042),
        ..test_args()
    };
    let mut app = build_headless_app(&args).expect("the probe world must load and build");

    // The first mission second at which each marker went up, sampled tick by
    // tick — so the ORDER below is causal rather than arithmetic on a schedule
    // this test wrote out again.
    let mut first: std::collections::BTreeMap<String, f64> = Default::default();
    // The four supplies as they stood on the tick the window opened, before any
    // booking had spent one of them.
    let mut at_open: std::collections::BTreeMap<String, i64> = Default::default();
    let markers = [
        "probe_window_open",
        "probe_window_closed",
        "probe_lifts_started",
        "served_alpha",
        "stood_down_beta",
        "probe_refused_short",
        "probe_refused_shut",
        "probe_lifts_aborted",
    ];

    for tick in 0..args.max_ticks {
        run(&mut app, 1);
        let now = (tick + 1) as f64 * dt;
        for marker in markers {
            if skyway_flag(&app, marker) > 0 {
                first.entry(marker.to_string()).or_insert(now);
            }
        }
        if at_open.is_empty() && skyway_flag(&app, "probe_window_open") > 0 {
            // One tick later: the publish is queued by the deadline handler and
            // applied by `tick_infrastructure_condition`, which is the same
            // one-tick bridge every condition move in this engine rides.
            run(&mut app, 2);
            for row in [
                "full_available",
                "full_demand",
                "full_shortfall",
                "struck_available",
                "low_available",
                "dead_available",
            ] {
                at_open.insert(row.to_string(), skyway_flag(&app, row));
            }
        }
    }

    let at = |name: &str| {
        *first
            .get(name)
            .unwrap_or_else(|| panic!("'{name}' never happened; the run recorded {first:?}"))
    };

    // ── The window is a window ──────────────────────────────────────────────
    assert!(
        (at("probe_window_open") - 10.0).abs() < 0.5,
        "the window opens on its authored deadline: {:.2} s",
        at("probe_window_open")
    );
    assert!(
        (at("probe_window_closed") - 30.0).abs() < 0.5,
        "…and shuts on its own: {:.2} s",
        at("probe_window_closed")
    );

    // ── AC2: four chains, four prices, from one function ────────────────────
    //
    // The two heads and the four rungs are read back off their own counters, so
    // the expected numbers below are the world's arithmetic rather than this
    // test's copy of it.
    let berths = skyway_flag(&app, "berths_certified");
    let load = skyway_flag(&app, "load_certified");
    let up = skyway_flag(&app, "rung_up_lift");
    let worked = skyway_flag(&app, "rung_worked_lift");
    assert_eq!(
        at_open["full_available"],
        (berths * load).min(up + worked),
        "the full chain is priced at the smaller of the head and the ladder"
    );
    assert_eq!(
        at_open["full_available"],
        berths * load,
        "…and on THIS chain the HEAD is the smaller of the two, which is the half of the \
         `min` a ladder-always world would never exercise"
    );
    assert_eq!(
        at_open["struck_available"], up,
        "the struck chain loses its second rung entirely: above its own line, and nobody \
         down there is signing anything"
    );
    assert_eq!(
        at_open["low_available"], up,
        "…and the under-line chain loses its second rung for the OTHER reason, with the \
         same people at work on it"
    );
    assert_eq!(
        at_open["dead_available"], 0,
        "a head that is not certified to lift lifts nothing, whatever the ladder is doing"
    );
    assert!(
        at_open["struck_available"] < at_open["full_available"],
        "the three chains must actually differ, or the matrix is measuring nothing"
    );

    // AC3/AC4: demand and the shortfall, published beside the supply.
    let demand = skyway_flag(&app, "claim_alpha")
        + skyway_flag(&app, "claim_beta")
        + skyway_flag(&app, "claim_gamma")
        + skyway_flag(&app, "claim_delta");
    assert_eq!(at_open["full_demand"], demand);
    assert_eq!(
        at_open["full_shortfall"],
        demand - at_open["full_available"]
    );
    assert!(at_open["full_shortfall"] > 0);

    // ── The ledger: two grants and two refusals, for two different reasons ──
    assert!(
        at("probe_lifts_started") > at("probe_window_open"),
        "nothing is booked before there is a window to book it in"
    );
    assert_eq!(
        skyway_flag(&app, "probe_lifts_started"),
        2,
        "alpha and beta were granted; gamma and delta were not"
    );
    assert_eq!(
        skyway_flag(&app, "probe_refused_short"),
        1,
        "gamma asked for more than the window had left — refused by the ARITHMETIC"
    );
    assert!(
        at("probe_refused_short") < at("probe_window_closed"),
        "…and refused while the window was still open, so it is a refusal about capacity \
         and not about the clock"
    );
    assert_eq!(
        skyway_flag(&app, "probe_refused_shut"),
        1,
        "delta asked after the shutters were down — the other refusal, counted apart"
    );
    assert!(
        at("probe_refused_shut") > at("probe_window_closed"),
        "…and that one IS about the clock"
    );

    // ── AC5: the authored close rule ────────────────────────────────────────
    assert_eq!(
        skyway_flag(&app, "served_alpha"),
        1,
        "a climber that reaches the top inside the window LANDS"
    );
    assert!(at("served_alpha") < at("probe_window_closed"));
    assert_eq!(
        skyway_flag(&app, "served_beta"),
        0,
        "beta's climber was still on the ribbon when the window shut"
    );
    assert_eq!(
        skyway_flag(&app, "stood_down_beta"),
        1,
        "…so it is STOOD DOWN — the authored rule, and neither completed nor part-paid"
    );
    assert!(
        at("stood_down_beta") > at("probe_window_closed"),
        "the abort happens at the landing that would have been, not at the close"
    );
    assert_eq!(skyway_flag(&app, "probe_lifts_landed"), 1);
    assert_eq!(skyway_flag(&app, "probe_lifts_aborted"), 1);

    // ── The registers end the run saying what happened ──────────────────────
    assert_eq!(
        skyway_flag(&app, "full_committed"),
        skyway_flag(&app, "claim_alpha"),
        "what actually went up, not what was booked: beta's share came back off"
    );
    assert_eq!(
        skyway_flag(&app, "full_available"),
        0,
        "the lift on offer went with the window"
    );
    assert_eq!(
        skyway_flag(&app, "full_demand"),
        demand,
        "…and what was asked for is still on the manifest, because that is a fact about \
         the run rather than a fact about the window"
    );
}

// ── Falling Skyway, Act 3: the choice and the endings (issue #1043, #852) ────
//
// The mission's last scene, walked end to end four ways. Every one of these is a
// WHOLE RUN of `falling_skyway.toml` — booted at the lobby, stepped through Act
// 1's survey, Act 2's dispute and weather, Act 3's tether and its window, and
// out the far side into the debrief — because what this slice is answerable for
// is what the mission HANDS ON, and a handoff assembled from a hand-built world
// state would be a handoff from a run nobody could have played.

/// The civilian convoy's lead hauler — the third claimant, and the only one that
/// has to be flown to the conversation.
const SKYWAY_CONVOY: &str = "world.falling_skyway.entity.convoy_meridian.name";
/// The ladder transit leg. Where all three claimants are inside the smaller of
/// their comms range and the destroyer's, which is why the choice objective
/// carries a Reach directive to it.
const CHOICE_LADDER: bevy::prelude::Vec3 = bevy::prelude::Vec3::new(900.0, 0.0, 40.0);

/// Step the mission to `secs`, holding the hull on `station` the whole way.
///
/// The pin is the honest analogue of helm doing as it is told, and the same
/// substitution every test in this file makes: Act 3's own Reach directives are
/// live throughout and would otherwise fly the ship off the berth the scene
/// needs it on. That the objective NAMES the berth is asserted below; that a
/// backfilled helm gets there is helm's business, not this group's subject.
fn parley_run_to(
    app: &mut bevy::prelude::App,
    ship: bevy::prelude::Entity,
    secs: f64,
    station: bevy::prelude::Vec3,
) {
    while window_now(app) < secs {
        skyway_move(app, ship, station);
        run(app, 10);
    }
    skyway_move(app, ship, station);
    run(app, 2);
}

/// Run past the window's close and the twenty-six seconds the endings wait for
/// the last climber to land or stand down, then assert they were written.
fn run_to_the_endings(
    app: &mut bevy::prelude::App,
    ship: bevy::prelude::Entity,
    station: bevy::prelude::Vec3,
) {
    let closes_at = skyway_deadline_secs(app, "skyway_window_closes") as f64;
    parley_run_to(app, ship, closes_at + 32.0, station);
    assert_eq!(
        skyway_flag(app, "a3_endings_written"),
        1,
        "the mission has to resolve itself exactly once, and after the last lift has \
         landed or stood down"
    );
}

/// One promise's state on the live ledger, as a word.
fn skyway_promise(app: &bevy::prelude::App, id: &str) -> String {
    app.world()
        .resource::<project_phoenix::world::server::WorldContentRuntime>()
        .commitments
        .state_of(id)
        .to_string()
}

/// The last thing `sender` said, which for every closing beat in this band is
/// the thing the crew are left holding.
fn skyway_last_said(app: &bevy::prelude::App, sender: &str) -> String {
    skyway_messages(app, sender)
        .last()
        .map(|m| m.body.clone())
        .unwrap_or_else(|| panic!("{sender} never said anything in this run"))
}

/// One subject's fact sheet as `text` ids, oldest first.
fn skyway_sheet_texts(app: &mut bevy::prelude::App, subject: &str) -> Vec<String> {
    let uuid = scan_uuid_named(app, subject);
    diff_file(app, &uuid)
        .into_iter()
        .map(|(text, _)| text)
        .collect()
}

/// **The invariant every run has to satisfy, whatever the crew did.**
///
/// Four of the six families are EXCLUSIVE — a claimant is carried or is left,
/// the strike ended one of three ways, the evidence is at one of three depths,
/// the structure held or fell — and the promise this slice makes to whatever
/// mission reads them next is that each says exactly one thing rather than
/// leaving the reader to infer an absence. #1037 set that rule for the Lyra and
/// #1040 kept it for the head; this asserts it across the whole handoff.
fn assert_the_campaign_record_is_complete(app: &bevy::prelude::App) {
    let exclusive: [(&str, Vec<&str>); 6] = [
        (
            "the workers",
            vec![
                "campaign.skyway.passage.committee",
                "campaign.skyway.passage.left_committee",
            ],
        ),
        (
            "the operator",
            vec![
                "campaign.skyway.passage.havelock",
                "campaign.skyway.passage.left_havelock",
            ],
        ),
        (
            "the convoy",
            vec![
                "campaign.skyway.passage.convoy",
                "campaign.skyway.passage.left_convoy",
            ],
        ),
        (
            "the strike",
            vec![
                "campaign.skyway.strike.negotiated",
                "campaign.skyway.strike.forced",
                "campaign.skyway.strike.unresolved",
            ],
        ),
        (
            "the evidence",
            vec![
                "campaign.skyway.evidence.none",
                "campaign.skyway.evidence.records",
                "campaign.skyway.evidence.corroborated",
            ],
        ),
        (
            "the structure",
            vec![
                "campaign.skyway.skyhook.held",
                "campaign.skyway.skyhook.lost",
            ],
        ),
    ];
    for (family, members) in exclusive {
        let set: Vec<&str> = members
            .iter()
            .copied()
            .filter(|f| skyway_flag(app, f) > 0)
            .collect();
        assert_eq!(
            set.len(),
            1,
            "{family}: exactly one of {members:?} must be written so a later mission reads \
             a fact rather than an absence — this run wrote {set:?}"
        );
    }

    let total = skyway_flag(app, "campaign.skyway.casualties.total");
    assert_eq!(
        total,
        skyway_flag(app, "campaign.skyway.casualties.picket")
            + skyway_flag(app, "campaign.skyway.casualties.head")
            + skyway_flag(app, "campaign.skyway.casualties.storm"),
        "the itemised casualties have to add up to the number a debrief reads"
    );
    assert_eq!(
        skyway_flag(app, "campaign.skyway.casualties.none"),
        i64::from(total == 0),
        "…and the 'nobody was hurt' bit is that sum, not a separate claim about it"
    );

    let taken = skyway_flag(app, "campaign.skyway.passage.taken");
    assert_eq!(
        taken,
        skyway_flag(app, "campaign.skyway.passage.committee")
            + skyway_flag(app, "campaign.skyway.passage.havelock")
            + skyway_flag(app, "campaign.skyway.passage.convoy")
    );
    assert!(
        taken <= 2,
        "THE WHOLE ACT: three claimants asked and the corridor can never carry all three. \
         This run carried {taken}"
    );
}

/// **Issue #1043, AC1/AC2/AC4/AC5/AC6/AC7 — the best road through the mission,
/// and it still leaves somebody on the rock.**
///
/// This crew did everything the scenario had to offer: they read the rung, got
/// the strike settled at the table, got a worker on the record, mended Ladder B
/// and caught the tether every time it slipped. So they arrive at the window
/// with the whole chain — and 52 units of lift against 66 of claims, which buys
/// two of the three.
///
/// They take the workers they gave their word to and the convoy who had nothing
/// to trade, refuse the operator to their face, and put the operator's own file
/// to them on the open channel without naming the woman who contradicted it.
/// Both promises on the books come out KEPT, and the ledger says so.
#[test]
fn falling_skyway_carries_the_workers_and_the_convoy_and_keeps_the_captains_word() {
    use project_phoenix::core::messages::ObjectiveStatus;

    let (mut app, ship) = skyway_at_act_two();
    let (_, ship_uuid) = window_ship(&mut app);

    // ── Act 2: read the rung, talk them down, get her on the record ──────────
    skyway_scan_ladder_b(&mut app, ship);
    assert_eq!(skyway_flag(&app, "skyway_records_diff_found"), 1);
    skyway_negotiate_to_a_vote(&mut app);
    assert_eq!(skyway_flag(&app, "skyway_settled_by_negotiation"), 1);
    assert_eq!(
        skyway_promise(&app, "skyway_safe_passage"),
        "open",
        "precondition: the captain has given the workers their word about this very window"
    );
    skyway_pick(
        &mut app,
        SKYWAY_RIGGER,
        "world.falling_skyway.comms.rigger_ask",
    );
    skyway_pick(
        &mut app,
        SKYWAY_RIGGER,
        "world.falling_skyway.comms.rigger_protect",
    );
    assert_eq!(
        skyway_flag(&app, "skyway_confront_unlocked"),
        1,
        "precondition: #1039's gate is open, so the confrontation is on this scene's menu"
    );
    assert_eq!(
        objective_status(&app, "obj-a3-confront"),
        ObjectiveStatus::Active,
        "…and #1039 left it Active for this slice to resolve"
    );

    let settled_by = window_now(&app) + 24.0;
    window_run_to(&mut app, settled_by);
    assert_eq!(skyway_flag(&app, "skyway_strike_settled"), 1);

    // ── Act 2's other work, and Act 3's: mend the rung, hold the head ────────
    window_field_repair(
        &mut app,
        ship,
        &ship_uuid,
        SKYWAY_DEPOT_B,
        bevy::prelude::Vec3::new(1180.0, 0.0, 300.0),
    );
    assert_eq!(skyway_flag(&app, "depot_b_pumping"), 1);

    let projection = skyway_deadline_secs(&app, "skyhook_failure_due") as f64;
    let watch_opens = skyway_deadline_secs(&app, "storm_passed_due") as f64;
    window_run_to(&mut app, watch_opens + 4.0);
    window_hold_the_tether(&mut app, ship, &ship_uuid, projection);
    assert_eq!(skyway_flag(&app, "skyhook_lift_capable"), 1);

    // ── The parley. Helm closes on the ladder, because that is where they are ─
    let opens_at = skyway_deadline_secs(&app, "skyway_transfer_window") as f64;
    parley_run_to(&mut app, ship, opens_at + 6.0, CHOICE_LADDER);

    assert_eq!(
        objective_status(&app, "obj-a3-parley"),
        ObjectiveStatus::Completed,
        "AC1: the scene posts an approach that makes the conversation answerable — the \
         committee are 947 units off the act's own station-keeping berth against a \
         900-unit channel — and, being the objective that carries the Reach directive, \
         it completes on arrival. That is why the DECIDING is a second objective"
    );
    assert_eq!(
        objective_status(&app, "obj-a3-choice"),
        ObjectiveStatus::Active
    );
    let supply = skyway_flag(&app, "skyway_window_supply");
    assert_eq!(
        supply, 52,
        "the whole chain: both rungs pumping under a certified head"
    );

    // ── AC2: each of them asks, from its own hull ───────────────────────────
    let committee = skyway_open_node(&app, SKYWAY_COMMITTEE);
    assert_eq!(
        committee.body,
        "world.falling_skyway.comms.committee_claims"
    );
    let havelock = skyway_open_node(&app, SKYWAY_CUTTER);
    assert_eq!(havelock.body, "world.falling_skyway.comms.havelock_claims");
    let convoy = skyway_open_node(&app, SKYWAY_CONVOY);
    assert_eq!(
        convoy.body, "world.falling_skyway.comms.convoy_claims",
        "the convoy asks for itself too, which is why the act marshals them onto the \
         ladder run at the top of it"
    );
    assert_eq!(
        skyway_options(&committee).first().map(String::as_str),
        Some("world.falling_skyway.comms.claim_stand_by"),
        "INDEX 0 IS THE HOLD on every one of these trees: an AI-backfilled Tactical seat \
         answers an open thread with its first response, and an empty chair must not be \
         able to decide who rides the storm out"
    );

    // AC2: the confrontation is on the operator's tree, and only because the
    // crew earned both halves of the case.
    assert!(
        skyway_options(&havelock)
            .contains(&"world.falling_skyway.comms.confront_unnamed".to_string()),
        "the confront option appears for a crew holding the record AND the witness: \
         {:?}",
        skyway_options(&havelock)
    );

    // ── The choice ──────────────────────────────────────────────────────────
    skyway_pick(
        &mut app,
        SKYWAY_COMMITTEE,
        "world.falling_skyway.comms.lift_committee",
    );
    skyway_pick(
        &mut app,
        SKYWAY_CONVOY,
        "world.falling_skyway.comms.lift_convoy",
    );
    run(&mut app, 8);
    assert_eq!(
        skyway_flag(&app, "skyway_window_lifts_started"),
        2,
        "both picks reached #1042's booking seam and both were granted"
    );
    assert_eq!(
        skyway_flag(&app, "skyway_window_reserved"),
        18 + 26,
        "…and spent 44 of the 52 on the ledger the third claimant is measured against"
    );

    // AC1 IN ITS STRONGEST FORM. A node is built once and answered later, so the
    // parties watch the manifest and say when it moves — and the tree they say
    // it on is rebuilt from the ledger as it now stands. The operator's own
    // screen no longer carries a lift line, and nothing in this file counted to
    // two to arrive at that.
    let repriced_by = window_now(&app) + 6.0;
    parley_run_to(&mut app, ship, repriced_by, CHOICE_LADDER);
    let havelock_now = skyway_open_node(&app, SKYWAY_CUTTER);
    assert_eq!(
        havelock_now.body, "world.falling_skyway.comms.havelock_reprices",
        "the operator noticed the board move"
    );
    assert!(
        !skyway_options(&havelock_now)
            .contains(&"world.falling_skyway.comms.lift_havelock".to_string()),
        "22 against 8 left: the option is GONE, because the arithmetic says so. \
         Offered: {:?}",
        skyway_options(&havelock_now)
    );
    skyway_pick(
        &mut app,
        SKYWAY_CUTTER,
        "world.falling_skyway.comms.confront_unnamed",
    );
    assert_eq!(skyway_flag(&app, "skyway_confronted"), 1);
    assert_eq!(
        skyway_flag(&app, "skyway_witness_named"),
        0,
        "her name stayed out of it, which is the promise she was given"
    );
    assert_eq!(
        objective_status(&app, "obj-a3-confront"),
        ObjectiveStatus::Completed
    );
    skyway_pick(
        &mut app,
        SKYWAY_CUTTER,
        "world.falling_skyway.comms.deny_havelock",
    );

    // ── The endings ─────────────────────────────────────────────────────────
    run_to_the_endings(&mut app, ship, CHOICE_LADDER);

    assert_eq!(skyway_flag(&app, "skyway_window_served_committee"), 1);
    assert_eq!(skyway_flag(&app, "skyway_window_served_convoy"), 1);
    assert_eq!(skyway_flag(&app, "skyway_window_served_havelock"), 0);

    // AC4: the ledger, settled at this scene, both promises kept.
    assert_eq!(
        skyway_promise(&app, "skyway_safe_passage"),
        "kept",
        "the workers were given the corridor and the workers got the corridor"
    );
    assert_eq!(skyway_promise(&app, "skyway_protect_witness"), "kept");
    assert_eq!(
        skyway_promise(&app, "skyway_surface_records"),
        "unknown",
        "a promise that was never made is not a promise that was broken — the file was \
         shown rather than sworn about, which is what saved the captain this one"
    );
    assert_eq!(skyway_flag(&app, "commitment.skyway_safe_passage.kept"), 1);

    // AC3: the excluded party's fate lands on their own channel, and what it
    // cost is read off the mission rather than rolled.
    assert_eq!(
        skyway_flag(&app, "skyway_left_havelock_cost"),
        1,
        "the operator's people ride it out aboard the cutter and the picket, both still \
         hulls with power on them, on a corridor whose head is standing and whose rung \
         is moving: two for being left, less one for the picket nobody shot at"
    );
    assert_eq!(
        skyway_last_said(&app, SKYWAY_CUTTER),
        "world.falling_skyway.comms.havelock_rides_it_out",
        "…and they say so themselves, on the channel the crew refused them on"
    );

    // AC5/AC7: the mission reads finished — a headline naming the pairing, and a
    // fact sheet saying what became of the record and of the captain's word.
    assert_eq!(
        skyway_last_said(&app, WINDOW_CONTROL),
        "world.falling_skyway.comms.debrief_workers_and_convoy"
    );
    let sheet = skyway_sheet_texts(&mut app, WINDOW_CONTROL);
    assert!(
        sheet.contains(&"world.falling_skyway.evidence.record_filed".to_string()),
        "the record went to Control and her name did not go with it: {sheet:?}"
    );
    assert!(sheet.contains(&"world.falling_skyway.evidence.word_kept".to_string()));

    assert_eq!(
        objective_status(&app, "obj-a3-choice"),
        ObjectiveStatus::Completed,
        "all three were answered — two carried and one told to its face"
    );

    // ── AC6: the six families ───────────────────────────────────────────────
    assert_the_campaign_record_is_complete(&app);
    assert_eq!(skyway_flag(&app, "campaign.skyway.passage.committee"), 1);
    assert_eq!(skyway_flag(&app, "campaign.skyway.passage.convoy"), 1);
    assert_eq!(
        skyway_flag(&app, "campaign.skyway.passage.left_havelock"),
        1
    );
    assert_eq!(
        skyway_flag(&app, "campaign.skyway.passage.refused_havelock"),
        1
    );
    assert_eq!(skyway_flag(&app, "campaign.skyway.passage.taken"), 2);
    assert_eq!(skyway_flag(&app, "campaign.skyway.strike.negotiated"), 1);
    assert_eq!(
        skyway_flag(&app, "campaign.skyway.evidence.corroborated"),
        1
    );
    assert_eq!(skyway_flag(&app, "campaign.skyway.evidence.filed"), 1);
    assert_eq!(skyway_flag(&app, "campaign.skyway.evidence.confronted"), 1);
    assert_eq!(
        skyway_flag(&app, "campaign.skyway.evidence.witness_named"),
        0
    );
    assert_eq!(skyway_flag(&app, "campaign.skyway.casualties.picket"), 0);
    assert_eq!(skyway_flag(&app, "campaign.skyway.casualties.head"), 0);
    assert_eq!(skyway_flag(&app, "campaign.skyway.casualties.storm"), 1);
    assert_eq!(skyway_flag(&app, "campaign.skyway.casualties.total"), 1);
    assert_eq!(skyway_flag(&app, "campaign.skyway.skyhook.held"), 1);
    assert_eq!(skyway_flag(&app, "campaign.skyway.commitments.kept"), 2);
    assert_eq!(skyway_flag(&app, "campaign.skyway.commitments.broken"), 0);
    assert_eq!(skyway_flag(&app, "campaign.skyway.commitments.clean"), 1);
}

/// **Issue #1043, AC4 — the captain who promised these people the corridor and
/// then left them standing on the rung, told so BY NAME.**
///
/// A different mission behind the same window: the word was given at the table
/// and then the picket was cleared over their heads anyway, which is where
/// #1036 broke the promise. Nothing this crew did afterwards found the file, so
/// the confrontation is not on the menu. They carry the operator and the convoy
/// and refuse the workers.
///
/// What comes back is not a severity band. It is the committee reading the terms
/// of the promise back to the ship that gave them.
#[test]
fn a_captain_who_leaves_the_workers_behind_hears_the_promise_read_back() {
    use project_phoenix::core::messages::ObjectiveStatus;

    let (mut app, ship) = skyway_at_act_two();
    let (_, ship_uuid) = window_ship(&mut app);

    // ── The word, given ─────────────────────────────────────────────────────
    skyway_pick(
        &mut app,
        SKYWAY_COMMITTEE,
        "world.falling_skyway.comms.listen",
    );
    skyway_pick(
        &mut app,
        SKYWAY_COMMITTEE,
        "world.falling_skyway.comms.promise_passage",
    );
    assert_eq!(skyway_promise(&app, "skyway_safe_passage"), "open");

    // ── …and the picket cleared over their heads anyway ─────────────────────
    skyway_pick(
        &mut app,
        SKYWAY_CUTTER,
        "world.falling_skyway.comms.force_warned",
    );
    let cleared_by = window_now(&app) + 18.0;
    window_run_to(&mut app, cleared_by);
    assert_eq!(skyway_flag(&app, "skyway_forced_open"), 1);
    assert_eq!(skyway_flag(&app, "skyway_strike_settled"), 1);
    assert_eq!(
        skyway_promise(&app, "skyway_safe_passage"),
        "broken",
        "precondition: #1036 broke it at the order, and this scene must not re-resolve \
         a promise somebody else already settled"
    );
    assert_eq!(skyway_flag(&app, "skyway_force_casualties"), 1);

    // Mend the rung the boarding party damaged, and hold the head, so the window
    // has enough in it for the pair this run is about (22 + 26 = 48).
    window_field_repair(
        &mut app,
        ship,
        &ship_uuid,
        SKYWAY_DEPOT_B,
        bevy::prelude::Vec3::new(1180.0, 0.0, 300.0),
    );
    let projection = skyway_deadline_secs(&app, "skyhook_failure_due") as f64;
    let watch_opens = skyway_deadline_secs(&app, "storm_passed_due") as f64;
    window_run_to(&mut app, watch_opens + 4.0);
    window_hold_the_tether(&mut app, ship, &ship_uuid, projection);

    let opens_at = skyway_deadline_secs(&app, "skyway_transfer_window") as f64;
    parley_run_to(&mut app, ship, opens_at + 6.0, CHOICE_LADDER);

    // A crew who never went looking are not offered the file line, on either
    // tree — the evidence branch reads the fact sheet, and theirs is empty.
    let havelock = skyway_open_node(&app, SKYWAY_CUTTER);
    assert!(
        !skyway_options(&havelock).contains(&"world.falling_skyway.comms.put_the_file".to_string()),
        "nothing to put: {:?}",
        skyway_options(&havelock)
    );
    assert!(!skyway_options(&havelock)
        .contains(&"world.falling_skyway.comms.confront_unnamed".to_string()));

    skyway_pick(
        &mut app,
        SKYWAY_CUTTER,
        "world.falling_skyway.comms.lift_havelock",
    );
    skyway_pick(
        &mut app,
        SKYWAY_CONVOY,
        "world.falling_skyway.comms.lift_convoy",
    );
    skyway_pick(
        &mut app,
        SKYWAY_COMMITTEE,
        "world.falling_skyway.comms.deny_committee",
    );

    run_to_the_endings(&mut app, ship, CHOICE_LADDER);

    // ── AC4, and the reason this test exists ────────────────────────────────
    assert_eq!(
        skyway_last_said(&app, SKYWAY_COMMITTEE),
        "world.falling_skyway.comms.committee_rides_it_out_promised",
        "the workers do not report a casualty count at a captain who gave them their \
         word and then left them on the rung. They read the promise back."
    );
    assert_eq!(
        skyway_flag(&app, "skyway_left_committee_cost"),
        3,
        "two for being left, one more for a rung their own people were cleared off at \
         somebody else's authorisation"
    );

    assert_eq!(
        skyway_last_said(&app, WINDOW_CONTROL),
        "world.falling_skyway.comms.debrief_operator_and_convoy"
    );
    let sheet = skyway_sheet_texts(&mut app, WINDOW_CONTROL);
    assert!(sheet.contains(&"world.falling_skyway.evidence.record_never_found".to_string()));
    assert!(sheet.contains(&"world.falling_skyway.evidence.word_broken".to_string()));
    assert_eq!(
        objective_status(&app, "obj-a3-choice"),
        ObjectiveStatus::Completed
    );

    assert_the_campaign_record_is_complete(&app);
    assert_eq!(
        skyway_flag(&app, "campaign.skyway.passage.left_committee"),
        1
    );
    assert_eq!(
        skyway_flag(&app, "campaign.skyway.passage.refused_committee"),
        1
    );
    assert_eq!(skyway_flag(&app, "campaign.skyway.passage.havelock"), 1);
    assert_eq!(skyway_flag(&app, "campaign.skyway.passage.convoy"), 1);
    assert_eq!(skyway_flag(&app, "campaign.skyway.strike.forced"), 1);
    assert_eq!(skyway_flag(&app, "campaign.skyway.evidence.none"), 1);
    assert_eq!(skyway_flag(&app, "campaign.skyway.evidence.filed"), 0);
    assert_eq!(skyway_flag(&app, "campaign.skyway.casualties.picket"), 1);
    assert_eq!(skyway_flag(&app, "campaign.skyway.casualties.storm"), 3);
    assert_eq!(skyway_flag(&app, "campaign.skyway.casualties.total"), 4);
    assert_eq!(skyway_flag(&app, "campaign.skyway.skyhook.held"), 1);
    assert_eq!(skyway_flag(&app, "campaign.skyway.commitments.kept"), 0);
    assert_eq!(skyway_flag(&app, "campaign.skyway.commitments.broken"), 1);
    assert_eq!(skyway_flag(&app, "campaign.skyway.commitments.clean"), 0);
}

/// **Issue #1043, AC1 — the third claimant refused BY THE ARITHMETIC, in front
/// of the captain, before they could pick it.**
///
/// The strike is talked down and nothing else is mended, so the window opens on
/// one rung's worth of lift: 40 units, which is exactly the workers' 18 and the
/// operator's 22 and not one unit more. The captain takes both — and the
/// convoy's tree, built from the same ledger a tick later, has no lift line on
/// it at all.
///
/// This is the acceptance criterion in its literal form. Nothing counted to two.
/// The two most expensive claims were 22 and 26 and the numbers decided which
/// pair fitted.
///
/// It is also the mixed-ledger run: the captain gave both promises to get the
/// vote, kept the one about the corridor and broke the one about the file.
#[test]
fn the_lift_runs_out_and_the_third_claimant_is_never_offered_one() {
    use project_phoenix::core::messages::ObjectiveStatus;

    let (mut app, ship) = skyway_at_act_two();
    let (_, ship_uuid) = window_ship(&mut app);

    skyway_negotiate_to_a_vote(&mut app);
    assert_eq!(
        skyway_promise(&app, "skyway_surface_records"),
        "open",
        "precondition: an empty-handed crew give BOTH promises to get the vote called"
    );
    let settled_by = window_now(&app) + 24.0;
    window_run_to(&mut app, settled_by);
    assert_eq!(skyway_flag(&app, "skyway_strike_settled"), 1);

    let projection = skyway_deadline_secs(&app, "skyhook_failure_due") as f64;
    let watch_opens = skyway_deadline_secs(&app, "storm_passed_due") as f64;
    window_run_to(&mut app, watch_opens + 4.0);
    window_hold_the_tether(&mut app, ship, &ship_uuid, projection);

    let opens_at = skyway_deadline_secs(&app, "skyway_transfer_window") as f64;
    parley_run_to(&mut app, ship, opens_at + 6.0, CHOICE_LADDER);
    assert_eq!(
        skyway_flag(&app, "skyway_window_supply"),
        40,
        "precondition: Ladder B was never mended, so the ladder puts in one rung's worth"
    );

    // Every option is on the table before anything is spent.
    for (sender, line) in [
        (
            SKYWAY_COMMITTEE,
            "world.falling_skyway.comms.lift_committee",
        ),
        (SKYWAY_CUTTER, "world.falling_skyway.comms.lift_havelock"),
        (SKYWAY_CONVOY, "world.falling_skyway.comms.lift_convoy"),
    ] {
        let node = skyway_open_node(&app, sender);
        assert!(
            skyway_options(&node).contains(&line.to_string()),
            "with 40 unspent every single claim fits on its own: {sender} offered {:?}",
            skyway_options(&node)
        );
        assert!(
            node.sender_in_range,
            "{sender} has to be answerable from the ladder leg the scene sends helm to, \
             or the choice is not a choice — they are at {:?}",
            skyway_position(&mut app, sender)
        );
    }

    skyway_pick(
        &mut app,
        SKYWAY_COMMITTEE,
        "world.falling_skyway.comms.lift_committee",
    );
    skyway_pick(
        &mut app,
        SKYWAY_CUTTER,
        "world.falling_skyway.comms.lift_havelock",
    );
    run(&mut app, 8);
    assert_eq!(skyway_flag(&app, "skyway_window_reserved"), 40);

    // The convoy watch the same board the captain does, and it just emptied.
    let repriced_by = window_now(&app) + 6.0;
    parley_run_to(&mut app, ship, repriced_by, CHOICE_LADDER);
    let convoy = skyway_open_node(&app, SKYWAY_CONVOY);
    assert_eq!(convoy.body, "world.falling_skyway.comms.convoy_reprices");
    assert!(
        !skyway_options(&convoy).contains(&"world.falling_skyway.comms.lift_convoy".to_string()),
        "AC1: the ledger is empty and the option is gone. The dialogue never counted to \
         two; it asked the window what was left. Offered: {:?}",
        skyway_options(&convoy)
    );
    assert!(
        skyway_options(&convoy).contains(&"world.falling_skyway.comms.deny_convoy".to_string()),
        "…and telling them so is still a thing the captain has to do"
    );
    skyway_pick(
        &mut app,
        SKYWAY_CONVOY,
        "world.falling_skyway.comms.deny_convoy",
    );

    run_to_the_endings(&mut app, ship, CHOICE_LADDER);

    assert_eq!(
        skyway_flag(&app, "skyway_window_refused_short"),
        0,
        "the backstop never had to fire: the option was withheld before it could be picked"
    );
    assert_eq!(
        skyway_flag(&app, "skyway_left_convoy_cost"),
        2,
        "a party with no ground of its own, on a corridor whose head is standing and \
         whose rung is moving, and a lane the crew never proved they could work"
    );
    assert_eq!(
        skyway_last_said(&app, SKYWAY_CONVOY),
        "world.falling_skyway.comms.convoy_rides_it_out"
    );
    assert_eq!(
        skyway_last_said(&app, WINDOW_CONTROL),
        "world.falling_skyway.comms.debrief_workers_and_operator"
    );

    // AC4: one promise kept and one broken in the same run, which is what makes
    // the ledger a record rather than a score.
    assert_eq!(skyway_promise(&app, "skyway_safe_passage"), "kept");
    assert_eq!(
        skyway_promise(&app, "skyway_surface_records"),
        "broken",
        "the captain swore the file would reach Control and never found the file"
    );
    let sheet = skyway_sheet_texts(&mut app, WINDOW_CONTROL);
    assert!(sheet.contains(&"world.falling_skyway.evidence.word_broken".to_string()));

    assert_the_campaign_record_is_complete(&app);
    assert_eq!(skyway_flag(&app, "campaign.skyway.passage.committee"), 1);
    assert_eq!(skyway_flag(&app, "campaign.skyway.passage.havelock"), 1);
    assert_eq!(skyway_flag(&app, "campaign.skyway.passage.left_convoy"), 1);
    assert_eq!(
        skyway_flag(&app, "campaign.skyway.passage.refused_convoy"),
        1
    );
    assert_eq!(skyway_flag(&app, "campaign.skyway.strike.negotiated"), 1);
    assert_eq!(skyway_flag(&app, "campaign.skyway.evidence.none"), 1);
    assert_eq!(skyway_flag(&app, "campaign.skyway.casualties.total"), 2);
    assert_eq!(skyway_flag(&app, "campaign.skyway.skyhook.held"), 1);
    assert_eq!(skyway_flag(&app, "campaign.skyway.commitments.kept"), 1);
    assert_eq!(skyway_flag(&app, "campaign.skyway.commitments.broken"), 1);
    assert_eq!(skyway_flag(&app, "campaign.skyway.commitments.clean"), 0);
    assert_eq!(
        objective_status(&app, "obj-a3-choice"),
        ObjectiveStatus::Completed
    );
}

/// **Issue #1043, AC5 — the early collapse, and the scene that is left.**
///
/// Nobody caught the tether, so #1040's floor took the head twenty-two seconds
/// before the window opened and #1042 priced the window at nothing. There is no
/// parley, because there is nothing to parley about: what is left on this
/// corridor is ONE mooring on the rung that still works, and Skyway Control asks
/// the captain for a name to put against it.
///
/// It is a different scene rather than a quieter one — a different channel, a
/// different resource, one winner instead of two, and its own debrief — and it
/// is keyed on the NUMBER the window published rather than on the collapse, so
/// a crew who kept the head standing and let it fall out of lift certification
/// walk into the same room.
///
/// The two who do not get it ride the storm out on a corridor with no lee in it
/// and a rung nobody is working, and their casualty numbers are the highest this
/// mission can produce.
#[test]
fn the_early_collapse_leaves_one_berth_and_control_asks_for_a_name() {
    use project_phoenix::core::messages::ObjectiveStatus;

    let (mut app, ship) = skyway_at_act_two();

    // Nothing is done about anything. The ship is parked where it can hear
    // Control, which is the only thing this crew get right.
    let opens_at = skyway_deadline_secs(&app, "skyway_transfer_window") as f64;
    parley_run_to(&mut app, ship, opens_at + 6.0, WINDOW_STATION);

    assert_eq!(
        skyway_flag(&app, "skyway_skyhook_lost"),
        1,
        "precondition: the head came down before the window opened"
    );
    assert!(!named_entity_present(&mut app, WINDOW_HEAD));
    assert_eq!(skyway_flag(&app, "skyway_window_supply"), 0);
    assert_eq!(
        skyway_flag(&app, "skyway_shelter_only"),
        1,
        "AC5: the window published nothing, so the smaller scene is the one that opened"
    );
    assert_eq!(
        skyway_flag(&app, "skyway_shelter_supply"),
        1,
        "…and what is left is Ladder A's one mooring, above its own line with its own \
         people still at work on it"
    );
    assert_eq!(
        objective_status_opt(&app, "obj-a3-parley"),
        None,
        "there is no ladder parley in this variant: it is not a re-skin of the other one"
    );
    assert_eq!(
        objective_status(&app, "obj-a3-berth"),
        ObjectiveStatus::Active
    );

    // AC2: all three still ask, from their own hulls, and are not answered —
    // the allocation is a name given to the authority that runs the corridor.
    assert_eq!(
        skyway_last_said(&app, SKYWAY_COMMITTEE),
        "world.falling_skyway.comms.committee_pleads"
    );
    assert_eq!(
        skyway_last_said(&app, SKYWAY_CUTTER),
        "world.falling_skyway.comms.havelock_pleads"
    );
    assert_eq!(
        skyway_last_said(&app, SKYWAY_CONVOY),
        "world.falling_skyway.comms.convoy_pleads"
    );

    let offer = skyway_open_node(&app, WINDOW_CONTROL);
    assert_eq!(offer.body, "world.falling_skyway.comms.control_berth_offer");
    assert_eq!(
        skyway_options(&offer),
        vec![
            "world.falling_skyway.comms.claim_stand_by".to_string(),
            "world.falling_skyway.comms.berth_to_committee".to_string(),
            "world.falling_skyway.comms.berth_to_havelock".to_string(),
            "world.falling_skyway.comms.berth_to_convoy".to_string(),
            "world.falling_skyway.comms.berth_to_nobody".to_string(),
        ],
        "one node, one berth, four names — and index 0 is still the hold"
    );

    skyway_pick(
        &mut app,
        WINDOW_CONTROL,
        "world.falling_skyway.comms.berth_to_convoy",
    );
    run(&mut app, 8);
    assert_eq!(skyway_flag(&app, "skyway_berth_convoy"), 1);
    assert_eq!(
        window_capacity_of(&mut app, WINDOW_DEPOT_A, "depot_a_shelter_berths"),
        0,
        "the berth leaves the rung's own manifest as it is spoken for — the same grammar \
         the window's four computed rows use"
    );

    run_to_the_endings(&mut app, ship, WINDOW_STATION);

    // AC3: the two who were left, and the worst numbers this mission produces.
    assert_eq!(
        skyway_flag(&app, "skyway_left_committee_cost"),
        4,
        "two for being left, one for a corridor with no head in it, one for a rung \
         nobody settled — the same claimant the forced run costs three"
    );
    assert_eq!(
        skyway_flag(&app, "skyway_left_havelock_cost"),
        3,
        "the same corridor, less one for a picket nobody shot at"
    );
    assert_eq!(
        skyway_last_said(&app, SKYWAY_COMMITTEE),
        "world.falling_skyway.comms.committee_rides_it_out_hurt",
        "nothing was promised on this run, so what comes back is the severity and not \
         the terms of a broken word"
    );
    assert_eq!(
        skyway_last_said(&app, WINDOW_CONTROL),
        "world.falling_skyway.comms.debrief_berth",
        "…and the debrief is the collapse variant's own, not the lift scene's"
    );
    assert_eq!(
        objective_status(&app, "obj-a3-berth"),
        ObjectiveStatus::Completed,
        "one name given and two parties told: all three were answered"
    );

    assert_the_campaign_record_is_complete(&app);
    assert_eq!(skyway_flag(&app, "campaign.skyway.passage.convoy"), 1);
    assert_eq!(
        skyway_flag(&app, "campaign.skyway.passage.left_committee"),
        1
    );
    assert_eq!(
        skyway_flag(&app, "campaign.skyway.passage.left_havelock"),
        1
    );
    assert_eq!(skyway_flag(&app, "campaign.skyway.passage.taken"), 1);
    assert_eq!(skyway_flag(&app, "campaign.skyway.passage.berth_only"), 1);
    assert_eq!(
        skyway_flag(&app, "campaign.skyway.strike.unresolved"),
        1,
        "nobody settled it and nobody forced it, and the next mission through here \
         inherits a stopped rung"
    );
    assert_eq!(skyway_flag(&app, "campaign.skyway.evidence.none"), 1);
    assert_eq!(skyway_flag(&app, "campaign.skyway.skyhook.lost"), 1);
    assert_eq!(skyway_flag(&app, "campaign.skyway.skyhook.held"), 0);
    assert_eq!(skyway_flag(&app, "campaign.skyway.casualties.storm"), 7);
    assert_eq!(
        skyway_flag(&app, "campaign.skyway.commitments.none"),
        1,
        "this captain gave nobody their word, which is a different record from keeping it"
    );
}

// ── What the next mission is handed (issue #867) ─────────────────────────────

/// **Issue #867 — the handoff fixture, on a real finished mission.**
///
/// `src/campaign/projection.rs` proves the projection's rules against payloads
/// built by hand; that is where inclusion, exclusion, identity and defaults are
/// settled, because they are claims about a pure function. What a hand-built
/// payload cannot prove is that the fold matches the shape a mission ACTUALLY
/// leaves behind — that the flag names are the ones a scenario wrote, that the
/// ledger is settled by the time the save is taken, that the structures are the
/// ones the crew wrecked. So this drives a whole mission to its ending, takes
/// the save the host would have taken, and reads the next mission's opening out
/// of it.
///
/// The road is the early collapse — the cheapest of the four endings to reach
/// and the harshest to inherit: the head comes down before the transfer window
/// opens, so the corridor carries nobody, Control offers one berth on the rung
/// instead, and the crew give it to the convoy. Two claimants are left on the
/// rock, the skyhook is gone, and no promise was ever made.
///
/// The demonstration at the end is deliberately NOT a second mission. It is the
/// two things a second mission would do with these facts: seed its flag store
/// (`campaign::seed_flags`) and read the counters its script would read, and
/// take the structures as the configuration a next world's `[[entity]]`
/// overrides would carry. Building an actual follow-on world would prove the
/// world file, not the projection.
#[test]
fn the_next_mission_opens_on_what_this_one_left_behind() {
    use project_phoenix::campaign::{project, seed_flags, CAMPAIGN_FACTS_VERSION};
    use project_phoenix::content_ledger;
    use project_phoenix::snapshot::{capture, run_for, versions};

    let (mut app, ship) = skyway_at_act_two();

    // Nothing is done about anything: the head comes down before the window
    // opens, and the one berth left on the rung goes to the convoy.
    let opens_at = skyway_deadline_secs(&app, "skyway_transfer_window") as f64;
    parley_run_to(&mut app, ship, opens_at + 6.0, WINDOW_STATION);
    assert_eq!(
        skyway_flag(&app, "skyway_skyhook_lost"),
        1,
        "precondition: this is the road where the structure is lost"
    );
    skyway_pick(
        &mut app,
        WINDOW_CONTROL,
        "world.falling_skyway.comms.berth_to_convoy",
    );
    run(&mut app, 8);
    run_to_the_endings(&mut app, ship, WINDOW_STATION);
    assert_the_campaign_record_is_complete(&app);

    // The save a host would have taken at the debrief, through the ordinary
    // capture — not a hand-built payload.
    let payload = capture(app.world());
    let run = run_for(
        payload,
        project_phoenix::sim_digest::world_digest(app.world()),
        42,
        SKYWAY_WORLD,
        versions(&content_ledger::frozen_or_live()),
    );

    let facts = project(&run);

    // ── What travelled ───────────────────────────────────────────────────────

    assert_eq!(facts.version, CAMPAIGN_FACTS_VERSION);
    assert_eq!(facts.mission, SKYWAY_WORLD);

    // THE CLAIM, stated against the live world rather than against a road this
    // test thinks it took: every `campaign.` counter the mission actually wrote
    // is in the facts, at the value the mission wrote it. Written this way on
    // purpose — an expectation list asserts what the author believed the ending
    // was, and the first thing to go wrong with a projection is that it drops a
    // family nobody thought to name.
    let live: std::collections::BTreeMap<String, i64> = app
        .world()
        .resource::<project_phoenix::world::server::WorldContentRuntime>()
        .flags
        .iter()
        .filter(|(name, _)| name.starts_with("campaign."))
        .map(|(name, value)| (name.to_string(), value))
        .collect();
    assert!(
        !live.is_empty(),
        "precondition: the mission reached its close and wrote its record"
    );
    for (name, value) in &live {
        assert_eq!(
            facts.tally(name),
            *value,
            "`{name}` was written by the mission and must survive the save"
        );
    }
    assert_eq!(
        facts.tallies.len(),
        live.len(),
        "and nothing else came with them"
    );

    // All six families are represented, by prefix — the shape #1043 guarantees,
    // checked here so a projection that silently dropped one is caught even
    // though this test does not presume which member each family answered with.
    for family in [
        "campaign.skyway.passage.",
        "campaign.skyway.strike.",
        "campaign.skyway.evidence.",
        "campaign.skyway.casualties.",
        "campaign.skyway.skyhook.",
        "campaign.skyway.commitments.",
    ] {
        assert!(
            facts
                .tallies
                .iter()
                .any(|(name, _)| name.starts_with(family)),
            "the `{family}` family reached the next mission"
        );
    }

    // The road-specific facts this variant exists for, and the ones a follow-on
    // mission would actually branch on.
    assert_eq!(
        facts.tally("campaign.skyway.skyhook.lost"),
        1,
        "the structure came down, and that is what the next mission inherits"
    );
    assert_eq!(facts.tally("campaign.skyway.skyhook.held"), 0);
    assert_eq!(
        facts.tally("campaign.skyway.passage.left_committee"),
        1,
        "the workers were left on the rock"
    );

    // Every tally is a `campaign.` name and every one is sorted — the filter and
    // the order the payload's own flag store cannot supply.
    assert!(
        facts
            .tallies
            .iter()
            .all(|(name, _)| name.starts_with("campaign.")),
        "a mission-local counter is not a handoff fact: {:?}",
        facts.tallies
    );
    let sorted = {
        let mut names: Vec<&String> = facts.tallies.iter().map(|(name, _)| name).collect();
        names.sort();
        names
    };
    assert_eq!(
        facts
            .tallies
            .iter()
            .map(|(name, _)| name)
            .collect::<Vec<_>>(),
        sorted
    );

    // The structures, by their AUTHORED names — the identity a later mission can
    // actually match, and the one thing a uuid could never be.
    assert!(
        !facts.structures.is_empty(),
        "this mission has structures and wrecked one of them"
    );
    for structure in &facts.structures {
        assert!(
            structure.name.starts_with("world.falling_skyway."),
            "a structure travels under the name the scenario wrote, not a uuid: \
             {structure:?}"
        );
        assert!((0.0..=1.0).contains(&structure.condition));
    }

    // ── What did not travel ──────────────────────────────────────────────────

    // The mission ends in a fight it does not win cleanly, so there is transient
    // state to leave behind — and the facts have no field it could arrive in.
    // Asserted here on a REAL payload as well as on the unit tests' built one,
    // because the payload this fold is handed in production is this one.
    let serialised = ron::ser::to_string(&facts).expect("the facts serialise");
    for absent in [
        "physics",
        "hull",
        "red_alert",
        "weapons",
        "beams",
        "torpedo",
        "asteroid",
        "rng",
        "collision",
        "blackboard",
        "patrol",
        "pass_surface",
    ] {
        assert!(
            !serialised.contains(absent),
            "`{absent}` reached the campaign facts — the next mission is being \
             handed this mission's combat state"
        );
    }

    // ── What a later mission does with them ──────────────────────────────────

    // (1) The flag store a follow-on world would open with. These are the reads
    // its script makes — `ctx.flags["campaign.skyway…"]` — and the names are the
    // ones THIS mission wrote, carried through unchanged.
    let seeded = seed_flags(&facts);
    assert_eq!(seeded.counter("campaign.skyway.skyhook.lost"), 1);
    assert_eq!(
        seeded.counter("campaign.skyway.passage.left_committee"),
        1,
        "a mission after this one opens knowing who was left behind, and can say          so without knowing which file left them"
    );
    for (name, value) in &live {
        assert_eq!(
            seeded.counter(name),
            *value,
            "`{name}` reads in the next mission exactly as it read in this one"
        );
    }

    // (2) The structures as CONFIGURATION: a follow-on world authoring the same
    // skyhook would carry the condition this mission left it in as an
    // `[[entity]] overrides` value rather than the template's own. Computed here
    // to show the shape; a world file consuming it is that world file's test.
    let overrides: Vec<(String, f32)> = facts
        .structures
        .iter()
        .map(|structure| (structure.name.clone(), structure.condition * 100.0))
        .collect();
    assert!(
        overrides
            .iter()
            .all(|(_, condition)| (0.0..=100.0).contains(condition)),
        "condition points a next world could author directly: {overrides:?}"
    );

    // And the whole thing survives being written down between missions, which is
    // what a campaign runner does with it.
    let text = ron::ser::to_string(&facts).expect("serialises");
    assert_eq!(
        ron::from_str::<project_phoenix::campaign::CampaignFacts>(&text).expect("parses back"),
        facts
    );
}

// ── #1156: the tractor beam, end to end ──────────────────────────────────────
//
// `assets/worlds/probe_tractor.toml` fields a backfilled player TUG (the
// dedicated `tractor_tug` hull — the shipped destroyer is deliberately not
// touched) and a DERELICT 80 units off. The test drives the beam through the
// ordinary admitted `EngageTractor`/`ReleaseTractor` path, sets the ship's one
// Tactical lock the way `SetTarget` would, and moves the operator by hand —
// proving a derelict is held on the rig and released by each interruption.

const DERELICT: &str = "world.probe_tractor.entity.derelict.name";
/// An `ai:` token: admission authorises it iff the target system is
/// AI-controlled, which the tug's engineering-owned tractor is while nobody is
/// at its console — the same seam #1162's tractor AI will use, and the same one
/// a human tenure token uses from the other side (AGENTS.md rule 6).
const TRACTOR_TOKEN: &str = "ai:tractor-probe";

fn tractor_args(dt: f64) -> HeadlessArgs {
    HeadlessArgs {
        world_path: "assets/worlds/probe_tractor.toml".into(),
        // The player ship IS the tug: the game-start spawn swaps the world's
        // `player-ship` placeholder for the lobby-SELECTED hull, so the selection
        // has to be the tug or its tractor never spawns.
        ship_path: "assets/entities/tractor_tug.toml".into(),
        dt,
        max_ticks: ticks_for_sim_seconds(60.0, dt),
        deterministic: true,
        seed: Some(42),
        ..test_args()
    }
}

fn tractor_operator(app: &mut App) -> Entity {
    app.world_mut()
        .query_filtered::<Entity, With<LocalShip>>()
        .iter(app.world())
        .next()
        .expect("the probe world spawns a local tug")
}

fn tractor_beam_of(app: &mut App) -> project_phoenix::tractor::TractorBeam {
    let op = tractor_operator(app);
    app.world()
        .get::<project_phoenix::tractor::TractorBeam>(op)
        .expect("the tug carries a TractorBeam")
        .clone()
}

fn operator_pos(app: &mut App) -> Vec3 {
    let op = tractor_operator(app);
    app.world()
        .get::<Transform>(op)
        .expect("the tug has a transform")
        .translation
}

/// Place the operator by writing its `ShipPhysics`, which `sync_ship_position`
/// projects into the transform the coupling reads.
fn place_operator(app: &mut App, position: Vec3) {
    let op = tractor_operator(app);
    let mut physics = app
        .world_mut()
        .get_mut::<ShipPhysics>(op)
        .expect("the tug is a ship");
    physics.x = position.x;
    physics.y = position.y;
    physics.z = position.z;
}

/// Set (or clear) the ship's one Tactical lock, the way a `SetTarget` would.
fn set_tractor_lock(app: &mut App, uuid: Option<String>) {
    let op = tractor_operator(app);
    app.world_mut()
        .entity_mut(op)
        .insert(project_phoenix::console::weapons::beam::TacticalRadarSelection(uuid));
}

/// Send an engage/release through the real admission path and give it the ticks
/// to arrive (drained in `PreUpdate`, admitted before `SimSet::Input`, consumed
/// and evaluated in `SimSet::Modifiers` of the same tick).
fn send_tractor(app: &mut App, payload: project_phoenix::messages::SystemControlPayload) {
    use project_phoenix::lobby::InboundMessage;
    use project_phoenix::messages::{ClientMessage, SystemId};
    app.world_mut()
        .resource_mut::<bevy::ecs::message::Messages<InboundMessage>>()
        .write(InboundMessage {
            token: TRACTOR_TOKEN.into(),
            msg: ClientMessage::ControlSystem {
                target: SystemId(project_phoenix::ship::system_registry::TRACTOR_SYSTEM_ID.into()),
                payload,
            },
        });
    run(app, 2);
}

/// Reset the operator to the origin and the derelict 80 units off, then engage
/// against it — the clean holding state each interruption starts from.
fn reengage_holding(app: &mut App, derelict_uuid: &str) {
    use project_phoenix::messages::SystemControlPayload;
    send_tractor(app, SystemControlPayload::ReleaseTractor);
    place_operator(app, Vec3::ZERO);
    move_named_to(app, DERELICT, Vec3::new(80.0, 0.0, 0.0));
    run(app, 1);
    set_tractor_lock(app, Some(derelict_uuid.to_string()));
    send_tractor(app, SystemControlPayload::EngageTractor);
    let beam = tractor_beam_of(app);
    assert!(
        beam.coupled_target.as_deref() == Some(derelict_uuid) && beam.engaged,
        "precondition: the beam should be holding the derelict, got {beam:?}"
    );
}

/// Drop the tractor power group below its authored `min_power_level` (2).
fn cut_tractor_power(app: &mut App) {
    let op = tractor_operator(app);
    let mut ps = app
        .world_mut()
        .get_mut::<project_phoenix::ship::power::ShipPowerSystem>(op)
        .expect("the tug has a power system");
    let _ = ps.0.set_group_allocation(
        &project_phoenix::messages::PowerGroupId("tractor".into()),
        1,
    );
}

/// Restore the tractor power group to its nominal level.
fn restore_tractor_power(app: &mut App) {
    let op = tractor_operator(app);
    let mut ps = app
        .world_mut()
        .get_mut::<project_phoenix::ship::power::ShipPowerSystem>(op)
        .expect("the tug has a power system");
    let _ = ps.0.set_group_allocation(
        &project_phoenix::messages::PowerGroupId("tractor".into()),
        2,
    );
}

/// Knock the tractor system's HP below its authored disabled threshold.
fn disable_tractor(app: &mut App) {
    let op = tractor_operator(app);
    let mut hull = app
        .world_mut()
        .get_mut::<project_phoenix::entity_spawner::EntitySystemHull>(op)
        .expect("the tug has a hull");
    hull.0
        .set_hp(&project_phoenix::messages::SystemId("tractor".into()), 1.0);
}

/// Restore the tractor system to full HP (a Disabled system stops accepting AI
/// control, so it must be operable again before it can be re-engaged).
fn repair_tractor(app: &mut App) {
    let op = tractor_operator(app);
    let mut hull = app
        .world_mut()
        .get_mut::<project_phoenix::entity_spawner::EntitySystemHull>(op)
        .expect("the tug has a hull");
    hull.0
        .set_hp(&project_phoenix::messages::SystemId("tractor".into()), 30.0);
}

/// AC: a derelict is moved by an engaged tractor and released by EACH
/// interruption (lock lost / release / out of range / power lost / disabled).
#[test]
fn a_tractor_holds_a_derelict_on_the_rig_and_every_interruption_drops_it() {
    use project_phoenix::messages::SystemControlPayload;
    use project_phoenix::tractor::TractorRefusal;

    let dt = 1.0 / 60.0;
    let mut app = build_headless_app(&tractor_args(dt)).expect("app should build");
    // Far enough in that the game is InProgress and the tug is backfilled, so its
    // engineering-owned tractor is AI-controlled and the `ai:` token is admitted.
    run(&mut app, 60);
    assert_eq!(
        app.world().resource::<State<GamePhase>>().get(),
        &GamePhase::InProgress,
        "precondition: admission only runs InProgress"
    );
    let derelict = scan_uuid_named(&mut app, DERELICT);

    // ── The hold forms and the derelict rides the rig ────────────────────────
    place_operator(&mut app, Vec3::ZERO);
    move_named_to(&mut app, DERELICT, Vec3::new(80.0, 0.0, 0.0));
    run(&mut app, 1);
    set_tractor_lock(&mut app, Some(derelict.clone()));
    send_tractor(&mut app, SystemControlPayload::EngageTractor);

    let beam = tractor_beam_of(&mut app);
    assert!(beam.engaged, "the beam engaged");
    assert_eq!(beam.coupled_target.as_deref(), Some(derelict.as_str()));
    assert!(
        beam.last_refusal.is_none(),
        "a clean hold carries no refusal"
    );

    // The rig, not the derelict's own last position: it spawned 80 units to
    // starboard and is now 120 astern of the tug (the authored offset).
    let offset = position_of(&mut app, DERELICT) - operator_pos(&mut app);
    assert!(
        (offset.length() - 120.0).abs() < 1.0,
        "the derelict rides the authored 120-unit coupling offset, got a separation of {}",
        offset.length()
    );

    // The tug flies a course; the derelict rides the rig the whole way. Steps
    // stay inside the authored 600-unit range (the derelict is only ~120 behind
    // each time), which is what a tug under power does — a single teleport past
    // the range would correctly part the towline, which the out-of-range case
    // below proves on purpose.
    for step in [
        Vec3::new(200.0, 0.0, 0.0),
        Vec3::new(400.0, 0.0, -150.0),
        Vec3::new(650.0, 0.0, -350.0),
    ] {
        place_operator(&mut app, step);
        run(&mut app, 2);
        let carried = position_of(&mut app, DERELICT) - operator_pos(&mut app);
        assert!(
            (carried.length() - 120.0).abs() < 1.0,
            "the derelict goes on riding the rig as the tug flies, which is the whole of what \
             the coupling is. Got a separation of {}",
            carried.length()
        );
    }
    assert!(
        position_of(&mut app, DERELICT).distance(Vec3::new(80.0, 0.0, 0.0)) > 400.0,
        "the derelict really travelled — a test comparing only the two positions would pass \
         with both sitting at the origin"
    );

    // ── Interruption 1: the lock is dropped ──────────────────────────────────
    reengage_holding(&mut app, &derelict);
    let parted = position_of(&mut app, DERELICT);
    set_tractor_lock(&mut app, None);
    run(&mut app, 2);
    let beam = tractor_beam_of(&mut app);
    assert!(
        !beam.engaged && beam.coupled_target.is_none(),
        "dropping the lock ended the hold"
    );
    assert_eq!(beam.last_refusal, Some(TractorRefusal::NoLock));
    place_operator(&mut app, Vec3::new(2000.0, 0.0, 0.0));
    run(&mut app, 3);
    assert!(
        position_of(&mut app, DERELICT).distance(parted) < 50.0,
        "a released derelict stays where the towline parted rather than following the tug"
    );

    // ── Interruption 2: the operator releases ────────────────────────────────
    reengage_holding(&mut app, &derelict);
    let parted = position_of(&mut app, DERELICT);
    send_tractor(&mut app, SystemControlPayload::ReleaseTractor);
    let beam = tractor_beam_of(&mut app);
    assert!(
        !beam.engaged && beam.coupled_target.is_none(),
        "release ended the hold"
    );
    assert!(
        beam.last_refusal.is_none(),
        "a deliberate release is not a refusal"
    );
    place_operator(&mut app, Vec3::new(3000.0, 0.0, 0.0));
    run(&mut app, 3);
    assert!(
        position_of(&mut app, DERELICT).distance(parted) < 50.0,
        "a released derelict stays put"
    );

    // ── Interruption 3: the tug flies out of the authored range ──────────────
    reengage_holding(&mut app, &derelict);
    let parted = position_of(&mut app, DERELICT);
    place_operator(&mut app, Vec3::new(90_000.0, 0.0, 90_000.0));
    run(&mut app, 3);
    let beam = tractor_beam_of(&mut app);
    assert!(
        !beam.engaged && beam.coupled_target.is_none(),
        "leaving range ended the hold"
    );
    assert_eq!(beam.last_refusal, Some(TractorRefusal::OutOfRange));
    assert!(
        position_of(&mut app, DERELICT).distance(parted) < 50.0,
        "a derelict left out of range stays where the towline parted rather than being yanked \
         ninety kilometres across the map"
    );

    // ── Interruption 4: the power allocation is lost ─────────────────────────
    reengage_holding(&mut app, &derelict);
    let parted = position_of(&mut app, DERELICT);
    cut_tractor_power(&mut app);
    run(&mut app, 2);
    let beam = tractor_beam_of(&mut app);
    assert!(
        !beam.engaged && beam.coupled_target.is_none(),
        "losing power ended the hold"
    );
    assert_eq!(beam.last_refusal, Some(TractorRefusal::Unpowered));
    place_operator(&mut app, Vec3::new(4000.0, 0.0, 0.0));
    run(&mut app, 3);
    assert!(
        position_of(&mut app, DERELICT).distance(parted) < 50.0,
        "an unpowered beam has let go"
    );

    // ── Interruption 5: the tractor is knocked out to Disabled ───────────────
    // Restore power first so the disable is the ONLY failing condition.
    restore_tractor_power(&mut app);
    reengage_holding(&mut app, &derelict);
    let parted = position_of(&mut app, DERELICT);
    disable_tractor(&mut app);
    run(&mut app, 2);
    let beam = tractor_beam_of(&mut app);
    assert!(
        !beam.engaged && beam.coupled_target.is_none(),
        "a disabled tractor ended the hold"
    );
    assert_eq!(beam.last_refusal, Some(TractorRefusal::Disabled));
    place_operator(&mut app, Vec3::new(5000.0, 0.0, 0.0));
    run(&mut app, 3);
    assert!(
        position_of(&mut app, DERELICT).distance(parted) < 50.0,
        "a knocked-out tractor has let go"
    );

    // ── The hold survives a snapshot resume ──────────────────────────────────
    // Repair the tractor first — interruption 5 left it Disabled, which stops it
    // accepting control at all — then capture with a live grip, release it, and
    // restore: the engage state and the coupled target come back, which is the
    // half the digest folds and a resume would otherwise drop.
    repair_tractor(&mut app);
    run(&mut app, 1);
    reengage_holding(&mut app, &derelict);
    let digest_holding = project_phoenix::sim_digest::state_digest(&app);
    let snap = project_phoenix::snapshot::capture(app.world());
    send_tractor(&mut app, SystemControlPayload::ReleaseTractor);
    assert!(
        !tractor_beam_of(&mut app).engaged,
        "precondition: the beam is released before the restore"
    );
    project_phoenix::snapshot::restore(app.world_mut(), &snap);
    let beam = tractor_beam_of(&mut app);
    assert!(
        beam.engaged && beam.coupled_target.as_deref() == Some(derelict.as_str()),
        "the engage state and the coupled target survived the snapshot resume, got {beam:?}"
    );
    assert_eq!(
        project_phoenix::sim_digest::state_digest(&app),
        digest_holding,
        "…and the restored world folds to the same digest the captured one did"
    );
}

// ── #1158: the target decides what being held means (arrest-decline) ──────────
//
// `assets/worlds/probe_held_response.toml` fields the same backfilled tug and a
// FAILING STRUCTURE that authors — in its own config — an `arrest-decline`
// held-response: it loses 6 condition points a second on its own, and while the
// beam is on it the decline is arrested and it recovers at an authored 20 points
// a second. The test drives the beam through the ordinary admitted path and
// reads the structure's OWN condition track and operational flag — observable
// target state, not a private field — to prove:
//
//   * unheld, the decline crosses the structure's own failure threshold DOWN;
//   * held, the decline is arrested and the recovery crosses the restore
//     threshold UP, setting the operational flag a scenario reads;
//   * released, the ordinary decline resumes on the next tick.

const STRUCTURE: &str = "world.probe_held_response.entity.structure.name";
/// The structure's own operational flag (`depot_transfer`'s authored threshold,
/// down below 40 % condition, back at 45 %).
const STRUCTURE_FLAG: &str = "depot_transfer_capable";

fn held_response_args(dt: f64) -> HeadlessArgs {
    HeadlessArgs {
        world_path: "assets/worlds/probe_held_response.toml".into(),
        ship_path: "assets/entities/tractor_tug.toml".into(),
        dt,
        max_ticks: ticks_for_sim_seconds(60.0, dt),
        deterministic: true,
        seed: Some(42),
        ..test_args()
    }
}

/// The named structure's operational flag as its own condition track holds it —
/// the observable state a scenario's `on_flag_set` reacts to.
fn structure_flag(app: &mut App, name: &str, flag: &str) -> Option<bool> {
    app.world_mut()
        .query::<(
            &project_phoenix::entities::spawner::EntityName,
            &project_phoenix::infrastructure::InfrastructureCondition,
        )>()
        .iter(app.world())
        .find(|(entity_name, _)| entity_name.0 == name)
        .map(|(_, condition)| condition.0.flag(flag))
        .unwrap_or_else(|| panic!("{name} carries no condition track"))
}

/// AC: a held target decides what being held means — a failing structure's
/// decline is arrested and recovered while the beam holds it, and resumes on
/// release. Progress lives on the structure's OWN condition track.
#[test]
fn holding_a_failing_structure_arrests_its_decline_and_releasing_resumes_it() {
    use project_phoenix::messages::SystemControlPayload;

    let dt = 1.0 / 60.0;
    let mut app = build_headless_app(&held_response_args(dt)).expect("app should build");
    // Far enough in that the game is InProgress and the tug is backfilled, so its
    // engineering-owned tractor is AI-controlled and the `ai:` token is admitted.
    run(&mut app, 60);
    assert_eq!(
        app.world().resource::<State<GamePhase>>().get(),
        &GamePhase::InProgress,
        "precondition: admission only runs InProgress"
    );
    let structure = scan_uuid_named(&mut app, STRUCTURE);
    place_operator(&mut app, Vec3::ZERO);

    // The structure opens above its 40 % failure point, so its flag begins UP.
    assert_eq!(
        structure_flag(&mut app, STRUCTURE, STRUCTURE_FLAG),
        Some(true),
        "precondition: an intact-enough structure starts capable"
    );

    // ── It declines on its own, crossing its own failure threshold DOWN ───────
    let before_decline = condition_of(&mut app, STRUCTURE);
    run(&mut app, 180); // 3 sim-seconds unheld
    let declined = condition_of(&mut app, STRUCTURE);
    assert!(
        declined < before_decline - 12.0,
        "left alone the structure declines at its authored rate: from {before_decline} to \
         {declined}"
    );
    assert_eq!(
        structure_flag(&mut app, STRUCTURE, STRUCTURE_FLAG),
        Some(false),
        "…and the decline crossed the structure's own 40 % failure point, dropping the \
         operational flag a scenario reads"
    );

    // ── Held, the decline is arrested and it recovers at the authored rate ────
    set_tractor_lock(&mut app, Some(structure.clone()));
    send_tractor(&mut app, SystemControlPayload::EngageTractor);
    let beam = tractor_beam_of(&mut app);
    assert!(
        beam.engaged && beam.coupled_target.as_deref() == Some(structure.as_str()),
        "precondition: the beam is holding the structure, got {beam:?}"
    );
    let held_start = condition_of(&mut app, STRUCTURE);
    run(&mut app, 180); // 3 sim-seconds held
    let recovered = condition_of(&mut app, STRUCTURE);
    let gain = recovered - held_start;
    // Net +20/s for three seconds is +60. Were the decline NOT arrested, the
    // +20/s recovery would fight the −6/s decline and net only +42 — so a gain
    // past 50 proves the decline is truly arrested, not merely outrun.
    assert!(
        gain > 50.0 && gain < 70.0,
        "held, the −6/s decline is arrested and the structure recovers at the authored +20/s \
         (≈+60 over three seconds), not the +42 an un-arrested recovery would give: gained \
         {gain} (from {held_start} to {recovered})"
    );
    assert_eq!(
        structure_flag(&mut app, STRUCTURE, STRUCTURE_FLAG),
        Some(true),
        "…and the recovered condition crossed the structure's own restore point UP, setting the \
         operational flag again — the crossing mirrored by the one system that owns the edges"
    );

    // ── Released, the ordinary decline resumes on the next tick ───────────────
    let before_release = condition_of(&mut app, STRUCTURE);
    send_tractor(&mut app, SystemControlPayload::ReleaseTractor);
    assert!(!tractor_beam_of(&mut app).engaged, "the beam released");
    run(&mut app, 120); // 2 sim-seconds unheld again
    let after_release = condition_of(&mut app, STRUCTURE);
    assert!(
        after_release < before_release - 6.0,
        "releasing the beam resumes the structure's ordinary decline: from {before_release} to \
         {after_release}"
    );
}

// ── #1159: helm docking, end to end ──────────────────────────────────────────
//
// `assets/worlds/probe_dock.toml` fields a backfilled player DOCK PROBE (the
// dedicated `dock_probe` hull — the shipped destroyer is deliberately not
// touched) and a passive BERTH 100 units to starboard. The test drives the dock
// through the ordinary admitted `Dock`/`Undock` path and lets the dock manoeuvre
// fly the own ship onto its mate — proving two hulls reach a mated dock, separate
// on undock, and that every interruption ends the dock cleanly.

const BERTH_DOCK: &str = "world.probe_dock.entity.berth.name";
/// An `ai:` token: admission authorises it iff the target system is
/// AI-controlled, which the probe's helm-owned dock is while nobody is at its
/// console — the same seam #1162's dock AI will use, and the same one a human
/// tenure token uses from the other side (AGENTS.md rule 6).
const DOCK_TOKEN: &str = "ai:dock-probe";

fn dock_args(dt: f64) -> HeadlessArgs {
    HeadlessArgs {
        world_path: "assets/worlds/probe_dock.toml".into(),
        // The player ship IS the probe: game-start swaps the world placeholder for
        // the lobby-SELECTED hull, so the selection has to be the probe or its
        // dock never spawns.
        ship_path: "assets/entities/dock_probe.toml".into(),
        dt,
        max_ticks: ticks_for_sim_seconds(120.0, dt),
        deterministic: true,
        seed: Some(1159),
        ..test_args()
    }
}

fn dock_operator(app: &mut App) -> Entity {
    app.world_mut()
        .query_filtered::<Entity, With<LocalShip>>()
        .iter(app.world())
        .next()
        .expect("the probe world spawns a local dock probe")
}

fn dock_control_of(app: &mut App) -> project_phoenix::dock::DockControl {
    let op = dock_operator(app);
    app.world()
        .get::<project_phoenix::dock::DockControl>(op)
        .expect("the probe carries a DockControl")
        .clone()
}

fn dock_operator_pos(app: &mut App) -> Vec3 {
    let op = dock_operator(app);
    app.world()
        .get::<Transform>(op)
        .expect("the probe has a transform")
        .translation
}

/// Place the operator by writing its `ShipPhysics`, which `sync_ship_position`
/// projects into the transform the manoeuvre reads.
fn place_dock_operator(app: &mut App, position: Vec3) {
    let op = dock_operator(app);
    let mut physics = app
        .world_mut()
        .get_mut::<ShipPhysics>(op)
        .expect("the probe is a ship");
    physics.x = position.x;
    physics.y = position.y;
    physics.z = position.z;
}

/// The berth entity, looked up by name.
fn berth_entity(app: &mut App) -> Entity {
    use project_phoenix::entities::spawner::EntityName;
    app.world_mut()
        .query::<(Entity, &EntityName)>()
        .iter(app.world())
        .find(|(_, n)| n.0 == BERTH_DOCK)
        .map(|(e, _)| e)
        .expect("the probe world spawns a berth")
}

/// Move the passive berth by writing its `Transform` directly — it is a
/// structure, not a ship, so nothing re-syncs it from a `ShipPhysics`.
fn move_berth(app: &mut App, position: Vec3) {
    let berth = berth_entity(app);
    app.world_mut()
        .get_mut::<Transform>(berth)
        .expect("the berth has a transform")
        .translation = position;
}

/// Send a dock/undock through the real admission path and give it the ticks to
/// arrive (drained in `PreUpdate`, admitted before `SimSet::Input`, consumed in
/// `SimSet::Modifiers` of the same tick).
fn send_dock(app: &mut App, payload: project_phoenix::messages::SystemControlPayload) {
    use project_phoenix::lobby::InboundMessage;
    use project_phoenix::messages::{ClientMessage, SystemId};
    app.world_mut()
        .resource_mut::<bevy::ecs::message::Messages<InboundMessage>>()
        .write(InboundMessage {
            token: DOCK_TOKEN.into(),
            msg: ClientMessage::ControlSystem {
                target: SystemId(project_phoenix::ship::system_registry::DOCK_SYSTEM_ID.into()),
                payload,
            },
        });
    run(app, 2);
}

/// Drop the dock power group below its authored `min_power_level` (2).
fn cut_dock_power(app: &mut App) {
    let op = dock_operator(app);
    let mut ps = app
        .world_mut()
        .get_mut::<project_phoenix::ship::power::ShipPowerSystem>(op)
        .expect("the probe has a power system");
    let _ =
        ps.0.set_group_allocation(&project_phoenix::messages::PowerGroupId("dock".into()), 1);
}

fn restore_dock_power(app: &mut App) {
    let op = dock_operator(app);
    let mut ps = app
        .world_mut()
        .get_mut::<project_phoenix::ship::power::ShipPowerSystem>(op)
        .expect("the probe has a power system");
    let _ =
        ps.0.set_group_allocation(&project_phoenix::messages::PowerGroupId("dock".into()), 2);
}

fn disable_dock(app: &mut App) {
    let op = dock_operator(app);
    let mut hull = app
        .world_mut()
        .get_mut::<project_phoenix::entity_spawner::EntitySystemHull>(op)
        .expect("the probe has a hull");
    hull.0
        .set_hp(&project_phoenix::messages::SystemId("dock".into()), 1.0);
}

fn repair_dock(app: &mut App) {
    let op = dock_operator(app);
    let mut hull = app
        .world_mut()
        .get_mut::<project_phoenix::entity_spawner::EntitySystemHull>(op)
        .expect("the probe has a hull");
    hull.0
        .set_hp(&project_phoenix::messages::SystemId("dock".into()), 30.0);
}

/// Reset the probe to the origin and the berth 100 units to starboard, engage,
/// and fly the manoeuvre to completion — the clean docked state each interruption
/// starts from.
fn redock(app: &mut App) {
    use project_phoenix::messages::SystemControlPayload;
    // Clear any prior state and settle the two hulls back into their start pose.
    send_dock(app, SystemControlPayload::Undock);
    place_dock_operator(app, Vec3::ZERO);
    move_berth(app, Vec3::new(100.0, 0.0, 0.0));
    run(app, 1);
    send_dock(app, SystemControlPayload::Dock);
    run(app, 140);
    let dock = dock_control_of(app);
    assert!(
        dock.docked && dock.docking_target.is_some(),
        "precondition: the two hulls should be mated, got {dock:?}"
    );
}

/// AC: a dock control appears only while a valid target is in range; running it
/// mates the two hulls; undock separates them; and every interruption ends the
/// dock cleanly. Also: the docked relationship folds and survives a resume.
#[test]
fn two_hulls_reach_a_mated_dock_separate_on_undock_and_every_interruption_ends_it() {
    use project_phoenix::dock::DockRefusal;
    use project_phoenix::messages::SystemControlPayload;

    let dt = 1.0 / 60.0;
    let mut app = build_headless_app(&dock_args(dt)).expect("app should build");
    // Far enough in that the game is InProgress and the probe is backfilled, so
    // its helm-owned dock is AI-controlled and the `ai:` token is admitted.
    run(&mut app, 60);
    assert_eq!(
        app.world().resource::<State<GamePhase>>().get(),
        &GamePhase::InProgress,
        "precondition: admission only runs InProgress"
    );
    let berth = scan_uuid_named(&mut app, BERTH_DOCK);

    // ── The contextual control appears only in range ─────────────────────────
    place_dock_operator(&mut app, Vec3::ZERO);
    move_berth(&mut app, Vec3::new(100.0, 0.0, 0.0));
    run(&mut app, 2);
    assert_eq!(
        dock_control_of(&mut app).available_target.as_deref(),
        Some(berth.as_str()),
        "a berth in range makes the dock control available"
    );

    // Drift the berth far out: the control disappears.
    move_berth(&mut app, Vec3::new(9000.0, 0.0, 0.0));
    run(&mut app, 2);
    assert!(
        dock_control_of(&mut app).available_target.is_none(),
        "with no berth in range the dock control is absent"
    );

    // Back in range: it returns.
    move_berth(&mut app, Vec3::new(100.0, 0.0, 0.0));
    run(&mut app, 2);
    assert_eq!(
        dock_control_of(&mut app).available_target.as_deref(),
        Some(berth.as_str()),
        "the control reappears when the berth is back in range"
    );

    // ── Dock: the two hulls fly to a mated dock ──────────────────────────────
    send_dock(&mut app, SystemControlPayload::Dock);
    run(&mut app, 140);
    let dock = dock_control_of(&mut app);
    assert!(
        dock.docked,
        "the manoeuvre mated the two hulls, got {dock:?}"
    );
    assert_eq!(dock.docking_target.as_deref(), Some(berth.as_str()));
    assert_eq!(
        dock.docked_partner(),
        Some(berth.as_str()),
        "the docked relationship names the berth — what the umbilical (#1160) reads"
    );
    // The probe flew itself onto the mate pose without a helm officer: its
    // starboard plate meets the berth's port plate at ~(95,0,0), so the probe
    // origin settles at ~(90,0,0).
    let mated = dock_operator_pos(&mut app);
    assert!(
        (mated - Vec3::new(90.0, 0.0, 0.0)).length() < 5.0,
        "the probe flew onto the mate pose, got {mated:?}"
    );

    // ── Undock: the ship backs clear and returns to ordinary flight ──────────
    send_dock(&mut app, SystemControlPayload::Undock);
    run(&mut app, 160);
    let dock = dock_control_of(&mut app);
    assert!(
        !dock.docked && !dock.engaged && dock.docking_target.is_none(),
        "undock ended the mate, got {dock:?}"
    );
    let cleared = dock_operator_pos(&mut app);
    assert!(
        cleared.distance(Vec3::new(100.0, 0.0, 0.0)) > 100.0,
        "undock backed the probe clear of the berth, got {cleared:?}"
    );

    // ── Interruption 1: the hulls drift apart ────────────────────────────────
    redock(&mut app);
    move_berth(&mut app, Vec3::new(9000.0, 0.0, 0.0));
    run(&mut app, 3);
    let dock = dock_control_of(&mut app);
    assert!(
        !dock.docked && !dock.engaged,
        "drifting apart ended the dock"
    );
    assert_eq!(dock.last_refusal, Some(DockRefusal::OutOfRange));

    // ── Interruption 2: the power allocation is lost ─────────────────────────
    redock(&mut app);
    cut_dock_power(&mut app);
    run(&mut app, 3);
    let dock = dock_control_of(&mut app);
    assert!(!dock.docked && !dock.engaged, "losing power ended the dock");
    assert_eq!(dock.last_refusal, Some(DockRefusal::Unpowered));

    // ── Interruption 3: the dock is knocked out to Disabled ──────────────────
    restore_dock_power(&mut app);
    redock(&mut app);
    disable_dock(&mut app);
    run(&mut app, 3);
    let dock = dock_control_of(&mut app);
    assert!(
        !dock.docked && !dock.engaged,
        "a disabled dock ended the mate"
    );
    assert_eq!(dock.last_refusal, Some(DockRefusal::Disabled));

    // ── Interruption 4: the berth is destroyed ───────────────────────────────
    repair_dock(&mut app);
    redock(&mut app);
    let berth_ent = berth_entity(&mut app);
    app.world_mut().entity_mut(berth_ent).despawn();
    run(&mut app, 3);
    let dock = dock_control_of(&mut app);
    assert!(
        !dock.docked && !dock.engaged,
        "a destroyed berth ended the mate"
    );
    assert_eq!(dock.last_refusal, Some(DockRefusal::TargetLost));

    // ── The docked relationship survives a snapshot resume ───────────────────
    // Interruption 4 despawned the berth, so prove the resume on a fresh app
    // whose world still has it.
    let mut app = build_headless_app(&dock_args(dt)).expect("app should build");
    run(&mut app, 60);
    let berth = scan_uuid_named(&mut app, BERTH_DOCK);
    redock(&mut app);
    let digest_docked = project_phoenix::sim_digest::state_digest(&app);
    let snap = project_phoenix::snapshot::capture(app.world());
    send_dock(&mut app, SystemControlPayload::Undock);
    run(&mut app, 5);
    assert!(
        !dock_control_of(&mut app).docked,
        "precondition: the dock is released before the restore"
    );
    project_phoenix::snapshot::restore(app.world_mut(), &snap);
    let dock = dock_control_of(&mut app);
    assert!(
        dock.docked && dock.docking_target.as_deref() == Some(berth.as_str()),
        "the docked relationship survived the snapshot resume, got {dock:?}"
    );
    assert_eq!(
        project_phoenix::sim_digest::state_digest(&app),
        digest_docked,
        "…and the restored world folds to the same digest the captured one did"
    );
}

// ── #1161: external repair-team dispatch, end to end ─────────────────────────
//
// `assets/worlds/probe_external_repair.toml` fields a backfilled player TENDER
// (the dedicated `repair_tender` hull — no shipped hull is touched) carrying a
// single repair team and, in its own `[repair.external_dispatch]` table, the
// reach and rate a dispatched team works at, and an ALLY DEPOT 80 units off that
// declines at zero on its own. The test drives the dispatch through the ordinary
// admitted `DispatchExternalRepair`/`RecallExternalRepair` path, sets the ship's
// one Tactical lock the way `SetTarget` would, and moves the operator by hand.

const ALLY: &str = "world.probe_external_repair.entity.ally.name";
/// An `ai:` token: admission authorises it iff the target system is
/// AI-controlled, which the tender's engineering-owned `repair` system is while
/// nobody is at its console — the same seam #1162's repair AI will use, and the
/// same one a human tenure token uses from the other side (AGENTS.md rule 6).
const REPAIR_DISPATCH_TOKEN: &str = "ai:external-repair-probe";

fn external_repair_args(dt: f64) -> HeadlessArgs {
    HeadlessArgs {
        world_path: "assets/worlds/probe_external_repair.toml".into(),
        // The player ship IS the tender: the game-start spawn swaps the world's
        // `player-ship` placeholder for the lobby-SELECTED hull, so the selection
        // has to be the tender or its repair team never spawns.
        ship_path: "assets/entities/repair_tender.toml".into(),
        dt,
        max_ticks: ticks_for_sim_seconds(60.0, dt),
        deterministic: true,
        seed: Some(42),
        ..test_args()
    }
}

fn external_dispatch_of(app: &mut App) -> project_phoenix::console::repair::ExternalRepairDispatch {
    let op = tractor_operator(app);
    app.world()
        .get::<project_phoenix::console::repair::ExternalRepairDispatch>(op)
        .expect("the tender carries an ExternalRepairDispatch")
        .clone()
}

/// The one availability answer both the human console and the repair AI read:
/// how many of the tender's teams are free for its OWN damage-control sweep,
/// with any external commitment already withdrawn (issue #1161, rule 6).
fn operator_free_team_count(app: &mut App) -> usize {
    let op = tractor_operator(app);
    let committed = app
        .world()
        .get::<project_phoenix::console::repair::ExternalRepairDispatch>(op)
        .map(|d| d.committed_repair_teams())
        .unwrap_or(0);
    let teams = app
        .world()
        .get::<project_phoenix::console::repair::server::ShipRepairTeams>(op)
        .expect("the tender carries repair teams");
    teams.0.free_team_indices(committed).len()
}

/// Send a dispatch/recall through the real admission path and give it the ticks
/// to arrive (drained in `PreUpdate`, admitted before `SimSet::Input`, consumed
/// in `SimSet::Input` of the same tick).
fn send_repair(app: &mut App, payload: project_phoenix::messages::SystemControlPayload) {
    use project_phoenix::lobby::InboundMessage;
    use project_phoenix::messages::{ClientMessage, SystemId};
    app.world_mut()
        .resource_mut::<bevy::ecs::message::Messages<InboundMessage>>()
        .write(InboundMessage {
            token: REPAIR_DISPATCH_TOKEN.into(),
            msg: ClientMessage::ControlSystem {
                target: SystemId(project_phoenix::ship::system_registry::REPAIR_SYSTEM_ID.into()),
                payload,
            },
        });
    run(app, 2);
}

/// AC: a dispatched team raises an ally's condition at the authored rate while
/// the hull's own repairs slow (its one team is withdrawn from the sweep), and
/// recall — or drifting out of range — brings the team home and stops the work,
/// leaving what it already did on the ally.
#[test]
fn a_dispatched_team_raises_an_allys_condition_while_the_hulls_own_repairs_slow() {
    use project_phoenix::console::repair::ExternalRepairRefusal;
    use project_phoenix::messages::SystemControlPayload;

    let dt = 1.0 / 60.0;
    let mut app = build_headless_app(&external_repair_args(dt)).expect("app should build");
    // Far enough in that the game is InProgress and the tender is backfilled, so
    // its engineering-owned `repair` system is AI-controlled and the `ai:` token
    // is admitted.
    run(&mut app, 60);
    assert_eq!(
        app.world().resource::<State<GamePhase>>().get(),
        &GamePhase::InProgress,
        "precondition: admission only runs InProgress"
    );
    let ally = scan_uuid_named(&mut app, ALLY);

    place_operator(&mut app, Vec3::ZERO);
    run(&mut app, 1);

    // Precondition: one team free for the tender's own sweep, none dispatched.
    assert_eq!(
        operator_free_team_count(&mut app),
        1,
        "the tender starts with its one team free for its own damage control"
    );
    assert!(external_dispatch_of(&mut app).dispatched_target.is_none());

    // Designate the ally and dispatch a team to it.
    set_tractor_lock(&mut app, Some(ally.clone()));
    send_repair(&mut app, SystemControlPayload::DispatchExternalRepair);
    let d = external_dispatch_of(&mut app);
    assert_eq!(
        d.dispatched_target.as_deref(),
        Some(ally.as_str()),
        "the team crossed to the designated ally"
    );
    assert!(
        d.last_refusal.is_none(),
        "a clean dispatch carries no refusal"
    );

    // ── The hull's own repairs slow: the one team is withdrawn from the sweep ─
    assert_eq!(
        operator_free_team_count(&mut app),
        0,
        "the dispatched team is unavailable to the tender's own damage-control sweep — the same \
         availability answer the console readout and the repair AI both read"
    );

    // ── The ally's condition rises at the authored rate ──────────────────────
    let before = condition_of(&mut app, ALLY);
    run(&mut app, 120); // 2 sim-seconds working the ally
    let after = condition_of(&mut app, ALLY);
    let gain = after - before;
    // Net +20/s (the ally declines at zero of its own) for two seconds is +40.
    assert!(
        gain > 35.0 && gain < 45.0,
        "the dispatched team raises the ally at the authored +20/s (≈+40 over two seconds), got \
         {gain} (from {before} to {after})"
    );

    // ── Recall returns the team to the sweep and leaves the work done ─────────
    let banked = condition_of(&mut app, ALLY);
    send_repair(&mut app, SystemControlPayload::RecallExternalRepair);
    let d = external_dispatch_of(&mut app);
    assert!(
        d.dispatched_target.is_none(),
        "recall brought the team home"
    );
    assert_eq!(
        operator_free_team_count(&mut app),
        1,
        "the recalled team is back in the tender's own sweep"
    );
    run(&mut app, 120); // 2 sim-seconds home
    let after_recall = condition_of(&mut app, ALLY);
    assert!(
        (after_recall - banked).abs() < 1.0,
        "recall leaves the work already done on the ally and stops adding more: {banked} vs \
         {after_recall}"
    );

    // ── Drifting out of range brings the team home the same way ──────────────
    place_operator(&mut app, Vec3::ZERO);
    run(&mut app, 1);
    send_repair(&mut app, SystemControlPayload::DispatchExternalRepair);
    assert_eq!(
        external_dispatch_of(&mut app).dispatched_target.as_deref(),
        Some(ally.as_str()),
        "re-dispatched with the team free again"
    );
    let banked = condition_of(&mut app, ALLY);
    place_operator(&mut app, Vec3::new(90_000.0, 0.0, 90_000.0));
    run(&mut app, 3);
    let d = external_dispatch_of(&mut app);
    assert!(
        d.dispatched_target.is_none(),
        "drifting past the authored range brought the team home"
    );
    assert_eq!(
        d.last_refusal,
        Some(ExternalRepairRefusal::OutOfRange),
        "…and the reason the console shows is out-of-range"
    );
    assert_eq!(
        operator_free_team_count(&mut app),
        1,
        "the team dropped by range is back in the sweep"
    );
    run(&mut app, 60);
    assert!(
        (condition_of(&mut app, ALLY) - banked).abs() < 1.0,
        "a team dropped out of range stops working the ally where it left off"
    );
}

/// AC: dispatching with no designated target, or a target out of range, is
/// refused with a reason the console shows — proved end to end through the same
/// admitted command, distinct from the pure `dispatch_status` unit tests.
#[test]
fn dispatching_with_no_target_or_out_of_range_is_refused_with_a_shown_reason() {
    use project_phoenix::console::repair::ExternalRepairRefusal;
    use project_phoenix::messages::SystemControlPayload;

    let dt = 1.0 / 60.0;
    let mut app = build_headless_app(&external_repair_args(dt)).expect("app should build");
    run(&mut app, 60);
    let ally = scan_uuid_named(&mut app, ALLY);

    // No designated target: refused NoTarget, nobody dispatched.
    place_operator(&mut app, Vec3::ZERO);
    set_tractor_lock(&mut app, None);
    run(&mut app, 1);
    send_repair(&mut app, SystemControlPayload::DispatchExternalRepair);
    let d = external_dispatch_of(&mut app);
    assert!(d.dispatched_target.is_none(), "no target — nobody sent");
    assert_eq!(d.last_refusal, Some(ExternalRepairRefusal::NoTarget));

    // Designated but out of range: refused OutOfRange, nobody dispatched.
    place_operator(&mut app, Vec3::new(50_000.0, 0.0, 0.0));
    run(&mut app, 1);
    set_tractor_lock(&mut app, Some(ally.clone()));
    send_repair(&mut app, SystemControlPayload::DispatchExternalRepair);
    let d = external_dispatch_of(&mut app);
    assert!(d.dispatched_target.is_none(), "out of range — nobody sent");
    assert_eq!(d.last_refusal, Some(ExternalRepairRefusal::OutOfRange));
}

// ── #1160: the transfer umbilical, end to end ────────────────────────────────
//
// `assets/worlds/probe_umbilical.toml` fields a backfilled player UMBILICAL PROBE
// (the dedicated `umbilical_probe` hull — the shipped destroyer is deliberately
// not touched) and a passive DEPOT 100 units to starboard. The probe carries two
// seats on one ship: Helm owns the dock (#1159), Engineering owns the umbilical.
// The test drives the dock through the ordinary admitted `Dock` path and the flow
// through the ordinary admitted `StartTransfer`/`StopTransfer` path — proving the
// authored `reserve_fuel` capacity moves between the two DOCKED hulls' ledgers,
// and that undock, power loss and umbilical damage each stop the flow where it
// stands while keeping what has already moved.

const DEPOT_NAME: &str = "world.probe_umbilical.entity.depot.name";
/// `ai:` tokens: admission authorises each iff its target system is AI-controlled,
/// which the probe's helm-owned dock and engineering-owned umbilical both are while
/// nobody is at their consoles — the same seam #1162's AI will use.
const UMBILICAL_DOCK_TOKEN: &str = "ai:umbilical-dock";
const UMBILICAL_FLOW_TOKEN: &str = "ai:umbilical-flow";

fn umbilical_args(dt: f64) -> HeadlessArgs {
    HeadlessArgs {
        world_path: "assets/worlds/probe_umbilical.toml".into(),
        ship_path: "assets/entities/umbilical_probe.toml".into(),
        dt,
        max_ticks: ticks_for_sim_seconds(120.0, dt),
        deterministic: true,
        seed: Some(1160),
        ..test_args()
    }
}

fn umbilical_operator(app: &mut App) -> Entity {
    app.world_mut()
        .query_filtered::<Entity, With<LocalShip>>()
        .iter(app.world())
        .next()
        .expect("the probe world spawns a local umbilical probe")
}

fn umbilical_of(app: &mut App) -> project_phoenix::umbilical::TransferUmbilical {
    let op = umbilical_operator(app);
    app.world()
        .get::<project_phoenix::umbilical::TransferUmbilical>(op)
        .expect("the probe carries a TransferUmbilical")
        .clone()
}

fn umbilical_dock_of(app: &mut App) -> project_phoenix::dock::DockControl {
    let op = umbilical_operator(app);
    app.world()
        .get::<project_phoenix::dock::DockControl>(op)
        .expect("the probe carries a DockControl")
        .clone()
}

/// Place the operator by writing its `ShipPhysics`, which `sync_ship_position`
/// projects into the transform the dock manoeuvre reads.
fn place_umbilical_operator(app: &mut App, position: Vec3) {
    let op = umbilical_operator(app);
    let mut physics = app
        .world_mut()
        .get_mut::<ShipPhysics>(op)
        .expect("the probe is a ship");
    physics.x = position.x;
    physics.y = position.y;
    physics.z = position.z;
}

fn depot_entity(app: &mut App) -> Entity {
    use project_phoenix::entities::spawner::EntityName;
    app.world_mut()
        .query::<(Entity, &EntityName)>()
        .iter(app.world())
        .find(|(_, n)| n.0 == DEPOT_NAME)
        .map(|(e, _)| e)
        .expect("the probe world spawns a depot")
}

fn move_depot(app: &mut App, position: Vec3) {
    let depot = depot_entity(app);
    app.world_mut()
        .get_mut::<Transform>(depot)
        .expect("the depot has a transform")
        .translation = position;
}

/// Read a hull's `reserve_fuel` level off its infrastructure ledger.
fn fuel_of(app: &mut App, entity: Entity) -> i64 {
    app.world()
        .get::<project_phoenix::infrastructure::InfrastructureCondition>(entity)
        .expect("the hull carries an infrastructure ledger")
        .0
        .capacity("reserve_fuel")
        .expect("the ledger declares reserve_fuel")
}

/// Send a dock/undock through the real admission path (Helm's seat).
fn send_umbilical_dock(app: &mut App, payload: project_phoenix::messages::SystemControlPayload) {
    use project_phoenix::lobby::InboundMessage;
    use project_phoenix::messages::{ClientMessage, SystemId};
    app.world_mut()
        .resource_mut::<bevy::ecs::message::Messages<InboundMessage>>()
        .write(InboundMessage {
            token: UMBILICAL_DOCK_TOKEN.into(),
            msg: ClientMessage::ControlSystem {
                target: SystemId(project_phoenix::ship::system_registry::DOCK_SYSTEM_ID.into()),
                payload,
            },
        });
    run(app, 2);
}

/// Send a start/stop through the real admission path (Engineering's seat).
fn send_umbilical_flow(app: &mut App, payload: project_phoenix::messages::SystemControlPayload) {
    use project_phoenix::lobby::InboundMessage;
    use project_phoenix::messages::{ClientMessage, SystemId};
    app.world_mut()
        .resource_mut::<bevy::ecs::message::Messages<InboundMessage>>()
        .write(InboundMessage {
            token: UMBILICAL_FLOW_TOKEN.into(),
            msg: ClientMessage::ControlSystem {
                target: SystemId(
                    project_phoenix::ship::system_registry::UMBILICAL_SYSTEM_ID.into(),
                ),
                payload,
            },
        });
    run(app, 2);
}

fn cut_umbilical_power(app: &mut App) {
    let op = umbilical_operator(app);
    let mut ps = app
        .world_mut()
        .get_mut::<project_phoenix::ship::power::ShipPowerSystem>(op)
        .expect("the probe has a power system");
    let _ = ps.0.set_group_allocation(
        &project_phoenix::messages::PowerGroupId("umbilical".into()),
        1,
    );
}

fn restore_umbilical_power(app: &mut App) {
    let op = umbilical_operator(app);
    let mut ps = app
        .world_mut()
        .get_mut::<project_phoenix::ship::power::ShipPowerSystem>(op)
        .expect("the probe has a power system");
    let _ = ps.0.set_group_allocation(
        &project_phoenix::messages::PowerGroupId("umbilical".into()),
        2,
    );
}

fn disable_umbilical(app: &mut App) {
    let op = umbilical_operator(app);
    let mut hull = app
        .world_mut()
        .get_mut::<project_phoenix::entity_spawner::EntitySystemHull>(op)
        .expect("the probe has a hull");
    hull.0.set_hp(
        &project_phoenix::messages::SystemId("umbilical".into()),
        1.0,
    );
}

fn repair_umbilical(app: &mut App) {
    let op = umbilical_operator(app);
    let mut hull = app
        .world_mut()
        .get_mut::<project_phoenix::entity_spawner::EntitySystemHull>(op)
        .expect("the probe has a hull");
    hull.0.set_hp(
        &project_phoenix::messages::SystemId("umbilical".into()),
        30.0,
    );
}

/// Reset the probe to the origin and the depot 100 units to starboard, engage the
/// dock, and fly the manoeuvre to completion — the clean docked state each
/// interruption starts from.
fn umbilical_redock(app: &mut App) {
    use project_phoenix::messages::SystemControlPayload;
    // Stop the flow and undock, then run long enough for the ship to back fully
    // clear so any in-progress `undock_target` clears — a lingering one would
    // steer the ship away from the berth the moment we re-issue Dock.
    send_umbilical_flow(app, SystemControlPayload::StopTransfer);
    send_umbilical_dock(app, SystemControlPayload::Undock);
    run(app, 200);
    // Settle the two hulls back into their start pose and re-dock.
    place_umbilical_operator(app, Vec3::ZERO);
    move_depot(app, Vec3::new(100.0, 0.0, 0.0));
    run(app, 2);
    send_umbilical_dock(app, SystemControlPayload::Dock);
    run(app, 160);
    let dock = umbilical_dock_of(app);
    assert!(
        dock.docked && dock.docking_target.is_some(),
        "precondition: the two hulls should be mated, got {dock:?}"
    );
}

/// AC: capacity moves between two docked hulls while the umbilical runs, and
/// undock, power loss and umbilical damage each stop it while keeping what has
/// moved. Also: the running flag folds and survives a snapshot resume.
#[test]
fn capacity_moves_between_two_docked_hulls_and_stops_on_undock_power_loss_and_damage() {
    use project_phoenix::messages::SystemControlPayload;
    use project_phoenix::umbilical::UmbilicalRefusal;

    let dt = 1.0 / 60.0;
    let mut app = build_headless_app(&umbilical_args(dt)).expect("app should build");
    // Far enough in that the game is InProgress and the probe is backfilled, so
    // its helm dock and engineering umbilical are AI-controlled and the `ai:`
    // tokens are admitted.
    run(&mut app, 60);
    assert_eq!(
        app.world().resource::<State<GamePhase>>().get(),
        &GamePhase::InProgress,
        "precondition: admission only runs InProgress"
    );
    let depot = depot_entity(&mut app);
    let operator = umbilical_operator(&mut app);

    // ── Dock the two hulls ───────────────────────────────────────────────────
    place_umbilical_operator(&mut app, Vec3::ZERO);
    move_depot(&mut app, Vec3::new(100.0, 0.0, 0.0));
    run(&mut app, 2);
    send_umbilical_dock(&mut app, SystemControlPayload::Dock);
    run(&mut app, 140);
    assert!(
        umbilical_dock_of(&mut app).docked,
        "precondition: the two hulls should mate before the flow runs"
    );

    // The authored start levels: the operator full, the depot empty.
    let op_start = fuel_of(&mut app, operator);
    let depot_start = fuel_of(&mut app, depot);
    assert_eq!(op_start, 100, "the operator starts full");
    assert_eq!(depot_start, 0, "the depot starts empty");

    // ── Start the flow: capacity crosses the umbilical ───────────────────────
    send_umbilical_flow(&mut app, SystemControlPayload::StartTransfer);
    run(&mut app, 90); // ~1.5s at rate 20/s → ~30 units, still partial
    let op_mid = fuel_of(&mut app, operator);
    let depot_mid = fuel_of(&mut app, depot);
    assert!(
        op_mid < op_start,
        "the operator's ledger drained, got {op_mid} from {op_start}"
    );
    assert!(
        depot_mid > depot_start,
        "the depot's ledger filled, got {depot_mid} from {depot_start}"
    );
    assert_eq!(
        op_start - op_mid,
        depot_mid - depot_start,
        "unit for unit — nothing is created or lost across the dock"
    );
    assert_eq!(
        op_mid + depot_mid,
        100,
        "the total across the two ledgers is conserved"
    );
    assert!(
        umbilical_of(&mut app).running,
        "the flow is still running mid-transfer"
    );

    // ── Undock: the flow stops and keeps what has moved ──────────────────────
    send_umbilical_dock(&mut app, SystemControlPayload::Undock);
    run(&mut app, 3);
    let u = umbilical_of(&mut app);
    assert!(!u.running, "undocking stopped the flow");
    assert_eq!(u.last_refusal, Some(UmbilicalRefusal::Undocked));
    let op_after_undock = fuel_of(&mut app, operator);
    let depot_after_undock = fuel_of(&mut app, depot);
    run(&mut app, 30);
    assert_eq!(
        fuel_of(&mut app, operator),
        op_after_undock,
        "what moved stays moved — the operator's ledger is frozen after undock"
    );
    assert_eq!(
        fuel_of(&mut app, depot),
        depot_after_undock,
        "…and the depot's is too"
    );

    // ── Power loss stops a running flow ──────────────────────────────────────
    umbilical_redock(&mut app);
    send_umbilical_flow(&mut app, SystemControlPayload::StartTransfer);
    run(&mut app, 20);
    assert!(
        umbilical_of(&mut app).running,
        "the flow runs before power is cut"
    );
    cut_umbilical_power(&mut app);
    run(&mut app, 3);
    let u = umbilical_of(&mut app);
    assert!(!u.running, "losing power stopped the flow");
    assert_eq!(u.last_refusal, Some(UmbilicalRefusal::Unpowered));
    let op_after_power = fuel_of(&mut app, operator);
    run(&mut app, 20);
    assert_eq!(
        fuel_of(&mut app, operator),
        op_after_power,
        "what moved stays moved after a power loss"
    );

    // ── Umbilical damage stops a running flow ────────────────────────────────
    restore_umbilical_power(&mut app);
    umbilical_redock(&mut app);
    send_umbilical_flow(&mut app, SystemControlPayload::StartTransfer);
    run(&mut app, 20);
    assert!(
        umbilical_of(&mut app).running,
        "the flow runs before it is damaged"
    );
    disable_umbilical(&mut app);
    run(&mut app, 3);
    let u = umbilical_of(&mut app);
    assert!(!u.running, "a disabled umbilical stopped the flow");
    assert_eq!(u.last_refusal, Some(UmbilicalRefusal::Disabled));
    let op_after_damage = fuel_of(&mut app, operator);
    run(&mut app, 20);
    assert_eq!(
        fuel_of(&mut app, operator),
        op_after_damage,
        "what moved stays moved after umbilical damage"
    );

    // ── The running flag folds and survives a snapshot resume ────────────────
    repair_umbilical(&mut app);
    umbilical_redock(&mut app);
    send_umbilical_flow(&mut app, SystemControlPayload::StartTransfer);
    run(&mut app, 5);
    assert!(
        umbilical_of(&mut app).running,
        "precondition: a running flow to capture"
    );
    let digest_running = project_phoenix::sim_digest::state_digest(&app);
    let snap = project_phoenix::snapshot::capture(app.world());
    send_umbilical_flow(&mut app, SystemControlPayload::StopTransfer);
    run(&mut app, 3);
    assert!(
        !umbilical_of(&mut app).running,
        "precondition: the flow is stopped before the restore"
    );
    project_phoenix::snapshot::restore(app.world_mut(), &snap);
    assert!(
        umbilical_of(&mut app).running,
        "the running flow survived the snapshot resume"
    );
    assert_eq!(
        project_phoenix::sim_digest::state_digest(&app),
        digest_running,
        "…and the restored world folds to the same digest the captured one did"
    );
}
