//! Recorded GM continuation through the full native boot graph (#1316).
//!
//! This separate binary owns the process-global native config cache. Its cases
//! serialize boot/restore because the frozen content ledger is process-global.
#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]

/// Run the already-built test executable with:
/// `--ignored --exact verify_browser_gm_exports --nocapture`.
/// The two inputs are ordinary browser StoredRun RON files; OUTPUT receives the
/// machine-readable report, including structured refusal on an invalid proof.
#[test]
#[ignore = "requires actual initial/final browser exports and a report path"]
fn verify_browser_gm_exports() {
    let _guard = RECORDING_TEST.lock().unwrap();
    let read = |key| std::fs::read_to_string(std::env::var(key).expect(key)).expect(key);
    let initial = read("PHOENIX_GM_RECORDING_INITIAL");
    let final_run = read("PHOENIX_GM_RECORDING_FINAL");
    let output = std::env::var("PHOENIX_GM_RECORDING_REPORT").expect("PHOENIX_GM_RECORDING_REPORT");
    let result =
        project_phoenix::headless::replay::recorded_gm::replay_exports(&initial, &final_run);
    let value = match &result {
        Ok(replay) => serde_json::json!({ "pass": true, "report": replay.report }),
        Err(error) => serde_json::json!({ "pass": false, "error": error }),
    };
    std::fs::write(output, serde_json::to_string_pretty(&value).unwrap()).unwrap();
    if let Err(error) = result {
        panic!("browser recording did not verify: {error}");
    }
}

use bevy::prelude::*;
use project_phoenix::command_admission::HostSlot;
use project_phoenix::core::messages::{GamePhase, StationId, SystemControlPayload, SystemId};
use project_phoenix::entities::spawner::EntityUuid;
use project_phoenix::gm_action::*;
use project_phoenix::headless::replay::recorded_gm::replay_exports;
use project_phoenix::headless::{build_headless_app, HeadlessArgs};
use project_phoenix::lockstep::FleetRoster;
use project_phoenix::lockstep::{FleetGm, FleetShip};
use project_phoenix::sim_tick::SimTick;
use project_phoenix::snapshot::{self, StoredRun};

const WORLD: &str = "assets/worlds/default.toml";
static RECORDING_TEST: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn recorded_gm_exports_refuse_pre_collision_history_rules() {
    let _guard = RECORDING_TEST.lock().unwrap();
    let (initial, final_run, _, _, _) = recorded_pair();
    let mut old = initial.clone();
    old.versions.rules = "0.4".into();
    let error = replay_exports(
        &snapshot::export_artifact(&old).unwrap(),
        &snapshot::export_artifact(&final_run).unwrap(),
    )
    .err()
    .expect("old browser folds must not be silently rewritten");
    assert_eq!(error.stage, "versions");
}

fn source() -> (App, StationId, String, SystemId) {
    let args = HeadlessArgs {
        world_path: WORLD.into(),
        ship_path: "assets/entities/alliance_cruiser.toml".into(),
        seed: Some(1316),
        deterministic: true,
        max_ticks: 300,
        ..Default::default()
    };
    let mut app = build_headless_app(&args).unwrap();
    let config = project_phoenix::entities::include_resolve::load_entity_config(&args.ship_path)
        .unwrap()
        .ship_config
        .unwrap();
    let (station, rating, system) = config
        .stations
        .iter()
        .filter(|station| station.id.0 == "captain")
        .find_map(|station| {
            station.ratings.iter().find_map(|rating| {
                config
                    .systems_for_station(&station.id)
                    .find(|system| {
                        system.kind == "red_alert" && !rating.automated_systems.contains(&system.id)
                    })
                    .map(|system| (station.id.clone(), rating.name.clone(), system.id.clone()))
            })
        })
        .expect("the real cruiser offers human-controlled Captain red alert");
    let roster = FleetRoster::with_participants_and_gms(
        vec![FleetShip {
            host: HostSlot(1),
            ship_path: Some(args.ship_path),
            crew: vec![(station.clone(), rating.clone())],
        }],
        vec![HostSlot(1), HostSlot(2)],
        vec![FleetGm {
            host: HostSlot(2),
            operator_id: "gm-recorder".into(),
        }],
        HostSlot(1),
        HostSlot(1),
    )
    .unwrap();
    let mut sessions = project_phoenix::lobby::session::SessionManager::new();
    let token = "actual-held-station";
    sessions
        .register(token.into(), "Recording crew".into())
        .unwrap();
    sessions.set_station(token, Some(station.clone()));
    sessions.set_pending_rating(&station, rating.clone());
    app.insert_resource(project_phoenix::lobby::Sessions(sessions));
    app.insert_resource(roster);
    app.insert_resource(project_phoenix::save_slots_lifecycle::SaveCaptureConsumer);
    app.finish();
    app.cleanup();
    for _ in 0..60 {
        app.update();
    }
    assert_eq!(
        app.world().resource::<State<GamePhase>>().get(),
        &GamePhase::InProgress
    );
    (app, station, rating, system)
}

