//! Closed authored HTN schema. No physical state, controller or resource claim.
use crate::world::activities::{SituationCondition, SituationDefinition};
use phoenix_model::{
    activity::RepairGoal,
    damage::RepairWorkTarget,
    messages::{RepairTarget, SystemId},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const DOMAIN_FORMAT: u32 = 1;
pub const MAX_LIBRARIES: usize = 32;
pub const MAX_IMPORT_DEPTH: usize = 8;
pub const MAX_DEFINITIONS: usize = 256;
pub const MAX_EXPANDED_NODES: usize = 256;
pub const MAX_LEAVES: usize = 32;
pub const MAX_EXPLANATION_CHARS: usize = 512;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParameterKind {
    Actor,
    System,
    Team,
    RepairRequest,
    RepairTarget,
    RepairGoal,
    Store,
    Commodity,
    Quantity,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum Value {
    Actor(String),
    System(SystemId),
    Team(u8),
    RepairRequest(#[serde(deserialize_with = "deserialize_repair_request")] RepairTarget),
    RepairTarget(RepairWorkTarget),
    RepairGoal(RepairGoal),
    Store(String),
    Commodity(String),
    Quantity(i64),
}
// The ordinary legacy command enum predates closed source schemas. Preserve
// its exact type while rejecting ignored nested keys at this authoring edge.
fn deserialize_repair_request<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<RepairTarget, D::Error> {
    let source = serde_json::Value::deserialize(deserializer)?;
    let parsed: RepairTarget =
        serde_json::from_value(source.clone()).map_err(serde::de::Error::custom)?;
    let canonical = serde_json::to_value(&parsed).map_err(serde::de::Error::custom)?;
    if source != canonical {
        return Err(serde::de::Error::custom(
            "Repair request must use its exact closed command shape",
        ));
    }
    Ok(parsed)
}
impl Value {
    pub fn kind(&self) -> ParameterKind {
        match self {
            Self::Actor(_) => ParameterKind::Actor,
            Self::System(_) => ParameterKind::System,
            Self::Team(_) => ParameterKind::Team,
            Self::RepairRequest(_) => ParameterKind::RepairRequest,
            Self::RepairTarget(_) => ParameterKind::RepairTarget,
            Self::RepairGoal(_) => ParameterKind::RepairGoal,
            Self::Store(_) => ParameterKind::Store,
            Self::Commodity(_) => ParameterKind::Commodity,
            Self::Quantity(_) => ParameterKind::Quantity,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case", deny_unknown_fields)]
pub enum Argument {
    Parameter { name: String },
    Literal { value: Value },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Parameter {
    pub name: String,
    pub kind: ParameterKind,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DomainImport {
    pub path: String,
    #[serde(rename = "as")]
    pub namespace: String,
    pub names: Vec<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanningConfig {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required_commitment: Option<String>,
    pub task: String,
    pub arguments: BTreeMap<String, Value>,
    #[serde(rename = "import")]
    pub imports: Vec<DomainImport>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temporal: Option<super::temporal::TemporalPolicy>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperatorFamily {
    CommunicationReportStatus,
    RepairInternal,
    DeliveryLocal,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DomainHeader {
    pub format: u32,
    pub id: String,
    pub exports: Vec<String>,
    pub requires: Vec<OperatorFamily>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DomainLibrary {
    pub domain: DomainHeader,
    #[serde(default, rename = "import")]
    pub imports: Vec<DomainImport>,
    #[serde(rename = "task")]
    pub tasks: Vec<Task>,
    #[serde(default, rename = "situation_definition")]
    pub definitions: Vec<SituationDefinition>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Task {
    Compound {
        id: String,
        parameters: Vec<Parameter>,
        #[serde(rename = "method")]
        methods: Vec<Method>,
    },
    Primitive {
        id: String,
        parameters: Vec<Parameter>,
        definition: String,
        request: RequestTemplate,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        forecast: Option<super::temporal::ActivityForecast>,
    },
}
impl Task {
    pub fn id(&self) -> &str {
        match self {
            Self::Compound { id, .. } | Self::Primitive { id, .. } => id,
        }
    }
    pub fn parameters(&self) -> &[Parameter] {
        match self {
            Self::Compound { parameters, .. } | Self::Primitive { parameters, .. } => parameters,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Method {
    pub id: String,
    #[serde(rename = "step")]
    pub steps: Vec<TaskCall>,
    #[serde(default, rename = "edge")]
    pub edges: Vec<Dependency>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskCall {
    pub id: String,
    pub task: String,
    pub arguments: BTreeMap<String, Argument>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Dependency {
    /// Relative paths address expanded primitive nodes, never implicit compound
    /// completion or a different actor's unbound work.
    pub from: String,
    pub to: String,
    pub when: SituationCondition,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RequestTemplate {
    Communication {
        actor: Argument,
        owner: Argument,
        recipient: Argument,
        channel: String,
        request: phoenix_model::comms_request::GenericCommsRequest,
    },
    Repair {
        actor: Argument,
        owner: Argument,
        team: Argument,
        request: Argument,
        target: Argument,
        goal: Argument,
    },
    Delivery {
        actor: Argument,
        owner: Argument,
        source: Argument,
        destination: Argument,
        commodity: Argument,
        amount: Argument,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskIdentity {
    pub library_path: String,
    pub task: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MethodChoice {
    pub task: TaskIdentity,
    pub method: String,
}
/// A closed execution scope, independent of the immutable task source.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PlanExecution {
    #[default]
    Primary,
    Optional {
        source: String,
        ordinal: u16,
    },
}
impl PlanExecution {
    pub fn is_primary(&self) -> bool {
        matches!(self, Self::Primary)
    }
    pub fn namespace(&self, source: &str) -> String {
        match self {
            Self::Primary => format!("htn/{source}"),
            Self::Optional { source, ordinal } => format!("htn-optional/{source}/{ordinal}"),
        }
    }
}
/// The one saved choice frontier. Definitions and expanded leaves are derived
/// from frozen content; SituationInstances own physical acceptance and evidence.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanChoice {
    pub id: String,
    #[serde(default, skip_serializing_if = "PlanExecution::is_primary")]
    pub execution: PlanExecution,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required_commitment: Option<String>,
    pub root_arguments: BTreeMap<String, Value>,
    pub chosen_tick: u64,
    /// Accepted physical identity, validated through existing base/slot provenance.
    pub actor_uuid: String,
    /// Mission release identity; the matching SituationInstance owns execution.
    pub released: BTreeMap<String, u64>,
    pub methods: BTreeMap<String, MethodChoice>,
    /// Original immutable-capability decisions, never current physical facts.
    #[serde(default)]
    pub refused_methods: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temporal: Option<super::temporal::TemporalCommitment>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub temporal_reconsiderations: Vec<super::temporal::TemporalReconsideration>,
}
