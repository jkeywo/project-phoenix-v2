use super::*;
use phoenix_model::sensing::PassiveSignature;

#[test]
fn emp_blocks_actual_paid_receiver_then_expires_without_erasing_intent_or_battery() {
    use phoenix_model::{
        damage::DamageBinding,
        emp::{ComponentEmpState, EmpClock, EmpPayload},
    };
    let config = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    let mut app = App::new();
    let mut time = Time::<()>::default();
    time.advance_by(std::time::Duration::from_millis(100));
    app.insert_resource(time)
        .insert_resource(Time::<Fixed>::from_seconds(1.0))
        .insert_resource(crate::sim_tick::SimTick(100))
        .add_systems(
            Update,
            (crate::ship::emp::stamp_clock, tick, publish).chain(),
        );
    let receiver = crate::entities::spawner::spawn_entity(
        &mut app.world_mut().commands(),
        &config,
        Vec3::ZERO,
        "observer".into(),
        None,
    );
    crate::entities::spawner::spawn_entity(
        &mut app.world_mut().commands(),
        &config,
        Vec3::new(0.0, 0.0, -4.0),
        "emitter".into(),
        None,
    );
    app.world_mut().flush();
    let owner = crate::core::messages::SystemId("passive-receiver".into());
    {
        let mut physical = app
            .world_mut()
            .get_mut::<crate::ship::compartment_damage::ShipDamage>(receiver)
            .unwrap();
        let component = physical.definition.components[&owner]
            .iter()
            .find(|(_, row)| row.binding == DamageBinding::Operation)
            .unwrap()
            .0
            .clone();
        let effect = ComponentEmpState::apply(
            None,
            physical.definition.config.emp.as_ref().unwrap(),
            EmpPayload { duration_secs: 3.0 },
            EmpClock {
                tick: 100,
                period_nanos: 1_000_000_000,
            },
        )
        .unwrap();
        physical
            .state
            .components
            .get_mut(&owner)
            .unwrap()
            .get_mut(&component)
            .unwrap()
            .emp = Some(effect);
    }
    let before = app
        .world()
        .get::<ShipPowerSystem>(receiver)
        .unwrap()
        .0
        .capture_continuation();
    app.update();
    let state = &app.world().get::<PassiveState>(receiver).unwrap().0[&owner];
    assert!(state.enabled);
    assert!(state.signals.is_empty());
    assert_eq!(state.last_sample_tick, None);
    assert_eq!(
        app.world()
            .get::<ShipPowerSystem>(receiver)
            .unwrap()
            .0
            .capture_continuation(),
        before
    );
    let SystemBlackboard::Passive(board) = &app
        .world()
        .get::<crate::server_app::ShipSystemBlackboards>(receiver)
        .unwrap()
        .0[&owner]
    else {
        panic!("receiver publication")
    };
    assert!(!board.can_operate);
    app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = 103;
    app.update();
    let state = &app.world().get::<PassiveState>(receiver).unwrap().0[&owner];
    assert!(state.enabled && !state.signals.is_empty());
    assert_eq!(state.last_sample_tick, Some(103));
    assert!(
        app.world()
            .get::<ShipPowerSystem>(receiver)
            .unwrap()
            .0
            .capture_continuation()
            .network
            .consumers[&owner]
            .energy
            < before.network.consumers[&owner].energy
    );
}

