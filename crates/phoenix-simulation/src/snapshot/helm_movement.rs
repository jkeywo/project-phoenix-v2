//! Whole-candidate movement intent validation, before clocks or actors mutate.
use super::*;
use phoenix_sim_gameplay::ship::helm_control::CompiledHelmControl;

fn validate_saved(
    row: &EntityState,
    profile: Option<&phoenix_model::helm_control::HelmControlConfig>,
    tick: u64,
) -> Result<(), String> {
    match (profile, row.helm_movement.as_ref()) {
        (None, None) => Ok(()),
        (Some(profile), Some(saved)) => {
            saved.validate(profile, tick)?;
            if let Some(raw) = &row.control {
                let display = saved.stick_display(profile);
                if raw.thrust != display[0]
                    || raw.steering != display[1]
                    || (profile.lateral.is_some() && raw.lateral != display[2])
                {
                    return Err("saved movement sticks disagree with canonical intent".into());
                }
            }
            Ok(())
        }
        _ => Err("saved movement intent disagrees with actual axis equipment".into()),
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
                    .find_map(|(entity, uuid)| (uuid.0 == row.uuid).then_some(entity))
            });
        let profile = if let Some(entity) = live {
            world
                .get::<CompiledHelmControl>(entity)
                .map(|profile| profile.0.clone())
        } else if let Some(origin) = &row.spawn {
            origin
                .resolve(
                    &crate::entities::loader::WasmTemplateLoader,
                    &mut Vec::new(),
                )
                .ok_or_else(|| {
                    (
                        row.uuid.clone(),
                        "cannot resolve saved movement equipment".into(),
                    )
                })?
                .helm_control
        } else {
            None
        };
        if profile.is_some()
            && (!world.contains_resource::<SimTick>()
                || !world.contains_resource::<Time<bevy::time::Fixed>>())
        {
            return Err((
                row.uuid.clone(),
                "restoring retained Helm intent requires the actual logical/fixed clock".into(),
            ));
        }
        validate_saved(row, profile.as_ref(), snapshot.tick)
            .map_err(|reason| (row.uuid.clone(), reason))?;
    }
    Ok(())
}

pub(super) fn validate_live(
    world: &World,
    entity: Entity,
    row: &EntityState,
    tick: u64,
) -> Result<(), String> {
    validate_saved(
        row,
        world
            .get::<CompiledHelmControl>(entity)
            .map(|profile| &profile.0),
        tick,
    )
}

#[cfg(test)]
#[path = "helm_movement_tests.rs"]
mod tests;
