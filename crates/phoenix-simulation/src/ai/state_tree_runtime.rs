//! Per-System private tree execution state and immutable compiled programs.
use std::collections::BTreeMap;

use bevy::prelude::*;
use phoenix_sim_gameplay::ai::state_tree::{
    compile, dock_bindings, AiTreeConfig, Program, TreeRuntime,
};
use phoenix_sim_gameplay::ship::composition::dock_composition;

use crate::core::messages::SystemId;
use crate::ship::config::ShipConfig;

/// Authoritative continuation, isolated by the owning System identity.
#[derive(Component, Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ShipAiTreeRuntime(pub BTreeMap<SystemId, TreeRuntime>);

/// Immutable executable content, reconstructed on spawn rather than saved.
#[derive(Component, Clone, Default)]
pub struct ShipAiTreePrograms(pub BTreeMap<SystemId, Program>);

/// Compile once from validated content. Runtime callers never substitute a
/// built-in controller when a definition is absent or invalid.
pub fn compile_ship_trees(
    ship: &ShipConfig,
) -> Result<(ShipAiTreePrograms, ShipAiTreeRuntime), String> {
    compile_ship_trees_with_energy(ship, None)
}

pub fn compile_ship_trees_with_energy(
    ship: &ShipConfig,
    energy: Option<&phoenix_sim_gameplay::entities::config::EnergyConfigSection>,
) -> Result<(ShipAiTreePrograms, ShipAiTreeRuntime), String> {
    let mut programs = BTreeMap::new();
    let mut runtimes = BTreeMap::new();
    for system in &ship.systems {
        let Some(authored) = system.config.as_ref().and_then(|config| config.get("ai")) else {
            continue;
        };
        let definition: AiTreeConfig = authored
            .clone()
            .try_into()
            .map_err(|error| format!("System {} AI: {error}", system.id.0))?;
        let program = if system.kind == crate::ship::system_registry::POWER_REACTOR_KIND {
            let energy = energy.ok_or_else(|| {
                format!(
                    "Power System {} requires an energy hardware profile",
                    system.id.0
                )
            })?;
            phoenix_sim_gameplay::ai::state_tree::compile_with_numbers(
                &definition,
                &phoenix_sim_gameplay::resources::energy_ai::power_bindings(),
                &phoenix_sim_gameplay::resources::energy_ai::power_numeric_bindings(energy),
            )
        } else if system.kind == crate::ship::system_registry::SHIELD_ARC_KIND {
            let profile: phoenix_sim_gameplay::entities::config::ShieldArcEnergyConfig = system
                .config
                .as_ref()
                .and_then(|value| value.get("shield_energy"))
                .ok_or("missing shield energy bindings")?
                .clone()
                .try_into()
                .map_err(|error| format!("shield energy bindings: {error}"))?;
            let (booleans, numbers) =
                phoenix_sim_gameplay::ship::shield_ai::bindings(&profile.recovery_targets);
            phoenix_sim_gameplay::ai::state_tree::compile_with_numbers(
                &definition,
                &booleans,
                &numbers,
            )
        } else if system.kind == crate::ship::system_registry::SENSORS_KIND {
            compile(
                &definition,
                &phoenix_sim_gameplay::science::sensing_ai::focused_bindings(),
            )
        } else if system.kind == crate::ship::system_registry::TACTICAL_RADAR_KIND {
            compile(
                &definition,
                &phoenix_sim_gameplay::science::sensing_ai::targeting_bindings(),
            )
        } else if system.kind == crate::ship::system_registry::PASSIVE_RECEIVER_KIND {
            compile(
                &definition,
                &phoenix_sim_gameplay::science::sensing_ai::bindings(),
            )
        } else if system.kind == crate::ship::system_registry::RECOVERY_KIND {
            compile(
                &definition,
                &phoenix_sim_gameplay::resources::recovery_ai::bindings(),
            )
        } else if system.kind == crate::ship::system_registry::FABRICATION_KIND {
            compile(
                &definition,
                &phoenix_sim_gameplay::resources::production_ai::bindings(),
            )
        } else if system.kind == crate::ship::system_registry::LOGISTICS_KIND {
            compile(
                &definition,
                &phoenix_sim_gameplay::resources::logistics_ai::bindings(),
            )
        } else if system.kind == crate::ship::system_registry::CRAFT_BAY_KIND {
            compile(&definition, &phoenix_sim_gameplay::ship::craft::bindings())
        } else if system.kind == crate::ship::system_registry::TRANSIT_DRIVE_KIND {
            compile(
                &definition,
                &phoenix_sim_gameplay::ship::transit::bindings(),
            )
        } else if system.kind == crate::ship::system_registry::ILLUMINATION_KIND {
            let binding = phoenix_sim_gameplay::ship::composition::illumination_composition(
                &system.components,
            )
            .ok_or("illumination state tree requires emitter composition")?;
            compile(
                &definition,
                &phoenix_sim_gameplay::weapons::illumination::bindings(
                    binding.enabled_channel,
                    binding.target_channel,
                ),
            )
        } else if system.kind == crate::ship::system_registry::IDENTITY_EMITTER_KIND {
            let binding =
                phoenix_sim_gameplay::ship::composition::identity_composition(&system.components)
                    .ok_or("identity emitter state tree requires actual composition")?;
            compile(
                &definition,
                &phoenix_sim_gameplay::electronic::identity::bindings(binding.enabled_channel),
            )
        } else if system.kind == crate::ship::system_registry::SIGNAL_PROTECTION_KIND {
            let binding =
                phoenix_sim_gameplay::ship::composition::protection_composition(&system.components)
                    .ok_or("protection state tree requires actual composition")?;
            compile(
                &definition,
                &phoenix_sim_gameplay::electronic::protection::bindings(binding.enabled_channel),
            )
        } else if system.kind == crate::ship::system_registry::JAMMING_KIND {
            let binding =
                phoenix_sim_gameplay::ship::composition::jamming_composition(&system.components)
                    .ok_or("jamming state tree requires emitter composition")?;
            compile(
                &definition,
                &phoenix_sim_gameplay::electronic::jamming::bindings(
                    binding.enabled_channel,
                    binding.target_channel,
                ),
            )
        } else if system.kind == crate::ship::system_registry::ACCESS_GATE_KIND {
            let [phoenix_sim_gameplay::ship::composition::SystemComponentConfig::AccessBoundary {
                sealed_channel,
                ..
            }] = system.components.as_slice()
            else {
                return Err(format!(
                    "System {} requires access boundary composition",
                    system.id.0
                ));
            };
            compile(
                &definition,
                &phoenix_sim_gameplay::ship::access::bindings(sealed_channel),
            )
        } else {
            let dock = dock_composition(&system.components)
                .ok_or_else(|| format!("System {} has no supported tree bindings", system.id.0))?;
            compile(&definition, &dock_bindings(dock.enabled_channel))
        }
        .map_err(|error| format!("System {} AI: {error}", system.id.0))?;
        programs.insert(system.id.clone(), program);
        runtimes.insert(system.id.clone(), TreeRuntime::default());
    }
    Ok((ShipAiTreePrograms(programs), ShipAiTreeRuntime(runtimes)))
}

