//! Bind a drive to existing accepted Navigation, Power and Stock owners.
use super::{
    composition::{transit_composition, validate_composition},
    system_registry::*,
};
use crate::entities::config::EntityConfig;
use phoenix_model::{
    console::ConsoleObservationKind,
    logistics::{ResourceEndpoint, StoreRole},
};

pub fn validate_fixed_body(config: &EntityConfig) -> Result<(), String> {
    if let Some(policy) = &config.arrival_gravity {
        policy.validate()?;
        if config.tags.iter().any(|tag| tag == "ship")
            || config
                .collider
                .as_ref()
                .is_some_and(|collider| collider.movable)
            || config.helm_console.is_some()
            || config.behaviour.is_some()
            || config.civilian.is_some()
            || config.debris.is_some()
            || config.ship_config.as_ref().is_some_and(|ship| {
                ship.systems.iter().any(|system| {
                    system.kind == TRANSIT_DRIVE_KIND
                        || super::composition::dock_composition(&system.components)
                            .is_some_and(|dock| dock.config.moves_body)
                })
            })
        {
            return Err("arrival_gravity first supports ordinary fixed bodies only; remove moving Helm/AI/civilian/debris/Dock/Transit equipment from this source".into());
        }
    }
    Ok(())
}

pub fn validate_entity(config: &EntityConfig) -> Result<(), String> {
    validate_fixed_body(config)?;
    let Some(ship) = &config.ship_config else {
        return Ok(());
    };
    for system in ship
        .systems
        .iter()
        .filter(|system| system.kind == TRANSIT_DRIVE_KIND)
    {
        validate_composition(system)?;
        let binding = transit_composition(&system.components).expect("validated actual drive");
        let nav = ship
            .system(binding.navigation_source)
            .filter(|navigation| navigation.kind == NAVIGATION_KIND)
            .ok_or("transit_drive.navigation_source must name an actual Navigation System")?;
        let console = ship
            .console_for_system(&system.id)
            .ok_or("Transit requires its actual controlling Console")?;
        if nav.console.as_ref() != Some(&console.id)
            && !console.feeds.iter().any(|feed| {
                feed.source == nav.id && feed.kind == ConsoleObservationKind::Navigation
            })
        {
            return Err("Transit requires its exact Navigation source as an owned or explicit typed Console observation".into());
        }
        let consumer = config
            .energy
            .as_ref()
            .and_then(|energy| {
                energy
                    .consumers
                    .iter()
                    .find(|consumer| consumer.system == system.id)
            })
            .ok_or("Transit requires its own real energy.consumer")?;
        if !consumer.continuous_cost_per_sec.is_finite()
            || consumer.continuous_cost_per_sec <= 0.0
            || !consumer.discrete_cost.is_finite()
            || consumer.discrete_cost <= 0.0
        {
            return Err("Transit preparation and commit require positive finite continuous and discrete work costs".into());
        }
        let store = config
            .logistics
            .as_ref()
            .and_then(|logistics| {
                logistics
                    .stores
                    .iter()
                    .find(|store| store.id == binding.config.fuel.store)
            })
            .ok_or("Transit fuel requires an actual managed Local store")?;
        let compartment = config
            .interior
            .as_ref()
            .and_then(|interior| interior.placements.get(&system.id))
            .ok_or("Transit requires an actual installed compartment")?;
        if store.role != StoreRole::Local
            || store.system.as_ref() != Some(&system.id)
            || &store.compartment != compartment
        {
            return Err(
                "Transit fuel must use this drive's own Local store in its physical compartment"
                    .into(),
            );
        }
        let ResourceEndpoint::Capacity { id } = &store.endpoint else {
            return Err("Transit fuel requires an existing Capacity endpoint".into());
        };
        let capacity = config
            .infrastructure
            .as_ref()
            .and_then(|infrastructure| {
                infrastructure
                    .capacities
                    .iter()
                    .find(|capacity| &capacity.id == id)
            })
            .ok_or("Transit fuel capacity is absent")?;
        if capacity.ceiling.unwrap_or(capacity.amount) < binding.config.fuel.amount {
            return Err("Transit fuel requirement exceeds its actual finite store ceiling".into());
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "transit_profile_tests.rs"]
mod tests;
