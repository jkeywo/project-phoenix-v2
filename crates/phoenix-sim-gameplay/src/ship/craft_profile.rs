//! Accepted fixed carried equipment and disjoint bay bindings; no runtime custody store.
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
        let fixed_beam = if config.weapons_console.is_some() {
            validate_fixed_beam_turret(config)?;
            true
        } else {
            false
        };
        let identity_only = ship
            .systems
            .iter()
            .any(|system| system.kind == IDENTITY_EMITTER_KIND);
        if identity_only {
            validate_fixed_identity_decoy(config)?;
        }
        if config.helm_console.is_some()
            || config.helm_control.is_some()
            || config.behaviour.is_some()
            || config.civilian.is_some()
            || config.debris.is_some()
            || config
                .mesh
                .as_ref()
                .is_some_and(|mesh| mesh.model.is_some())
            || config.star.is_some()
            || config.planet.is_some()
            || !config.light.is_empty()
            || config.asteroid_field.is_some()
            || config.shape.is_some()
            || config.field.is_some()
            || config.personnel.is_some()
            || config.civilian_rescue.is_some()
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
            || (!identity_only
                && config
                    .sensing
                    .as_ref()
                    .is_none_or(|sensing| sensing.broad.is_empty()))
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
                ) && !(fixed_beam && system.kind == PHASER_BANK_KIND)
                    && !(identity_only && system.kind == IDENTITY_EMITTER_KIND)
            })
        {
            return Err("fixed carried equipment must be battery-only engineless childless fixed physical sensor, supported beam-turret or identity-only decoy equipment with ordinary damage; moving, mesh-child, projectile, own-crew, fitting, bay and other actuator profiles are not supported yet".into());
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

/// Select the existing static point-defence spawn path, never a new family
/// marker/controller. This narrow subtype does not widen ordinary probe/pod
/// acceptance to arbitrary ship weapons or actuators.
fn validate_fixed_beam_turret(config: &EntityConfig) -> Result<(), String> {
    use phoenix_model::console::ConsoleObservationKind;
    let refuse = || {
        "fixed beam turret requires ordinary static point-defence, AI-only radar/beam Systems with explicit permitted feeds, no passengers/projectiles, and positive finite actual local work costs".to_string()
    };
    if !config.is_static_point_defence()
        || config
            .probe
            .as_ref()
            .is_none_or(|profile| !profile.passenger_berths.is_empty())
    {
        return Err(refuse());
    }
    let ship = config.ship_config.as_ref().ok_or_else(refuse)?;
    let weapons = config.weapons_console.as_ref().ok_or_else(refuse)?;
    let energy = config.energy.as_ref().ok_or_else(refuse)?;
    if !ship.stations.is_empty()
        || !weapons.blaster_banks.is_empty()
        || weapons.selector.is_none()
        || weapons.selector_idle
    {
        return Err(refuse());
    }
    let radars: Vec<_> = ship
        .systems
        .iter()
        .filter(|system| system.kind == TACTICAL_RADAR_KIND)
        .collect();
    if radars.len() != 1 {
        return Err(refuse());
    }
    let radar = radars[0];
    let sensing = config.sensing.as_ref().ok_or_else(refuse)?;
    if sensing.control.combat_lock.as_ref() != Some(&radar.id)
        || !sensing
            .broad
            .iter()
            .any(|scanner| scanner.system == radar.id)
    {
        return Err(refuse());
    }
    let permitted = |owner: &super::config::SystemInstanceConfig,
                     source: &crate::core::messages::SystemId,
                     kind| {
        owner.ai_only
            && ship.console_for_system(&owner.id).is_some_and(|console| {
                console.screen.is_none()
                    && console.home_station.is_none()
                    && console
                        .feeds
                        .iter()
                        .any(|feed| &feed.source == source && feed.kind == kind)
            })
    };
    let paid = |owner: &crate::core::messages::SystemId| {
        energy.consumers.iter().any(|consumer| {
            &consumer.system == owner
                && consumer.continuous_cost_per_sec.is_finite()
                && consumer.continuous_cost_per_sec > 0.0
        })
    };
    if !permitted(radar, &radar.id, ConsoleObservationKind::TacticalRadar) || !paid(&radar.id) {
        return Err(refuse());
    }
    for bank in &weapons.phaser_banks {
        let id = phaser_bank_system_id(&bank.id).ok_or_else(refuse)?;
        let system = ship
            .system(&id)
            .filter(|system| system.kind == PHASER_BANK_KIND)
            .ok_or_else(refuse)?;
        if !permitted(system, &radar.id, ConsoleObservationKind::TacticalRadar)
            || !permitted(system, &id, ConsoleObservationKind::PhaserBank)
            || !permitted(
                system,
                &viewscreen_system_id(),
                ConsoleObservationKind::Viewscreen,
            )
            || !paid(&id)
        {
            return Err(refuse());
        }
    }
    if ship
        .systems
        .iter()
        .filter(|system| system.kind == PHASER_BANK_KIND)
        .count()
        != weapons.phaser_banks.len()
    {
        return Err(refuse());
    }
    Ok(())
}

/// Identity-only equipment needs no observer or targeting capability of its own.
/// Its finite ordinary emitter battery limits useful claim life; the physical
/// body remains recoverable without a new timer or destruction owner.
fn validate_fixed_identity_decoy(config: &EntityConfig) -> Result<(), String> {
    let refuse = || {
        "identity-only decoy requires one AI-only Console-owned IdentityEmitter, supported enabled composition/tree, finite paid local reserve, no source energy, passengers, sensing or weapons".to_string()
    };
    let ship = config.ship_config.as_ref().ok_or_else(refuse)?;
    let energy = config.energy.as_ref().ok_or_else(refuse)?;
    let profile = config.probe.as_ref().ok_or_else(refuse)?;
    if ship.systems.len() != 1
        || ship.consoles.len() != 1
        || !ship.stations.is_empty()
        || !profile.passenger_berths.is_empty()
        || config.weapons_console.is_some()
        || config.sensing.is_some()
        || config.sensors_console.is_some()
        || config.navigation_console.is_some()
        || config.comms.is_some()
        || config.comms_console.is_some()
        || energy.reactor.initial_energy != 0.0
        || energy.consumers.len() != 1
    {
        return Err(refuse());
    }
    let system = &ship.systems[0];
    let console = ship.console_for_system(&system.id).ok_or_else(refuse)?;
    if system.kind != IDENTITY_EMITTER_KIND
        || !system.ai_only
        || console.screen.is_some()
        || console.home_station.is_some()
        || !console.feeds.is_empty()
        || system
            .config
            .as_ref()
            .and_then(|value| value.get("ai"))
            .is_none()
        || system.components.len() != 2
        || energy.consumers[0].system != system.id
        || !energy.consumers[0].continuous_cost_per_sec.is_finite()
        || energy.consumers[0].continuous_cost_per_sec <= 0.0
        || !energy.consumers[0].initial_energy.is_finite()
        || energy.consumers[0].initial_energy <= 0.0
    {
        return Err(refuse());
    }
    crate::electronic::identity::validate_equipment(config, system)?;
    Ok(())
}

#[cfg(test)]
#[path = "craft_profile_tests.rs"]
mod tests;
