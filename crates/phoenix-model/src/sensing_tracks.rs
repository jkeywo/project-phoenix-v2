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

/// A conditional projection of permitted measured motion, never a guidance
/// solution, collision guarantee or hidden target order.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservedApproach {
    pub contact: String,
    pub observed_tick: u64,
    pub predicted: bool,
    pub seconds: f32,
    pub own_position: [f32; 2],
    pub contact_position: [f32; 2],
    pub position_error: f32,
    pub separation: [f32; 2],
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrackIdentity {
    pub name: String,
    pub faction: Option<String>,
    /// Routine held reference records whose published name matches this
    /// observed identity. A routing UUID is never an identification proof.
    #[serde(default)]
    pub references: Vec<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrackAssessment {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protection: Option<ObservedShieldCover>,
    pub alert: Option<bool>,
    pub weapons_restrained: Option<bool>,
    pub disabled: Option<bool>,
}
/// The shielding actually observed along one ray, not a complete shield map.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservedShieldCover {
    /// World azimuth from the subject towards the observing instrument.
    pub incoming_sector: u16,
    pub blocked: bool,
}
impl ObservedShieldCover {
    pub fn validate(&self) -> Result<(), String> {
        if self.incoming_sector >= 360 {
            Err("invalid observed shield bearing".into())
        } else {
            Ok(())
        }
    }
}
/// Height belongs to the same dated observation as planar position. Velocity
/// is inferred from supported samples, never copied from an actor's live state.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservedElevation {
    pub height: f32,
    pub error: f32,
    pub velocity: Option<f32>,
}
impl ObservedElevation {
    pub fn validate(&self) -> Result<(), String> {
        if !self.height.is_finite()
            || !self.error.is_finite()
            || self.error < 0.0
            || self.velocity.is_some_and(|velocity| !velocity.is_finite())
        {
            return Err("invalid observed elevation".into());
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SensorTrackSnapshot {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub references: Vec<crate::sensing_references::AttachedSensorReference>,
    /// Original independent samples supporting this projection. Fusion is a
    /// derived view and does not create another sample identity.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub samples: Vec<crate::sensing_sharing::RadarSampleProvenance>,
    pub contact_id: String,
    /// A substituted report retains its original sample time and public source.
    /// Its subject is this instrument's contact handle, never a routing UUID.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observation: Option<crate::observation::ObservationMetadata>,
    /// The observing instrument permits acquisition of this return. This is
    /// independent of whether its identity or classification is known yet.
    pub selectable: bool,
    pub observed_tick: u64,
    pub position: [f32; 2],
    pub elevation: Option<ObservedElevation>,
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
