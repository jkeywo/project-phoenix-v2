//! Permitted equipment-recovery observations; no private component condition.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EquipmentRecoveryOption {
    pub target_uuid: String,
    pub target_name: String,
    pub slot: String,
    pub equipment: String,
    pub destination_locker: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EquipmentRecoveryView {
    pub options: Vec<EquipmentRecoveryOption>,
    pub selected: Option<EquipmentRecoveryOption>,
    /// Completed thousandths of actual paid work.
    pub progress: u16,
    pub phase: String,
    pub can_start: bool,
    pub can_cancel: bool,
}
