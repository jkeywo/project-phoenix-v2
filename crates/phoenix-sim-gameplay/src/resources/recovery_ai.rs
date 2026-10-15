//! Ordinary per-System policy consumes only its permitted recovery facts.
use crate::ai::state_tree::{AiTreeConfig, SupportedBindings};

pub fn bindings() -> SupportedBindings {
    SupportedBindings {
        facts: ["enabled", "recovery_available"].map(str::to_owned).into(),
        channels: ["recover".to_owned()].into(),
    }
}

pub fn initial_tree() -> AiTreeConfig {
    serde_json::from_value(serde_json::json!({"root":"recovery","tree":[{"id":"recovery","initial":"recover","state":[{
        "id":"recover","task":[{"id":"recover-available","action":{"kind":"write","channel":"recover","value":{"source":"fact","name":"recovery_available"}}}]
    }]}]})).expect("closed authored recovery starter")
}
