use super::*;
use bevy::ecs::system::RunSystemOnce;
use phoenix_model::{messages::HelmBlackboard, sensing_tracks::TrackIdentity};

fn objective(target: &str) -> crate::core::messages::ScoredObjective {
    use crate::core::messages::*;
    ScoredObjective {
        id: "known-order".into(),
        score: 1.0,
        directive: AiDirective::Destroy {
            target: target.into(),
        },
        source: ObjectiveSource::Mission,
        relevance: vec![SystemAffinity::Helm],
        snapshot: ObjectiveSnapshot {
            progress: None,
            unassigned: false,
            id: "known-order".into(),
            text: "known-order".into(),
            text_params: Default::default(),
            mandatory: true,
            status: ObjectiveStatus::Active,
            targets: vec![target.into()],
            source: ObjectiveSource::Mission,
        },
    }
}

fn rig() -> (World, Entity, crate::core::messages::SystemId, uuid::Uuid) {
    use crate::core::messages::*;
    use crate::science::broad::*;
    let authored = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    let mut config = authored.ship_config.clone().unwrap();
    // Keep the real Console topology but omit cross-radar grants so a Science
    // publication cannot become common Helm knowledge just by sharing the ship.
    for console in &mut config.consoles {
        console.feeds.retain(|feed| {
            !matches!(
                feed.kind,
                phoenix_model::console::ConsoleObservationKind::SensorRadar
                    | phoenix_model::console::ConsoleObservationKind::TacticalRadar
            )
        });
    }
    let profiles = BroadProfile::from_config(&authored).unwrap().unwrap();
    let owner = crate::ship::system_registry::helm_radar_system_id();
    let profile = &profiles.0[&owner].profile;
    let actual = uuid::Uuid::from_u128(123);
    let mut state = BroadTrackState::new(profile);
    state
        .advance(
            profile,
            1,
            0.5,
            1.0,
            &[BroadMeasurement {
                elevation: None,
                source: actual.to_string(),
                observation: None,
                selectable: true,
                position: [12.0, 34.0],
                support: 1.0,
                identity: None,
                classification: None,
                assessment: None,
            }],
        )
        .unwrap();
    let observed = state.projection(profile, 1, 0.5).unwrap();
    let mut boards = crate::server_app::ShipSystemBlackboards::default();
    boards.0.insert(
        owner.clone(),
        SystemBlackboard::Helm(HelmBlackboard {
            movement: None,
            tracks: Some(observed.clone()),
            ..Default::default()
        }),
    );
    boards.0.insert(
        crate::ship::system_registry::sensor_radar_system_id(),
        SystemBlackboard::SensorRadar(SensorRadarBlackboard {
            tracks: Some(vec![phoenix_model::sensing_tracks::SensorTrackSnapshot {
                elevation: None,
                contact_id: "private-science-return".into(),
                position: [88.0, 99.0],
                ..observed[0].clone()
            }]),
            ..Default::default()
        }),
    );
    boards.0.insert(
        crate::ship::system_registry::viewscreen_system_id(),
        SystemBlackboard::Viewscreen(ViewscreenBlackboard {
            combat_lock: Some(actual.to_string()),
            scored_objectives: vec![objective("")],
            ..Default::default()
        }),
    );
    let mut sources = crate::ship::control_source::ControlSourceResolver::new();
    for system in &config.systems {
        sources.set(
            system.id.clone(),
            crate::ship::control_source::ControlSource::Ai,
        );
    }
    let mut world = World::new();
    world.insert_resource(HelmAiSurfacesFrame::default());
    world.insert_resource(crate::ai::server::WorldSnapshot {
        entities: vec![crate::ai::AiWorldEntity {
            uuid: actual,
            name: Some("unseen-secret-name".into()),
            position: [900.0, 400.0, 999.0],
            hull_fraction: Some(0.1),
            yaw: Some(2.0),
            forward_speed: 300.0,
            direct_fire_range: 1000.0,
            weapon_arcs: vec![crate::weapons::arc_geometry::WeaponArcSector {
                bearing_deg: 0.0,
                half_angle_deg: 180.0,
                range: 1000.0,
            }],
            ..Default::default()
        }],
        ..Default::default()
    });
    let observer = world
        .spawn((
            crate::ship::components::ShipConfigComponent(config),
            crate::ship::components::ShipSystemControlSources(sources),
            ShipPhysics::default(),
            boards,
            crate::entities::spawner::EntityUuid(uuid::Uuid::from_u128(1).to_string()),
            crate::ai::server::AiHighFidelity,
            SensingControls(authored.sensing.as_ref().unwrap().control.clone()),
            profiles,
            BroadTracks(BTreeMap::from([(owner.clone(), state)])),
        ))
        .id();
    (world, observer, owner, actual)
}
fn frame(world: &mut World, entity: Entity) -> HelmAiShipFrame {
    world.run_system_once(build_helm_ai_surfaces_frame).unwrap();
    world.resource::<HelmAiSurfacesFrame>().ships[&entity].clone()
}

