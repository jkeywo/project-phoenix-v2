use super::*;
use crate::resources::logistics_server::tests::{fixture_with, send};
use crate::ship::control_source::ControlSource;

fn id(value: &str) -> SystemId {
    SystemId(value.into())
}

fn rig(source: ControlSource, supply: i64, goal: i64) -> (App, Entity, Entity) {
    rig_with(source, supply, goal, |_| {})
}
fn rig_with(
    source: ControlSource,
    supply: i64,
    goal: i64,
    edit: impl FnOnce(&mut crate::entities::config::EntityConfig),
) -> (App, Entity, Entity) {
    let content = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    let profile = content
        .recovery
        .clone()
        .expect("authored fitted recovery tool");
    let layout = content.interior.clone().unwrap();
    let (mut app, ship) = fixture_with(source, edit);
    app.world_mut()
        .get_mut::<ShipLogistics>(ship)
        .unwrap()
        .0
        .enabled = false;
    app.world_mut()
        .get_mut::<ShipSystemControlSources>(ship)
        .unwrap()
        .0
        .set(id("logistics"), ControlSource::Offline);
    app.world_mut()
        .get_mut::<ShipSystemControlSources>(ship)
        .unwrap()
        .0
        .set(profile.system.clone(), source);
    app.world_mut()
        .get_mut::<ShipLogistics>(ship)
        .unwrap()
        .0
        .goals
        .insert("hold-scrap".into(), goal);
    app.world_mut().entity_mut(ship).insert((
        Transform::default(),
        crate::ship::access::InteriorLayout(layout),
        CompiledShipRecovery(profile.clone()),
        ShipRecovery(RecoveryState::new(&profile).unwrap()),
    ));
    register(&mut app);
    app.init_resource::<crate::world::server::WorldContentRuntime>()
        .init_resource::<EffectQueue<crate::infrastructure::CapacityAdjustment>>()
        .init_resource::<EffectQueue<crate::infrastructure::ConditionAdjustment>>()
        .init_resource::<EffectQueue<crate::resources::server::ResourceTransfer>>()
        .init_resource::<EffectQueue<crate::resources::server::ResourceTransferReceipt>>()
        .add_systems(
            FixedUpdate,
            crate::infrastructure::server::tick_infrastructure_condition
                .in_set(crate::sim_sets::SimSet::Modifiers),
        )
        .add_systems(
            FixedUpdate,
            crate::resources::server::tick_resource_transfers
                .in_set(crate::sim_sets::SimSet::Modifiers)
                .after(crate::infrastructure::server::tick_infrastructure_condition)
                .before(crate::resources::logistics_server::tick),
        );
    let source_config = RecoverableSourceConfig {
        availability: RecoveryAvailability::Always,
        resources: BTreeMap::from([(
            "scrap".into(),
            phoenix_model::logistics::ResourceEndpoint::Capacity { id: "scrap".into() },
        )]),
    };
    let target = app
        .world_mut()
        .spawn((
            EntityUuid("finite-source".into()),
            EntityName("Finite source".into()),
            Transform::from_xyz(6.0, 0.0, 0.0),
            RecoverableSource(source_config),
            InfrastructureCondition(crate::infrastructure::InfrastructureState::from_config(
                &crate::infrastructure::InfrastructureConfig {
                    capacities: vec![crate::infrastructure::CapacityConfig {
                        id: "scrap".into(),
                        amount: supply,
                        ceiling: Some(supply),
                        label: None,
                    }],
                    ..Default::default()
                },
            )),
        ))
        .id();
    let broad = crate::science::broad::BroadProfile::from_config(&content)
        .unwrap()
        .unwrap();
    let mut tracks = crate::science::broad::BroadTracks::new(&broad);
    let radar = id("sensor-radar");
    // This reduced owner fixture supplies one real paid instrument sample; it
    // does not run the complete Science/Power publisher graph.
    let paid = app
        .world_mut()
        .get_mut::<crate::ship::power::ShipPowerSystem>(ship)
        .unwrap()
        .0
        .work_continuous(&radar, 0.5, 1.0)
        .unwrap()
        .fraction;
    assert!(paid > 0.0);
    let instrument = &broad.0[&radar].profile;
    let state = tracks.0.get_mut(&radar).unwrap();
    state
        .advance(
            instrument,
            0,
            0.5,
            paid,
            &[crate::science::broad::BroadMeasurement {
                elevation: None,
                source: "finite-source".into(),
                selectable: true,
                observation: None,
                position: [6.0, 0.0],
                support: 1.0,
                identity: None,
                classification: None,
                assessment: None,
            }],
        )
        .unwrap();
    let public = state.projection(instrument, 0, 0.5).unwrap();
    let selected = public[0].contact_id.clone();
    app.world_mut().entity_mut(ship).insert((
        broad,
        tracks,
        crate::science::broad::SensingControls(content.sensing.as_ref().unwrap().control.clone()),
    ));
    app.world_mut()
        .get_mut::<crate::server_app::ShipSystemBlackboards>(ship)
        .unwrap()
        .0
        .insert(
            id("sensor-radar"),
            SystemBlackboard::SensorRadar(crate::core::messages::SensorRadarBlackboard {
                selected_target: Some(selected),
                tracks: Some(public),
                ..Default::default()
            }),
        );
    // Publish normal local readings before either controller chooses work.
    app.update();
    (app, ship, target)
}

