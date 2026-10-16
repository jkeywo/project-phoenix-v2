//! Real-owner proofs. The incident and measured return are explicit fixture
//! preconditions; this is not an encounter, transport or Human-device playtest.
use super::*;
use crate::core::messages::{
    AdmittedCommand, AdmittedCommands, SystemBlackboard, SystemControlPayload,
};
use crate::science::broad::{BroadMeasurement, BroadProfile, BroadTracks};
use crate::security::server::{
    handle_security_commands, publish_security_blackboard, tick_security_teams,
};
use crate::ship::system_registry::{security_system_id, sensor_radar_system_id};
use crate::sim_tick::SimTick;
use bevy::ecs::system::RunSystemOnce;

fn fixture() -> (App, Entity, Entity, String) {
    let operator = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    let target = crate::entities::include_resolve::load_entity_config(
        "assets/entities/acquirable_harrow.toml",
    )
    .unwrap();
    let mut app = App::new();
    // Match the ordinary snapshot machinery schema without inserting equipment
    // or progress. Sparse owner apps must not silently omit their publications.
    app.world_mut()
        .register_component::<crate::console::weapons::beam::ActiveBeam>();
    app.world_mut()
        .register_component::<crate::console::weapons::beam::PhaserCooldown>();
    app.world_mut()
        .register_component::<crate::console::weapons::TorpedoSystemResource>();
    app.world_mut()
        .register_component::<crate::entities::spawner::EntityShipArcHull>();
    app.world_mut()
        .register_component::<crate::console::repair::server::ShipRepairTeams>();
    app.world_mut()
        .register_component::<crate::console::repair::server::RepairRequestQueue>();
    app.world_mut()
        .register_component::<crate::ship_plugin::RepairHumanAlerted>();
    app.world_mut()
        .register_component::<crate::server_app::ShipSystemBlackboards>();
    app.world_mut()
        .register_component::<crate::ai::server::ObjectiveCursors>();
    app.world_mut()
        .register_component::<crate::console::weapons::blaster::BlasterSystemResource>();
    app.world_mut()
        .register_component::<crate::ship::shields::ShipShields>();
    app.init_resource::<crate::world::server::WorldContentRuntime>();
    app.init_resource::<crate::effect_queue::EffectQueue<crate::core::task_lifecycle::TaskLifecycleRequest>>();
    app.insert_resource(SimTick(0));
    app.insert_resource(Time::<Fixed>::from_hz(60.0));
    // The same read-model inputs used by ordinary post-restore publishers.
    // No scene contacts, measurements, work or grants are inserted here.
    app.insert_resource(crate::lobby::server::ShipClientConfigResource(
        crate::lobby::server::project_ship_client_config(&operator),
    ));
    app.insert_resource(crate::lobby::WorldResource(
        crate::core::messages::WorldData::default(),
    ));
    let mut time = Time::<()>::default();
    time.advance_by(std::time::Duration::from_secs_f64(1.0 / 60.0));
    app.insert_resource(time);
    let mut config = crate::world::config::parse_world(include_str!(
        "../../../../tests/fixtures/worlds/gm_npc_doctrine.toml"
    ))
    .unwrap();
    let profile = crate::gm::operations::npc::NpcDoctrinePaletteEntry {
        id: "acquired-hold".into(),
        label: "Hold acquired vessel".into(),
        targets: vec!["prize".into()],
        origin_layer: None,
        doctrine: vec![crate::entities::config::DoctrineObjective {
            id: "acquired-hold".into(),
            directive_kind: Some("None".into()),
            base_priority: 100.0,
            ..Default::default()
        }],
    };
    config.gm_npc_doctrine_palette = vec![profile.clone()];
    app.world_mut()
        .resource_mut::<crate::world::server::WorldContentRuntime>()
        .gm_npc_doctrine_palette = vec![profile];
    app.insert_resource(config);
    let mut factions = crate::ai::faction::FactionRegistry::new();
    for source in [
        include_str!("../../../../assets/factions/alliance.toml"),
        include_str!("../../../../assets/factions/harrow.toml"),
    ] {
        factions.insert(crate::ai::faction::parse_faction_config(source).unwrap());
    }
    app.insert_resource(crate::entities::config_cache::FactionRegistryResource(
        factions,
    ));
    let a = crate::entities::spawner::spawn_entity(
        &mut app.world_mut().commands(),
        &operator,
        Vec3::ZERO,
        "operator".into(),
        None,
    );
    let b = crate::entities::spawner::spawn_entity(
        &mut app.world_mut().commands(),
        &target,
        Vec3::new(80., 0., 0.),
        "prize".into(),
        None,
    );
    app.world_mut().flush();
    // Explicit reversible physical incident: wreck compartment structure, not
    // catastrophic destruction. No acquisition code edits HP or energy.
    for room in app
        .world_mut()
        .get_mut::<ShipDamage>(b)
        .unwrap()
        .state
        .compartments
        .values_mut()
    {
        room.hp = 0.0;
        room.disabled = true;
    }
    let ship = app.world().get::<ShipConfigComponent>(b).unwrap();
    assert!(app.world().get::<ShipDamage>(b).unwrap().disabled(
        &ship.0,
        app.world().get::<crate::ship::power::ShipPowerSystem>(b)
    ));
    let owner = sensor_radar_system_id();
    let profiles = &app.world().get::<BroadProfile>(a).unwrap().0;
    let profile = profiles[&owner].profile.clone();
    let before = app.world().get::<BroadTracks>(a).unwrap().0.clone();
    assert!(profiles.keys().eq(before.keys()));
    let dt = app
        .world()
        .resource::<Time<Fixed>>()
        .timestep()
        .as_secs_f32();
    let measurement = BroadMeasurement {
        elevation: None,
        source: "prize".into(),
        selectable: true,
        observation: None,
        position: [80., 0.],
        support: 1.0,
        identity: None,
        classification: None,
        assessment: Some(phoenix_model::sensing_tracks::TrackAssessment {
            protection: None,
            alert: None,
            weapons_restrained: None,
            disabled: Some(true),
        }),
    };
    // Explicit measured-return precondition for this Security-owner fixture,
    // not paid sensor/encounter evidence. Earn assessment through the accepted
    // instrument's real cadence instead of inventing a public assessed field.
    let budget = (f64::from(profile.assessment_secs) / f64::from(dt)).ceil() as u64
        + (f64::from(profile.sample_interval_secs) / f64::from(dt)).ceil() as u64
        + 1;
    let mut assessed = None;
    for tick in 0..budget {
        let mut all = app.world_mut().get_mut::<BroadTracks>(a).unwrap();
        let tracks = all
            .0
            .get_mut(&owner)
            .expect("ordinary spawned sensor owner");
        tracks
            .advance(&profile, tick, dt, 1.0, std::slice::from_ref(&measurement))
            .unwrap();
        if tracks
            .projection(&profile, tick, dt)
            .unwrap()
            .iter()
            .any(|track| {
                track
                    .assessment
                    .as_ref()
                    .is_some_and(|assessment| assessment.disabled == Some(true))
            })
        {
            assessed = Some(tick);
            break;
        }
    }
    let sampled = assessed.expect("accepted instrument earns the measured disabled assessment");
    app.world_mut().resource_mut::<SimTick>().0 = sampled + 1;
    let measured = &app.world().get::<BroadTracks>(a).unwrap().0;
    assert!(measured.keys().eq(before.keys()));
    for (instrument, original) in &before {
        if instrument != &owner {
            assert_eq!(
                &measured[instrument], original,
                "other accepted Broad owners stay intact"
            );
        }
    }
    fixture_publications(app.world_mut());
    let boards = &app
        .world()
        .get::<crate::server_app::ShipSystemBlackboards>(a)
        .unwrap()
        .0;
    let Some(SystemBlackboard::SensorRadar(board)) = boards.get(&owner) else {
        panic!("ordinary sensor publisher supplies the measured return");
    };
    let contact = board
        .tracks
        .as_ref()
        .unwrap()
        .iter()
        .find(|track| {
            track
                .assessment
                .as_ref()
                .is_some_and(|assessment| assessment.disabled == Some(true))
        })
        .expect("canonical assessed return")
        .contact_id
        .clone();
    assert_eq!(
        acquisition_alias(
            &operator.ship_config.unwrap(),
            app.world()
                .get::<crate::server_app::ShipSystemBlackboards>(a)
                .unwrap(),
            app.world().get::<BroadTracks>(a).unwrap(),
            "prize"
        ),
        Some(contact.clone())
    );
    app.add_systems(
        Update,
        (
            handle_security_commands,
            tick_security_teams,
            commit_ready_acquisitions,
            publish_security_blackboard,
            fixture_publications,
            clear,
        )
            .chain(),
    );
    (app, a, b, contact)
}
/// The same ordinary derived-capability and read-model rebuilds as restore;
/// no sensing or machinery work runs here. Use the real current frontier.
fn fixture_publications(world: &mut World) {
    world
        .run_system_once(crate::modifiers::coordination::translate_power_modifiers)
        .unwrap();
    world
        .run_system_once(crate::modifiers::coordination::apply_radar_damage_modifiers)
        .unwrap();
    world
        .run_system_once(crate::modifiers::coordination::translate_impulse_modifiers)
        .unwrap();
    world
        .run_system_once(crate::science::server::publish_scan_blackboard)
        .unwrap();
    world
        .run_system_once(crate::science::passive::publish)
        .unwrap();
    world
        .run_system_once(crate::ship::sensors::publish_sensor_radar_blackboard)
        .unwrap();
    world
        .run_system_once(crate::console::helm::server::publish_helm_blackboard)
        .unwrap();
    world
        .run_system_once(crate::console::navigation::server::publish_navigation_blackboard)
        .unwrap();
    world
        .run_system_once(crate::console::weapons::blackboard::publish_tactical_radar_blackboard)
        .unwrap();
    world
        .run_system_once(crate::console::command::server::publish_command_blackboard)
        .unwrap();
}
fn clear(mut commands: Query<&mut AdmittedCommands>) {
    for mut commands in &mut commands {
        commands.0.clear();
    }
}
fn step(app: &mut App) {
    app.update();
    app.world_mut().resource_mut::<SimTick>().0 += 1;
}
fn dispatch(app: &mut App, actor: Entity, contact: &str, team: u8) {
    app.world_mut()
        .get_mut::<AdmittedCommands>(actor)
        .unwrap()
        .0
        .push(AdmittedCommand {
            target: security_system_id(),
            payload: SystemControlPayload::DispatchSecurityTeam {
                team_idx: team,
                target: contact.into(),
                action: "board".into(),
            },
            response_token: None,
            feedback_correlation: None,
        });
}
fn partial(app: &mut App, actor: Entity) {
    for _ in 0..900 {
        step(app);
        let jobs = &app.world().get::<ShipSecurityTeams>(actor).unwrap().teams;
        if jobs[0].state == SecurityTeamState::Working
            && jobs[0].elapsed > 0.0
            && jobs[0].elapsed < jobs[0].phase_duration
        {
            return;
        }
    }
    panic!("same paid Security job did not reach a partial work checkpoint");
}
#[test]
fn paid_board_acquires_same_vessel_then_returns_same_team_and_json_continues() {
    let (mut live, actor, target, contact) = fixture();
    let damage = live
        .world()
        .get::<ShipDamage>(target)
        .unwrap()
        .state
        .clone();
    let power = live
        .world()
        .get::<crate::ship::power::ShipPowerSystem>(target)
        .unwrap()
        .0
        .clone();
    let initial = live
        .world()
        .get::<crate::ship::power::ShipPowerSystem>(actor)
        .unwrap()
        .0
        .network()
        .consumers[&security_system_id()]
        .energy;
    dispatch(&mut live, actor, &contact, 0);
    partial(&mut live, actor);
    assert!(live.world().get::<VesselOperationalOwner>(target).is_none());
    assert!(
        live.world()
            .get::<crate::ship::power::ShipPowerSystem>(actor)
            .unwrap()
            .0
            .network()
            .consumers[&security_system_id()]
            .energy
            < initial
    );
    let saved = crate::snapshot::capture(live.world());
    let source = saved
        .entities
        .iter()
        .find(|row| row.uuid == "operator")
        .unwrap();
    let profiles = &live.world().get::<BroadProfile>(actor).unwrap().0;
    let saved_tracks = source
        .broad
        .as_ref()
        .expect("all real Broad owners are captured");
    assert!(
        profiles.keys().eq(saved_tracks.keys()),
        "measuring one sensor must retain every accepted instrument owner"
    );
    let saved_board = source
        .blackboards
        .iter()
        .find(|(owner, _)| owner == &sensor_radar_system_id().0)
        .map(|(_, board)| board)
        .expect("saved actual sensor publication");
    let SystemBlackboard::SensorRadar(board) = saved_board else {
        panic!("actual saved sensor view");
    };
    assert!(board
        .tracks
        .as_ref()
        .unwrap()
        .iter()
        .any(|track| track.contact_id == contact));
    let dt = live
        .world()
        .resource::<Time<Fixed>>()
        .timestep()
        .as_secs_f32();
    let instrument = sensor_radar_system_id();
    assert!(
        [Some(saved.tick), saved.tick.checked_sub(1)]
            .into_iter()
            .flatten()
            .any(|tick| crate::science::sharing::projection(
                "operator",
                &saved_tracks[&instrument],
                &profiles[&instrument].profile,
                tick,
                dt
            )
            .is_ok_and(|canonical| board.tracks.as_deref() == Some(canonical.as_slice()))),
        "saved held view must be the actual source projection at its publication frontier"
    );
    let job = source.security.as_ref().unwrap().jobs.teams[0]
        .as_ref()
        .unwrap();
    assert!(
        job.activity.team.elapsed > 0.0
            && job.activity.team.elapsed < job.activity.team.phase_duration
    );
    assert_eq!(
        job.activity.acquisition_order.as_ref().unwrap().target,
        "prize"
    );
    let saved: crate::snapshot::PhoenixSnapshot =
        serde_json::from_slice(&serde_json::to_vec(&saved).unwrap()).unwrap();
    let (mut resumed, _, resumed_target, _) = fixture();
    let report = crate::snapshot::restore(resumed.world_mut(), &saved);
    assert!(report.is_complete(), "{:?}", report.gaps);
    for _ in 0..1200 {
        step(&mut live);
        step(&mut resumed);
        assert_eq!(
            crate::sim_digest::world_digest(live.world()),
            crate::sim_digest::world_digest(resumed.world())
        );
        assert_eq!(
            crate::snapshot::capture(live.world()),
            crate::snapshot::capture(resumed.world())
        );
    }
    let owner = &live
        .world()
        .get::<VesselOperationalOwner>(target)
        .unwrap()
        .0;
    assert_eq!(owner.controller, "operator");
    assert_eq!(
        live.world().get::<FactionComponent>(target).unwrap().0,
        owner.faction
    );
    assert_eq!(
        live.world().get::<ShipDamage>(target).unwrap().state,
        damage
    );
    assert_eq!(
        live.world()
            .get::<crate::ship::power::ShipPowerSystem>(target)
            .unwrap()
            .0,
        power
    );
    assert_eq!(
        resumed
            .world()
            .get::<VesselOperationalOwner>(resumed_target)
            .unwrap()
            .0,
        *owner
    );
    assert_eq!(
        live.world()
            .get::<crate::gm::operations::npc::NpcDoctrineState>(target)
            .unwrap()
            .0
            .as_ref()
            .unwrap()
            .id,
        "acquired-hold"
    );
    let jobs = &live.world().get::<ShipSecurityTeams>(actor).unwrap().teams;
    assert!(jobs[0].is_available());
    let job = jobs.jobs.teams[0].as_ref().unwrap();
    assert_eq!(job.outcome, Some(phoenix_model::job::JobOutcome::Completed));
    assert_eq!(job.activity.acquisition.as_ref(), Some(owner));
}

