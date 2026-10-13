use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_TREES: usize = 32;
pub const MAX_STATES: usize = 128;
pub const MAX_TASKS: usize = 256;
pub const MAX_TRANSITIONS: usize = 256;
pub const MAX_DEPTH: usize = 16;
pub const MAX_VISITS: usize = 4096;
pub const MAX_CONDITIONS: usize = 16;
pub const MAX_VARIABLES: usize = 128;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AiTreeConfig {
    pub root: String,
    pub tree: Vec<TreeDef>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TreeDef {
    pub id: String,
    #[serde(default)]
    pub extends: Option<String>,
    #[serde(default)]
    pub initial: Option<String>,
    #[serde(default)]
    pub parameters: BTreeMap<String, bool>,
    #[serde(default)]
    pub memory: BTreeMap<String, bool>,
    #[serde(default)]
    pub state: Vec<StateDef>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StateDef {
    pub id: String,
    #[serde(default)]
    pub parent: Option<String>,
    #[serde(default)]
    pub initial_child: Option<String>,
    #[serde(default)]
    pub task: Vec<TaskDef>,
    #[serde(default)]
    pub transition: Vec<TransitionDef>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskDef {
    pub id: String,
    #[serde(default)]
    pub when: Vec<BoolCondition>,
    pub action: TaskAction,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TaskAction {
    Write {
        channel: String,
        value: BoolSource,
    },
    Remember {
        key: String,
        value: BoolSource,
    },
    Call {
        tree: String,
        #[serde(default)]
        parameters: BTreeMap<String, BoolSource>,
    },
    Complete {
        result: Completion,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransitionDef {
    pub id: String,
    #[serde(default)]
    pub when: Vec<BoolCondition>,
    pub target: TransitionTarget,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TransitionTarget {
    State { state: String },
    Complete { result: Completion },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Completion {
    Success,
    Failure,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoolCondition {
    pub value: BoolSource,
    pub equals: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case", deny_unknown_fields)]
pub enum BoolSource {
    Literal { value: bool },
    Fact { name: String },
    Parameter { name: String },
    Memory { name: String },
    Pending { channel: String },
    CallResult { task: String, result: Completion },
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SupportedBindings {
    pub facts: BTreeSet<String>,
    pub channels: BTreeSet<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SchemaMetadata {
    pub sources: Vec<&'static str>,
    pub actions: Vec<&'static str>,
    pub facts: Vec<String>,
    pub channels: Vec<String>,
    pub results: Vec<&'static str>,
    pub bounds: BTreeMap<&'static str, usize>,
}
pub fn schema_metadata(bindings: &SupportedBindings) -> SchemaMetadata {
    SchemaMetadata {
        sources: vec![
            "literal",
            "fact",
            "parameter",
            "memory",
            "pending",
            "call_result",
        ],
        actions: vec!["write", "remember", "call", "complete"],
        facts: bindings.facts.iter().cloned().collect(),
        channels: bindings.channels.iter().cloned().collect(),
        results: vec!["success", "failure"],
        bounds: BTreeMap::from([
            ("trees", MAX_TREES),
            ("states", MAX_STATES),
            ("tasks", MAX_TASKS),
            ("transitions", MAX_TRANSITIONS),
            ("depth", MAX_DEPTH),
            ("visits", MAX_VISITS),
            ("conditions", MAX_CONDITIONS),
            ("variables", MAX_VARIABLES),
        ]),
    }
}

/// Closed Dock observation contract; the channel name comes from composition.
pub fn dock_bindings(enabled_channel: &str) -> SupportedBindings {
    SupportedBindings {
        facts: ["order_present", "ready", "active", "engaged"]
            .map(str::to_owned)
            .into(),
        channels: [enabled_channel.to_owned()].into(),
    }
}
