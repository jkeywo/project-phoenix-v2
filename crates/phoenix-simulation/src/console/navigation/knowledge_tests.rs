use super::*;
use crate::entities::spawner::*;
use crate::science::broad::*;
use bevy::ecs::system::{RunSystemOnce, SystemState};

const TARGET: &str = "00000000-0000-0000-0000-000000000771";
fn rig() -> (App, Entity, Entity) {
    rig_for("assets/entities/alliance_cruiser.toml", true)
}
fn rig_for(path: &str, sensor_feed: bool) -> (App, Entity, Entity) {
    let mut config = crate::entities::include_resolve::load_entity_config(path).unwrap();
    let ship = config.ship_config.as_mut().unwrap();
    let console = ship
        .system(&SystemId("navigation".into()))
        .unwrap()
        .console
        .clone()
        .unwrap();
    let feeds = &mut ship
        .consoles
        .iter_mut()
        .find(|row| row.id == console)
        .unwrap()
        .feeds;
    if sensor_feed && !feeds.iter().any(|feed| feed.source.0 == "sensor-radar") {
        feeds.push(phoenix_model::console::ConsoleFeed {
            source: SystemId("sensor-radar".into()),
            kind: phoenix_model::console::ConsoleObservationKind::SensorRadar,
        });
    }
    let mut app = App::new();
    app.insert_resource(Time::<Fixed>::from_seconds(0.5))
        .insert_resource(crate::sim_tick::SimTick(0))
        .init_resource::<crate::world::server::WorldContentRuntime>()
        .init_resource::<crate::effect_queue::EffectQueue<crate::civilian::server::PendingCivilianOrder>>();
    let observer = crate::entities::spawner::spawn_entity(
        &mut app.world_mut().commands(),
        &config,
        Vec3::ZERO,
        "observer".into(),
        None,
    );
    app.world_mut().flush();
    app.world_mut()
        .entity_mut(observer)
        .insert((crate::server_app::Ship, NavigationWaypoint::default()));
    let target = app
        .world_mut()
        .spawn((
            EntityUuid(TARGET.into()),
            EntityName("held-public-name".into()),
            EntityTagsSection(vec!["ship".into()]),
            EntityTarget(crate::entities::target::TargetSection {
                tags: vec!["ship".into()],
                threat_level: Default::default(),
                description: None,
            }),
            Transform::from_xyz(10.0, 0.0, 3.0),
        ))
        .id();
    (app, observer, target)
}

/// Real existing reserve grant and paid producer, then its ordinary DTO. No
/// contact/current-route state is installed by the fixture.
pub(super) fn observe(app: &mut App, observer: Entity, identified: bool) -> String {
    observe_with(app, observer, identified, "sensor-radar")
}
fn observe_with(app: &mut App, observer: Entity, identified: bool, instrument: &str) -> String {
    let owner = SystemId(instrument.into());
    let profile = app.world().get::<BroadProfile>(observer).unwrap().0[&owner]
        .profile
        .clone();
    for _ in 0..40 {
        app.world_mut()
            .get_mut::<crate::ship::power::ShipPowerSystem>(observer)
            .unwrap()
            .0
            .work_continuous(&owner, 0.5, 1.0)
            .unwrap();
        app.world_mut()
            .run_system_once(crate::science::broad::tick)
            .unwrap();
        let tick = app.world().resource::<crate::sim_tick::SimTick>().0;
        let state = &app.world().get::<BroadTracks>(observer).unwrap().0[&owner];
        let records = state.projection(&profile, tick, 0.5).unwrap();
        if let Some(record) = records
            .iter()
            .find(|row| !identified || row.identity.is_some())
        {
            let id = record.contact_id.clone();
            app.world_mut()
                .get_mut::<crate::server_app::ShipSystemBlackboards>(observer)
                .unwrap()
                .0
                .insert(
                    owner,
                    if instrument == "tactical-radar" {
                        SystemBlackboard::TacticalRadar(
                            crate::core::messages::TacticalRadarBlackboard {
                                tracks: Some(records),
                                ..Default::default()
                            },
                        )
                    } else {
                        SystemBlackboard::SensorRadar(
                            crate::core::messages::SensorRadarBlackboard {
                                tracks: Some(records),
                                ..Default::default()
                            },
                        )
                    },
                );
            return id;
        }
        app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 += 1;
    }
    panic!("finite paid radar did not acquire the required observation");
}