#[test]
fn guessed_identity_and_lost_observation_cannot_dispatch_acquisition() {
    let (mut app, actor, target, contact) = fixture();
    dispatch(&mut app, actor, "prize", 0);
    step(&mut app);
    assert!(app.world().get::<ShipSecurityTeams>(actor).unwrap().teams[0].is_available());
    app.world_mut()
        .get_mut::<crate::server_app::ShipSystemBlackboards>(actor)
        .unwrap()
        .0
        .remove(&sensor_radar_system_id());
    dispatch(&mut app, actor, &contact, 0);
    step(&mut app);
    assert!(app.world().get::<VesselOperationalOwner>(target).is_none());
    assert!(app.world().get::<ShipSecurityTeams>(actor).unwrap().teams[0].is_available());
}

#[test]
fn competing_same_tick_work_has_one_owner_and_a_separate_failed_return() {
    let (mut app, actor, target, contact) = fixture();
    dispatch(&mut app, actor, &contact, 0);
    dispatch(&mut app, actor, &contact, 1);
    for _ in 0..900 {
        step(&mut app);
        if app.world().get::<VesselOperationalOwner>(target).is_some() {
            break;
        }
    }
    let acquired = app
        .world()
        .get::<VesselOperationalOwner>(target)
        .unwrap()
        .0
        .clone();
    let VesselAcquisitionCause::SecurityBoard { team, .. } = acquired.cause;
    assert_eq!(team, 0);
    let jobs = &app.world().get::<ShipSecurityTeams>(actor).unwrap().teams;
    assert_eq!(jobs[0].state, SecurityTeamState::Withdrawing);
    assert_eq!(jobs[1].state, SecurityTeamState::Withdrawing);
    assert_eq!(
        jobs.jobs.teams[1]
            .as_ref()
            .unwrap()
            .activity
            .returning_outcome,
        Some(phoenix_model::job::JobOutcome::Failed)
    );
    assert_eq!(
        jobs.jobs.teams[0]
            .as_ref()
            .unwrap()
            .activity
            .acquisition
            .as_ref(),
        Some(&acquired)
    );
}

