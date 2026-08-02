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
        max_ticks: ticks_for_sim_seconds(60.0, dt),
        deterministic: true,
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
/// exactly once.
///
/// Two emitters used to fire for one kill: the weapon kill site
/// (`tick_beams_apply_damage` and siblings) despawned the entity and pushed
/// `EntityDespawned`, then the reconcile sweep (`reconcile_runtime_entities`)
/// pushed a *second* one because the kill site never cleared the uuid from the
/// `TrackedEntities` registry. The fix has every kill site call
/// `TrackedEntities::forget`, so the sweep no longer re-emits.
///
/// `probe_despawn.toml` produces exactly one kill (a Harrow battleship destroys
/// a Federation destroyer, far from the uninvolved player), so the count of
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
        max_ticks: ticks_for_sim_seconds(60.0, dt),
        deterministic: true,
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
/// This boots the real `before_the_fire.toml`, so it covers the whole chain the
/// bug ran through: the template's directive fields, the world's anchor table,
/// `plan_helm_travel`'s `Reach` arm, and the per-axis helm actuators.
#[test]
fn requiem_courier_reaches_its_destination_anchor() {
    use project_phoenix::entity_spawner::EntityName;

    let dt = 1.0 / 30.0;
    let args = HeadlessArgs {
        world_path: "assets/worlds/before_the_fire.toml".into(),
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
            let a = world_config
                .anchors
                .get(name)
                .unwrap_or_else(|| panic!("before_the_fire.toml must declare the `{name}` anchor"));
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
        .find(|(name, _)| name.0 == "world.entity.requiem_courier.name")
        .map(|(_, physics)| (physics.x, physics.z, physics.forward_speed))
        .expect("before_the_fire.toml spawns the Requiem Courier");

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
/// Seed 9 is measured, not assumed — RE-MEASURED for #897's generator swap
/// (`rand`'s SmallRng -> `vellum_rng::Pcg32`), which re-rolls which system
/// every hit lands on and so can move which seeds resolve and when. On the
/// current generator (`phoenix-headless --world assets/worlds/combat_test.toml
/// --seed 9 --hz 30 --sim-seconds 400 --deterministic --report-format json`),
/// seed 9 still reaches `GameOver` as a defeat: player `damage_dealt` 79.0 and
/// `damage_taken` 493.808 against the `> 0` thresholds below, 1 kill (`wave_4`)
/// against the `> 0` floor. The old figures here (~265 / ~1200 / 5 kills) and
/// the claim that 9 is "the earliest resolution" of a 1–12 sweep both predate
/// #897 and were not re-verified — this re-measurement covers only seed 9,
/// which is all the test pins.
///
/// The old note also claimed `combat_test` is *not* bit-reproducible at this
/// seed (11 runs landing anywhere in 246–275 sim-seconds, blamed on
/// per-process `HashMap` seeding). That does not reproduce here either: 12
/// consecutive runs on the current generator and build produced byte-identical
/// exit reports, all resolving at tick 10899 / sim_t 181.6667 s. Whatever
/// drove the old spread, it is not observed under this generator — treat this
/// scenario as bit-reproducible at this seed until shown otherwise, not as
/// inherently noisy. The 400 s budget (vs. 300 s) predates this finding and is
/// left as-is: it still clears the observed 181.7 s resolution with room to
/// spare, and the timing assertion remains the only one that could flake if a
/// future change reopens that variance.
#[test]
fn combat_test_develops_two_sided_combat_and_resolves() {
    let dt = 1.0 / 30.0;
    let args = HeadlessArgs {
        world_path: "assets/worlds/combat_test.toml".into(),
        dt,
        max_ticks: ticks_for_sim_seconds(400.0, dt),
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
        "combat_test did not resolve within 400s — final_phase {:?}, ship {:?}",
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
    // is seed-dependent (the scenario's victory triggers and the player-death
    // latch both feed the flag), so this stays deliberately outcome-agnostic.
    assert!(
        matches!(
            report.outcome_report.outcome,
            RunOutcome::Victory | RunOutcome::Defeat
        ),
        "a resolved combat_test run must classify as victory or defeat, got {:?}",
        report.outcome_report.outcome
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
/// — the shape `duel.toml` and `combat_test.toml`'s wave 8 both use, and the one
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
        is_command_authorized("ai:npc", &radar, &payload, &sources, &sessions, &config.0),
        "the backfilled seat's AI must hold the radar before the human sits down"
    );
    assert!(
        !is_command_authorized(
            "player-token",
            &radar,
            &payload,
            &sources,
            &sessions,
            &config.0
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
            &config.0
        ),
        "a human holding the NPC hull's Tactical seat must be admitted to its radar"
    );
    assert!(
        !is_command_authorized("ai:npc", &radar, &payload, &sources, &sessions, &config.0),
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
            &config.0
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
    assert!(
        duel.orbit > 300,
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
/// The `requested` count is the control condition, and it is asserted on the
/// DUEL alone. Nothing here raises a request by hand — the cruiser's own Weapons
/// AI does, because its three tubes are 90-degree fore/aft and a settled ring
/// puts the target on the beam — and without it this test would go green while
/// measuring an engagement in which nothing was ever declined. It is not
/// asserted on the aggressor probe because there it is legitimately near zero:
/// that hull spends the run in the torpedo leg, whose whole purpose is to
/// SATISFY the request rather than decline it (#876). Measured across the duel's
/// 45 s: 1336 ring ticks with the leg yielding, 41 of them flown at a yaw the
/// doctrine did not solve (worst 2.0 — a full-scale reversal); with the leg
/// declining, 1307 ring ticks and none.
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
        r
    };

    let duel = sample("assets/worlds/probe_duel.toml", 45.0);
    let aggressor = sample("assets/worlds/probe_aggressor.toml", 30.0);

    // The control: in the world where the ring IS the engagement, the hull
    // genuinely spent ring ticks with a request standing against it.
    assert!(
        duel.requested > 0,
        "the cruiser's Weapons never had a standing arc-bearing request across {} \
         ring ticks of a live duel. Nothing was declined, so this probe proves \
         nothing — the control condition has rotted",
        duel.ticks
    );

    for (name, ring, min_ticks) in [("duel", &duel, 300), ("aggressor", &aggressor, 50)] {
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

/// **A backfilled hull's phasers reach the `beam_range` its own file authors,
/// in a real engagement (issue #923).**
///
/// The unit pin in `modifiers::coordination` resolves the authored policy by
/// hand; this one lets the SHIPPED schedule do it — captain AI raises the alert
/// off real combat, `ai_power_allocation` resolves the `sensors` channel and
/// emits an admitted `SetPowerGroupAllocation`, `handle_power_messages` applies
/// it, and `translate_power_modifiers` folds it into `ModifierSlot::RadarRange`.
/// Nothing here sets a power level or an alert by hand: if any link in that
/// chain stops firing the reach never reaches 1.0 and this fails.
///
/// It is deliberately a MAXIMUM over the run rather than a reading at the end.
/// Combat stations is a burst the battery pays for — the sensor point is shed
/// again below the authored reserve, and by 45 s the cruiser in `probe_duel`
/// has long since dropped back to its resting allocation. The claim is "the
/// hull reaches its authored range while it is fighting", not "for ever", and
/// the two-thirds floor is asserted alongside it so a regression in either
/// direction is visible.
#[test]
fn a_backfilled_cruiser_reaches_its_authored_beam_range_while_at_combat_stations() {
    use project_phoenix::messages::ModifierSlot;
    use project_phoenix::modifiers::ShipModifiers;

    // The authored number this test is about, read off the shipped hull rather
    // than restated — a retune of the bank retunes the assertion with it.
    let hull = project_phoenix::entity_includes::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .expect("the shipped cruiser composes");
    let authored_beam_range = hull
        .weapons_console
        .as_ref()
        .and_then(|wc| wc.phaser_banks.iter().find(|b| b.id == "fore"))
        .map(|b| b.beam_range)
        .expect("the cruiser authors a `fore` phaser bank with a beam_range");
    assert!(authored_beam_range > 0.0);

    let dt = 1.0 / 30.0;
    let args = HeadlessArgs {
        world_path: "assets/worlds/probe_duel.toml".into(),
        dt,
        max_ticks: ticks_for_sim_seconds(45.0, dt),
        deterministic: true,
        ..test_args()
    };
    let mut app = build_headless_app(&args).expect("probe_duel must build an app");

    let mut best: f32 = 0.0;
    let mut worst: f32 = f32::MAX;
    for _ in 0..args.max_ticks {
        run(&mut app, 1);
        let mut q = app
            .world_mut()
            .query_filtered::<&ShipModifiers, With<LocalShip>>();
        let Ok(mods) = q.single(app.world()) else {
            continue;
        };
        let reach = authored_beam_range * mods.get(&ModifierSlot::RadarRange);
        best = best.max(reach);
        worst = worst.min(reach);
    }

    assert!(
        (best - authored_beam_range).abs() < 1e-3,
        "the cruiser's best effective phaser reach across 45 s of a live duel was \
         {best:.2} against an authored beam_range of {authored_beam_range:.2}. The \
         sensors → RadarRange → reach chain is not delivering nominal reach at \
         combat stations, so every doctrine range on this hull is sized against a \
         number its guns never have"
    );
    // The control, and it matters: a slot pinned at 1.0 for the whole run would
    // satisfy the assertion above while proving nothing about the power chain.
    // Reach must also read BELOW nominal somewhere, which it does for two
    // independent reasons — the sensor point is shed below its authored reserve,
    // and `apply_radar_damage_modifiers` drives the same slot when this hull's
    // tactical radar is hit. No lower BOUND is asserted, for exactly that second
    // reason: a destroyed radar takes the slot to ~0.001 by design.
    assert!(
        worst < authored_beam_range,
        "effective reach never left {authored_beam_range:.2} across the whole run, so \
         this probe never exercised the power chain it claims to measure — something \
         other than the sensors group is holding the RadarRange slot at 1.0"
    );
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
/// Deliberately not just the player ship: a slice narrow enough to miss a
/// divergence is worse than no assertion at all. It folds the logical tick
/// count, the seeded RNG's position on EVERY stream, and every ship's physics
/// and hull integrity — the three places a frame-coupled system would show up
/// (an extra/missed step, an extra/missed random draw, an extra/missed
/// integration or damage application).
#[derive(Debug, PartialEq)]
struct RunFingerprint {
    tick: u64,
    seed: u64,
    /// One probe draw per `SimStream`, taken after the run: two runs that made
    /// a different NUMBER of draws on any stream land at different positions,
    /// so this catches divergence in the damage/uuid paths without needing the
    /// generators' private state. The width is the generator's (`Pcg32` draws
    /// 32 bits); the comparison is generator-agnostic either way.
    rng_positions: Vec<u32>,
    /// `(entity index, x, z, yaw, forward_speed, hull current, hull max)` for
    /// every `Ship`, sorted by entity index — which is itself part of the
    /// comparison, so a run that spawned a different number of entities, or
    /// spawned them in a different order, fails here too.
    ships: Vec<(bevy::ecs::entity::EntityIndex, f32, f32, f32, f32, f32, f32)>,
}

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
/// # Why `patrol.toml`, and what that excludes
/// Rapier is still driven once per rendered FRAME (`TimestepMode::Fixed { dt }`
/// in `headless::app`, where `dt` is the frame period) — moving it onto the
/// logical tick is issue #896's slice, not this one. So anything whose state
/// depends on the physics pipeline — collisions, and therefore collision hull
/// damage and the `SimStream::CollisionDamage` draws it makes — would still
/// diverge between these two drives, and this test would fail for a reason
/// #895 cannot fix. `patrol.toml` is chosen because its backfilled player flies
/// a deterministic non-contact course: no collision ever fires, so rapier
/// feeds nothing back into ship state and what remains under test is exactly
/// the schedule. **A collision-bearing world (`combat_test`, `duel`) cannot be
/// added here until #896 lands** — when it does, this test should gain a
/// second world that does collide.
#[test]
fn the_simulation_reaches_the_same_state_at_wildly_different_frame_rates() {
    use project_phoenix::entity_spawner::EntitySystemHull;
    use project_phoenix::sim_rng::{SimRng, SimStream};
    use project_phoenix::sim_tick::SimTick;
    use project_phoenix::simulation::Ship;

    /// Drive `frames` frames of `ticks_per_frame` logical ticks each and
    /// fingerprint the world it leaves behind.
    fn end_state(frames: u64, ticks_per_frame: u32) -> RunFingerprint {
        let dt = 1.0 / 60.0;
        let args = HeadlessArgs {
            world_path: "assets/worlds/patrol.toml".into(),
            dt,
            max_ticks: 0, // driven by hand below
            seed: Some(42),
            deterministic: true,
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

        let mut ships: Vec<_> = app
            .world_mut()
            .query_filtered::<(Entity, &ShipPhysics, Option<&EntitySystemHull>), With<Ship>>()
            .iter(app.world())
            .map(|(e, p, hull)| {
                let (current, max) =
                    hull.map_or((0.0, 0.0), |h| (h.0.total_current(), h.0.total_max()));
                (e.index(), p.x, p.z, p.yaw, p.forward_speed, current, max)
            })
            .collect();
        ships.sort_by_key(|s| s.0);

        let rng = app.world().resource::<SimRng>();
        let rng_positions = SimStream::ALL
            .iter()
            .map(|s| rng.stream(*s).next_u32())
            .collect();

        RunFingerprint {
            tick: app.world().resource::<SimTick>().0,
            seed: rng.seed(),
            rng_positions,
            ships,
        }
    }

    let per_tick = end_state(240, 1);
    let per_four = end_state(60, 4);
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