fn picture(world: &mut World, observer: Entity) -> super::super::knowledge::Picture {
    let mut state = SystemState::<super::super::knowledge::Environment>::new(world);
    state.get(world).picture(
        observer,
        world.get::<crate::ship_plugin::ShipConfigComponent>(observer),
        &SystemId("navigation".into()),
        world.get::<crate::server_app::ShipSystemBlackboards>(observer),
        world.get::<sharing::NavigationSharingProfile>(observer),
    )
}
fn reserve(world: &World, ship: Entity) -> f32 {
    world
        .get::<crate::ship::power::ShipPowerSystem>(ship)
        .unwrap()
        .0
        .network()
        .consumers[&SystemId("navigation".into())]
        .energy
}
fn send(world: &mut World, ship: Entity, target: &str) {
    world.get_mut::<AdmittedCommands>(ship).unwrap().0 =
        vec![crate::core::messages::AdmittedCommand {
            target: SystemId("navigation".into()),
            payload: SystemControlPayload::OrderCivilian {
                target: target.into(),
                order: crate::civilian::CivilianOrder::Hold,
            },
            response_token: None,
            feedback_correlation: None,
        }];
}
fn traffic(world: &mut World, target: Entity) {
    let mut config = crate::civilian::CivilianConfig::default();
    config
        .order_options
        .push(crate::civilian::traffic::CivilianOrderOption {
            id: "hold".into(),
            label: "public.hold".into(),
            order: crate::civilian::CivilianOrder::Hold,
        });
    world.entity_mut(target).insert((
        crate::civilian::CivilianTraffic(crate::civilian::CivilianState::from_config(&config)),
        crate::civilian::CivilianSection(config),
        BehaviourSection(crate::entities::config::BehaviourConfig::default()),
    ));
}

#[test]
fn current_measured_anchor_holds_on_scope_loss_conceal_and_actor_despawn() {
    let (mut app, observer, target) = rig();
    let handle = observe(&mut app, observer, true);
    let measured = picture(app.world_mut(), observer).contacts[&handle].position;
    app.world_mut()
        .get_mut::<AdmittedCommands>(observer)
        .unwrap()
        .0 = vec![crate::core::messages::AdmittedCommand {
        target: SystemId("navigation".into()),
        payload: SystemControlPayload::SetNavigationWaypoint {
            x: 999.0,
            z: 888.0,
            source_uuid: Some(handle),
        },
        response_token: None,
        feedback_correlation: None,
    }];
    app.world_mut()
        .run_system_once(handle_navigation_waypoint)
        .unwrap();
    let point = app
        .world()
        .get::<NavigationWaypoint>(observer)
        .unwrap()
        .snapshot()
        .unwrap();
    assert_eq!([point.x, point.z], [measured[0], measured[2]]);
    app.world_mut()
        .get_mut::<Transform>(target)
        .unwrap()
        .translation = Vec3::new(9000.0, 0.0, 8000.0);
    app.world_mut()
        .run_system_once(refresh_anchored_waypoint)
        .unwrap();
    assert_eq!(
        app.world()
            .get::<NavigationWaypoint>(observer)
            .unwrap()
            .snapshot(),
        Some(point.clone())
    );
    app.world_mut()
        .resource_mut::<crate::world::server::WorldContentRuntime>()
        .contact_overrides
        .entry("observer".into())
        .or_default()
        .insert(
            TARGET.into(),
            crate::gm::operations::contact::ContactMode::Conceal,
        );
    assert!(picture(app.world_mut(), observer).contacts.is_empty());
    app.world_mut()
        .get_mut::<crate::server_app::ShipSystemBlackboards>(observer)
        .unwrap()
        .0
        .clear();
    app.world_mut().despawn(target);
    app.world_mut()
        .run_system_once(refresh_anchored_waypoint)
        .unwrap();
    assert_eq!(
        app.world()
            .get::<NavigationWaypoint>(observer)
            .unwrap()
            .snapshot(),
        Some(point)
    );
}

#[test]
fn names_are_public_evidence_and_hidden_actor_names_never_resolve() {
    let (mut app, observer, _) = rig();
    let handle = observe(&mut app, observer, false);
    let unknown = picture(app.world_mut(), observer);
    assert!(unknown.request("held-public-name").is_none());
    assert!(unknown.request(TARGET).is_none());
    assert!(unknown.request(&handle).is_some());
    observe(&mut app, observer, true);
    assert!(picture(app.world_mut(), observer)
        .request("held-public-name")
        .is_some());
}

#[test]
fn fitted_order_ingress_refuses_truth_then_records_only_its_successful_paid_request() {
    let (mut app, observer, target) = rig();
    traffic(app.world_mut(), target);
    let before = reserve(app.world(), observer);
    send(app.world_mut(), observer, TARGET);
    app.world_mut()
        .run_system_once(crate::civilian::tick_civilian_traffic)
        .unwrap();
    assert_eq!(reserve(app.world(), observer), before);
    assert!(app
        .world()
        .get::<NavigationWaypoint>(observer)
        .unwrap()
        .accepted_requests
        .is_empty());
    let handle = observe(&mut app, observer, true);
    send(app.world_mut(), observer, &handle);
    app.world_mut()
        .run_system_once(crate::civilian::tick_civilian_traffic)
        .unwrap();
    assert_eq!(reserve(app.world(), observer), before - 1.0);
    let receipt = app
        .world()
        .get::<NavigationWaypoint>(observer)
        .unwrap()
        .accepted_requests[&handle]
        .clone();
    assert_eq!(receipt.contact, handle);
    assert_eq!(receipt.order, crate::civilian::CivilianOrder::Hold);
    assert_eq!(
        app.world()
            .get::<crate::civilian::CivilianTraffic>(target)
            .unwrap()
            .0
            .order(),
        Some(&receipt.order)
    );
    let encoded = serde_json::to_vec(&receipt).unwrap();
    assert_eq!(
        serde_json::from_slice::<AcceptedTrafficRequest>(&encoded).unwrap(),
        receipt
    );
}

