use super::*;
use crate::entities::spawner::*;
use crate::science::broad::*;
use bevy::ecs::system::{RunSystemOnce, SystemState};

const TARGET: &str = "00000000-0000-0000-0000-000000000771";
fn rig() -> (App, Entity, Entity) {
    let mut config = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    let ship = config.ship_config.as_mut().unwrap();
    let console = ship
        .system(&SystemId("navigation".into()))
        .unwrap()
        .console
        .clone()
        .unwrap();
    ship.consoles
        .iter_mut()
        .find(|row| row.id == console)
        .unwrap()
        .feeds
        .push(phoenix_model::console::ConsoleFeed {
            source: SystemId("sensor-radar".into()),
            kind: phoenix_model::console::ConsoleObservationKind::SensorRadar,
        });
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
    let owner = SystemId("sensor-radar".into());
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
                    SystemBlackboard::SensorRadar(crate::core::messages::SensorRadarBlackboard {
                        tracks: Some(records),
                        ..Default::default()
                    }),
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
