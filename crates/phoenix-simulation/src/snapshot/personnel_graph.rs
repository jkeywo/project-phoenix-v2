//! Whole-candidate cross-actor custody, berth and escort claim closure.
use super::*;
use phoenix_model::{
    craft::CraftLocation,
    personnel::{PersonnelDestination, PersonnelTransferStage},
};
use phoenix_sim_gameplay::jobs::repair::RepairActivity;
use std::collections::{BTreeMap, BTreeSet};
pub(super) fn validate(
    world: &World,
    snapshot: &PhoenixSnapshot,
    profiles: &BTreeMap<String, CompiledPersonnel>,
) -> Result<(), (String, String)> {
    if snapshot.personnel.origins.is_empty() {
        return Ok(());
    }
    let fail = |actor: &str, reason: &str| (actor.to_owned(), reason.to_owned());
    let mut craft = BTreeMap::new();
    let mut ships = BTreeMap::new();
    let mut rows = BTreeMap::new();
    for row in &snapshot.entities {
        if rows.insert(row.uuid.as_str(), row).is_some() {
            return Err(fail(&row.uuid, "duplicate personnel graph actor identity"));
        }
        let (probe, ship) = super::super::craft::personnel_shape(world, row)
            .map_err(|reason| (row.uuid.clone(), reason))?;
        if let Some(probe) = probe {
            craft.insert(row.uuid.as_str(), probe);
        }
        if let Some(ship) = ship {
            ships.insert(row.uuid.as_str(), ship);
        }
    }
    let mut occupied = BTreeSet::new();
    for (actor, origin) in &snapshot.personnel.origins {
        for (id, person) in &origin.state.officers {
            if person.changed_tick > snapshot.tick {
                return Err(fail(actor, "officer custody is from a future logical tick"));
            }
            match &person.location {
                OfficerLocation::AtCompartment { actor: holder, .. }
                | OfficerLocation::WithRepairTeam { actor: holder, .. } => {
                    if holder != actor || !profiles.contains_key(actor) {
                        return Err(fail(actor, "local officer has no actual live origin owner"));
                    }
                }
                OfficerLocation::WithTransfer {
                    actor: carrier,
                    team_idx,
                    job,
                } => {
                    if carrier != actor || !profiles.contains_key(actor) {
                        return Err(fail(actor, "escort has no supported actual owner"));
                    }
                    let actual = rows
                        .get(carrier.as_str())
                        .and_then(|row| row.repair.as_ref())
                        .and_then(|repair| repair.jobs.as_ref())
                        .and_then(|jobs| jobs.teams.get(*team_idx as usize))
                        .and_then(Option::as_ref)
                        .ok_or_else(|| fail(actor, "escort has no actual team Job"))?;
                    if actual.id != *job
                        || actual.outcome.is_some()
                        || !matches!(&actual.activity,RepairActivity::PersonnelTransfer {work,..} if work.officer.origin==*actor && work.officer.officer==*id && !work.completed && work.stage!=PersonnelTransferStage::Collecting)
                    {
                        return Err(fail(
                            actor,
                            "officer escort disagrees with its one actual Job/cursor",
                        ));
                    }
                }
                OfficerLocation::AboardCraft { craft: host, berth } => {
                    let profile = craft
                        .get(host.as_str())
                        .ok_or_else(|| fail(actor, "officer berth has no actual accepted craft"))?;
                    if !profile.passenger_berths.contains(berth)
                        || !occupied.insert((host.as_str(), berth.as_str()))
                    {
                        return Err(fail(
                            actor,
                            "officers duplicate or exceed accepted actual berths",
                        ));
                    }
                    if !rows
                        .get(host.as_str())
                        .is_some_and(|row| row.probe_custody.is_some())
                    {
                        return Err(fail(
                            actor,
                            "occupied craft has no canonical actual custody",
                        ));
                    }
                }
                OfficerLocation::Lost { actor: host } => {
                    if host == actor {
                        if origin.removed_tick != Some(person.changed_tick) {
                            return Err(fail(
                                actor,
                                "origin casualty differs from actual retained removal",
                            ));
                        }
                    } else {
                        let receipt = crate::world_id::WorldId::parse(host)
                            .filter(|receipt| {
                                receipt.namespace == crate::world_id::IdNamespace::Entity
                                    && receipt.render() == *host
                            })
                            .ok_or_else(|| {
                                fail(
                                    actor,
                                    "craft casualty has no canonical actual host identity",
                                )
                            })?;
                        if rows.contains_key(host.as_str()) || receipt.tick > person.changed_tick {
                            return Err(fail(
                                actor,
                                "craft casualty contradicts actual body or birth clock",
                            ));
                        }
                        let mint = snapshot.mint.as_ref().ok_or_else(|| {
                            fail(actor, "craft casualty requires its actual mint frontier")
                        })?;
                        if receipt.tick > mint.tick
                            || mint.tick > snapshot.tick
                            || (receipt.tick == mint.tick
                                && receipt.seq
                                    >= mint.next_seq
                                        [crate::world_id::IdNamespace::Entity.code() as usize])
                        {
                            return Err(fail(
                                actor,
                                "craft casualty exceeds the actual mint frontier",
                            ));
                        }
                    }
                }
            }
        }
    }
    let mut claims = BTreeSet::new();
    let mut bay_claims = BTreeSet::new();
    for actor in profiles.keys() {
        let row = rows[actor.as_str()];
        let Some(repair) = row.repair.as_ref() else {
            continue;
        };
        let Some(jobs) = repair.jobs.as_ref() else {
            continue;
        };
        for job in jobs
            .teams
            .iter()
            .flatten()
            .filter(|job| job.outcome.is_none())
        {
            let RepairActivity::PersonnelTransfer {
                cancelled: false,
                work,
                ..
            } = &job.activity
            else {
                continue;
            };
            if work.completed {
                continue;
            }
            if !bay_claims.insert((actor.as_str(), &work.plan.bay)) || jobs.teams.iter().flatten().any(|other|other.outcome.is_none() && matches!(&other.activity,RepairActivity::Refit {target,..} if target==&work.plan.bay)) || snapshot.entities.iter().any(|other| other.equipment_recovery.as_ref().and_then(|owner|owner.job.as_ref()).is_some_and(|job|job.outcome.is_none() && job.activity.target_uuid == *actor && job.activity.source_system == work.plan.bay)) {
                return Err(fail(actor,"crew handoff duplicates a Bay or conflicts with actual equipment removal"));
            }
            let pod = craft
                .get(work.plan.craft.as_str())
                .ok_or_else(|| fail(actor, "active escort has no actual accepted craft"))?;
            let actual = rows[work.plan.craft.as_str()];
            if !pod.passenger_berths.contains(&work.plan.berth)
                || !matches!(actual.probe_custody.as_ref().map(|custody|&custody.0),Some(CraftLocation::Carried {parent_uuid,bay}) if parent_uuid==actor && bay==&work.plan.bay)
            {
                return Err(fail(
                    actor,
                    "active escort is not at its actual carried bay/berth",
                ));
            }
            let bay = ships
                .get(actor.as_str())
                .and_then(|ship| ship.system(&work.plan.bay))
                .filter(|system| system.kind == crate::ship::system_registry::CRAFT_BAY_KIND)
                .and_then(|system| {
                    phoenix_sim_gameplay::ship::composition::craft_bay_composition(
                        &system.components,
                    )
                })
                .ok_or_else(|| fail(actor, "escort has no actual accepted Bay equipment"))?;
            let room = match work.request.destination {
                PersonnelDestination::Craft { .. } => &work.plan.delivery_compartment,
                PersonnelDestination::Home => &work.plan.pickup_compartment,
            };
            if room != &bay.config.compartment {
                return Err(fail(
                    actor,
                    "escort handoff differs from the actual Bay compartment",
                ));
            }
            let bay_state = row
                .bay_jobs
                .as_ref()
                .and_then(|bays| bays.0.get(&work.plan.bay))
                .ok_or_else(|| fail(actor, "escort has no actual Bay Job owner"))?;
            if bay_state
                .job
                .as_ref()
                .is_some_and(|job| job.outcome.is_none())
            {
                return Err(fail(
                    actor,
                    "Bay handling and crew escort cannot concurrently claim the same Bay",
                ));
            }
            if matches!(work.request.destination, PersonnelDestination::Craft { .. })
                && (occupied.contains(&(work.plan.craft.as_str(), work.plan.berth.as_str()))
                    || !claims.insert((work.plan.craft.as_str(), work.plan.berth.as_str())))
            {
                return Err(fail(
                    actor,
                    "escort duplicates an occupied or committed actual berth",
                ));
            }
        }
    }
    Ok(())
}
