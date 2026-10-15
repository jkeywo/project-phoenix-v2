use super::*;

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
    let state = app.world().get::<PassiveState>(receiver).unwrap();
    assert_eq!(state.signals.len(), 1);
    assert_eq!(state.signals[0].signature, PassiveSignature::Propulsion);
    assert_eq!(state.signals[0].sample_id, "observer/passive-receiver/1/0");
    assert!(!serde_json::to_string(state)
        .unwrap()
        .contains("secret-source"));
    state
        .validate(
            &app.world().get::<PassiveProfile>(receiver).unwrap().0,
            "observer",
            1,
        )
        .unwrap();
    let mut invalid = state.clone();
    invalid.last_tick = Some(2);
    assert!(invalid
        .validate(
            &app.world().get::<PassiveProfile>(receiver).unwrap().0,
            "observer",
            1
        )
        .is_err());
    app.world_mut()
        .get_mut::<PassiveProfile>(receiver)
        .unwrap()
        .0
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
        app.world()
            .get::<PassiveState>(receiver)
            .unwrap()
            .last_sample_tick,
        Some(100)
    );
    assert!(app
        .world()
        .get::<PassiveState>(receiver)
        .unwrap()
        .signals
        .is_empty());
}
