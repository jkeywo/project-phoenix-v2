//! Whole-candidate actual drive and frozen private placement validation.
use super::*;
use crate::console::helm::transit::ShipTransitDrives;

fn validate_row(
    world: &World,
    row: &EntityState,
    live: Option<Entity>,
    ship: Option<&crate::ship::config::ShipConfig>,
    tick: u64,
) -> Result<(), String> {
    let expected = ship.and_then(ShipTransitDrives::from_ship);
    match (expected, &row.transit) {
        (None, None) => return Ok(()),
        (Some(_), Some(saved)) => {
            let ship = ship.expect("actual drive roster");
            saved.validate(ship, tick)?;
            let period = world
                .get_resource::<Time<bevy::time::Fixed>>()
                .ok_or("Transit continuation requires its actual fixed clock")?
                .timestep()
                .as_secs_f64();
            if !world.contains_resource::<SimTick>() || !period.is_finite() || period <= 0.0 {
                return Err("Transit continuation requires its actual logical clock".into());
            }
            if row.physics.is_none() {
                return Err("Transit requires its actual saved ShipPhysics owner".into());
            }
            let costs = accepted_costs(world, live, row)?;
            for (owner, state) in &saved.0 {
                if let Some(order) = state.phase.order() {
                    if costs.get(owner)
                        != Some(&(
                            order.profile.preparation_energy_per_sec,
                            order.profile.commit_energy,
                        ))
                    {
                        return Err(
                            "saved Transit work costs differ from accepted actual Power consumer"
                                .into(),
                        );
                    }
                    let (last, worked) = match state.phase {
                        phoenix_sim_gameplay::ship::transit::TransitPhase::Committed {
                            commit_tick,
                            ..
                        }
                        | phoenix_sim_gameplay::ship::transit::TransitPhase::Arrived {
                            commit_tick,
                            ..
                        } => (
                            Some(commit_tick),
                            f64::from(order.profile.config.preparation_work_secs),
                        ),
                        _ => (state.last_tick, state.worked_secs()),
                    };
                    let maximum_work =
                        last.filter(|last| *last >= order.accepted_tick)
                            .map_or(0.0, |last| {
                                ((u128::from(last) - u128::from(order.accepted_tick) + 1) as f64)
                                    * period
                            });
                    if worked > maximum_work {
                        return Err(
                            "Transit preparation exceeds its possible actual fixed work frontier"
                                .into(),
                        );
                    }
                }
                if matches!(
                    state.phase,
                    phoenix_sim_gameplay::ship::transit::TransitPhase::Committed { .. }
                        | phoenix_sim_gameplay::ship::transit::TransitPhase::Arrived { .. }
                ) {
                    let base = world
                        .get_resource::<crate::world::config::WorldConfig>()
                        .ok_or("saved arrival requires its accepted base World")?;
                    crate::console::helm::transit::placement::validate_saved(
                        state, &row.uuid, owner, base,
                    )?;
                }
            }
        }
        _ => {
            return Err("Transit saved state presence differs from accepted drive equipment".into())
        }
    }
    Ok(())
}

pub(super) fn validate_roster(
    world: &World,
    snapshot: &PhoenixSnapshot,
) -> Result<(), (String, String)> {
    let mut seen = std::collections::BTreeSet::new();
    for row in &snapshot.entities {
        if row.transit.is_some() && !seen.insert(&row.uuid) {
            return Err((
                row.uuid.clone(),
                "duplicate candidate Transit actor identity".into(),
            ));
        }
        let live = world
            .try_query_filtered::<(Entity, &EntityUuid), Allow<bevy::ecs::entity_disabling::Disabled>>()
            .and_then(|mut query| {
                query
                    .iter(world)
                    .find_map(|(entity, id)| (id.0 == row.uuid).then_some(entity))
            });
        let ship = accepted_ship(world, live, row).map_err(|reason| (row.uuid.clone(), reason))?;
        validate_row(world, row, live, ship.as_ref(), snapshot.tick)
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
    let ship = accepted_ship(world, Some(entity), row)?;
    validate_row(world, row, Some(entity), ship.as_ref(), tick)
}

/// Immutable equipment costs only; current reserves, flow and damage remain live state.
fn accepted_costs(
    world: &World,
    live: Option<Entity>,
    row: &EntityState,
) -> Result<std::collections::BTreeMap<SystemId, (f32, f32)>, String> {
    if row.fitting.is_some() {
        let (profile, _) = super::fitting::definitions(world, row)?;
        let profile =
            profile.ok_or("saved Transit fitting has no accepted equipment definition")?;
        let power = super::fitting::fitted_power(
            &profile,
            row.fitting.as_ref().expect("selected fitting"),
            row,
        )?;
        return Ok(power
            .0
            .config()
            .consumers
            .iter()
            .map(|consumer| {
                (
                    consumer.system.clone(),
                    (consumer.continuous_cost_per_sec, consumer.discrete_cost),
                )
            })
            .collect());
    }
    if let Some(origin) = &row.spawn {
        let mut warnings = Vec::new();
        let source = origin
            .resolve(&crate::entities::loader::WasmTemplateLoader, &mut warnings)
            .ok_or("cannot resolve accepted Transit Power recipe")?;
        if !warnings.is_empty() {
            return Err("accepted Transit Power recipe did not apply exactly".into());
        }
        let energy = source
            .energy
            .as_ref()
            .ok_or("accepted Transit equipment has no Power profile")?;
        Ok(energy
            .consumers
            .iter()
            .map(|consumer| {
                (
                    consumer.system.clone(),
                    (consumer.continuous_cost_per_sec, consumer.discrete_cost),
                )
            })
            .collect())
    } else {
        let power = live
            .and_then(|entity| world.get::<crate::ship::power::ShipPowerSystem>(entity))
            .ok_or("actual accepted Transit Power owner is missing")?;
        Ok(power
            .0
            .config()
            .consumers
            .iter()
            .map(|consumer| {
                (
                    consumer.system.clone(),
                    (consumer.continuous_cost_per_sec, consumer.discrete_cost),
                )
            })
            .collect())
    }
}

fn accepted_ship(
    world: &World,
    live: Option<Entity>,
    row: &EntityState,
) -> Result<Option<crate::ship::config::ShipConfig>, String> {
    if let Some(origin) = &row.spawn {
        let mut warnings = Vec::new();
        let source = origin
            .resolve(&crate::entities::loader::WasmTemplateLoader, &mut warnings)
            .ok_or("cannot resolve accepted Transit source recipe")?;
        if !warnings.is_empty() {
            return Err("accepted Transit source recipe did not apply exactly".into());
        }
        Ok(source.ship_config)
    } else {
        Ok(live
            .and_then(|entity| world.get::<crate::ship_plugin::ShipConfigComponent>(entity))
            .map(|ship| ship.0.clone()))
    }
}

#[cfg(test)]
#[path = "transit_tests.rs"]
mod tests;
