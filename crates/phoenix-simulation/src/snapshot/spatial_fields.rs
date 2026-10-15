//! Whole-candidate field preflight, including absent spawned entities.
use super::*;

pub(super) fn validate_roster(
    world: &World,
    snapshot: &PhoenixSnapshot,
) -> Result<(), (String, String)> {
    super::shield_enquiry::validate_roster(world, snapshot)?;
    super::navigation_formation::validate_roster(world, snapshot)?;
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
            || row.navigation_sharing.is_some()
            || row.broad.is_some()
            || row.scan.is_some()
            || row.passive.is_some()
            || row
                .navigation_waypoint
                .as_ref()
                .is_some_and(|waypoint| !waypoint.accepted_requests.is_empty()))
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
            .try_query_filtered::<(Entity, &EntityUuid), Allow<bevy::ecs::entity_disabling::Disabled>>()
            .and_then(|mut query| {
                query
                    .iter(world)
                    .find_map(|(entity, uuid)| (uuid.0 == row.uuid).then_some(entity))
            });
        for (contact, receipt) in row
            .navigation_waypoint
            .iter()
            .flat_map(|waypoint| &waypoint.accepted_requests)
        {
            let accepted_owner = if let Some(entity) = live {
                crate::console::navigation::server::navigation_owner(
                    world.get::<crate::ship_plugin::ShipConfigComponent>(entity),
                    world.get::<crate::console::navigation::sharing::NavigationSharingProfile>(
                        entity,
                    ),
                )
            } else if let Some(origin) = &row.spawn {
                let config = origin
                    .resolve(
                        &crate::entities::loader::WasmTemplateLoader,
                        &mut Vec::new(),
                    )
                    .ok_or_else(|| {
                        (
                            row.uuid.clone(),
                            "cannot resolve own Navigation request capability".into(),
                        )
                    })?;
                let profile = config.navigation_console.as_ref().and_then(|nav| {
                    nav.sharing.as_ref().map(|sharing| {
                        crate::console::navigation::sharing::NavigationSharingProfile {
                            owner: sharing.system.clone(),
                            radar: nav.system_chart.clone(),
                        }
                    })
                });
                let ship = config
                    .ship_config
                    .map(crate::ship_plugin::ShipConfigComponent);
                crate::console::navigation::server::navigation_owner(
                    ship.as_ref(),
                    profile.as_ref(),
                )
            } else {
                Some(crate::ship::system_registry::navigation_system_id())
            };
            let valid_owner = accepted_owner.as_ref() == Some(&receipt.owner);
            if !valid_owner
                || contact != &receipt.contact
                || receipt.contact.trim().is_empty()
                || receipt.order.validate().is_err()
                || receipt.tick > snapshot.tick
            {
                return Err((
                    row.uuid.clone(),
                    "invalid own Navigation request receipt".into(),
                ));
            }
        }
        let navigation_owner = if let Some(entity) = live {
            world
                .get::<crate::console::navigation::sharing::NavigationSharingProfile>(entity)
                .map(|profile| profile.owner.clone())
        } else if let Some(origin) = &row.spawn {
            let config = origin
                .resolve(
                    &crate::entities::loader::WasmTemplateLoader,
                    &mut Vec::new(),
                )
                .ok_or_else(|| {
                    (
                        row.uuid.clone(),
                        "cannot resolve saved Navigation capability".into(),
                    )
                })?;
            config
                .navigation_console
                .and_then(|nav| nav.sharing)
                .map(|sharing| sharing.system)
        } else {
            None
        };
        match (&navigation_owner, &row.navigation_sharing) {
            (None, None) => {}
            (Some(owner), Some(saved)) => saved
                .validate_owner(&row.uuid, owner, snapshot.tick)
                .map_err(|reason| (row.uuid.clone(), reason))?,
            _ => {
                return Err((
                    row.uuid.clone(),
                    "saved Navigation sharing disagrees with accepted equipment".into(),
                ))
            }
        }
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
