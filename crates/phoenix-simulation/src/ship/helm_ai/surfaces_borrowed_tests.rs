// Frozen owned-input oracle from e13490cc; deliberately keep its copies.
use super::*;

fn owned_snapshot_entities(
    world_snapshot: Option<&crate::ai::server::WorldSnapshot>,
    runtime_ref: Option<&crate::world::server::WorldContentRuntime>,
    entity_fallback_q: &HelmAiFallbackQuery,
) -> Vec<crate::ai::AiWorldEntity> {
    if let Some(ws) = world_snapshot {
        return ws.entities.clone();
    }
    entity_fallback_q
        .iter()
        .map(|(uuid, transform, name, faction, hull, damage, collider)| {
            let runtime_name = runtime_ref.and_then(|rt| {
                rt.name_to_uuid
                    .iter()
                    .find_map(|(n, mapped)| (mapped == &uuid.0).then(|| n.clone()))
            });
            let hull_fraction = crate::ship::compartment_damage::structural_fraction(damage, hull);
            crate::ai::AiWorldEntity {
                uuid: uuid::Uuid::parse_str(&uuid.0).unwrap_or_default(),
                name: runtime_name.or_else(|| name.map(|n| n.0.clone())),
                position: [
                    transform.translation.x,
                    transform.translation.y,
                    transform.translation.z,
                ],
                faction: faction.map(|f| f.0),
                hull_fraction,
                yaw: Some(-transform.rotation.to_euler(EulerRot::YXZ).0),
                radius: collider.map(|c| c.0.radius).unwrap_or(0.0),
                // Mobility is the authored `[collider] movable` fact, matching
                // `ai::server::build_world_snapshot` (issue #958) so the
                // fallback picture and the production one classify terrain the
                // same way. Dangerous, with size rating tracking the collision
                // radius (issue #743).
                movable: collider.map(|c| c.0.movable).unwrap_or(false),
                dangerous: true,
                size_rating: collider.map(|c| c.0.radius).unwrap_or(0.0),
                ..Default::default()
            }
        })
        .collect()
}

/// Shared coordination work is restricted to the intersection of its readers.
fn owned_shared_helm_observations(
    config: Option<&crate::ship::config::ShipConfig>,
    sources: &ShipSystemControlSources,
    boards: &crate::server_app::ShipSystemBlackboards,
) -> crate::server_app::ShipSystemBlackboards {
    let Some(config) = config else {
        return Default::default();
    };
    let readers: Vec<_> = config
        .systems
        .iter()
        .filter(|system| {
            matches!(
                system.kind.as_str(),
                "helm_thrust"
                    | "helm_steering"
                    | "lateral_thrust"
                    | "vertical_thrust"
                    | "helm_impulse"
                    | "helm_boost"
            ) && sources.0.policy_for(&system.id).operate_ai
        })
        .collect();
    crate::server_app::ShipSystemBlackboards(
        boards
            .0
            .iter()
            .filter(|(source, _)| {
                !readers.is_empty()
                    && readers.iter().all(|system| {
                        config
                            .observations_for_system(&system.id, &boards.0)
                            .get(source)
                            .is_some()
                    })
            })
            .map(|(source, board)| (source.clone(), board.clone()))
            .collect(),
    )
}

