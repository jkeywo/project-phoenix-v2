//! Closed Job context; ordinary System AI still chooses admitted assignments.
use crate::ai::state_tree::*;
use std::collections::BTreeMap;
pub fn bindings() -> SupportedBindings {
    SupportedBindings {
        facts: [
            "internal",
            "travelling",
            "repairing",
            "returning",
            "external",
            "target_present",
        ]
        .map(str::to_owned)
        .into(),
        channels: [
            "advance_travel",
            "apply_repair",
            "advance_return",
            "apply_external",
        ]
        .map(str::to_owned)
        .into(),
    }
}
pub fn default_config() -> AiTreeConfig {
    AiTreeConfig {
        root: "repair-job".into(),
        tree: vec![TreeDef {
            id: "repair-job".into(),
            extends: None,
            initial: Some("execute".into()),
            parameters: BTreeMap::new(),
            memory: BTreeMap::new(),
            state: vec![StateDef {
                id: "execute".into(),
                parent: None,
                initial_child: None,
                transition: vec![],
                task: [
                    ("travel", "advance_travel", "travelling"),
                    ("repair", "apply_repair", "repairing"),
                    ("return", "advance_return", "returning"),
                    ("external", "apply_external", "external"),
                ]
                .into_iter()
                .map(|(id, channel, fact)| TaskDef {
                    id: id.into(),
                    when: vec![],
                    action: TaskAction::Write {
                        channel: channel.into(),
                        value: BoolSource::Fact { name: fact.into() },
                    },
                })
                .collect(),
            }],
        }],
    }
}
pub fn compile_workflow(config: Option<&AiTreeConfig>) -> Result<Program, TreeError> {
    compile(config.unwrap_or(&default_config()), &bindings())
}
