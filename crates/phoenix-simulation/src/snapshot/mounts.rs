//! Validate every candidate mount before any restore mutation, including rows
//! that must be recreated from their accepted source configuration.
use super::*;
use crate::weapons::mounts::{MountProfiles, ShipMounts};

pub(super) fn validate_roster(
    world: &World,
    snapshot: &PhoenixSnapshot,
) -> Result<(), (String, String)> {
    for row in &snapshot.entities {
        let live = world
            .try_query_filtered::<(Entity, &EntityUuid), Allow<bevy::ecs::entity_disabling::Disabled>>()
            .and_then(|mut query| {
                query
                    .iter(world)
                    .find_map(|(entity, uuid)| (uuid.0 == row.uuid).then_some(entity))
            });
        let profile = if let Some(config) =
            live.and_then(|entity| world.get::<crate::ship_plugin::ShipConfigComponent>(entity))
        {
            MountProfiles::from_ship(&config.0)
        } else if let Some(origin) = &row.spawn {
            let config = origin
                .resolve(
                    &crate::entities::loader::WasmTemplateLoader,
                    &mut Vec::new(),
                )
                .ok_or_else(|| {
                    (
                        row.uuid.clone(),
                        "cannot resolve saved turret equipment".into(),
                    )
                })?;
            config
                .ship_config
                .as_ref()
                .map_or(Ok(None), MountProfiles::from_ship)
        } else {
            Ok(None)
        }
        .map_err(|reason| (row.uuid.clone(), reason))?;
        match (&profile, &row.mounts) {
            (None, None) => {}
            (Some(profile), Some(saved)) => {
                if !world.contains_resource::<crate::sim_tick::SimTick>()
                    || !world.contains_resource::<Time<bevy::time::Fixed>>()
                {
                    return Err((
                        row.uuid.clone(),
                        "restoring a turret requires its logical tick and fixed clock".into(),
                    ));
                }
                saved
                    .validate(profile, snapshot.tick)
                    .map_err(|reason| (row.uuid.clone(), reason))?;
            }
            _ => {
                return Err((
                    row.uuid.clone(),
                    "saved turret state disagrees with fitted equipment".into(),
                ));
            }
        }
        if let Some(entity) = live {
            let actual = world.get::<MountProfiles>(entity);
            if actual != profile.as_ref()
                || (profile.is_none() && world.get::<ShipMounts>(entity).is_some())
            {
                return Err((
                    row.uuid.clone(),
                    "live turret profile disagrees with accepted equipment".into(),
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "mounts_tests.rs"]
mod tests;
