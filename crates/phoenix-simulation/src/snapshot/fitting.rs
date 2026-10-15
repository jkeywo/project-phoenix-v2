//! Fit-first continuation preflight. No machinery runs or live state changes here.
use super::*;
use crate::resources::fitting::{CompiledFitting, EquipmentLocation};
use crate::resources::fitting_server::{CompiledShipFitting, RefitStage, ShipFitting};
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn validate_entity(
    world: &World,
    entity: Entity,
    row: &EntityState,
) -> Result<(), String> {
    match (world.get::<CompiledShipFitting>(entity), &row.fitting) {
        (None, None) => Ok(()),
        (Some(profile), Some(saved)) => {
            validate_state(&profile.0, saved, row, u64::MAX)?;
            fitted_power(&profile.0, saved, row).map(|_| ())
        }
        _ => Err("saved equipment owner disagrees with accepted fitting capability".into()),
    }
}

pub(super) fn validate_roster(
    world: &World,
    snapshot: &PhoenixSnapshot,
) -> Result<(), (String, String)> {
    let mut all_items = BTreeSet::new();
    for row in &snapshot.entities {
        let result = (|| {
            let live = world
                .try_query::<(Entity, &EntityUuid)>()
                .and_then(|mut query| {
                    query
                        .iter(world)
                        .find_map(|(entity, uuid)| (uuid.0 == row.uuid).then_some(entity))
                });
            let profile = if let Some(entity) = live {
                world
                    .get::<CompiledShipFitting>(entity)
                    .map(|profile| profile.0.clone())
            } else if let Some(origin) = &row.spawn {
                let config = origin
                    .resolve(
                        &crate::entities::loader::WasmTemplateLoader,
                        &mut Vec::new(),
                    )
                    .ok_or("cannot resolve saved fitting entity")?;
                CompiledFitting::compile(&config)?
            } else {
                None
            };
            match (profile, &row.fitting) {
                (None, None) => Ok(()),
                (Some(profile), Some(saved)) => {
                    validate_state(&profile, saved, row, snapshot.tick)?;
                    // This compiles the complete candidate definition, including
                    // absent entities, before ANY other continuation validation.
                    let _ = fitted_power(&profile, saved, row)?;
                    for item in saved.equipment.assemblies.keys() {
                        if !all_items.insert(item.clone()) {
                            return Err(
                                "an equipment assembly is owned by multiple entities".into()
                            );
                        }
                    }
                    Ok(())
                }
                _ => {
                    Err("saved equipment owner disagrees with the accepted fitting profile".into())
                }
            }
        })();
        result.map_err(|reason| (row.uuid.clone(), reason))?;
    }
    Ok(())
}