#[test]
fn actual_helm_frame_uses_permitted_measured_pose_and_never_fills_foreign_facts_from_truth() {
    let (mut world, observer, owner, actual) = rig();
    let crate::core::messages::SystemBlackboard::Helm(issued) = &world
        .get::<crate::server_app::ShipSystemBlackboards>(observer)
        .unwrap()
        .0[&owner]
    else {
        panic!()
    };
    let issued_position = issued.tracks.as_ref().unwrap()[0].position;
    let first = frame(&mut world, observer);
    assert_eq!(
        first.visible_view.entities.len(),
        1,
        "Science-only return is not a Helm feed"
    );
    let target = &first.visible_view.entities[0];
    assert_eq!(
        target.position,
        [issued_position[0], 0.0, issued_position[1]]
    );
    assert_ne!(target.uuid, actual);
    assert!(
        target.dangerous,
        "current measured obstacles retain avoidance"
    );
    assert_eq!(target.name, None);
    assert_eq!(target.hull_fraction, None);
    assert_eq!(target.yaw, None);
    assert!(target.shields.is_none() && target.weapon_arcs.is_empty());
    assert_eq!(target.forward_speed, 0.0);
    assert_eq!(target.direct_fire_range, 0.0);
    assert_eq!(first.weapons_target, Some(target.uuid));
    assert_eq!(
        first.current_contact_routes.as_ref().unwrap()[&actual],
        target.uuid
    );
    let hidden = &mut world
        .resource_mut::<crate::ai::server::WorldSnapshot>()
        .entities[0];
    hidden.position = [-900.0, 10.0, -999.0];
    hidden.name = Some("another-secret".into());
    hidden.hull_fraction = Some(0.9);
    let next = frame(&mut world, observer);
    assert_eq!(next.merged_view.entities, first.merged_view.entities);
    assert_eq!(next.weapons_target, first.weapons_target);
    assert_eq!(next.current_contact_routes, first.current_contact_routes);
}

