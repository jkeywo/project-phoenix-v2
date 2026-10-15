use super::*;
use crate::console::navigation::{NavigationWaypoint, WaypointMode};
use crate::core::messages::{AdmittedCommand, NavigationBlackboard, WaypointSnapshot};
use crate::ship::control_source::ControlSource;
use bevy::ecs::system::RunSystemOnce;

fn owner() -> SystemId {
    SystemId("local-jump".into())
}
fn fuel(app: &App, entity: Entity) -> i64 {
    app.world()
        .get::<InfrastructureCondition>(entity)
        .unwrap()
        .0
        .capacity("drive-transit-fuel")
        .unwrap()
}
fn energy(app: &App, entity: Entity) -> f32 {
    app.world()
        .get::<ShipPowerSystem>(entity)
        .unwrap()
        .0
        .network()
        .consumers[&owner()]
        .energy
}
fn fixture() -> (App, Entity) {
    let authored = crate::entities::include_resolve::load_entity_config(
        "assets/entities/local_jump_cruiser.toml",
    )
    .unwrap();
    fixture_with_config(authored)
}
fn fixture_with_config(authored: crate::entities::config::EntityConfig) -> (App, Entity) {
    let mut app = App::new();
    app.insert_resource(crate::sim_tick::SimTick(1));
    app.insert_resource(Time::<Fixed>::from_duration(
        std::time::Duration::from_secs(1),
    ));
    app.insert_resource(crate::world::config::WorldConfig::default());
    let entity = crate::entities::spawner::spawn_entity(
        &mut app.world_mut().commands(),
        &authored,
        Vec3::ZERO,
        "jump-ship".into(),
        None,
    );
    app.world_mut().flush();
    app.world_mut()
        .get_mut::<ShipSystemControlSources>(entity)
        .unwrap()
        .0
        .set(owner(), ControlSource::Human);
    app.world_mut()
        .get_mut::<ShipPhysics>(entity)
        .unwrap()
        .forward_speed = 7.0;
    set_course(&mut app, entity, 100.0, 30.0);
    (app, entity)
}
fn set_course(app: &mut App, entity: Entity, x: f32, z: f32) {
    let mut waypoint = app
        .world_mut()
        .get_mut::<NavigationWaypoint>(entity)
        .unwrap();
    waypoint.set(WaypointMode::Free { x, z });
    let generation = waypoint.generation();
    app.world_mut()
        .get_mut::<ShipSystemBlackboards>(entity)
        .unwrap()
        .0
        .insert(
            SystemId("navigation".into()),
            SystemBlackboard::Navigation(NavigationBlackboard {
                course_generation: generation,
                navigation_waypoint: Some(WaypointSnapshot {
                    x,
                    z,
                    source_uuid: None,
                }),
                ..Default::default()
            }),
        );
}
fn command(app: &mut App, entity: Entity, payload: Control) {
    command_to(app, entity, owner(), payload);
}
fn command_to(app: &mut App, entity: Entity, target: SystemId, payload: Control) {
    app.world_mut()
        .get_mut::<AdmittedCommands>(entity)
        .unwrap()
        .0
        .push(AdmittedCommand {
            target,
            payload,
            response_token: None,
            feedback_correlation: None,
        });
    app.world_mut().run_system_once(handle_commands).unwrap();
    app.world_mut()
        .get_mut::<AdmittedCommands>(entity)
        .unwrap()
        .0
        .clear();
}
fn step(app: &mut App) {
    app.world_mut().run_system_once(tick).unwrap();
    app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 += 1;
}

