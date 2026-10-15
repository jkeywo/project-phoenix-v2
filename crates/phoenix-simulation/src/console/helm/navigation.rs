//! Current own-axis Navigation permission, not another goal or controller.
use crate::{
    core::messages::{SystemBlackboard, SystemId},
    server_app::ShipSystemBlackboards,
    ship_plugin::ShipConfigComponent,
};
use bevy::prelude::Entity;
use phoenix_sim_gameplay::ship::helm_control::NavigationMotion;

pub(super) fn motion(
    knowledge: &crate::console::navigation::knowledge::Environment,
    entity: Entity,
    config: &ShipConfigComponent,
    boards: &ShipSystemBlackboards,
    axis: &SystemId,
    waypoint: Option<&crate::console::navigation::NavigationWaypoint>,
) -> Option<NavigationMotion> {
    motion_at(
        knowledge,
        entity,
        config,
        boards,
        axis,
        waypoint,
        knowledge.now()?,
    )
}

fn motion_at(
    knowledge: &crate::console::navigation::knowledge::Environment,
    entity: Entity,
    config: &ShipConfigComponent,
    boards: &ShipSystemBlackboards,
    axis: &SystemId,
    waypoint: Option<&crate::console::navigation::NavigationWaypoint>,
    at: u64,
) -> Option<NavigationMotion> {
    let waypoint = waypoint?;
    let canonical = match waypoint.mode()? {
        crate::console::navigation::WaypointMode::Formation { course } => course,
        _ => return None,
    };
    let permitted = config.0.observations_for_system(axis, &boards.0);
    let mut current = None;
    for system in config
        .0
        .systems
        .iter()
        .filter(|system| system.kind == crate::ship::system_registry::NAVIGATION_KIND)
    {
        let Some(SystemBlackboard::Navigation(board)) = permitted.get(&system.id) else {
            continue;
        };
        let Some(course) = board
            .formation
            .as_ref()
            .and_then(|view| view.course.as_ref())
        else {
            continue;
        };
        if board.course_generation == 0
            || board.course_generation != waypoint.generation()
            || course.paused
            || canonical.paused
            || course.owner != system.id
            || !course.same_intent(canonical)
        {
            continue;
        }
        // An explicit Navigation feed deliberately shares its earned sample.
        // The receiving axis must not correlate another instrument's alias or
        // read the leader's physical truth. Its dated sample never gains age
        // merely because the source has since received another observation.
        if !knowledge.formation_sample_current_at(entity, config, boards, course, at) {
            continue;
        }
        let motion = NavigationMotion {
            generation: board.course_generation,
            position: course.measurement.slot_point(&course.request)?,
            velocity: course.measurement.velocity,
        };
        if current.is_some_and(|held| held != motion) {
            return None;
        }
        current = Some(motion);
    }
    current
}

/// Same pure read-model projection for normal publication and a validated held
/// publication frontier. No actuator context, command or work is created.
pub(crate) fn availability_at(
    knowledge: &crate::console::navigation::knowledge::Environment,
    entity: Entity,
    config: &ShipConfigComponent,
    boards: &ShipSystemBlackboards,
    profile: &phoenix_model::helm_control::HelmControlConfig,
    waypoint: Option<&crate::console::navigation::NavigationWaypoint>,
    at: u64,
) -> (Option<u64>, Vec<SystemId>) {
    let inputs: std::collections::BTreeMap<_, _> = [&profile.thrust, &profile.steering]
        .into_iter()
        .chain(profile.lateral.iter())
        .filter_map(|axis| {
            motion_at(knowledge, entity, config, boards, axis, waypoint, at)
                .map(|motion| (axis.clone(), motion))
        })
        .collect();
    (
        inputs.values().next().map(|motion| motion.generation),
        inputs.into_keys().collect(),
    )
}
