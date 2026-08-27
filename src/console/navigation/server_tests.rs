use super::*;
use crate::core::messages::{ClientMessage, ServerMessage};
use crate::lobby::{InboundMessage, LobbyPlugin, OutboundMessage};
use crate::server_app::{
    sim_state_broadcaster, LastBroadcastEntityPositions, LastBroadcastHull, ShipImpulse,
};

#[derive(Resource, Default)]
struct Outbox(Vec<OutboundMessage>);

fn collect(mut reader: MessageReader<OutboundMessage>, mut sink: ResMut<Outbox>) {
    for msg in reader.read() {
        sink.0.push(msg.clone());
    }
}

fn test_app() -> App {
    let mut app = App::new();
    crate::ai::host::register_ai_host_env(&mut app);
    app.add_plugins(LobbyPlugin)
        .add_plugins(bevy::time::TimePlugin)
        .add_plugins(crate::server_app::AdmissionPlugin)
        // Chain SimSet phases so handle (Input) → refresh (Modifiers) →
        // broadcast (Broadcast) run in the right order. Without this,
        // adding a second resource-touching system to a different set
        // makes the schedule non-deterministic and breaks the existing
        // broadcast assertions.
        .configure_sets(
            FixedUpdate,
            (
                crate::sim_sets::SimSet::Input,
                crate::sim_sets::SimSet::Physics,
                crate::sim_sets::SimSet::Damage,
                crate::sim_sets::SimSet::Modifiers,
                crate::sim_sets::SimSet::Publish,
                crate::sim_sets::SimSet::PublishAggregate,
                crate::sim_sets::SimSet::Broadcast,
            )
                .chain(),
        )
        .init_resource::<crate::server_app::LastBroadcastBlackboards>()
        .init_resource::<crate::lobby::server::ShipClientConfigResource>()
        .add_plugins(NavigationPlugin)
        .add_plugins(sim_state_broadcaster())
        .add_plugins(crate::server_app::sim_outbox_broadcaster())
        .init_resource::<crate::server_app::SimOutbox>()
        .add_systems(
            FixedUpdate,
            crate::server_app::broadcast_blackboard_updates
                .in_set(crate::sim_sets::SimSet::PublishAggregate),
        )
        .init_resource::<Outbox>()
        .init_resource::<LastBroadcastEntityPositions>()
        .init_resource::<crate::server_app::LastBroadcastEntityHealth>()
        .init_resource::<LastBroadcastHull>()
        .add_message::<crate::ship_plugin::CoordinationEnqueue>()
        .add_systems(PostUpdate, collect);
    // Spawn the player ship entity so handle_navigation_waypoint can query it.
    app.world_mut().spawn((
        crate::server_app::Ship,
        crate::server_app::LocalShip,
        crate::server_app::ShipSystemBlackboards::default(),
        crate::ship_plugin::ShipConfigComponent::default(),
        crate::ship_plugin::ShipSystemControlSources::default(),
        crate::core::messages::AdmittedCommands::default(),
        crate::ship_plugin::ActiveStationRatings::default(),
        crate::ship_plugin::CoordinationQueue::default(),
        // PR 7 (issue #597) — NavigationWaypoint is now a per-entity Component.
        NavigationWaypoint::default(),
        ShipImpulse(crate::ship::impulse::ImpulseState::new()),
        crate::modifiers::ShipModifiers::new(),
        crate::ship::state::ShipPhysics::default(),
        // The AUTHORED `[navigation_console.selector]` block every shipped
        // hull carries. Since #885b stage 5d `operate_navigation_ai` has no
        // synthesised fallback — a ship with no selector ranks nothing — so
        // a fixture that wants a waypoint must attach the declaration a real
        // hull writes.
        NavigationTargetSelector {
            selector: crate::entities::authored_ai_pins::shipped_selector_toml("navigation")
                .to_selector()
                .expect("the shipped Navigation selector decodes"),
            power_rating: None,
        },
    ));
    // One fixed step per update (issue #895): the plugin's systems run on
    // the logical tick, and each harness tick advances it once.
    crate::ship::test_support::drive_one_fixed_step_per_update(
        &mut app,
        std::time::Duration::from_millis(200),
    );
    app
}

/// PR 7 test helper — read the LocalShip's `NavigationWaypoint` component.
fn get_nav_waypoint(app: &mut App) -> Option<WaypointMode> {
    let mut q = app
        .world_mut()
        .query_filtered::<&NavigationWaypoint, With<crate::server_app::LocalShip>>();
    q.single(app.world()).ok().and_then(|w| w.mode().cloned())
}

/// The LocalShip waypoint's current generation (issue #702).
fn nav_waypoint_generation(app: &mut App) -> u64 {
    let mut q = app
        .world_mut()
        .query_filtered::<&NavigationWaypoint, With<crate::server_app::LocalShip>>();
    q.single(app.world())
        .expect("LocalShip must carry a NavigationWaypoint")
        .generation()
}

fn push(app: &mut App, token: &str, msg: ClientMessage) {
    app.world_mut()
        .resource_mut::<Messages<InboundMessage>>()
        .write(InboundMessage {
            token: token.into(),
            msg,
        });
}

fn tick(app: &mut App) -> Vec<OutboundMessage> {
    app.update();
    let out = app.world().resource::<Outbox>().0.clone();
    app.world_mut().resource_mut::<Outbox>().0.clear();
    out
}

fn start_game_with_navigation(app: &mut App) {
    push(
        app,
        "captain",
        ClientMessage::Identify {
            token: "captain".into(),
            name: "Alice".into(),
        },
    );
    tick(app);
    push(
        app,
        "captain",
        ClientMessage::SelectStation {
            station: "Captain".into(),
        },
    );
    tick(app);
    push(
        app,
        "navigation",
        ClientMessage::Identify {
            token: "navigation".into(),
            name: "Decker".into(),
        },
    );
    tick(app);
    push(
        app,
        "navigation",
        ClientMessage::SelectStation {
            station: "Navigation".into(),
        },
    );
    tick(app);
    push(app, "captain", ClientMessage::SetReady { ready: true });
    push(app, "navigation", ClientMessage::SetReady { ready: true });
    tick(app);
}

fn latest_navigation_blackboard(
    out: &[OutboundMessage],
) -> Option<crate::core::messages::NavigationBlackboard> {
    out.iter().rev().find_map(|m| match &m.msg {
        ServerMessage::BlackboardUpdate { updates } => {
            updates.iter().find_map(|(_, bb)| match bb {
                crate::core::messages::SystemBlackboard::Navigation(nav) => Some(nav.clone()),
                _ => None,
            })
        }
        _ => None,
    })
}

