use super::*;
use bevy::ecs::system::RunSystemOnce;

fn rig(shared: bool, withhold: bool) -> (App, Entity, String) {
    rig_with_aim(
        shared,
        withhold,
        phoenix_model::focused_enquiry::FocusedAim::General,
    )
}
fn rig_with_aim(
    shared: bool,
    withhold: bool,
    aim: phoenix_model::focused_enquiry::FocusedAim,
) -> (App, Entity, String) {
    let mut config = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    crate::science::test_support::clone_instrument(&mut config, "sensors", "focused-two");
    config
        .sensing
        .as_mut()
        .unwrap()
        .focused
        .iter_mut()
        .find(|row| row.system.0 == "focused-two")
        .unwrap()
        .default_aim = aim;
    if !shared {
        let ship = config.ship_config.as_mut().unwrap();
        ship.systems
            .iter_mut()
            .find(|system| system.id.0 == "focused-two")
            .unwrap()
            .console = Some(crate::core::messages::ConsoleId("engineering".into()));
        ship.consoles
            .iter_mut()
            .find(|console| console.id.0 == "engineering")
            .unwrap()
            .feeds
            .retain(|feed| {
                feed.kind != phoenix_model::console::ConsoleObservationKind::SensorRadar
            });
    }
    if withhold {
        let ai = config
            .ship_config
            .as_mut()
            .unwrap()
            .systems
            .iter_mut()
            .find(|system| system.id.0 == "focused-two")
            .unwrap()
            .config
            .as_mut()
            .unwrap();
        ai["ai"]["tree"][0]["state"][0]["task"][1]["action"]["value"] =
            toml::Value::Table(toml::map::Map::from_iter([
                ("source".into(), toml::Value::String("literal".into())),
                ("value".into(), toml::Value::Boolean(false)),
            ]));
    }
    phoenix_sim_gameplay::science::sensing::validate_sensing(
        config.sensing.as_ref().unwrap(),
        &config,
    )
    .unwrap();
    let mut app = App::new();
    app.insert_resource(crate::sim_tick::SimTick(1))
        .insert_resource(crate::lobby::Sessions(
            crate::lobby::session::SessionManager::new(),
        ))
        .add_systems(Update, operate);
    let observer = crate::entities::spawner::spawn_entity(
        &mut app.world_mut().commands(),
        &config,
        Vec3::ZERO,
        "observer".into(),
        None,
    );
    app.world_mut().flush();
    app.world_mut()
        .run_system_once(super::super::server::publish_scan_blackboard)
        .unwrap();
    let radar = SystemId("sensor-radar".into());
    let profile = app
        .world()
        .get::<super::super::broad::BroadProfile>(observer)
        .unwrap()
        .0[&radar]
        .profile
        .clone();
    let picture = {
        let mut tracks = app.world_mut().get_mut::<BroadTracks>(observer).unwrap();
        let state = tracks.0.get_mut(&radar).unwrap();
        state
            .advance(
                &profile,
                1,
                0.1,
                1.0,
                &[super::super::broad::BroadMeasurement {
                    elevation: None,
                    observation: None,
                    source: "private-not-a-live-world-entity".into(),
                    selectable: true,
                    position: [5.0, 0.0],
                    support: 1.0,
                    identity: None,
                    classification: None,
                    assessment: None,
                }],
            )
            .unwrap();
        state.projection(&profile, 1, 0.1).unwrap()
    };
    let alias = picture[0].contact_id.clone();
    {
        let mut boards = app
            .world_mut()
            .get_mut::<ShipSystemBlackboards>(observer)
            .unwrap();
        boards.0.insert(
            radar,
            SystemBlackboard::SensorRadar(crate::core::messages::SensorRadarBlackboard {
                tracks: Some(picture),
                ..Default::default()
            }),
        );
        boards.0.insert(
            SystemId("sensors".into()),
            SystemBlackboard::Sensors(crate::core::messages::SensorsBlackboard {
                science_target_uuid: Some("private-not-a-live-world-entity".into()),
                ..Default::default()
            }),
        );
    }
    (app, observer, alias)
}

