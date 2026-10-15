//! Whole-candidate physical custody and actual Bay Job continuation.
use super::*;
use crate::entities::craft::{PhysicalProbe, ProbeCustody};
use phoenix_model::craft::{BayOperation, CraftLocation, ProbeProfile};
use std::collections::{BTreeMap, BTreeSet};

fn shape(
    world: &World,
    row: &EntityState,
) -> Result<
    (
        Option<ProbeProfile>,
        Option<crate::ship::config::ShipConfig>,
        Option<crate::entities::config::EnergyConfigSection>,
    ),
    String,
> {
    if let Some(origin) = &row.spawn {
        let mut warnings = Vec::new();
        let config = origin
            .resolve(&crate::entities::loader::WasmTemplateLoader, &mut warnings)
            .ok_or("cannot resolve accepted physical craft recipe")?;
        if !warnings.is_empty() {
            return Err("physical craft recipe did not resolve exactly".into());
        }
        phoenix_sim_gameplay::ship::craft_profile::validate_entity(&config)?;
        return Ok((config.probe, config.ship_config, config.energy));
    }
    let live = world
        .try_query_filtered::<(Entity, &EntityUuid), Allow<bevy::ecs::entity_disabling::Disabled>>()
        .and_then(|mut query| {
            query
                .iter(world)
                .find_map(|(entity, id)| (id.0 == row.uuid).then_some(entity))
        });
    Ok((
        live.and_then(|entity| world.get::<PhysicalProbe>(entity))
            .map(|profile| profile.0.clone()),
        live.and_then(|entity| world.get::<crate::ship_plugin::ShipConfigComponent>(entity))
            .map(|profile| profile.0.clone()),
        live.and_then(|entity| world.get::<crate::ship::power::ShipPowerSystem>(entity))
            .map(|power| power.0.config().clone()),
    ))
}