fn validate_state(
    profile: &CompiledFitting,
    saved: &ShipFitting,
    row: &EntityState,
    tick: u64,
) -> Result<(), String> {
    saved.equipment.validate(profile)?;
    if saved.next_request == 0
        || saved.last_tick.is_some_and(|last| last > tick)
        || saved
            .requested
            .iter()
            .any(|slot| profile.slot(slot).is_err())
    {
        return Err("invalid fitting request frontier".into());
    }
    let jobs = row
        .repair
        .as_ref()
        .and_then(|repair| repair.jobs.as_ref())
        .ok_or("fitting requires its actual repair-team owner")?;
    let mut requests = BTreeSet::new();
    let mut claimed_items = BTreeSet::new();
    let mut expected_parts = BTreeMap::new();
    for (key, work) in &saved.work {
        let slot = profile.slot(key)?;
        let room = profile.compartment(key)?;
        if key != &work.slot
            || !saved.requested.contains(key)
            || work.request == 0
            || work.request >= saved.next_request
            || !requests.insert(work.request)
            || !work.progress.is_finite()
            || work.progress < 0.0
            || profile.locker(&work.locker)?.compartment != room
        {
            return Err("invalid fitting stage identity, progress or physical storage".into());
        }
        let item = saved
            .equipment
            .assemblies
            .get(&work.item)
            .ok_or("refit lost its actual assembly")?;
        let definition = profile.equipment(&item.definition)?;
        if work.stage != RefitStage::Installing {
            if work.removed_item.as_deref()
                != saved.equipment.installed(key).map(|item| item.id.as_str())
            {
                return Err("removal stage does not own the actual installed assembly".into());
            }
        } else if let Some(old) = &work.removed_item {
            if old == &work.item || !saved.equipment.assemblies.get(old).is_some_and(|old|
                matches!(&old.location, EquipmentLocation::Stored { locker } if locker == &work.locker)) {
                return Err("removed equipment is absent from its actual receiving locker".into());
            }
        }
        profile.compatible(key, &item.definition)?;
        if saved.equipment.desired.get(key) != Some(&item.definition)
            || work.progress > definition.work.work_secs
            || !claimed_items.insert(&work.item)
            || !matches!(&item.location, EquipmentLocation::InWork { job, compartment } if *job == work.job.0 && compartment == room)
        {
            return Err("refit intent or equipment claim disagrees with its physical stage".into());
        }
        let job = jobs
            .teams
            .iter()
            .flatten()
            .find(|job| job.id == work.job && job.outcome.is_none())
            .ok_or("refit has no actual team commitment")?;
        if !matches!(&job.activity, crate::jobs::repair::RepairActivity::Refit { request, target, .. } if *request == work.request && target == &slot.system)
        {
            return Err("refit stage and team target disagree".into());
        }
        match work.stage {
            RefitStage::Preparing if work.reserved || work.progress != 0.0 => {
                return Err("unprepared work cannot own paid progress".into())
            }
            RefitStage::Removing if !work.reserved => {
                return Err("removal requires its exact parts reservation".into())
            }
            RefitStage::Installing if work.reserved || saved.equipment.installed(key).is_some() => {
                return Err(
                    "installation needs a vacant slot and already consumed removal parts".into(),
                )
            }
            _ => {}
        }
        if work.reserved {
            for (index, part) in definition.work.requirements.iter().enumerate() {
                let services = profile
                    .effective_config(&saved.equipment)?
                    .resource_services;
                let endpoint = crate::resources::service::resolve_endpoint(
                    &part.resource,
                    &services,
                    &crate::resources::service::ResourceLink {
                        service: Some(part.service.clone()),
                        provider: None,
                    },
                )?;
                let crate::resources::service::ResourceEndpoint::Capacity { id } = endpoint else {
                    return Err("fitting parts must be physical capacities".into());
                };
                expected_parts.insert((id, format!("{}/{index}", work.claim())), part.amount);
            }
        }
    }
    for item in saved.equipment.assemblies.values() {
        if matches!(item.location, EquipmentLocation::InWork { .. })
            && !claimed_items.contains(&item.id)
        {
            return Err("unowned portable equipment work claim".into());
        }
    }
    for job in jobs
        .teams
        .iter()
        .flatten()
        .filter(|job| job.outcome.is_none())
    {
        if let crate::jobs::repair::RepairActivity::Refit {
            request,
            target,
            phase,
            ..
        } = &job.activity
        {
            if !matches!(phase, crate::core::messages::TeamSlot::Returning { .. })
                && !saved.work.values().any(|work| {
                    work.request == *request
                        && work.job == job.id
                        && profile
                            .slot(&work.slot)
                            .is_ok_and(|slot| &slot.system == target)
                })
            {
                return Err("refit team has no corresponding physical stage".into());
            }
        }
    }
    let actual_parts: BTreeMap<_, _> = row
        .infrastructure
        .as_ref()
        .into_iter()
        .flat_map(|inventory| inventory.capacities())
        .flat_map(|capacity| {
            capacity
                .stock
                .reserved
                .iter()
                .filter(|(claim, _)| claim.starts_with("refit/"))
                .map(|(claim, amount)| ((capacity.id.clone(), claim.clone()), *amount))
        })
        .collect();
    if actual_parts != expected_parts {
        return Err("fitting progress and actual parts claims disagree".into());
    }
    Ok(())
}

pub(super) fn fitted_power(
    profile: &CompiledFitting,
    saved: &ShipFitting,
    row: &EntityState,
) -> Result<crate::ship::power::ShipPowerSystem, String> {
    let effective = profile.effective_config(&saved.equipment)?;
    let mut power = crate::modifiers::power_system::PowerSystem::from_energy(
        effective
            .energy
            .clone()
            .ok_or("fitted ship has no energy profile")?,
        effective
            .ship_config
            .clone()
            .ok_or("fitted ship has no topology")?,
    )?
    .with_strike_reserve(
        effective
            .power
            .as_ref()
            .and_then(|power| power.strike_reserve.clone()),
    )?;
    let state = row
        .power
        .as_ref()
        .ok_or("fitted ship has no energy continuation")?;
    let capacity_ids = row
        .infrastructure
        .as_ref()
        .map(|inventory| {
            inventory
                .capacities()
                .iter()
                .map(|capacity| capacity.id.clone())
                .collect()
        })
        .unwrap_or_default();
    power.restore_continuation_with_capacities(state, &capacity_ids)?;
    Ok(crate::ship::power::ShipPowerSystem(power))
}

pub(super) fn apply(world: &mut World, entity: Entity, row: &EntityState) {
    let Some(saved) = &row.fitting else { return };
    let profile = world
        .get::<CompiledShipFitting>(entity)
        .expect("preflight compiled fitting")
        .0
        .clone();
    let effective = profile
        .effective_config(&saved.equipment)
        .expect("preflight compiled candidate hardware");
    let power = fitted_power(&profile, saved, row).expect("preflight validated fitted energy");
    crate::resources::fitting_server::apply_capability(world, entity, &effective);
    world.entity_mut(entity).insert((power, saved.clone()));
}
