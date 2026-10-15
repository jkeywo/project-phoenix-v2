//! Whole-candidate and single-row validation share exact accepted recipe ownership.
//! Retained selection confers neither a live route nor resumed device consent.
use super::*;

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
                .ok_or("media candidate source recipe cannot resolve")?,
        )
    } else {
        None
    };
    if !warnings.is_empty() {
        return Err("media candidate source recipe is not exact".into());
    }
    let has_radio = match accepted.as_ref() {
        Some(config) => config
            .comms
            .as_ref()
            .and_then(|comms| comms.radio.as_ref())
            .is_some(),
        None => live.is_some_and(|entity| {
            world
                .get::<crate::comms::radio::RadioProfile>(entity)
                .is_some()
        }),
    };
    match (has_radio, &row.comms_media) {
        (false, None) => Ok(()),
        (true, Some(saved))
            if saved
                .origin_id
                .as_ref()
                .is_none_or(|origin| phoenix_model::comms_channel::identity(origin)) =>
        {
            Ok(())
        }
        (true, Some(_)) => Err("invalid retained media call identity".into()),
        _ => Err("saved media intent owner differs from accepted radio equipment".into()),
    }
}
pub(super) fn validate_roster(
    world: &World,
    snapshot: &PhoenixSnapshot,
) -> Result<(), (String, String)> {
    for row in &snapshot.entities {
        let live = world.try_query_filtered::<(Entity, &EntityUuid), Allow<bevy::ecs::entity_disabling::Disabled>>()
            .and_then(|mut query| query.iter(world).find_map(|(entity,id)| (id.0 == row.uuid).then_some(entity)));
        validate_row(world, live, row).map_err(|reason| (row.uuid.clone(), reason))?;
    }
    Ok(())
}
#[cfg(test)]
#[path = "comms_media_tests.rs"]
mod tests;
