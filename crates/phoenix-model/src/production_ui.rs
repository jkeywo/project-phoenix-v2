//! Console-scoped production readouts; authoritative work lives in Gameplay.
use crate::production::ProductionAmount;
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProductionBlackboard {
    pub enabled: bool,
    pub selected_recipe: Option<String>,
    pub target: i64,
    pub recipes: Vec<ProductionRecipeView>,
    pub active: Option<ProductionJobView>,
    pub can_start: bool,
    pub blocked: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProductionRecipeView {
    pub id: String,
    pub inputs: Vec<ProductionAmount>,
    pub outputs: Vec<ProductionAmount>,
    pub work_secs: f32,
    pub ceiling: i64,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProductionJobView {
    pub id: u64,
    pub recipe: String,
    pub progress: f32,
}
