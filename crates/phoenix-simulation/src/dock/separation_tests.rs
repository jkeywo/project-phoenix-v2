use super::*;
use bevy::ecs::system::RunSystemOnce;

fn fixture() -> (App, Entity, Entity) {
    let config =
        crate::entities::include_resolve::load_entity_config("assets/entities/dock_probe.toml")
            .unwrap();
    let mut app = App::new();
    let mut time: Time = Time::default();
    time.advance_by(std::time::Duration::from_secs_f32(1.0 / 60.0));
    app.insert_resource(time);
    let owner = crate::entities::spawner::spawn_entity(
        &mut app.world_mut().commands(),
        &config,
        Vec3::new(10.0, 0.0, 0.0),
        "separation-owner".into(),
        None,
    );
    let target = crate::entities::spawner::spawn_entity(
        &mut app.world_mut().commands(),
        &config,
        Vec3::new(20.0, 0.0, 0.0),
        "separation-target".into(),
        None,
    );
    app.world_mut().flush();
    assert!(
        app.world().get::<ShipPhysics>(owner).is_some(),
        "accepted moving Dock spawn has the captured physical owner"
    );
    assert!(app.world().get::<ShipPhysics>(target).is_some());
    install_actual_dock_equipment(app.world_mut(), owner);
    app.world_mut().entity_mut(owner).insert((
        ShipPhysics {
            x: 10.0,
            ..Default::default()
        },
        crate::core::messages::AdmittedCommands::default(),
        crate::server_app::ShipSystemBlackboards::default(),
    ));
    app.world_mut().entity_mut(target).insert(ShipPhysics {
        x: 20.0,
        ..Default::default()
    });
    let mut dock = app.world_mut().get_mut::<DockControl>(owner).unwrap();
    dock.set_engaged(true);
    dock.docked = true;
    dock.set_docking_target(Some("separation-target".into()));
    (app, owner, target)
}
fn request(app: &mut App, owner: Entity) {
    app.world_mut()
        .get_mut::<crate::core::messages::AdmittedCommands>(owner)
        .unwrap()
        .0 = vec![correlated_dock_command(
        SystemControlPayload::Undock,
        "normal-release",
    )];
    app.world_mut()
        .run_system_once(handle_dock_commands)
        .unwrap();
    app.world_mut()
        .get_mut::<crate::core::messages::AdmittedCommands>(owner)
        .unwrap()
        .0
        .clear();
}
fn step(app: &mut App) {
    app.world_mut().run_system_once(tick_dock).unwrap();
}
fn energy(app: &App, owner: Entity) -> f32 {
    app.world()
        .get::<ShipPowerSystem>(owner)
        .unwrap()
        .0
        .network()
        .consumers[&SystemId("dock".into())]
        .energy
}
fn pose(app: &App, owner: Entity) -> [f32; 8] {
    let p = app.world().get::<ShipPhysics>(owner).unwrap();
    [
        p.x,
        p.y,
        p.z,
        p.yaw,
        p.forward_speed,
        p.roll,
        p.lateral_speed,
        p.vertical_speed,
    ]
}
fn state(app: &App, owner: Entity) -> serde_json::Value {
    serde_json::to_value((
        app.world().get::<DockControl>(owner).unwrap().save_state(),
        pose(app, owner),
        app.world()
            .get::<ShipPowerSystem>(owner)
            .unwrap()
            .0
            .capture_continuation(),
    ))
    .unwrap()
}
#[test]
fn accepted_release_uses_current_physical_marker_and_no_observer_rediscovery() {
    let (mut app, owner, _) = fixture();
    app.world_mut()
        .get_mut::<Transform>(owner)
        .unwrap()
        .translation = Vec3::new(-900.0, 80.0, 70.0);
    request(&mut app, owner);
    let dock = app.world().get::<DockControl>(owner).unwrap();
    assert!(!dock.docked && !dock.engaged());
    let clear = dock.undock_target.unwrap();
    assert!(clear.x < 10.0);
    assert_eq!(clear.z, 0.0);
    assert_eq!(clear.y, 0.0);
    let before = energy(&app, owner);
    step(&mut app);
    assert!(pose(&app, owner)[0] < 10.0);
    assert_eq!(pose(&app, owner)[2], 0.0);
    assert!(energy(&app, owner) < before);
}
#[test]
fn ordinary_release_refuses_unresolved_ambiguous_marker_without_mutating_the_mate() {
    let (mut app, owner, target) = fixture();
    app.world_mut().despawn(target);
    let before = state(&app, owner);
    request(&mut app, owner);
    let dock = app.world().get::<DockControl>(owner).unwrap();
    assert!(dock.docked && dock.engaged());
    assert!(dock.undock_target.is_none());
    assert_eq!(dock.last_refusal, Some(DockRefusal::NoDockMarkers));
    assert_eq!(
        before,
        state(&app, owner),
        "derived refusal does not change physical/saved owners"
    );
}
#[test]
fn missing_offline_destroyed_or_unpaid_dock_holds_then_resumes_the_same_clear_point() {
    for reason in ["missing", "offline", "destroyed", "empty"] {
        let (mut app, owner, _) = fixture();
        request(&mut app, owner);
        let destination = app.world().get::<DockControl>(owner).unwrap().undock_target;
        let config = app
            .world()
            .get::<crate::ship_plugin::ShipConfigComponent>(owner)
            .unwrap()
            .clone();
        let damage = app.world().get::<ShipDamage>(owner).unwrap().clone();
        let power = app
            .world()
            .get::<ShipPowerSystem>(owner)
            .unwrap()
            .0
            .capture_continuation();
        if reason == "missing" {
            app.world_mut()
                .entity_mut(owner)
                .remove::<crate::ship_plugin::ShipConfigComponent>();
        }
        if reason == "offline" {
            app.world_mut()
                .get_mut::<crate::ship_plugin::ShipSystemControlSources>(owner)
                .unwrap()
                .0
                .set(
                    SystemId("dock".into()),
                    crate::ship::control_source::ControlSource::Offline,
                );
        }
        if reason == "destroyed" {
            for part in app
                .world_mut()
                .get_mut::<ShipDamage>(owner)
                .unwrap()
                .state
                .components
                .get_mut(&SystemId("dock".into()))
                .unwrap()
                .values_mut()
            {
                part.condition = phoenix_model::damage::ComponentCondition::Destroyed;
            }
        }
        if reason == "empty" {
            app.world_mut()
                .get_mut::<ShipPowerSystem>(owner)
                .unwrap()
                .0
                .network_mut()
                .consumers
                .get_mut(&SystemId("dock".into()))
                .unwrap()
                .energy = 0.0;
        }
        let before = pose(&app, owner);
        let reserve = energy(&app, owner);
        step(&mut app);
        assert_eq!(pose(&app, owner), before, "{reason}");
        assert_eq!(energy(&app, owner), reserve, "{reason}");
        assert_eq!(
            app.world().get::<DockControl>(owner).unwrap().undock_target,
            destination
        );
        app.world_mut().entity_mut(owner).insert((config, damage));
        app.world_mut()
            .get_mut::<crate::ship_plugin::ShipSystemControlSources>(owner)
            .unwrap()
            .0
            .set(
                SystemId("dock".into()),
                crate::ship::control_source::ControlSource::Human,
            );
        app.world_mut()
            .get_mut::<ShipPowerSystem>(owner)
            .unwrap()
            .0
            .restore_continuation(&power)
            .unwrap();
        step(&mut app);
        assert_ne!(pose(&app, owner), before, "{reason}");
        assert!(energy(&app, owner) < power.network.consumers[&SystemId("dock".into())].energy);
    }
}
#[test]
fn paid_separation_json_resumes_exact_own_geometry_and_local_energy_for_360_steps() {
    let (mut original, owner, _) = fixture();
    request(&mut original, owner);
    step(&mut original);
    assert!(original
        .world()
        .get::<DockControl>(owner)
        .unwrap()
        .undock_target
        .is_some());
    let saved: (
        DockSaveState,
        [f32; 8],
        crate::modifiers::power_system::PowerState,
    ) = serde_json::from_value(state(&original, owner)).unwrap();
    let (mut resumed, other, _) = fixture();
    validate_saved_entity(resumed.world(), other, Some(&saved.0), Some(&saved.1)).unwrap();
    resumed
        .world_mut()
        .get_mut::<DockControl>(other)
        .unwrap()
        .restore(&saved.0);
    let [x, y, z, yaw, forward_speed, roll, lateral_speed, vertical_speed] = saved.1;
    resumed.world_mut().entity_mut(other).insert(ShipPhysics {
        x,
        y,
        z,
        yaw,
        forward_speed,
        lateral_speed,
        vertical_speed,
        roll,
    });
    resumed
        .world_mut()
        .get_mut::<ShipPowerSystem>(other)
        .unwrap()
        .0
        .restore_continuation(&saved.2)
        .unwrap();
    assert_eq!(state(&original, owner), state(&resumed, other));
    for _ in 0..360 {
        step(&mut original);
        step(&mut resumed);
        assert_eq!(state(&original, owner), state(&resumed, other));
    }
}