#[test]
fn actual_drive_prepares_and_commits_real_power_fuel_then_arrives_once_after_feed_loss() {
    let (mut app, entity) = fixture();
    assert_eq!(fuel(&app, entity), 6);
    let initial_energy = energy(&app, entity);
    command(
        &mut app,
        entity,
        Control::BeginTransitToWaypoint { request: None },
    );
    assert!(matches!(
        app.world().get::<ShipTransitDrives>(entity).unwrap().0[&owner()].phase,
        TransitPhase::Preparing { .. }
    ));
    step(&mut app);
    assert_eq!(fuel(&app, entity), 6);
    assert!(energy(&app, entity) < initial_energy);
    assert_eq!(app.world().get::<ShipPhysics>(entity).unwrap().x, 0.0);
    // Accepted intent is independent of subsequent permitted feed loss.
    app.world_mut()
        .get_mut::<ShipSystemBlackboards>(entity)
        .unwrap()
        .0
        .clear();
    step(&mut app);
    let phase = app.world().get::<ShipTransitDrives>(entity).unwrap().0[&owner()]
        .phase
        .clone();
    let TransitPhase::Committed { arrival, .. } = phase else {
        panic!("actual paid commitment required")
    };
    assert_eq!(fuel(&app, entity), 4);
    assert!((f64::from(initial_energy - energy(&app, entity)) - 5.0).abs() < 1e-5);
    assert_eq!(app.world().get::<ShipPhysics>(entity).unwrap().x, 0.0);
    // An identical logical tick cannot debit again or apply arrival early.
    app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 -= 1;
    let paid = energy(&app, entity);
    app.world_mut().run_system_once(tick).unwrap();
    assert_eq!(energy(&app, entity), paid);
    assert_eq!(fuel(&app, entity), 4);
    app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 += 1;
    app.world_mut()
        .get_mut::<ShipSystemControlSources>(entity)
        .unwrap()
        .0
        .set(owner(), ControlSource::Offline);
    step(&mut app);
    let physics = app.world().get::<ShipPhysics>(entity).unwrap();
    assert_eq!([physics.x, physics.y, physics.z], arrival.point);
    assert_eq!(
        [
            physics.forward_speed,
            physics.lateral_speed,
            physics.vertical_speed
        ],
        [0.0; 3]
    );
    let exact = *physics;
    for _ in 0..5 {
        step(&mut app);
    }
    assert_eq!(app.world().get::<ShipPhysics>(entity).unwrap(), &exact);
    assert_eq!(energy(&app, entity), paid);
    assert_eq!(fuel(&app, entity), 4);
}

#[test]
fn insufficient_commit_power_keeps_actual_stock_claims_unchanged_and_retries_frozen_order() {
    let (mut app, entity) = fixture();
    let inventory = app
        .world()
        .get::<InfrastructureCondition>(entity)
        .unwrap()
        .0
        .clone();
    let logistics = app
        .world()
        .get::<crate::resources::logistics_server::CompiledShipLogistics>(entity)
        .unwrap()
        .clone();
    let mut power = app
        .world()
        .get::<ShipPowerSystem>(entity)
        .unwrap()
        .0
        .clone();
    command(
        &mut app,
        entity,
        Control::BeginTransitToWaypoint { request: None },
    );
    let order = app.world().get::<ShipTransitDrives>(entity).unwrap().0[&owner()]
        .phase
        .order()
        .unwrap()
        .clone();
    let mut paid = 0;
    while power
        .try_work_discrete_amount(&owner(), order.profile.commit_energy)
        .unwrap()
    {
        paid += 1;
        assert!(paid < 100);
    }
    assert!(paid > 0);
    let before = power.clone();
    let mut stock = inventory.clone();
    let mut flags = Vec::new();
    assert_eq!(
        work(
            TransitWork::Commit,
            &order,
            &owner(),
            &mut power,
            &mut stock,
            &logistics,
            &BTreeMap::new(),
            &mut flags
        )
        .unwrap(),
        TransitGrant::Commitment { paid: false }
    );
    assert_eq!(stock, inventory);
    assert_eq!(power, before);
    assert!(flags.is_empty());
    // Offline/missing actual source retains preparation without free work.
    app.world_mut()
        .get_mut::<ShipSystemControlSources>(entity)
        .unwrap()
        .0
        .set(owner(), ControlSource::Offline);
    let reserve = energy(&app, entity);
    step(&mut app);
    assert_eq!(energy(&app, entity), reserve);
    assert_eq!(fuel(&app, entity), 6);
    assert_eq!(
        app.world().get::<ShipTransitDrives>(entity).unwrap().0[&owner()].worked_secs(),
        0.0
    );
    app.world_mut()
        .get_mut::<ShipSystemControlSources>(entity)
        .unwrap()
        .0
        .set(owner(), ControlSource::Human);
    step(&mut app);
    step(&mut app);
    assert_eq!(fuel(&app, entity), 4);
}

