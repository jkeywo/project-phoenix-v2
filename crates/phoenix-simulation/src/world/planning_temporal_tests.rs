//! Actual resolved equipment/crew metadata projection. Physical completion and
//! whole-host workload calibration remain separate consumer proofs.
use super::*;
use crate::lobby::crew_replication::ShipCrewPresence;
use phoenix_sim_world::world::planning::{temporal::*, DomainLibrary, Task};

fn temporal_fixture() -> (App, Entity) {
    let (mut app, actor) = setup(2);
    let mut planning = app
        .world()
        .resource::<WorldConfig>()
        .planning
        .clone()
        .unwrap();
    planning.temporal = Some(TemporalPolicy {
        profiles: vec![
            DifficultyProfile {
                min_operators: 0,
                deadline_secs: 30,
            },
            DifficultyProfile {
                min_operators: 1,
                deadline_secs: 20,
            },
        ],
    });
    let (compiled, _) =
        phoenix_sim_world::world::planning::CompiledPlanning::load(planning.clone(), |path| {
            let mut library: DomainLibrary =
                toml::from_str(&crate::content_fs::read_to_string(path).ok()?).ok()?;
            for task in &mut library.tasks {
                if let Task::Primitive { forecast, .. } = task {
                    *forecast = Some(ActivityForecast {
                        min_secs: 1,
                        max_secs: 2,
                        recovery_secs: 1,
                        phases: vec![AttentionPhase {
                            kind: AttentionKind::Active,
                            offset_secs: 0,
                            duration_secs: 1,
                            attention_permille: 1000,
                        }],
                    });
                }
            }
            toml::to_string(&library).ok()
        })
        .unwrap();
    let mut base = app.world_mut().resource_mut::<WorldConfig>();
    base.planning = Some(planning);
    base.compiled_planning = Some(compiled);
    app.world_mut()
        .get_mut::<ShipSystemControlSources>(actor)
        .unwrap()
        .0
        .set(
            SystemId("logistics".into()),
            crate::ship::control_source::ControlSource::Ai,
        );
    let mut fixed = Time::<Fixed>::default();
    fixed.set_timestep(std::time::Duration::from_millis(100));
    app.insert_resource(fixed);
    (app, actor)
}
fn with_sources(app: &mut App, actor: Entity) {
    let base = app.world().resource::<WorldConfig>().clone();
    let tick = app.world().resource::<crate::sim_tick::SimTick>().0;
    let config = app
        .world()
        .get::<ShipConfigComponent>(actor)
        .unwrap()
        .clone();
    let sources = app
        .world()
        .get::<ShipSystemControlSources>(actor)
        .unwrap()
        .clone();
    let ratings = app
        .world()
        .get::<crate::ship_plugin::ActiveStationRatings>(actor)
        .unwrap()
        .clone();
    let presence = app.world().get::<ShipCrewPresence>(actor).cloned();
    let damage = app
        .world()
        .get::<crate::ship::compartment_damage::ShipDamage>(actor)
        .unwrap()
        .clone();
    let teams = app.world().get::<ShipRepairTeams>(actor).unwrap();
    let team_owner = teams.0.job_state().owner.clone();
    let count = teams.0.slots().len();
    let logistics = app
        .world()
        .get::<crate::resources::logistics_server::CompiledShipLogistics>(actor)
        .unwrap()
        .0
        .clone();
    let forecast = presence.as_ref().map(|presence| ForecastSources {
        config: &config,
        sources: &sources,
        ratings: &ratings,
        presence,
        hosts: None,
        tick_nanos: 100_000_000,
    });
    let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
    advance_with_forecast(
        &mut runtime,
        &base,
        "actual-maintenance",
        None,
        tick,
        forecast.as_ref(),
        |request| match request {
            ActivityRequest::Repair { .. } => crate::world::activities::repair_binding_supported(
                request,
                &config.0,
                &damage,
                &team_owner,
                count,
            ),
            ActivityRequest::Communication { .. } | ActivityRequest::AcquireVessel { .. } => false,
            ActivityRequest::Delivery { .. } => {
                crate::world::activities::delivery_binding_supported(request, &logistics)
            }
        },
    )
    .unwrap();
}
#[test]
fn real_bootstrap_presence_is_required_and_default_or_gm_human_never_adds_an_operator() {
    let (mut app, actor) = temporal_fixture();
    app.world_mut()
        .entity_mut(actor)
        .remove::<ShipCrewPresence>();
    with_sources(&mut app, actor);
    assert!(
        app.world()
            .resource::<WorldContentRuntime>()
            .plan_choice
            .is_none(),
        "wait before choosing while actual bootstrap is incomplete"
    );
    app.world_mut()
        .entity_mut(actor)
        .insert(ShipCrewPresence::default());
    let engineering = StationId("engineering".into());
    app.world_mut()
        .get_mut::<crate::ship_plugin::ActiveStationRatings>(actor)
        .unwrap()
        .0
        .insert(engineering.clone(), "Std".into());
    for id in ["repair", "logistics"] {
        app.world_mut()
            .get_mut::<ShipSystemControlSources>(actor)
            .unwrap()
            .0
            .set(
                SystemId(id.into()),
                crate::ship::control_source::ControlSource::Human,
            );
    }
    let before = app
        .world()
        .get::<ShipRepairTeams>(actor)
        .unwrap()
        .0
        .save_state();
    with_sources(&mut app, actor);
    let runtime = app.world().resource::<WorldContentRuntime>();
    let choice = runtime.plan_choice.as_ref().unwrap();
    assert!(matches!(
        &choice.temporal.as_ref().unwrap().outcome,
        TemporalOutcome::Refused { .. }
    ));
    assert!(choice.released.is_empty());
    assert!(runtime.situations.instances.is_empty());
    assert!(choice
        .temporal
        .as_ref()
        .unwrap()
        .input
        .owners
        .values()
        .all(|owner| matches!(
            owner,
            OwnerInput::Unavailable {
                reason: UnavailableReason::Unoccupied
            }
        )));
    assert_eq!(
        app.world()
            .get::<ShipRepairTeams>(actor)
            .unwrap()
            .0
            .save_state(),
        before
    );
}
#[test]
fn later_actual_human_presence_counts_once_and_disconnect_never_recounts_original_history() {
    let (mut app, actor) = temporal_fixture();
    let station = StationId("engineering".into());
    app.world_mut()
        .get_mut::<ShipCrewPresence>(actor)
        .unwrap()
        .0
        .insert(station.clone());
    app.world_mut()
        .get_mut::<crate::ship_plugin::ActiveStationRatings>(actor)
        .unwrap()
        .0
        .insert(station.clone(), "Std".into());
    for id in ["repair", "logistics"] {
        app.world_mut()
            .get_mut::<ShipSystemControlSources>(actor)
            .unwrap()
            .0
            .set(
                SystemId(id.into()),
                crate::ship::control_source::ControlSource::Human,
            );
    }
    with_sources(&mut app, actor);
    let original = app
        .world()
        .resource::<WorldContentRuntime>()
        .plan_choice
        .clone()
        .unwrap();
    let temporal = original.temporal.as_ref().unwrap();
    assert_eq!(
        temporal.profile_index, 1,
        "two Systems on one real Station share one operator"
    );
    assert!(matches!(
        &temporal.outcome,
        TemporalOutcome::Committed { .. }
    ));
    assert!(temporal.input.owners.values().all(|value|matches!(value,OwnerInput::Human{station: actual,rating} if actual==&station && rating=="Std")));
    assert_eq!(
        original.released.len(),
        1,
        "actual dependency still waits for Repair achievement"
    );
    app.world_mut()
        .get_mut::<ShipCrewPresence>(actor)
        .unwrap()
        .0
        .clear();
    app.world_mut()
        .get_mut::<crate::ship_plugin::ActiveStationRatings>(actor)
        .unwrap()
        .0
        .insert(station, "Backfill".into());
    for id in ["repair", "logistics"] {
        app.world_mut()
            .get_mut::<ShipSystemControlSources>(actor)
            .unwrap()
            .0
            .set(
                SystemId(id.into()),
                crate::ship::control_source::ControlSource::Ai,
            );
    }
    for now in 21..381 {
        app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = now;
        with_sources(&mut app, actor);
        assert_eq!(
            app.world()
                .resource::<WorldContentRuntime>()
                .plan_choice
                .as_ref(),
            Some(&original)
        );
    }
    let config = &app.world().get::<ShipConfigComponent>(actor).unwrap().0;
    validate_forecast_input(&original.temporal.as_ref().unwrap().input, config).unwrap();
    let mut forged = original.temporal.unwrap().input;
    if let OwnerInput::Human { station, .. } =
        forged.owners.get_mut(&SystemId("repair".into())).unwrap()
    {
        station.0 = "foreign-seat".into();
    }
    assert!(validate_forecast_input(&forged, config).is_err());
}

