use super::*;

#[test]
fn actual_cruiser_rig_missing_listed_barrel_cannot_borrow_centre_or_buy_a_round() {
    let config = crate::entities::include_resolve::load_entity_config(
        "assets/entities/broadside_cruiser.toml",
    )
    .unwrap();
    let bank = config
        .weapons_console
        .as_ref()
        .unwrap()
        .blaster_banks
        .iter()
        .find(|bank| bank.id == "side")
        .unwrap()
        .to_runtime();
    let owner = crate::ship::system_registry::blaster_bank_system_id(&bank.id).unwrap();
    let mut rig = crate::entities::model_rig::parse_model_rig(include_str!(
        "../../../../../assets/models/alliance_cruiser_recreated.model.toml"
    ))
    .unwrap();
    assert!(rig.markers.remove(&bank.barrels[0]).is_some());
    let markers = ModelMarkers::from_rig(&rig);
    let transform = Transform::from_xyz(4.0, 0.0, 8.0)
        .with_rotation(Quat::from_rotation_y(std::f32::consts::FRAC_PI_2));
    let physics = ShipPhysics {
        x: 4.0,
        y: 6.0,
        z: 8.0,
        yaw: -std::f32::consts::FRAC_PI_2,
        ..Default::default()
    };
    let actual_pose =
        Transform::from_xyz(physics.x, physics.y, physics.z).with_rotation(transform.rotation);
    let expected = markers
        .resolve_world_position(&actual_pose, &bank.barrels[1])
        .unwrap();
    let origins = resolve_barrel_origins(&bank, Some(&markers), &transform, &physics);
    assert_eq!(origins, vec![None, Some(expected.to_array())]);
    let mut power = crate::modifiers::power_system::PowerSystem::from_energy(
        config.energy.unwrap(),
        config.ship_config.unwrap(),
    )
    .unwrap();
    let before = power.network().consumers[&owner].energy;
    let cost = power
        .config()
        .consumers
        .iter()
        .find(|consumer| consumer.system == owner)
        .unwrap()
        .discrete_cost;
    assert!(cost > 0.0);
    let mut system = crate::weapons::blaster::BlasterSystem::new(bank);
    assert!(system.request_charge_start());
    let mut attempts = Vec::new();
    let mut minted = 0;
    let events = system.tick_with_origins_3d(
        0.1,
        0.1,
        &origins,
        0.0,
        [20.0, 14.0, 0.0],
        [0.0; 3],
        "source",
        &mut |index| {
            attempts.push(index);
            power.try_work_discrete_count(&owner, 1).unwrap()
        },
        &mut || {
            minted += 1;
            format!("actual-{minted}")
        },
    );
    assert_eq!(attempts, vec![1]);
    assert_eq!(minted, 1);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].barrel, 1);
    assert_eq!([events[0].x, events[0].y, events[0].z], expected.to_array());
    assert!(events[0].pitch > 0.0);
    assert_eq!(power.network().consumers[&owner].energy, before - cost);
}

#[test]
fn only_an_implicit_unmarked_barrel_keeps_legacy_centre_and_missing_rigs_keep_indices() {
    let mut config = crate::weapons::blaster::BlasterBankConfig::default();
    let transform = Transform::IDENTITY;
    let physics = ShipPhysics {
        x: 7.0,
        y: 5.0,
        z: 9.0,
        ..Default::default()
    };
    assert_eq!(
        resolve_barrel_origins(&config, None, &transform, &physics),
        vec![Some([7.0, 5.0, 9.0])]
    );
    config.marker = Some("missing".into());
    assert_eq!(
        resolve_barrel_origins(&config, None, &transform, &physics),
        vec![None]
    );
    config.barrels = vec!["one".into(), "two".into()];
    assert_eq!(
        resolve_barrel_origins(&config, None, &transform, &physics),
        vec![None, None]
    );
}