#[test]
fn already_committed_arrival_needs_only_the_existing_physical_owner_after_power_loss() {
    let (mut app, entity) = fixture();
    command(
        &mut app,
        entity,
        Control::BeginTransitToWaypoint { request: None },
    );
    step(&mut app);
    step(&mut app);
    let state = &app.world().get::<ShipTransitDrives>(entity).unwrap().0[&owner()];
    let TransitPhase::Committed { arrival, .. } = &state.phase else {
        panic!("paid before equipment loss")
    };
    let expected = arrival.point;
    app.world_mut()
        .entity_mut(entity)
        .remove::<ShipPowerSystem>();
    step(&mut app);
    let physics = app.world().get::<ShipPhysics>(entity).unwrap();
    assert_eq!([physics.x, physics.y, physics.z], expected);
    assert_eq!(fuel(&app, entity), 4);
    assert!(app.world().get::<ShipPowerSystem>(entity).is_none());
    assert!(matches!(
        app.world().get::<ShipTransitDrives>(entity).unwrap().0[&owner()].phase,
        TransitPhase::Arrived { .. }
    ));
}

#[test]
fn stale_or_unpermitted_course_cannot_begin_and_current_drive_view_does_not_publish_gravity_inputs()
{
    let (mut app, entity) = fixture();
    app.world_mut()
        .get_mut::<NavigationWaypoint>(entity)
        .unwrap()
        .set(WaypointMode::Free { x: 120.0, z: 20.0 });
    let reserve = energy(&app, entity);
    command(
        &mut app,
        entity,
        Control::BeginTransitToWaypoint { request: None },
    );
    assert_eq!(
        app.world().get::<ShipTransitDrives>(entity).unwrap().0[&owner()].phase,
        TransitPhase::Idle
    );
    assert_eq!(energy(&app, entity), reserve);
    set_course(&mut app, entity, 100.0, 30.0);
    command(
        &mut app,
        entity,
        Control::BeginTransitToWaypoint {
            request: Some(TransitRequestKey {
                objective_id: "unheard".into(),
                revision: 0,
            }),
        },
    );
    assert_eq!(
        app.world().get::<ShipTransitDrives>(entity).unwrap().0[&owner()].phase,
        TransitPhase::Idle
    );
    command(
        &mut app,
        entity,
        Control::BeginTransitToWaypoint { request: None },
    );
    step(&mut app);
    step(&mut app);
    app.world_mut().run_system_once(publish).unwrap();
    let boards = app.world().get::<ShipSystemBlackboards>(entity).unwrap();
    let Some(SystemBlackboard::Transit(view)) = boards.0.get(&owner()) else {
        panic!("actual owner publication")
    };
    assert_eq!(view.phase, TransitPhaseView::Committed);
    assert_eq!(view.actual_arrival, None);
    let encoded = serde_json::to_value(view).unwrap();
    for private in [
        "gravity",
        "scatter_offset",
        "at_center_direction",
        "origin",
        "profile",
    ] {
        assert!(encoded.get(private).is_none());
    }
    let config = &app.world().get::<ShipConfigComponent>(entity).unwrap().0;
    assert!(config
        .observations_for_system(&owner(), &boards.0)
        .get(&owner())
        .is_some());
    let foreign = config
        .systems
        .iter()
        .find(|system| system.console.as_ref().is_some_and(|id| id.0 == "tactical"))
        .unwrap();
    assert!(config
        .observations_for_system(&foreign.id, &boards.0)
        .get(&owner())
        .is_none());
}

