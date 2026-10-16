//! Closed first-probe physical custody and accepted bay data.
//! Actual entities, finite energy and physical jobs retain their existing owners.
use crate::identity::SystemId;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BayOperation {
    Launch,
    Recover,
}

/// Own stored identities and already-permitted dated contacts only. The actual
/// Bay Job/location owners remain private continuation state.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BayView {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub receptions: Vec<AcceptedCraftReception>,
    #[serde(default)]
    pub crew_transfer_in_progress: bool,
    pub can_launch: bool,
    pub can_recover: bool,
    pub capacity_space: u64,
    pub occupied_space: u64,
    pub stored: Vec<StoredCraftView>,
    pub contacts: Vec<RecoveryContactView>,
    pub job: Option<BayJobView>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoredCraftView {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub people: Vec<crate::personnel::PersonRef>,
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub space: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryContactView {
    pub contact: String,
    pub observed_tick: u64,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BayJobView {
    pub id: u64,
    pub operation: BayOperation,
    /// Launch uses an own stored/handled identity. Recovery uses a current
    /// permitted alias, or None when no such observation remains available.
    pub craft: Option<String>,
    pub fraction: f32,
    pub paused: bool,
    pub ready: bool,
    pub outcome: Option<crate::job::JobOutcome>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProbeProfile {
    pub space: u64,
    /// Empty for ordinary probes. Occupants live solely in PersonnelRoster.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub passenger_berths: Vec<String>,
}
impl ProbeProfile {
    pub fn validate(&self) -> Result<(), String> {
        if self.space == 0 {
            return Err("physical probe requires positive occupied bay space".into());
        }
        let mut ids = std::collections::BTreeSet::new();
        if self
            .passenger_berths
            .iter()
            .any(|id| !identity(id, 128) || !ids.insert(id))
        {
            return Err("passenger berths require unique accepted identities".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BayConfig {
    pub compartment: String,
    pub capacity_space: u64,
    pub launch_work_secs: f32,
    pub recovery_work_secs: f32,
    pub recovery_range: f32,
    pub maximum_parent_speed: f32,
    pub release_offset: [f32; 3],
}
impl BayConfig {
    pub fn validate(&self) -> Result<(), String> {
        if !identity(&self.compartment, 128)
            || self.capacity_space == 0
            || [
                self.launch_work_secs,
                self.recovery_work_secs,
                self.recovery_range,
            ]
            .iter()
            .any(|value| !value.is_finite() || *value <= 0.0)
            || !self.maximum_parent_speed.is_finite()
            || self.maximum_parent_speed < 0.0
            || !self.release_offset.iter().all(|value| value.is_finite())
        {
            return Err("bay requires actual compartment, positive space/work/range and finite stopped-parent geometry".into());
        }
        Ok(())
    }
}

/// Initial accepted base World-row binding; no entity or identity is minted here.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InitialCraftCustody {
    pub parent: String,
    pub bay: SystemId,
}
impl InitialCraftCustody {
    pub fn validate(&self) -> Result<(), String> {
        if !identity(&self.parent, 512) || !identity(&self.bay.0, 128) {
            return Err(
                "initial carried probe requires a declared parent and actual bay System".into(),
            );
        }
        Ok(())
    }
}

/// Sole actual probe location. Launching/recovering derive from the existing
/// actual BayJob, never another location phase/progress copy.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CraftLocation {
    Carried { parent_uuid: String, bay: SystemId },
    Deployed {},
}
impl Default for CraftLocation {
    fn default() -> Self {
        Self::Deployed {}
    }
}
impl CraftLocation {
    pub fn validate(&self, craft_uuid: &str) -> Result<(), String> {
        if !identity(craft_uuid, 128) {
            return Err("probe requires its actual bounded entity identity".into());
        }
        if let Self::Carried { parent_uuid, bay } = self {
            if !identity(parent_uuid, 128) || !identity(&bay.0, 128) || parent_uuid == craft_uuid {
                return Err("carried probe requires a distinct actual parent and bay".into());
            }
        }
        Ok(())
    }
}

fn identity(value: &str, bound: usize) -> bool {
    !value.trim().is_empty() && value.len() <= bound && !value.chars().any(char::is_control)
}

#[cfg(test)]
#[path = "craft_tests.rs"]
mod tests;

/// Own permitted machinery presentation, separate from actual private Job state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BayControlsView {
    pub receptions: Vec<AcceptedCraftReception>,
    pub crew_transfer_in_progress: bool,
    pub controllable: bool,
    pub can_launch: bool,
    pub can_recover: bool,
    pub can_cancel: bool,
    pub capacity_space: u64,
    pub occupied_space: u64,
    pub stored: Vec<StoredCraftView>,
    pub contacts: Vec<RecoveryContactView>,
    pub progress: Option<u16>,
    pub paused: bool,
    pub ready: bool,
    pub outcome: Option<crate::job::JobOutcome>,
}

/// Recipient-local consent, not a machinery handle or a space reservation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CraftReceptionOrder {
    pub request: crate::comms_request::CommsRequestReference,
    pub decision_tick: u64,
}
impl CraftReceptionOrder {
    pub fn craft(&self) -> Option<&str> {
        match &self.request.request {
            crate::comms_request::GenericCommsRequest::ReceiveCraft { craft } => Some(craft),
            _ => None,
        }
    }
    pub fn validate(&self, now: u64) -> Result<(), &'static str> {
        self.request.validate(now)?;
        if self.craft().is_none()
            || self.decision_tick < self.request.heard_tick
            || self.decision_tick > now
        {
            return Err("craft reception requires the exact heard receiving consent");
        }
        Ok(())
    }
}
/// Dated physical acquisition. This is historical evidence, never custody.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CraftReceptionReceipt {
    pub order: CraftReceptionOrder,
    pub scope: crate::activity::ActivityJob,
    pub craft: String,
    pub handoff_tick: u64,
    pub people: Vec<crate::personnel::PersonRef>,
}
impl CraftReceptionReceipt {
    pub fn validate(&self, now: u64) -> Result<(), &'static str> {
        self.order.validate(self.handoff_tick)?;
        if self.handoff_tick > now
            || self.scope.actor != self.order.request.recipient_ship
            || !identity(&self.scope.owner.0, 128)
            || self.scope.job.0 == 0
            || self.order.craft() != Some(self.craft.as_str())
            || self.people.windows(2).any(|rows| rows[0] >= rows[1])
            || self.people.iter().any(|person| person.validate().is_err())
        {
            return Err("invalid actual receiving Bay handoff receipt");
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcceptedCraftReception {
    pub source_name: String,
    pub order: CraftReceptionOrder,
    /// None while the receiver has no earned current contact for this claim.
    pub contact: Option<String>,
}