/// **Issue #1028, AC4.** A civilian's lane, leg and compliance reach the
/// Navigation blackboard, so a console can show who is and is not doing as
/// asked — and a world with no traffic publishes exactly what it did before.
#[test]
fn the_navigation_blackboard_carries_the_civilian_traffic_picture() {
    use crate::civilian::{
        CivilianConfig, CivilianOrder, CivilianOrderOption, CivilianSection, CivilianState,
        CivilianTraffic, ComplianceDisposition,
    };

    // Read off the local ship's own blackboard rather than off the wire:
    // `broadcast_blackboard_updates` is diffed, so an unchanged picture is
    // deliberately not re-sent and the control below would have nothing to
    // look at.
    fn local_blackboard(app: &mut App) -> crate::core::messages::NavigationBlackboard {
        let mut q = app.world_mut().query_filtered::<
            &crate::server_app::ShipSystemBlackboards,
            With<crate::server_app::LocalShip>,
        >();
        let bbs = q
            .iter(app.world())
            .next()
            .expect("the local ship publishes");
        match bbs.0.get(&SystemId(NAVIGATION_SYSTEM_ID.to_string())) {
            Some(crate::core::messages::SystemBlackboard::Navigation(nav)) => nav.clone(),
            other => panic!("expected a navigation blackboard, got {other:?}"),
        }
    }

    let mut app = test_app();
    start_game_with_navigation(&mut app);
    tick(&mut app);
    assert!(
        local_blackboard(&mut app).civilians.is_empty(),
        "a world with no civilian traffic publishes the payload it always did"
    );

    // One hauler, ordered to dock and already complying.
    let config = CivilianConfig {
        route: Some("depot_run".into()),
        order_options: vec![CivilianOrderOption {
            id: "clear_lane".into(),
            label: "world.test.civilian.clear_lane".into(),
            order: CivilianOrder::divert_to_route("storm_shelter_run"),
        }],
        ..CivilianConfig::default()
    };
    let mut state = CivilianState::from_config(&config);
    let disposition = ComplianceDisposition {
        ack_secs: 0,
        decide_secs: 0,
        ..ComplianceDisposition::default()
    };
    state.receive_order(
        CivilianOrder::dock_at("world.entity.skyhook_depot.name"),
        &disposition,
        0,
        60.0,
    );
    state.advance(0, true, &disposition, 60.0);
    state.advance(0, true, &disposition, 60.0);
    app.world_mut().spawn((
        crate::entities::spawner::EntityUuid("civ-1".into()),
        crate::entities::spawner::EntityName("world.entity.hauler_kestrel.name".into()),
        CivilianSection(config),
        CivilianTraffic(state),
    ));

    let out = tick(&mut app);
    let bb =
        latest_navigation_blackboard(&out).expect("the changed traffic picture reaches the wire");
    assert_eq!(
        bb.civilians.len(),
        1,
        "the hauler is on the traffic picture"
    );
    let row = &bb.civilians[0];
    assert_eq!(
        row.uuid, "civ-1",
        "the row key is what an order names it by"
    );
    assert_eq!(row.name, "world.entity.hauler_kestrel.name");
    assert_eq!(row.order_options.len(), 1);
    assert_eq!(row.order_options[0].id, "clear_lane");
    assert_eq!(row.order_options[0].label, "world.test.civilian.clear_lane");
    assert_eq!(
        row.order_options[0].order,
        CivilianOrder::divert_to_route("storm_shelter_run")
    );
    assert_eq!(row.route, "depot_run");
    assert_eq!(row.order, "dock");
    assert_eq!(row.order_destination, "world.entity.skyhook_depot.name");
    assert_eq!(
        row.compliance, "complying",
        "the whole point of the row: whether it is doing as it was asked"
    );
}

#[test]
fn navigation_holder_can_set_and_clear_waypoint() {
    let mut app = test_app();
    start_game_with_navigation(&mut app);

    push(
        &mut app,
        "navigation",
        ClientMessage::ControlSystem {
            target: crate::core::messages::SystemId("navigation".into()),
            payload: SystemControlPayload::SetNavigationWaypoint {
                x: 120.0,
                z: -45.0,
                source_uuid: None,
            },
        },
    );
    tick(&mut app);
    assert_eq!(
        get_nav_waypoint(&mut app),
        Some(WaypointMode::Free { x: 120.0, z: -45.0 })
    );

    push(
        &mut app,
        "navigation",
        ClientMessage::ControlSystem {
            target: crate::core::messages::SystemId("navigation".into()),
            payload: SystemControlPayload::ClearNavigationWaypoint,
        },
    );
    tick(&mut app);
    assert!(get_nav_waypoint(&mut app).is_none());
}

#[test]
fn non_navigation_sender_cannot_change_waypoint() {
    let mut app = test_app();
    start_game_with_navigation(&mut app);

    push(
        &mut app,
        "captain",
        ClientMessage::ControlSystem {
            target: crate::core::messages::SystemId("navigation".into()),
            payload: SystemControlPayload::SetNavigationWaypoint {
                x: 5.0,
                z: 6.0,
                source_uuid: None,
            },
        },
    );
    tick(&mut app);
    assert!(get_nav_waypoint(&mut app).is_none());
}

#[test]
fn invalid_waypoint_coordinates_are_ignored() {
    let mut app = test_app();
    start_game_with_navigation(&mut app);

    push(
        &mut app,
        "navigation",
        ClientMessage::ControlSystem {
            target: crate::core::messages::SystemId("navigation".into()),
            payload: SystemControlPayload::SetNavigationWaypoint {
                x: f32::NAN,
                z: 1.0,
                source_uuid: None,
            },
        },
    );
    tick(&mut app);
    assert!(get_nav_waypoint(&mut app).is_none());
}

#[test]
fn sim_state_broadcast_includes_and_omits_waypoint() {
    let mut app = test_app();
    start_game_with_navigation(&mut app);

    push(
        &mut app,
        "navigation",
        ClientMessage::ControlSystem {
            target: crate::core::messages::SystemId("navigation".into()),
            payload: SystemControlPayload::SetNavigationWaypoint {
                x: 10.0,
                z: 20.0,
                source_uuid: None,
            },
        },
    );
    let out = tick(&mut app);
    let bb = latest_navigation_blackboard(&out).expect("expected NavigationBlackboard");
    assert_eq!(
        bb.navigation_waypoint,
        Some(WaypointSnapshot {
            x: 10.0,
            z: 20.0,
            source_uuid: None,
        })
    );

    push(
        &mut app,
        "navigation",
        ClientMessage::ControlSystem {
            target: crate::core::messages::SystemId("navigation".into()),
            payload: SystemControlPayload::ClearNavigationWaypoint,
        },
    );
    let out = tick(&mut app);
    let bb = latest_navigation_blackboard(&out).expect("expected NavigationBlackboard");
    assert!(bb.navigation_waypoint.is_none());
}

#[test]
fn anchored_waypoint_tracks_moving_entity() {
    let mut app = test_app();
    start_game_with_navigation(&mut app);

    // Spawn an entity carrying EntityUuid + Transform that the waypoint
    // will anchor to.
    let target_uuid = "target-1";
    let target = app
        .world_mut()
        .spawn((
            crate::entities::spawner::EntityUuid(target_uuid.into()),
            Transform::from_xyz(50.0, 0.0, -100.0),
        ))
        .id();

    // Anchor the waypoint to that entity. The seed coords are the
    // entity's current position.
    push(
        &mut app,
        "navigation",
        ClientMessage::ControlSystem {
            target: crate::core::messages::SystemId("navigation".into()),
            payload: SystemControlPayload::SetNavigationWaypoint {
                x: 50.0,
                z: -100.0,
                source_uuid: Some(target_uuid.into()),
            },
        },
    );
    let out = tick(&mut app);
    let bb = latest_navigation_blackboard(&out).expect("expected NavigationBlackboard");
    assert_eq!(
        bb.navigation_waypoint,
        Some(WaypointSnapshot {
            x: 50.0,
            z: -100.0,
            source_uuid: Some(target_uuid.into()),
        })
    );

    // Move the entity. The next broadcast should reflect the new
    // position with source_uuid preserved.
    app.world_mut()
        .entity_mut(target)
        .insert(Transform::from_xyz(75.0, 0.0, -150.0));
    let out = tick(&mut app);
    let bb = latest_navigation_blackboard(&out).expect("expected NavigationBlackboard");
    assert_eq!(
        bb.navigation_waypoint,
        Some(WaypointSnapshot {
            x: 75.0,
            z: -150.0,
            source_uuid: Some(target_uuid.into()),
        })
    );
}

#[test]
fn anchored_waypoint_auto_clears_when_parent_despawns() {
    let mut app = test_app();
    start_game_with_navigation(&mut app);

    let target_uuid = "target-despawn";
    let target = app
        .world_mut()
        .spawn((
            crate::entities::spawner::EntityUuid(target_uuid.into()),
            Transform::from_xyz(10.0, 0.0, 20.0),
        ))
        .id();

    push(
        &mut app,
        "navigation",
        ClientMessage::ControlSystem {
            target: crate::core::messages::SystemId("navigation".into()),
            payload: SystemControlPayload::SetNavigationWaypoint {
                x: 10.0,
                z: 20.0,
                source_uuid: Some(target_uuid.into()),
            },
        },
    );
    tick(&mut app);
    assert!(get_nav_waypoint(&mut app).is_some());

    // Despawn the parent entity. The next tick must auto-clear.
    app.world_mut().entity_mut(target).despawn();
    let out = tick(&mut app);
    assert!(get_nav_waypoint(&mut app).is_none());
    let bb = latest_navigation_blackboard(&out).expect("expected NavigationBlackboard");
    assert!(bb.navigation_waypoint.is_none());
}

