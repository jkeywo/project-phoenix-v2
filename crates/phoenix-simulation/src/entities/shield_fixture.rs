//! Explicit powered hardware for narrow shield-schema/spawn fixtures. This
//! builds authored TOML and still passes through the ordinary production parser.
use crate::core::messages::{PowerGroupId, SystemId};
use crate::entities::config::{EnergyConsumerConfig, EnergyGroupConfig, ShieldArcEnergyConfig};
use crate::modifiers::power_system::{PowerConfig, PowerSystem};

pub(super) fn with_energy(source: &str) -> String {
    let mut value: toml::Value = toml::from_str(source).unwrap();
    let mut template = PowerSystem::new(&PowerConfig::default()).config().clone();
    let reserve = template.reactor.reserve;
    let coolant = template.reactor.coolant.clone();
    template.consumers.clear();
    template.groups.clear();
    let mut groups = toml::Table::new();
    if let Some(arcs) = value
        .get_mut("shield_arc")
        .and_then(toml::Value::as_array_mut)
    {
        for arc in arcs {
            let id = arc["id"].as_str().unwrap().to_owned();
            let system = SystemId(format!("shield-arc-{id}"));
            let group = PowerGroupId(format!("energy-{}", system.0));
            arc.as_table_mut().unwrap().insert(
                "energy".into(),
                toml::Value::try_from(ShieldArcEnergyConfig {
                    recovery_energy_per_hp: 0.0,
                    discharge_hp_per_sec: 0.0,
                    ..ShieldArcEnergyConfig::default()
                })
                .unwrap(),
            );
            arc.as_table_mut().unwrap().insert(
                "ai".into(),
                toml::from_str(include_str!(
                    "../../../../tests/fixtures/shield-arc-ai.toml"
                ))
                .unwrap(),
            );
            groups.insert(
                group.0.clone(),
                toml::from_str::<toml::Value>(
                    "label='Fixture shield'\ndefault_level=2\nmin_level=0\nmax_level=4",
                )
                .unwrap(),
            );
            template.groups.push(EnergyGroupConfig {
                id: group,
                parent: None,
                max_flow_per_sec: 4.0,
                default_flow_per_sec: 2.0,
                max_cooling_per_sec: 1.0,
                default_cooling_per_sec: 0.1,
                priority: 0,
                restore_delay_secs: 0.5,
            });
            template.consumers.push(EnergyConsumerConfig {
                system,
                initial_energy: 50.0,
                initial_heat: 0.0,
                continuous_cost_per_sec: 0.0,
                discrete_cost: 0.0,
                reserve,
                coolant: coolant.clone(),
            });
        }
    }
    if let Some(systems) = value.get_mut("system").and_then(toml::Value::as_array_mut) {
        for system in systems {
            let kind = system["kind"].as_str().unwrap();
            if !crate::entities::config::requires_energy_profile(kind) {
                continue;
            }
            let id = system["id"].as_str().unwrap().to_owned();
            let group = system
                .get("power_group")
                .and_then(toml::Value::as_str)
                .map(str::to_owned)
                .unwrap_or_else(|| format!("fixture-energy-{id}"));
            system
                .as_table_mut()
                .unwrap()
                .insert("power_group".into(), toml::Value::String(group.clone()));
            groups.insert(
                group.clone(),
                toml::from_str::<toml::Value>(
                    "label='Fixture equipment'\ndefault_level=2\nmin_level=0\nmax_level=4",
                )
                .unwrap(),
            );
            template.groups.push(EnergyGroupConfig {
                id: PowerGroupId(group),
                parent: None,
                max_flow_per_sec: 4.0,
                default_flow_per_sec: 2.0,
                max_cooling_per_sec: 1.0,
                default_cooling_per_sec: 0.1,
                priority: 0,
                restore_delay_secs: 0.5,
            });
            template.consumers.push(EnergyConsumerConfig {
                system: SystemId(id),
                initial_energy: 50.0,
                initial_heat: 0.0,
                continuous_cost_per_sec: 1.0,
                discrete_cost: 1.0,
                reserve,
                coolant: coolant.clone(),
            });
        }
    }
    let root = value.as_table_mut().unwrap();
    root.insert("power_groups".into(), toml::Value::Table(groups));
    root.insert("energy".into(), toml::Value::try_from(template).unwrap());
    root.insert(
        "infrastructure".into(),
        toml::from_str::<toml::Value>(
            "[[capacity]]\nid='fixture-coolant'\namount=100\nceiling=100\nlabel='Fixture coolant'",
        )
        .unwrap(),
    );
    toml::to_string(&value).unwrap()
}
