//! Fit-first continuation preflight. No machinery runs or live state changes here.
use super::*;
use crate::core::messages::SystemId;
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
    }?;
    match (
        world.get::<crate::resources::recovery_server::CompiledShipRecovery>(entity),
        &row.equipment_recovery,
    ) {
        (None, None) => Ok(()),
        (Some(tool), Some(state)) => crate::resources::equipment_recovery_server::validate_state(
            state,
            &tool.0,
            world
                .get::<CompiledShipFitting>(entity)
                .map(|profile| &profile.0),
            u64::MAX,
        ),
        _ => Err("equipment recovery continuation disagrees with its actual fitted tool".into()),
    }
}

pub(super) fn definitions(
    world: &World,
    row: &EntityState,
) -> Result<
    (
        Option<CompiledFitting>,
        Option<crate::resources::recovery::RecoveryToolConfig>,
    ),
    String,
> {
    if let Some(entity) = world
        .try_query_filtered::<(Entity, &EntityUuid), Allow<bevy::ecs::entity_disabling::Disabled>>()
        .and_then(|mut query| {
            query
                .iter(world)
                .find_map(|(entity, uuid)| (uuid.0 == row.uuid).then_some(entity))
        })
    {
        return Ok((
            world
                .get::<CompiledShipFitting>(entity)
                .map(|profile| profile.0.clone()),
            world
                .get::<crate::resources::recovery_server::CompiledShipRecovery>(entity)
                .map(|tool| tool.0.clone()),
        ));
    }
    let Some(origin) = &row.spawn else {
        return Ok((None, None));
    };
    let config = origin
        .resolve(
            &crate::entities::loader::WasmTemplateLoader,
            &mut Vec::new(),
        )
        .ok_or("cannot resolve saved equipment entity")?;
    Ok((CompiledFitting::compile(&config)?, config.recovery.clone()))
}