fn request(app: &mut App, action: GmAction) {
    let tick = app.world().resource::<SimTick>().0;
    let sequence = app.world().resource::<GmActionJournal>().next_sequence();
    app.world_mut()
        .resource_mut::<GmActionJournal>()
        .insert(GmActionGrant {
            from: HostSlot(2),
            sequenced_by: HostSlot(1),
            operator_id: "gm-recorder".into(),
            correlation: GmActionId::new(format!("recorded-{sequence}")).unwrap(),
            recovery_generation: 0,
            apply_tick: tick,
            order: GmActionOrder::new(HostSlot(2), sequence),
            action,
        })
        .unwrap();
    app.update();
}

fn captured(
    app: &mut App,
    reason: project_phoenix::save_slots::CaptureReason,
) -> Option<StoredRun> {
    let mut pending = app
        .world_mut()
        .resource_mut::<project_phoenix::save_slots_lifecycle::PendingStoredRuns>();
    while let Some(capture) = pending.pop_front() {
        if capture.decision.reason == reason {
            let text = snapshot::export_artifact(&capture.run).unwrap();
            return Some(StoredRun::from_ron(&text).unwrap());
        }
    }
    None
}

fn export(app: &mut App) -> StoredRun {
    project_phoenix::save_slots_lifecycle::request_manual_save(app.world_mut(), "recording");
    for _ in 0..3 {
        app.update();
        if let Some(run) = captured(app, project_phoenix::save_slots::CaptureReason::Manual) {
            return run;
        }
    }
    panic!("ordinary manual capture did not reach its fixed boundary");
}

fn assert_held(app: &mut App, station: &StationId, rating: &str, system: &SystemId) {
    let mut query = app.world_mut().query_filtered::<(
        &project_phoenix::ship::components::ActiveStationRatings,
        &project_phoenix::ship::components::ShipSystemControlSources,
    ), With<project_phoenix::server_app::LocalShip>>();
    let (ratings, sources) = query.single(app.world()).unwrap();
    assert_eq!(ratings.0.get(station).map(String::as_str), Some(rating));
    assert_eq!(
        sources.0.source_for(system),
        project_phoenix::ship::control_source::ControlSource::Human
    );
}

fn recorded_pair() -> (StoredRun, StoredRun, StationId, String, SystemId) {
    let (mut app, station, rating, system) = source();
    assert_held(&mut app, &station, &rating, &system);
    // A real applied origin prefix must remain applied, not be replayed again.
    request(&mut app, GmAction::SetSessionPaused { active: false });
    let initial = export(&mut app);
    let mut query = app
        .world_mut()
        .query_filtered::<&EntityUuid, With<project_phoenix::server_app::LocalShip>>();
    let target = query.single(app.world()).unwrap().0.clone();
    request(
        &mut app,
        GmAction::ApplyDirectEffect {
            target,
            scope: project_phoenix::gm_effect::GmDirectEffectScope::Entity,
            effect: project_phoenix::gm_effect::GmDirectEffectKind::Damage,
            amount_milli_hp: 20_000,
        },
    );
    request(&mut app, GmAction::SetSessionPaused { active: true });
    request(&mut app, GmAction::SetSessionPaused { active: false });
    for _ in 0..20 {
        app.update();
    }
    assert_held(&mut app, &station, &rating, &system);
    assert_eq!(
        app.world().resource::<GmActionJournal>().applied_results()[1].outcome,
        GmActionOutcome::Applied
    );
    assert!(
        app.world()
            .resource::<project_phoenix::command_admission::CommandLog>()
            .is_empty(),
        "no fabricated ordinary human command lane"
    );
    (initial, export(&mut app), station, rating, system)
}

