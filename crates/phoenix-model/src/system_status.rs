//! Derived, observed equipment status; never an authoritative machinery store.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Availability {
    Normal,
    Degraded,
    Unavailable,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StatusReasonCode {
    Damaged,
    Disabled,
    Destroyed,
    Offline,
    Unpowered,
    Overheated,
    IntakeCut,
    NoDockMarkers,
    EquipmentRemoved,
    Emp,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusReason {
    pub code: StatusReasonCode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub component_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemStatus {
    pub display_name: String,
    pub availability: Availability,
    pub reasons: Vec<StatusReason>,
    /// Authored/effective operator mode remains separate from availability.
    pub delegated: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fitting: Vec<FittingSlotView>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fitting_planner: Option<FittingPlannerContext>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub equipment_recovery: Option<crate::equipment_recovery_ui::EquipmentRecoveryView>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guidance: Option<crate::guided_orders::GuidanceControlsView>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub illumination: Option<crate::illumination_ui::IlluminationControlsView>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jamming: Option<crate::jamming_ui::JammingControlsView>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signal_protection: Option<crate::protection_ui::ProtectionControlsView>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FittingPlannerContext {
    pub hull_path: String,
    pub content_id: String,
    pub slots: Vec<FittingPlannerSlot>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FittingPlannerSlot {
    pub slot: String,
    pub system: crate::messages::SystemId,
    pub home_station: String,
    pub default_equipment: String,
    pub compatible_equipment: Vec<String>,
}

/// Permitted fitting facts only: no private component dose or AI memory.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FittingSlotView {
    pub slot: String,
    pub system: crate::messages::SystemId,
    pub installed: Option<String>,
    pub desired: Option<String>,
    pub choices: Vec<FittingChoiceView>,
    pub phase: String,
    pub progress: u16,
    pub can_request: bool,
    pub can_dispatch: bool,
    pub free_teams: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FittingChoiceView {
    pub equipment: String,
    pub available: u32,
    pub facility_required: bool,
    /// Numeric facts; each receiving device localizes the labelled forecast.
    pub family: String,
    pub statistics: std::collections::BTreeMap<String, String>,
}

impl SystemStatus {
    pub fn observe(
        display_name: String,
        damage: Option<crate::wire::DamageTier>,
        offline: bool,
        delegated: bool,
    ) -> Self {
        use crate::wire::DamageTier;
        let (availability, reason) = match damage {
            Some(DamageTier::Operational) => (Availability::Normal, None),
            Some(DamageTier::Damaged) => (Availability::Degraded, Some(StatusReasonCode::Damaged)),
            Some(DamageTier::Disabled) => {
                (Availability::Unavailable, Some(StatusReasonCode::Disabled))
            }
            Some(DamageTier::Destroyed) => {
                (Availability::Unavailable, Some(StatusReasonCode::Destroyed))
            }
            None => (Availability::Unknown, None),
        };
        let mut status = Self {
            display_name,
            availability,
            reasons: reason
                .into_iter()
                .map(|code| StatusReason {
                    code,
                    component_id: None,
                })
                .collect(),
            delegated,
            fitting: Vec::new(),
            fitting_planner: None,
            equipment_recovery: None,
            guidance: None,
            illumination: None,
            jamming: None,
            signal_protection: None,
        };
        if offline && !matches!(damage, Some(DamageTier::Disabled | DamageTier::Destroyed)) {
            status.unavailable(StatusReasonCode::Offline, None);
        }
        status
    }
    pub fn unavailable(&mut self, code: StatusReasonCode, component_id: Option<String>) {
        self.availability = Availability::Unavailable;
        self.reasons.push(StatusReason { code, component_id });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_evidence_is_unknown_and_multiple_causes_remain_distinct() {
        let mut status = SystemStatus::observe("dock".into(), None, false, true);
        assert_eq!(status.availability, Availability::Unknown);
        status.unavailable(StatusReasonCode::Unpowered, Some("supply".into()));
        status.unavailable(StatusReasonCode::NoDockMarkers, Some("actuator".into()));
        assert_eq!(status.reasons.len(), 2);
        assert_eq!(status.availability, Availability::Unavailable);
        assert!(status.delegated);
    }
    #[test]
    fn damaged_is_degraded_but_complete_loss_is_unavailable() {
        use crate::wire::DamageTier;
        assert_eq!(
            SystemStatus::observe("x".into(), Some(DamageTier::Damaged), false, false).availability,
            Availability::Degraded
        );
        let status = SystemStatus::observe("x".into(), Some(DamageTier::Destroyed), true, false);
        assert_eq!(status.availability, Availability::Unavailable);
        assert_eq!(status.reasons.len(), 1);
    }
}