#[test]
fn empty_source_uuid_is_treated_as_free_waypoint() {
    let mut app = test_app();
    start_game_with_navigation(&mut app);

    push(
        &mut app,
        "navigation",
        ClientMessage::ControlSystem {
            target: crate::core::messages::SystemId("navigation".into()),
            payload: SystemControlPayload::SetNavigationWaypoint {
                x: 1.0,
                z: 2.0,
                source_uuid: Some(String::new()),
            },
        },
    );
    tick(&mut app);
    assert_eq!(
        get_nav_waypoint(&mut app),
        Some(WaypointMode::Free { x: 1.0, z: 2.0 })
    );
}

// ── ControlSystem dispatch tests ─────────────────────────────────────────

/// Navigation holder sends `ControlSystem` waypoint — accepted.
#[test]
fn control_system_navigation_holder_can_set_and_clear_waypoint() {
    let mut app = test_app();
    start_game_with_navigation(&mut app);

    push(
        &mut app,
        "navigation",
        ClientMessage::ControlSystem {
            target: crate::core::messages::SystemId("navigation".into()),
            payload: SystemControlPayload::SetNavigationWaypoint {
                x: 200.0,
                z: -80.0,
                source_uuid: None,
            },
        },
    );
    tick(&mut app);
    assert_eq!(
        get_nav_waypoint(&mut app),
        Some(WaypointMode::Free { x: 200.0, z: -80.0 })
    );

    push(
        &mut app,
        "navigation",
        ClientMessage::ControlSystem {
            target: crate::core::messages::SystemId("navigation".into()),
            payload: SystemControlPayload::ClearNavigationWaypoint,
        },
    );
    tick(&mut app);
    assert!(get_nav_waypoint(&mut app).is_none());
}

/// Non-navigation sender sends `ControlSystem` waypoint — rejected.
#[test]
fn control_system_unauthorized_sender_rejected() {
    let mut app = test_app();
    start_game_with_navigation(&mut app);

    push(
        &mut app,
        "captain",
        ClientMessage::ControlSystem {
            target: crate::core::messages::SystemId("navigation".into()),
            payload: SystemControlPayload::SetNavigationWaypoint {
                x: 5.0,
                z: 6.0,
                source_uuid: None,
            },
        },
    );
    tick(&mut app);
    assert!(
        get_nav_waypoint(&mut app).is_none(),
        "non-navigation sender should be rejected"
    );
}

/// When navigation system is AI-controlled, `ControlSystem` waypoint is rejected.
#[test]
fn control_system_rejected_when_ai_controlled() {
    let mut app = test_app();
    start_game_with_navigation(&mut app);

    {
        let mut q = app.world_mut().query_filtered::<&mut crate::ship_plugin::ShipSystemControlSources, With<crate::server_app::LocalShip>>();
        for mut cs in q.iter_mut(app.world_mut()) {
            cs.0.set(
                crate::ship::system_registry::navigation_system_id(),
                crate::ship::control_source::ControlSource::Ai,
            );
        }
    }

    push(
        &mut app,
        "navigation",
        ClientMessage::ControlSystem {
            target: crate::core::messages::SystemId("navigation".into()),
            payload: SystemControlPayload::SetNavigationWaypoint {
                x: 99.0,
                z: 99.0,
                source_uuid: None,
            },
        },
    );
    tick(&mut app);
    assert!(
        get_nav_waypoint(&mut app).is_none(),
        "should reject waypoint when navigation is AI-controlled"
    );
}

/// Anchored waypoint set via `ControlSystem` still tracks the entity.
#[test]
fn control_system_anchored_waypoint_tracks_entity() {
    let mut app = test_app();
    start_game_with_navigation(&mut app);

    let target_uuid = "anchor-cs-test";
    let target = app
        .world_mut()
        .spawn((
            crate::entities::spawner::EntityUuid(target_uuid.into()),
            Transform::from_xyz(30.0, 0.0, -60.0),
        ))
        .id();

    push(
        &mut app,
        "navigation",
        ClientMessage::ControlSystem {
            target: crate::core::messages::SystemId("navigation".into()),
            payload: SystemControlPayload::SetNavigationWaypoint {
                x: 30.0,
                z: -60.0,
                source_uuid: Some(target_uuid.into()),
            },
        },
    );
    tick(&mut app);
    assert_eq!(
        get_nav_waypoint(&mut app),
        Some(WaypointMode::Anchored {
            source_uuid: target_uuid.into(),
            last_x: 30.0,
            last_z: -60.0,
        })
    );

    // Move entity — next tick should update last_x/last_z.
    app.world_mut()
        .entity_mut(target)
        .insert(Transform::from_xyz(40.0, 0.0, -70.0));
    tick(&mut app);
    assert_eq!(
        get_nav_waypoint(&mut app),
        Some(WaypointMode::Anchored {
            source_uuid: target_uuid.into(),
            last_x: 40.0,
            last_z: -70.0,
        })
    );
}

#[test]
fn control_system_set_navigation_waypoint_works() {
    let mut app = test_app();
    start_game_with_navigation(&mut app);

    push(
        &mut app,
        "navigation",
        ClientMessage::ControlSystem {
            target: crate::core::messages::SystemId("navigation".into()),
            payload: SystemControlPayload::SetNavigationWaypoint {
                x: 15.0,
                z: 25.0,
                source_uuid: None,
            },
        },
    );
    tick(&mut app);
    assert_eq!(
        get_nav_waypoint(&mut app),
        Some(WaypointMode::Free { x: 15.0, z: 25.0 })
    );
}

#[test]
fn control_system_clear_navigation_waypoint_works() {
    let mut app = test_app();
    start_game_with_navigation(&mut app);

    push(
        &mut app,
        "navigation",
        ClientMessage::ControlSystem {
            target: crate::core::messages::SystemId("navigation".into()),
            payload: SystemControlPayload::SetNavigationWaypoint {
                x: 5.0,
                z: 5.0,
                source_uuid: None,
            },
        },
    );
    tick(&mut app);
    push(
        &mut app,
        "navigation",
        ClientMessage::ControlSystem {
            target: crate::core::messages::SystemId("navigation".into()),
            payload: SystemControlPayload::ClearNavigationWaypoint,
        },
    );
    tick(&mut app);
    assert!(get_nav_waypoint(&mut app).is_none());
}

// ── Helpers for operate_navigation_ai integration tests ────────────────

fn set_navigation_control_source(
    app: &mut App,
    source: crate::ship::control_source::ControlSource,
) {
    let mut q = app.world_mut().query_filtered::<&mut crate::ship_plugin::ShipSystemControlSources, With<crate::server_app::LocalShip>>();
    for mut cs in q.iter_mut(app.world_mut()) {
        cs.0.set(crate::ship::system_registry::navigation_system_id(), source);
    }
}

fn inject_viewscreen_objective(
    app: &mut App,
    objectives: Vec<crate::core::messages::ScoredObjective>,
) {
    use crate::core::messages::{SystemBlackboard, ViewscreenBlackboard};
    use crate::server_app::ShipSystemBlackboards;

    let bb = ViewscreenBlackboard {
        scored_objectives: objectives,
        ..Default::default()
    };
    let mut q = app
        .world_mut()
        .query_filtered::<&mut ShipSystemBlackboards, With<crate::server_app::LocalShip>>();
    if let Ok(mut bbs) = q.single_mut(app.world_mut()) {
        bbs.0.insert(
            crate::ship::system_registry::viewscreen_system_id(),
            SystemBlackboard::Viewscreen(bb),
        );
    }
}

