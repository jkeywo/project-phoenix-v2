//! Accepted finite sensor/radio marker equipment, not a second craft family.
use crate::{
    entities::config::{ColliderShape, EntityConfig},
    ship::system_registry::*,
};
use phoenix_model::{console::ConsoleObservationKind, damage::DamageBinding};

pub fn validate_entity(config: &EntityConfig) -> Result<(), String> {
    let Some(marker) = &config.marking_beacon else {
        return Ok(());
    };
    marker.validate()?;
    let refuse = || {
        "marking beacon requires ordinary finite battery-only screenless sensor/radio equipment with physical damage and no other actuator".to_string()
    };
    let ship = config.ship_config.as_ref().ok_or_else(refuse)?;
    let energy = config.energy.as_ref().ok_or_else(refuse)?;
    let interior = config.interior.as_ref().ok_or_else(refuse)?;
    let radio = config
        .comms
        .as_ref()
        .and_then(|c| c.radio.as_ref())
        .ok_or_else(refuse)?;
    if !ship.stations.is_empty()
        || ship.systems.len() != 2
        || config.probe.is_some()
        || config.helm_console.is_some()
        || config.helm_control.is_some()
        || config.behaviour.is_some()
        || config.civilian.is_some()
        || config.civilian_rescue.is_some()
        || config.debris.is_some()
        || config.asteroid_field.is_some()
        || config.shape.is_some()
        || config.field.is_some()
        || config.signal_material.is_some()
        || config.effects.is_some()
        || config.star.is_some()
        || config.planet.is_some()
        || !config.light.is_empty()
        || config
            .mesh
            .as_ref()
            .is_some_and(|mesh| mesh.model.is_some())
        || config.held_response.is_some()
        || config.production.is_some()
        || config.recovery.is_some()
        || config.recoverable.is_some()
        || config.port_service.is_some()
        || config.refit_facility.is_some()
        || config.security_target.is_some()
        || config.demolition_target.is_some()
        || config.weapons_console.is_some()
        || config.torpedoes.is_some()
        || config.personnel.is_some()
        || config.fitting.is_some()
        || config.tractor.is_some()
        || config.dock.is_some()
        || config.umbilical.is_some()
        || config.security.is_some()
        || config.transporter.is_some()
        || config.arrival_gravity.is_some()
        || !config.shield_arcs.is_empty()
        || config.damage.is_none()
        || config
            .collider
            .as_ref()
            .is_none_or(|c| c.shape != ColliderShape::Ball || c.movable)
        || energy.reactor.max_output_per_sec != 0.0
        || energy.reactor.sustainable_output_per_sec != 0.0
        || energy.reactor.initial_output_per_sec != 0.0
        || energy.reactor.initial_energy != 0.0
        || energy
            .groups
            .iter()
            .any(|g| g.max_flow_per_sec != 0.0 || g.default_flow_per_sec != 0.0)
        || radio.system != marker.radio
        || radio.channels.len() != 1
        || radio.channels[0].id != marker.channel
        || radio.media.is_some()
        || radio.capture.is_some()
        || config.sensing.as_ref().is_none_or(|s| {
            s.broad.len() != 1
                || s.broad[0].system != marker.sensor
                || !s.focused.is_empty()
                || !s.passive.is_empty()
                || !s.sharing.is_empty()
        })
    {
        return Err(refuse());
    }
    for (owner, kind) in [
        (&marker.sensor, SENSOR_RADAR_KIND),
        (&marker.radio, COMMS_KIND),
    ] {
        let system = ship.system(owner).ok_or_else(refuse)?;
        let console = ship.console_for_system(owner).ok_or_else(refuse)?;
        let battery = energy
            .consumers
            .iter()
            .find(|battery| &battery.system == owner)
            .ok_or_else(refuse)?;
        if system.kind != kind
            || !system.ai_only
            || console.screen.is_some()
            || console.home_station.is_some()
            || !system
                .damage_components
                .iter()
                .any(|c| c.binding == DamageBinding::Operation)
            || !interior.placements.contains_key(owner)
            || battery.initial_energy <= 0.0
            || battery.initial_energy > battery.reserve.capacity
            || battery.continuous_cost_per_sec <= 0.0
        {
            return Err(refuse());
        }
        let expected = if kind == SENSOR_RADAR_KIND {
            ConsoleObservationKind::SensorRadar
        } else {
            ConsoleObservationKind::Comms
        };
        if !console
            .feeds
            .iter()
            .any(|feed| &feed.source == owner && feed.kind == expected)
            || (kind == COMMS_KIND
                && !console.feeds.iter().any(|feed| {
                    feed.source == marker.sensor && feed.kind == ConsoleObservationKind::SensorRadar
                }))
        {
            return Err(refuse());
        }
    }
    if energy.consumers.len() != 2
        || energy
            .consumers
            .iter()
            .find(|c| c.system == marker.radio)
            .is_none_or(|c| c.discrete_cost <= 0.0)
    {
        return Err(refuse());
    }
    Ok(())
}