#[test]
fn static_placement_requires_actual_current_policy_and_pose_but_committed_receipt_survives_loss() {
    let (mut app, entity) = fixture();
    let mut row: crate::world::config::WorldEntity = toml::from_str(
        r#"
template_path="assets/entities/nav_beacon.toml"
name="gravity"
transform={position=[100,0,30]}
[overrides.arrival_gravity]
exclusion_radius=8
"#,
    )
    .unwrap();
    // The accepted static source is resolved through the ordinary World override.
    let source = crate::entities::loader::resolve_entity_via(
        &row,
        &crate::entities::config_cache::get_config_cache(),
        &crate::entities::loader::WasmTemplateLoader,
    )
    .unwrap();
    let body = crate::entities::spawner::spawn_entity(
        &mut app.world_mut().commands(),
        &source,
        Vec3::new(100.0, 0.0, 30.0),
        "gravity-id".into(),
        None,
    );
    app.world_mut().flush();
    row.spawn_on = crate::world::config::WorldEntitySpawnOn::Immediate;
    app.world_mut()
        .resource_mut::<crate::world::config::WorldConfig>()
        .entities
        .push(row);
    app.world_mut()
        .resource_mut::<crate::world::config::WorldConfig>()
        .name_to_uuid
        .insert("gravity".into(), "gravity-id".into());
    command(
        &mut app,
        entity,
        Control::BeginTransitToWaypoint { request: None },
    );
    step(&mut app);
    app.world_mut()
        .get_mut::<Transform>(body)
        .unwrap()
        .translation
        .x += 1.0;
    let before = energy(&app, entity);
    step(&mut app);
    assert_eq!(fuel(&app, entity), 6);
    assert!((before - energy(&app, entity) - 1.0).abs() < 1e-5); // preparation only, no commit
    assert_eq!(
        app.world().get::<TransitReadouts>(entity).unwrap().0[&owner()],
        TransitAdvance::Waiting(TransitWait::NoPlacement)
    );
    app.world_mut()
        .get_mut::<Transform>(body)
        .unwrap()
        .translation
        .x -= 1.0;
    step(&mut app);
    let state = &app.world().get::<ShipTransitDrives>(entity).unwrap().0[&owner()];
    let TransitPhase::Committed { arrival, .. } = &state.phase else {
        panic!("current physical source permits commitment")
    };
    let expected = arrival.point;
    assert!(
        f64::from(expected[0] - 100.0)
            .hypot(f64::from(expected[1]))
            .hypot(f64::from(expected[2] - 30.0))
            >= 8.0
    );
    app.world_mut().despawn(body);
    step(&mut app);
    let physics = app.world().get::<ShipPhysics>(entity).unwrap();
    assert_eq!([physics.x, physics.y, physics.z], expected);
    assert_eq!(fuel(&app, entity), 4);
}

#[test]
fn ordinary_drive_ai_requires_read_explicit_permitted_request_and_uses_normal_admission() {
    use crate::core::messages::*;
    let (mut app, entity) = fixture();
    crate::ai::host::register_ai_host_env(&mut app);
    crate::science::reading::register(&mut app);
    app.world_mut()
        .get_mut::<ShipSystemControlSources>(entity)
        .unwrap()
        .0
        .set(owner(), ControlSource::Ai);
    app.world_mut().run_system_once(ai::operate).unwrap();
    assert!(
        app.world()
            .get::<AdmittedCommands>(entity)
            .unwrap()
            .0
            .is_empty(),
        "a course alone never orders a jump"
    );
    let key = TransitRequestKey {
        objective_id: "authorized-jump".into(),
        revision: 7,
    };
    app.world_mut()
        .get_mut::<ShipSystemBlackboards>(entity)
        .unwrap()
        .0
        .insert(
            crate::ship::system_registry::viewscreen_system_id(),
            SystemBlackboard::Viewscreen(ViewscreenBlackboard {
                scored_objectives: vec![ScoredObjective {
                    id: key.objective_id.clone(),
                    score: 10.0,
                    directive: AiDirective::Transit {
                        revision: key.revision,
                    },
                    source: ObjectiveSource::Mission,
                    relevance: vec![SystemAffinity::Helm],
                    snapshot: ObjectiveSnapshot {
                        id: key.objective_id.clone(),
                        text: "transit.request".into(),
                        text_params: Default::default(),
                        mandatory: false,
                        status: ObjectiveStatus::Active,
                        unassigned: false,
                        progress: None,
                        targets: vec![],
                        source: ObjectiveSource::Mission,
                    },
                }],
                ..Default::default()
            }),
        );
    let reserve = energy(&app, entity);
    for now in 2..=122 {
        app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = now;
        app.world_mut().run_schedule(FixedUpdate);
        app.world_mut().run_system_once(ai::operate).unwrap();
        if now < 122 {
            assert!(
                app.world()
                    .get::<AdmittedCommands>(entity)
                    .unwrap()
                    .0
                    .is_empty(),
                "must read actual directive before emission"
            );
        }
    }
    assert_eq!(
        energy(&app, entity),
        reserve,
        "reading does not buy physical work"
    );
    let commands = &app.world().get::<AdmittedCommands>(entity).unwrap().0;
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].target, owner());
    assert_eq!(
        commands[0].payload,
        Control::BeginTransitToWaypoint {
            request: Some(key.clone())
        }
    );
    assert_eq!(commands[0].response_token.as_deref(), Some("ai:jump-ship"));
    app.world_mut().run_system_once(handle_commands).unwrap();
    app.world_mut()
        .get_mut::<AdmittedCommands>(entity)
        .unwrap()
        .0
        .clear();
    assert!(matches!(
        app.world().get::<ShipTransitDrives>(entity).unwrap().0[&owner()].phase,
        TransitPhase::Preparing { .. }
    ));
    app.world_mut()
        .get_mut::<ShipSystemControlSources>(entity)
        .unwrap()
        .0
        .set(owner(), ControlSource::Human);
    app.world_mut()
        .get_mut::<ShipSystemBlackboards>(entity)
        .unwrap()
        .0
        .clear();
    for _ in 0..3 {
        step(&mut app);
    }
    assert!(matches!(
        app.world().get::<ShipTransitDrives>(entity).unwrap().0[&owner()].phase,
        TransitPhase::Arrived { .. }
    ));
    assert!(
        !app.world().get::<ShipTransitDrives>(entity).unwrap().0[&owner()].request_is_fresh(&key)
    );
}