#[test]
fn optional_shield_equipment_uses_the_same_ordinary_tree_and_admitted_scan_command() {
    let (mut app, observer, alias) = rig_with_aim(
        true,
        false,
        phoenix_model::focused_enquiry::FocusedAim::Shields,
    );
    app.update();
    let commands = &app.world().get::<AdmittedCommands>(observer).unwrap().0;
    assert_eq!(commands.len(), 1);
    assert!(
        matches!(&commands[0].payload,SystemControlPayload::ScanTarget{uuid,aim:phoenix_model::focused_enquiry::FocusedAim::Shields,intrusive:false} if uuid==&alias)
    );
    assert_eq!(commands[0].target, SystemId("focused-two".into()));
}

#[test]
fn extra_focused_controller_uses_only_its_permitted_picture_and_never_steps_bound_owner() {
    let owner = SystemId("focused-two".into());
    let bound = SystemId("sensors".into());
    for shared in [true, false] {
        let (mut app, observer, alias) = rig(shared, false);
        let before = app
            .world()
            .get::<crate::ai::state_tree_runtime::ShipAiTreeRuntime>(observer)
            .unwrap()
            .clone();
        app.update();
        let commands = &app.world().get::<AdmittedCommands>(observer).unwrap().0;
        if shared {
            assert_eq!(commands.len(), 1);
            assert_eq!(commands[0].target, owner);
            assert!(
                matches!(&commands[0].payload,SystemControlPayload::ScanTarget{uuid, .. } if uuid==&alias)
            );
        } else {
            assert!(commands.is_empty(),"co-hosting or a private current association cannot grant missing Console observations");
        }
        let frames = app
            .world()
            .get::<crate::ai::state_tree_runtime::ShipAiTreeRuntime>(observer)
            .unwrap();
        assert_eq!(frames.0[&bound], before.0[&bound]);
        assert_ne!(frames.0[&owner], before.0[&owner]);
    }
}

#[test]
fn extra_focused_human_offline_missing_program_and_missing_report_preserve_private_frame() {
    let (mut app, observer, _) = rig(true, false);
    let owner = SystemId("focused-two".into());
    for (tick, source) in [
        (2, crate::ship::control_source::ControlSource::Human),
        (3, crate::ship::control_source::ControlSource::Offline),
    ] {
        app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = tick;
        app.world_mut()
            .get_mut::<ShipSystemControlSources>(observer)
            .unwrap()
            .0
            .set(owner.clone(), source);
        let before = app
            .world()
            .get::<crate::ai::state_tree_runtime::ShipAiTreeRuntime>(observer)
            .unwrap()
            .clone();
        app.update();
        assert_eq!(
            *app.world()
                .get::<crate::ai::state_tree_runtime::ShipAiTreeRuntime>(observer)
                .unwrap(),
            before
        );
        assert!(app
            .world()
            .get::<AdmittedCommands>(observer)
            .unwrap()
            .0
            .is_empty());
    }
    app.world_mut()
        .get_mut::<ShipSystemControlSources>(observer)
        .unwrap()
        .0
        .set(
            owner.clone(),
            crate::ship::control_source::ControlSource::Ai,
        );
    app.world_mut()
        .get_mut::<ShipSystemBlackboards>(observer)
        .unwrap()
        .0
        .remove(&super::super::scan_blackboard_key_for(&owner));
    let before = app
        .world()
        .get::<crate::ai::state_tree_runtime::ShipAiTreeRuntime>(observer)
        .unwrap()
        .clone();
    app.update();
    assert_eq!(
        *app.world()
            .get::<crate::ai::state_tree_runtime::ShipAiTreeRuntime>(observer)
            .unwrap(),
        before
    );
    app.world_mut()
        .run_system_once(super::super::server::publish_scan_blackboard)
        .unwrap();
    app.world_mut()
        .get_mut::<crate::ai::state_tree_runtime::ShipAiTreePrograms>(observer)
        .unwrap()
        .0
        .remove(&owner);
    app.update();
    assert_eq!(
        *app.world()
            .get::<crate::ai::state_tree_runtime::ShipAiTreeRuntime>(observer)
            .unwrap(),
        before
    );
    assert!(app
        .world()
        .get::<AdmittedCommands>(observer)
        .unwrap()
        .0
        .is_empty());
}

#[test]
fn authored_extra_focus_tree_can_withhold_real_available_enquiry() {
    let (mut app, observer, _) = rig(true, true);
    app.update();
    assert!(app
        .world()
        .get::<AdmittedCommands>(observer)
        .unwrap()
        .0
        .is_empty());
    assert!(
        !app.world().get::<BroadTracks>(observer).unwrap().0[&SystemId("sensor-radar".into())]
            .current
            .is_empty()
    );
}