/// **Issue #1141, AC2/AC3.** A payload-bearing `Order` objective is translated
/// into the exact command the Navigation console emits, through admission. The
/// target stays in its authored-name form for the shared civilian applier to
/// resolve; neither this host nor the test writes `CivilianState` directly.
#[test]
fn civilian_order_ai_emits_the_console_order_through_admission() {
    use crate::civilian::{CivilianConfig, CivilianOrder, CivilianState, CivilianTraffic};
    use crate::core::messages::{
        AiDirective, ObjectiveSnapshot, ObjectiveSource, ObjectiveStatus, ScoredObjective,
        SystemAffinity,
    };

    let mut app = test_app();
    start_game_with_navigation(&mut app);
    set_navigation_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);

    let route = "storm_shelter_run";
    let target = "world.test.hauler.name";
    let uuid = "civilian-1141";
    let mut world = crate::world::config::WorldConfig::default();
    world.routes.push(crate::civilian::RouteConfig {
        id: route.into(),
        ..Default::default()
    });
    app.world_mut().insert_resource(world);
    app.world_mut()
        .resource_mut::<crate::world::server::WorldContentRuntime>()
        .name_to_uuid
        .insert(target.into(), uuid.into());
    app.world_mut().spawn((
        crate::entities::spawner::EntityUuid(uuid.into()),
        CivilianTraffic(CivilianState::from_config(&CivilianConfig::default())),
    ));
    inject_viewscreen_objective(
        &mut app,
        vec![ScoredObjective {
            id: "order-hauler".into(),
            score: 49.0,
            directive: AiDirective::Order {
                target: target.into(),
                route: route.into(),
            },
            source: ObjectiveSource::Mission,
            relevance: vec![SystemAffinity::Navigation],
            snapshot: ObjectiveSnapshot {
                id: "order-hauler".into(),
                text: "world.test.objective.order_hauler".into(),
                text_params: Default::default(),
                mandatory: false,
                status: ObjectiveStatus::Active,
                targets: vec![target.into()],
                source: ObjectiveSource::Mission,
            },
        }],
    );

    // Mission objectives are published onto every ship's viewscreen
    // blackboard. An AI-operated NPC must not become a second bridge crew and
    // fulfil the player's Navigation work when the LocalShip is human-held.
    let (npc_sources, npc_blackboards, npc_config) = {
        let mut q = app.world_mut().query_filtered::<(
            &crate::ship_plugin::ShipSystemControlSources,
            &crate::server_app::ShipSystemBlackboards,
            &crate::ship_plugin::ShipConfigComponent,
        ), With<crate::server_app::LocalShip>>();
        let (sources, blackboards, config) = q
            .single(app.world())
            .expect("the local ship carries the Navigation AI inputs");
        (sources.clone(), blackboards.clone(), config.clone())
    };
    let npc = app
        .world_mut()
        .spawn((
            crate::entities::spawner::EntityUuid("npc-navigation-1141".into()),
            npc_sources,
            npc_blackboards,
            npc_config,
            crate::core::messages::AdmittedCommands::default(),
        ))
        .id();

    crate::ai::cadence::arm_ai_tick(&mut app);
    tick(&mut app);
    let admitted = {
        let mut q = app.world_mut().query_filtered::<
            &crate::core::messages::AdmittedCommands,
            With<crate::server_app::LocalShip>,
        >();
        q.single(app.world())
            .expect("the local ship has an admission queue")
            .0
            .clone()
    };
    assert!(
        admitted.iter().any(|command| {
            command.target == crate::ship::system_registry::navigation_system_id()
                && command.payload
                    == SystemControlPayload::OrderCivilian {
                        target: target.into(),
                        order: CivilianOrder::divert_to_route(route),
                    }
        }),
        "Backfill must admit the exact target+route command a console button emits"
    );

    // Reset the observable target before checking the human-held path. Without
    // this reset the first AI order's idempotency latch could make a broken
    // source gate look inert.
    {
        let mut q = app
            .world_mut()
            .query::<(&crate::entities::spawner::EntityUuid, &mut CivilianTraffic)>();
        let (_, mut traffic) = q
            .iter_mut(app.world_mut())
            .find(|(entity_uuid, _)| entity_uuid.0 == uuid)
            .expect("the test civilian remains in the world");
        traffic.0 = CivilianState::from_config(&CivilianConfig::default());
    }

    // The same objective under a human-held Navigation system is inert. Re-arm
    // cadence explicitly so an empty queue proves the source gate, not a skipped
    // decision tick. The AI-operated NPC above sees the same mission objective,
    // so the target staying unordered also proves only the LocalShip can act on
    // player-crew Navigation work.
    set_navigation_control_source(&mut app, crate::ship::control_source::ControlSource::Human);
    crate::ai::cadence::arm_ai_tick(&mut app);
    tick(&mut app);
    let has_ai_order = {
        let mut q = app.world_mut().query_filtered::<
            &crate::core::messages::AdmittedCommands,
            With<crate::server_app::LocalShip>,
        >();
        q.single(app.world())
            .expect("the local ship has an admission queue")
            .0
            .iter()
            .any(|command| matches!(command.payload, SystemControlPayload::OrderCivilian { .. }))
    };
    assert!(
        !has_ai_order,
        "human-held Navigation must suppress the AI emitter"
    );
    let civilian_order = {
        let mut q = app
            .world_mut()
            .query::<(&crate::entities::spawner::EntityUuid, &CivilianTraffic)>();
        q.iter(app.world())
            .find(|(entity_uuid, _)| entity_uuid.0 == uuid)
            .and_then(|(_, traffic)| traffic.0.order().cloned())
    };
    assert_eq!(
        civilian_order, None,
        "an AI-operated NPC must not execute the human-held LocalShip's mission objective"
    );
    assert!(
        app.world()
            .entity(npc)
            .get::<crate::core::messages::AdmittedCommands>()
            .expect("the NPC has an admission queue")
            .0
            .iter()
            .all(|command| !matches!(command.payload, SystemControlPayload::OrderCivilian { .. })),
        "the player mission order must never enter an NPC ship's queue"
    );
}

fn spawn_test_entity(app: &mut App, uuid: &str, x: f32, z: f32) {
    app.world_mut().spawn((
        crate::entities::spawner::EntityUuid(uuid.into()),
        Transform::from_xyz(x, 0.0, z),
    ));
}

#[derive(Resource, Default)]
struct NavCoordCapture(Vec<crate::ship_plugin::CoordinationEnqueue>);

fn capture_nav_coord(
    mut reader: MessageReader<crate::ship_plugin::CoordinationEnqueue>,
    mut capture: ResMut<NavCoordCapture>,
) {
    for ev in reader.read() {
        capture.0.push(ev.clone());
    }
}

fn drain_nav_coord(app: &mut App) -> Vec<crate::ship_plugin::CoordinationEnqueue> {
    let msgs = app.world().resource::<NavCoordCapture>().0.clone();
    app.world_mut().resource_mut::<NavCoordCapture>().0.clear();
    msgs
}

#[test]
fn operate_navigation_ai_destroy_sets_anchored_waypoint_and_emits_navigate_to() {
    let mut app = test_app();
    start_game_with_navigation(&mut app);
    set_navigation_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);
    app.init_resource::<NavCoordCapture>()
        .add_systems(PostUpdate, capture_nav_coord);

    // Insert the entity within nav range (default 500).
    spawn_test_entity(&mut app, "target-entity", 400.0, 0.0);

    // Inject Destroy objective with score > 0.
    inject_viewscreen_objective(
        &mut app,
        vec![crate::core::messages::ScoredObjective {
            id: "destroy-test".into(),
            score: 80.0,
            directive: crate::core::messages::AiDirective::Destroy {
                target: "target-entity".into(),
            },
            source: crate::core::messages::ObjectiveSource::Mission,
            relevance: vec![
                crate::core::messages::SystemAffinity::Helm,
                crate::core::messages::SystemAffinity::Weapons,
            ],
            snapshot: crate::core::messages::ObjectiveSnapshot {
                id: "destroy-test".into(),
                text: "Destroy target".into(),
                text_params: Default::default(),
                mandatory: true,
                status: crate::core::messages::ObjectiveStatus::Active,
                targets: vec!["target-entity".into()],
                source: crate::core::messages::ObjectiveSource::Mission,
            },
        }],
    );

    tick(&mut app);

    // Check waypoint is set (Anchored).
    let wp = get_nav_waypoint(&mut app);
    assert!(
        matches!(wp, Some(WaypointMode::Anchored { .. })),
        "expected Anchored waypoint, got {:?}",
        wp
    );
    if let Some(WaypointMode::Anchored {
        source_uuid,
        last_x,
        last_z,
    }) = wp
    {
        assert_eq!(source_uuid, "target-entity");
        assert!((last_x - 400.0).abs() < 0.01);
        assert!((last_z - 0.0).abs() < 0.01);
    }

    // Check NavigateTo was emitted.
    let coords = drain_nav_coord(&mut app);
    let nav_to = coords.iter().find(|c| {
        matches!(
            &c.payload,
            crate::core::messages::CoordinationPayload::NavigateTo { .. }
        )
    });
    assert!(nav_to.is_some(), "expected NavigateTo coordination event");
}

