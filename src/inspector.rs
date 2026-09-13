//! Runtime-owned field metadata shared by source Authoring and constrained Live
//! inspectors. A descriptor describes a field; it never grants write authority.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LiveMutability {
    NamedAction,
    Derived,
    RecreateRequired,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldOrigin {
    pub schema_path: String,
    pub document: Option<String>,
    pub line: Option<usize>,
    pub layer: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldDescriptor {
    pub kind: String,
    pub default_source: Option<String>,
    pub live_mutability: LiveMutability,
    pub origin: FieldOrigin,
    /// Localized explanations of the owning adapter's validation, not a second
    /// schema evaluator in JavaScript.
    pub validation: Vec<String>,
}