pub(super) fn validate_roster(
    world: &World,
    snapshot: &PhoenixSnapshot,
) -> Result<(), (String, String)> {
    let mut rows = BTreeMap::new();
    for row in &snapshot.entities {
        if rows.insert(row.uuid.as_str(), row).is_some() {
            return Err((
                row.uuid.clone(),
                "duplicate canonical craft roster identity".into(),
            ));
        }
    }
    let mut profiles = BTreeMap::new();
    let mut bays = BTreeMap::new();
    for row in &snapshot.entities {
        let (probe, ship, energy) =
            shape(world, row).map_err(|reason| (row.uuid.clone(), reason))?;
        if probe.is_some() != row.probe_custody.is_some() {
            return Err((
                row.uuid.clone(),
                "custody owner differs from actual accepted probe".into(),
            ));
        }
        if let Some(profile) = probe {
            let custody = &row.probe_custody.as_ref().unwrap().0;
            custody
                .validate(&row.uuid)
                .map_err(|reason| (row.uuid.clone(), reason))?;
            let pose = row.physics.ok_or_else(|| {
                (
                    row.uuid.clone(),
                    "probe lacks its captured physical pose".into(),
                )
            })?;
            if !pose.iter().all(|v| v.is_finite())
                || [4, 5, 6, 7].into_iter().any(|index| pose[index] != 0.0)
            {
                return Err((
                    row.uuid.clone(),
                    "fixed probe requires finite pose and zero roll/motion".into(),
                ));
            }
            profiles.insert(row.uuid.as_str(), profile);
        }
        let expected: BTreeMap<_, _> = ship
            .as_ref()
            .into_iter()
            .flat_map(|ship| &ship.systems)
            .filter(|system| system.kind == crate::ship::system_registry::CRAFT_BAY_KIND)
            .map(|system| {
                (
                    &system.id,
                    crate::ship::composition::craft_bay_composition(&system.components)
                        .expect("accepted bay"),
                )
            })
            .collect();
        if expected.is_empty() != row.bay_jobs.is_none() {
            return Err((
                row.uuid.clone(),
                "Bay Job owner differs from accepted equipment".into(),
            ));
        }
        if let Some(saved) = &row.bay_jobs {
            if saved.0.len() != expected.len()
                || saved.0.keys().any(|id| !expected.contains_key(id))
            {
                return Err((
                    row.uuid.clone(),
                    "Bay Job roster differs from accepted Systems".into(),
                ));
            }
            for (owner, state) in &saved.0 {
                state
                    .validate(owner, snapshot.tick)
                    .map_err(|reason| (row.uuid.clone(), reason))?;
                let config = expected[owner].config;
                let cost = energy
                    .as_ref()
                    .and_then(|energy| {
                        energy
                            .consumers
                            .iter()
                            .find(|consumer| &consumer.system == owner)
                    })
                    .ok_or_else(|| {
                        (
                            row.uuid.clone(),
                            "Bay lacks actual finite work energy".into(),
                        )
                    })?
                    .continuous_cost_per_sec;
                if let Some(job) = &state.job {
                    let period = world
                        .get_resource::<Time<Fixed>>()
                        .ok_or_else(|| {
                            (row.uuid.clone(), "Bay requires actual fixed clock".into())
                        })?
                        .timestep()
                        .as_secs_f32() as f64;
                    if &job.activity.profile != config
                        || job.activity.energy_per_sec != cost
                        || !period.is_finite()
                        || period <= 0.0
                        || job.activity.worked_secs
                            > (job.execution_tick - job.activity.accepted_tick) as f64 * period
                    {
                        return Err((
                            row.uuid.clone(),
                            "Bay accepted recipe or physical work frontier differs".into(),
                        ));
                    }
                }
                bays.insert((row.uuid.as_str(), owner.clone()), (config.clone(), state));
            }
        }
    }
    let mut used: BTreeMap<_, u64> = BTreeMap::new();
    for row in &snapshot.entities {
        if let Some(ProbeCustody(CraftLocation::Carried { parent_uuid, bay })) = &row.probe_custody
        {
            let key = (parent_uuid.as_str(), bay.clone());
            let (config, _) = bays.get(&key).ok_or_else(|| {
                (
                    row.uuid.clone(),
                    "Carried craft lacks its actual parent/bay".into(),
                )
            })?;
            let occupied = used.entry(key).or_default();
            *occupied = occupied
                .checked_add(profiles[row.uuid.as_str()].space)
                .ok_or_else(|| (row.uuid.clone(), "craft occupied space overflows".into()))?;
            if *occupied > config.capacity_space {
                return Err((
                    row.uuid.clone(),
                    "actual Carried craft exceeds bay space".into(),
                ));
            }
        }
    }
    let mut claimed = BTreeSet::new();
    for ((parent, owner), (config, state)) in &bays {
        let Some(job) = state.job.as_ref().filter(|job| job.outcome.is_none()) else {
            continue;
        };
        let craft = rows.get(job.activity.craft_uuid.as_str()).ok_or_else(|| {
            (
                parent.to_string(),
                "active Bay Job names removed craft".into(),
            )
        })?;
        let profile = profiles
            .get(job.activity.craft_uuid.as_str())
            .ok_or_else(|| {
                (
                    parent.to_string(),
                    "active Bay Job names unsupported craft".into(),
                )
            })?;
        if !claimed.insert(job.activity.craft_uuid.as_str()) {
            return Err((
                parent.to_string(),
                "multiple active Bay claims name one craft".into(),
            ));
        }
        let custody = &craft.probe_custody.as_ref().unwrap().0;
        match job.activity.operation {
            BayOperation::Launch if !matches!(custody,CraftLocation::Carried{parent_uuid,bay} if parent_uuid.as_str()==*parent && bay==owner) => {
                return Err((
                    parent.to_string(),
                    "active launch does not own its actual stored craft".into(),
                ))
            }
            BayOperation::Recover => {
                if !matches!(custody, CraftLocation::Deployed {}) {
                    return Err((
                        parent.to_string(),
                        "active recovery targets already-carried craft".into(),
                    ));
                }
                let occupied = used.entry((*parent, owner.clone())).or_default();
                *occupied = occupied
                    .checked_add(profile.space)
                    .ok_or_else(|| (parent.to_string(), "recovery space claim overflow".into()))?;
                if *occupied > config.capacity_space {
                    return Err((
                        parent.to_string(),
                        "active recovery exceeds actual reserved bay space".into(),
                    ));
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// Snapshot canonical state is already preflighted. Presence rebuild must not
/// perform new measurements, erase dated history or re-run custody work.
pub(super) fn restore_presence(world: &mut World, snapshot: &PhoenixSnapshot) {
    let mut query = world
        .query_filtered::<(Entity, &EntityUuid), Allow<bevy::ecs::entity_disabling::Disabled>>();
    let rows: Vec<_> = query
        .iter(world)
        .filter_map(|(entity, id)| {
            snapshot
                .entities
                .iter()
                .find(|row| row.uuid == id.0)
                .and_then(|row| row.probe_custody.as_ref())
                .map(|custody| (entity, custody.0.clone()))
        })
        .collect();
    for (entity, location) in rows {
        crate::entities::craft::restore_presence(world, entity, &location);
    }
}