#[test]
fn operate_navigation_ai_reach_sets_free_waypoint_and_emits_navigate_to() {
    let mut app = test_app();
    start_game_with_navigation(&mut app);
    set_navigation_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);
    app.init_resource::<NavCoordCapture>()
        .add_systems(PostUpdate, capture_nav_coord);

    // Insert a WorldConfig with an anchor.
    let mut wc = crate::world::config::WorldConfig::default();
    wc.anchors.insert("base".into(), [300.0, 0.0, -100.0]);
    app.world_mut().insert_resource(wc);

    inject_viewscreen_objective(
        &mut app,
        vec![crate::core::messages::ScoredObjective {
            id: "reach-test".into(),
            score: 70.0,
            directive: crate::core::messages::AiDirective::Reach {
                anchor: "base".into(),
            },
            source: crate::core::messages::ObjectiveSource::Mission,
            relevance: vec![crate::core::messages::SystemAffinity::Helm],
            snapshot: crate::core::messages::ObjectiveSnapshot {
                id: "reach-test".into(),
                text: "Reach base".into(),
                text_params: Default::default(),
                mandatory: true,
                status: crate::core::messages::ObjectiveStatus::Active,
                targets: vec![],
                source: crate::core::messages::ObjectiveSource::Mission,
            },
        }],
    );

    tick(&mut app);

    // Check waypoint is Free.
    let wp = get_nav_waypoint(&mut app);
    assert_eq!(
        wp,
        Some(WaypointMode::Free {
            x: 300.0,
            z: -100.0
        })
    );

    // Check NavigateTo was emitted.
    let coords = drain_nav_coord(&mut app);
    let nav_to = coords.iter().find(|c| {
        matches!(
            &c.payload,
            crate::core::messages::CoordinationPayload::NavigateTo { .. }
        )
    });
    assert!(nav_to.is_some(), "expected NavigateTo coordination event");
    if let Some(crate::core::messages::CoordinationPayload::NavigateTo { generation, x, z }) =
        nav_to.map(|c| &c.payload)
    {
        // The generation is the navigation contract the Helm latches on; it
        // must be the waypoint's own so the clearance can match.
        assert_eq!(
            *generation,
            nav_waypoint_generation(&mut app),
            "NavigateTo must carry the current waypoint's generation, or the                  Helm's clearance can never match and it will never fly it"
        );
        // `x` / `z` ride for the chatter popup's display only (issue #977) —
        // Rust no longer composes the English "waypoint (x, z)" label; the
        // client's `coordination.navigate.title` template formats them. They
        // are the waypoint's own coordinates.
        assert_eq!((*x, *z), (300.0, -100.0));
    }
}

/// The shared issuer sends exactly ONE `NavigateTo` per waypoint
/// generation while the order stays unlatched — never one per tick. The
/// old AI path re-enqueued every tick it ran; at a human helm every
/// delivery is a popup, so a per-tick loop would popup-spam the operator
/// and flood the coordination queue unboundedly.
#[test]
fn navigate_to_clearance_is_issued_once_per_generation_not_per_tick() {
    let mut app = test_app();
    start_game_with_navigation(&mut app);
    set_navigation_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);
    app.init_resource::<NavCoordCapture>()
        .add_systems(PostUpdate, capture_nav_coord);

    // Helm axes stay on their default Human control: the delivered order
    // can never latch, which is exactly the state a per-tick re-issue
    // loop would spam in.
    let mut wc = crate::world::config::WorldConfig::default();
    wc.anchors.insert("base".into(), [300.0, 0.0, -100.0]);
    app.world_mut().insert_resource(wc);
    inject_viewscreen_objective(
        &mut app,
        vec![crate::core::messages::ScoredObjective {
            id: "reach-test".into(),
            score: 70.0,
            directive: crate::core::messages::AiDirective::Reach {
                anchor: "base".into(),
            },
            source: crate::core::messages::ObjectiveSource::Mission,
            relevance: vec![crate::core::messages::SystemAffinity::Helm],
            snapshot: crate::core::messages::ObjectiveSnapshot {
                id: "reach-test".into(),
                text: "Reach base".into(),
                text_params: Default::default(),
                mandatory: true,
                status: crate::core::messages::ObjectiveStatus::Active,
                targets: vec![],
                source: crate::core::messages::ObjectiveSource::Mission,
            },
        }],
    );

    // Many ticks: `operate_navigation_ai` re-sets the same waypoint every
    // one of them, and the clearance never latches (human helm).
    let mut navigate_to_count = 0;
    for _ in 0..20 {
        tick(&mut app);
        navigate_to_count += drain_nav_coord(&mut app)
            .iter()
            .filter(|c| {
                matches!(
                    &c.payload,
                    crate::core::messages::CoordinationPayload::NavigateTo { .. }
                )
            })
            .count();
    }
    assert_eq!(
        navigate_to_count, 1,
        "exactly one NavigateTo per waypoint generation — a re-issue loop \
         at a human helm would popup-spam and grow the queue unboundedly"
    );
}

/// Same no-spam property on the human write path: one admitted
/// `SetNavigationWaypoint` while the helm is human-manned produces exactly
/// one `NavigateTo`, no matter how many ticks pass unlatched.
#[test]
fn human_set_waypoint_issues_one_navigate_to_while_helm_stays_human() {
    let mut app = test_app();
    start_game_with_navigation(&mut app);
    app.init_resource::<NavCoordCapture>()
        .add_systems(PostUpdate, capture_nav_coord);

    push(
        &mut app,
        "navigation",
        ClientMessage::ControlSystem {
            target: crate::core::messages::SystemId("navigation".into()),
            payload: SystemControlPayload::SetNavigationWaypoint {
                x: 120.0,
                z: -45.0,
                source_uuid: None,
            },
        },
    );
    let mut navigate_to_count = 0;
    for _ in 0..20 {
        tick(&mut app);
        navigate_to_count += drain_nav_coord(&mut app)
            .iter()
            .filter(|c| {
                matches!(
                    &c.payload,
                    crate::core::messages::CoordinationPayload::NavigateTo { .. }
                )
            })
            .count();
    }
    assert_eq!(
        navigate_to_count, 1,
        "one admitted waypoint set must issue exactly one NavigateTo"
    );
}