/// Cross-recipe validation is called by the accepted dependency loader, not by
/// a controller or a runtime filesystem fallback.
pub fn validate_payload_recipe(
    template: &phoenix_model::marking::MarkingPayloadConfig,
    child: &EntityConfig,
) -> Result<(), String> {
    template.validate()?;
    if child.marking_beacon.is_none() {
        return Err("marking payload recipe has no actual beacon equipment".into());
    }
    validate_entity(child)
}

/// Validate authored references only against the accepted content closure.
/// Callers own source discovery; runtime machinery cannot fetch missing data.
pub fn validate_payload_dependencies<'a>(
    config: &EntityConfig,
    mut accepted: impl FnMut(&str) -> Option<&'a EntityConfig>,
) -> Result<(), String> {
    for payload in config
        .torpedoes
        .iter()
        .flat_map(|torpedoes| &torpedoes.tubes)
        .filter_map(|tube| tube.marking.as_ref())
    {
        let child = accepted(&payload.template).ok_or_else(|| format!(
            "marking payload {:?} is missing from the accepted content closure; include the complete finite beacon recipe", payload.template))?;
        validate_payload_recipe(payload, child)
            .map_err(|error| format!("marking payload {:?}: {error}", payload.template))?;
    }
    Ok(())
}

/// The actual body owns one causal attachment relation. Its pose is a checked
/// projection while attached, then the ordinary retained pose after detachment.
#[derive(bevy::prelude::Component, Clone, Debug, PartialEq)]
pub struct ShipMarkingAttachment(pub phoenix_model::marking::MarkingAttachment);

