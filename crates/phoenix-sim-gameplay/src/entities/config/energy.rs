//! Authored local energy and thermal equipment. These definitions carry no
//! alternate battery state or allocation-to-output multipliers.
use crate::core::messages::{PowerGroupId, SystemId};
use crate::resources::energy::EnergyConfig;
use crate::resources::service::{resolve_endpoint, ResourceEndpoint, ResourceLink};
use crate::ship::config::ShipConfig;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnergyConfigSection {
    pub reactor: EnergyReactorConfig,
    #[serde(default, rename = "consumer")]
    pub consumers: Vec<EnergyConsumerConfig>,
    #[serde(default, rename = "group")]
    pub groups: Vec<EnergyGroupConfig>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnergyReactorConfig {
    pub sustainable_output_per_sec: f32,
    pub max_output_per_sec: f32,
    pub ramp_per_sec: f32,
    pub initial_output_per_sec: f32,
    pub initial_energy: f32,
    pub initial_heat: f32,
    pub max_cooling_per_sec: f32,
    pub default_cooling_per_sec: f32,
    pub reserve: EnergyConfig,
    #[serde(default = "default_coolant_link")]
    pub coolant: ResourceLink,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnergyConsumerConfig {
    pub system: SystemId,
    pub initial_energy: f32,
    pub initial_heat: f32,
    pub continuous_cost_per_sec: f32,
    pub discrete_cost: f32,
    pub reserve: EnergyConfig,
    #[serde(default = "default_coolant_link")]
    pub coolant: ResourceLink,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnergyGroupConfig {
    pub id: PowerGroupId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<PowerGroupId>,
    pub max_flow_per_sec: f32,
    pub default_flow_per_sec: f32,
    pub max_cooling_per_sec: f32,
    pub default_cooling_per_sec: f32,
    pub priority: u16,
    pub restore_delay_secs: f32,
}

fn default_coolant_link() -> ResourceLink {
    ResourceLink {
        service: Some("coolant".into()),
        provider: None,
    }
}

/// Equipment that performs real work needs explicit local storage even on NPCs.
/// Aggregate/control-only surfaces do not add a second physical consumer.
pub fn requires_energy_profile(kind: &str) -> bool {
    use crate::ship::system_registry::*;
    matches!(
        kind,
        HELM_THRUST_KIND
            | HELM_STEERING_KIND
            | LATERAL_THRUST_KIND
            | VERTICAL_THRUST_KIND
            | HELM_IMPULSE_KIND
            | HELM_BOOST_KIND
            | HELM_RADAR_KIND
            | TACTICAL_RADAR_KIND
            | SENSOR_RADAR_KIND
            | PASSIVE_RECEIVER_KIND
            | SENSORS_KIND
            | NAVIGATION_KIND
            | COMMS_KIND
            | REPAIR_KIND
            | TRACTOR_KIND
            | DOCK_KIND
            | ILLUMINATION_KIND
            | JAMMING_KIND
            | SIGNAL_PROTECTION_KIND
            | TRANSIT_DRIVE_KIND
            | CRAFT_BAY_KIND
            | UMBILICAL_KIND
            | TRANSPORTER_KIND
            | SECURITY_KIND
            | TORPEDO_MAGAZINE_KIND
            | PHASER_BANK_KIND
            | BLASTER_BANK_KIND
            | TORPEDO_TUBE_KIND
            | SHIELD_ARC_KIND
    )
}

fn finite_nonnegative(owner: &str, field: &str, value: f32) -> Result<(), String> {
    if !value.is_finite() || value < 0.0 {
        return Err(format!("{owner}.{field} must be finite and non-negative"));
    }
    Ok(())
}

fn initial_state(
    owner: &str,
    reserve: &EnergyConfig,
    energy: f32,
    heat: f32,
) -> Result<(), String> {
    reserve
        .validate()
        .map_err(|error| format!("{owner}.reserve: {error}"))?;
    finite_nonnegative(owner, "initial_energy", energy)?;
    finite_nonnegative(owner, "initial_heat", heat)?;
    if energy > reserve.capacity {
        return Err(format!("{owner}.initial_energy exceeds reserve capacity"));
    }
    Ok(())
}

fn coolant_link(
    owner: &str,
    link: &ResourceLink,
    services: &BTreeMap<String, ResourceEndpoint>,
    capacities: &BTreeSet<String>,
) -> Result<(), String> {
    if link.is_empty() {
        return Err(format!(
            "{owner}.coolant requires an explicit service or provider"
        ));
    }
    let endpoint = resolve_endpoint("coolant", services, link)
        .map_err(|error| format!("{owner}.coolant: {error}"))?;
    match endpoint {
        ResourceEndpoint::Capacity { id } if capacities.contains(&id) => Ok(()),
        ResourceEndpoint::Capacity { id } => {
            Err(format!("{owner}.coolant capacity {id:?} is not declared"))
        }
        ResourceEndpoint::TorpedoMagazine {} => {
            Err(format!("{owner}.coolant must bind a capacity"))
        }
    }
}

impl EnergyConfigSection {
    /// Check local hardware, actual resource references, existing System/group
    /// identity and complete consumer coverage before materialization.
    pub fn validate(
        &self,
        ship: &ShipConfig,
        services: &BTreeMap<String, ResourceEndpoint>,
        capacities: &BTreeSet<String>,
    ) -> Result<(), String> {
        let reactor = &self.reactor;
        initial_state(
            "energy.reactor",
            &reactor.reserve,
            reactor.initial_energy,
            reactor.initial_heat,
        )?;
        for (field, value) in [
            (
                "sustainable_output_per_sec",
                reactor.sustainable_output_per_sec,
            ),
            ("max_output_per_sec", reactor.max_output_per_sec),
            ("ramp_per_sec", reactor.ramp_per_sec),
            ("initial_output_per_sec", reactor.initial_output_per_sec),
            ("max_cooling_per_sec", reactor.max_cooling_per_sec),
            ("default_cooling_per_sec", reactor.default_cooling_per_sec),
        ] {
            finite_nonnegative("energy.reactor", field, value)?;
        }
        if reactor.max_output_per_sec < reactor.sustainable_output_per_sec
            || reactor.initial_output_per_sec > reactor.max_output_per_sec
            || reactor.ramp_per_sec <= 0.0
            || reactor.default_cooling_per_sec > reactor.max_cooling_per_sec
        {
            return Err("energy.reactor requires nonnegative generation and positive ramp, sustainable <= maximum, initial output <= maximum and initial cooling <= maximum".into());
        }
        coolant_link("energy.reactor", &reactor.coolant, services, capacities)?;
        let groups: BTreeMap<_, _> = self.groups.iter().map(|group| (&group.id, group)).collect();
        if groups.len() != self.groups.len() {
            return Err("energy.group ids must be unique".into());
        }
        for group in &self.groups {
            let owner = format!("energy.group[{}]", group.id.0);
            if group.id.0.trim().is_empty() || !ship.power_groups.contains_key(&group.id) {
                return Err(format!("{owner} must name an authored power group"));
            }
            for (field, value) in [
                ("max_flow_per_sec", group.max_flow_per_sec),
                ("default_flow_per_sec", group.default_flow_per_sec),
                ("max_cooling_per_sec", group.max_cooling_per_sec),
                ("default_cooling_per_sec", group.default_cooling_per_sec),
                ("restore_delay_secs", group.restore_delay_secs),
            ] {
                finite_nonnegative(&owner, field, value)?;
            }
            if group.default_flow_per_sec > group.max_flow_per_sec
                || group.default_cooling_per_sec > group.max_cooling_per_sec
            {
                return Err(format!("{owner} default flow/cooling exceeds its maximum"));
            }
            let mut seen = BTreeSet::from([&group.id]);
            let mut parent = group.parent.as_ref();
            while let Some(id) = parent {
                if !seen.insert(id) {
                    return Err(format!("{owner} parent links contain a cycle"));
                }
                parent = groups
                    .get(id)
                    .ok_or_else(|| format!("{owner} parent {} is not declared", id.0))?
                    .parent
                    .as_ref();
            }
        }
        let consumers: BTreeSet<_> = self
            .consumers
            .iter()
            .map(|consumer| &consumer.system)
            .collect();
        if consumers.len() != self.consumers.len() {
            return Err("energy.consumer System references must be unique".into());
        }
        let aliases = |kind: &str| {
            matches!(
                kind,
                crate::ship::system_registry::POWER_REACTOR_KIND
                    | crate::ship::system_registry::POWER_BATTERY_KIND
                    | crate::ship::system_registry::POWER_KIND
            )
        };
        for consumer in &self.consumers {
            let owner = format!("energy.consumer[{}]", consumer.system.0);
            let system = ship
                .systems
                .iter()
                .find(|system| system.id == consumer.system)
                .ok_or_else(|| format!("{owner} System is not declared"))?;
            if aliases(&system.kind) {
                return Err(format!(
                    "{owner} cannot add a second reserve to reactor/battery controls"
                ));
            }
            if let Some(group) = &system.power_group {
                if !groups.contains_key(group) {
                    return Err(format!(
                        "{owner} power group {} has no energy.group declaration",
                        group.0
                    ));
                }
            } else {
                return Err(format!(
                    "{owner} requires an explicit power_group for energy distribution"
                ));
            }
            initial_state(
                &owner,
                &consumer.reserve,
                consumer.initial_energy,
                consumer.initial_heat,
            )?;
            finite_nonnegative(
                &owner,
                "continuous_cost_per_sec",
                consumer.continuous_cost_per_sec,
            )?;
            finite_nonnegative(&owner, "discrete_cost", consumer.discrete_cost)?;
            if ship.system(&consumer.system).is_some_and(|system| {
                matches!(
                    system.kind.as_str(),
                    crate::ship::system_registry::ILLUMINATION_KIND
                        | crate::ship::system_registry::JAMMING_KIND
                        | crate::ship::system_registry::SIGNAL_PROTECTION_KIND
                )
            }) && consumer.continuous_cost_per_sec <= 0.0
            {
                return Err(format!(
                    "{owner} emitter requires positive continuous work cost"
                ));
            }
            if consumer.discrete_cost > consumer.reserve.capacity {
                return Err(format!(
                    "{owner}.discrete_cost exceeds reserve capacity and can never be afforded"
                ));
            }
            coolant_link(&owner, &consumer.coolant, services, capacities)?;
        }
        for system in &ship.systems {
            if (system.power_group.is_some() || requires_energy_profile(&system.kind))
                && !aliases(&system.kind)
                && !consumers.contains(&system.id)
            {
                return Err(format!(
                    "powered System {} requires an explicit energy.consumer profile",
                    system.id.0
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "energy_tests.rs"]
mod tests;