/// Rule-6 symmetry: a *human*-set waypoint issues the same Channel-3
/// `NavigateTo` clearance — carrying the waypoint's generation — that the
/// AI path issues (mirrors
/// `operate_navigation_ai_reach_sets_free_waypoint_and_emits_navigate_to`).
/// Without it, an AI Helm silently never follows a human-set waypoint:
/// `cleared_nav_waypoint` only releases a generation the clearance has
/// latched, and only this message ever latches one.
#[test]
fn human_set_waypoint_emits_navigate_to_with_current_generation() {
    let mut app = test_app();
    start_game_with_navigation(&mut app);
    app.init_resource::<NavCoordCapture>()
        .add_systems(PostUpdate, capture_nav_coord);

    push(
        &mut app,
        "navigation",
        ClientMessage::ControlSystem {
            target: crate::core::messages::SystemId("navigation".into()),
            payload: SystemControlPayload::SetNavigationWaypoint {
                x: 120.0,
                z: -45.0,
                source_uuid: None,
            },
        },
    );
    tick(&mut app);

    assert_eq!(
        get_nav_waypoint(&mut app),
        Some(WaypointMode::Free { x: 120.0, z: -45.0 })
    );

    let coords = drain_nav_coord(&mut app);
    let nav_to = coords
        .iter()
        .find(|c| {
            matches!(
                &c.payload,
                crate::core::messages::CoordinationPayload::NavigateTo { .. }
            )
        })
        .expect("a human-set waypoint must enqueue the same NavigateTo clearance the AI path does");
    assert_eq!(
        nav_to.target,
        crate::ship::system_registry::helm_station_key()
    );
    assert_eq!(
        nav_to.sender_origin,
        crate::ship::control_source::ControlSource::Human
    );
    let crate::core::messages::CoordinationPayload::NavigateTo { generation, .. } = &nav_to.payload
    else {
        unreachable!()
    };
    assert_eq!(
        *generation,
        nav_waypoint_generation(&mut app),
        "NavigateTo must carry the current waypoint's generation, or the \
         AI Helm's clearance can never match and it will never fly it"
    );
}

#[test]
fn operate_navigation_ai_patrol_sets_free_waypoint() {
    let mut app = test_app();
    start_game_with_navigation(&mut app);
    set_navigation_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);

    let mut wc = crate::world::config::WorldConfig::default();
    wc.anchors.insert("patrol_pt".into(), [200.0, 0.0, 50.0]);
    app.world_mut().insert_resource(wc);

    inject_viewscreen_objective(
        &mut app,
        vec![crate::core::messages::ScoredObjective {
            id: "patrol-test".into(),
            score: 60.0,
            directive: crate::core::messages::AiDirective::Patrol {
                anchors: vec!["patrol_pt".into()],
                loop_path: true,
            },
            source: crate::core::messages::ObjectiveSource::Mission,
            relevance: vec![crate::core::messages::SystemAffinity::Helm],
            snapshot: crate::core::messages::ObjectiveSnapshot {
                id: "patrol-test".into(),
                text: "Patrol area".into(),
                text_params: Default::default(),
                mandatory: false,
                status: crate::core::messages::ObjectiveStatus::Active,
                targets: vec![],
                source: crate::core::messages::ObjectiveSource::Mission,
            },
        }],
    );

    tick(&mut app);

    let wp = get_nav_waypoint(&mut app);
    assert_eq!(wp, Some(WaypointMode::Free { x: 200.0, z: 50.0 }));
}

/// Navigation resolves a Patrol from the objective's **active cursor
/// target**, not `anchors[0]` (issue #702).
///
/// This system was cursor-blind: it parked the waypoint on the first anchor
/// of the route and left it there for the whole patrol, so once the ship had
/// rounded its first waypoint Navigation was still telling the Helm to fly
/// to a leg it had finished laps ago. The cursor is the objective's own
/// record of where it is on its route — the same one `helm_patrol` steers
/// from and `advance_objective_cursors` advances — so reading it is what
/// makes the two consoles agree.
///
/// The route below needs two distinct anchors to tell the two behaviours
/// apart: with a one-anchor route (as
/// `operate_navigation_ai_patrol_sets_free_waypoint` uses) index 0 and the
/// cursor always agree, and a cursor-blind implementation passes.
#[test]
fn operate_navigation_ai_patrol_follows_the_objective_cursor() {
    let mut app = test_app();
    start_game_with_navigation(&mut app);
    set_navigation_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);

    let mut wc = crate::world::config::WorldConfig::default();
    wc.anchors.insert("leg_a".into(), [100.0, 0.0, 0.0]);
    wc.anchors.insert("leg_b".into(), [900.0, 0.0, -400.0]);
    app.world_mut().insert_resource(wc);

    inject_viewscreen_objective(
        &mut app,
        vec![crate::core::messages::ScoredObjective {
            id: "patrol-test".into(),
            score: 60.0,
            directive: crate::core::messages::AiDirective::Patrol {
                anchors: vec!["leg_a".into(), "leg_b".into()],
                loop_path: true,
            },
            source: crate::core::messages::ObjectiveSource::Mission,
            relevance: vec![crate::core::messages::SystemAffinity::Helm],
            snapshot: crate::core::messages::ObjectiveSnapshot {
                id: "patrol-test".into(),
                text: "Patrol area".into(),
                text_params: Default::default(),
                mandatory: false,
                status: crate::core::messages::ObjectiveStatus::Active,
                targets: vec![],
                source: crate::core::messages::ObjectiveSource::Mission,
            },
        }],
    );

    // The ship has already rounded leg_a; its cursor names leg_b.
    let ship = app
        .world_mut()
        .query_filtered::<Entity, With<crate::server_app::LocalShip>>()
        .single(app.world())
        .expect("LocalShip");
    let mut cursor = crate::ai::patrol_cursor::PatrolCursor::new("patrol-test");
    crate::ai::patrol_cursor::advance_cursor(
        &mut cursor,
        &["leg_a".to_string(), "leg_b".to_string()],
        true,
        [100.0, 0.0, 0.0], // sitting on leg_a
        &app.world()
            .resource::<crate::world::config::WorldConfig>()
            .anchors
            .clone(),
        crate::ai::WAYPOINT_ARRIVAL_RADIUS,
    );
    assert_eq!(cursor.index(), 1, "precondition: cursor must name leg_b");
    app.world_mut()
        .entity_mut(ship)
        .insert(crate::ai::server::ObjectiveCursors(vec![cursor]));

    tick(&mut app);

    assert_eq!(
        get_nav_waypoint(&mut app),
        Some(WaypointMode::Free {
            x: 900.0,
            z: -400.0
        }),
        "Navigation must place the waypoint on the cursor's current leg (leg_b), \
         not on the route's first anchor (leg_a) — a cursor-blind Navigation \
         keeps ordering the Helm back to a leg it has already flown"
    );
}

#[test]
fn operate_navigation_ai_no_objective_clears_waypoint() {
    let mut app = test_app();
    start_game_with_navigation(&mut app);
    set_navigation_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);

    // First set a waypoint to verify it gets cleared.
    {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut NavigationWaypoint, With<crate::server_app::LocalShip>>();
        if let Ok(mut wp) = q.single_mut(app.world_mut()) {
            wp.set(WaypointMode::Free { x: 500.0, z: 500.0 });
        }
    }
    assert!(
        get_nav_waypoint(&mut app).is_some(),
        "waypoint must be set before clearing test"
    );

    // Inject empty scored_objectives.
    inject_viewscreen_objective(&mut app, vec![]);

    tick(&mut app);

    assert!(
        get_nav_waypoint(&mut app).is_none(),
        "waypoint must be cleared when no objective"
    );
}

/// Helper: a single Helm-relevant `Destroy` objective naming `target`.
fn inject_destroy_objective(app: &mut App, target: &str) {
    inject_viewscreen_objective(
        app,
        vec![crate::core::messages::ScoredObjective {
            id: "destroy-far".into(),
            score: 80.0,
            directive: crate::core::messages::AiDirective::Destroy {
                target: target.into(),
            },
            source: crate::core::messages::ObjectiveSource::Mission,
            relevance: vec![crate::core::messages::SystemAffinity::Helm],
            snapshot: crate::core::messages::ObjectiveSnapshot {
                id: "destroy-far".into(),
                text: "Destroy far target".into(),
                text_params: Default::default(),
                mandatory: true,
                status: crate::core::messages::ObjectiveStatus::Active,
                targets: vec![target.into()],
                source: crate::core::messages::ObjectiveSource::Mission,
            },
        }],
    );
}

