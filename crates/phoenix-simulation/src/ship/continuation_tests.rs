use super::*;
use crate::ship::components::ShipSystemControlSources;
use crate::ship::state::ShipPhysics;

fn ship(app: &mut App) -> Entity {
    app.world_mut()
        .spawn((
            ThrustInput::default(),
            SteeringInput::default(),
            LateralThrustInput::default(),
            VerticalThrustInput::default(),
            BoostCommand::default(),
            ImpulseCommand::default(),
            LastHelmInput::default(),
            TacticalRadarSelection::default(),
            SensorRadarSelection::default(),
            LastShipAttacker::default(),
            HelmEnginesAiPolicyState::default(),
            HelmSteeringAiPolicyState::default(),
            HelmBoostAiPolicyState::default(),
            HelmRecoveryHistory::default(),
        ))
        .id()
}

fn moving() -> ControlState {
    ControlState {
        thrust: 0.8,
        steering: -0.4,
        lateral: 0.3,
        vertical: 0.2,
        boost: true,
        impulse_phase: 1,
        last_helm: [0.1, 0.2, -0.3],
        target_lock: Some("combat-target".into()),
        sensor_lock: Some("science-target".into()),
        last_attacker: Some("attacker".into()),
        helm_policies: Some(std::array::from_fn(|index| PolicyState {
            current: format!("state-{index}"),
            entered_at_secs: 12.5,
            memory: Default::default(),
        })),
        helm_recovery: Some(RecoveryHistory {
            target: Some("00000000-0000-0000-0000-000000000007".into()),
            ranges: vec![3.0, 2.0, 1.0],
            ranges_capacity: 4,
            separation: vec![1.0, 2.0],
            separation_capacity: 3,
        }),
    }
}

#[test]
fn saved_control_replaces_defaults_and_preserves_desired_applied_and_target_memory() {
    let mut app = App::new();
    let entity = ship(&mut app);
    let neutral = ControlState::capture_from(app.world().entity(entity)).unwrap();
    let saved = moving();
    saved.restore_into(&mut app.world_mut().entity_mut(entity));
    assert_eq!(
        ControlState::capture_from(app.world().entity(entity)),
        Some(saved.clone())
    );
    app.world_mut().clear_trackers();
    saved.restore_into(&mut app.world_mut().entity_mut(entity));
    assert!(
        !app.world()
            .entity(entity)
            .get_ref::<LastShipAttacker>()
            .unwrap()
            .is_changed(),
        "restoring the same attacker must not raise the already-spent attacked edge"
    );
    neutral.restore_into(&mut app.world_mut().entity_mut(entity));
    assert_eq!(
        ControlState::capture_from(app.world().entity(entity)),
        Some(neutral)
    );
    assert!(app
        .world()
        .entity(entity)
        .get::<TacticalRadarSelection>()
        .unwrap()
        .0
        .is_none());
    assert!(app
        .world()
        .entity(entity)
        .get::<SensorRadarSelection>()
        .unwrap()
        .0
        .is_none());
    assert!(app
        .world()
        .entity(entity)
        .get::<LastShipAttacker>()
        .unwrap()
        .0
        .is_none());
}

#[test]
fn saved_control_retains_absent_components_and_legacy_impulse_fallback() {
    let mut world = World::new();
    let entity = world.spawn_empty().id();
    moving().restore_into(&mut world.entity_mut(entity));
    assert!(ControlState::capture_from(world.entity(entity)).is_none());
    assert!(!world.entity(entity).contains::<ThrustInput>());
    world
        .entity_mut(entity)
        .insert((ThrustInput::default(), ImpulseCommand::default()));
    let mut saved = moving();
    saved.impulse_phase = 255;
    saved.restore_into(&mut world.entity_mut(entity));
    assert_eq!(
        world.entity(entity).get::<ImpulseCommand>().unwrap().0,
        ImpulsePhase::Idle
    );
    assert!(!world.entity(entity).contains::<TacticalRadarSelection>());
}

