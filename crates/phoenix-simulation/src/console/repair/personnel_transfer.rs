//! Current actual carried-craft inputs, reusing the existing Bay readiness owner.
//! This is a tick-stamped derived view, never a passenger/claim/controller store.
use super::server::{RepairRequestQueue, ShipRepairTeams};
use crate::core::messages::{SystemBlackboard, SystemId};
use crate::entities::{
    craft::{PhysicalProbe, ProbeCustody, ShipBayJobs},
    spawner::{EntityName, EntityUuid},
};
use bevy::prelude::*;
use phoenix_model::{craft::CraftLocation, personnel::*};
use phoenix_sim_gameplay::{
    jobs::repair::{
        PersonnelTransferPlan, PersonnelTransferWork, RepairActivity, TransferAvailability,
    },
    personnel::{CompiledPersonnel, PersonnelRoster},
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug)]
pub(crate) struct StoredPod {
    pub bay: SystemId,
    pub compartment: String,
    pub berths: Vec<String>,
    pub available: bool,
    pub bay_busy: bool,
    pub name: Option<String>,
}
#[derive(Resource, Default)]
pub(crate) struct PersonnelTransferContext {
    pub tick: Option<u64>,
    pub carried: BTreeMap<(String, String), StoredPod>,
}
impl PersonnelTransferContext {
    pub fn pod(&self, actor: &str, craft: &str, tick: u64) -> Option<&StoredPod> {
        (self.tick == Some(tick))
            .then(|| self.carried.get(&(actor.into(), craft.into())))
            .flatten()
    }
}
pub(crate) fn prepare(world: &mut World) {
    let Some(tick) = world
        .get_resource::<crate::sim_tick::SimTick>()
        .map(|tick| tick.0)
    else {
        world.insert_resource(PersonnelTransferContext::default());
        return;
    };
    let mut context = PersonnelTransferContext {
        tick: Some(tick),
        ..Default::default()
    };
    let mut parents = world.query::<(Entity, &EntityUuid, &CompiledPersonnel, &ShipBayJobs)>();
    let mut parent_rows: Vec<_> = parents
        .iter(world)
        .map(|(entity, id, _, bays)| (id.0.clone(), entity, bays.clone()))
        .collect();
    parent_rows.sort_by(|a, b| a.0.cmp(&b.0));
    let mut duplicate = BTreeSet::new();
    for (actor, parent, bays) in parent_rows {
        for (bay, state) in bays.0 {
            let Some(actual) = crate::entities::craft::bay_context(world, parent, &bay) else {
                continue;
            };
            let mut craft = world.query_filtered::<(
                &EntityUuid,
                &PhysicalProbe,
                &ProbeCustody,
                Option<&EntityName>,
            ), bevy::ecs::query::Allow<bevy::ecs::entity_disabling::Disabled>>(
            );
            for (id, profile, custody, name) in craft.iter(world) {
                if !matches!(&custody.0,CraftLocation::Carried {parent_uuid,bay:home} if parent_uuid==&actor && home==&bay)
                    || profile.0.passenger_berths.is_empty()
                {
                    continue;
                }
                let key = (actor.clone(), id.0.clone());
                if duplicate.contains(&key) {
                    continue;
                }
                if context.carried.contains_key(&key) {
                    context.carried.remove(&key);
                    duplicate.insert(key);
                    continue;
                }
                context.carried.insert(
                    key,
                    StoredPod {
                        bay: bay.clone(),
                        compartment: actual.config.compartment.clone(),
                        berths: profile.0.passenger_berths.clone(),
                        available: actual.available,
                        bay_busy: state.job.as_ref().is_some_and(|job| job.outcome.is_none()),
                        name: name.map(|name| name.0.clone()),
                    },
                );
            }
        }
    }
    world.insert_resource(context);
}
/// Canonical occupants plus current actual Jobs; no mutable manifest/counter.
pub(crate) fn free_berths<'a>(
    roster: &PersonnelRoster,
    teams: &ShipRepairTeams,
    craft: &str,
    berths: &'a [String],
) -> Vec<&'a String> {
    let occupied: BTreeSet<_> = roster
        .origins
        .values()
        .flat_map(|origin| origin.state.officers.values())
        .filter_map(|person| match &person.location {
            OfficerLocation::AboardCraft { craft: host, berth } if host == craft => {
                Some(berth.as_str())
            }
            _ => None,
        })
        .collect();
    let claimed:BTreeSet<_>=teams.0.job_state().teams.iter().flatten().filter_map(|job|match &job.activity {RepairActivity::PersonnelTransfer {cancelled:false,work,..} if job.outcome.is_none() && !work.completed && matches!(&work.request.destination,PersonnelDestination::Craft {craft:target} if target==craft)=>Some(work.plan.berth.as_str()),_=>None}).collect();
    berths
        .iter()
        .filter(|berth| !occupied.contains(berth.as_str()) && !claimed.contains(berth.as_str()))
        .collect()
}
pub(crate) fn permitted_carried(
    config: &crate::ship::config::ShipConfig,
    boards: &crate::server_app::ShipSystemBlackboards,
) -> BTreeSet<String> {
    let view = config
        .observations_for_system(&crate::ship::system_registry::repair_system_id(), &boards.0);
    boards
        .0
        .keys()
        .filter_map(|owner| view.get(owner))
        .flat_map(|board| match board {
            SystemBlackboard::Bay(bay) => bay
                .stored
                .iter()
                .map(|craft| craft.id.clone())
                .collect::<Vec<_>>(),
            _ => Vec::new(),
        })
        .collect()
}
pub(crate) fn plan(
    context: &PersonnelTransferContext,
    actor: &str,
    profile: &CompiledPersonnel,
    roster: &PersonnelRoster,
    teams: &ShipRepairTeams,
    officer: &str,
    request: &PersonnelDestinationRequest,
    tick: u64,
    permitted: &BTreeSet<String>,
) -> Option<PersonnelTransferPlan> {
    let person = roster.state(actor)?.officers.get(officer)?;
    let definition = profile
        .config
        .officers
        .iter()
        .find(|person| person.id == officer)?;
    let craft = match (&request.destination, &person.location) {
        (PersonnelDestination::Craft { craft }, OfficerLocation::AtCompartment { compartment })
            if compartment == &definition.home_compartment =>
        {
            craft
        }
        (PersonnelDestination::Home, OfficerLocation::AboardCraft { craft, .. }) => craft,
        _ => return None,
    };
    if !permitted.contains(craft) {
        return None;
    }
    let pod = context.pod(actor, craft, tick)?;
    if !pod.available
        || pod.bay_busy
        || teams.0.claims_personnel_bay(&pod.bay)
        || teams.0.refit_claims_system(&pod.bay)
    {
        return None;
    }
    let (berth, pickup_compartment, delivery_compartment) = match &request.destination {
        PersonnelDestination::Craft { .. } => (
            free_berths(roster, teams, craft, &pod.berths)
                .first()?
                .to_string(),
            definition.home_compartment.clone(),
            pod.compartment.clone(),
        ),
        PersonnelDestination::Home => {
            let OfficerLocation::AboardCraft { berth, .. } = &person.location else {
                return None;
            };
            if !pod.berths.contains(berth) {
                return None;
            }
            (
                berth.clone(),
                pod.compartment.clone(),
                definition.home_compartment.clone(),
            )
        }
    };
    Some(PersonnelTransferPlan {
        craft: craft.clone(),
        berth,
        bay: pod.bay.clone(),
        pickup: person.location.clone(),
        pickup_compartment,
        delivery_compartment,
    })
}
pub(crate) fn readiness(
    context: &PersonnelTransferContext,
    actor: &str,
    work: &PersonnelTransferWork,
    tick: u64,
) -> TransferAvailability {
    let Some(pod) = context.pod(actor, &work.plan.craft, tick) else {
        return if context.tick == Some(tick) {
            TransferAvailability::Missing
        } else {
            TransferAvailability::Paused
        };
    };
    if pod.bay != work.plan.bay || !pod.berths.contains(&work.plan.berth) {
        return TransferAvailability::Missing;
    }
    if pod.available && !pod.bay_busy {
        TransferAvailability::Ready
    } else {
        TransferAvailability::Paused
    }
}

