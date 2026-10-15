use super::*;
use crate::console::navigation::sharing::{NavigationSharing, NavigationSharingProfile};
use phoenix_model::navigation_sharing::NavigationSharedKind;
use phoenix_sim_gameplay::science::navigation_sharing::{
    NavigationSamplePoint, NavigationSharingState,
};

fn fixture() -> (World, Entity, PhoenixSnapshot) {
    let config = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    let mut world = World::new();
    // The production snapshot plugin registers the legacy optional row type;
    // this renderer-free World otherwise has only the fitted physical owner.
    world.register_component::<EntitySystemHull>();
    world.insert_resource(SimTick(20));
    world.insert_resource(Time::<Fixed>::default());
    let entity = crate::entities::spawner::spawn_entity(
        &mut world.commands(),
        &config,
        Vec3::ZERO,
        "navigation-source".into(),
        None,
    );
    world.flush();
    let mut owner = world.get_mut::<NavigationSharing>(entity).unwrap();
    owner.0.share_own_faction = true;
    owner
        .0
        .sample(
            "navigation-source",
            20,
            [0.0; 2],
            &[NavigationSamplePoint {
                kind: NavigationSharedKind::Waypoint,
                position: [8.0, 9.0],
                observed_tick: None,
                observer_position: None,
            }],
        )
        .unwrap();
    let mut sender = owner.0.clone();
    sender.local.clear();
    sender
        .sample(
            "another-ship",
            20,
            [10.0; 2],
            &[NavigationSamplePoint {
                kind: NavigationSharedKind::Waypoint,
                position: [12.0, 13.0],
                observed_tick: None,
                observer_position: None,
            }],
        )
        .unwrap();
    owner
        .0
        .accept("navigation-source", 20, &sender.exports())
        .unwrap();
    drop(owner);
    let snapshot = capture(&world);
    assert_eq!(snapshot.entities.len(), 1);
    assert_eq!(
        snapshot.entities[0]
            .navigation_sharing
            .as_ref()
            .unwrap()
            .received
            .len(),
        1
    );
    (world, entity, snapshot)
}

#[test]
fn actual_spawn_capture_and_json_application_preserve_the_navigation_frontier() {
    let (mut world, entity, snapshot) = fixture();
    let decoded: PhoenixSnapshot =
        serde_json::from_slice(&serde_json::to_vec(&snapshot).unwrap()).unwrap();
    super::spatial_fields::validate_roster(&world, &decoded).unwrap();
    super::preflight::validate_entity_resources(&world, entity, &decoded.entities[0]).unwrap();
    let expected = decoded.entities[0]
        .navigation_sharing
        .as_ref()
        .unwrap()
        .clone();
    let authored = phoenix_model::sensing_sharing::RadarSharingConfig {
        system: expected.owner.clone(),
        share_own_faction: false,
        receive: true,
    };
    world
        .entity_mut(entity)
        .insert(NavigationSharing(NavigationSharingState::new(&authored)));
    super::application::apply_entity_state(&mut world, entity, &decoded.entities[0], decoded.tick);
    assert_eq!(world.get::<NavigationSharing>(entity).unwrap().0, expected);
    assert_eq!(
        capture(&world).entities[0].navigation_sharing,
        Some(expected)
    );
}

#[test]
fn missing_wrong_or_future_navigation_state_refuses_before_any_live_write() {
    for mutation in 0..5 {
        let (mut world, entity, mut snapshot) = fixture();
        match mutation {
            0 => snapshot.entities[0].navigation_sharing = None,
            1 => {
                snapshot.entities[0]
                    .navigation_sharing
                    .as_mut()
                    .unwrap()
                    .owner
                    .0 = "wrong-owner".into()
            }
            2 => {
                snapshot.entities[0]
                    .navigation_sharing
                    .as_mut()
                    .unwrap()
                    .last_tick = Some(21)
            }
            3 => {
                snapshot.entities[0]
                    .navigation_sharing
                    .as_mut()
                    .unwrap()
                    .local
                    .values_mut()
                    .next()
                    .unwrap()
                    .provenance
                    .origin
                    .source = "radar-origin-0000000000000001".into()
            }
            _ => {
                world
                    .entity_mut(entity)
                    .remove::<NavigationSharingProfile>();
            }
        }
        world.resource_mut::<SimTick>().0 = 7;
        world.spawn((
            EntityUuid("surplus-must-survive".into()),
            ShipRedAlert(false),
        ));
        snapshot.entities[0].red_alert = Some(true);
        let before = capture(&world);
        let report = restore(&mut world, &snapshot);
        assert!(!report.is_complete());
        assert!(
            matches!(report.gaps.as_slice(), [RestoreGap::ResourceStateInvalid { uuid, reason }]
            if uuid == "navigation-source" && reason.contains("Navigation"))
        );
        assert_eq!(report.entities_spawned, 0);
        assert_eq!(report.despawned, 0);
        assert_eq!(world.resource::<SimTick>().0, 7);
        assert_eq!(capture(&world), before);
    }
}

#[test]
fn navigation_digest_folds_intent_and_original_received_sample_frontiers() {
    let (mut world, entity, _) = fixture();
    let before = crate::sim_digest::world_digest(&world);
    world
        .get_mut::<NavigationSharing>(entity)
        .unwrap()
        .0
        .receive = false;
    let intent_changed = crate::sim_digest::world_digest(&world);
    assert_ne!(before, intent_changed);
    world
        .get_mut::<NavigationSharing>(entity)
        .unwrap()
        .0
        .received
        .values_mut()
        .next()
        .unwrap()
        .observation
        .position[0] += 1.0;
    assert_ne!(intent_changed, crate::sim_digest::world_digest(&world));
}
