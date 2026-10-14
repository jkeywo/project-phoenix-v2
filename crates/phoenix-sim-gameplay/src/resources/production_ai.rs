//! Facts are supplied by the owning Console, never read from hidden inventory.
use crate::ai::state_tree::{AiTreeConfig, SupportedBindings};
pub fn bindings() -> SupportedBindings {
    SupportedBindings {
        facts: ["enabled", "production_available"]
            .map(str::to_owned)
            .into(),
        channels: ["start".to_owned()].into(),
    }
}
pub fn initial_tree() -> AiTreeConfig {
    serde_json::from_value(serde_json::json!({"root":"production","tree":[{"id":"production","initial":"maintain","state":[{
        "id":"maintain","task":[{"id":"start-needed","action":{"kind":"write","channel":"start","value":{"source":"fact","name":"production_available"}}}]
    }]}]})).expect("closed authored production starter")
}