fn stock(app: &App, entity: Entity) -> i64 {
    app.world()
        .get::<InfrastructureCondition>(entity)
        .unwrap()
        .0
        .capacity_stock("scrap")
        .unwrap()
        .level
}
fn energy(app: &App, entity: Entity) -> f32 {
    app.world()
        .get::<crate::ship::power::ShipPowerSystem>(entity)
        .unwrap()
        .0
        .network()
        .consumers[&id("resource-recovery")]
        .energy
}
fn start(app: &mut App) {
    let target = app
        .world_mut()
        .query::<&crate::server_app::ShipSystemBlackboards>()
        .iter(app.world())
        .find_map(|boards| match boards.0.get(&id("resource-recovery")) {
            Some(SystemBlackboard::Recovery(board)) => board
                .candidates
                .first()
                .map(|candidate| candidate.uuid.clone()),
            _ => None,
        })
        .unwrap_or_else(|| "unobserved-source".into());
    send(
        app,
        "engineer",
        "resource-recovery",
        Control::SetRecoverySelection {
            target_uuid: Some(target),
            resource: Some("scrap".into()),
            destination_store: Some("hold-scrap".into()),
        },
    );
    send(
        app,
        "engineer",
        "resource-recovery",
        Control::SetRecoveryEnabled { enabled: true },
    );
}
fn assert_settled(app: &App, ship: Entity) {
    let state = &app.world().get::<ShipRecovery>(ship).unwrap().0;
    let profile = &app.world().get::<CompiledShipRecovery>(ship).unwrap().0;
    state
        .validate(
            profile,
            app.world().resource::<crate::sim_tick::SimTick>().0,
        )
        .unwrap();
    assert!(app
        .world()
        .resource::<EffectQueue<crate::resources::server::ResourceTransfer>>()
        .0
        .is_empty());
    assert!(app
        .world()
        .resource::<EffectQueue<crate::resources::server::ResourceTransferReceipt>>()
        .0
        .is_empty());
}

fn reading(app: &App, ship: Entity) -> RecoveryBlackboard {
    let boards = &app
        .world()
        .get::<crate::server_app::ShipSystemBlackboards>(ship)
        .unwrap()
        .0;
    let SystemBlackboard::Recovery(board) = &boards[&id("resource-recovery")] else {
        panic!("Recovery publication")
    };
    board.clone()
}

#[test]
fn measured_recovery_candidates_never_reveal_the_source_identity_or_live_position() {
    let (mut app, ship, source) = rig(ControlSource::Human, 12, 3);
    let initial = reading(&app, ship);
    assert_eq!(initial.candidates.len(), 1);
    assert_ne!(initial.candidates[0].uuid, "finite-source");
    assert_eq!(initial.candidates[0].name, "console.common.unknown");
    let measured = {
        let boards = &app
            .world()
            .get::<crate::server_app::ShipSystemBlackboards>(ship)
            .unwrap()
            .0;
        let SystemBlackboard::SensorRadar(board) = &boards[&id("sensor-radar")] else {
            unreachable!()
        };
        board.tracks.as_ref().unwrap()[0].position
    };
    assert_eq!(
        initial.candidates[0].distance,
        f64::from(measured[0]).hypot(f64::from(measured[1])) as f32
    );
    app.world_mut().entity_mut(source).insert((
        EntityName("SECRET LIVE IDENTITY".into()),
        Transform::from_xyz(700.0, 900.0, 400.0),
    ));
    app.update();
    let changed = reading(&app, ship);
    assert_eq!(
        changed.candidates, initial.candidates,
        "held public measurement does not follow unobserved motion/name"
    );
    assert!(
        changed.next_recovery.is_none(),
        "physical executor still refuses work outside its real range"
    );
    let wire = serde_json::to_string(&changed).unwrap();
    for secret in [
        "finite-source",
        "SECRET LIVE IDENTITY",
        "700.0",
        "900.0",
        "400.0",
    ] {
        assert!(!wire.contains(secret), "{secret} escaped: {wire}");
    }
}

