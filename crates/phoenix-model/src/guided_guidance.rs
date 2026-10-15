//! Closed equipment and measured inputs of the existing guided projectile owner.
use crate::{
    identity::{ConsoleId, SystemId},
    observation::ObservationMetadata,
    sensing_sharing::RadarSampleProvenance,
};
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum GuidanceSource {
    /// An actual measurement by this round's frozen seeker equipment.
    Seeker,
    /// Retains original Console measurement provenance across a link receipt.
    Console {
        ship_uuid: String,
        console: ConsoleId,
        system: SystemId,
        /// Original measuring samples, including remote receipt history.
        samples: Vec<RadarSampleProvenance>,
        /// Existing report substitution/uncertainty, never a fresh receipt.
        observation: Option<ObservationMetadata>,
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
        let valid_id = |id: &str| {
            !id.trim().is_empty() && id.len() <= 128 && !id.chars().any(char::is_control)
        };
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
            samples,
            observation,
        } = &self.source
        {
            if !valid_id(ship_uuid) || !valid_id(&console.0) || !valid_id(&system.0) {
                return Err(
                    "guidance observation is missing its original Console provenance".into(),
                );
            }
            let valid_text = |id: &str| {
                !id.trim().is_empty() && id.len() <= 1024 && !id.chars().any(char::is_control)
            };
            if samples.iter().any(|sample| {
                !valid_text(&sample.origin.source)
                    || !valid_id(&sample.origin.system.0)
                    || !valid_text(&sample.origin.contact)
                    || !valid_text(&sample.sample_id)
                    || sample.observed_tick > self.sampled_tick
                    || sample
                        .received_tick
                        .is_some_and(|received| received < sample.observed_tick || received > now)
                    || sample
                        .observer_position
                        .is_some_and(|point| point.iter().any(|coordinate| !coordinate.is_finite()))
                    || !sample.velocity_error.is_finite()
                    || sample.velocity_error < 0.0
                    || !sample.drift_error_per_sec.is_finite()
                    || sample.drift_error_per_sec < 0.0
            }) || observation.as_ref().is_some_and(|report| {
                !report.is_valid()
                    || report
                        .observed_tick
                        .is_some_and(|taken| taken > self.sampled_tick)
                    || report.received_tick.is_some_and(|received| {
                        received > now || report.observed_tick.is_some_and(|taken| received < taken)
                    })
            }) {
                return Err(
                    "guidance must retain finite, originally dated sample/report provenance".into(),
                );
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "guided_guidance_tests.rs"]
mod tests;
