//! Closed authored logistics vocabulary. Quantities remain in their actual owners.
use crate::messages::SystemId;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ResourceEndpoint {
    Capacity { id: String },
    TorpedoMagazine {},
}
impl ResourceEndpoint {
    pub fn validate_resource(&self, resource: &str) -> Result<(), String> {
        if resource.trim().is_empty() {
            return Err("resource must have a non-empty name".into());
        }
        match self {
            Self::Capacity { id } if id.trim().is_empty() => {
                Err("capacity endpoint must have a non-empty id".into())
            }
            Self::TorpedoMagazine {} if resource != "torpedoes" => Err(format!(
                "torpedo_magazine endpoint only supports resource \"torpedoes\", got {resource:?}"
            )),
            _ => Ok(()),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StoreRole {
    Main,
    Local,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogisticsConfig {
    #[serde(rename = "store")]
    pub stores: Vec<ManagedStoreConfig>,
    #[serde(rename = "storage")]
    pub storage: Vec<StorageBayConfig>,
    #[serde(rename = "handling")]
    pub handling: Vec<HandlingConfig>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedStoreConfig {
    pub id: String,
    pub endpoint: ResourceEndpoint,
    pub resource: String,
    pub compartment: String,
    pub role: StoreRole,
    pub system: Option<SystemId>,
    pub source: Option<String>,
    pub target: i64,
    pub reserve: i64,
    #[serde(default)]
    pub priority: u16,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StorageBayConfig {
    pub compartment: String,
    pub space: i64,
    pub unit_space: BTreeMap<String, i64>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HandlingConfig {
    pub system: SystemId,
    pub max_active: u16,
    pub max_batch: i64,
    pub units_per_sec: f32,
    #[serde(default = "full_reorder_fraction")]
    pub reorder_fraction: f32,
}
fn full_reorder_fraction() -> f32 {
    1.0
}
