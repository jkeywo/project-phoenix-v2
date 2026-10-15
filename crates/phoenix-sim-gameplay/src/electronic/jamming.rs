//! Finite receiver exposure; actual mounts, dated evidence and paid work remain
//! caller-owned. This has no receiver registry, actuator or signal history.
use crate::{
    regions::{field::normalized_weight, shape::RegionShape},
    weapons::mount::MountConfig,
};
use phoenix_model::field::FieldFalloff;
use serde::{Deserialize, Serialize};

/// One supported receiver effect, sharing the same physical emitter owner.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JammingEffect {
    #[default]
    Comms,
    Sensors,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JammingConfig {
    #[serde(default)]
    pub effect: JammingEffect,
    pub mount: MountConfig,
    pub falloff: FieldFalloff,
    pub strength: f32,
    pub maximum_sample_age_secs: f32,
    /// Same existing per-System reading frontier, not an emitter timer.
    pub reading_ticks: u32,
}
impl JammingConfig {
    pub fn validate(&self) -> Result<(), String> {
        self.mount.validate()?;
        if !self.strength.is_finite()
            || self.strength <= 0.0
            || !self.maximum_sample_age_secs.is_finite()
            || self.maximum_sample_age_secs <= 0.0
        {
            return Err("jamming requires finite positive strength and maximum sample age".into());
        }
        Ok(())
    }
    pub fn sample_fresh(&self, sampled_tick: u64, now: u64, fixed_secs: f64) -> bool {
        self.validate().is_ok()
            && sampled_tick <= now
            && fixed_secs.is_finite()
            && fixed_secs > 0.0
            && (now - sampled_tick) as f64 * fixed_secs <= f64::from(self.maximum_sample_age_secs)
    }
}

/// One emitter-to-receiver contribution. Natural conversation-path attenuation
/// is deliberately absent. Heading is the one actual shared mount bearing.
#[allow(clippy::too_many_arguments)]
pub fn exposure(
    profile: &JammingConfig,
    origin: [f32; 3],
    heading: f32,
    receiver: [f32; 3],
    natural_transmission: f32,
    paid_fraction: f32,
) -> f64 {
    if profile.validate().is_err()
        || !natural_transmission.is_finite()
        || !(0.0..=1.0).contains(&natural_transmission)
        || !paid_fraction.is_finite()
        || !(0.0..=1.0).contains(&paid_fraction)
        || !crate::weapons::mount::contains_3d(
            origin,
            heading,
            0.0,
            receiver,
            profile.mount.range,
            profile.mount.fire_arc_deg,
        )
    {
        return 0.0;
    }
    let point = glam::DVec3::from_array(receiver.map(f64::from))
        - glam::DVec3::from_array(origin.map(f64::from));
    let weight = normalized_weight(
        &RegionShape::Sphere {
            radius: profile.mount.range,
        },
        profile.falloff,
        point,
    );
    f64::from(profile.strength)
        * weight
        * f64::from(natural_transmission)
        * f64::from(paid_fraction)
}

pub fn bindings(enabled: &str, target: &str) -> crate::ai::state_tree::SupportedBindings {
    crate::ship::composition::enabled_target_bindings(enabled, target)
}

#[cfg(test)]
#[path = "jamming_tests.rs"]
mod tests;
