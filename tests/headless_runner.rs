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
use project_phoenix::balance::RunOutcome;
use project_phoenix::headless::args::ticks_for_sim_seconds;
use project_phoenix::headless::{build_headless_app, build_report, run, HeadlessArgs};
use project_phoenix::messages::GamePhase;
use project_phoenix::server_app::LocalShip;
use project_phoenix::ship::control_source::ControlSource;
use project_phoenix::ship::state::ShipPhysics;
use project_phoenix::ship_plugin::{ActiveStationRatings, ShipSystemControlSources};

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

/// Runs `patrol` for `sim_secs` at `hz` and returns the ship's position.
///
/// `patrol.toml`, not `combat_test.toml` (issue #842): the player hull now
/// carries a default `[behaviour]` doctrine, so in `combat_test` the backfilled
/// player proactively engages, and combat pursuit is a chaotic feedback loop
/// that is *legitimately* frame-rate-coupled (the `HELM_AI_MAX_DT_SECS` clamp —
/// the very coupling PRD #620 exists to remove). Measuring the player's position
/// there would conflate the fixed-timestep guarantee this test exists to check
/// with combat-AI rate-coupling that is out of its scope. In `patrol.toml` the
/// backfilled player travels a deterministic, non-combat course (~51 units in
/// 4 s), which stays rate-independent to well under a unit across 30/60/120 Hz —
/// exactly the property under test, without the combat confound.
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

/// The point of the fixed timestep: the *simulation* — not just the clock —
/// lands in the same place regardless of tick rate.
///
/// Only holds at or above 30 Hz. `HELM_AI_MAX_DT_SECS` (`ship_plugin.rs`)
/// clamps the AI helm integration step to 1/30 s, so slower rates
/// under-integrate and the ship falls behind. That clamp is the frame-rate
/// coupling PRD #620 exists to remove; until it is gone, 30 Hz is the floor for
/// a faithful run. Measured on `patrol.toml`, where the backfilled player
/// travels a deterministic non-combat course — see `ship_position_after` for why
/// a combat scenario is the wrong place to assert this after #842.
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
#[test]
fn backfilled_player_hull_proactively_engages_on_template_doctrine() {
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
/// Seed 9 is measured, not assumed. Across seeds 1–12 every run reaches
/// GameOver as a defeat, and 9 is the earliest resolution of the set: player
/// `damage_dealt` ~265 and `damage_taken` ~1200 against the `> 0` thresholds
/// below, 5 kills against `> 0`.
///
/// Note the seed does *not* make `combat_test` bit-reproducible: measured over
/// 11 runs at this seed and rate, resolution lands anywhere in 246–275
/// sim-seconds (the belt-dense scenario has a second variance source beyond the
/// RNG — `--deterministic` is already in force, so per-process `HashMap` seeding
/// is the likely culprit; `rng_coverage.toml` under `tests/rng_determinism.rs`
/// is byte-identical run to run, so this is scenario-specific, not a hole in the
/// seeded-RNG guarantee). The budget is 400 s rather than 300 s so the slowest
/// observed resolution clears by ~45 %: the timing assertion is the only one
/// that could flake, and every other assertion is a `> 0` that holds across the
/// whole spread. The seed removes one source of drift, it does not carry the
/// test.
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
