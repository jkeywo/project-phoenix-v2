//! Candidate-only officer custody validation against the SAME saved Repair jobs.
use super::*;
use phoenix_sim_gameplay::personnel::{CompiledPersonnel, PersonnelState};

fn validate_row(
    row: &EntityState,
    profile: Option<&CompiledPersonnel>,
    tick: u64,
) -> Result<(), String> {
    match (profile, row.personnel.as_ref()) {
        (None, None) => Ok(()),
        (Some(profile), Some(state)) => {
            let jobs = row
                .repair
                .as_ref()
                .and_then(|repair| repair.jobs.as_ref())
                .ok_or("officer custody requires its actual candidate Repair team owner")?;
            state.validate(profile, jobs, tick)
        }
        _ => Err("officer custody disagrees with the accepted finite personnel roster".into()),
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
            world.get::<CompiledPersonnel>(entity).cloned()
        } else if let Some(origin) = &row.spawn {
            let config = origin
                .resolve(
                    &crate::entities::loader::WasmTemplateLoader,
                    &mut Vec::new(),
                )
                .ok_or_else(|| {
                    (
                        row.uuid.clone(),
                        "cannot resolve saved personnel equipment".into(),
                    )
                })?;
            phoenix_sim_gameplay::personnel::compile_entity(&config)
                .map_err(|reason| (row.uuid.clone(), reason))?
        } else {
            None
        };
        if profile.is_some() && !world.contains_resource::<SimTick>() {
            return Err((
                row.uuid.clone(),
                "restoring officer postings requires the actual logical clock".into(),
            ));
        }
        validate_row(row, profile.as_ref(), snapshot.tick)
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
    validate_row(row, world.get::<CompiledPersonnel>(entity), tick)
}

pub(super) fn apply(world: &mut World, entity: Entity, row: &EntityState) {
    if let Some(state) = &row.personnel {
        world.entity_mut(entity).insert(state.clone());
    } else {
        world.entity_mut(entity).remove::<PersonnelState>();
    }
}

#[cfg(test)]
#[path = "personnel_tests.rs"]
mod tests;