#[test]
fn saved_control_keeps_the_existing_wire_layout_and_optional_defaults() {
    let wire = r#"(thrust:0.5,steering:-0.25,lateral:0.0,vertical:0.0,boost:false,impulse_phase:2,last_helm:(0.1,0.2,0.3))"#;
    let saved: ControlState = ron::from_str(wire).unwrap();
    assert_eq!(ron::to_string(&saved).unwrap(), wire);
    assert!(saved.target_lock.is_none());
    assert!(saved.sensor_lock.is_none());
    assert!(saved.helm_policies.is_none());
    assert!(saved.helm_recovery.is_none());
}

#[test]
fn restored_drive_intents_do_not_restart_exhausted_or_cancelled_drives() {
    let mut app = App::new();
    app.add_systems(Update, crate::ship::physics_systems::apply_helm_commands);
    let entity = ship(&mut app);
    app.world_mut().entity_mut(entity).insert((
        crate::ai::server::AiHighFidelity,
        crate::server_app::ShipBoost(crate::ship::boost::BoostState {
            active: false,
            battery: 0.5,
        }),
        crate::server_app::ShipImpulse(crate::ship::impulse::ImpulseState::new()),
        crate::ship::helm::DriveCommandWrites::default(),
    ));
    app.update(); // consume insertion before restoring a running peer
    moving().restore_into(&mut app.world_mut().entity_mut(entity));
    app.update();
    let ship = app.world().entity(entity);
    assert!(!ship.get::<crate::server_app::ShipBoost>().unwrap().0.active);
    assert_eq!(
        ship.get::<crate::server_app::ShipImpulse>()
            .unwrap()
            .0
            .phase,
        ImpulsePhase::Idle,
    );
    // A subsequent real producer write still engages both drives.
    let mut ship = app.world_mut().entity_mut(entity);
    ship.get_mut::<BoostCommand>().unwrap().0 = true;
    ship.get_mut::<ImpulseCommand>().unwrap().0 = ImpulsePhase::Charging;
    app.update();
    let ship = app.world().entity(entity);
    assert!(ship.get::<crate::server_app::ShipBoost>().unwrap().0.active);
    assert_eq!(
        ship.get::<crate::server_app::ShipImpulse>()
            .unwrap()
            .0
            .phase,
        ImpulsePhase::Charging,
    );
}

#[test]
fn restored_control_drives_the_same_first_physics_action() {
    for saved in [moving(), ControlState::default()] {
        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin)
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
                std::time::Duration::from_secs_f64(1.0 / 60.0),
            ))
            .add_systems(Update, crate::ship::physics_systems::integrate_ship_physics);
        #[cfg(debug_assertions)]
        app.init_resource::<crate::ship::helm::HelmPhysicsFrame>()
            .add_systems(First, crate::ship::helm::tick_helm_physics_frame);
        let live = ship(&mut app);
        let resumed = ship(&mut app);
        let baseline = ShipPhysics {
            forward_speed: 3.0,
            ..Default::default()
        };
        for entity in [live, resumed] {
            app.world_mut().entity_mut(entity).insert((
                baseline,
                ShipSystemControlSources::default(),
                crate::ai::server::AiHighFidelity,
            ));
        }
        app.update(); // initialize Time before the measured action
        saved.restore_into(&mut app.world_mut().entity_mut(live));
        moving().restore_into(&mut app.world_mut().entity_mut(resumed));
        let captured = ControlState::capture_from(app.world().entity(live)).unwrap();
        captured.restore_into(&mut app.world_mut().entity_mut(resumed));
        app.update();
        let actual = app.world().entity(resumed).get::<ShipPhysics>().unwrap();
        assert_eq!(
            actual,
            app.world().entity(live).get::<ShipPhysics>().unwrap()
        );
        assert_ne!(*actual, baseline, "the first integration actually ran");
        assert_eq!(
            ControlState::capture_from(app.world().entity(live)),
            ControlState::capture_from(app.world().entity(resumed))
        );
    }
}
