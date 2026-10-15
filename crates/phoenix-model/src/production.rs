//! Closed finite-production vocabulary; stock remains in its installed owner.
use crate::messages::SystemId;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProductionConfig {
    pub system: SystemId,
    #[serde(rename = "recipe")]
    pub recipes: Vec<ProductionRecipeConfig>,
    pub initial_recipe: Option<String>,
    /// Desired stock in the first output store, including held/incoming units.
    pub initial_target: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProductionRecipeConfig {
    pub id: String,
    pub inputs: Vec<ProductionAmount>,
    /// Authored order identifies the primary stock target; every output is kept.
    pub outputs: Vec<ProductionAmount>,
    pub work_secs: f32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProductionAmount {
    pub store: String,
    pub amount: i64,
}
