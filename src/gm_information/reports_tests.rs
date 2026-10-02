use super::*;
fn policy(delay_ticks: u32) -> ReportPolicy {
    ReportPolicy {
        delay_ticks,
        position_step_mm: 0,
        hide_identity: false,
    }
}
fn sample(tick: u64) -> Option<ReportSample> {
    Some(ReportSample {
        observed_tick: tick,
        name: "observed".into(),
        position_mm: [tick as i64, 0, 0],
    })
}
#[test]
fn delayed_report_withholds_initial_truth_and_releases_absence_at_the_next_interval() {
    let mut state = ReportState::new(policy(4));
    state.advance(10, || sample(10));
    assert!(state.presented.is_none());
    state.advance(13, || panic!("no sample between authored boundaries"));
    state.advance(14, || sample(14));
    assert_eq!(state.presented, sample(10));
    state.advance(18, || None);
    assert_eq!(state.presented, sample(14));
    state.advance(22, || None);
    assert!(state.presented.is_none());
    state.clear_samples();
    state.advance(25, || sample(25));
    assert!(state.presented.is_none());
}
#[test]
fn reconfiguring_clears_samples_but_identical_policy_keeps_pending_work_and_roundtrips() {
    let mut reports = Reports::new();
    assert!(set(&mut reports, "a", "b", Some(policy(4))));
    reports
        .get_mut("a")
        .unwrap()
        .get_mut("b")
        .unwrap()
        .advance(10, || sample(10));
    assert!(!set(&mut reports, "a", "b", Some(policy(4))));
    let saved = reports.clone();
    let codec = vellum_digest::ShareCodec::new("REPORT-TEST-");
    assert_eq!(
        codec
            .decode::<Reports>(&codec.encode(&saved).unwrap())
            .unwrap(),
        saved
    );
    assert!(set(&mut reports, "a", "b", Some(policy(8))));
    assert!(reports["a"]["b"].pending.is_none());
    assert!(projection(&reports, "other", 10).is_empty());
    assert!(set(&mut reports, "a", "b", None));
    assert!(reports.is_empty());
}
fn world() -> (World, Entity, Entity) {
    let mut world = World::new();
    world.insert_resource(WorldContentRuntime::default());
    world.insert_resource(crate::sim_tick::SimTick(10));
    let observer = world
        .spawn((
            EntityUuid("a".into()),
            crate::server_app::Ship,
            crate::lockstep::FleetSlotOf(crate::command_admission::HostSlot(1)),
            ShipPhysics::default(),
            SensorsObservationConfig(crate::radar_config::RadarConfig {
                range: 100.0,
                shows: vec![crate::entities::tags::EntityTag::Ship],
                selects: vec![],
            }),
        ))
        .id();
    let target = world
        .spawn((
            EntityUuid("b".into()),
            EntityName("true-name".into()),
            EntityTagsSection(vec!["ship".into()]),
            Transform::from_xyz(5000.0, 0.0, 0.0),
            ShipPhysics {
                x: 12.25,
                z: -9.75,
                ..Default::default()
            },
            RadarAppearanceSection(toml::from_str("icon = 'ship'").unwrap()),
        ))
        .id();
    (world, observer, target)
}
#[test]
fn capture_uses_own_radar_canonical_pose_and_observed_identity_with_conceal_floor() {
    let (mut world, observer, target) = world();
    let policy = ReportPolicy {
        position_step_mm: 5000,
        ..policy(2)
    };
    set(
        &mut world
            .resource_mut::<WorldContentRuntime>()
            .contact_information
            .reports,
        "a",
        "b",
        Some(policy),
    );
    advance(&mut world);
    assert!(world
        .resource::<WorldContentRuntime>()
        .contact_information
        .reports["a"]["b"]
        .presented
        .is_none());
    world.entity_mut(target).get_mut::<EntityName>().unwrap().0 = "new-name".into();
    world.resource_mut::<crate::sim_tick::SimTick>().0 = 12;
    advance(&mut world);
    let public = projection(
        &world
            .resource::<WorldContentRuntime>()
            .contact_information
            .reports,
        "a",
        13,
    );
    let row = public["b"].as_ref().unwrap();
    assert_eq!(row.name, "true-name");
    assert_eq!(row.position_mm, [10000, 0, -10000]);
    assert_eq!(row.age_ticks, 3);
    world
        .entity_mut(observer)
        .insert(crate::server_app::LocalShip); // presentation ownership cannot alter allowed capture
    gm_contact::set(
        &mut world
            .resource_mut::<WorldContentRuntime>()
            .contact_overrides,
        "a",
        "b",
        ContactMode::Conceal,
    );
    advance(&mut world);
    assert!(world
        .resource::<WorldContentRuntime>()
        .contact_information
        .reports["a"]["b"]
        .pending
        .is_none());
    gm_contact::set(
        &mut world
            .resource_mut::<WorldContentRuntime>()
            .contact_overrides,
        "a",
        "b",
        ContactMode::Normal,
    );
    world.entity_mut(target).get_mut::<ShipPhysics>().unwrap().x = 500.0;
    advance(&mut world);
    assert!(world
        .resource::<WorldContentRuntime>()
        .contact_information
        .reports["a"]["b"]
        .pending
        .is_none());
    gm_contact::set(
        &mut world
            .resource_mut::<WorldContentRuntime>()
            .contact_overrides,
        "a",
        "b",
        ContactMode::Reveal,
    );
    world.resource_mut::<crate::sim_tick::SimTick>().0 = 14;
    advance(&mut world);
    assert_eq!(
        world
            .resource::<WorldContentRuntime>()
            .contact_information
            .reports["a"]["b"]
            .pending
            .as_ref()
            .unwrap()
            .name,
        "console.sensors.basic_contact"
    );
    world.despawn(target);
    advance(&mut world);
    assert!(world
        .resource::<WorldContentRuntime>()
        .contact_information
        .reports
        .is_empty());
}
#[test]
fn native_projection_never_enriches_a_sample_with_current_truth_or_concealed_data() {
    let mut reports = Reports::new();
    set(&mut reports, "a", "b", Some(policy(0)));
    reports
        .get_mut("a")
        .unwrap()
        .get_mut("b")
        .unwrap()
        .advance(10, || sample(10));
    let truth = [EntitySnapshot {
        uuid: "b".into(),
        name: Some("secret".into()),
        position: Some([90.0, 0.0, 0.0]),
        tags: vec!["ship".into()],
        ..Default::default()
    }];
    let projected = viewscreen(
        &truth,
        &reports,
        &Default::default(),
        "a",
        12,
        0.0,
        0.0,
        100.0,
    );
    assert_eq!(projected[0].name.as_deref(), Some("observed"));
    assert_eq!(projected[0].position, Some([0.01, 0.0, 0.0]));
    assert!(!projected[0].tags.contains(&"ship".into()));
    let mut modes = Default::default();
    gm_contact::set(&mut modes, "a", "b", ContactMode::Conceal);
    assert!(viewscreen(&truth, &reports, &modes, "a", 12, 0.0, 0.0, 100.0).is_empty());
    assert_eq!(
        viewscreen(&truth, &reports, &modes, "other", 12, 0.0, 0.0, 100.0),
        truth
    );
}