fn two_drive_fixture() -> (App, Entity, SystemId) {
    let mut config = crate::entities::include_resolve::load_entity_config(
        "assets/entities/local_jump_cruiser.toml",
    )
    .unwrap();
    let alternate = SystemId("alternate-jump".into());
    let ship = config.ship_config.as_mut().unwrap();
    let mut system = ship.system(&owner()).unwrap().clone();
    system.id = alternate.clone();
    for component in &mut system.components {
        if let crate::ship::composition::SystemComponentConfig::TransitDrive { config, .. } =
            component
        {
            config.fuel.store = "alternate-transit-fuel".into();
        }
    }
    ship.systems.push(system);
    for rating in &mut ship
        .stations
        .iter_mut()
        .find(|station| station.id.0 == "helm")
        .unwrap()
        .ratings
    {
        rating
            .detailed_systems
            .as_mut()
            .expect("actual Helm ratings explicitly bind direct-control Systems")
            .push(alternate.clone());
    }
    for console in ship
        .consoles
        .iter_mut()
        .filter(|console| console.id.0 == "helm" || console.id.0 == "engineering")
    {
        console.feeds.push(phoenix_model::console::ConsoleFeed {
            source: SystemId("stock/alternate-jump".into()),
            kind: phoenix_model::console::ConsoleObservationKind::Stock,
        });
    }
    let energy = config.energy.as_mut().unwrap();
    let mut consumer = energy
        .consumers
        .iter()
        .find(|row| row.system == owner())
        .unwrap()
        .clone();
    consumer.system = alternate.clone();
    energy.consumers.push(consumer);
    config
        .interior
        .as_mut()
        .unwrap()
        .placements
        .insert(alternate.clone(), "engineering".into());
    let infrastructure = config.infrastructure.as_mut().unwrap();
    let mut capacity = infrastructure
        .capacities
        .iter()
        .find(|row| row.id == "drive-transit-fuel")
        .unwrap()
        .clone();
    capacity.id = "alternate-transit-fuel".into();
    infrastructure.capacities.push(capacity);
    let logistics = config.logistics.as_mut().unwrap();
    let mut store = logistics
        .stores
        .iter()
        .find(|row| row.id == "local-transit-fuel")
        .unwrap()
        .clone();
    store.id = "alternate-transit-fuel".into();
    store.system = Some(alternate.clone());
    store.endpoint = phoenix_model::logistics::ResourceEndpoint::Capacity {
        id: "alternate-transit-fuel".into(),
    };
    logistics.stores.push(store);
    logistics
        .storage
        .iter_mut()
        .find(|row| row.compartment == "engineering")
        .unwrap()
        .space += 12;
    // Both drives must pass ordinary strict authored validation and compile
    // actual Power, Stock and ownership before either receives a command.
    let config = crate::entities::config::EntityConfig::from_toml(
        &toml::to_string(&config.to_toml_value().unwrap()).unwrap(),
    )
    .unwrap();
    let (mut app, entity) = fixture_with_config(config);
    app.world_mut()
        .get_mut::<ShipSystemControlSources>(entity)
        .unwrap()
        .0
        .set(alternate.clone(), ControlSource::Human);
    (app, entity, alternate)
}