#[test]
fn new_recovery_selection_requires_the_same_tools_current_permitted_handle() {
    let (mut app, ship, source) = rig(ControlSource::Human, 12, 3);
    let handle = reading(&app, ship).candidates[0].uuid.clone();
    let before = energy(&app, ship);
    send(
        &mut app,
        "engineer",
        "resource-recovery",
        Control::SetRecoverySelection {
            target_uuid: Some("finite-source".into()),
            resource: Some("scrap".into()),
            destination_store: Some("hold-scrap".into()),
        },
    );
    app.update();
    assert!(app
        .world()
        .get::<ShipRecovery>(ship)
        .unwrap()
        .0
        .target
        .is_none());
    assert_eq!(energy(&app, ship), before);
    assert_eq!((stock(&app, ship), stock(&app, source)), (0, 12));
    let observer = app.world().get::<EntityUuid>(ship).unwrap().0.clone();
    app.world_mut()
        .resource_mut::<crate::world::server::WorldContentRuntime>()
        .contact_overrides
        .entry(observer.clone())
        .or_default()
        .insert(
            "finite-source".into(),
            crate::gm::operations::contact::ContactMode::Conceal,
        );
    send(
        &mut app,
        "engineer",
        "resource-recovery",
        Control::SetRecoverySelection {
            target_uuid: Some(handle.clone()),
            resource: Some("scrap".into()),
            destination_store: Some("hold-scrap".into()),
        },
    );
    app.update();
    assert!(app
        .world()
        .get::<ShipRecovery>(ship)
        .unwrap()
        .0
        .target
        .is_none());
    assert!(reading(&app, ship).candidates.is_empty());
    assert_eq!(energy(&app, ship), before);
    app.world_mut()
        .resource_mut::<crate::world::server::WorldContentRuntime>()
        .contact_overrides
        .remove(&observer);
    send(
        &mut app,
        "engineer",
        "resource-recovery",
        Control::SetRecoverySelection {
            target_uuid: Some(handle),
            resource: Some("scrap".into()),
            destination_store: Some("hold-scrap".into()),
        },
    );
    send(
        &mut app,
        "engineer",
        "resource-recovery",
        Control::SetRecoveryEnabled { enabled: true },
    );
    app.update();
    let state = &app.world().get::<ShipRecovery>(ship).unwrap().0;
    assert_eq!(
        state.target.as_deref(),
        Some("finite-source"),
        "only internal machinery keeps the actual endpoint route"
    );
    assert!(state.fraction > 0.0);
    assert!(energy(&app, ship) < before);
}

#[test]
fn human_and_ordinary_ai_keep_admitted_paid_work_after_the_public_handle_fades() {
    for controller in [ControlSource::Human, ControlSource::Ai] {
        let (mut app, ship, source) = rig(controller, 12, 3);
        if controller == ControlSource::Human {
            start(&mut app);
        }
        for _ in 0..8 {
            app.update();
            if app.world().get::<ShipRecovery>(ship).unwrap().0.fraction > 0.0 {
                break;
            }
        }
        assert!(app.world().get::<ShipRecovery>(ship).unwrap().0.fraction > 0.0);
        let before = energy(&app, ship);
        app.world_mut()
            .get_mut::<crate::server_app::ShipSystemBlackboards>(ship)
            .unwrap()
            .0
            .insert(
                id("sensor-radar"),
                SystemBlackboard::SensorRadar(Default::default()),
            );
        app.update();
        let board = reading(&app, ship);
        assert!(board.enabled && board.target_uuid.is_none() && board.candidates.is_empty());
        for _ in 0..40 {
            app.update();
            assert_settled(&app, ship);
        }
        assert_eq!((stock(&app, ship), stock(&app, source)), (3, 9));
        assert!(energy(&app, ship) < before);
        assert!(reading(&app, ship).target_uuid.is_none());
    }
}

