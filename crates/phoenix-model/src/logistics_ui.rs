//! Permitted logistics readouts. These contain no authoritative stock or jobs.
use crate::identity::SystemId;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogisticsBlackboard {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub planned_requests: Vec<crate::activity::PlannedDeliveryRequest>,
    pub enabled: bool,
    pub max_batch: i64,
    pub max_active: u16,
    pub stores: Vec<LogisticsStoreView>,
    pub deliveries: Vec<DeliveryView>,
    pub next_delivery: Option<DeliveryRequest>,
    pub refusal: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogisticsStoreView {
    pub id: String,
    pub resource: String,
    pub role: String,
    pub system: Option<SystemId>,
    pub level: i64,
    pub available: i64,
    pub held: i64,
    pub ceiling: i64,
    pub headroom: i64,
    pub bay_free_space: i64,
    pub incoming: i64,
    pub target: i64,
    pub reserve: i64,
    pub priority: u16,
    pub compartment: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeliveryView {
    pub id: u64,
    pub source: String,
    pub destination: String,
    pub quantity: i64,
    pub stage: String,
    pub compartment: String,
    pub progress: f32,
    pub return_requested: bool,
    pub blocked: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeliveryRequest {
    pub source: String,
    pub destination: String,
    pub amount: i64,
}

/// A publication of the owning System's actual local stores, never a stock owner.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SystemStockBlackboard {
    pub system: SystemId,
    pub stores: Vec<LocalStockView>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalStockView {
    pub id: String,
    pub resource: String,
    pub level: i64,
    pub available: i64,
    pub ceiling: i64,
    pub headroom: i64,
    pub bay_free_space: i64,
    pub incoming: i64,
    pub held: i64,
    pub target: i64,
    pub compartment: String,
}
pub fn stock_publication(system: &SystemId) -> SystemId {
    SystemId(format!("stock/{}", system.0))
}
