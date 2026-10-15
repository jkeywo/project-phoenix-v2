//! Permitted docked service terms and the operator's own bounded order progress.
use crate::logistics_ui::LogisticsStoreView;
use crate::ports::{PortCost, PortServicePolicy};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortServiceView {
    pub partner_uuid: String,
    pub visit: u64,
    pub policy: PortServicePolicy,
    pub eligible: bool,
    pub offers: Vec<PortOfferView>,
    pub stores: Vec<LogisticsStoreView>,
    pub order: Option<PortOrderView>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortOfferView {
    pub id: String,
    pub resource: String,
    pub unit_amount: i64,
    /// Remaining whole lots for this visitor's physical visit.
    pub quota_remaining: i64,
    pub costs: Vec<PortCost>,
    pub available_lots: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortOrderView {
    pub active: bool,
    pub lines: Vec<PortOrderLineView>,
    pub blocked: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortOrderLineView {
    pub offer: String,
    pub store: String,
    pub requested_lots: i64,
    pub delivered_lots: i64,
}