#[test]
fn human_standing_recovery_pays_for_real_finite_supply_and_stops_at_goal() {
    let (mut app, ship, source) = rig(ControlSource::Human, 12, 3);
    let before = energy(&app, ship);
    start(&mut app);
    for _ in 0..40 {
        app.update();
        assert_eq!(stock(&app, ship) + stock(&app, source), 12);
        assert_settled(&app, ship);
    }
    assert_eq!((stock(&app, ship), stock(&app, source)), (3, 9));
    assert!((before - energy(&app, ship) - 1.5).abs() < 0.001);
    let finished = energy(&app, ship);
    for _ in 0..10 {
        app.update();
    }
    assert_eq!(energy(&app, ship), finished);
}

#[test]
fn ordinary_recovery_ai_uses_the_same_permitted_goal_and_paid_executor() {
    let (mut app, ship, source) = rig(ControlSource::Ai, 2, 3);
    let before = energy(&app, ship);
    for _ in 0..60 {
        app.update();
        assert_eq!(stock(&app, ship) + stock(&app, source), 2);
        assert_settled(&app, ship);
    }
    assert_eq!((stock(&app, ship), stock(&app, source)), (2, 0));
    assert!((before - energy(&app, ship) - 1.0).abs() < 0.001);
}

#[test]
fn no_goal_or_out_of_range_target_takes_neither_power_nor_stock() {
    let (mut app, ship, source) = rig(ControlSource::Human, 4, 0);
    start(&mut app);
    let before = energy(&app, ship);
    for _ in 0..8 {
        app.update();
    }
    assert_eq!(energy(&app, ship), before);
    app.world_mut()
        .get_mut::<ShipLogistics>(ship)
        .unwrap()
        .0
        .goals
        .insert("hold-scrap".into(), 3);
    app.world_mut()
        .get_mut::<Transform>(source)
        .unwrap()
        .translation
        .x = 20.0;
    for _ in 0..8 {
        app.update();
        assert_settled(&app, ship);
    }
    assert_eq!((stock(&app, ship), stock(&app, source)), (0, 4));
    assert_eq!(energy(&app, ship), before);
}

#[test]
fn cohosted_unbound_sensor_reading_cannot_select_or_automate_recovery() {
    let (mut app, ship, source) = rig_with(ControlSource::Human, 4, 3, |content| {
        let config = content.ship_config.as_mut().unwrap();
        let owner = config
            .systems
            .iter()
            .find(|system| system.id == id("resource-recovery"))
            .unwrap()
            .console
            .clone()
            .unwrap();
        let mut isolated = config
            .consoles
            .iter()
            .find(|console| console.id == owner)
            .unwrap()
            .clone();
        isolated.id = crate::core::messages::ConsoleId("recovery-scope".into());
        assert_eq!(
            isolated.home_station,
            config
                .consoles
                .iter()
                .find(|console| console.id == owner)
                .unwrap()
                .home_station
        );
        config
            .systems
            .iter_mut()
            .find(|system| system.id == id("resource-recovery"))
            .unwrap()
            .console = Some(isolated.id.clone());
        config.consoles.push(isolated);
    });
    let mut config = app
        .world_mut()
        .get_mut::<ShipConfigComponent>(ship)
        .unwrap();
    let owner = config
        .0
        .systems
        .iter()
        .find(|system| system.id == id("resource-recovery"))
        .unwrap()
        .console
        .clone();
    let console = config
        .0
        .consoles
        .iter_mut()
        .find(|console| Some(&console.id) == owner.as_ref())
        .unwrap();
    let original = console.feeds.clone();
    console.feeds.retain(|binding| {
        binding.kind != phoenix_model::console::ConsoleObservationKind::SensorRadar
    });
    drop(config);
    let before = energy(&app, ship);
    start(&mut app);
    for _ in 0..15 {
        app.update();
    }
    assert_eq!((stock(&app, ship), stock(&app, source)), (0, 4));
    assert_eq!(energy(&app, ship), before);
    assert!(app
        .world()
        .get::<ShipRecovery>(ship)
        .unwrap()
        .0
        .target
        .is_none());
    let boards = &app
        .world()
        .get::<crate::server_app::ShipSystemBlackboards>(ship)
        .unwrap()
        .0;
    let SystemBlackboard::Recovery(board) = &boards[&id("resource-recovery")] else {
        panic!("recovery readout");
    };
    assert!(board.candidates.is_empty() && board.next_recovery.is_none());
    // Restore only the feed. A positive command on the same compiled hosting
    // binding proves the previous refusal was knowledge, not missing authority.
    app.world_mut()
        .get_mut::<ShipConfigComponent>(ship)
        .unwrap()
        .0
        .consoles
        .iter_mut()
        .find(|console| Some(&console.id) == owner.as_ref())
        .unwrap()
        .feeds = original;
    app.update();
    start(&mut app);
    for _ in 0..40 {
        app.update();
        assert_settled(&app, ship);
    }
    assert_eq!((stock(&app, ship), stock(&app, source)), (3, 1));
    assert!(energy(&app, ship) < before);
}