#[test]
fn another_valid_station_cannot_forge_the_historical_owner_host() {
    let (app, actor) = temporal_fixture();
    let config = &app.world().get::<ShipConfigComponent>(actor).unwrap().0;
    let foreign = config
        .stations
        .iter()
        .find(|station| station.id.0 != "engineering")
        .expect("Cruiser has other real Stations");
    let rating = foreign
        .ratings
        .first()
        .map(|rating| rating.name.clone())
        .unwrap_or_else(|| "Std".into());
    let forged = ForecastInput {
        tick_nanos: 100_000_000,
        owners: std::collections::BTreeMap::from([(
            SystemId("repair".into()),
            OwnerInput::Human {
                station: foreign.id.clone(),
                rating,
            },
        )]),
    };
    assert!(validate_forecast_input(&forged, config).is_err());
}

#[test]
fn authored_temporal_world_uses_imported_estimates_and_read_only_workload_projection() {
    let (mut app, actor) = setup(2);
    // setup() deliberately delegates Repair for ordinary maintenance AI tests.
    // This case instead models the explicitly occupied Human engineer below.
    app.world_mut()
        .get_mut::<ShipSystemControlSources>(actor)
        .unwrap()
        .0
        .set(
            SystemId("repair".into()),
            crate::ship::control_source::ControlSource::Human,
        );
    let reader = crate::world::load::OverlayFsReader;
    let mut base = crate::world::load::load(crate::world::load::LoadRequest::new(
        "tests/fixtures/maintenance-temporal-world.toml",
        &reader,
        &crate::world::script::load::NoSiblingScripts,
        crate::world::load::LoadPolicy::Merge,
    ))
    .unwrap()
    .config;
    base.name_to_uuid
        .insert("maintenance-ship".into(), "actual-maintenance".into());
    app.insert_resource(base);
    app.world_mut()
        .get_mut::<ShipCrewPresence>(actor)
        .unwrap()
        .0
        .insert(StationId("engineering".into()));
    app.world_mut()
        .get_mut::<crate::ship_plugin::ActiveStationRatings>(actor)
        .unwrap()
        .0
        .insert(StationId("engineering".into()), "Std".into());
    app.world_mut()
        .get_mut::<ShipSystemControlSources>(actor)
        .unwrap()
        .0
        .set(
            SystemId("logistics".into()),
            crate::ship::control_source::ControlSource::Ai,
        );
    let teams = app
        .world()
        .get::<ShipRepairTeams>(actor)
        .unwrap()
        .0
        .save_state();
    with_sources(&mut app, actor);
    let runtime = app.world().resource::<WorldContentRuntime>();
    let original = serde_json::to_value(runtime.plan_choice.as_ref().unwrap()).unwrap();
    let payload = crate::debug::scenario::collect_scenario_state_with_plan(
        runtime,
        &crate::objectives::ObjectiveManager::default(),
        &crate::debug::scenario::TriggerFireRecorder::default(),
        Some(app.world().resource::<WorldConfig>()),
    );
    let view = payload.planning.unwrap();
    assert_eq!(
        view.choice
            .as_ref()
            .unwrap()
            .temporal
            .as_ref()
            .unwrap()
            .profile_index,
        1
    );
    assert!(!view.workload.is_empty());
    assert!(view
        .workload
        .iter()
        .all(|bucket| bucket.station.0 == "engineering"
            && bucket.attention_nanos.parse::<u128>().unwrap() > 0));
    assert_eq!(
        serde_json::to_value(runtime.plan_choice.as_ref().unwrap()).unwrap(),
        original
    );
    assert_eq!(
        app.world()
            .get::<ShipRepairTeams>(actor)
            .unwrap()
            .0
            .save_state(),
        teams
    );
}

