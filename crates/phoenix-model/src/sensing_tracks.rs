//! Permitted broad measurements. Contact routing identity is observer-local.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackStage {
    #[default]
    Return,
    Identified,
    Classified,
    Assessed,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrackIdentity {
    pub name: String,
    pub faction: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrackAssessment {
    pub alert: Option<bool>,
    pub weapons_restrained: Option<bool>,
    pub disabled: Option<bool>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SensorTrackSnapshot {
    pub contact_id: String,
    pub observed_tick: u64,
    pub position: [f32; 2],
    pub velocity: Option<[f32; 2]>,
    pub position_error: f32,
    pub stage: TrackStage,
    pub identity: Option<TrackIdentity>,
    pub identity_tick: Option<u64>,
    pub classification: Option<String>,
    pub classification_tick: Option<u64>,
    pub assessment: Option<TrackAssessment>,
    pub assessment_tick: Option<u64>,
    pub predicted: bool,
}
