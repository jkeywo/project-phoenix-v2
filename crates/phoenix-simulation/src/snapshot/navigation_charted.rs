//! Validate the retained original fixed chart course, never refresh its knowledge.
use super::*;

fn validate(
    world: &World,
    live: Option<Entity>,
    row: &EntityState,
    tick: u64,
    original_chart: Option<&crate::core::messages::WorldData>,
) -> Result<(), String> {
    let Some(saved) = &row.navigation_waypoint else {
        return if has_published_course(row) {
            Err("retained charted publication lacks its canonical Navigation waypoint".into())
        } else {
            Ok(())
        };
    };
    let Some(course) = &saved.charted else {
        return if has_published_course(row) {
            Err("retained charted publication lacks its canonical fixed XYZ course".into())
        } else {
            Ok(())
        };
    };
    if !course.valid_at(tick)
        || saved.generation == 0
        || saved.formation.is_some()
        || saved.position != Some([course.destination.point[0], course.destination.point[2]])
        || saved.source_uuid.as_deref() != Some(course.destination.id.as_str())
    {
        return Err("saved charted XYZ course has invalid identity, point or clock".into());
    }
    let accepted = row.spawn.as_ref().and_then(|origin| {
        origin.resolve(
            &crate::entities::loader::WasmTemplateLoader,
            &mut Vec::new(),
        )
    });
    let config = live
        .and_then(|entity| {
            world
                .get::<crate::ship_plugin::ShipConfigComponent>(entity)
                .cloned()
        })
        .or_else(|| {
            accepted
                .as_ref()?
                .ship_config
                .clone()
                .map(crate::ship_plugin::ShipConfigComponent)
        })
        .ok_or("charted XYZ course requires actual Navigation composition")?;
    let capability = live
        .and_then(|entity| {
            world
                .get::<crate::entities::spawner::HelmCapabilitySection>(entity)
                .map(|section| section.0.vertical_movement_mode)
        })
        .or_else(|| {
            accepted
                .as_ref()?
                .helm_capability
                .as_ref()
                .map(|capability| capability.vertical_movement_mode)
        });
    let profile = live
        .and_then(|entity| {
            world
                .get::<crate::console::navigation::sharing::NavigationSharingProfile>(entity)
                .cloned()
        })
        .or_else(|| {
            let nav = accepted.as_ref()?.navigation_console.as_ref()?;
            Some(
                crate::console::navigation::sharing::NavigationSharingProfile {
                    owner: nav.sharing.as_ref()?.system.clone(),
                    radar: nav.system_chart.clone(),
                },
            )
        });
    if capability != Some(crate::entities::config::VerticalMovementMode::Full3D)
        || crate::console::navigation::server::navigation_owner(Some(&config), profile.as_ref())
            .as_ref()
            != Some(&course.owner)
    {
        return Err(
            "charted XYZ course disagrees with actual Navigation or movement equipment".into(),
        );
    }
    if row.blackboards.iter().any(|(source, board)| {
        matches!(board, crate::core::messages::SystemBlackboard::Navigation(board)
            if board.charted_course.as_ref().is_some_and(|published| source != &course.owner.0 || published != course))
    }) {
        return Err("retained charted publication disagrees with its sole canonical owner/course".into());
    }
    let held = row
        .blackboards
        .iter()
        .find_map(|(owner, board)| {
            if owner != &course.owner.0 {
                return None;
            }
            match board {
                crate::core::messages::SystemBlackboard::Navigation(board) => Some(board),
                _ => None,
            }
        })
        .ok_or("charted XYZ course lacks its retained own Navigation publication")?;
    if held.course_generation != saved.generation
        || held.charted_course.as_ref() != Some(course)
        || held.navigation_waypoint.as_ref().is_none_or(|point| {
            point.x != course.destination.point[0]
                || point.z != course.destination.point[2]
                || point.source_uuid.as_deref() != Some(course.destination.id.as_str())
        })
    {
        return Err(
            "saved charted XYZ course differs from its original held Navigation point".into(),
        );
    }
    // Use the candidate's own retained original chart, never current foreign
    // physical truth or a different fresh host's bootstrap chart. A removed
    // source has no retained original evidence to authenticate.
    if original_chart
        .and_then(|chart| {
            chart
                .entities
                .iter()
                .find(|entity| entity.uuid == course.destination.id)
        })
        .and_then(|entity| entity.position)
        .is_some_and(|position| position != course.destination.point)
    {
        return Err(
            "saved fixed XYZ point disagrees with its retained original chart record".into(),
        );
    }
    // The original fixed point can outlive its offer. A current chart withdrawal,
    // GM concealment or removed source neither grants a fresh observation nor
    // erases the legitimate fixed course. No foreign live pose is consulted.
    Ok(())
}
fn has_published_course(row: &EntityState) -> bool {
    row.blackboards.iter().any(|(_, board)| matches!(board,
        crate::core::messages::SystemBlackboard::Navigation(board) if board.charted_course.is_some()))
}
pub(super) fn validate_roster(
    world: &World,
    snapshot: &PhoenixSnapshot,
) -> Result<(), (String, String)> {
    for row in &snapshot.entities {
        if row
            .navigation_waypoint
            .as_ref()
            .is_none_or(|saved| saved.charted.is_none())
            && !has_published_course(row)
        {
            continue;
        }
        if !world.contains_resource::<SimTick>()
            || !world.contains_resource::<Time<bevy::time::Fixed>>()
        {
            return Err((
                row.uuid.clone(),
                "charted course continuation requires its logical/fixed clocks".into(),
            ));
        }
        let live = world.try_query_filtered::<(Entity, &EntityUuid), Allow<bevy::ecs::entity_disabling::Disabled>>()
            .and_then(|mut query| query.iter(world).find_map(|(entity, uuid)| (uuid.0 == row.uuid).then_some(entity)));
        validate(world, live, row, snapshot.tick, snapshot.world.as_ref())
            .map_err(|reason| (row.uuid.clone(), reason))?;
    }
    Ok(())
}
pub(super) fn validate_live(
    world: &World,
    entity: Entity,
    row: &EntityState,
) -> Result<(), String> {
    if row
        .navigation_waypoint
        .as_ref()
        .is_none_or(|saved| saved.charted.is_none())
        && !has_published_course(row)
    {
        return Ok(());
    }
    let tick = world
        .get_resource::<SimTick>()
        .ok_or("charted course requires its logical clock")?
        .0;
    validate(
        world,
        Some(entity),
        row,
        tick,
        world
            .get_resource::<crate::lobby::WorldResource>()
            .map(|chart| &chart.0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::console::navigation::{NavigationWaypoint, WaypointMode};
    use crate::core::messages::{NavigationBlackboard, SystemBlackboard, SystemId};
    use phoenix_model::navigation_point::{ChartedNavigationCourse, NavigationPointView};

    fn fixture() -> (World, Entity, PhoenixSnapshot) {
        let accepted = crate::entities::include_resolve::load_entity_config(
            "assets/entities/utility_shuttle.toml",
        )
        .unwrap();
        let mut world = World::new();
        world.insert_resource(SimTick(20));
        world.insert_resource(Time::<Fixed>::default());
        world.register_component::<EntitySystemHull>();
        world.register_component::<EntityShipArcHull>();
        world.register_component::<crate::console::repair::server::RepairRequestQueue>();
        world.register_component::<crate::ship_plugin::RepairHumanAlerted>();
        world.register_component::<crate::ai::server::ObjectiveCursors>();
        world.register_component::<ActiveBeam>();
        world.register_component::<PhaserCooldown>();
        world.register_component::<TorpedoSystemResource>();
        world.register_component::<crate::console::weapons::blaster::BlasterSystemResource>();
        world.register_component::<crate::ship::shields::ShipShields>();
        world.register_component::<crate::console::repair::server::ShipRepairTeams>();
        world.register_component::<crate::console::navigation::NavClearanceIssueState>();
        world.register_component::<crate::ship_plugin::HelmWaypointClearance>();
        world.register_component::<crate::console::command::server::ShipStationStances>();
        let entity = crate::entities::spawner::spawn_entity(
            &mut world.commands(),
            &accepted,
            Vec3::ZERO,
            "charted-owner".into(),
            None,
        );
        world.flush();
        let course = ChartedNavigationCourse {
            owner: SystemId("navigation".into()),
            destination: NavigationPointView {
                id: "original-public-station".into(),
                label: "public.station".into(),
                point: [10.25, 18.5, -23.75],
            },
            selected_tick: 19,
        };
        world.insert_resource(crate::lobby::WorldResource(
            crate::core::messages::WorldData {
                entities: vec![crate::core::messages::EntitySnapshot {
                    uuid: course.destination.id.clone(),
                    name: Some(course.destination.label.clone()),
                    position: Some(course.destination.point),
                    tags: vec![
                        crate::console_delivery::CHARTED_TAG.into(),
                        "station".into(),
                    ],
                    ..Default::default()
                }],
                ..Default::default()
            },
        ));
        let waypoint = NavigationWaypoint::new(WaypointMode::ChartedXYZ {
            course: course.clone(),
        });
        let board = NavigationBlackboard {
            course_generation: waypoint.generation(),
            charted_course: Some(course),
            navigation_waypoint: waypoint.snapshot(),
            ..Default::default()
        };
        world.entity_mut(entity).insert(waypoint);
        world
            .get_mut::<crate::server_app::ShipSystemBlackboards>(entity)
            .unwrap()
            .0
            .insert(
                SystemId("navigation".into()),
                SystemBlackboard::Navigation(board),
            );
        let snapshot = capture(&world);
        assert!(snapshot
            .entities
            .iter()
            .find(|row| row.uuid == "charted-owner")
            .unwrap()
            .navigation_waypoint
            .as_ref()
            .unwrap()
            .charted
            .is_some());
        (world, entity, snapshot)
    }
    #[test]
    fn original_fixed_chart_course_json_and_existing_apply_keep_generation_without_relearning_chart(
    ) {
        let (mut world, entity, snapshot) = fixture();
        let saved: PhoenixSnapshot =
            serde_json::from_slice(&serde_json::to_vec(&snapshot).unwrap()).unwrap();
        validate_roster(&world, &saved).unwrap();
        let mut withdrawn = saved.clone();
        withdrawn.world.as_mut().unwrap().entities.clear();
        validate_roster(&world, &withdrawn).unwrap();
        let row = saved
            .entities
            .iter()
            .find(|row| row.uuid == "charted-owner")
            .unwrap();
        let expected = row.navigation_waypoint.as_ref().unwrap();
        world.get_mut::<NavigationWaypoint>(entity).unwrap().clear();
        super::super::application::apply_navigation_waypoint(
            &mut world.entity_mut(entity),
            expected,
        );
        let restored = capture(&world);
        assert_eq!(
            restored
                .entities
                .iter()
                .find(|row| row.uuid == "charted-owner")
                .unwrap()
                .navigation_waypoint
                .as_ref(),
            Some(expected)
        );
        assert_eq!(restored.entities, saved.entities);
    }
    #[test]
    fn malformed_charted_owner_clock_point_frontier_and_publication_refuse_whole_restore_without_writes(
    ) {
        let (mut world, _, saved) = fixture();
        validate_roster(&world, &saved).unwrap();
        let before = capture(&world);
        let digest = crate::sim_digest::world_digest(&world);
        for fault in 0..10 {
            let mut bad = saved.clone();
            let row = bad
                .entities
                .iter_mut()
                .find(|row| row.uuid == "charted-owner")
                .unwrap();
            let waypoint = row.navigation_waypoint.as_mut().unwrap();
            match fault {
                0 => waypoint.charted.as_mut().unwrap().owner = SystemId("comms".into()),
                1 => waypoint.charted.as_mut().unwrap().selected_tick = saved.tick + 1,
                2 => waypoint.charted.as_mut().unwrap().destination.point[1] = 90.0,
                3 => waypoint.generation = 0,
                4 => waypoint.source_uuid = Some("foreign-private-actor".into()),
                5 => row.blackboards.retain(|(owner, _)| owner != "navigation"),
                6 => waypoint.charted = None,
                7 => row.navigation_waypoint = None,
                8 => {
                    let board = row
                        .blackboards
                        .iter_mut()
                        .find_map(|(_, board)| match board {
                            SystemBlackboard::Navigation(board) => Some(board),
                            _ => None,
                        })
                        .unwrap();
                    board.charted_course = None;
                }
                _ => {
                    waypoint.charted.as_mut().unwrap().destination.point[1] = 90.0;
                    let board = row
                        .blackboards
                        .iter_mut()
                        .find_map(|(_, board)| match board {
                            SystemBlackboard::Navigation(board) => Some(board),
                            _ => None,
                        })
                        .unwrap();
                    board.charted_course.as_mut().unwrap().destination.point[1] = 90.0;
                }
            }
            assert!(
                validate_roster(&world, &bad).is_err(),
                "exact charted owner guard must reject fault {fault}"
            );
            let report = restore(&mut world, &bad);
            assert!(
                !report.is_complete(),
                "forged charted course {fault} must refuse"
            );
            assert_eq!(capture(&world), before);
            assert_eq!(crate::sim_digest::world_digest(&world), digest);
        }
    }
}