#[test]
fn real_admitted_crew_arrival_reconsiders_only_the_unreleased_refusal() {
    let (mut app, actor) = temporal_fixture();
    // The shared fixture starts Repair on AI. Refusal here specifically needs
    // a Human-required owner without occupied provenance, not ordinary Backfill.
    app.world_mut()
        .get_mut::<ShipSystemControlSources>(actor)
        .unwrap()
        .0
        .set(
            SystemId("repair".into()),
            crate::ship::control_source::ControlSource::Human,
        );
    app.world_mut()
        .get_mut::<crate::ship_plugin::ActiveStationRatings>(actor)
        .unwrap()
        .0
        .insert(StationId("engineering".into()), "Std".into());
    with_sources(&mut app, actor);
    let original = app
        .world()
        .resource::<WorldContentRuntime>()
        .plan_choice
        .clone()
        .unwrap();
    assert!(original.effective_temporal().unwrap().refused());
    assert!(original.released.is_empty());
    app.world_mut()
        .entity_mut(actor)
        .insert(AdmittedCommands(vec![AdmittedCommand {
            target: SystemId(crate::ship::system_registry::ASSIGN_STATION_RATING_SYSTEM_ID.into()),
            payload: SystemControlPayload::AssignStationRating {
                station: StationId("engineering".into()),
                rating: "Std".into(),
                occupied: Some(true),
            },
            response_token: None,
            feedback_correlation: None,
        }]));
    app.world_mut()
        .run_system_once(crate::lobby::crew_replication::apply_assigned_station_rating)
        .unwrap();
    app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 += 1;
    with_sources(&mut app, actor);
    let runtime = app.world().resource::<WorldContentRuntime>();
    let revised = runtime.plan_choice.as_ref().unwrap();
    assert_eq!(revised.temporal, original.temporal);
    assert_eq!(revised.methods, original.methods);
    assert_eq!(revised.temporal_reconsiderations.len(), 1);
    assert!(!revised.effective_temporal().unwrap().refused());
    assert_eq!(
        revised.effective_temporal().unwrap().deadline_tick,
        original.temporal.as_ref().unwrap().deadline_tick
    );
    assert_eq!(revised.released.len(), 1);
}
