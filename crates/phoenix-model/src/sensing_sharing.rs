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

impl RadarSampleProvenance {
    pub fn canonical_sample_id(&self) -> String {
        format!(
            "{}:{}{}:{}{}:{}:{}",
            self.origin.source.len(),
            self.origin.source,
            self.origin.system.0.len(),
            self.origin.system.0,
            self.origin.contact.len(),
            self.origin.contact,
            self.observed_tick
        )
    }
    pub fn valid_at(&self, tick: u64) -> bool {
        let value = self;
        let source = value.origin.source.strip_prefix("radar-origin-");
        let prefix = format!(
            "{}:{}:contact-",
            value.origin.system.0.len(),
            value.origin.system.0
        );
        let contact = value.origin.contact.strip_prefix(&prefix);
        source.is_some_and(|source| {
            source.len() == 16
                && source
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        }) && (!value.origin.system.0.is_empty()
            && value.origin.system.0.len() <= 1024
            && !value.origin.system.0.chars().any(char::is_control))
            && value.origin.system.0.len() <= 128
            && contact.is_some_and(|contact| {
                contact
                    .parse::<u64>()
                    .is_ok_and(|n| n > 0 && n < u64::MAX && n.to_string() == contact)
            })
            && value.sample_id == value.canonical_sample_id()
            && value.observed_tick <= tick
            && value
                .received_tick
                .is_none_or(|received| received >= value.observed_tick && received <= tick)
            && value
                .observer_position
                .is_none_or(|pose| pose.iter().all(|x| x.is_finite()))
            && value.velocity_error.is_finite()
            && value.velocity_error >= 0.0
            && value.drift_error_per_sec.is_finite()
            && value.drift_error_per_sec >= 0.0
    }
}