#[test]
fn malformed_saved_clearance_or_missing_pose_refuses_before_any_owner_write() {
    let (mut app, owner, _) = fixture();
    request(&mut app, owner);
    step(&mut app);
    let before = state(&app, owner);
    let saved = app.world().get::<DockControl>(owner).unwrap().save_state();
    let physical = pose(&app, owner);
    validate_saved_entity(app.world(), owner, Some(&saved), Some(&physical)).unwrap();
    assert!(validate_saved_entity(app.world(), owner, Some(&saved), None).is_err());
    assert!(validate_saved_entity(app.world(), owner, None, Some(&physical)).is_err());
    for failure in 0..5 {
        let mut bad = saved.clone();
        match failure {
            0 => bad.undock_target.as_mut().unwrap()[0] = f32::INFINITY,
            1 => bad.undock_target.as_mut().unwrap()[2] = f32::NAN,
            2 => bad.docked = true,
            3 => {
                bad.channels
                    .0
                    .insert("forged".into(), CommandChannelValue::Boolean(false));
            }
            _ => {
                let key = app
                    .world()
                    .get::<DockControl>(owner)
                    .unwrap()
                    .enabled_channel
                    .clone();
                bad.channels
                    .0
                    .insert(key, CommandChannelValue::EntityTarget(None));
            }
        }
        assert!(
            validate_saved_entity(app.world(), owner, Some(&bad), Some(&physical)).is_err(),
            "{failure}"
        );
        assert_eq!(state(&app, owner), before);
    }
}
#[test]
fn stationary_provider_releases_its_relationship_without_self_propulsion() {
    let config =
        crate::entities::include_resolve::load_entity_config("assets/entities/station_axiom.toml")
            .unwrap();
    let mut app = App::new();
    let mut time: Time = Time::default();
    time.advance_by(std::time::Duration::from_secs(1));
    app.insert_resource(time);
    let owner = crate::entities::spawner::spawn_entity(
        &mut app.world_mut().commands(),
        &config,
        Vec3::new(100.0, 0.0, 0.0),
        "stationary-provider".into(),
        None,
    );
    app.world_mut().flush();
    assert!(
        !app.world()
            .get::<DockControl>(owner)
            .unwrap()
            .config
            .moves_body
    );
    let physical_before = app.world().get::<ShipPhysics>(owner).copied();
    let before = *app.world().get::<Transform>(owner).unwrap();
    {
        let mut dock = app.world_mut().get_mut::<DockControl>(owner).unwrap();
        dock.set_engaged(true);
        dock.docked = true;
        dock.set_docking_target(Some("visitor".into()));
    }
    request(&mut app, owner);
    step(&mut app);
    let dock = app.world().get::<DockControl>(owner).unwrap();
    assert!(!dock.docked && !dock.engaged());
    assert!(dock.undock_target.is_none());
    assert_eq!(*app.world().get::<Transform>(owner).unwrap(), before);
    assert_eq!(
        app.world().get::<ShipPhysics>(owner).copied(),
        physical_before
    );
}

