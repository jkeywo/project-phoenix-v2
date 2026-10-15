//! Own finite equipment choices and paid allocation only; no hidden exposure.
use crate::identity::SystemId;
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProtectionSystemOption {
    pub system: SystemId,
    pub display_name: String,
    pub cost: u32,
    pub home_station: Option<crate::identity::StationId>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProtectionControlsView {
    pub enabled: bool,
    pub priorities: Vec<SystemId>,
    pub capacity: u32,
    pub systems: Vec<ProtectionSystemOption>,
    /// Actual own paid operation, independent of whether interference exists.
    pub active: bool,
    pub protected_systems: Vec<SystemId>,
    pub controllable: bool,
}
#[cfg(test)]
#[path = "protection_ui_tests.rs"]
mod tests;