#[allow(clippy::too_many_arguments)]
fn build_owned_helm_ai_surfaces_frame(
    world_snapshot: Option<Res<crate::ai::server::WorldSnapshot>>,
    world_config: Option<Res<crate::world::config::WorldConfig>>,
    chart: Option<Res<crate::lobby::server::WorldResource>>,
    runtime: Option<Res<crate::world::server::WorldContentRuntime>>,
    faction_registry: Option<Res<crate::entities::config_cache::FactionRegistryResource>>,
    ship_client_config: Option<Res<crate::lobby::server::ShipClientConfigResource>>,
    entity_fallback_q: HelmAiFallbackQuery,
    ships: Query<
        (
            Entity,
            &ShipSystemControlSources,
            &ShipPhysics,
            &crate::server_app::ShipSystemBlackboards,
            Option<&crate::ship::components::ShipConfigComponent>,
            Option<&crate::entities::spawner::EntityUuid>,
            Option<&crate::entities::spawner::FactionComponent>,
            Option<&crate::entities::spawner::ColliderSection>,
            Option<&crate::entities::spawner::HelmConsoleSection>,
            Has<crate::server_app::LocalShip>,
            HelmAiSurfaces,
            (
                Option<&crate::science::broad::SensingControls>,
                Option<&crate::science::broad::BroadProfile>,
                Option<&crate::science::broad::BroadTracks>,
            ),
            // Issue #875: the ship's own alert state, folded onto the frame so
            // every helm policy host seeds the same `posture` this tick.
            Option<&crate::ship::state::ShipRedAlert>,
        ),
        With<crate::ai::server::AiHighFidelity>,
    >,
    mut frame: ResMut<HelmAiSurfacesFrame>,
) {
    frame.anchors = world_config
        .as_ref()
        .map(|wc| wc.anchors.clone())
        .unwrap_or_default();
    frame.ships.clear();

    let snapshot_entities = owned_snapshot_entities(
        world_snapshot.as_deref(),
        runtime.as_deref(),
        &entity_fallback_q,
    );
    let default_registry = crate::ai::faction::FactionRegistry::default();
    let registry = faction_registry
        .as_deref()
        .map(|r| &r.0)
        .unwrap_or(&default_registry);

    for (
        entity,
        sources,
        physics,
        blackboards,
        ship_config,
        entity_uuid,
        faction,
        collider,
        helm_section,
        is_local,
        surfaces,
        (sensing_controls, broad_profile, broad_tracks),
        ship_red_alert,
    ) in ships.iter()
    {
        let red_alert = ship_red_alert.is_some_and(|r| r.0);
        let fitted =
            sensing_controls.is_some() || broad_profile.is_some() || broad_tracks.is_some();
        // Build only for ships some helm axis is actually flying: the frame
        // is a decision surface, and a fully human-held helm makes none.
        let any_axis_ai = [
            crate::ship::system_registry::helm_thrust_system_id(),
            crate::ship::system_registry::helm_steering_system_id(),
            crate::ship::system_registry::lateral_thrust_system_id(),
            crate::ship::system_registry::vertical_thrust_system_id(),
            crate::ship::system_registry::helm_impulse_system_id(),
        ]
        .iter()
        .any(|id| sources.0.policy_for(id).operate_ai);
        if !any_axis_ai {
            continue;
        }

        // The existing shared movement planner may only use knowledge available
        // to every participating axis. It never unions co-hosted Console feeds.
        // This temporary projection borrows existing publications at assembly;
        // no policy memory or authoritative observation store is replaced.
        let scoped =
            owned_shared_helm_observations(ship_config.map(|c| &c.0), sources, blackboards);
        let blackboards = &scoped;
        let has_navigation = blackboards.0.values().any(|board| {
            matches!(
                board,
                crate::core::messages::SystemBlackboard::Navigation(_)
            )
        });
        let waypoint = has_navigation.then_some(surfaces.waypoint).flatten();
        let scored = helm_ai_scored_objectives(blackboards);
        let navigation_generation = waypoint
            .filter(|waypoint| {
                matches!(
                    waypoint.mode(),
                    Some(
                        crate::console::navigation::WaypointMode::Formation { .. }
                            | crate::console::navigation::WaypointMode::ChartedXYZ { .. }
                    )
                )
            })
            .filter(|waypoint| cleared_nav_waypoint(Some(waypoint), surfaces.clearance).is_some())
            .map(|waypoint| waypoint.generation());
        let nav_waypoint_xyz = waypoint.and_then(|waypoint| {
            cleared_nav_waypoint(Some(waypoint), surfaces.clearance)?;
            let crate::console::navigation::WaypointMode::ChartedXYZ { course } =
                waypoint.mode()?
            else {
                return None;
            };
            let Some(crate::core::messages::SystemBlackboard::Navigation(board)) =
                blackboards.0.get(&course.owner)
            else {
                return None;
            };
            (board.course_generation == waypoint.generation()
                && board.charted_course.as_ref() == Some(course))
            .then_some(course.destination.point)
        });
        let navigation_generation = navigation_generation.filter(|_| {
            !matches!(
                waypoint.and_then(|waypoint| waypoint.mode()),
                Some(crate::console::navigation::WaypointMode::ChartedXYZ { .. })
            ) || nav_waypoint_xyz.is_some()
        });
        let has_objective = has_helm_objective(&scored) || navigation_generation.is_some();
        if !has_objective {
            // The axis systems still need the entry (to zero their axis /
            // stand impulse down correctly), but none of them reads a view
            // without a live objective — skip the expensive build.
            frame.ships.insert(
                entity,
                HelmAiShipFrame {
                    scored,
                    has_objective,
                    current_contact_routes: fitted.then(Default::default),
                    forward_speed: physics.forward_speed,
                    // Seeded on the objective-less path too: posture is a
                    // reading of the ship's own bridge, not of what it has been
                    // ordered to do, and a doctrine that holds a defensive line
                    // with no objective at all still has to know which line.
                    red_alert,
                    ..Default::default()
                },
            );
            continue;
        }

        let combat_lock = match blackboards
            .0
            .get(&crate::ship::system_registry::viewscreen_system_id())
        {
            Some(crate::core::messages::SystemBlackboard::Viewscreen(board)) => {
                board.combat_lock.as_deref()
            }
            _ => None,
        };
        let (
            visible_view,
            merged_view,
            destroy_target,
            weapons_target,
            current_contact_routes,
            observed_velocities,
            nav_waypoint_anchor,
        ) = if fitted {
            let measured = measured::build(
                blackboards,
                broad_tracks,
                sensing_controls,
                physics,
                entity_uuid,
                faction,
                collider,
                &frame.anchors,
                chart.as_deref(),
                runtime.as_deref(),
            );
            let mut shared = Vec::new();
            if let Some(crate::core::messages::SystemBlackboard::Viewscreen(board)) = blackboards
                .0
                .get(&crate::ship::system_registry::viewscreen_system_id())
            {
                for selected in [&board.combat_lock, &board.science_target] {
                    if let Some(key) = measured.selection(selected.as_deref()) {
                        if !shared.contains(&key) {
                            shared.push(key);
                        }
                    }
                }
            }
            let nav_anchor =
                cleared_nav_waypoint_anchor(waypoint, surfaces.clearance).and_then(|reference| {
                    measured
                        .selection(Some(&reference.to_string()))
                        .or_else(|| measured.named_target(&reference.to_string()))
                });
            if let Some(anchor) = nav_anchor {
                if !shared.contains(&anchor) {
                    shared.push(anchor);
                }
            }
            let destroy = measured.destroy_target(&scored, &shared, registry);
            let weapon = measured.selection(combat_lock);
            (
                measured.view.clone(),
                measured.view,
                destroy,
                weapon,
                Some(measured.routes),
                measured.velocities,
                nav_anchor,
            )
        } else {
            let mut visible = helm_ai_world_view(
                physics,
                entity_uuid,
                faction,
                collider,
                helm_section,
                blackboards,
                ship_client_config.as_deref(),
                is_local,
                &frame.anchors,
                &snapshot_entities,
            );
            if !blackboards
                .0
                .values()
                .any(|board| matches!(board, crate::core::messages::SystemBlackboard::Helm(_)))
            {
                visible.entities.clear();
            }
            let (merged, shared) =
                helm_shared_target_view(visible.clone(), &snapshot_entities, blackboards, waypoint);
            let destroy = helm_destroy_target(&scored, &merged, &shared, registry);
            (
                visible,
                merged,
                destroy,
                helm_weapons_target(combat_lock),
                None,
                Default::default(),
                cleared_nav_waypoint_anchor(waypoint, surfaces.clearance),
            )
        };
        let hostile_arc_exposure = crate::ai::hostile_arc_exposure(&merged_view, registry);

        frame.ships.insert(
            entity,
            HelmAiShipFrame {
                scored,
                has_objective,
                visible_view,
                merged_view,
                destroy_target,
                weapons_target,
                current_contact_routes,
                observed_velocities,
                nav_waypoint: cleared_nav_waypoint(waypoint, surfaces.clearance),
                nav_waypoint_xyz,
                navigation_generation,
                nav_waypoint_anchor,
                forward_speed: physics.forward_speed,
                hostile_arc_exposure,
                red_alert,
            },
        );
    }
}