#[test]
fn recorded_gm_exports_restore_exact_origin_and_continue_with_a_real_human_held_station() {
    let _guard = RECORDING_TEST.lock().unwrap();
    let (initial, final_run, station, rating, system) = recorded_pair();
    let mut replay = replay_exports(
        &snapshot::export_artifact(&initial).unwrap(),
        &snapshot::export_artifact(&final_run).unwrap(),
    )
    .unwrap();
    assert_eq!(
        replay.report.initial_digest,
        initial.snapshot.as_ref().unwrap().digest.to_string()
    );
    assert_eq!(
        replay.report.actual_final_digest,
        final_run.snapshot.as_ref().unwrap().digest.to_string()
    );
    assert_eq!(replay.report.initial_applied_actions, 1);
    assert_eq!(replay.report.final_applied_actions, 4);
    assert_eq!(replay.report.frozen_crew, 1);
    assert_held(replay.simulation.app_mut(), &station, &rating, &system);
}

#[test]
fn recorded_gm_exports_derive_outcomes_instead_of_trusting_the_final_record() {
    let _guard = RECORDING_TEST.lock().unwrap();
    let (initial, mut final_run, _, _, _) = recorded_pair();
    let saved = &mut final_run.snapshot.as_mut().unwrap().state.gm_actions;
    let mut altered = serde_json::to_value(&*saved).unwrap();
    altered["applied_results"][1]["outcome"] = serde_json::to_value(GmActionOutcome::NoOp).unwrap();
    *saved = serde_json::from_value(altered).unwrap();
    let error = replay_exports(
        &snapshot::export_artifact(&initial).unwrap(),
        &snapshot::export_artifact(&final_run).unwrap(),
    )
    .err()
    .unwrap();
    assert_eq!(error.stage, "results", "{error}");
}

#[test]
fn recorded_gm_exports_reach_the_ordinary_game_over_autosave_boundary() {
    let _guard = RECORDING_TEST.lock().unwrap();
    let (mut app, station, rating, system) = source();
    assert_held(&mut app, &station, &rating, &system);
    let initial = export(&mut app);
    let mut query = app
        .world_mut()
        .query_filtered::<&EntityUuid, With<project_phoenix::server_app::LocalShip>>();
    let target = query.single(app.world()).unwrap().0.clone();
    request(
        &mut app,
        GmAction::ApplyDirectEffect {
            target,
            scope: project_phoenix::gm_effect::GmDirectEffectScope::Entity,
            effect: project_phoenix::gm_effect::GmDirectEffectKind::Damage,
            amount_milli_hp: u32::MAX,
        },
    );
    let final_run = captured(
        &mut app,
        project_phoenix::save_slots::CaptureReason::GameOver,
    )
    .expect("a lethal ordinary GM effect must produce the real GameOver autosave");
    assert_eq!(
        final_run.snapshot.as_ref().unwrap().state.phase,
        Some(GamePhase::GameOver)
    );
    let mut replay = replay_exports(
        &snapshot::export_artifact(&initial).unwrap(),
        &snapshot::export_artifact(&final_run).unwrap(),
    )
    .unwrap();
    assert_eq!(
        replay
            .simulation
            .app_mut()
            .world()
            .resource::<State<GamePhase>>()
            .get(),
        &GamePhase::GameOver
    );
    assert_eq!(
        replay.report.actual_final_digest,
        final_run.ledger.final_digest.to_string()
    );
}

