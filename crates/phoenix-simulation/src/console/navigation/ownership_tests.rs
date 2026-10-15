use super::*;
use crate::core::messages::{AdmittedCommand, SystemId};
use bevy::ecs::system::RunSystemOnce;

fn custom_config() -> crate::entities::config::EntityConfig {
    let config = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    // Rename the actual authored identity and every reference, including the
    // finite energy consumer and compartment key. Preserve the kind vocabulary.
    fn rename(value: &mut toml::Value) {
        match value {
            toml::Value::String(text) if text == "navigation" => *text = "custom-navigation".into(),
            toml::Value::Array(rows) => rows.iter_mut().for_each(rename),
            toml::Value::Table(table) => {
                if let Some(row) = table.remove("navigation") {
                    table.insert("custom-navigation".into(), row);
                }
                for (key, value) in table.iter_mut() {
                    if key != "kind" {
                        rename(value);
                    }
                }
            }
            _ => {}
        }
    }
    let mut authored = config.to_toml_value().unwrap();
    rename(&mut authored);
    crate::entities::config::EntityConfig::from_toml(&toml::to_string(&authored).unwrap()).unwrap()
}
fn command(target: &str, payload: SystemControlPayload) -> AdmittedCommand {
    AdmittedCommand {
        target: SystemId(target.into()),
        payload,
        response_token: None,
        feedback_correlation: None,
    }
}
fn reserve(world: &World, entity: Entity) -> f32 {
    world
        .get::<crate::ship::power::ShipPowerSystem>(entity)
        .unwrap()
        .0
        .network()
        .consumers[&SystemId("custom-navigation".into())]
        .energy
}

#[test]
fn custom_navigation_controls_pay_the_fitted_owner_and_reject_an_absent_canonical_alias() {
    let mut config = custom_config();
    let ship_config = config.ship_config.as_mut().unwrap();
    let console = ship_config
        .system(&SystemId("custom-navigation".into()))
        .unwrap()
        .console
        .clone()
        .unwrap();
    ship_config
        .consoles
        .iter_mut()
        .find(|row| row.id == console)
        .unwrap()
        .feeds
        .push(phoenix_model::console::ConsoleFeed {
            source: SystemId("sensor-radar".into()),
            kind: phoenix_model::console::ConsoleObservationKind::SensorRadar,
        });
    let mut app = App::new();
    app.insert_resource(Time::<Fixed>::from_seconds(0.5));
    app.insert_resource(crate::sim_tick::SimTick(0));
    app.init_resource::<crate::world::server::WorldContentRuntime>();
    app.init_resource::<crate::effect_queue::EffectQueue<crate::civilian::server::PendingCivilianOrder>>();
    let ship = crate::entities::spawner::spawn_entity(
        &mut app.world_mut().commands(),
        &config,
        Vec3::ZERO,
        "ordering-ship".into(),
        None,
    );
    app.world_mut().flush();
    app.world_mut()
        .entity_mut(ship)
        .insert(NavigationWaypoint::default());
    let traffic_uuid = uuid::Uuid::from_u128(0xc14).to_string();
    let mut civilian_config = crate::civilian::CivilianConfig::default();
    civilian_config
        .order_options
        .push(crate::civilian::traffic::CivilianOrderOption {
            id: "hold".into(),
            label: "public.hold".into(),
            order: crate::civilian::CivilianOrder::Hold,
        });
    let civilian = app
        .world_mut()
        .spawn((
            crate::entities::spawner::EntityUuid(traffic_uuid.clone()),
            Transform::from_xyz(10.0, 0.0, 0.0),
            crate::entities::spawner::EntityName("public-traffic".into()),
            crate::entities::spawner::EntityTagsSection(vec!["ship".into()]),
            crate::entities::spawner::EntityTarget(crate::entities::target::TargetSection {
                tags: vec!["ship".into()],
                threat_level: Default::default(),
                description: None,
            }),
            crate::civilian::CivilianSection(civilian_config),
            crate::civilian::CivilianTraffic(crate::civilian::CivilianState::from_config(
                &crate::civilian::CivilianConfig::default(),
            )),
            crate::entities::spawner::BehaviourSection(
                crate::entities::config::BehaviourConfig::default(),
            ),
        ))
        .id();
    let before = reserve(app.world(), ship);
    let contact = super::knowledge_tests::observe(&mut app, ship, true);
    let order = || SystemControlPayload::OrderCivilian {
        target: contact.clone(),
        order: crate::civilian::CivilianOrder::Hold,
    };
    app.world_mut().get_mut::<AdmittedCommands>(ship).unwrap().0 =
        vec![command("navigation", order())];
    app.world_mut()
        .run_system_once(crate::civilian::tick_civilian_traffic)
        .unwrap();
    assert!(app
        .world()
        .get::<crate::civilian::CivilianTraffic>(civilian)
        .unwrap()
        .0
        .order()
        .is_none());
    assert_eq!(reserve(app.world(), ship), before);
    app.world_mut().get_mut::<AdmittedCommands>(ship).unwrap().0 =
        vec![command("custom-navigation", order())];
    app.world_mut()
        .run_system_once(crate::civilian::tick_civilian_traffic)
        .unwrap();
    assert_eq!(
        app.world()
            .get::<crate::civilian::CivilianTraffic>(civilian)
            .unwrap()
            .0
            .order(),
        Some(&crate::civilian::CivilianOrder::Hold)
    );
    assert_eq!(reserve(app.world(), ship), before - 1.0);
    app.world_mut().get_mut::<AdmittedCommands>(ship).unwrap().0 = vec![command(
        "custom-navigation",
        SystemControlPayload::SetNavigationWaypoint {
            x: 5.0,
            z: 6.0,
            source_uuid: None,
        },
    )];
    app.world_mut()
        .run_system_once(handle_navigation_waypoint)
        .unwrap();
    assert_eq!(
        app.world()
            .get::<NavigationWaypoint>(ship)
            .unwrap()
            .snapshot()
            .map(|point| (point.x, point.z)),
        Some((5.0, 6.0))
    );
    assert_eq!(reserve(app.world(), ship), before - 2.0);
}

