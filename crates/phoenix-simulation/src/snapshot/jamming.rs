//! Closed row-local jammer intent, including candidate recipe precedence.
use super::*;
use crate::comms::jamming::ShipJamming;
pub(super) fn validate_entity(
    world: &World,
    entity: Entity,
    row: &EntityState,
) -> Result<(), String> {
    validate_row(world, Some(entity), row)
}
fn validate_row(world: &World, live: Option<Entity>, row: &EntityState) -> Result<(), String> {
    let mut warnings = Vec::new();
    let accepted = if let Some(origin) = &row.spawn {
        Some(
            origin
                .resolve(&crate::entities::loader::WasmTemplateLoader, &mut warnings)
                .ok_or("jamming candidate source recipe cannot resolve")?,
        )
    } else {
        None
    };
    if !warnings.is_empty() {
        return Err("jamming candidate recipe is not exact".into());
    }
    let config = match accepted.as_ref() {
        Some(config) => config.ship_config.as_ref(),
        None => live
            .and_then(|entity| world.get::<crate::ship_plugin::ShipConfigComponent>(entity))
            .map(|config| &config.0),
    };
    let expected = config.and_then(ShipJamming::from_ship);
    match (expected, &row.jamming) {
        (None, None) => Ok(()),
        (Some(_), Some(state)) => {
            state.validate(config.ok_or("jamming has no actual owning graph")?)
        }
        _ => Err("saved jamming owner differs from accepted source equipment".into()),
    }
}

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
                    .find_map(|(entity, id)| (id.0 == row.uuid).then_some(entity))
            });
        validate_row(world, live, row).map_err(|reason| (row.uuid.clone(), reason))?;
    }
    Ok(())
}
#[cfg(test)]
#[path = "jamming_tests.rs"]
mod tests;
