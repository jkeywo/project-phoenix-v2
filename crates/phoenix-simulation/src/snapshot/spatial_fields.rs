//! Whole-candidate field preflight, including absent spawned entities.
use super::*;

pub(super) fn validate_roster(
    world: &World,
    snapshot: &PhoenixSnapshot,
) -> Result<(), (String, String)> {
    let evidence = snapshot
        .scenario
        .as_ref()
        .map(|scenario| &scenario.evidence);
    if let Some(evidence) = evidence {
        evidence
            .validate_references(snapshot.tick)
            .map_err(|reason| ("scenario/evidence".into(), reason))?;
    }
    // Resolve against the candidate's findings, never a convenient live log
    // that restore is about to replace. Original acquisition time bounds each
    // deliberate hypothesis as well as its identity.
    let references = evidence
        .into_iter()
        .flat_map(|evidence| &evidence.entries)
        .filter_map(|entry| {
            entry
                .reference
                .as_ref()
                .map(|reference| (reference.id.clone(), entry.gathered_at_tick))
        })
        .collect();
    for row in &snapshot.entities {
        if let Some(trees) = &row.ai_trees {
            for runtime in trees.0.values() {
                if let Some(reading) = &runtime.reading {
                    // Reading advances independently of the tree's evaluation
                    // cadence; its clock is bounded by the captured tick.
                    reading
                        .validate(snapshot.tick)
                        .map_err(|reason| (row.uuid.clone(), reason))?;
                }
            }
        }
        if let Some(saved) = &row.broad {
            for state in saved.values() {
                phoenix_sim_gameplay::science::references::validate_attachments(
                    state,
                    &references,
                    snapshot.tick,
                )
                .map_err(|reason| (row.uuid.clone(), reason))?;
            }
            if saved
                .values()
                .any(|state| state.last_tick.is_some_and(|tick| tick > snapshot.tick))
            {
                return Err((
                    row.uuid.clone(),
                    "broad observation frontier is newer than snapshot".into(),
                ));
            }
        }
        if (row.region_field.is_some()
            || row.broad.is_some()
            || row.scan.is_some()
            || row.passive.is_some())
            && (!world.contains_resource::<crate::sim_tick::SimTick>()
                || !world.contains_resource::<Time<bevy::time::Fixed>>())
        {
            return Err((
                row.uuid.clone(),
                "restoring a spatial field requires its logical tick and fixed-step clock".into(),
            ));
        }
        if let Some(passive) = &row.passive {
            if passive
                .0
                .values()
                .any(|state| state.last_tick.is_some_and(|last| last > snapshot.tick))
            {
                return Err((
                    row.uuid.clone(),
                    "passive sample is newer than the captured tick".into(),
                ));
            }
        }
        if let Some(scan) = &row.scan {
            if scan.instruments.values().any(|scan| {
                scan.history
                    .iter()
                    .any(|reading| reading.taken_at_tick > snapshot.tick)
                    || scan
                        .focused
                        .iter()
                        .chain(scan.unfinished.values())
                        .any(|work| work.last_tick > snapshot.tick)
            }) {
                return Err((
                    row.uuid.clone(),
                    "recorded scan work is newer than the captured tick".into(),
                ));
            }
        }
        let live = world
            .try_query::<(Entity, &EntityUuid)>()
            .and_then(|mut query| {
                query
                    .iter(world)
                    .find_map(|(entity, uuid)| (uuid.0 == row.uuid).then_some(entity))
            });
        let expected = if let Some(entity) = live {
            world
                .get::<crate::regions::field::RegionFieldState>(entity)
                .is_some()
        } else if let Some(origin) = &row.spawn {
            origin
                .resolve(
                    &crate::entities::loader::WasmTemplateLoader,
                    &mut Vec::new(),
                )
                .ok_or_else(|| {
                    (
                        row.uuid.clone(),
                        "cannot resolve saved field capability".into(),
                    )
                })?
                .field
                .is_some()
        } else {
            // Missing authored entities are refused by ordinary materialisation.
            row.region_field.is_some()
        };
        if expected != row.region_field.is_some() {
            return Err((
                row.uuid.clone(),
                "saved field owner disagrees with its accepted entity".into(),
            ));
        }
        if let Some(field) = &row.region_field {
            field
                .validate()
                .map_err(|reason| (row.uuid.clone(), reason))?;
            let geometry = live
                .and_then(|entity| {
                    world
                        .get::<crate::entities::spawner::RegionShapeSection>(entity)
                        .map(|shape| {
                            (
                                shape.0.clone(),
                                world
                                    .get::<crate::entities::spawner::RegionEffectsSection>(entity)
                                    .map_or_else(Vec::new, |effects| effects.0.clone()),
                            )
                        })
                })
                .or_else(|| {
                    let config = row.spawn.as_ref()?.resolve(
                        &crate::entities::loader::WasmTemplateLoader,
                        &mut Vec::new(),
                    )?;
                    Some((
                        config.shape?,
                        config
                            .effects
                            .map_or_else(Vec::new, |effects| effects.to_kinds()),
                    ))
                })
                .ok_or_else(|| {
                    (
                        row.uuid.clone(),
                        "saved field has no accepted physical geometry".into(),
                    )
                })?;
            field
                .validate_environment(&geometry.0, &geometry.1)
                .map_err(|reason| (row.uuid.clone(), reason))?;
            if field.anchor_tick > snapshot.tick {
                return Err((
                    row.uuid.clone(),
                    "field causal anchor is newer than the captured tick".into(),
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "spatial_fields_tests.rs"]
mod tests;