// Exhaustive destructuring keeps the oracle comparison complete when a frame or
// view gains a field. JSON bytes also preserve signed zero in float payloads.
fn view_value(view: &crate::ai::WorldView) -> serde_json::Value {
    let crate::ai::WorldView {
        sim_time,
        entity_pos,
        entity_yaw,
        anchors,
        entities,
        attacker_this_tick,
        self_faction,
        entity_weapons_range,
        entity_phaser_ready,
        torpedo_tube_ready,
        self_hull_fraction,
        scenario_unloaded,
        self_radius,
        self_size_rating,
    } = view;
    let anchors: BTreeMap<_, _> = anchors.iter().collect();
    serde_json::json!({
        "sim_time":sim_time,"entity_pos":entity_pos,"entity_yaw":entity_yaw,
        "anchors":anchors,"entities":entities,"attacker_this_tick":attacker_this_tick,
        "self_faction":self_faction,"entity_weapons_range":entity_weapons_range,
        "entity_phaser_ready":entity_phaser_ready,"torpedo_tube_ready":torpedo_tube_ready,
        "self_hull_fraction":self_hull_fraction,"scenario_unloaded":scenario_unloaded,
        "self_radius":self_radius,"self_size_rating":self_size_rating,
    })
}

fn frame_value(frame: &HelmAiShipFrame) -> serde_json::Value {
    let HelmAiShipFrame {
        scored,
        has_objective,
        visible_view,
        merged_view,
        destroy_target,
        weapons_target,
        current_contact_routes,
        observed_velocities,
        nav_waypoint,
        nav_waypoint_xyz,
        navigation_generation,
        nav_waypoint_anchor,
        forward_speed,
        hostile_arc_exposure,
        red_alert,
    } = frame;
    let crate::weapons::arc_geometry::ArcExposure {
        covering_count,
        escape_offset_deg,
        inescapable,
    } = hostile_arc_exposure;
    serde_json::json!({
        "scored":scored,"has_objective":has_objective,
        "visible_view":view_value(visible_view),"merged_view":view_value(merged_view),
        "destroy_target":destroy_target,"weapons_target":weapons_target,
        "current_contact_routes":current_contact_routes,"observed_velocities":observed_velocities,
        "nav_waypoint":nav_waypoint,"nav_waypoint_xyz":nav_waypoint_xyz,
        "navigation_generation":navigation_generation,"nav_waypoint_anchor":nav_waypoint_anchor,
        "forward_speed":forward_speed,"red_alert":red_alert,
        "exposure":{"count":covering_count,"offset":escape_offset_deg,"inescapable":inescapable},
    })
}

