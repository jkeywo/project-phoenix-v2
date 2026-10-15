//! Deliberate same-faction radar feeds. Receiving a sample grants information only.
use crate::identity::SystemId;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RadarSharingConfig {
    pub system: SystemId,
    #[serde(default)]
    pub share_own_faction: bool,
    #[serde(default = "receive_by_default")]
    pub receive: bool,
}
pub const fn receive_by_default() -> bool {
    true
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RadarSampleOrigin {
    /// Public stable origin handle, never the actor's private routing UUID.
    pub source: String,
    pub system: SystemId,
    pub contact: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RadarSampleProvenance {
    pub origin: RadarSampleOrigin,
    pub sample_id: String,
    pub observed_tick: u64,
    /// Pose of the original measuring instrument at sample time. A substituted
    /// report with no known original measuring pose leaves this absent.
    pub observer_position: Option<[f32; 2]>,
    pub velocity_error: f32,
    pub drift_error_per_sec: f32,
    /// First receipt by this observing owner; copying never renews sample time.
    pub received_tick: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RadarSharingView {
    pub owner: SystemId,
    pub share_own_faction: bool,
    pub receive: bool,
    /// Counts independently originated held samples, not relay hops.
    pub received_samples: u32,
    pub can_operate: bool,
}
