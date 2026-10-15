//! Closed per-row illumination channel owner; no physical receiver joins.
use super::*;
use crate::console::weapons::illumination::ShipIllumination;

pub(super) fn validate_entity(
    world: &World,
    entity: Entity,
    row: &EntityState,
) -> Result<(), String> {
    let config = world.get::<crate::ship_plugin::ShipConfigComponent>(entity);
    let expected = config.and_then(|config| ShipIllumination::from_ship(&config.0));
    match (expected, &row.illumination) {
        (None, None) => Ok(()),
        (Some(_), Some(state)) => {
            state.validate(&config.ok_or("illumination has no actual owning graph")?.0)
        }
        _ => Err("saved illumination owner differs from accepted source equipment".into()),
    }
}

#[cfg(test)]
#[path = "illumination_tests.rs"]
mod tests;
