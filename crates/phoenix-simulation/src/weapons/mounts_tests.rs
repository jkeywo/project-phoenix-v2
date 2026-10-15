use super::*;

#[test]
fn aiming_uses_only_the_same_permitted_current_measurement_and_conceal_wins() {
    use crate::core::messages::{SensorRadarBlackboard, SystemBlackboard};
    use crate::science::broad::{BroadMeasurement, BroadProfile, BroadTrackState, BroadTracks};
    let authored = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    let config = authored.ship_config.as_ref().unwrap();
    let profile = BroadProfile::from_config(&authored).unwrap().unwrap();
    let instrument = SystemId("sensor-radar".into());
    let owner = SystemId("passive-receiver".into());
    let profile = &profile.0[&instrument].profile;
    let mut state = BroadTrackState::new(profile);
    state
        .advance(
            profile,
            0,
            0.5,
            1.0,
            &[BroadMeasurement {
                source: "private-actor".into(),
                selectable: true,
                observation: None,
                position: [40.0, 10.0],
                support: 1.0,
                identity: None,
                classification: None,
                assessment: None,
            }],
        )
        .unwrap();
    let public = state.projection(profile, 0, 0.5).unwrap();
    let contact = public[0].contact_id.clone();
    let mut boards = crate::server_app::ShipSystemBlackboards::default();
    boards.0.insert(
        instrument.clone(),
        SystemBlackboard::SensorRadar(SensorRadarBlackboard {
            tracks: Some(public),
            ..Default::default()
        }),
    );
    let tracks = BroadTracks(BTreeMap::from([(instrument, state)]));
    assert_eq!(
        current_point(config, &boards, &tracks, &owner, &contact, "observer", None),
        Some([40.0, 10.0])
    );
    assert!(measured_point(config, &boards, &tracks, &owner, "private-actor").is_none());
    assert!(measured_point(
        config,
        &boards,
        &tracks,
        &SystemId("unowned".into()),
        &contact
    )
    .is_none());
    let mut overrides = crate::gm::operations::contact::ContactOverrides::default();
    crate::gm::operations::contact::set(
        &mut overrides,
        "observer",
        "private-actor",
        crate::gm::operations::contact::ContactMode::Conceal,
    );
    assert!(current_point(
        config,
        &boards,
        &tracks,
        &owner,
        &contact,
        "observer",
        Some(&overrides)
    )
    .is_none());
    // A different observer's conceal cannot revoke this Console's picture.
    assert_eq!(
        current_point(
            config,
            &boards,
            &tracks,
            &owner,
            &contact,
            "another",
            Some(&overrides)
        ),
        Some([40.0, 10.0])
    );
}
fn profile() -> (
    SystemId,
    MountProfiles,
    ShipMounts,
    crate::ship::power::ShipPowerSystem,
) {
    let entity = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    let owner = SystemId("phaser-fore".into());
    let profiles = MountProfiles(BTreeMap::from([(
        owner.clone(),
        TurretConfig {
            facing_deg: 0.0,
            traverse_arc_deg: 270.0,
            traverse_deg_per_sec: 90.0,
            aim_tolerance_deg: 1.0,
        },
    )]));
    let states = ShipMounts::new(&profiles);
    let power = crate::modifiers::power_system::PowerSystem::from_energy(
        entity.energy.unwrap(),
        entity.ship_config.unwrap(),
    )
    .unwrap();
    (
        owner,
        profiles,
        states,
        crate::ship::power::ShipPowerSystem(power),
    )
}