fn complete_frame_bytes(world: &World) -> Vec<u8> {
    let HelmAiSurfacesFrame { anchors, ships } = world.resource::<HelmAiSurfacesFrame>();
    let anchors: BTreeMap<_, _> = anchors.iter().collect();
    let ships: BTreeMap<_, _> = ships
        .iter()
        .map(|(entity, frame)| (entity.to_bits().to_string(), frame_value(frame)))
        .collect();
    serde_json::to_vec(&serde_json::json!({"anchors":anchors,"ships":ships})).unwrap()
}

fn assert_frame_parity(world: &mut World, observer: Entity, label: &str) {
    let original = world
        .get::<crate::server_app::ShipSystemBlackboards>(observer)
        .unwrap()
        .clone();
    world.run_system_once(build_helm_ai_surfaces_frame).unwrap();
    let borrowed = complete_frame_bytes(world);
    world
        .run_system_once(build_owned_helm_ai_surfaces_frame)
        .unwrap();
    assert_eq!(
        borrowed,
        complete_frame_bytes(world),
        "{label}: complete owned frame oracle"
    );
    assert_eq!(
        world
            .get::<crate::server_app::ShipSystemBlackboards>(observer)
            .unwrap()
            .0,
        original.0,
        "{label}: frame assembly cannot edit observations"
    );
}

fn assert_projection_parity(world: &World, observer: Entity) {
    let config = world.get::<crate::ship::components::ShipConfigComponent>(observer);
    let sources = world.get::<ShipSystemControlSources>(observer).unwrap();
    let boards = world
        .get::<crate::server_app::ShipSystemBlackboards>(observer)
        .unwrap();
    let borrowed = shared_helm_observations(config.map(|config| &config.0), sources, boards);
    let owned = owned_shared_helm_observations(config.map(|config| &config.0), sources, boards);
    let actual: BTreeMap<_, _> = borrowed.iter().collect();
    let expected: BTreeMap<_, _> = owned.0.iter().collect();
    assert_eq!(
        serde_json::to_vec(&actual).unwrap(),
        serde_json::to_vec(&expected).unwrap()
    );
    for (source, board) in borrowed.iter() {
        assert!(
            std::ptr::eq(board, &boards.0[source]),
            "permitted board must be borrowed"
        );
        assert!(std::ptr::eq(borrowed.get(source).unwrap(), board));
    }
    for source in boards.0.keys() {
        assert_eq!(borrowed.get(source).is_some(), owned.0.contains_key(source));
    }
}

