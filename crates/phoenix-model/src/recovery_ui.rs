//! Console-scoped recovery information, with categorical source manifests only.
use crate::logistics_ui::LogisticsStoreView;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryBlackboard {
    pub enabled: bool,
    pub target_uuid: Option<String>,
    pub resource: Option<String>,
    pub destination_store: Option<String>,
    pub fraction: f64,
    pub candidates: Vec<RecoveryCandidate>,
    pub destinations: Vec<LogisticsStoreView>,
    pub next_recovery: Option<RecoverySelection>,
    pub last_moved: i64,
    pub blocked: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryCandidate {
    pub uuid: String,
    pub name: String,
    pub distance: f32,
    pub resources: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoverySelection {
    pub target_uuid: String,
    pub resource: String,
    pub destination_store: String,
}