#[test]
fn competing_actual_drive_begin_refuses_until_arrival_and_forged_parallel_restore_is_no_write() {
    let (mut app, entity, alternate) = two_drive_fixture();
    let alternate_energy = app
        .world()
        .get::<ShipPowerSystem>(entity)
        .unwrap()
        .0
        .network()
        .consumers[&alternate]
        .energy;
    let commands = [owner(), alternate.clone()]
        .into_iter()
        .map(|target| AdmittedCommand {
            target,
            payload: Control::BeginTransitToWaypoint { request: None },
            response_token: None,
            feedback_correlation: None,
        })
        .collect();
    app.world_mut()
        .get_mut::<AdmittedCommands>(entity)
        .unwrap()
        .0 = commands;
    app.world_mut().run_system_once(handle_commands).unwrap();
    app.world_mut()
        .get_mut::<AdmittedCommands>(entity)
        .unwrap()
        .0
        .clear();
    assert!(
        app.world().get::<ShipTransitDrives>(entity).unwrap().0[&owner()]
            .phase
            .active()
    );
    assert_eq!(
        app.world().get::<ShipTransitDrives>(entity).unwrap().0[&alternate],
        TransitState::default()
    );
    step(&mut app);
    assert_eq!(
        app.world()
            .get::<ShipPowerSystem>(entity)
            .unwrap()
            .0
            .network()
            .consumers[&alternate]
            .energy,
        alternate_energy
    );
    assert_eq!(
        app.world()
            .get::<InfrastructureCondition>(entity)
            .unwrap()
            .0
            .capacity("alternate-transit-fuel"),
        Some(6)
    );
    app.world_mut().run_system_once(publish).unwrap();
    let SystemBlackboard::Transit(view) =
        &app.world().get::<ShipSystemBlackboards>(entity).unwrap().0[&alternate]
    else {
        panic!("actual alternate drive publication")
    };
    assert!(!view.can_begin);

    let before = crate::snapshot::capture(app.world());
    let digest = crate::sim_digest::world_digest(app.world());
    let mut candidate = before.clone();
    let saved = candidate
        .entities
        .iter_mut()
        .find(|row| row.uuid == "jump-ship")
        .unwrap()
        .transit
        .as_mut()
        .unwrap();
    let second = saved.0[&owner()].clone();
    saved.0.insert(alternate.clone(), second);
    let ship = &app.world().get::<ShipConfigComponent>(entity).unwrap().0;
    assert!(saved
        .validate(ship, candidate.tick)
        .unwrap_err()
        .contains("multiple active"));
    let report = crate::snapshot::restore(app.world_mut(), &candidate);
    assert!(!report.is_complete());
    assert!(format!("{report:?}").contains("multiple active"));
    assert_eq!(crate::snapshot::capture(app.world()), before);
    assert_eq!(crate::sim_digest::world_digest(app.world()), digest);
    crate::snapshot::apply_entity_state(
        app.world_mut(),
        entity,
        &candidate.entities[0],
        candidate.tick,
    );
    assert_eq!(crate::snapshot::capture(app.world()), before);

    step(&mut app); // first drive commits; alternate remains blocked.
    let waiting = app.world().get::<ShipTransitDrives>(entity).unwrap().0[&alternate].clone();
    command_to(
        &mut app,
        entity,
        alternate.clone(),
        Control::BeginTransitToWaypoint { request: None },
    );
    assert_eq!(
        app.world().get::<ShipTransitDrives>(entity).unwrap().0[&alternate],
        waiting
    );
    step(&mut app); // first drive arrives, preserving its completed history.
    let completed = app.world().get::<ShipTransitDrives>(entity).unwrap().0[&owner()].clone();
    assert!(matches!(completed.phase, TransitPhase::Arrived { .. }));
    app.world_mut()
        .get_mut::<AdmittedCommands>(entity)
        .unwrap()
        .0
        .push(AdmittedCommand {
            target: alternate.clone(),
            payload: Control::BeginTransitToWaypoint { request: None },
            response_token: None,
            feedback_correlation: None,
        });
    app.world_mut().run_system_once(handle_commands).unwrap();
    let drives = app.world().get::<ShipTransitDrives>(entity).unwrap();
    assert!(drives.0[&alternate].phase.active());
    assert_eq!(drives.0[&owner()], completed);
}