#[test]
fn anonymous_and_lost_traffic_do_not_gain_manifest_or_live_itinerary() {
    let (mut app, observer, target) = rig();
    traffic(app.world_mut(), target);
    let handle = observe(&mut app, observer, false);
    let rows = |world: &mut World| {
        let picture = picture(world, observer);
        let mut query = SystemState::<
            Query<(
                &EntityUuid,
                Option<&EntityName>,
                &crate::civilian::CivilianSection,
                &crate::civilian::CivilianTraffic,
            )>,
        >::new(world);
        super::super::knowledge::civilian_rows(
            &picture,
            &query.get(world),
            None,
            &SystemId("navigation".into()),
        )
    };
    assert!(rows(app.world_mut()).is_empty());
    observe(&mut app, observer, true);
    let public = rows(app.world_mut());
    assert_eq!(public.len(), 1);
    assert_eq!(public[0].uuid, handle);
    assert_eq!(public[0].order_options.len(), 1);
    assert!(
        public[0].route.is_empty() && public[0].compliance.is_empty() && public[0].order.is_empty()
    );
    app.world_mut()
        .get_mut::<crate::server_app::ShipSystemBlackboards>(observer)
        .unwrap()
        .0
        .clear();
    assert!(rows(app.world_mut()).is_empty());
}

#[test]
fn ordinary_ai_orders_two_contacts_once_and_keeps_highest_request_per_contact() {
    use crate::core::messages::*;
    let (mut app, observer, target) = rig();
    traffic(app.world_mut(), target);
    let order = crate::civilian::CivilianOrder::divert_to_route("public-route");
    app.world_mut()
        .get_mut::<crate::civilian::CivilianSection>(target)
        .unwrap()
        .0
        .order_options
        .push(crate::civilian::traffic::CivilianOrderOption {
            id: "divert".into(),
            label: "public.divert".into(),
            order: order.clone(),
        });
    let other = app
        .world_mut()
        .spawn((
            EntityUuid("00000000-0000-0000-0000-000000000772".into()),
            EntityName("second-public-name".into()),
            EntityTagsSection(vec!["ship".into()]),
            EntityTarget(crate::entities::target::TargetSection {
                tags: vec!["ship".into()],
                threat_level: Default::default(),
                description: None,
            }),
            Transform::from_xyz(25.0, 0.0, 5.0),
        ))
        .id();
    traffic(app.world_mut(), other);
    let choices = app
        .world()
        .get::<crate::civilian::CivilianSection>(target)
        .unwrap()
        .0
        .order_options
        .clone();
    app.world_mut()
        .get_mut::<crate::civilian::CivilianSection>(other)
        .unwrap()
        .0
        .order_options = choices;
    let handle = observe(&mut app, observer, true);
    let other_handle = picture(app.world_mut(), observer)
        .request("second-public-name")
        .unwrap()
        .key
        .clone();
    app.insert_resource(crate::lobby::Sessions(
        crate::lobby::session::SessionManager::new(),
    ));
    let mut world_config = crate::world::config::WorldConfig::default();
    world_config.routes.push(crate::civilian::RouteConfig {
        id: "public-route".into(),
        ..Default::default()
    });
    app.insert_resource(world_config);
    app.world_mut()
        .entity_mut(observer)
        .insert(crate::server_app::LocalShip);
    let objective = ScoredObjective {
        id: "order".into(),
        score: 1.0,
        directive: AiDirective::Order {
            target: "held-public-name".into(),
            route: "public-route".into(),
        },
        source: ObjectiveSource::Mission,
        relevance: vec![SystemAffinity::Navigation],
        snapshot: ObjectiveSnapshot {
            progress: None,
            unassigned: false,
            id: "order".into(),
            text: "order".into(),
            text_params: Default::default(),
            mandatory: false,
            status: ObjectiveStatus::Active,
            targets: vec!["held-public-name".into()],
            source: ObjectiveSource::Mission,
        },
    };
    let mut other_objective = objective.clone();
    other_objective.id = "second-order".into();
    other_objective.directive = AiDirective::Order {
        target: "second-public-name".into(),
        route: "public-route".into(),
    };
    // A conflicting lower order for the first contact must not replace its
    // highest applicable standing request on a later cadence.
    let mut lower = objective.clone();
    lower.id = "lower".into();
    lower.score = 0.5;
    lower.directive = AiDirective::Order {
        target: "held-public-name".into(),
        route: "other-route".into(),
    };
    app.world_mut()
        .resource_mut::<crate::world::config::WorldConfig>()
        .routes
        .push(crate::civilian::RouteConfig {
            id: "other-route".into(),
            ..Default::default()
        });
    let lower_order = crate::civilian::CivilianOrder::divert_to_route("other-route");
    app.world_mut()
        .get_mut::<crate::civilian::CivilianSection>(target)
        .unwrap()
        .0
        .order_options
        .push(crate::civilian::traffic::CivilianOrderOption {
            id: "lower".into(),
            label: "public.lower".into(),
            order: lower_order,
        });
    app.world_mut()
        .get_mut::<crate::server_app::ShipSystemBlackboards>(observer)
        .unwrap()
        .0
        .insert(
            crate::ship::system_registry::viewscreen_system_id(),
            SystemBlackboard::Viewscreen(ViewscreenBlackboard {
                scored_objectives: vec![objective, other_objective, lower],
                ..Default::default()
            }),
        );
    let before = reserve(app.world(), observer);
    app.world_mut()
        .run_system_once(operate_civilian_order_ai)
        .unwrap();
    assert!(
        matches!(&app.world().get::<AdmittedCommands>(observer).unwrap().0[0].payload,
        SystemControlPayload::OrderCivilian { target, .. } if target == &handle)
    );
    app.world_mut()
        .run_system_once(crate::civilian::tick_civilian_traffic)
        .unwrap();
    assert_eq!(reserve(app.world(), observer), before - 1.0);
    app.world_mut()
        .get_mut::<AdmittedCommands>(observer)
        .unwrap()
        .0
        .clear();
    app.world_mut()
        .run_system_once(operate_civilian_order_ai)
        .unwrap();
    assert!(
        matches!(&app.world().get::<AdmittedCommands>(observer).unwrap().0[0].payload,
        SystemControlPayload::OrderCivilian { target, .. } if target == &other_handle)
    );
    app.world_mut()
        .run_system_once(crate::civilian::tick_civilian_traffic)
        .unwrap();
    assert_eq!(reserve(app.world(), observer), before - 2.0);
    app.world_mut()
        .get_mut::<AdmittedCommands>(observer)
        .unwrap()
        .0
        .clear();
    for _ in 0..3 {
        app.world_mut()
            .run_system_once(operate_civilian_order_ai)
            .unwrap();
        assert!(app
            .world()
            .get::<AdmittedCommands>(observer)
            .unwrap()
            .0
            .is_empty());
    }
    app.world_mut()
        .run_system_once(crate::civilian::tick_civilian_traffic)
        .unwrap();
    assert_eq!(reserve(app.world(), observer), before - 2.0);
    assert_eq!(
        app.world()
            .get::<NavigationWaypoint>(observer)
            .unwrap()
            .accepted_requests
            .len(),
        2
    );
}