#[test]
fn predicted_and_missing_helm_scope_never_reacquire_the_real_actor() {
    use crate::science::broad::*;
    let (mut world, observer, owner, actual) = rig();
    let profile = world.get::<BroadProfile>(observer).unwrap().0[&owner]
        .profile
        .clone();
    let mut tracks = world.get_mut::<BroadTracks>(observer).unwrap();
    tracks
        .0
        .get_mut(&owner)
        .unwrap()
        .advance(&profile, 2, 0.5, 1.0, &[])
        .unwrap();
    let mut held = tracks.0[&owner].projection(&profile, 2, 0.5).unwrap();
    held[0].position_error = 500.0;
    assert!(held[0].predicted);
    let expected = held[0].position;
    let contact = held[0].contact_id.clone();
    world
        .get_mut::<crate::server_app::ShipSystemBlackboards>(observer)
        .unwrap()
        .0
        .insert(
            owner.clone(),
            crate::core::messages::SystemBlackboard::Helm(HelmBlackboard {
                movement: None,
                tracks: Some(held),
                ..Default::default()
            }),
        );
    let predicted = frame(&mut world, observer);
    assert_eq!(
        predicted.visible_view.entities[0].position,
        [expected[0], 0.0, expected[1]]
    );
    let history = &predicted.visible_view.entities[0];
    assert!(!history.dangerous, "lost history is not a solid obstacle");
    assert_eq!(
        history.radius, 500.0,
        "uncertainty remains in the knowledge view"
    );
    assert_eq!(
        crate::ai::avoidance_steering(
            [0.0; 3],
            0.0,
            5.0,
            1.5,
            uuid::Uuid::nil(),
            &predicted.visible_view.entities,
            crate::ai::AVOIDANCE_BUFFER,
            crate::ai::AVOIDANCE_LOOK_AHEAD_SECS,
            crate::ai::HAZARD_THREAT_EXPONENT,
        ),
        0.0
    );
    assert_eq!(
        crate::ai::assess_hazards(
            &predicted.visible_view,
            5.0,
            crate::ai::AVOIDANCE_BUFFER,
            crate::ai::AVOIDANCE_LOOK_AHEAD_SECS,
            crate::ai::HAZARD_IGNORE_SIZE_RATIO,
            crate::ai::HAZARD_THREAT_EXPONENT,
        )
        .urgency,
        0.0
    );
    assert_eq!(predicted.weapons_target, None);
    assert!(!predicted
        .current_contact_routes
        .as_ref()
        .unwrap()
        .contains_key(&actual));
    let scoped = shared_helm_observations(
        Some(
            &world
                .get::<crate::ship::components::ShipConfigComponent>(observer)
                .unwrap()
                .0,
        ),
        world.get::<ShipSystemControlSources>(observer).unwrap(),
        world
            .get::<crate::server_app::ShipSystemBlackboards>(observer)
            .unwrap(),
    );
    let context = build(
        &scoped,
        world.get::<BroadTracks>(observer),
        world.get::<SensingControls>(observer),
        &ShipPhysics::default(),
        None,
        None,
        None,
        &Default::default(),
        None,
        None,
    );
    assert_eq!(
        context.selection(Some(&contact)),
        None,
        "a predicted handle is not a current actuator grant"
    );
    world
        .get_mut::<crate::server_app::ShipSystemBlackboards>(observer)
        .unwrap()
        .0
        .remove(&owner);
    let absent = frame(&mut world, observer);
    assert!(absent.visible_view.entities.is_empty() && absent.merged_view.entities.is_empty());
    assert_eq!(absent.current_contact_routes, Some(BTreeMap::new()));
}

#[test]
fn public_chart_geometry_and_concealment_are_kept_without_charting_actor_condition() {
    let (mut world, observer, _, _) = rig();
    let charted = uuid::Uuid::from_u128(44).to_string();
    world.insert_resource(crate::lobby::server::WorldResource(
        crate::core::messages::WorldData {
            entities: vec![crate::core::messages::EntitySnapshot {
                uuid: charted.clone(),
                name: Some("public-landmark".into()),
                position: Some([500.0, 0.0, 10.0]),
                radius: Some(5.0),
                tags: vec![crate::console_delivery::CHARTED_TAG.into()],
                hull_fraction: Some(0.2),
                yaw: Some(2.5),
                ..Default::default()
            }],
            ..Default::default()
        },
    ));
    let visible = frame(&mut world, observer);
    let landmark = visible
        .visible_view
        .entities
        .iter()
        .find(|entity| entity.name.as_deref() == Some("public-landmark"))
        .unwrap();
    assert_eq!(landmark.position, [500.0, 0.0, 10.0]);
    assert_eq!(landmark.radius, 5.0);
    assert!(
        landmark.yaw.is_none()
            && landmark.hull_fraction.is_none()
            && landmark.weapon_arcs.is_empty()
    );
    world.insert_resource(crate::world::server::WorldContentRuntime::default());
    crate::gm::operations::contact::set(
        &mut world
            .resource_mut::<crate::world::server::WorldContentRuntime>()
            .contact_overrides,
        &uuid::Uuid::from_u128(1).to_string(),
        &charted,
        crate::gm::operations::contact::ContactMode::Conceal,
    );
    assert_eq!(frame(&mut world, observer).visible_view.entities.len(), 1);
}