#[test]
fn disabling_then_resuming_retains_paid_fraction_without_duplicate_supply() {
    let (mut app, ship, source) = rig(ControlSource::Human, 4, 3);
    start(&mut app);
    app.update();
    let partial = app.world().get::<ShipRecovery>(ship).unwrap().0.fraction;
    assert!(partial > 0.0 && partial < 1.0);
    send(
        &mut app,
        "engineer",
        "resource-recovery",
        Control::SetRecoveryEnabled { enabled: false },
    );
    app.update();
    let paused = energy(&app, ship);
    for _ in 0..8 {
        app.update();
    }
    assert_eq!(
        app.world().get::<ShipRecovery>(ship).unwrap().0.fraction,
        partial
    );
    assert_eq!(energy(&app, ship), paused);
    send(
        &mut app,
        "engineer",
        "resource-recovery",
        Control::SetRecoveryEnabled { enabled: true },
    );
    for _ in 0..40 {
        app.update();
        assert_settled(&app, ship);
    }
    assert_eq!((stock(&app, ship), stock(&app, source)), (3, 1));
}

#[test]
fn acquired_target_continues_when_science_moves_on_but_cannot_acquire_an_unknown_source() {
    let (mut app, ship, source) = rig(ControlSource::Human, 4, 3);
    start(&mut app);
    app.update();
    assert!(app.world().get::<ShipRecovery>(ship).unwrap().0.fraction > 0.0);
    app.world_mut()
        .get_mut::<crate::server_app::ShipSystemBlackboards>(ship)
        .unwrap()
        .0
        .insert(
            id("sensor-radar"),
            SystemBlackboard::SensorRadar(Default::default()),
        );
    for _ in 0..40 {
        app.update();
        assert_settled(&app, ship);
    }
    assert_eq!((stock(&app, ship), stock(&app, source)), (3, 1));
    let source_config = app
        .world()
        .get::<RecoverableSource>(source)
        .unwrap()
        .clone();
    let inventory = app
        .world()
        .get::<InfrastructureCondition>(source)
        .unwrap()
        .clone();
    let unknown = app
        .world_mut()
        .spawn((
            EntityUuid("unshared-source".into()),
            Transform::from_xyz(5.0, 0.0, 0.0),
            source_config,
            inventory,
        ))
        .id();
    let before = energy(&app, ship);
    send(
        &mut app,
        "engineer",
        "resource-recovery",
        Control::SetRecoverySelection {
            target_uuid: Some("unshared-source".into()),
            resource: Some("scrap".into()),
            destination_store: Some("hold-scrap".into()),
        },
    );
    app.update();
    assert_eq!(
        app.world()
            .get::<ShipRecovery>(ship)
            .unwrap()
            .0
            .target
            .as_deref(),
        Some("finite-source")
    );
    assert_eq!(stock(&app, unknown), 1);
    assert_eq!(energy(&app, ship), before);
}

#[test]
fn delegated_tree_can_stop_an_existing_human_standing_order() {
    let (mut app, ship, source) = rig(ControlSource::Human, 4, 3);
    start(&mut app);
    app.update();
    assert!(app.world().get::<ShipRecovery>(ship).unwrap().0.enabled);
    let definition=serde_json::from_value(serde_json::json!({"root":"recovery","tree":[{
        "id":"recovery","initial":"hold","state":[{"id":"hold","task":[{
            "id":"wait","action":{"kind":"write","channel":"recover","value":{"source":"literal","value":false}}
        }]}]
    }]})).unwrap();
    let program = phoenix_sim_gameplay::ai::state_tree::compile(
        &definition,
        &phoenix_sim_gameplay::resources::recovery_ai::bindings(),
    )
    .unwrap();
    app.world_mut()
        .get_mut::<crate::ai::state_tree_runtime::ShipAiTreePrograms>(ship)
        .unwrap()
        .0
        .insert(id("resource-recovery"), program);
    app.world_mut()
        .get_mut::<ShipSystemControlSources>(ship)
        .unwrap()
        .0
        .set(id("resource-recovery"), ControlSource::Ai);
    for _ in 0..20 {
        app.update();
    }
    assert!(!app.world().get::<ShipRecovery>(ship).unwrap().0.enabled);
    assert!(stock(&app, ship) < 3);
    let paused = (stock(&app, ship), stock(&app, source), energy(&app, ship));
    for _ in 0..30 {
        app.update();
        assert_settled(&app, ship);
    }
    assert_eq!(
        (stock(&app, ship), stock(&app, source), energy(&app, ship)),
        paused
    );
    assert_eq!(stock(&app, ship) + stock(&app, source), 4);
}

