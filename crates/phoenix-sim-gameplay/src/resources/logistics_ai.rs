//! Closed facts supplied from the owning Console; no world or stock access.
use crate::ai::state_tree::{AiTreeConfig, SupportedBindings};

pub fn bindings() -> SupportedBindings {
    SupportedBindings {
        facts: ["enabled", "delivery_available"].map(str::to_owned).into(),
        channels: ["dispatch".to_owned()].into(),
    }
}
/// Authored starter, not a second controller or runtime fallback.
pub fn initial_tree() -> AiTreeConfig {
    serde_json::from_value(serde_json::json!({"root":"logistics","tree":[{"id":"logistics","initial":"supply","state":[{
        "id":"supply","task":[{"id":"dispatch-needed","action":{"kind":"write","channel":"dispatch","value":{"source":"fact","name":"delivery_available"}}}]
    }]}]})).expect("closed authored logistics starter")
}