#[test]
fn borrowed_intersection_preserves_each_axis_scope_and_current_control_source() {
    use crate::ship::control_source::ControlSource;
    let (mut world, observer, owner, _) = rig();
    let config = world
        .get::<crate::ship::components::ShipConfigComponent>(observer)
        .unwrap()
        .0
        .clone();
    let axes: Vec<_> = config
        .systems
        .iter()
        .filter(|system| {
            matches!(
                system.kind.as_str(),
                "helm_thrust"
                    | "helm_steering"
                    | "lateral_thrust"
                    | "vertical_thrust"
                    | "helm_impulse"
                    | "helm_boost"
            )
        })
        .map(|system| system.id.clone())
        .collect();
    assert!(
        axes.len() >= 5,
        "accepted Cruiser must exercise several independent readers"
    );
    assert_projection_parity(&world, observer);
    assert_frame_parity(&mut world, observer, "all axes AI");
    for axis in axes {
        let mut isolated = config.clone();
        let mut console = isolated.console_for_system(&axis).unwrap().clone();
        console.id = phoenix_model::identity::ConsoleId("borrowed-private-axis".into());
        console.feeds.clear();
        isolated
            .systems
            .iter_mut()
            .find(|system| system.id == axis)
            .unwrap()
            .console = Some(console.id.clone());
        isolated.consoles.push(console);
        world
            .get_mut::<crate::ship::components::ShipConfigComponent>(observer)
            .unwrap()
            .0 = isolated;
        assert_projection_parity(&world, observer);
        assert_frame_parity(
            &mut world,
            observer,
            "one reader has no cross-console feeds",
        );
        let boards = world
            .get::<crate::server_app::ShipSystemBlackboards>(observer)
            .unwrap();
        let scoped = shared_helm_observations(
            Some(
                &world
                    .get::<crate::ship::components::ShipConfigComponent>(observer)
                    .unwrap()
                    .0,
            ),
            world.get::<ShipSystemControlSources>(observer).unwrap(),
            boards,
        );
        assert!(
            scoped.get(&owner).is_none(),
            "private axis must restrict the shared picture"
        );
        world
            .get_mut::<ShipSystemControlSources>(observer)
            .unwrap()
            .0
            .set(axis.clone(), ControlSource::Human);
        assert_projection_parity(&world, observer);
        assert_frame_parity(
            &mut world,
            observer,
            "human reader no longer joins intersection",
        );
        world
            .get_mut::<ShipSystemControlSources>(observer)
            .unwrap()
            .0
            .set(axis.clone(), ControlSource::Ai);
        world
            .get_mut::<ShipSystemControlSources>(observer)
            .unwrap()
            .0
            .set_offline(axis.clone(), true);
        assert_projection_parity(&world, observer);
        assert_frame_parity(
            &mut world,
            observer,
            "offline reader no longer joins intersection",
        );
        world
            .get_mut::<ShipSystemControlSources>(observer)
            .unwrap()
            .0
            .set_offline(axis.clone(), false);
        world
            .get_mut::<ShipSystemControlSources>(observer)
            .unwrap()
            .0
            .set(axis, ControlSource::Ai);
    }
    world
        .get_mut::<crate::ship::components::ShipConfigComponent>(observer)
        .unwrap()
        .0 = config;
    for system in world
        .get::<crate::ship::components::ShipConfigComponent>(observer)
        .unwrap()
        .0
        .systems
        .clone()
    {
        world
            .get_mut::<ShipSystemControlSources>(observer)
            .unwrap()
            .0
            .set(system.id, ControlSource::Human);
    }
    assert_projection_parity(&world, observer);
    assert_frame_parity(&mut world, observer, "no AI readers");
    assert!(world.resource::<HelmAiSurfacesFrame>().ships.is_empty());
    world
        .get_mut::<ShipSystemControlSources>(observer)
        .unwrap()
        .0
        .set(
            crate::ship::system_registry::helm_thrust_system_id(),
            ControlSource::Ai,
        );
    world
        .entity_mut(observer)
        .remove::<crate::ship::components::ShipConfigComponent>();
    assert_projection_parity(&world, observer);
    assert_frame_parity(&mut world, observer, "missing Console topology");
    assert!(!world.resource::<HelmAiSurfacesFrame>().ships[&observer].has_objective);
}