#[test]
fn exact_opaque_handle_keeps_its_meaning_when_another_name_collides() {
    let mut picture = super::super::knowledge::Picture::default();
    for (key, name) in [("opaque-one", "public"), ("opaque-two", "opaque-one")] {
        picture.contacts.insert(
            key.into(),
            super::super::knowledge::Contact {
                key: key.into(),
                route: key.into(),
                name: Some(name.into()),
                references: Vec::new(),
                position: [0.0; 3],
                charted: false,
            },
        );
    }
    assert_eq!(picture.request("opaque-one").unwrap().key, "opaque-one");
}

#[test]
fn declared_dock_destination_resolves_only_the_same_public_picture() {
    let mut picture = super::super::knowledge::Picture::default();
    let contact = super::super::knowledge::Contact {
        key: "public-berth".into(),
        route: "private-facility".into(),
        name: Some("Observed Port".into()),
        references: vec!["declared-port".into()],
        position: [1.0; 3],
        charted: false,
    };
    picture
        .contacts
        .insert(contact.key.clone(), contact.clone());
    let options = [crate::civilian::traffic::CivilianOrderOption {
        id: "dock".into(),
        label: "public.dock".into(),
        order: crate::civilian::CivilianOrder::Dock {
            structure: "declared-port".into(),
        },
    }];
    let request = |reference: &str| crate::civilian::CivilianOrder::Dock {
        structure: reference.into(),
    };
    assert_eq!(
        picture.canonical_order(&request("public-berth"), &options),
        Some(options[0].order.clone())
    );
    assert!(picture
        .canonical_order(&request("private-facility"), &options)
        .is_none());
    assert!(picture
        .canonical_order(&request("unknown"), &options)
        .is_none());
    let mut other = contact;
    other.key = "second-public-berth".into();
    other.route = "other-private".into();
    picture.contacts.insert(other.key.clone(), other);
    assert!(picture
        .canonical_order(&request("public-berth"), &options)
        .is_none());
}

#[test]
fn chart_permission_is_explicit_filtered_and_conceal_still_wins() {
    let (mut app, observer, _) = rig();
    app.insert_resource(crate::lobby::WorldResource(
        crate::core::messages::WorldData {
            entities: vec![
                crate::core::messages::EntitySnapshot {
                    uuid: "public-planet".into(),
                    name: Some("Charted planet".into()),
                    tags: vec!["planet".into(), crate::console_delivery::CHARTED_TAG.into()],
                    position: Some([40.0, 0.0, 0.0]),
                    ..Default::default()
                },
                crate::core::messages::EntitySnapshot {
                    uuid: "private-planet".into(),
                    name: Some("SECRET".into()),
                    tags: vec!["planet".into()],
                    position: Some([41.0, 0.0, 0.0]),
                    ..Default::default()
                },
            ],
            ..Default::default()
        },
    ));
    let known = picture(app.world_mut(), observer);
    assert!(known.request("public-planet").unwrap().charted);
    assert!(known.request("private-planet").is_none());
    assert!(known.request("SECRET").is_none());
    app.world_mut()
        .resource_mut::<crate::world::server::WorldContentRuntime>()
        .contact_overrides
        .entry("observer".into())
        .or_default()
        .insert(
            "public-planet".into(),
            crate::gm::operations::contact::ContactMode::Conceal,
        );
    assert!(picture(app.world_mut(), observer)
        .request("public-planet")
        .is_none());
}