pub(super) fn validate_roster(
    world: &World,
    snapshot: &PhoenixSnapshot,
) -> Result<(), (String, String)> {
    let mut all_items = BTreeSet::new();
    let mut profiles = BTreeMap::new();
    for row in &snapshot.entities {
        let result = (|| {
            let (profile, tool) = definitions(world, row)?;
            match (&tool, &row.equipment_recovery) {
                (None, None) => {}
                (Some(tool), Some(state)) => {
                    crate::resources::equipment_recovery_server::validate_state(
                        state,
                        tool,
                        profile.as_ref(),
                        snapshot.tick,
                    )?
                }
                _ => {
                    return Err(
                        "equipment recovery continuation disagrees with its actual fitted tool"
                            .into(),
                    )
                }
            }
            match (&profile, &row.fitting) {
                (None, None) => Ok::<(), String>(()),
                (Some(profile), Some(saved)) => {
                    validate_state(profile, saved, row, snapshot.tick)?;
                    // This compiles the complete candidate definition, including
                    // absent entities, before ANY other continuation validation.
                    let _ = fitted_power(profile, saved, row)?;
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
            }?;
            if let Some(profile) = profile {
                profiles.insert(row.uuid.clone(), profile);
            }
            Ok(())
        })();
        result.map_err(|reason| (row.uuid.clone(), reason))?;
    }
    validate_shared_claims(snapshot, &profiles)
}

fn validate_shared_claims(
    snapshot: &PhoenixSnapshot,
    profiles: &BTreeMap<String, CompiledFitting>,
) -> Result<(), (String, String)> {
    let rows: BTreeMap<_, _> = snapshot
        .entities
        .iter()
        .map(|row| (row.uuid.as_str(), row))
        .collect();
    let mut claims: BTreeMap<(String, SystemId, String), (String, SystemId, u64)> = BTreeMap::new();
    let mut add = |uuid: &str,
                   system: &SystemId,
                   component: &str,
                   owner: &str,
                   controller: &SystemId,
                   id: u64|
     -> Result<(), (String, String)> {
        let key = (uuid.to_owned(), system.clone(), component.to_owned());
        if claims.iter().any(|((entity, target, part), claim)| {
            entity == uuid
                && target == system
                && (part == component || part == "*" || component == "*")
                && claim != &(owner.to_owned(), controller.clone(), id)
        }) {
            return Err((
                owner.to_owned(),
                "multiple jobs own the same physical equipment task".into(),
            ));
        }
        claims.insert(key, (owner.to_owned(), controller.clone(), id));
        Ok(())
    };
    for row in &snapshot.entities {
        if let Some(jobs) = row.repair.as_ref().and_then(|repair| repair.jobs.as_ref()) {
            for job in jobs
                .teams
                .iter()
                .flatten()
                .filter(|job| job.outcome.is_none())
            {
                let target = match &job.activity {
                    crate::jobs::repair::RepairActivity::Internal { .. } => jobs
                        .physical_work
                        .get(&job.id.0)
                        .and_then(|work| work.target.as_ref())
                        .map(|target| (row.uuid.as_str(), target)),
                    crate::jobs::repair::RepairActivity::External { target } => jobs
                        .external_physical_work
                        .get(&job.id.0)
                        .and_then(|work| work.target.as_ref())
                        .map(|work| (target.as_str(), work)),
                    crate::jobs::repair::RepairActivity::Treatment { .. }
                    | crate::jobs::repair::RepairActivity::PersonnelTransfer { .. } => None,
                    crate::jobs::repair::RepairActivity::Refit { target, phase, .. } => {
                        if !matches!(phase, crate::core::messages::TeamSlot::Returning { .. }) {
                            add(&row.uuid, target, "*", &row.uuid, &job.owner, job.id.0)?;
                        }
                        None
                    }
                };
                if let Some((
                    uuid,
                    phoenix_model::damage::RepairWorkTarget::Component { system, component },
                )) = target
                {
                    add(uuid, system, component, &row.uuid, &job.owner, job.id.0)?;
                }
            }
        }
        let Some(job) = row
            .equipment_recovery
            .as_ref()
            .and_then(|owner| owner.job.as_ref())
            .filter(|job| job.outcome.is_none())
        else {
            continue;
        };
        let work = &job.activity;
        add(
            &work.target_uuid,
            &work.source_system,
            &work.source_component,
            &row.uuid,
            &job.owner,
            job.id.0,
        )?;
        if let Some(source) = rows.get(work.target_uuid.as_str()) {
            let source_profile = profiles.get(&work.target_uuid).ok_or_else(|| {
                (
                    row.uuid.clone(),
                    "equipment source lost its supported profile".into(),
                )
            })?;
            let slot = source_profile
                .slot(&work.source_slot)
                .map_err(|reason| (row.uuid.clone(), reason))?;
            let item = source
                .fitting
                .as_ref()
                .and_then(|state| state.equipment.installed(&work.source_slot));
            let matches = item
                .is_some_and(|item| item.id == work.assembly && item.definition == work.definition)
                && slot.system == work.source_system
                && slot.operation_component == work.source_component;
            if !matches && !(job.paused && work.waiting == "source_unavailable") {
                return Err((
                    row.uuid.clone(),
                    "equipment work no longer names the actual source assembly".into(),
                ));
            }
        } else if !(job.paused && work.waiting == "source_unavailable") {
            return Err((
                row.uuid.clone(),
                "active equipment work source is absent".into(),
            ));
        }
    }
    for row in &snapshot.entities {
        let Some(profile) = profiles.get(&row.uuid) else {
            continue;
        };
        let Some(state) = &row.fitting else { continue };
        for locker in &profile.config().lockers {
            let mut used = 0_u64;
            for item in state.equipment.assemblies.values().filter(|item|matches!(&item.location,EquipmentLocation::Stored{locker:id} if id==&locker.id)) {
                used=used.saturating_add(profile.equipment(&item.definition).map_err(|reason|(row.uuid.clone(),reason))?.storage_space as u64);
            }
            used = used.saturating_add(crate::resources::fitting_server::reserved_space(
                profile, state, &locker.id,
            ));
            if let Some(owner) = &row.equipment_recovery {
                used = used.saturating_add(
                    crate::resources::equipment_recovery_server::reserved_space(
                        owner, &locker.id, profile,
                    ),
                );
            }
            if used > locker.space as u64 {
                return Err((
                    row.uuid.clone(),
                    "equipment jobs overbook actual finite receiving space".into(),
                ));
            }
        }
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
            || work.waiting.len() > 128
            || work.waiting.chars().any(char::is_control)
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
        if !matches!(work.stage, RefitStage::Installing | RefitStage::Returning) {
            if work.removed_item.as_deref()
                != saved.equipment.installed(key).map(|item| item.id.as_str())
            {
                return Err("removal stage does not own the actual installed assembly".into());
            }
        } else if let Some(old) = &work.removed_item {
            if old == &work.item || !saved.equipment.assemblies.get(old).is_some_and(|old|
                matches!(&old.location, EquipmentLocation::Stored { locker } if locker == &work.locker)
                || (work.stage == RefitStage::Returning && matches!(&old.location, EquipmentLocation::Installed { slot } if slot == key))) {
                return Err("removed equipment is absent from its actual receiving locker".into());
            }
        }
        profile.compatible(key, &item.definition)?;
        if saved.equipment.desired.get(key) != Some(&item.definition)
            || work.progress > definition.work.work_secs
            || !claimed_items.insert(&work.item)
        {
            return Err("refit intent or equipment claim disagrees with its physical stage".into());
        }
        let (team, job) = jobs
            .teams
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| entry.as_ref().map(|job| (index, job)))
            .find(|(_, job)| job.id == work.job && job.outcome.is_none())
            .ok_or("refit has no actual team commitment")?;
        if !matches!(&job.activity, crate::jobs::repair::RepairActivity::Refit { request, target, .. } if *request == work.request && target == &slot.system)
        {
            return Err("refit stage and team target disagree".into());
        }
        let crate::jobs::repair::RepairActivity::Refit {
            destination_compartment,
            phase,
            cancelled,
            ..
        } = &job.activity
        else {
            unreachable!()
        };
        let source_room = &profile.locker(&work.source_locker)?.compartment;
        let expected_destination =
            if matches!(work.stage, RefitStage::Fetching | RefitStage::Returning) {
                source_room.as_str()
            } else {
                room
            };
        let destination = destination_compartment.as_deref().unwrap_or(room);
        if destination != expected_destination
            || *cancelled != (work.stage == RefitStage::Returning)
            || (work.stage == RefitStage::Returning
                && (!matches!(phase, crate::core::messages::TeamSlot::Returning { .. })
                    || destination_compartment.is_none()))
            || (work.stage != RefitStage::Returning
                && matches!(phase, crate::core::messages::TeamSlot::Returning { .. }))
        {
            return Err("refit team route and equipment stage disagree".into());
        }
        let cursor = jobs
            .navigation
            .as_ref()
            .and_then(|rows| rows.get(team))
            .ok_or("refit carrier has no physical cursor")?;
        let actual_room = match &cursor.location {
            phoenix_model::interior::TeamLocation::Compartment { compartment } => compartment,
            phoenix_model::interior::TeamLocation::Crossing { from, .. } => from,
            _ => return Err("refit carrier has no internal physical location".into()),
        };
        let location_valid = match &item.location {
            EquipmentLocation::Stored { locker } => {
                work.stage == RefitStage::Fetching && locker == &work.source_locker
            }
            EquipmentLocation::InWork { job, compartment } => {
                work.stage != RefitStage::Fetching
                    && *job == work.job.0
                    && compartment == actual_room
            }
            _ => false,
        };
        if !location_valid
            || (matches!(
                work.stage,
                RefitStage::Preparing | RefitStage::Removing | RefitStage::Installing
            ) && (!matches!(phase, crate::core::messages::TeamSlot::Repairing { .. })
                || actual_room != room))
        {
            return Err("refit assembly is not at its actual carrier/work frontier".into());
        }
        match work.stage {
            RefitStage::Fetching
            | RefitStage::Delivering
            | RefitStage::Returning
            | RefitStage::Preparing
                if work.reserved || work.progress != 0.0 =>
            {
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

#[cfg(test)]
#[path = "fitting_fetch_tests.rs"]
mod fetch_tests;
