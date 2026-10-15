//! Closed equipment and measured inputs of the existing guided projectile owner.
use crate::identity::{ConsoleId, SystemId};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeekerProfile {
    pub range: f32,
    /// Full three-dimensional forward cone, in degrees.
    pub fire_arc_deg: f32,
    pub minimum_transmission: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuidanceReceiverProfile {
    pub range: f32,
    pub minimum_transmission: f32,
    pub maximum_sample_age_secs: f32,
}

fn valid_link(range: f32, transmission: f32) -> bool {
    range.is_finite()
        && range > 0.0
        && transmission.is_finite()
        && transmission > 0.0
        && transmission <= 1.0
}

impl SeekerProfile {
    pub fn validate(&self) -> Result<(), String> {
        if !valid_link(self.range, self.minimum_transmission)
            || !self.fire_arc_deg.is_finite()
            || !(0.0..=360.0).contains(&self.fire_arc_deg)
        {
            return Err(
                "seeker requires finite positive range/transmission and a supported forward arc"
                    .into(),
            );
        }
        Ok(())
    }
}

impl GuidanceReceiverProfile {
    pub fn validate(&self) -> Result<(), String> {
        if !valid_link(self.range, self.minimum_transmission)
            || !self.maximum_sample_age_secs.is_finite()
            || self.maximum_sample_age_secs <= 0.0
        {
            return Err("guidance receiver requires finite positive range and transmission".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum GuidanceSource {
    /// An actual measurement by this round's frozen seeker equipment.
    Seeker,
    /// Retains original Console measurement provenance across a link receipt.
    Console {
        ship_uuid: String,
        console: ConsoleId,
        system: SystemId,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuidanceObservation {
    /// Physical intent identity; this is not a crew-facing contact handle.
    pub target_uuid: String,
    pub position: [f32; 3],
    pub sampled_tick: u64,
    pub source: GuidanceSource,
}

impl GuidanceObservation {
    pub fn validate(&self, now: u64) -> Result<(), String> {
        let valid_id =
            |id: &str| !id.is_empty() && id.len() <= 128 && !id.chars().any(char::is_control);
        if !valid_id(&self.target_uuid)
            || !self.position.iter().all(|value| value.is_finite())
            || self.sampled_tick > now
        {
            return Err("guidance observation requires a finite measured point, identity and nonfuture sample tick".into());
        }
        if let GuidanceSource::Console {
            ship_uuid,
            console,
            system,
        } = &self.source
        {
            if !valid_id(ship_uuid) || !valid_id(&console.0) || !valid_id(&system.0) {
                return Err(
                    "guidance observation is missing its original Console provenance".into(),
                );
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "guided_guidance_tests.rs"]
mod tests;