#[test]
fn transfer_approach_uses_the_same_paid_permitted_contact_as_docking() {
    use crate::core::messages::*;
    let (mut app, observer, target) = rig();
    crate::ai::host::register_ai_host_env(&mut app);
    let objective = ScoredObjective {
        id: "supply-port".into(),
        score: 100.0,
        directive: AiDirective::Transfer {
            target: "held-public-name".into(),
        },
        source: ObjectiveSource::Mission,
        relevance: vec![SystemAffinity::Helm],
        snapshot: ObjectiveSnapshot {
            id: "supply-port".into(),
            text: String::new(),
            text_params: Default::default(),
            mandatory: false,
            status: ObjectiveStatus::Active,
            targets: vec![],
            source: ObjectiveSource::Mission,
            progress: None,
            unassigned: false,
        },
    };
    app.world_mut()
        .get_mut::<crate::server_app::ShipSystemBlackboards>(observer)
        .unwrap()
        .0
        .insert(
            crate::ship::system_registry::viewscreen_system_id(),
            SystemBlackboard::Viewscreen(ViewscreenBlackboard {
                scored_objectives: vec![objective],
                ..Default::default()
            }),
        );
    app.world_mut()
        .run_system_once(operate_navigation_ai)
        .unwrap();
    assert!(
        app.world()
            .get::<AdmittedCommands>(observer)
            .unwrap()
            .0
            .is_empty(),
        "an authored Transfer name does not reveal an unobserved ship"
    );
    let handle = observe(&mut app, observer, true);
    let measured = picture(app.world_mut(), observer)
        .request("held-public-name")
        .unwrap()
        .position;
    // Private movement without a new observation must not steer Navigation.
    app.world_mut()
        .get_mut::<Transform>(target)
        .unwrap()
        .translation = Vec3::new(9000.0, 0.0, 8000.0);
    app.world_mut()
        .run_system_once(operate_navigation_ai)
        .unwrap();
    let commands = app.world().get::<AdmittedCommands>(observer).unwrap();
    let command = commands
        .for_target("navigation")
        .next()
        .expect("actual permitted approach command");
    assert!(
        matches!(&command.payload, SystemControlPayload::SetNavigationWaypoint { x,z,source_uuid: Some(contact) }
        if *x == measured[0] && *z == measured[2] && contact == &handle)
    );
    assert_ne!(
        handle, TARGET,
        "public handle remains distinct from the private route"
    );
}

fn formation_boot() -> (App, Entity, Entity) {
    use crate::comms::server::{CommsInboxRes, CommsRuntime};
    use crate::world_id::InstallWorldIdMint;
    let (mut app, observer, bare_target) = rig_for("assets/entities/formation_cruiser.toml", false);
    let accepted = crate::entities::include_resolve::load_entity_config(
        "assets/entities/formation_cruiser.toml",
    )
    .unwrap();
    app.insert_resource(crate::lobby::server::ShipClientConfigResource(
        crate::lobby::server::project_ship_client_config(&accepted),
    ));
    app.world_mut().despawn(bare_target);
    app.insert_resource(crate::lobby::WorldResource(
        crate::core::messages::WorldData::default(),
    ));
    app.init_resource::<CommsInboxRes>()
        .init_resource::<CommsRuntime>()
        .init_resource::<crate::server_app::SimOutbox>();
    app.world_mut()
        .insert_world_id_mint(crate::world_id::WorldIdMint::default());
    let leader_config = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    let leader = crate::entities::spawner::spawn_entity(
        &mut app.world_mut().commands(),
        &leader_config,
        Vec3::new(10.0, 0.0, 3.0),
        TARGET.into(),
        None,
    );
    app.world_mut().flush();
    {
        let mut pose = app
            .world_mut()
            .get_mut::<crate::ship::state::ShipPhysics>(leader)
            .unwrap();
        pose.x = 10.0;
        pose.z = 3.0;
    }
    app.world_mut()
        .entity_mut(leader)
        .insert(EntityName("held-public-name".into()));
    (app, observer, leader)
}