/// Navigation is the ship's chart, not a radar: it plots the whole system.
/// It used to cull candidates by `nav_chart_range` — a *display* extent read
/// off the local player's ship config and applied to NPCs — which defeated
/// the entire point of the Channel-3 handoff, whose job is to steer a
/// short-ranged Helm toward something it cannot see for itself.
#[test]
fn operate_navigation_ai_sets_waypoint_for_distant_target() {
    let mut app = test_app();
    start_game_with_navigation(&mut app);
    set_navigation_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);

    // Far beyond any chart range a hull declares (the largest authored is 800).
    spawn_test_entity(&mut app, "far-entity", 5000.0, 0.0);
    inject_destroy_objective(&mut app, "far-entity");

    tick(&mut app);

    let wp = get_nav_waypoint(&mut app).expect("distant target must still get a waypoint");
    let WaypointMode::Anchored { last_x, last_z, .. } = wp else {
        panic!("a Destroy target anchors to the entity, got {wp:?}");
    };
    assert_eq!(last_x, 5000.0);
    assert_eq!(last_z, 0.0);
}

/// `combat_test.toml` authors its assault doctrine as
/// `directive_target = "Starbase Alpha"` — a name, not a UUID. Matching on
/// UUID alone left it unresolvable, so Navigation cleared the waypoint and
/// the raider fell back to patrolling.
#[test]
fn operate_navigation_ai_resolves_destroy_target_by_entity_name() {
    let mut app = test_app();
    start_game_with_navigation(&mut app);
    set_navigation_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);

    let uuid = uuid::Uuid::new_v4().to_string();
    app.world_mut().spawn((
        crate::entities::spawner::EntityUuid(uuid.clone()),
        crate::entities::spawner::EntityName("Starbase Alpha".into()),
        Transform::from_xyz(500.0, 0.0, 100.0),
    ));
    inject_destroy_objective(&mut app, "Starbase Alpha");

    tick(&mut app);

    let wp = get_nav_waypoint(&mut app).expect("a name-authored Destroy must resolve");
    let WaypointMode::Anchored {
        source_uuid,
        last_x,
        last_z,
    } = wp
    else {
        panic!("a Destroy target anchors to the entity, got {wp:?}");
    };
    assert_eq!(last_x, 500.0);
    assert_eq!(last_z, 100.0);
    assert_eq!(
        source_uuid, uuid,
        "an Anchored waypoint tracks its parent by UUID, so it must store the \
         resolved UUID rather than the authored name"
    );
}

#[test]
fn operate_navigation_ai_clears_waypoint_for_unknown_destroy_target() {
    let mut app = test_app();
    start_game_with_navigation(&mut app);
    set_navigation_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);

    inject_destroy_objective(&mut app, "no-such-entity");

    tick(&mut app);

    assert!(
        get_nav_waypoint(&mut app).is_none(),
        "a Destroy naming nothing in the world resolves nowhere"
    );
}

#[test]
fn operate_navigation_ai_human_controlled_does_not_set_waypoint() {
    let mut app = test_app();
    start_game_with_navigation(&mut app);
    // Keep Navigation on Human control (default).
    // set_navigation_control_source is NOT called.

    let mut wc = crate::world::config::WorldConfig::default();
    wc.anchors.insert("some_anchor".into(), [100.0, 0.0, 0.0]);
    app.world_mut().insert_resource(wc);

    inject_viewscreen_objective(
        &mut app,
        vec![crate::core::messages::ScoredObjective {
            id: "reach-human".into(),
            score: 50.0,
            directive: crate::core::messages::AiDirective::Reach {
                anchor: "some_anchor".into(),
            },
            source: crate::core::messages::ObjectiveSource::Mission,
            relevance: vec![crate::core::messages::SystemAffinity::Helm],
            snapshot: crate::core::messages::ObjectiveSnapshot {
                id: "reach-human".into(),
                text: "Reach".into(),
                text_params: Default::default(),
                mandatory: false,
                status: crate::core::messages::ObjectiveStatus::Active,
                targets: vec![],
                source: crate::core::messages::ObjectiveSource::Mission,
            },
        }],
    );

    tick(&mut app);

    assert!(
        get_nav_waypoint(&mut app).is_none(),
        "human-controlled navigation must not set waypoints"
    );
}

/// Verifies operate_navigation_ai runs per-entity for AI-controlled ships (issue #592 AC).
#[test]
fn operate_navigation_ai_per_entity_ai_gate() {
    use crate::ship::control_source::{ControlSource, ControlSourceResolver};
    use crate::ship_plugin::ShipSystemControlSources;

    let mut ai_resolver = ControlSourceResolver::new();
    ai_resolver.set(
        crate::ship::system_registry::navigation_system_id(),
        ControlSource::Ai,
    );
    let ai_sources = ShipSystemControlSources(ai_resolver);
    let policy = ai_sources
        .0
        .policy_for(&crate::ship::system_registry::navigation_system_id());
    assert!(
        policy.operate_ai,
        "AI Navigation must gate through operate_ai"
    );

    // Human-controlled navigation must not operate AI.
    let mut human_resolver = ControlSourceResolver::new();
    human_resolver.set(
        crate::ship::system_registry::navigation_system_id(),
        ControlSource::Human,
    );
    let human_sources = ShipSystemControlSources(human_resolver);
    let human_policy = human_sources
        .0
        .policy_for(&crate::ship::system_registry::navigation_system_id());
    assert!(
        !human_policy.operate_ai,
        "Human Navigation must not operate AI"
    );
}

// ── #778 selector: replacement, clearing, chart-contact source ─────────

/// AC6 (replacement): swapping the active objective's destination makes the
/// selector pick the new one, and the published waypoint is replaced — the
/// same observable set-then-set path a human console drives.
#[test]
fn operate_navigation_ai_replaces_waypoint_when_objective_changes() {
    let mut app = test_app();
    start_game_with_navigation(&mut app);
    set_navigation_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);

    let mut wc = crate::world::config::WorldConfig::default();
    wc.anchors.insert("first".into(), [100.0, 0.0, 0.0]);
    wc.anchors.insert("second".into(), [-250.0, 0.0, 80.0]);
    app.world_mut().insert_resource(wc);

    let reach = |anchor: &str| {
        vec![crate::core::messages::ScoredObjective {
            id: "reach".into(),
            score: 70.0,
            directive: crate::core::messages::AiDirective::Reach {
                anchor: anchor.into(),
            },
            source: crate::core::messages::ObjectiveSource::Mission,
            relevance: vec![crate::core::messages::SystemAffinity::Helm],
            snapshot: crate::core::messages::ObjectiveSnapshot {
                id: "reach".into(),
                text: "Reach".into(),
                text_params: Default::default(),
                mandatory: true,
                status: crate::core::messages::ObjectiveStatus::Active,
                targets: vec![],
                source: crate::core::messages::ObjectiveSource::Mission,
            },
        }]
    };

    inject_viewscreen_objective(&mut app, reach("first"));
    tick(&mut app);
    assert_eq!(
        get_nav_waypoint(&mut app),
        Some(WaypointMode::Free { x: 100.0, z: 0.0 })
    );

    inject_viewscreen_objective(&mut app, reach("second"));
    tick(&mut app);
    assert_eq!(
        get_nav_waypoint(&mut app),
        Some(WaypointMode::Free { x: -250.0, z: 80.0 }),
        "the selector must replace the waypoint when the objective destination changes"
    );
}

/// AC2 / AC6: the `chart-contacts` source is genuinely wired — an authored
/// selector that admits chart contacts (the canonical default keys
/// eligibility on `reachable`, which they do not carry) selects a live
/// entity as an anchored destination with NO objective present at all,
/// exercising the "live entity-anchored destination" path through the
/// reusable selector.
#[test]
fn operate_navigation_ai_selects_chart_contact_when_author_widens_eligibility() {
    let mut app = test_app();
    start_game_with_navigation(&mut app);
    set_navigation_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);

    let cfg = crate::entities::config::FineSystemAiSelectorToml {
        param: Default::default(),
        sources: vec!["navigation-objectives".into(), "chart-contacts".into()],
        horizon: 1.0e9,
        switch_margin: 0.0,
        eligibility: "candidate_fact(source_chart_contact) > 0".into(),
        score: vec![crate::entities::config::ScoreTermToml {
            when: "candidate_fact(source_chart_contact) > 0".into(),
            weight: 1.0,
        }],
    };
    let selector = cfg.to_selector().expect("authored selector resolves");
    let ship = app
        .world_mut()
        .query_filtered::<Entity, With<crate::server_app::LocalShip>>()
        .single(app.world())
        .expect("LocalShip");
    app.world_mut()
        .entity_mut(ship)
        .insert(NavigationTargetSelector {
            selector,
            power_rating: None,
        });

    // No objective — only a live chart contact.
    inject_viewscreen_objective(&mut app, vec![]);
    spawn_test_entity(&mut app, "contact-1", 300.0, -120.0);

    tick(&mut app);

    assert_eq!(
        get_nav_waypoint(&mut app),
        Some(WaypointMode::Anchored {
            source_uuid: "contact-1".into(),
            last_x: 300.0,
            last_z: -120.0,
        }),
        "a widened selector must select a chart contact as an anchored destination"
    );
}