#[test]
fn borrowed_frame_matches_owned_measured_feeds_conflicts_prediction_and_live_gm_scope() {
    let (mut world, observer, owner, actual) = rig();
    assert_frame_parity(&mut world, observer, "measured baseline");
    assert_eq!(
        world.resource::<HelmAiSurfacesFrame>().ships[&observer]
            .visible_view
            .entities
            .len(),
        1
    );
    let copy = crate::ship::system_registry::tactical_radar_system_id();
    for console in &mut world
        .get_mut::<crate::ship::components::ShipConfigComponent>(observer)
        .unwrap()
        .0
        .consoles
    {
        console.feeds.push(phoenix_model::console::ConsoleFeed {
            source: copy.clone(),
            kind: phoenix_model::console::ConsoleObservationKind::TacticalRadar,
        });
    }
    world
        .get_mut::<crate::server_app::ShipSystemBlackboards>(observer)
        .unwrap()
        .0
        .insert(
            copy.clone(),
            crate::core::messages::SystemBlackboard::Sensors(Default::default()),
        );
    assert_projection_parity(&world, observer);
    assert_frame_parity(&mut world, observer, "wrong typed feed remains unavailable");
    let records = match &world
        .get::<crate::server_app::ShipSystemBlackboards>(observer)
        .unwrap()
        .0[&owner]
    {
        crate::core::messages::SystemBlackboard::Helm(board) => board.tracks.clone(),
        _ => panic!(),
    };
    world
        .get_mut::<crate::server_app::ShipSystemBlackboards>(observer)
        .unwrap()
        .0
        .insert(
            copy.clone(),
            crate::core::messages::SystemBlackboard::TacticalRadar(
                crate::core::messages::TacticalRadarBlackboard {
                    tracks: records,
                    ..Default::default()
                },
            ),
        );
    assert_projection_parity(&world, observer);
    assert_frame_parity(&mut world, observer, "identical explicit feed copies");
    assert_eq!(
        world.resource::<HelmAiSurfacesFrame>().ships[&observer]
            .visible_view
            .entities
            .len(),
        1
    );
    if let crate::core::messages::SystemBlackboard::TacticalRadar(board) = world
        .get_mut::<crate::server_app::ShipSystemBlackboards>(observer)
        .unwrap()
        .0
        .get_mut(&copy)
        .unwrap()
    {
        board.tracks.as_mut().unwrap()[0].position = [900.0, 999.0];
    }
    assert_frame_parity(&mut world, observer, "contradictory explicit feed copies");
    assert!(world.resource::<HelmAiSurfacesFrame>().ships[&observer]
        .visible_view
        .entities
        .is_empty());
    world
        .get_mut::<crate::server_app::ShipSystemBlackboards>(observer)
        .unwrap()
        .0
        .remove(&copy);
    let mut runtime = crate::world::server::WorldContentRuntime::default();
    crate::gm::operations::contact::set(
        &mut runtime.contact_overrides,
        &uuid::Uuid::from_u128(1).to_string(),
        &actual.to_string(),
        crate::gm::operations::contact::ContactMode::Conceal,
    );
    world.insert_resource(runtime);
    assert_frame_parity(
        &mut world,
        observer,
        "new GM conceal on retained observation",
    );
    assert!(world.resource::<HelmAiSurfacesFrame>().ships[&observer]
        .visible_view
        .entities
        .is_empty());
    world
        .entity_mut(observer)
        .remove::<crate::entities::spawner::EntityUuid>();
    assert_frame_parity(&mut world, observer, "missing observer with scoped GM data");
    world.remove_resource::<crate::world::server::WorldContentRuntime>();
    world
        .entity_mut(observer)
        .insert(crate::entities::spawner::EntityUuid(
            uuid::Uuid::from_u128(1).to_string(),
        ));
    let profile = world
        .get::<crate::science::broad::BroadProfile>(observer)
        .unwrap()
        .0[&owner]
        .profile
        .clone();
    world
        .get_mut::<crate::science::broad::BroadTracks>(observer)
        .unwrap()
        .0
        .get_mut(&owner)
        .unwrap()
        .advance(&profile, 2, 0.5, 1.0, &[])
        .unwrap();
    let predicted = world
        .get::<crate::science::broad::BroadTracks>(observer)
        .unwrap()
        .0[&owner]
        .projection(&profile, 2, 0.5)
        .unwrap();
    assert!(predicted[0].predicted);
    if let crate::core::messages::SystemBlackboard::Helm(board) = world
        .get_mut::<crate::server_app::ShipSystemBlackboards>(observer)
        .unwrap()
        .0
        .get_mut(&owner)
        .unwrap()
    {
        board.tracks = Some(predicted);
    }
    assert_frame_parity(
        &mut world,
        observer,
        "predicted observation supplies no current route",
    );
    assert!(world.resource::<HelmAiSurfacesFrame>().ships[&observer]
        .current_contact_routes
        .as_ref()
        .unwrap()
        .is_empty());
    if let crate::core::messages::SystemBlackboard::Viewscreen(board) = world
        .get_mut::<crate::server_app::ShipSystemBlackboards>(observer)
        .unwrap()
        .0
        .get_mut(&crate::ship::system_registry::viewscreen_system_id())
        .unwrap()
    {
        board.scored_objectives.clear();
    }
    assert_frame_parity(&mut world, observer, "no objective early exit");
    assert!(!world.resource::<HelmAiSurfacesFrame>().ships[&observer].has_objective);
}