#[test]
fn original_eligibility_is_not_rechecked_later_and_malformed_candidates_write_nothing() {
    let (mut app, actor, target, contact) = fixture();
    dispatch(&mut app, actor, &contact, 0);
    for _ in 0..900 {
        step(&mut app);
        if app.world().get::<VesselOperationalOwner>(target).is_some() {
            break;
        }
    }
    let saved = crate::snapshot::capture(app.world());
    assert!(saved
        .entities
        .iter()
        .find(|row| row.uuid == "prize")
        .unwrap()
        .operational_owner
        .is_some());
    // A later control change is not evidence that the old acquisition was illegal.
    app.world_mut()
        .get_mut::<ShipSystemControlSources>(target)
        .unwrap()
        .0
        .set(
            crate::ship::system_registry::security_system_id(),
            crate::ship::control_source::ControlSource::Human,
        );
    let valid = crate::snapshot::restore(app.world_mut(), &saved);
    assert!(valid.is_complete(), "{:?}", valid.gaps);
    for fault in 0..5 {
        let mut bad = saved.clone();
        let row = bad
            .entities
            .iter_mut()
            .find(|row| row.uuid == "prize")
            .unwrap();
        let owner = row.operational_owner.as_mut().unwrap();
        let VesselAcquisitionCause::SecurityBoard {
            scope,
            work_completed_tick,
            ..
        } = &mut owner.cause;
        match fault {
            0 => scope.job.0 += 1000,
            1 => owner.doctrine = "unknown".into(),
            2 => *work_completed_tick += 1,
            3 => owner.faction = uuid::Uuid::from_u128(999),
            _ => owner.controller = "other".into(),
        }
        let before = crate::snapshot::capture(app.world());
        let digest = crate::sim_digest::world_digest(app.world());
        let report = crate::snapshot::restore(app.world_mut(), &bad);
        assert!(!report.is_complete());
        assert_eq!(crate::snapshot::capture(app.world()), before);
        assert_eq!(crate::sim_digest::world_digest(app.world()), digest);
    }
    // Removal of the original operator never silently transfers or deletes a prize.
    app.world_mut().despawn(actor);
    let orphaned = crate::snapshot::capture(app.world());
    let (mut fresh, _, fresh_target, _) = fixture();
    let restored = crate::snapshot::restore(fresh.world_mut(), &orphaned);
    assert!(restored.is_complete(), "{:?}", restored.gaps);
    assert_eq!(
        fresh
            .world()
            .get::<VesselOperationalOwner>(fresh_target)
            .unwrap()
            .0,
        app.world().get::<VesselOperationalOwner>(target).unwrap().0
    );
}