#[test]
fn separation_uses_canonical_heading_and_preserves_it_through_physical_writeback() {
    let (mut app, owner, _) = fixture();
    app.world_mut().get_mut::<ShipPhysics>(owner).unwrap().yaw = std::f32::consts::FRAC_PI_2;
    request(&mut app, owner);
    let clear = app
        .world()
        .get::<DockControl>(owner)
        .unwrap()
        .undock_target
        .unwrap();
    assert!((clear.x - 10.0).abs() < 0.0001);
    assert!(clear.z.abs() > 0.0);
    step(&mut app);
    let physical = app.world().get::<ShipPhysics>(owner).unwrap();
    assert!((physical.yaw - std::f32::consts::FRAC_PI_2).abs() < 0.0001);
    assert!((physical.x - 10.0).abs() < 0.0001);
    let expected = Quat::from_euler(EulerRot::YXZ, -physical.yaw, 0.0, physical.roll);
    assert!(app
        .world()
        .get::<Transform>(owner)
        .unwrap()
        .rotation
        .abs_diff_eq(expected, 0.0001));
}

#[test]
fn missing_physical_owner_cannot_form_a_nearby_mate_or_pay_for_a_proposed_snap() {
    let (mut app, owner, target) = fixture();
    app.init_resource::<EffectQueue<TaskLifecycleRequest>>();
    let own_pose = physical_pose(
        app.world().get::<Transform>(owner).unwrap(),
        app.world().get::<ShipPhysics>(owner),
    );
    let target_pose = physical_pose(
        app.world().get::<Transform>(target).unwrap(),
        app.world().get::<ShipPhysics>(target),
    );
    let mate = nearest_viable_pair(
        own_pose,
        &app.world().get::<DockMarkers>(owner).unwrap().markers,
        target_pose,
        &app.world().get::<DockMarkers>(target).unwrap().markers,
    )
    .unwrap()
    .mate;
    *app.world_mut().get_mut::<Transform>(owner).unwrap() = Transform {
        translation: mate.translation,
        rotation: mate.rotation,
        ..Default::default()
    };
    app.world_mut().entity_mut(owner).remove::<ShipPhysics>();
    app.world_mut()
        .get_mut::<DockControl>(owner)
        .unwrap()
        .docked = false;
    let channels = app.world().get::<DockControl>(owner).unwrap().save_state();
    let reserve = energy(&app, owner);
    step(&mut app);
    let dock = app.world().get::<DockControl>(owner).unwrap();
    assert!(dock.engaged() && !dock.docked);
    assert_eq!(dock.save_state(), channels);
    assert_eq!(dock.last_refusal, Some(DockRefusal::Disabled));
    assert_eq!(energy(&app, owner), reserve);
    assert_eq!(
        app.world().get::<Transform>(owner).unwrap().translation,
        mate.translation
    );
    assert!(drain_lifecycle(&mut app)
        .iter()
        .all(|request| !matches!(request, TaskLifecycleRequest::Start { .. })));
}

#[test]
fn stationary_dock_never_advertises_own_movement_availability() {
    let (mut app, owner, _) = fixture();
    {
        let mut dock = app.world_mut().get_mut::<DockControl>(owner).unwrap();
        dock.config.moves_body = false;
        dock.set_engaged(false);
        dock.docked = false;
        dock.set_docking_target(None);
    }
    step(&mut app);
    assert!(app
        .world()
        .get::<DockControl>(owner)
        .unwrap()
        .available_target
        .is_none());
    app.world_mut()
        .run_system_once(publish_dock_blackboard)
        .unwrap();
    let board = &app
        .world()
        .get::<crate::server_app::ShipSystemBlackboards>(owner)
        .unwrap()
        .0[&SystemId("dock".into())];
    assert!(matches!(board, SystemBlackboard::Dock(board) if !board.available));
}
