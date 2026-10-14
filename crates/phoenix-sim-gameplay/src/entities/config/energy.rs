//! Authored local energy and thermal equipment. These definitions carry no
//! alternate battery state or allocation-to-output multipliers.
use crate::core::messages::{PowerGroupId, SystemId};
use crate::resources::energy::EnergyConfig;
use crate::resources::service::{ResourceEndpoint, ResourceLink, resolve_endpoint};
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
        if reactor.sustainable_output_per_sec <= 0.0
            || reactor.max_output_per_sec < reactor.sustainable_output_per_sec
            || reactor.initial_output_per_sec > reactor.max_output_per_sec
            || reactor.ramp_per_sec <= 0.0
            || reactor.default_cooling_per_sec > reactor.max_cooling_per_sec
        {
            return Err("energy.reactor requires positive sustainable output/ramp, sustainable <= maximum, initial output <= maximum and initial cooling <= maximum".into());
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
            if consumer.discrete_cost > consumer.reserve.capacity {
                return Err(format!(
                    "{owner}.discrete_cost exceeds reserve capacity and can never be afforded"
                ));
            }
            coolant_link(&owner, &consumer.coolant, services, capacities)?;
        }
        for system in &ship.systems {
            if system.power_group.is_some()
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
mod tests {
    use super::*;
    use crate::ship::config::{PowerGroupConfig, SystemInstanceConfig};

    fn reserve() -> EnergyConfig {
        EnergyConfig {
            capacity: 100.0,
            max_intake_per_sec: 20.0,
            passive_loss_at_full_per_sec: 1.0,
            charge_heat_per_unit: 0.1,
            discharge_heat_per_unit: 0.1,
            passive_cooling_per_sec: 0.2,
            coolant_heat_per_unit: 10.0,
            trip_heat: 50.0,
            recover_heat: 30.0,
        }
    }
    fn fixture() -> (
        EnergyConfigSection,
        ShipConfig,
        BTreeMap<String, ResourceEndpoint>,
        BTreeSet<String>,
    ) {
        let group = PowerGroupId("propulsion".into());
        let ship = ShipConfig {
            consoles: vec![],
            stations: vec![],
            systems: vec![SystemInstanceConfig {
                components: vec![],
                channels: vec![],
                id: SystemId("impulse".into()),
                kind: "impulse".into(),
                console: None,
                ai_only: false,
                power_group: Some(group.clone()),
                marker: None,
                config: None,
            }],
            power_groups: std::collections::HashMap::from([(
                group.clone(),
                PowerGroupConfig {
                    label: "propulsion".into(),
                    default_level: 2,
                    min_level: 0,
                    max_level: 4,
                },
            )]),
            coordination_lag_secs: 2.0,
        };
        let energy = EnergyConfigSection {
            reactor: EnergyReactorConfig {
                sustainable_output_per_sec: 20.0,
                max_output_per_sec: 40.0,
                ramp_per_sec: 10.0,
                initial_output_per_sec: 20.0,
                initial_energy: 80.0,
                initial_heat: 0.0,
                max_cooling_per_sec: 5.0,
                default_cooling_per_sec: 1.0,
                reserve: reserve(),
                coolant: default_coolant_link(),
            },
            consumers: vec![EnergyConsumerConfig {
                system: SystemId("impulse".into()),
                initial_energy: 50.0,
                initial_heat: 0.0,
                continuous_cost_per_sec: 10.0,
                discrete_cost: 0.0,
                reserve: reserve(),
                coolant: default_coolant_link(),
            }],
            groups: vec![EnergyGroupConfig {
                id: group,
                parent: None,
                max_flow_per_sec: 20.0,
                default_flow_per_sec: 10.0,
                max_cooling_per_sec: 5.0,
                default_cooling_per_sec: 1.0,
                priority: 0,
                restore_delay_secs: 0.5,
            }],
        };
        (
            energy,
            ship,
            BTreeMap::from([(
                "coolant".into(),
                ResourceEndpoint::Capacity {
                    id: "coolant-tank".into(),
                },
            )]),
            BTreeSet::from(["coolant-tank".into()]),
        )
    }
    #[test]
    fn authored_energy_round_trips_and_default_link_binds_actual_stock() {
        let (energy, ship, services, capacities) = fixture();
        assert!(energy.validate(&ship, &services, &capacities).is_ok());
        let source = toml::to_string(&energy).unwrap();
        let decoded: EnergyConfigSection = toml::from_str(&source).unwrap();
        assert_eq!(decoded, energy);
        assert!(
            energy
                .validate(&ship, &BTreeMap::new(), &capacities)
                .unwrap_err()
                .contains("not declared")
        );
        assert!(
            energy
                .validate(&ship, &services, &BTreeSet::new())
                .unwrap_err()
                .contains("capacity")
        );
    }
    #[test]
    fn rejects_missing_duplicate_unknown_and_alias_consumers() {
        let (mut energy, mut ship, services, capacities) = fixture();
        energy.consumers.clear();
        assert!(
            energy
                .validate(&ship, &services, &capacities)
                .unwrap_err()
                .contains("requires an explicit")
        );
        let (good, _, _, _) = fixture();
        energy = good;
        energy.consumers.push(energy.consumers[0].clone());
        assert!(
            energy
                .validate(&ship, &services, &capacities)
                .unwrap_err()
                .contains("unique")
        );
        energy.consumers.pop();
        energy.consumers[0].system = SystemId("absent".into());
        assert!(
            energy
                .validate(&ship, &services, &capacities)
                .unwrap_err()
                .contains("not declared")
        );
        energy.consumers[0].system = SystemId("impulse".into());
        ship.systems[0].kind = crate::ship::system_registry::POWER_BATTERY_KIND.into();
        assert!(
            energy
                .validate(&ship, &services, &capacities)
                .unwrap_err()
                .contains("second reserve")
        );
        energy.consumers.clear();
        assert!(energy.validate(&ship, &services, &capacities).is_ok());
    }
    #[test]
    fn rejects_cycles_missing_groups_and_excess_group_defaults() {
        let (mut energy, ship, services, capacities) = fixture();
        energy.groups[0].parent = Some(energy.groups[0].id.clone());
        assert!(
            energy
                .validate(&ship, &services, &capacities)
                .unwrap_err()
                .contains("cycle")
        );
        energy.groups[0].parent = Some(PowerGroupId("absent".into()));
        assert!(
            energy
                .validate(&ship, &services, &capacities)
                .unwrap_err()
                .contains("not declared")
        );
        energy.groups[0].parent = None;
        energy.groups[0].default_flow_per_sec = 21.0;
        assert!(
            energy
                .validate(&ship, &services, &capacities)
                .unwrap_err()
                .contains("exceeds")
        );
        energy.groups.clear();
        assert!(
            energy
                .validate(&ship, &services, &capacities)
                .unwrap_err()
                .contains("no energy.group")
        );
    }
    #[test]
    fn rejects_nonfinite_negative_excess_initial_and_invalid_thermal_hardware() {
        let (mut energy, ship, services, capacities) = fixture();
        energy.consumers[0].initial_energy = 101.0;
        assert!(
            energy
                .validate(&ship, &services, &capacities)
                .unwrap_err()
                .contains("exceeds reserve")
        );
        energy.consumers[0].initial_energy = 50.0;
        energy.consumers[0].discrete_cost = f32::NAN;
        assert!(
            energy
                .validate(&ship, &services, &capacities)
                .unwrap_err()
                .contains("finite")
        );
        energy.consumers[0].discrete_cost = -1.0;
        assert!(
            energy
                .validate(&ship, &services, &capacities)
                .unwrap_err()
                .contains("non-negative")
        );
        energy.consumers[0].discrete_cost = 1.0;
        energy.reactor.reserve.recover_heat = energy.reactor.reserve.trip_heat;
        assert!(energy.validate(&ship, &services, &capacities).is_err());
    }
    #[test]
    fn closed_schema_refuses_old_direct_output_and_unknown_fields() {
        let (energy, _, _, _) = fixture();
        let source = toml::to_string(&energy).unwrap();
        for field in ["power_multipliers = [1.0, 2.0]\n", "capacity = 99.0\n"] {
            assert!(toml::from_str::<EnergyConfigSection>(&format!("{field}{source}")).is_err());
        }
    }
    #[test]
    fn rejects_unaffordable_actions_invalid_restoration_delay_and_excess_reactor_cooling() {
        let (mut energy, ship, services, capacities) = fixture();
        energy.consumers[0].discrete_cost = energy.consumers[0].reserve.capacity + 1.0;
        assert!(
            energy
                .validate(&ship, &services, &capacities)
                .unwrap_err()
                .contains("can never be afforded")
        );
        energy.consumers[0].discrete_cost = 1.0;
        energy.groups[0].restore_delay_secs = f32::INFINITY;
        assert!(
            energy
                .validate(&ship, &services, &capacities)
                .unwrap_err()
                .contains("finite")
        );
        energy.groups[0].restore_delay_secs = 0.5;
        energy.reactor.default_cooling_per_sec = energy.reactor.max_cooling_per_sec + 1.0;
        assert!(
            energy
                .validate(&ship, &services, &capacities)
                .unwrap_err()
                .contains("cooling")
        );
    }
}