/// Issue #891 stage 2, per-host both-directions proof for the Navigation
/// target selector: an authored eligibility gated on a world flag selects
/// no destination while the flag is clear and anchors the chart contact
/// once it is set.
#[test]
fn operate_navigation_ai_flag_guard_reads_the_world_in_both_directions() {
    let mut app = test_app();
    start_game_with_navigation(&mut app);
    set_navigation_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);
    app.init_resource::<crate::world::server::WorldContentRuntime>();

    let cfg = crate::entities::config::FineSystemAiSelectorToml {
        param: Default::default(),
        sources: vec!["chart-contacts".into()],
        horizon: 1.0e9,
        switch_margin: 0.0,
        eligibility: "candidate_fact(source_chart_contact) > 0 and flag(survey_authorised)".into(),
        score: vec![crate::entities::config::ScoreTermToml {
            when: "candidate_fact(source_chart_contact) > 0".into(),
            weight: 1.0,
        }],
    };
    let selector = cfg.to_selector().expect("flag-gated selector resolves");
    let ship = app
        .world_mut()
        .query_filtered::<Entity, With<crate::server_app::LocalShip>>()
        .single(app.world())
        .expect("LocalShip");
    app.world_mut()
        .entity_mut(ship)
        .insert(NavigationTargetSelector {
            selector,
            power_rating: None,
        });
    inject_viewscreen_objective(&mut app, vec![]);
    spawn_test_entity(&mut app, "contact-891", 300.0, -120.0);

    // Flag CLEAR → nothing is eligible, no waypoint.
    tick(&mut app);
    assert_eq!(
        get_nav_waypoint(&mut app),
        None,
        "with the world flag clear the eligibility must admit no destination"
    );

    // Flag SET → the SAME eligibility anchors the chart contact.
    app.world_mut()
        .resource_mut::<crate::world::server::WorldContentRuntime>()
        .flags
        .set_flag("survey_authorised");
    tick(&mut app);
    assert_eq!(
        get_nav_waypoint(&mut app),
        Some(WaypointMode::Anchored {
            source_uuid: "contact-891".into(),
            last_x: 300.0,
            last_z: -120.0,
        }),
        "with the world flag set the same eligibility must select the contact"
    );
}

/// AC6 (lifecycle reset): a chart contact selected as a destination is
/// auto-cleared when its entity despawns — AI-authored anchored waypoints get
/// the same despawn-clear semantics as human-authored ones (AC4), because the
/// host keeps emitting the same admitted `SetNavigationWaypoint` and
/// `refresh_anchored_waypoint` is origin-blind.
#[test]
fn operate_navigation_ai_chart_contact_waypoint_auto_clears_on_despawn() {
    let mut app = test_app();
    start_game_with_navigation(&mut app);
    set_navigation_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);

    let cfg = crate::entities::config::FineSystemAiSelectorToml {
        param: Default::default(),
        sources: vec!["chart-contacts".into()],
        horizon: 1.0e9,
        switch_margin: 0.0,
        eligibility: "candidate_fact(source_chart_contact) > 0".into(),
        score: vec![crate::entities::config::ScoreTermToml {
            when: "candidate_fact(source_chart_contact) > 0".into(),
            weight: 1.0,
        }],
    };
    let selector = cfg.to_selector().expect("authored selector resolves");
    let ship = app
        .world_mut()
        .query_filtered::<Entity, With<crate::server_app::LocalShip>>()
        .single(app.world())
        .expect("LocalShip");
    app.world_mut()
        .entity_mut(ship)
        .insert(NavigationTargetSelector {
            selector,
            power_rating: None,
        });

    inject_viewscreen_objective(&mut app, vec![]);
    let contact = app
        .world_mut()
        .spawn((
            crate::entities::spawner::EntityUuid("contact-despawn".into()),
            Transform::from_xyz(150.0, 0.0, 40.0),
        ))
        .id();

    tick(&mut app);
    assert!(
        matches!(
            get_nav_waypoint(&mut app),
            Some(WaypointMode::Anchored { .. })
        ),
        "chart contact must be selected before the despawn"
    );

    app.world_mut().entity_mut(contact).despawn();
    tick(&mut app);
    assert!(
        get_nav_waypoint(&mut app).is_none(),
        "an AI-anchored waypoint must auto-clear when its entity despawns, like a human one"
    );
}

// ── Host teleport-to-waypoint override (issue #770) ────────────────────
//
// These exercise the host-only debug override against a real LocalShip
// entity: the query shape mirrors `drain_teleport_to_waypoint` (which is
// wasm-gated and so cannot run under native `cargo test`), and the move
// itself goes through the pure, testable `apply_teleport_to_waypoint`.

/// The teleport override snaps the LocalShip's authoritative planar position
/// onto the shared waypoint while leaving its altitude unchanged, and the
/// existence predicate the disable-gate reads is `Some` while a waypoint is
/// set.
#[test]
fn host_teleport_moves_local_ship_to_waypoint() {
    use super::apply_teleport_to_waypoint;

    let mut world = World::new();
    let ship = world
        .spawn((
            crate::server_app::LocalShip,
            crate::ship::state::ShipPhysics {
                x: 0.0,
                y: 5.0,
                z: 0.0,
                ..Default::default()
            },
            NavigationWaypoint::new(WaypointMode::Free { x: 111.0, z: 222.0 }),
        ))
        .id();

    // Existence predicate (AC2) — a waypoint is set.
    assert!(world
        .get::<NavigationWaypoint>(ship)
        .unwrap()
        .mode()
        .is_some());

    // Mirror `drain_teleport_to_waypoint`'s query + apply.
    let mut q = world.query_filtered::<(
        &mut crate::ship::state::ShipPhysics,
        &NavigationWaypoint,
    ), With<crate::server_app::LocalShip>>();
    for (mut physics, waypoint) in q.iter_mut(&mut world) {
        assert!(apply_teleport_to_waypoint(&mut physics, waypoint));
    }

    let physics = world.get::<crate::ship::state::ShipPhysics>(ship).unwrap();
    assert_eq!(physics.x, 111.0);
    assert_eq!(physics.z, 222.0);
    assert_eq!(physics.y, 5.0, "altitude must be preserved");
}

/// With no waypoint the existence predicate reads `None` (the panel disables
/// the control) and the teleport apply is a no-op.
#[test]
fn host_teleport_disabled_and_noop_without_waypoint() {
    use super::apply_teleport_to_waypoint;

    let mut world = World::new();
    let ship = world
        .spawn((
            crate::server_app::LocalShip,
            crate::ship::state::ShipPhysics {
                x: 3.0,
                y: 1.0,
                z: 4.0,
                ..Default::default()
            },
            NavigationWaypoint::default(),
        ))
        .id();

    // Existence predicate (AC2) — no waypoint, so the control is disabled.
    assert!(world
        .get::<NavigationWaypoint>(ship)
        .unwrap()
        .mode()
        .is_none());

    let mut q = world.query_filtered::<(
        &mut crate::ship::state::ShipPhysics,
        &NavigationWaypoint,
    ), With<crate::server_app::LocalShip>>();
    for (mut physics, waypoint) in q.iter_mut(&mut world) {
        assert!(!apply_teleport_to_waypoint(&mut physics, waypoint));
    }

    let physics = world.get::<crate::ship::state::ShipPhysics>(ship).unwrap();
    assert_eq!((physics.x, physics.y, physics.z), (3.0, 1.0, 4.0));
}
