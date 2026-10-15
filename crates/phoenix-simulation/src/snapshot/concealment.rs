//! Closed row-local concealment emitter intent, including candidate recipe precedence.
use super::*;
use crate::science::concealment::ShipConcealmentEmitters;
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
                .ok_or("concealment emitter candidate source recipe cannot resolve")?,
        )
    } else {
        None
    };
    if !warnings.is_empty() {
        return Err("concealment emitter candidate recipe is not exact".into());
    }
    let config = match accepted.as_ref() {
        Some(config) => config.ship_config.as_ref(),
        None => live
            .and_then(|entity| world.get::<crate::ship_plugin::ShipConfigComponent>(entity))
            .map(|config| &config.0),
    };
    let expected = config.and_then(ShipConcealmentEmitters::from_ship);
    match (expected, &row.concealment_emitters) {
        (None, None) => Ok(()),
        (Some(_), Some(state)) => {
            state.validate(config.ok_or("concealment emitter has no actual owning graph")?)
        }
        _ => Err("saved concealment emitter owner differs from accepted source equipment".into()),
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
#[path = "concealment_tests.rs"]
mod tests;
