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

/// The designation and enquiry channels share one controller on their bound System.
pub fn focused_bindings() -> SupportedBindings {
    SupportedBindings {
        facts: [
            "can_operate",
            "request_available",
            "acquiring",
            "has_target",
        ]
        .map(str::to_owned)
        .into(),
        channels: ["designate", "enquire", "cancel"].map(str::to_owned).into(),
    }
}
pub fn focused_tree() -> AiTreeConfig {
    serde_json::from_value(serde_json::json!({"root":"focused","tree":[{"id":"focused","initial":"observe","state":[{"id":"observe","task":[
        {"id":"designation","action":{"kind":"write","channel":"designate","value":{"source":"fact","name":"can_operate"}}},
        {"id":"enquiry","action":{"kind":"write","channel":"enquire","value":{"source":"fact","name":"request_available"}}},
        {"id":"cancel","action":{"kind":"write","channel":"cancel","value":{"source":"literal","value":false}}}
    ]}]}]})).expect("closed focused starter")
}
pub fn targeting_bindings() -> SupportedBindings {
    SupportedBindings {
        facts: ["can_operate", "has_target"].map(str::to_owned).into(),
        channels: ["designate"].map(str::to_owned).into(),
    }
}
pub fn targeting_tree() -> AiTreeConfig {
    serde_json::from_value(serde_json::json!({"root":"targeting","tree":[{"id":"targeting","initial":"observe","state":[{"id":"observe","task":[
        {"id":"designation","action":{"kind":"write","channel":"designate","value":{"source":"fact","name":"can_operate"}}}
    ]}]}]})).expect("closed targeting starter")
}