#[test]
fn stale_current_human_authority_refuses_work_without_a_successful_doctrine_or_owner() {
    let (mut app, actor, target, contact) = fixture();
    dispatch(&mut app, actor, &contact, 0);
    partial(&mut app, actor);
    let doctrine = app
        .world()
        .get::<crate::entities::spawner::BehaviourSection>(target)
        .unwrap()
        .0
        .doctrine
        .clone();
    app.world_mut()
        .get_mut::<ShipSystemControlSources>(target)
        .unwrap()
        .0
        .set(
            crate::ship::system_registry::security_system_id(),
            crate::ship::control_source::ControlSource::Human,
        );
    for _ in 0..300 {
        step(&mut app);
    }
    assert!(app.world().get::<VesselOperationalOwner>(target).is_none());
    assert_eq!(
        app.world()
            .get::<crate::entities::spawner::BehaviourSection>(target)
            .unwrap()
            .0
            .doctrine,
        doctrine
    );
    assert_eq!(
        app.world()
            .get::<ShipSecurityTeams>(actor)
            .unwrap()
            .teams
            .jobs
            .teams[0]
            .as_ref()
            .unwrap()
            .activity
            .returning_outcome,
        Some(phoenix_model::job::JobOutcome::Failed)
    );
}

#[test]
fn separate_actors_compete_in_canonical_actor_order_without_second_credit() {
    let (mut app, actor, target, contact) = fixture();
    let source = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    let second = crate::entities::spawner::spawn_entity(
        &mut app.world_mut().commands(),
        &source,
        Vec3::ZERO,
        "z-other".into(),
        None,
    );
    app.world_mut().flush();
    let tracks = app.world().get::<BroadTracks>(actor).unwrap().clone();
    let boards = app
        .world()
        .get::<crate::server_app::ShipSystemBlackboards>(actor)
        .unwrap()
        .clone();
    app.world_mut().entity_mut(second).insert((tracks, boards));
    dispatch(&mut app, second, &contact, 0);
    dispatch(&mut app, actor, &contact, 0);
    for _ in 0..900 {
        step(&mut app);
        if app.world().get::<VesselOperationalOwner>(target).is_some() {
            break;
        }
    }
    assert_eq!(
        app.world()
            .get::<VesselOperationalOwner>(target)
            .unwrap()
            .0
            .controller,
        "operator"
    );
    let losing = &app
        .world()
        .get::<ShipSecurityTeams>(second)
        .unwrap()
        .teams
        .jobs
        .teams[0]
        .as_ref()
        .unwrap()
        .activity;
    assert!(losing.acquisition.is_none());
    assert_eq!(
        losing.returning_outcome,
        Some(phoenix_model::job::JobOutcome::Failed)
    );
}