#[test]
fn actual_directional_receiver_samples_and_publishes_its_one_paid_turret_bearing() {
    use crate::weapons::mounts::{MountProfiles, ShipMounts};
    let config = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    let mut app = App::new();
    let mut time = Time::<()>::default();
    time.advance_by(std::time::Duration::from_millis(100));
    app.insert_resource(time)
        .insert_resource(crate::sim_tick::SimTick(1))
        .add_systems(Update, (tick, publish).chain());
    let receiver = crate::entities::spawner::spawn_entity(
        &mut app.world_mut().commands(),
        &config,
        Vec3::ZERO,
        "observer".into(),
        None,
    );
    crate::entities::spawner::spawn_entity(
        &mut app.world_mut().commands(),
        &config,
        Vec3::new(4.0, 0.0, 0.0),
        "emitter".into(),
        None,
    );
    app.world_mut().flush();
    let owner = crate::core::messages::SystemId("passive-receiver".into());
    app.update();
    assert!(app.world().get::<PassiveState>(receiver).unwrap().0[&owner]
        .signals
        .is_empty());
    let profile = app.world().get::<MountProfiles>(receiver).unwrap().clone();
    let mut mounts = app.world().get::<ShipMounts>(receiver).unwrap().clone();
    let mut power = ShipPowerSystem(
        app.world()
            .get::<ShipPowerSystem>(receiver)
            .unwrap()
            .0
            .clone(),
    );
    let before = power.0.capture_continuation().network.consumers[&owner].energy;
    crate::weapons::mounts::advance_system(
        &profile,
        &mut mounts,
        &owner,
        2,
        1.0,
        true,
        Some(90.0),
        1.0,
        &mut power,
    )
    .unwrap();
    assert!(power.0.capture_continuation().network.consumers[&owner].energy < before);
    assert_eq!(mounts.0[&owner].bearing_deg(&profile.0[&owner]), 90.0);
    app.world_mut().entity_mut(receiver).insert((mounts, power));
    app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = 20;
    app.update();
    assert_eq!(
        app.world().get::<PassiveState>(receiver).unwrap().0[&owner]
            .signals
            .len(),
        1
    );
    let SystemBlackboard::Passive(board) = &app
        .world()
        .get::<crate::server_app::ShipSystemBlackboards>(receiver)
        .unwrap()
        .0[&owner]
    else {
        panic!("passive publication");
    };
    assert_eq!(board.mount.as_ref().unwrap().facing_deg, 90.0);
    assert_eq!(board.mount.as_ref().unwrap().arc_deg, 120.0);
}

#[test]
fn two_actual_receivers_keep_independent_intent_work_and_sample_frontiers() {
    use crate::core::messages::{AdmittedCommand, SystemId};
    let mut config = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    crate::science::test_support::clone_instrument(&mut config, "passive-receiver", "receiver-two");
    let first = SystemId("passive-receiver".into());
    let second = SystemId("receiver-two".into());
    let mut app = App::new();
    let mut time = Time::<()>::default();
    time.advance_by(std::time::Duration::from_millis(100));
    app.insert_resource(time)
        .insert_resource(crate::sim_tick::SimTick(1))
        .add_systems(Update, (tick, publish).chain());
    let receiver = crate::entities::spawner::spawn_entity(
        &mut app.world_mut().commands(),
        &config,
        Vec3::ZERO,
        "observer".into(),
        None,
    );
    crate::entities::spawner::spawn_entity(
        &mut app.world_mut().commands(),
        &config,
        Vec3::new(0.0, 0.0, -4.0),
        "secret-source".into(),
        None,
    );
    app.world_mut().flush();
    let before = app
        .world()
        .get::<ShipPowerSystem>(receiver)
        .unwrap()
        .0
        .capture_continuation();
    app.world_mut()
        .get_mut::<AdmittedCommands>(receiver)
        .unwrap()
        .0
        .push(AdmittedCommand {
            target: second.clone(),
            payload: Control::SetPassiveEnabled { enabled: false },
            response_token: None,
            feedback_correlation: None,
        });
    app.update();
    let state = app.world().get::<PassiveState>(receiver).unwrap();
    assert!(!state.0[&first].signals.is_empty());
    assert!(state.0[&second].signals.is_empty());
    assert!(!state.0[&second].enabled);
    let saved: PassiveState = serde_json::from_slice(&serde_json::to_vec(state).unwrap()).unwrap();
    let profile = app.world().get::<PassiveProfile>(receiver).unwrap();
    saved.validate(profile, "observer", 1).unwrap();
    let mut bad = saved.clone();
    bad.0.remove(&second);
    assert!(bad.validate(profile, "observer", 1).is_err());
    let after = app
        .world()
        .get::<ShipPowerSystem>(receiver)
        .unwrap()
        .0
        .capture_continuation();
    assert!(after.network.consumers[&first].energy < before.network.consumers[&first].energy);
    assert_eq!(
        after.network.consumers[&second].energy,
        before.network.consumers[&second].energy
    );
    app.world_mut()
        .get_mut::<AdmittedCommands>(receiver)
        .unwrap()
        .0[0]
        .payload = Control::SetPassiveEnabled { enabled: true };
    app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = 2;
    app.update();
    let state = app.world().get::<PassiveState>(receiver).unwrap();
    assert_eq!(state.0[&first].last_sample_tick, Some(1));
    assert_eq!(state.0[&second].last_sample_tick, Some(2));
    assert!(!state.0[&second].signals.is_empty());
    assert!(state.0[&second]
        .signals
        .iter()
        .all(|signal| signal.sample_id.starts_with("observer/receiver-two/2/")));
    let paid = app
        .world()
        .get::<ShipPowerSystem>(receiver)
        .unwrap()
        .0
        .capture_continuation();
    app.world_mut()
        .get_mut::<ShipSystemControlSources>(receiver)
        .unwrap()
        .0 = crate::ship::control_source::ControlSourceResolver::default();
    app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = 3;
    app.update();
    let boards = &app
        .world()
        .get::<crate::server_app::ShipSystemBlackboards>(receiver)
        .unwrap()
        .0;
    for owner in [&first, &second] {
        let Some(SystemBlackboard::Passive(board)) = boards.get(owner) else {
            panic!("actual passive publication")
        };
        assert!(!board.can_operate);
    }
    assert_eq!(
        app.world()
            .get::<ShipPowerSystem>(receiver)
            .unwrap()
            .0
            .capture_continuation(),
        paid
    );
}