#[test]
fn derived_contact_keys_and_permitted_identity_survive_existing_track_json_without_shadow_state() {
    let (mut world, observer, owner, actual) = rig();
    let mut boards = world
        .get_mut::<crate::server_app::ShipSystemBlackboards>(observer)
        .unwrap();
    let crate::core::messages::SystemBlackboard::Helm(board) = boards.0.get_mut(&owner).unwrap()
    else {
        unreachable!()
    };
    board.tracks.as_mut().unwrap()[0].identity = Some(TrackIdentity {
        name: "observed-name".into(),
        faction: None,
        references: vec!["known-reference".into()],
    });
    let first = frame(&mut world, observer);
    let saved = serde_json::to_vec(
        &world
            .get::<crate::science::broad::BroadTracks>(observer)
            .unwrap()
            .0,
    )
    .unwrap();
    world
        .entity_mut(observer)
        .insert(crate::science::broad::BroadTracks(
            serde_json::from_slice(&saved).unwrap(),
        ));
    let next = frame(&mut world, observer);
    assert_eq!(first.merged_view.entities, next.merged_view.entities);
    assert_eq!(first.current_contact_routes, next.current_contact_routes);
    let scoped = shared_helm_observations(
        Some(
            &world
                .get::<crate::ship::components::ShipConfigComponent>(observer)
                .unwrap()
                .0,
        ),
        world.get::<ShipSystemControlSources>(observer).unwrap(),
        world
            .get::<crate::server_app::ShipSystemBlackboards>(observer)
            .unwrap(),
    );
    let context = build(
        &scoped,
        world.get::<crate::science::broad::BroadTracks>(observer),
        world.get::<crate::science::broad::SensingControls>(observer),
        &ShipPhysics::default(),
        None,
        None,
        None,
        &Default::default(),
        None,
        None,
    );
    assert_eq!(
        context.named_target("known-reference"),
        Some(first.merged_view.entities[0].uuid)
    );
    assert_eq!(
        context.named_target(&actual.to_string()),
        None,
        "a physical routing UUID does not identify a measured return"
    );
    let known = first.merged_view.entities[0].uuid;
    assert_ne!(known, actual);
    assert_eq!(
        first.merged_view.entities[0].name.as_deref(),
        Some("observed-name")
    );
}