#[test]
fn primary_navigation_registration_covers_actual_custom_owner_and_all_real_commands() {
    let config = custom_config();
    let mut app = App::new();
    app.add_plugins(NavigationPlugin);
    let registry = app
        .world()
        .resource::<crate::command_admission::AdmittedConsumerRegistry>();
    let ship = config.ship_config.as_ref().unwrap();
    let owner = ship.system(&SystemId("custom-navigation".into())).unwrap();
    assert!(registry.is_system_routed(owner));
    for payload in [
        SystemControlPayload::ClearNavigationWaypoint,
        SystemControlPayload::SetNavigationWaypoint {
            x: 1.0,
            z: 2.0,
            source_uuid: None,
        },
        SystemControlPayload::OrderCivilian {
            target: "actual-traffic".into(),
            order: crate::civilian::CivilianOrder::Hold,
        },
        SystemControlPayload::SetRadarSharing {
            share_own_faction: true,
            receive: true,
        },
    ] {
        assert!(registry.is_command_routed(owner, &payload));
        assert_eq!(
            registry.feedback_support(&owner.id, &payload, ship),
            crate::command_admission::FeedbackSupport::Supported
        );
    }
    let mut absent = crate::ship_plugin::ShipConfigComponent(ship.clone());
    absent
        .0
        .systems
        .retain(|system| system.kind != crate::ship::system_registry::NAVIGATION_KIND);
    assert!(navigation_owner(Some(&absent), None).is_none());
    assert_eq!(
        navigation_owner(None, None),
        Some(crate::ship::system_registry::navigation_system_id())
    );
}

#[test]
fn unsupported_nonprimary_navigation_commands_refuse_once_without_work_or_state_changes() {
    let config = custom_config();
    let mut app = App::new();
    app.init_resource::<crate::world::server::WorldContentRuntime>();
    app.init_resource::<crate::server_app::SimOutbox>();
    app.init_resource::<Messages<crate::lobby::OutboundMessage>>();
    app.init_resource::<crate::effect_queue::EffectQueue<crate::civilian::server::PendingCivilianOrder>>();
    let ship = crate::entities::spawner::spawn_entity(
        &mut app.world_mut().commands(),
        &config,
        Vec3::ZERO,
        "ordering-ship".into(),
        None,
    );
    app.world_mut().flush();
    // Model the unsupported alternate address in the actual matcher topology.
    // It has no work owner: these requests must refuse before looking for or
    // spending any reserve, rather than disappearing after normal admission.
    let mut topology = app
        .world_mut()
        .get_mut::<crate::ship_plugin::ShipConfigComponent>(ship)
        .unwrap();
    let mut alternate = topology
        .0
        .system(&SystemId("custom-navigation".into()))
        .unwrap()
        .clone();
    alternate.id = SystemId("alternate-navigation".into());
    topology.0.systems.push(alternate);
    drop(topology);
    let before = reserve(app.world(), ship);
    let frontier = app
        .world()
        .get::<sharing::NavigationSharing>(ship)
        .unwrap()
        .0
        .clone();
    let payloads = [
        SystemControlPayload::SetNavigationWaypoint {
            x: 10.0,
            z: 20.0,
            source_uuid: None,
        },
        SystemControlPayload::ClearNavigationWaypoint,
        SystemControlPayload::OrderCivilian {
            target: "no-traffic-lookup-needed".into(),
            order: crate::civilian::CivilianOrder::Hold,
        },
        SystemControlPayload::SetRadarSharing {
            share_own_faction: true,
            receive: false,
        },
    ];
    app.world_mut().get_mut::<AdmittedCommands>(ship).unwrap().0 = payloads
        .into_iter()
        .enumerate()
        .map(|(index, payload)| {
            let mut command = command("alternate-navigation", payload);
            command.response_token = Some("human-holder".into());
            command.feedback_correlation = Some(
                crate::core::messages::ActionCorrelationId::new(format!("alternate-nav-{index}"))
                    .unwrap(),
            );
            command
        })
        .collect();
    app.world_mut()
        .run_system_once(handle_navigation_waypoint)
        .unwrap();
    app.world_mut()
        .run_system_once(sharing::apply_commands)
        .unwrap();
    app.world_mut()
        .run_system_once(crate::civilian::tick_civilian_traffic)
        .unwrap();
    let mut outcomes: Vec<_> = app
        .world_mut()
        .resource_mut::<Messages<crate::lobby::OutboundMessage>>()
        .drain()
        .map(|message| message.msg)
        .collect();
    outcomes.extend(
        app.world()
            .resource::<crate::server_app::SimOutbox>()
            .iter()
            .map(|(_, message)| message.clone()),
    );
    assert_eq!(outcomes.len(), 4);
    assert!(outcomes.iter().all(|message| matches!(
        message,
        crate::core::messages::ServerMessage::ActionFeedback {
            outcome: ActionFeedbackOutcome::Refused,
            ..
        }
    )));
    assert_eq!(reserve(app.world(), ship), before);
    assert!(app
        .world()
        .get::<NavigationWaypoint>(ship)
        .unwrap()
        .snapshot()
        .is_none());
    assert_eq!(
        app.world()
            .get::<sharing::NavigationSharing>(ship)
            .unwrap()
            .0,
        frontier
    );
}