#[test]
fn actual_local_reserve_caps_motion_replay_does_not_pay_and_unavailable_work_freezes() {
    let (owner, profiles, mut states, mut power) = profile();
    let rate = power
        .0
        .config()
        .consumers
        .iter()
        .find(|consumer| consumer.system == owner)
        .unwrap()
        .continuous_cost_per_sec;
    assert!(rate > 0.0);
    power
        .0
        .network_mut()
        .consumers
        .get_mut(&owner)
        .unwrap()
        .energy = rate * 0.1;
    advance_system(
        &profiles,
        &mut states,
        &owner,
        0,
        1.0,
        true,
        Some(90.0),
        1.0,
        &mut power,
    )
    .unwrap();
    assert!(states.0[&owner].offset_deg > 0.0 && states.0[&owner].offset_deg < 90.0);
    let paid = power.0.capture_continuation();
    let emitted = power.0.emitted_work(&owner);
    let before = states.clone();
    advance_system(
        &profiles,
        &mut states,
        &owner,
        0,
        100.0,
        true,
        Some(-90.0),
        1.0,
        &mut power,
    )
    .unwrap();
    assert_eq!(states, before);
    assert_eq!(power.0.capture_continuation(), paid);
    assert_eq!(power.0.emitted_work(&owner), emitted);
    power
        .0
        .network_mut()
        .consumers
        .get_mut(&owner)
        .unwrap()
        .energy = 0.0;
    advance_system(
        &profiles,
        &mut states,
        &owner,
        1,
        1.0,
        true,
        Some(90.0),
        1.0,
        &mut power,
    )
    .unwrap();
    assert_eq!(states.0[&owner].offset_deg, before.0[&owner].offset_deg);
    let paid = power.0.capture_continuation();
    advance_system(
        &profiles,
        &mut states,
        &owner,
        2,
        1.0,
        false,
        Some(90.0),
        1.0,
        &mut power,
    )
    .unwrap();
    advance_system(
        &profiles,
        &mut states,
        &owner,
        3,
        1.0,
        true,
        Some(90.0),
        0.0,
        &mut power,
    )
    .unwrap();
    assert_eq!(power.0.capture_continuation(), paid);
    assert_eq!(states.0[&owner].offset_deg, before.0[&owner].offset_deg);
}

#[test]
fn static_geometry_remains_equivalent_and_authored_missing_state_cannot_fire() {
    let (owner, profiles, states, _) = profile();
    let mount = MountConfig {
        facing_deg: 0.0,
        fire_arc_deg: 270.0,
        range: 100.0,
    };
    for target in [[0.0, -40.0], [40.0, 0.0], [0.0, 110.0]] {
        assert_eq!(
            geometry(None, None, &owner, &mount, target, [0.0, 0.0], 0.0),
            mount.geometry(target[0], target[1], 0.0, 0.0, 0.0)
        );
    }
    assert!(
        !geometry(
            Some(&profiles),
            None,
            &owner,
            &mount,
            [0.0, -40.0],
            [0.0, 0.0],
            0.0
        )
        .in_arc
    );
    assert!(
        geometry(
            Some(&profiles),
            Some(&states),
            &owner,
            &mount,
            [0.0, -40.0],
            [0.0, 0.0],
            0.0
        )
        .in_arc
    );
}

#[test]
fn sensor_field_of_view_uses_the_single_actual_bearing_instead_of_weapon_tolerance() {
    let (owner, profiles, mut states, _) = profile();
    let mount = MountConfig {
        facing_deg: 0.0,
        fire_arc_deg: 30.0,
        range: 100.0,
    };
    states
        .0
        .get_mut(&owner)
        .unwrap()
        .advance(&profiles.0[&owner], 0, 1.0, true, Some(90.0), 1.0)
        .unwrap();
    let rotated = sensor_mount(Some(&profiles), Some(&states), &owner, &mount).unwrap();
    assert_eq!(rotated.facing_deg, 90.0);
    assert_eq!(rotated.fire_arc_deg, 30.0);
    assert!(rotated.geometry(40.0, 0.0, 0.0, 0.0, 0.0).in_arc);
    assert!(!rotated.geometry(0.0, -40.0, 0.0, 0.0, 0.0).in_arc);
    assert!(sensor_mount(Some(&profiles), None, &owner, &mount).is_none());
}