#[test]
fn identical_explicit_feed_copies_collapse_but_contradictory_records_are_withheld() {
    let (world, observer, owner, actual) = rig();
    let mut boards = world
        .get::<crate::server_app::ShipSystemBlackboards>(observer)
        .unwrap()
        .clone();
    let crate::core::messages::SystemBlackboard::Helm(board) = &boards.0[&owner] else {
        panic!()
    };
    let records = board.tracks.clone().unwrap();
    let expected_key = contact_key(&records[0].contact_id);
    let copy_owner = crate::ship::system_registry::tactical_radar_system_id();
    boards.0.insert(
        copy_owner.clone(),
        crate::core::messages::SystemBlackboard::TacticalRadar(
            crate::core::messages::TacticalRadarBlackboard {
                tracks: Some(records),
                ..Default::default()
            },
        ),
    );
    let context = build(
        &boards,
        world.get::<crate::science::broad::BroadTracks>(observer),
        world.get::<crate::science::broad::SensingControls>(observer),
        &ShipPhysics::default(),
        None,
        None,
        None,
        &Default::default(),
        None,
        None,
    );
    assert_eq!(
        context.view.entities.len(),
        2,
        "one held duplicate plus independent Science return"
    );
    assert_eq!(
        context.selection(Some(&actual.to_string())),
        Some(expected_key)
    );
    let crate::core::messages::SystemBlackboard::TacticalRadar(board) =
        boards.0.get_mut(&copy_owner).unwrap()
    else {
        panic!()
    };
    board.tracks.as_mut().unwrap()[0].position = [900.0, 999.0];
    let conflict = build(
        &boards,
        world.get::<crate::science::broad::BroadTracks>(observer),
        world.get::<crate::science::broad::SensingControls>(observer),
        &ShipPhysics::default(),
        None,
        None,
        None,
        &Default::default(),
        None,
        None,
    );
    assert_eq!(
        conflict.view.entities.len(),
        1,
        "conflicting duplicate cannot prefer source order"
    );
    assert_eq!(conflict.selection(Some(&actual.to_string())), None);
}

#[test]
fn current_held_board_cannot_bypass_new_conceal_or_missing_observer_scope() {
    let (mut world, observer, _, actual) = rig();
    assert_eq!(frame(&mut world, observer).visible_view.entities.len(), 1);
    let mut runtime = crate::world::server::WorldContentRuntime::default();
    crate::gm::operations::contact::set(
        &mut runtime.contact_overrides,
        &uuid::Uuid::from_u128(1).to_string(),
        &actual.to_string(),
        crate::gm::operations::contact::ContactMode::Conceal,
    );
    world.insert_resource(runtime);
    let hidden = frame(&mut world, observer);
    assert!(hidden.merged_view.entities.is_empty());
    assert!(hidden.current_contact_routes.unwrap().is_empty());
    world
        .entity_mut(observer)
        .remove::<crate::entities::spawner::EntityUuid>();
    let missing = frame(&mut world, observer);
    assert!(missing.merged_view.entities.is_empty());
    assert_eq!(missing.current_contact_routes, Some(BTreeMap::new()));
}

#[path = "surfaces_borrowed_tests.rs"]
mod borrowed_input_tests;

fn pursuit_fixture() -> (
    phoenix_model::sensing_tracks::SensorTrackSnapshot,
    crate::ai::faction::FactionRegistry,
) {
    let (world, observer, owner, _) = rig();
    let boards = world
        .get::<crate::server_app::ShipSystemBlackboards>(observer)
        .unwrap();
    let crate::core::messages::SystemBlackboard::Helm(board) = &boards.0[&owner] else {
        panic!()
    };
    let mut track = board.tracks.as_ref().unwrap()[0].clone();
    track.identity = Some(TrackIdentity {
        name: "same-visible-name".into(),
        faction: Some(uuid::Uuid::from_u128(2).to_string()),
        references: vec![],
    });
    let mut registry = crate::ai::faction::FactionRegistry::default();
    registry.insert(crate::ai::faction::FactionConfig {
        uuid: uuid::Uuid::from_u128(1),
        name: "observer".into(),
        display_name: None,
        enemies: vec![uuid::Uuid::from_u128(2)],
        compliance: None,
    });
    (track, registry)
}

fn pursuit_picture(
    tracks: Vec<phoenix_model::sensing_tracks::SensorTrackSnapshot>,
) -> MeasuredHelm {
    let mut boards = crate::server_app::ShipSystemBlackboards::default();
    boards.0.insert(
        crate::ship::system_registry::helm_radar_system_id(),
        crate::core::messages::SystemBlackboard::Helm(HelmBlackboard {
            tracks: Some(tracks),
            ..Default::default()
        }),
    );
    build(
        &boards,
        None,
        None,
        &ShipPhysics::default(),
        None,
        Some(&crate::entities::spawner::FactionComponent(
            uuid::Uuid::from_u128(1),
        )),
        None,
        &Default::default(),
        None,
        None,
    )
}

