//! Dated Navigation observations. These are fixed points, never remote courses.
use crate::sensing_sharing::RadarSampleProvenance;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum NavigationSharedKind {
    Waypoint,
    Landmark { id: String, name: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NavigationSharedObservation {
    pub provenance: RadarSampleProvenance,
    /// The source's course/chart revision, independent of a held measurement's age.
    pub published_tick: u64,
    pub position: [f32; 2],
    pub kind: NavigationSharedKind,
}
