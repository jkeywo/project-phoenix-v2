//! The passive controller sees only local machinery facts, never hidden emitters.
use crate::ai::state_tree::{AiTreeConfig, SupportedBindings};
pub fn bindings() -> SupportedBindings {
    SupportedBindings {
        facts: ["enabled", "can_operate"].map(str::to_owned).into(),
        channels: ["operate".to_owned()].into(),
    }
}
pub fn initial_tree() -> AiTreeConfig {
    serde_json::from_value(serde_json::json!({"root":"passive","tree":[{"id":"passive","initial":"receive","state":[{
        "id":"receive","task":[{"id":"operate-capable-receiver","action":{"kind":"write","channel":"operate","value":{"source":"fact","name":"can_operate"}}}]
    }]}]})).expect("closed authored passive starter")
}