/// Same read-model rebuilds used after restoration; no sensing work or phase advances.
fn formation_publications(world: &mut World) {
    world
        .run_system_once(crate::science::server::publish_scan_blackboard)
        .unwrap();
    world
        .run_system_once(crate::science::passive::publish)
        .unwrap();
    world
        .run_system_once(crate::ship::sensors::publish_sensor_radar_blackboard)
        .unwrap();
    world
        .run_system_once(crate::console::helm::server::publish_helm_blackboard)
        .unwrap();
    world
        .run_system_once(publish_navigation_blackboard)
        .unwrap();
    world
        .run_system_once(crate::console::weapons::blackboard::publish_tactical_radar_blackboard)
        .unwrap();
}

/// Real radio lineage plus a separately paid Broad measurement. The fixture
/// installs accepted hardware and initial physical poses, never a contact,
/// sender association, received message or successful machinery result.
#[test]
fn formation_selection_uses_actual_heard_acceptance_and_earned_measurement_with_stable_pause() {
    use crate::comms::{
        radio::tick_radio,
        server::{CommsInboxRes, CommsRuntime},
    };
    use crate::console::comms::server::publish_comms_blackboard;
    use crate::core::messages::AdmittedCommand;
    use phoenix_model::{comms_channel::CommsAudience, comms_request::GenericCommsRequest};
    let (mut app, observer, leader) = formation_boot();
    let nav = SystemId("navigation".into());
    let comms = SystemId("comms".into());
    let nav_console = app
        .world()
        .get::<crate::ship_plugin::ShipConfigComponent>(observer)
        .unwrap()
        .0
        .console_for_system(&nav)
        .unwrap()
        .id
        .clone();
    app.world_mut()
        .get_mut::<AdmittedCommands>(observer)
        .unwrap()
        .0
        .push(AdmittedCommand {
            target: comms.clone(),
            payload: SystemControlPayload::SendCommsText {
                channel: "public".into(),
                audience: CommsAudience::All,
                body: "Actual receiver introduction".into(),
            },
            response_token: None,
            feedback_correlation: None,
        });
    app.world_mut().run_system_once(tick_radio).unwrap();
    app.world_mut()
        .get_mut::<AdmittedCommands>(observer)
        .unwrap()
        .0
        .clear();
    app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 += 1;
    app.world_mut()
        .get_mut::<AdmittedCommands>(leader)
        .unwrap()
        .0
        .push(AdmittedCommand {
            target: comms,
            payload: SystemControlPayload::SendCommsRequest {
                channel: "public".into(),
                audience: CommsAudience::Contact {
                    target_uuid: "observer".into(),
                },
                request: GenericCommsRequest::FormUp { slot: [-40, 80] },
                planned_request: None,
            },
            response_token: None,
            feedback_correlation: None,
        });
    app.world_mut().run_system_once(tick_radio).unwrap();
    app.world_mut()
        .get_mut::<AdmittedCommands>(leader)
        .unwrap()
        .0
        .clear();
    app.world_mut()
        .run_system_once(publish_comms_blackboard)
        .unwrap();
    let message = app
        .world()
        .resource::<CommsInboxRes>()
        .0
        .iter()
        .find(|message| {
            message.is_for_ship(Some("observer")) && crate::comms::requests::is_request(message)
        })
        .expect("physically heard request")
        .id
        .clone();
    let config = app
        .world()
        .get::<crate::ship_plugin::ShipConfigComponent>(observer)
        .unwrap()
        .clone();
    let boards = app
        .world()
        .get::<crate::server_app::ShipSystemBlackboards>(observer)
        .unwrap()
        .clone();
    let sources = app
        .world()
        .get::<crate::ship_plugin::ShipSystemControlSources>(observer)
        .unwrap()
        .clone();
    let damage = app
        .world()
        .get::<crate::ship::compartment_damage::ShipDamage>(observer)
        .cloned();
    let power = app
        .world()
        .get::<crate::ship::power::ShipPowerSystem>(observer)
        .cloned();
    let radio = app
        .world()
        .get::<crate::comms::radio::RadioProfile>(observer)
        .cloned();
    let now = app.world().resource::<crate::sim_tick::SimTick>().0;
    assert!(crate::comms::requests::respond(
        &mut app.world_mut().resource_mut::<CommsInboxRes>(),
        "observer",
        &message,
        0,
        now,
        &config,
        &boards,
        &sources,
        damage.as_ref(),
        power.as_ref(),
        radio.as_ref()
    ));
    app.world_mut()
        .run_system_once(publish_comms_blackboard)
        .unwrap();
    assert!(
        app.world()
            .get::<NavigationWaypoint>(observer)
            .unwrap()
            .mode()
            .is_none(),
        "Comms accepts coordination; it does not issue a Navigation or Helm order"
    );
    let handle = observe_with(&mut app, observer, true, "tactical-radar");
    assert_ne!(handle, TARGET);
    // Real second sample supplies measured velocity; a private current-route
    // match or an identity-only record cannot substitute for that reading.
    app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 += 2;
    let handle_again = observe_with(&mut app, observer, true, "tactical-radar");
    assert_eq!(handle_again, handle);
    let before = reserve(app.world(), observer);
    let command = |leader: String| AdmittedCommand {
        target: nav.clone(),
        payload: SystemControlPayload::SelectFormationCourse {
            request_id: message.clone(),
            leader,
        },
        response_token: None,
        feedback_correlation: None,
    };
    app.world_mut()
        .get_mut::<AdmittedCommands>(observer)
        .unwrap()
        .0 = vec![command(TARGET.into())];
    app.world_mut()
        .run_system_once(handle_navigation_waypoint)
        .unwrap();
    assert_eq!(
        reserve(app.world(), observer),
        before,
        "private route UUID is not an earned contact handle"
    );
    assert!(app
        .world()
        .get::<NavigationWaypoint>(observer)
        .unwrap()
        .mode()
        .is_none());
    app.world_mut()
        .get_mut::<AdmittedCommands>(observer)
        .unwrap()
        .0 = vec![command(handle.clone())];
    app.world_mut()
        .run_system_once(handle_navigation_waypoint)
        .unwrap();
    app.world_mut()
        .get_mut::<AdmittedCommands>(observer)
        .unwrap()
        .0
        .clear();
    assert!(
        reserve(app.world(), observer) < before,
        "actual Navigation selection pays discrete work"
    );
    let waypoint = app.world().get::<NavigationWaypoint>(observer).unwrap();
    let generation = waypoint.generation();
    let WaypointMode::Formation { course } = waypoint.mode().unwrap() else {
        panic!("own selected course")
    };
    assert_eq!(course.request.message_id, message);
    assert_eq!(course.leader, handle);
    assert_eq!(course.request.source_name, "held-public-name");
    assert_eq!(course.request.slot, [-40, 80]);
    let retained = course.measurement.clone();
    let point = waypoint.snapshot().unwrap();
    assert_eq!(
        [point.x, point.z],
        retained.slot_point(&course.request).unwrap()
    );
    app.world_mut()
        .get_mut::<crate::ship::power::ShipPowerSystem>(observer)
        .unwrap()
        .0
        .work_continuous(&nav, 0.5, 1.0)
        .unwrap();
    app.world_mut()
        .run_system_once(refresh_anchored_waypoint)
        .unwrap();
    assert_eq!(
        app.world()
            .get::<NavigationWaypoint>(observer)
            .unwrap()
            .generation(),
        generation
    );
    app.world_mut()
        .get_mut::<crate::ship_plugin::ShipConfigComponent>(observer)
        .unwrap()
        .0
        .consoles
        .iter_mut()
        .find(|console| console.id == nav_console)
        .unwrap()
        .feeds
        .retain(|feed| feed.kind != phoenix_model::console::ConsoleObservationKind::TacticalRadar);
    app.world_mut()
        .run_system_once(refresh_anchored_waypoint)
        .unwrap();
    let waypoint = app.world().get::<NavigationWaypoint>(observer).unwrap();
    assert!(
        waypoint.snapshot().is_none(),
        "missing permitted current leader suppresses execution"
    );
    assert_eq!(waypoint.canonical_snapshot(), Some(point));
    assert_eq!(waypoint.generation(), generation);
    let WaypointMode::Formation { course } = waypoint.mode().unwrap() else {
        panic!("dated retained intent")
    };
    assert!(course.paused);
    assert_eq!(course.measurement, retained);
    app.world_mut()
        .get_mut::<crate::ship_plugin::ShipConfigComponent>(observer)
        .unwrap()
        .0
        .consoles
        .iter_mut()
        .find(|console| console.id == nav_console)
        .unwrap()
        .feeds
        .push(phoenix_model::console::ConsoleFeed {
            source: SystemId("tactical-radar".into()),
            kind: phoenix_model::console::ConsoleObservationKind::TacticalRadar,
        });
    app.world_mut()
        .run_system_once(refresh_anchored_waypoint)
        .unwrap();
    assert!(app
        .world()
        .get::<NavigationWaypoint>(observer)
        .unwrap()
        .snapshot()
        .is_some());
    app.world_mut()
        .get_mut::<AdmittedCommands>(observer)
        .unwrap()
        .0 = vec![AdmittedCommand {
        target: nav.clone(),
        payload: SystemControlPayload::ClearNavigationWaypoint,
        response_token: None,
        feedback_correlation: None,
    }];
    app.world_mut()
        .run_system_once(handle_navigation_waypoint)
        .unwrap();
    app.world_mut()
        .get_mut::<AdmittedCommands>(observer)
        .unwrap()
        .0
        .clear();
    crate::ai::host::register_ai_host_env(&mut app);
    app.world_mut()
        .run_system_once(operate_navigation_ai)
        .unwrap();
    let admitted = app.world().get::<AdmittedCommands>(observer).unwrap();
    assert!(admitted.0.iter().any(|command|command.target==nav && matches!(&command.payload,
        SystemControlPayload::SelectFormationCourse{request_id,leader} if request_id==&message && leader==&handle)),
        "ordinary own Navigation AI independently correlates the mature public identity and uses normal admission");
    app.world_mut()
        .run_system_once(handle_navigation_waypoint)
        .unwrap();
    assert!(
        matches!(app.world().get::<NavigationWaypoint>(observer).unwrap().mode(),Some(WaypointMode::Formation{course}) if course.leader==handle)
    );
    app.world_mut()
        .get_mut::<AdmittedCommands>(observer)
        .unwrap()
        .0
        .clear();
    formation_publications(app.world_mut());
    let saved: crate::snapshot::PhoenixSnapshot = serde_json::from_slice(
        &serde_json::to_vec(&crate::snapshot::capture(app.world())).unwrap(),
    )
    .unwrap();
    let (mut fresh, fresh_observer, _) = formation_boot();
    assert!(fresh
        .world()
        .resource::<CommsInboxRes>()
        .0
        .messages()
        .is_empty());
    let report = crate::snapshot::restore(fresh.world_mut(), &saved);
    assert!(
        report.is_complete(),
        "genuine candidate request absent from bootstrap must restore: {:?}",
        report.gaps
    );
    assert_eq!(
        crate::snapshot::capture(fresh.world()).entities,
        saved.entities
    );
    let before = crate::snapshot::capture(fresh.world());
    let before_digest = crate::sim_digest::world_digest(fresh.world());
    for mutation in 0..5 {
        let mut bad = saved.clone();
        let waypoint = bad
            .entities
            .iter_mut()
            .find(|row| row.uuid == "observer")
            .unwrap()
            .navigation_waypoint
            .as_mut()
            .unwrap();
        let course = waypoint.formation.as_mut().unwrap();
        let received_id = course.request.message_id.clone();
        match mutation {
            0 => course.owner = SystemId("comms".into()),
            1 => course.leader = TARGET.into(),
            2 => course.request.origin_id = "forged-origin".into(),
            3 => course.measurement.observed_tick = saved.tick + 1,
            _ => {
                bad.comms
                    .as_mut()
                    .unwrap()
                    .inbox
                    .retain(|message| message.id != received_id);
            }
        }
        let report = crate::snapshot::restore(fresh.world_mut(), &bad);
        assert!(
            !report.is_complete(),
            "forged Formation candidate {mutation} refused"
        );
        assert_eq!(crate::snapshot::capture(fresh.world()), before);
        assert_eq!(
            crate::sim_digest::world_digest(fresh.world()),
            before_digest
        );
    }
    // Ordinary ClearComms dismisses this structured record; its canonical
    // provenance and already selected Navigation intent remain available.
    fresh
        .world_mut()
        .get_mut::<AdmittedCommands>(fresh_observer)
        .unwrap()
        .0 = vec![AdmittedCommand {
        target: SystemId("comms".into()),
        payload: SystemControlPayload::ClearComms,
        response_token: None,
        feedback_correlation: None,
    }];
    fresh
        .world_mut()
        .run_system_once(crate::console::comms::server::handle_clear_comms)
        .unwrap();
    fresh
        .world_mut()
        .get_mut::<AdmittedCommands>(fresh_observer)
        .unwrap()
        .0
        .clear();
    fresh
        .world_mut()
        .run_system_once(publish_comms_blackboard)
        .unwrap();
    assert!(fresh
        .world()
        .resource::<CommsInboxRes>()
        .0
        .messages()
        .iter()
        .any(|row| row.id == message
            && row
                .request_progress
                .as_ref()
                .is_some_and(|progress| progress.dismissed)));
    formation_publications(fresh.world_mut());
    let dismissed = crate::snapshot::capture(fresh.world());
    let (mut again, _, _) = formation_boot();
    let report = crate::snapshot::restore(again.world_mut(), &dismissed);
    assert!(
        report.is_complete(),
        "dismissed actual structured lineage still restores: {:?}",
        report.gaps
    );
    assert_eq!(
        crate::snapshot::capture(again.world()).entities,
        dismissed.entities
    );
}