#[test]
fn local_battery_work_emits_without_generation_and_samples_hide_source_identity() {
    let config = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    let mut app = App::new();
    let mut time = Time::<()>::default();
    time.advance_by(std::time::Duration::from_secs_f32(0.1));
    app.insert_resource(time)
        .insert_resource(crate::sim_tick::SimTick(1))
        .add_systems(Update, tick);
    let receiver = crate::entities::spawner::spawn_entity(
        &mut app.world_mut().commands(),
        &config,
        Vec3::ZERO,
        "observer".into(),
        None,
    );
    let source = crate::entities::spawner::spawn_entity(
        &mut app.world_mut().commands(),
        &config,
        Vec3::new(0.0, 0.0, -4.0),
        "secret-source".into(),
        None,
    );
    app.world_mut().flush();
    {
        let mut power = app.world_mut().get_mut::<ShipPowerSystem>(source).unwrap();
        let mut saved = power.0.capture_continuation();
        saved.network.generation_enabled = false;
        saved.network.output_actual_per_sec = 0.0;
        saved.network.output_target_per_sec = 0.0;
        power.0.restore_continuation(&saved).unwrap();
        let grant = power
            .0
            .work_continuous(
                &crate::core::messages::SystemId("helm-thrust".into()),
                0.1,
                1.0,
            )
            .unwrap();
        assert!(grant.granted > 0.0);
    }
    app.update();
    let owner = crate::core::messages::SystemId("passive-receiver".into());
    let all = app.world().get::<PassiveState>(receiver).unwrap();
    let state = &all.0[&owner];
    assert_eq!(state.signals.len(), 1);
    assert_eq!(state.signals[0].signature, PassiveSignature::Propulsion);
    assert_eq!(state.signals[0].sample_id, "observer/passive-receiver/1/0");
    assert!(!serde_json::to_string(state)
        .unwrap()
        .contains("secret-source"));
    state
        .validate(
            &app.world().get::<PassiveProfile>(receiver).unwrap().0[&owner],
            "observer",
            1,
        )
        .unwrap();
    let mut invalid = state.clone();
    invalid.last_tick = Some(2);
    assert!(invalid
        .validate(
            &app.world().get::<PassiveProfile>(receiver).unwrap().0[&owner],
            "observer",
            1
        )
        .is_err());
    app.world_mut()
        .get_mut::<PassiveProfile>(receiver)
        .unwrap()
        .0
        .get_mut(&owner)
        .unwrap()
        .min_signal_strength = 0.0;
    app.world_mut().spawn((
        EntityUuid("opaque-terrain".into()),
        Transform::from_xyz(0.0, 0.0, -2.0),
        crate::entities::spawner::RegionShapeSection(crate::regions::shape::RegionShape::Sphere {
            radius: 0.5,
        }),
        phoenix_model::field::SignalMaterialConfig {
            sensor_opaque: true,
            comms_opaque: false,
            sensor_absorption_per_unit: 0.0,
            comms_absorption_per_unit: 0.0,
        },
    ));
    app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = 100;
    app.update();
    assert_eq!(
        app.world().get::<PassiveState>(receiver).unwrap().0[&owner].last_sample_tick,
        Some(100)
    );
    assert!(app.world().get::<PassiveState>(receiver).unwrap().0[&owner]
        .signals
        .is_empty());
}