#[test]
fn borrowed_frame_preserves_legacy_world_fallback_and_navigation_clearance() {
    use crate::core::messages::*;
    let (mut world, observer, owner, actual) = rig();
    world.entity_mut(observer).remove::<(
        crate::science::broad::SensingControls,
        crate::science::broad::BroadProfile,
        crate::science::broad::BroadTracks,
    )>();
    if let SystemBlackboard::Helm(board) = world
        .get_mut::<crate::server_app::ShipSystemBlackboards>(observer)
        .unwrap()
        .0
        .get_mut(&owner)
        .unwrap()
    {
        board.radar_range = 2000.0;
    }
    assert_frame_parity(&mut world, observer, "legacy actual WorldSnapshot");
    assert!(world.resource::<HelmAiSurfacesFrame>().ships[&observer]
        .visible_view
        .entities
        .iter()
        .any(|e| e.uuid == actual));
    world.remove_resource::<crate::ai::server::WorldSnapshot>();
    world.spawn((
        crate::entities::spawner::EntityUuid(actual.to_string()),
        crate::entities::spawner::EntityName("fallback-name".into()),
        Transform::from_xyz(71.0, 9.0, -23.0),
    ));
    assert_frame_parity(
        &mut world,
        observer,
        "absent snapshot uses actual ECS fallback",
    );
    assert_eq!(
        world.resource::<HelmAiSurfacesFrame>().ships[&observer]
            .visible_view
            .entities[0]
            .position,
        [71.0, 9.0, -23.0]
    );
    let navigation = SystemId("navigation".into());
    for console in &mut world
        .get_mut::<crate::ship::components::ShipConfigComponent>(observer)
        .unwrap()
        .0
        .consoles
    {
        console.feeds.push(phoenix_model::console::ConsoleFeed {
            source: navigation.clone(),
            kind: phoenix_model::console::ConsoleObservationKind::Navigation,
        });
    }
    world
        .get_mut::<crate::server_app::ShipSystemBlackboards>(observer)
        .unwrap()
        .0
        .insert(navigation, SystemBlackboard::Navigation(Default::default()));
    let waypoint = crate::console::navigation::NavigationWaypoint::new(
        crate::console::navigation::WaypointMode::Free { x: 25.0, z: -11.0 },
    );
    let generation = waypoint.generation();
    world
        .entity_mut(observer)
        .insert((waypoint, HelmWaypointClearance(Some(generation))));
    assert_frame_parity(
        &mut world,
        observer,
        "current Navigation feed and clearance",
    );
    assert_eq!(
        world.resource::<HelmAiSurfacesFrame>().ships[&observer].nav_waypoint,
        Some([25.0, -11.0])
    );
    world.get_mut::<HelmWaypointClearance>(observer).unwrap().0 = None;
    assert_frame_parity(
        &mut world,
        observer,
        "Navigation coordination lag remains missing",
    );
    assert_eq!(
        world.resource::<HelmAiSurfacesFrame>().ships[&observer].nav_waypoint,
        None
    );
}

#[test]
fn world_snapshot_storage_is_borrowed_only_when_present_and_fallback_matches_owned_oracle() {
    fn inspect(
        snapshot: Option<Res<crate::ai::server::WorldSnapshot>>,
        runtime: Option<Res<crate::world::server::WorldContentRuntime>>,
        query: HelmAiFallbackQuery,
    ) {
        let actual = helm_ai_snapshot_entities(snapshot.as_deref(), runtime.as_deref(), &query);
        let expected = owned_snapshot_entities(snapshot.as_deref(), runtime.as_deref(), &query);
        assert_eq!(actual.as_ref(), expected.as_slice());
        match (snapshot.as_deref(), actual) {
            (Some(snapshot), Cow::Borrowed(rows)) => {
                assert!(std::ptr::eq(rows, snapshot.entities.as_slice()))
            }
            (None, Cow::Owned(_)) => {}
            _ => panic!("only a present snapshot may supply borrowed backing"),
        }
    }
    let (mut world, _, _, _) = rig();
    world.run_system_once(inspect).unwrap();
    world.insert_resource(crate::ai::server::WorldSnapshot::default());
    world.run_system_once(inspect).unwrap();
    world.remove_resource::<crate::ai::server::WorldSnapshot>();
    world.spawn((
        crate::entities::spawner::EntityUuid(uuid::Uuid::from_u128(45).to_string()),
        crate::entities::spawner::EntityName("fallback".into()),
        Transform::from_xyz(7.0, -0.0, 11.0),
    ));
    world.run_system_once(inspect).unwrap();
}

