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
        let dock = dock_composition(&system.components)
            .ok_or_else(|| format!("System {} has no supported tree bindings", system.id.0))?;
        let bindings = dock_bindings(dock.enabled_channel);
        let program = compile(&definition, &bindings)
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