#[test]
fn recorded_gm_exports_rederive_station_consumer_feedback_and_preserve_human_ownership() {
    let _guard = RECORDING_TEST.lock().unwrap();
    let (mut app, station, rating, system) = source();
    assert_held(&mut app, &station, &rating, &system);
    let initial = export(&mut app);
    let mut query = app.world_mut().query_filtered::<(
        Entity,
        &EntityUuid,
        &project_phoenix::ship::state::ShipRedAlert,
    ), With<project_phoenix::server_app::LocalShip>>();
    let (entity, uuid, alert) = query.single(app.world()).unwrap();
    let ship = project_phoenix::command_admission::log::ShipKey(uuid.0.clone());
    let selected_alert = !alert.0;
    let member = |active| GmAction::SetStationPuppet {
        ship: ship.clone(),
        station: station.clone(),
        active,
    };
    let command = || GmAction::IssueStationCommand {
        ship: ship.clone(),
        station: station.clone(),
        target: system.clone(),
        payload: project_phoenix::core::codec::canonical_system_command(
            &SystemControlPayload::SetRedAlert {
                active: selected_alert,
            },
        )
        .unwrap(),
    };
    request(&mut app, member(true));
    request(&mut app, command());
    assert_eq!(
        app.world()
            .get::<project_phoenix::ship::state::ShipRedAlert>(entity)
            .unwrap()
            .0,
        selected_alert
    );
    // A second correlation still receives the ordinary consumer's Applied
    // feedback for an idempotent assignment; it is not a journal retry.
    request(&mut app, command());
    request(&mut app, member(false));
    assert_held(&mut app, &station, &rating, &system);
    let final_run = export(&mut app);
    let journal = &final_run.snapshot.as_ref().unwrap().state.gm_actions;
    assert_eq!(
        journal
            .applied_results()
            .iter()
            .map(|result| result.outcome)
            .collect::<Vec<_>>(),
        [
            GmActionOutcome::Applied,
            GmActionOutcome::Applied,
            GmActionOutcome::Applied,
            GmActionOutcome::Applied
        ]
    );
    assert!(app
        .world()
        .resource::<project_phoenix::command_admission::CommandLog>()
        .is_empty());
    assert!(app
        .world()
        .resource::<project_phoenix::gm_puppet::PendingGmStationCommands>()
        .entries()
        .is_empty());
    let mut replay = replay_exports(
        &snapshot::export_artifact(&initial).unwrap(),
        &snapshot::export_artifact(&final_run).unwrap(),
    )
    .unwrap();
    assert_held(replay.simulation.app_mut(), &station, &rating, &system);
    let mut query = replay.simulation.app_mut().world_mut().query_filtered::<&project_phoenix::ship::state::ShipRedAlert, With<project_phoenix::server_app::LocalShip>>();
    assert_eq!(
        query.single(replay.simulation.app_mut().world()).unwrap().0,
        selected_alert
    );
    assert_eq!(replay.simulation.recorded_gm_actions(), *journal);
    assert!(replay.simulation.recorded_log().is_empty());
}

#[test]
fn recorded_gm_exports_refuse_changed_crew_recovery_and_false_origin_digest() {
    let _guard = RECORDING_TEST.lock().unwrap();
    let (initial, final_run, _, _, _) = recorded_pair();
    let first = snapshot::export_artifact(&initial).unwrap();
    let mut changed = final_run.clone();
    changed
        .snapshot
        .as_mut()
        .unwrap()
        .state
        .boot_identity
        .as_mut()
        .unwrap()
        .fleet
        .depart_slot(HostSlot(1));
    assert_eq!(
        replay_exports(&first, &snapshot::export_artifact(&changed).unwrap())
            .err()
            .unwrap()
            .stage,
        "scope"
    );
    let mut changed = final_run.clone();
    let tick = changed.snapshot.as_ref().unwrap().tick;
    changed
        .snapshot
        .as_mut()
        .unwrap()
        .state
        .gm_actions
        .record_slot_recovery(HostSlot(2), tick)
        .unwrap();
    assert_eq!(
        replay_exports(&first, &snapshot::export_artifact(&changed).unwrap())
            .err()
            .unwrap()
            .stage,
        "scope"
    );
    let mut changed = initial.clone();
    changed.snapshot.as_mut().unwrap().digest ^= 1;
    changed.ledger.final_digest ^= 1;
    assert_eq!(
        replay_exports(
            &snapshot::export_artifact(&changed).unwrap(),
            &snapshot::export_artifact(&final_run).unwrap()
        )
        .err()
        .unwrap()
        .stage,
        "initial digest"
    );
}