/// Publish only local earned facts. Remote aboard/foreign-carried/lost variants
/// are deliberately indistinguishable until a real later telemetry consumer.
pub(crate) fn decorate_view(
    view: &mut PersonnelView,
    context: Option<&PersonnelTransferContext>,
    actor: &str,
    profile: &CompiledPersonnel,
    roster: &PersonnelRoster,
    teams: &ShipRepairTeams,
    queue: Option<&RepairRequestQueue>,
    tick: u64,
    permitted: &BTreeSet<String>,
) {
    let Some(state) = roster.state(actor) else {
        view.officers.clear();
        view.carried_destinations.clear();
        return;
    };
    view.officers.retain(|person| match &person.location {
        OfficerLocation::AtCompartment { .. } | OfficerLocation::WithRepairTeam { .. } => true,
        OfficerLocation::WithTransfer { actor: carrier, .. } => carrier == actor,
        OfficerLocation::AboardCraft { craft, .. } => {
            permitted.contains(craft)
                && context
                    .and_then(|context| context.pod(actor, craft, tick))
                    .is_some()
        }
        OfficerLocation::Lost { .. } => false,
    });
    view.carried_destinations = context
        .filter(|context| context.tick == Some(tick))
        .map(|context| {
            context
                .carried
                .iter()
                .filter_map(|((parent, craft), pod)| {
                    (parent == actor && permitted.contains(craft)).then(|| PersonnelCraftChoice {
                        craft: craft.clone(),
                        name: pod.name.clone(),
                        free_berths: free_berths(roster, teams, craft, &pod.berths).len() as u32,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    for person in &mut view.officers {
        person.destination_request = queue
            .and_then(|queue| queue.personnel_destinations.get(&person.id))
            .cloned();
        person.transfer_teams = person
            .destination_request
            .as_ref()
            .and_then(|request| {
                plan(
                    context?, actor, profile, roster, teams, &person.id, request, tick, permitted,
                )?;
                Some(teams.0.personnel_transfer_teams(profile, state, &person.id))
            })
            .unwrap_or_default();
        person.transfer = teams
            .0
            .job_state()
            .teams
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| {
                let job = entry.as_ref().filter(|job| job.outcome.is_none())?;
                let RepairActivity::PersonnelTransfer { work, .. } = &job.activity else {
                    return None;
                };
                (work.officer == person.id).then_some((work.completed, index, job, work))
            })
            .min_by_key(|(completed, index, _, _)| (*completed, *index))
            .map(|(_, index, job, work)| OfficerTransferView {
                team_idx: index as u32,
                stage: work.stage,
                completed: work.completed,
                blocked: job.paused
                    || teams
                        .0
                        .navigation()
                        .and_then(|rows| rows.get(index))
                        .is_some_and(|cursor| cursor.blocked),
            });
    }
}

#[cfg(test)]
#[path = "personnel_transfer_tests.rs"]
mod tests;