#[test]
fn borrowed_scan_projection_preserves_canonical_instrument_and_current_typed_feed_rules() {
    use crate::core::messages::{ScanBlackboard, SystemBlackboard, SystemId};
    use phoenix_model::console::{ConsoleFeed, ConsoleObservationKind};
    let (mut world, observer, _, _) = rig();
    let authored = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    let instrument = authored
        .sensing
        .as_ref()
        .unwrap()
        .focused
        .first()
        .unwrap()
        .system
        .clone();
    let key = phoenix_model::sensing::scan_blackboard_key_for(&instrument);
    let permitted = |world: &World, source: &SystemId| {
        shared_helm_observations(
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
        )
        .get(source)
        .is_some()
    };
    for console in &mut world
        .get_mut::<crate::ship::components::ShipConfigComponent>(observer)
        .unwrap()
        .0
        .consoles
    {
        console.feeds.push(ConsoleFeed {
            source: key.clone(),
            kind: ConsoleObservationKind::Scan,
        });
    }
    world
        .get_mut::<crate::server_app::ShipSystemBlackboards>(observer)
        .unwrap()
        .0
        .insert(
            key.clone(),
            SystemBlackboard::Scan(ScanBlackboard {
                owner: Some(instrument.clone()),
                capable: true,
                ..Default::default()
            }),
        );
    assert!(permitted(&world, &key));
    assert_projection_parity(&world, observer);
    assert_frame_parity(&mut world, observer, "canonical generated Scan feed");

    if let SystemBlackboard::Scan(board) = world
        .get_mut::<crate::server_app::ShipSystemBlackboards>(observer)
        .unwrap()
        .0
        .get_mut(&key)
        .unwrap()
    {
        board.owner = Some(SystemId("other-instrument".into()));
    }
    assert!(!permitted(&world, &key));
    assert_projection_parity(&world, observer);
    assert_frame_parity(&mut world, observer, "mismatched Scan owner is withheld");
    if let SystemBlackboard::Scan(board) = world
        .get_mut::<crate::server_app::ShipSystemBlackboards>(observer)
        .unwrap()
        .0
        .get_mut(&key)
        .unwrap()
    {
        board.owner = Some(instrument.clone());
    }
    let malformed = SystemId(format!("scan/0{}:{}", instrument.0.len(), instrument.0));
    for console in &mut world
        .get_mut::<crate::ship::components::ShipConfigComponent>(observer)
        .unwrap()
        .0
        .consoles
    {
        console.feeds.push(ConsoleFeed {
            source: malformed.clone(),
            kind: ConsoleObservationKind::Scan,
        });
    }
    world
        .get_mut::<crate::server_app::ShipSystemBlackboards>(observer)
        .unwrap()
        .0
        .insert(
            malformed.clone(),
            SystemBlackboard::Scan(ScanBlackboard {
                owner: Some(instrument),
                capable: true,
                ..Default::default()
            }),
        );
    assert!(!permitted(&world, &malformed));
    assert!(permitted(&world, &key));
    assert_projection_parity(&world, observer);
    assert_frame_parity(
        &mut world,
        observer,
        "noncanonical Scan channel is withheld",
    );

    let mut config = world
        .get_mut::<crate::ship::components::ShipConfigComponent>(observer)
        .unwrap();
    let console = config
        .0
        .console_for_system(&crate::ship::system_registry::helm_thrust_system_id())
        .unwrap()
        .id
        .clone();
    config
        .0
        .consoles
        .iter_mut()
        .find(|candidate| candidate.id == console)
        .unwrap()
        .feeds
        .retain(|feed| feed.source != key);
    drop(config);
    assert!(!permitted(&world, &key));
    assert_projection_parity(&world, observer);
    assert_frame_parity(
        &mut world,
        observer,
        "withdrawn Scan feed takes effect on next assembly",
    );
    world
        .get_mut::<crate::ship::components::ShipConfigComponent>(observer)
        .unwrap()
        .0
        .consoles
        .iter_mut()
        .find(|candidate| candidate.id == console)
        .unwrap()
        .feeds
        .push(ConsoleFeed {
            source: key.clone(),
            kind: ConsoleObservationKind::Sensors,
        });
    assert!(!permitted(&world, &key));
    assert_projection_parity(&world, observer);
    assert_frame_parity(
        &mut world,
        observer,
        "wrong typed Scan feed remains withheld",
    );
}
