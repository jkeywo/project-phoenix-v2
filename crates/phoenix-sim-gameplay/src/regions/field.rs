//! Closed spatial field profiles. Pose is a pure function of the saved anchor,
//! logical tick and fixed step; rendering cadence never advances a field.
use bevy::prelude::Component;
use glam::Vec3;
use serde::{Deserialize, Serialize};

use super::shape::RegionShape;

pub use phoenix_model::field::{FieldConfig, FieldEvolution, FieldFalloff};

/// Canonical causal parameters; replacing the profile creates a new anchor at
/// the current sampled origin. No mutable second clock or integrated velocity.
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegionFieldState {
    pub profile: FieldConfig,
    pub anchor_tick: u64,
    pub origin: [f32; 3],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FieldPose {
    pub origin: Vec3,
    pub scale: f32,
    pub strength: f32,
}

impl FieldPose {
    pub fn stationary(origin: Vec3) -> Self {
        Self {
            origin,
            scale: 1.0,
            strength: 1.0,
        }
    }
}

impl RegionFieldState {
    /// Validate a candidate against its actual geometry and effects before
    /// spawn, causal replacement or snapshot adoption. Maximum exposure can
    /// occur at either endpoint of the bounded analytic evolution.
    pub fn validate_environment(
        &self,
        shape: &RegionShape,
        effects: &[super::effects::RegionEffectKind],
    ) -> Result<(), String> {
        self.validate()?;
        shape.validate()?;
        let strength = self
            .profile
            .evolution
            .as_ref()
            .map_or(self.profile.strength, |end| {
                self.profile.strength.max(end.final_strength)
            });
        let scale = self
            .profile
            .evolution
            .as_ref()
            .map_or(1.0, |end| end.final_scale.max(1.0));
        let min_scale = self
            .profile
            .evolution
            .as_ref()
            .map_or(1.0, |end| end.final_scale.min(1.0));
        let finite_scaled =
            |value: f32, multiplier: f32| value.is_finite() && (value * multiplier).is_finite();
        let positive_geometry = |value: f32| {
            [min_scale, scale]
                .into_iter()
                .all(|scale| finite_scaled(value, scale) && value * scale > 0.0)
        };
        let dimensions: &[f32] = match shape {
            RegionShape::Sphere { radius } => std::slice::from_ref(radius),
            RegionShape::Box { half_extents, .. } => half_extents,
            RegionShape::Torus {
                inner_radius,
                outer_radius,
            } => {
                if *inner_radius != 0.0 && !positive_geometry(*inner_radius) {
                    return Err("field-scaled geometry must remain finite and valid".into());
                }
                if [min_scale, scale]
                    .into_iter()
                    .any(|scale| inner_radius * scale >= outer_radius * scale)
                {
                    return Err("field-scaled annulus must retain positive width".into());
                }
                std::slice::from_ref(outer_radius)
            }
        };
        if dimensions.iter().any(|value| !positive_geometry(*value)) {
            return Err("field-scaled geometry must remain positive and finite".into());
        }
        use super::effects::RegionEffectKind;
        for effect in effects {
            let valid = match effect {
                RegionEffectKind::SlowZone {
                    thrust_modifier,
                    yaw_rate_modifier,
                } => [thrust_modifier, yaw_rate_modifier]
                    .into_iter()
                    .flatten()
                    .all(|value| finite_scaled(*value, strength)),
                RegionEffectKind::RadarDampening { multiplier } => {
                    finite_scaled(*multiplier, strength)
                }
                RegionEffectKind::NebulaFog { density, color } => {
                    finite_scaled(*density, strength) && color.iter().all(|value| value.is_finite())
                }
                RegionEffectKind::DamageZone { dps, shield_pierce } => {
                    finite_scaled(*dps, strength) && shield_pierce.is_finite()
                }
                RegionEffectKind::BlocksImpulse
                | RegionEffectKind::CommsJam
                | RegionEffectKind::SensorBlind => true,
            };
            if !valid {
                return Err("field strength and effect must produce finite local values throughout evolution".into());
            }
        }
        Ok(())
    }

    pub fn new(profile: FieldConfig, anchor_tick: u64, origin: [f32; 3]) -> Result<Self, String> {
        let state = Self {
            profile,
            anchor_tick,
            origin,
        };
        state.validate()?;
        Ok(state)
    }

    pub fn validate(&self) -> Result<(), String> {
        self.profile.validate()?;
        if self.origin.iter().any(|value| !value.is_finite()) {
            return Err("field origin must be finite".into());
        }
        if self.profile.evolution.as_ref().is_some_and(|evolution| {
            self.origin
                .iter()
                .zip(evolution.translation)
                .any(|(origin, delta)| !(*origin + delta).is_finite())
        }) {
            return Err("field evolution endpoint must be finite".into());
        }
        Ok(())
    }

    pub fn sample_pose(&self, tick: u64, fixed_dt: f32) -> Result<FieldPose, String> {
        self.validate()?;
        if !fixed_dt.is_finite() || fixed_dt <= 0.0 {
            return Err("field sampling requires a positive finite fixed step".into());
        }
        let Some(evolution) = &self.profile.evolution else {
            return Ok(FieldPose {
                origin: Vec3::from_array(self.origin),
                scale: 1.0,
                strength: self.profile.strength,
            });
        };
        let t = ((tick.saturating_sub(self.anchor_tick) as f64 * fixed_dt as f64)
            / evolution.duration_secs as f64)
            .min(1.0);
        let interpolate = |start: f32, end: f32| ((1.0 - t) * start as f64 + t * end as f64) as f32;
        Ok(FieldPose {
            origin: Vec3::from_array(std::array::from_fn(|axis| {
                interpolate(
                    self.origin[axis],
                    self.origin[axis] + evolution.translation[axis],
                )
            })),
            scale: interpolate(1.0, evolution.final_scale),
            strength: interpolate(self.profile.strength, evolution.final_strength),
        })
    }
}

/// Local exposure is separate from path transmission. Legacy effects use a
/// stationary step profile of strength one when no [field] was authored.
pub fn local_exposure(
    shape: &RegionShape,
    pose: FieldPose,
    falloff: FieldFalloff,
    point: Vec3,
) -> f32 {
    if !pose.origin.is_finite()
        || !point.is_finite()
        || !pose.scale.is_finite()
        || pose.scale <= 0.0
        || !pose.strength.is_finite()
        || pose.strength <= 0.0
    {
        return 0.0;
    }
    let point = (point.as_dvec3() - pose.origin.as_dvec3()) / pose.scale as f64;
    (pose.strength as f64 * normalized_weight(shape, falloff, point)) as f32
}

pub(crate) fn normalized_weight(
    shape: &RegionShape,
    falloff: FieldFalloff,
    point: glam::DVec3,
) -> f64 {
    let radius = match shape {
        RegionShape::Sphere { radius } => point.length() / *radius as f64,
        RegionShape::Box { half_extents, yaw } => {
            let (sin, cos) = crate::simmath::sin_cos(*yaw);
            ((point.x * cos as f64 + point.z * sin as f64).abs() / half_extents[0] as f64)
                .max(point.y.abs() / half_extents[1] as f64)
                .max((-point.x * sin as f64 + point.z * cos as f64).abs() / half_extents[2] as f64)
        }
        RegionShape::Torus {
            inner_radius,
            outer_radius,
        } => {
            let radial = (point.x * point.x + point.z * point.z).sqrt();
            (radial - (*inner_radius as f64 + *outer_radius as f64) * 0.5).abs()
                / ((*outer_radius as f64 - *inner_radius as f64) * 0.5)
        }
    };
    if !radius.is_finite() || radius > 1.0 {
        return 0.0;
    }
    match falloff {
        FieldFalloff::Step => 1.0,
        FieldFalloff::Linear => 1.0 - radius.clamp(0.0, 1.0),
        FieldFalloff::Smooth => {
            let t = 1.0 - radius.clamp(0.0, 1.0);
            t * t * (3.0 - 2.0 * t)
        }
    }
}

#[cfg(test)]
#[path = "field_tests.rs"]
mod tests;