#[test]
fn ordinary_security_ai_selects_the_same_permitted_capture_and_returns_normally() {
    let (mut app, actor, target, _) = fixture();
    app.insert_resource(crate::lobby::Sessions(
        crate::lobby::session::SessionManager::new(),
    ));
    app.add_systems(
        Update,
        crate::security::server::operate_security_ai.before(handle_security_commands),
    );
    for _ in 0..1500 {
        step(&mut app);
        if app.world().get::<VesselOperationalOwner>(target).is_some()
            && app.world().get::<ShipSecurityTeams>(actor).unwrap().teams[0].is_available()
        {
            break;
        }
    }
    let owner = app.world().get::<VesselOperationalOwner>(target).unwrap();
    assert_eq!(owner.0.controller, "operator");
    let jobs = &app.world().get::<ShipSecurityTeams>(actor).unwrap().teams;
    assert!(jobs[0].is_available());
    assert_eq!(
        jobs.jobs.next_id, 2,
        "the acquired target cannot be re-dispatched"
    );
}

#[test]
fn a_foreign_person_in_actual_custody_is_not_an_empty_vessel() {
    use phoenix_model::personnel::OfficerLocation;
    let content = crate::entities::include_resolve::load_entity_config(
        "assets/entities/personnel_cruiser.toml",
    )
    .unwrap();
    let profile = phoenix_sim_gameplay::personnel::compile_entity(&content)
        .unwrap()
        .unwrap();
    let mut roster = phoenix_sim_gameplay::personnel::PersonnelRoster::default();
    roster.accept("origin", profile, None, 0).unwrap();
    assert!(!has_personnel(&roster, "prize"));
    let person = roster
        .origins
        .get_mut("origin")
        .unwrap()
        .state
        .officers
        .values_mut()
        .next()
        .unwrap();
    person.location = OfficerLocation::AboardCraft {
        craft: "prize".into(),
        berth: "bay".into(),
    };
    assert!(has_personnel(&roster, "prize"));
    roster
        .origins
        .get_mut("origin")
        .unwrap()
        .state
        .officers
        .values_mut()
        .next()
        .unwrap()
        .location = OfficerLocation::Lost {
        actor: "prize".into(),
    };
    assert!(!has_personnel(&roster, "prize"));
    assert!(
        has_personnel(&roster, "origin"),
        "an originally populated crew profile stays unsupported after loss"
    );
}
