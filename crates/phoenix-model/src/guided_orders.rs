//! Source-local launch/order knowledge; never a projection of remote physical flight.
use crate::{guided_guidance::GuidanceReceiverProfile, identity::SystemId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IssuedGuidanceOrder {
    pub revision: u64,
    pub issued_tick: u64,
    /// Source Console's public contact handle; None means stop guidance.
    pub target: Option<String>,
    /// Original sample time of a targeting order, never its delivery time.
    pub sampled_tick: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuidanceAcknowledgement {
    pub order: IssuedGuidanceOrder,
    /// When a viable return link actually brought the acknowledgement home.
    pub received_tick: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KnownGuidedLaunch {
    pub round_uuid: String,
    pub tube: SystemId,
    pub launched_tick: u64,
    /// Publicly known frozen launch endurance, not remote remaining life.
    pub lifespan_secs: f32,
    pub receiver: Option<GuidanceReceiverProfile>,
    /// Latest locally issued order. It does not simulate the physical intent.
    pub issued: IssuedGuidanceOrder,
    pub acknowledged: Option<GuidanceAcknowledgement>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KnownGuidedLaunchCommand {
    pub tube: SystemId,
    pub issued_tick: u64,
    pub target: Option<String>,
    pub sampled_tick: Option<u64>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuidedSourceKnowledge {
    /// Only identities this source actually emitted; no all-world roster.
    pub launches: BTreeMap<String, KnownGuidedLaunch>,
    /// Original accepted launch command, keyed by its first actual emitted ID.
    /// Kept only while the existing actual tube owns a pending burst.
    pub launch_commands: BTreeMap<String, KnownGuidedLaunchCommand>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuidedOrderView {
    pub round_uuid: String,
    pub tube: SystemId,
    pub target: Option<String>,
    pub issued_tick: u64,
    pub sampled_tick: Option<u64>,
    pub acknowledged_tick: Option<u64>,
    pub unresolved: bool,
    pub can_retarget: bool,
}
