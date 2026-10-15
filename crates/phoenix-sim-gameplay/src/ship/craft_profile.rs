//! Accepted physical probe and disjoint bay bindings; no runtime custody store.
use super::{
    composition::{craft_bay_composition, validate_composition},
    system_registry::*,
};
use crate::entities::config::EntityConfig;
use std::collections::BTreeSet;

pub fn validate_entity(config: &EntityConfig) -> Result<(), String> {
    if let Some(probe) = &config.probe {
        probe.validate()?;
        let ship = config
            .ship_config
            .as_ref()
            .ok_or("probe requires its actual equipped Systems")?;
        let energy = config
            .energy
            .as_ref()
            .ok_or("probe requires real finite battery reserves")?;
        if config.helm_console.is_some()
            || config.helm_control.is_some()
            || config.behaviour.is_some()
            || config.civilian.is_some()
            || config.debris.is_some()
            || config.mesh.is_some()
            || config.star.is_some()
            || config.planet.is_some()
            || !config.light.is_empty()
            || config.asteroid_field.is_some()
            || config.shape.is_some()
            || config.field.is_some()
            || config.personnel.is_some()
            || config.civilian_rescue.is_some()
            || config.weapons_console.is_some()
            || config.torpedoes.is_some()
            || !config.shield_arcs.is_empty()
            || config.fitting.is_some()
            || config.tractor.is_some()
            || config.umbilical.is_some()
            || config.dock.is_some()
            || config.security.is_some()
            || config.transporter.is_some()
            || config.arrival_gravity.is_some()
            || config.collider.as_ref().is_none_or(|body| body.movable)
            || config.damage.is_none()
            || config.interior.is_none()
            || config
                .sensing
                .as_ref()
                .is_none_or(|sensing| sensing.broad.is_empty())
            || energy.reactor.max_output_per_sec != 0.0
            || energy.reactor.sustainable_output_per_sec != 0.0
            || energy.reactor.initial_output_per_sec != 0.0
            || ship.systems.iter().any(|system| {
                !matches!(
                    system.kind.as_str(),
                    POWER_REACTOR_KIND
                        | SENSOR_RADAR_KIND
                        | TACTICAL_RADAR_KIND
                        | PASSIVE_RECEIVER_KIND
                        | SENSORS_KIND
                        | COMMS_KIND
                )
            })
        {
            return Err("first probe must be battery-only engineless childless fixed physical sensor equipment with ordinary damage; moving, mesh-child, weapon, passenger, fitting, bay and other actuator profiles are not supported yet".into());
        }
    }
    let Some(ship) = &config.ship_config else {
        return Ok(());
    };
    let mut compartments = BTreeSet::new();
    for system in ship
        .systems
        .iter()
        .filter(|system| system.kind == CRAFT_BAY_KIND)
    {
        validate_composition(system)?;
        if config.damage.is_none()
            || !system.damage_components.iter().any(|component| {
                component.binding == phoenix_model::damage::DamageBinding::Operation
            })
        {
            return Err(
                "Craft Bay requires its actual component damage owner and Operation binding".into(),
            );
        }
        let bay = craft_bay_composition(&system.components).expect("validated actual bay");
        let interior = config
            .interior
            .as_ref()
            .ok_or("Craft Bay requires actual compartment/access layout")?;
        if interior.placements.get(&system.id) != Some(&bay.config.compartment)
            || !interior
                .compartments
                .iter()
                .any(|room| room.id == bay.config.compartment)
            || !compartments.insert(&bay.config.compartment)
            || config.logistics.as_ref().is_some_and(|logistics| {
                logistics
                    .storage
                    .iter()
                    .any(|storage| storage.compartment == bay.config.compartment)
            })
            || config.fitting.as_ref().is_some_and(|fitting| {
                fitting
                    .lockers
                    .iter()
                    .any(|locker| locker.compartment == bay.config.compartment)
            })
        {
            return Err("Craft Bay requires its own installed compartment allocation disjoint from other bays, cargo and equipment lockers".into());
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
            .ok_or("Craft Bay requires its actual energy consumer")?;
        if !consumer.continuous_cost_per_sec.is_finite() || consumer.continuous_cost_per_sec <= 0.0
        {
            return Err(
                "Craft Bay handling requires positive finite actual continuous work cost".into(),
            );
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "craft_profile_tests.rs"]
mod tests;