#[test]
fn measured_pursuit_prefers_current_then_newest_history_without_fusing_named_contacts() {
    let (mut old, registry) = pursuit_fixture();
    old.contact_id = "old-west".into();
    old.position = [-260.0, 0.0];
    old.observed_tick = 2820;
    old.predicted = true;
    old.selectable = false;
    let mut recent = old.clone();
    recent.contact_id = "recent-east".into();
    recent.position = [313.0, 0.0];
    recent.observed_tick = 4590;
    let mut picture = pursuit_picture(vec![old.clone(), recent.clone()]);
    assert_eq!(
        picture.destroy_target(&[objective("")], &[], &registry),
        Some(contact_key("recent-east"))
    );
    assert_eq!(
        picture.selection(Some("recent-east")),
        None,
        "history never grants actuator selection"
    );
    assert_eq!(
        picture.named_target("same-visible-name"),
        None,
        "matching names do not fuse distinct contacts"
    );
    assert_eq!(
        picture.destroy_target(&[objective("old-west")], &[], &registry),
        Some(contact_key("old-west")),
        "explicit historical navigation target remains explicit"
    );

    old.predicted = false; // Fresh information-only observation, not selectable.
    old.position = [-500.0, 0.0];
    picture = pursuit_picture(vec![old.clone(), recent.clone()]);
    assert_eq!(
        picture.destroy_target(&[objective("")], &[], &registry),
        Some(contact_key("old-west")),
        "fresh information-only movement beats nearer history"
    );
    assert_eq!(picture.selection(Some("old-west")), None);

    recent.predicted = false;
    recent.selectable = true;
    recent.position = [700.0, 0.0];
    picture = pursuit_picture(vec![old, recent]);
    assert_eq!(
        picture.destroy_target(&[objective("")], &[], &registry),
        Some(contact_key("old-west")),
        "nearest fresh observation wins autonomously"
    );
    let selected = picture.selection(Some("recent-east")).unwrap();
    assert_eq!(
        picture.destroy_target(&[objective("")], &[selected], &registry),
        Some(selected),
        "current explicit choice wins over autonomous acquisition"
    );
}

#[test]
fn measured_pursuit_filters_hostility_and_breaks_history_ties_independently_of_input_order() {
    let (mut a, registry) = pursuit_fixture();
    a.contact_id = "a".into();
    a.position = [-50.0, 0.0];
    a.predicted = true;
    a.selectable = false;
    a.observed_tick = 100;
    let mut b = a.clone();
    b.contact_id = "b".into();
    b.position = [50.0, 0.0];
    let mut friend = a.clone();
    friend.contact_id = "friend".into();
    friend.position = [1.0, 0.0];
    friend.predicted = false;
    friend.identity.as_mut().unwrap().faction = Some(uuid::Uuid::from_u128(1).to_string());
    let mut unknown = friend.clone();
    unknown.contact_id = "unknown".into();
    unknown.identity = None;
    let expected = contact_key("a").min(contact_key("b"));
    let picture = pursuit_picture(vec![friend.clone(), a.clone(), unknown.clone(), b.clone()]);
    assert_eq!(
        picture.destroy_target(
            &[objective("")],
            &[contact_key("friend"), contact_key("unknown")],
            &registry
        ),
        Some(expected)
    );
    let reversed = pursuit_picture(vec![b.clone(), unknown, a.clone(), friend]);
    assert_eq!(
        reversed.destroy_target(&[objective("")], &[], &registry),
        Some(expected)
    );
    b.position = [20.0, 0.0];
    let nearer = pursuit_picture(vec![a, b]);
    assert_eq!(
        nearer.destroy_target(&[objective("")], &[], &registry),
        Some(contact_key("b")),
        "equal-age history uses distance before key"
    );
}