#[test]
fn optional_formation_hull_preserves_actual_hardware_and_existing_console_feeds() {
    let base = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    let optional = crate::entities::include_resolve::load_entity_config(
        "assets/entities/formation_cruiser.toml",
    )
    .unwrap();
    let base_ship = base.ship_config.as_ref().unwrap();
    let optional_ship = optional.ship_config.as_ref().unwrap();
    assert_eq!(
        serde_json::to_value(&base_ship.systems).unwrap(),
        serde_json::to_value(&optional_ship.systems).unwrap()
    );
    assert_eq!(
        serde_json::to_value(&base_ship.stations).unwrap(),
        serde_json::to_value(&optional_ship.stations).unwrap()
    );
    for original in &base_ship.consoles {
        let actual = optional_ship
            .consoles
            .iter()
            .find(|row| row.id == original.id)
            .unwrap();
        for feed in &original.feeds {
            assert!(actual.feeds.contains(feed));
        }
        assert_eq!(actual.ratings, original.ratings);
        assert_eq!(actual.host_order, original.host_order);
    }
    let nav = optional_ship
        .console_for_system(&SystemId("navigation".into()))
        .unwrap();
    assert!(nav.feeds.iter().any(|feed| feed.source.0 == "comms"
        && feed.kind == phoenix_model::console::ConsoleObservationKind::Comms));
    assert!(nav
        .feeds
        .iter()
        .any(|feed| feed.source.0 == "tactical-radar"
            && feed.kind == phoenix_model::console::ConsoleObservationKind::TacticalRadar));
}