#[cfg(test)]
mod tests {
    use super::*;
    fn accepted() -> EntityConfig {
        let resolved =
            crate::test_fixtures::resolve_from_disk("assets/entities/marking_beacon.toml").unwrap();
        EntityConfig::from_toml(&resolved.toml).unwrap()
    }
    #[test]
    fn actual_beacon_recipe_resolves_complete_finite_ordinary_equipment() {
        let config = accepted();
        validate_payload_recipe(
            &phoenix_model::marking::MarkingPayloadConfig {
                template: "assets/entities/marking_beacon.toml".into(),
            },
            &config,
        )
        .unwrap();
        let ship = config.ship_config.as_ref().unwrap();
        assert!(ship.stations.is_empty());
        assert_eq!(ship.systems.len(), 2);
        assert_eq!(ship.consoles.len(), 2);
        let marker = config.marking_beacon.as_ref().unwrap();
        let radio_console = ship.console_for_system(&marker.radio).unwrap();
        assert!(radio_console.feeds.iter().any(|feed| {
            feed.source == marker.sensor && feed.kind == ConsoleObservationKind::SensorRadar
        }));
        let energy = config.energy.as_ref().unwrap();
        assert_eq!(energy.reactor.max_output_per_sec, 0.0);
        assert_eq!(energy.consumers.len(), 2);
        assert_eq!(
            energy
                .consumers
                .iter()
                .map(|consumer| consumer.initial_energy)
                .sum::<f32>(),
            36.0
        );
        let interior = config.interior.as_ref().unwrap();
        assert_eq!(interior.placements.len(), 2);
        assert!(config.damage.is_some());
        assert!(config.probe.is_none() && config.helm_console.is_none());
        let original =
            crate::test_fixtures::load_entity_config("assets/entities/battery_probe.toml").unwrap();
        assert_eq!(energy.reactor, original.energy.as_ref().unwrap().reactor);
        assert_eq!(
            energy.consumers[0],
            original.energy.as_ref().unwrap().consumers[0]
        );
        assert_eq!(config.sensing, original.sensing);
        assert_eq!(config.sensors_console, original.sensors_console);
        assert_eq!(config.damage, original.damage);
        let mut expected_interior = original.interior.unwrap();
        expected_interior.placements.insert(
            phoenix_model::identity::SystemId("comms".into()),
            "probe-core".into(),
        );
        assert_eq!(config.interior.as_ref().unwrap(), &expected_interior);
    }
    #[test]
    fn beacon_refuses_unfunded_or_unowned_equipment_and_competing_actuators() {
        let config = accepted();
        for fault in 0..10 {
            let mut bad = config.clone();
            match fault {
                0 => bad.energy.as_mut().unwrap().reactor.max_output_per_sec = 1.0,
                1 => bad.energy.as_mut().unwrap().consumers[1].initial_energy = 0.0,
                2 => bad.ship_config.as_mut().unwrap().systems[0].ai_only = false,
                3 => bad.ship_config.as_mut().unwrap().consoles.clear(),
                4 => {
                    let sensor = bad.marking_beacon.as_ref().unwrap().sensor.clone();
                    bad.marking_beacon.as_mut().unwrap().radio = sensor;
                }
                5 => bad
                    .comms
                    .as_mut()
                    .unwrap()
                    .radio
                    .as_mut()
                    .unwrap()
                    .channels
                    .clear(),
                6 => bad.interior.as_mut().unwrap().placements.clear(),
                7 => bad.ship_config.as_mut().unwrap().systems[0]
                    .damage_components
                    .clear(),
                8 => bad.energy.as_mut().unwrap().groups[0].max_flow_per_sec = 1.0,
                _ => {
                    let sensor = bad.marking_beacon.as_ref().unwrap().sensor.clone();
                    let radio = bad.marking_beacon.as_ref().unwrap().radio.clone();
                    let console = bad
                        .ship_config
                        .as_mut()
                        .unwrap()
                        .consoles
                        .iter_mut()
                        .find(|console| console.feeds.iter().any(|feed| feed.source == radio))
                        .unwrap();
                    console.feeds.retain(|feed| feed.source != sensor);
                }
            }
            assert!(
                validate_entity(&bad).is_err(),
                "invalid marker equipment {fault}"
            );
        }
    }

    #[test]
    fn actual_marking_launcher_resolves_complete_inherited_hardware_and_real_dependency() {
        let resolved =
            crate::test_fixtures::resolve_from_disk("assets/entities/marking_cruiser.toml")
                .unwrap();
        let config = EntityConfig::from_toml(&resolved.toml).unwrap();
        let child = accepted();
        validate_payload_dependencies(&config, |path| {
            (path == "assets/entities/marking_beacon.toml").then_some(&child)
        })
        .unwrap();
        assert!(validate_payload_dependencies(&config, |_| None)
            .unwrap_err()
            .contains("accepted content closure"));
        let mut incompatible = child.clone();
        incompatible.marking_beacon = None;
        assert!(validate_payload_dependencies(&config, |_| Some(&incompatible)).is_err());
        let original =
            crate::test_fixtures::load_entity_config("assets/entities/alliance_cruiser.toml")
                .unwrap();
        validate_payload_dependencies(&original, |_| None).unwrap();
        assert_eq!(config.ship_config.as_ref().unwrap().stations.len(), 6);
        let mut compare = config.clone();
        compare.name = original.name.clone();
        let tubes = &mut compare.torpedoes.as_mut().unwrap().tubes;
        assert_eq!(
            tubes.iter().filter(|tube| tube.marking.is_some()).count(),
            1
        );
        let tube = tubes
            .iter_mut()
            .find(|tube| tube.id == "fore_port")
            .unwrap();
        assert_eq!(
            tube.marking.as_ref().unwrap().template,
            "assets/entities/marking_beacon.toml"
        );
        tube.marking = None;
        // Canonical whole-recipe equality pins actual inherited Energy,
        // Interior, Console, finite stock, ordinary AI and independent tubes.
        assert_eq!(compare, original);
    }
}