#[cfg(test)]
pub(crate) fn continued_tree() -> TreeRuntime {
    let definition: AiTreeConfig = serde_json::from_value(serde_json::json!({
        "root": "root", "tree": [
            {"id":"root", "initial":"hold", "memory":{"owned":false}, "state":[{
                "id":"hold", "task":[
                    {"id":"adopt", "action":{"kind":"remember", "key":"owned", "value":{"source":"literal", "value":true}}},
                    {"id":"child", "action":{"kind":"call", "tree":"child"}}
                ]
            }]},
            {"id":"child", "initial":"active", "memory":{"visited":false}, "state":[{
                "id":"active", "task":[
                    {"id":"remember", "action":{"kind":"remember", "key":"visited", "value":{"source":"literal", "value":true}}},
                    {"id":"engage", "action":{"kind":"write", "channel":"enabled", "value":{"source":"literal", "value":true}}}
                ]
            }]}
        ]
    })).unwrap();
    let program = compile(
        &definition,
        &phoenix_sim_gameplay::ai::state_tree::SupportedBindings {
            facts: Default::default(),
            channels: ["enabled".into()].into(),
        },
    )
    .unwrap();
    let mut state = TreeRuntime::default();
    let result = phoenix_sim_gameplay::ai::state_tree::step(
        &program,
        &BTreeMap::new(),
        &BTreeMap::from([("enabled".into(), false)]),
        &mut state,
        7,
    )
    .unwrap();
    assert_eq!(result.intents.get("enabled"), Some(&true));
    assert_eq!(state.memory("owned"), Some(true));
    state
}