#[test]
fn idle_recovery_advances_its_settled_receipt_without_marking_power_changed() {
    use bevy::ecs::system::RunSystemOnce;
    let (mut app, ship, source) = rig(ControlSource::Human, 4, 2);
    let before = app
        .world()
        .get::<crate::ship::power::ShipPowerSystem>(ship)
        .unwrap()
        .0
        .clone();
    let mut expected = app.world().get::<ShipRecovery>(ship).unwrap().0.clone();
    assert!(!expected.enabled && expected.target.is_none());
    let before_stock = stock(&app, source);
    app.world_mut().clear_trackers();
    app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 += 1;
    let now = app.world().resource::<crate::sim_tick::SimTick>().0;
    app.world_mut().run_system_once(tick).unwrap();
    expected.last_tick = now;
    expected.last_outcome = Some(crate::resources::recovery::RecoveryOutcome {
        tick: now,
        offered: 0,
        moved: 0,
        settled: true,
    });
    assert_eq!(app.world().get::<ShipRecovery>(ship).unwrap().0, expected);
    assert_eq!(stock(&app, source), before_stock);
    assert_settled(&app, ship);
    let power = app
        .world()
        .entity(ship)
        .get_ref::<crate::ship::power::ShipPowerSystem>()
        .unwrap();
    assert_eq!(power.0, before);
    assert!(
        !power.is_changed(),
        "no payment callback must leave Power untouched"
    );
}

#[test]
fn recovery_lazy_payment_matches_eager_power_for_funding_shortfall_and_errors() {
    let (app, ship, _) = rig(ControlSource::Human, 4, 2);
    let original = app
        .world()
        .get::<crate::ship::power::ShipPowerSystem>(ship)
        .unwrap();
    for (system, seconds, factor) in [
        (id("resource-recovery"), 0.2, 1.0),
        (id("resource-recovery"), 1_000_000.0, 0.5),
        (id("missing-system"), 0.2, 1.0),
        (id("resource-recovery"), 0.2, 0.0),
    ] {
        let mut eager = original.clone();
        let mut staged = None;
        for _ in 0..2 {
            let expected = if factor > 0.0 {
                eager
                    .0
                    .work_continuous(&system, seconds, 1.0)
                    .map(|grant| grant.fraction * factor)
            } else {
                Ok(0.0)
            };
            let actual = pay_work(&system, seconds, factor, Some(original), &mut staged);
            assert_eq!(actual.map(f32::to_bits), expected.map(f32::to_bits));
            assert_eq!(
                staged.as_ref().map(|power| &power.0).unwrap_or(&original.0),
                &eager.0
            );
        }
        assert_eq!(staged.is_some(), factor > 0.0);
    }
}

#[test]
fn refused_recovery_after_actual_payment_discards_staged_power_and_work() {
    let (mut app, ship, _) = rig(ControlSource::Human, 4, 2);
    start(&mut app);
    app.update();
    let mut next = app.world().get::<ShipRecovery>(ship).unwrap().0.clone();
    assert!(next.enabled && next.target.is_some());
    let profile = &app.world().get::<CompiledShipRecovery>(ship).unwrap().0;
    let before = next.clone();
    let power = app
        .world()
        .get::<crate::ship::power::ShipPowerSystem>(ship)
        .unwrap();
    let before_power = power.0.clone();
    let mut staged = None;
    let refused = next.advance(profile, next.last_tick + 1, 0.2, 1, |system, seconds| {
        pay_work(system, seconds, f32::NAN, Some(power), &mut staged)
    });
    assert!(refused.is_err());
    let paid = &staged.as_ref().unwrap().0;
    assert!(
        paid.emitted_work(&id("resource-recovery"))
            > before_power.emitted_work(&id("resource-recovery"))
    );
    assert!(
        paid.network().consumers[&id("resource-recovery")].energy
            < before_power.network().consumers[&id("resource-recovery")].energy
    );
    assert_eq!(next, before);
    assert_eq!(
        power.0, before_power,
        "a refused transaction cannot publish its Power candidate"
    );
}
