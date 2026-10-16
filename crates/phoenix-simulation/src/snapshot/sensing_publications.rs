//! Keep an acquired radar view at its recorded publication frontier. Fixed
//! publishers precede the logical-tick advance; rebuilding at the saved next
//! tick must not silently age the Console's existing knowledge during restore.
use super::PhoenixSnapshot;
use crate::core::messages::{SystemBlackboard, SystemId};
use crate::entities::spawner::EntityUuid;
use crate::science::broad::{BroadProfile, BroadTracks};
use crate::server_app::ShipSystemBlackboards;
use crate::ship::system_registry::{HELM_RADAR_KIND, SENSOR_RADAR_KIND, TACTICAL_RADAR_KIND};
use crate::ship_plugin::ShipConfigComponent;
use bevy::prelude::*;
use phoenix_model::sensing_tracks::SensorTrackSnapshot;

fn kind(board: &SystemBlackboard) -> Option<&'static str> {
    match board {
        SystemBlackboard::SensorRadar(_) => Some(SENSOR_RADAR_KIND),
        SystemBlackboard::TacticalRadar(_) => Some(TACTICAL_RADAR_KIND),
        SystemBlackboard::Helm(_) => Some(HELM_RADAR_KIND),
        _ => None,
    }
}

fn tracks_mut(board: &mut SystemBlackboard) -> Option<&mut Vec<SensorTrackSnapshot>> {
    match board {
        SystemBlackboard::SensorRadar(board) => board.tracks.as_mut(),
        SystemBlackboard::TacticalRadar(board) => board.tracks.as_mut(),
        SystemBlackboard::Helm(board) => board.tracks.as_mut(),
        _ => None,
    }
}

/// Rebuild missing views first, then retain only saved measurements that agree
/// with the actual source at this frontier. No selection, machinery intent,
/// control authority, clock or physical association is copied back. Navigation
/// availability is separately recomputed against its held frontier below.
pub(super) fn restore_held(world: &mut World, snapshot: &PhoenixSnapshot) {
    let Some(fixed) = world.get_resource::<Time<Fixed>>() else {
        return;
    };
    let dt = fixed.timestep().as_secs_f32();
    for row in &snapshot.entities {
        let entity = world
            .try_query_filtered::<(Entity, &EntityUuid), Allow<bevy::ecs::entity_disabling::Disabled>>()
            .and_then(|mut query| {
                query
                    .iter(world)
                    .find_map(|(entity, uuid)| (uuid.0 == row.uuid).then_some(entity))
            });
        let Some(entity) = entity else { continue };
        let retained: Vec<_> = row
            .blackboards
            .iter()
            .filter_map(|(source, board)| {
                let owner = SystemId(source.clone());
                let kind = kind(board)?;
                let config = &world.get::<ShipConfigComponent>(entity)?.0;
                let instrument = world.get::<BroadProfile>(entity)?.0.get(&owner)?;
                if config.system(&owner)?.kind != kind || instrument.kind != kind {
                    return None;
                }
                let records = crate::science::contact_routing::observations(board)?;
                let state = world.get::<BroadTracks>(entity)?.0.get(&owner)?;
                // A real fixed-step publication can precede the saved counter
                // by one. Bare owner fixtures may publish at the current counter.
                // Require exact canonical records rather than trusting saved
                // aliases, geometry, classifications or selectable flags.
                let supported = [Some(snapshot.tick), snapshot.tick.checked_sub(1)]
                    .into_iter()
                    .flatten()
                    .any(|tick| {
                        crate::science::sharing::projection(
                            &row.uuid,
                            state,
                            &instrument.profile,
                            tick,
                            dt,
                        )
                        .is_ok_and(|canonical| canonical.as_slice() == records)
                    });
                supported.then(|| (owner, kind, records.to_vec()))
            })
            .collect();
        let Some(mut boards) = world.get_mut::<ShipSystemBlackboards>(entity) else {
            continue;
        };
        for (owner, expected_kind, records) in retained {
            if let Some(board) = boards
                .0
                .get_mut(&owner)
                .filter(|board| kind(board) == Some(expected_kind))
            {
                if let Some(current) = tracks_mut(board) {
                    *current = records;
                }
            }
        }
    }
    restore_navigation_availability(world, snapshot);
}

/// Control affordances are recomputed, never trusted from the saved board. A
/// real fixed publication may precede the saved next-tick counter by one. Keep
/// only an exact permitted projection at either accepted frontier; the servo's
/// current-tick context remains empty until ordinary mechanical admission.
fn restore_navigation_availability(world: &mut World, snapshot: &PhoenixSnapshot) {
    use bevy::ecs::system::SystemState;
    use phoenix_sim_gameplay::ship::helm_control::CompiledHelmControl;
    if !snapshot.entities.iter().any(|row| {
        row.blackboards.iter().any(
            |(_, board)| matches!(board, SystemBlackboard::Helm(board) if board.movement.is_some()),
        )
    }) {
        return;
    }
    let mut environment =
        SystemState::<crate::console::navigation::knowledge::Environment>::new(world);
    let updates = {
        let knowledge = environment.get(world);
        snapshot.entities.iter().flat_map(|row| {
            let entity = world.try_query_filtered::<(Entity, &EntityUuid), Allow<bevy::ecs::entity_disabling::Disabled>>()
                .and_then(|mut query| query.iter(world).find_map(|(entity, uuid)| (uuid.0 == row.uuid).then_some(entity)));
            let Some(entity) = entity else { return Vec::new(); };
            let (Some(config), Some(profile), Some(boards)) = (
                world.get::<ShipConfigComponent>(entity), world.get::<CompiledHelmControl>(entity), world.get::<ShipSystemBlackboards>(entity)
            ) else { return Vec::new(); };
            let waypoint = world.get::<crate::console::navigation::NavigationWaypoint>(entity);
            row.blackboards.iter().filter_map(|(source, held)| {
                let SystemBlackboard::Helm(held) = held else { return None; };
                let held = held.movement.as_ref()?;
                let expected = (held.navigation_generation, held.navigation_axes.clone());
                // An empty held affordance can reflect the source publication
                // not yet having run. Retaining withholding adds no knowledge
                // or control; positive availability needs exact source proof.
                let withheld = expected.0.is_none() && expected.1.is_empty();
                (withheld || [Some(snapshot.tick), snapshot.tick.checked_sub(1)].into_iter().flatten().any(|at| {
                    crate::console::helm::navigation::availability_at(&knowledge, entity, config, boards, &profile.0, waypoint, at) == expected
                })).then(|| (entity, SystemId(source.clone()), expected))
            }).collect::<Vec<_>>()
        }).collect::<Vec<_>>()
    };
    for (entity, source, (generation, axes)) in updates {
        if let Some(mut boards) = world.get_mut::<ShipSystemBlackboards>(entity) {
            if let Some(SystemBlackboard::Helm(board)) = boards.0.get_mut(&source) {
                if let Some(movement) = board.movement.as_mut() {
                    movement.navigation_generation = generation;
                    movement.navigation_axes = axes;
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "sensing_publications_tests.rs"]
mod tests;
