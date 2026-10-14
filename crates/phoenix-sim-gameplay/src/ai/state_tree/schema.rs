use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_TREES: usize = 32;
pub const MAX_STATES: usize = 128;
pub const MAX_TASKS: usize = 512;
pub const MAX_TRANSITIONS: usize = 256;
pub const MAX_DEPTH: usize = 16;
pub const MAX_VISITS: usize = 4096;
pub const MAX_CONDITIONS: usize = 16;
pub const MAX_VARIABLES: usize = 128;
pub const MAX_NUMERIC_BINDINGS: usize = 512;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AiTreeConfig {
    pub root: String,
    pub tree: Vec<TreeDef>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskDef {
    pub id: String,
    #[serde(default)]
    pub when: Vec<BoolCondition>,
    pub action: TaskAction,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TaskAction {
    /// A literal bounded machinery setting, admitted by the owning host.
    WriteNumber {
        channel: String,
        value: f64,
    },
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
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoolCondition {
    pub value: BoolSource,
    pub equals: bool,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case", deny_unknown_fields)]
pub enum BoolSource {
    /// Closed comparison of a permitted reading with an authored literal.
    NumberCompare {
        name: String,
        comparison: NumberComparison,
        value: f64,
    },
    Literal {
        value: bool,
    },
    Fact {
        name: String,
    },
    Parameter {
        name: String,
    },
    Memory {
        name: String,
    },
    Pending {
        channel: String,
    },
    CallResult {
        task: String,
        result: Completion,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NumberComparison {
    Less,
    LessOrEqual,
    Equal,
    GreaterOrEqual,
    Greater,
}
impl NumberComparison {
    pub fn compare(self, reading: f64, value: f64) -> bool {
        match self {
            Self::Less => reading < value,
            Self::LessOrEqual => reading <= value,
            Self::Equal => reading == value,
            Self::GreaterOrEqual => reading >= value,
            Self::Greater => reading > value,
        }
    }
}
/// The host supplies the exact permitted names and authored output bounds.
/// This never enables expressions, world access or arbitrary Workshop code.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NumericBindings {
    pub facts: BTreeSet<String>,
    pub channels: BTreeMap<String, NumericBounds>,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NumericBounds {
    pub min: f64,
    pub max: f64,
}
impl NumericBounds {
    pub fn contains(self, value: f64) -> bool {
        value.is_finite()
            && self.min.is_finite()
            && self.max.is_finite()
            && self.min <= self.max
            && value >= self.min
            && value <= self.max
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SupportedBindings {
    pub facts: BTreeSet<String>,
    pub channels: BTreeSet<String>,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SchemaMetadata {
    pub sources: Vec<&'static str>,
    pub actions: Vec<&'static str>,
    pub facts: Vec<String>,
    pub channels: Vec<String>,
    pub results: Vec<&'static str>,
    pub bounds: BTreeMap<&'static str, usize>,
    pub numeric_facts: Vec<String>,
    pub numeric_channels: BTreeMap<String, NumericBounds>,
    pub comparisons: Vec<&'static str>,
}
pub fn schema_metadata(bindings: &SupportedBindings) -> SchemaMetadata {
    schema_metadata_with_numbers(bindings, &NumericBindings::default())
}
pub fn schema_metadata_with_numbers(
    bindings: &SupportedBindings,
    numeric: &NumericBindings,
) -> SchemaMetadata {
    let mut sources = vec![
        "literal",
        "fact",
        "parameter",
        "memory",
        "pending",
        "call_result",
    ];
    let mut actions = vec!["write", "remember", "call", "complete"];
    if !numeric.facts.is_empty() {
        sources.push("number_compare");
    }
    if !numeric.channels.is_empty() {
        actions.push("write_number");
    }
    SchemaMetadata {
        sources,
        actions,
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
            ("numeric_bindings", MAX_NUMERIC_BINDINGS),
        ]),
        numeric_facts: numeric.facts.iter().cloned().collect(),
        numeric_channels: numeric.channels.clone(),
        comparisons: vec![
            "less",
            "less_or_equal",
            "equal",
            "greater_or_equal",
            "greater",
        ],
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
