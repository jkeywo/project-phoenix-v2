//! Shared mount geometry and shield/hull delivery, without controller or cycle state.
//!
//! Beam, straight projectile and guided projectile strategies retain their own
//! lifecycles. These contracts describe the reusable geometry and the payload
//! each strategy presents to the existing damage owner.

use crate::simmath;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MountConfig {
    /// Clockwise degrees from ship forward.
    pub facing_deg: f32,
    /// Full arc width, including its boundary.
    pub fire_arc_deg: f32,
    /// Effective reach in world units.
    pub range: f32,
}

impl MountConfig {
    pub fn validate(&self) -> Result<(), String> {
        if !(self.facing_deg.is_finite()
            && self.fire_arc_deg.is_finite()
            && 0.0 < self.fire_arc_deg
            && self.fire_arc_deg <= 360.0
            && self.range.is_finite()
            && self.range > 0.0)
        {
            return Err(
                "mount requires finite facing, fire_arc_deg in (0, 360] and positive finite range"
                    .into(),
            );
        }
        Ok(())
    }

    pub fn geometry(
        &self,
        target_x: f32,
        target_z: f32,
        ship_x: f32,
        ship_z: f32,
        ship_yaw: f32,
    ) -> crate::core::messages::WeaponTargetGeometry {
        target_geometry(
            target_x,
            target_z,
            ship_x,
            ship_z,
            ship_yaw,
            self.range,
            self.facing_deg,
            self.fire_arc_deg,
        )
    }
}

/// A resolved payload at the moment of impact, after the strategy's own
/// fractional accumulation or strike modifier. No duplicate projectile state.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DamageDelivery {
    pub damage_hull: f32,
    pub damage_shields: f32,
    pub shield_pierce: f32,
}

impl DamageDelivery {
    pub fn validate(&self) -> Result<(), String> {
        if !self.damage_hull.is_finite()
            || self.damage_hull < 0.0
            || !self.damage_shields.is_finite()
            || self.damage_shields < 0.0
            || !self.shield_pierce.is_finite()
            || !(0.0..=1.0).contains(&self.shield_pierce)
        {
            return Err(
                "delivery requires finite nonnegative damage and shield_pierce in [0, 1]".into(),
            );
        }
        Ok(())
    }

    pub fn payload(&self, shields_up: bool) -> f32 {
        if shields_up {
            self.damage_shields
        } else {
            self.damage_hull
        }
    }

    /// `(direct hull, shield-routed)` damage. A down shield has nothing to
    /// pierce: the entire hull payload is delivered directly.
    pub fn split(&self, shields_up: bool) -> (f32, f32) {
        if shields_up {
            crate::ship::damage::split_damage_for_pierce(self.damage_shields, self.shield_pierce)
        } else {
            (self.damage_hull, 0.0)
        }
    }
}

/// World-space target expressed in the existing ship-local radar frame.
pub fn ship_local(
    target_x: f32,
    target_z: f32,
    ship_x: f32,
    ship_z: f32,
    ship_yaw: f32,
) -> (f32, f32) {
    let dx = target_x - ship_x;
    let dz = target_z - ship_z;
    let cos_y = simmath::cos(ship_yaw);
    let sin_y = simmath::sin(ship_yaw);
    let radar_x = dx * cos_y + dz * sin_y;
    let radar_y = dx * sin_y - dz * cos_y;
    (radar_x, radar_y)
}

/// True when a ship-relative radian bearing lies inside the full authored arc.
pub fn bearing_in_arc(bearing_rad: f32, facing_deg: f32, arc_deg: f32) -> bool {
    let half = arc_deg.to_radians() * 0.5;
    let facing = facing_deg.to_radians();
    super::arc_geometry::signed_difference(bearing_rad, facing).abs() <= half
}

pub fn in_arc(radar_x: f32, radar_y: f32, facing_deg: f32, arc_deg: f32) -> bool {
    let bearing = simmath::atan2(radar_x, radar_y);
    bearing_in_arc(bearing, facing_deg, arc_deg)
}

/// Readiness geometry uses the exact existing arithmetic, including the squared
/// distance gate. Callers keep their ordinary target knowledge and admission.
#[allow(clippy::too_many_arguments)]
pub fn target_geometry(
    target_x: f32,
    target_z: f32,
    ship_x: f32,
    ship_z: f32,
    ship_yaw: f32,
    effective_range: f32,
    facing_deg: f32,
    fire_arc_deg: f32,
) -> crate::core::messages::WeaponTargetGeometry {
    let (rx, ry) = ship_local(target_x, target_z, ship_x, ship_z, ship_yaw);
    let dx = target_x - ship_x;
    let dz = target_z - ship_z;
    let range = (dx * dx + dz * dz).sqrt();
    let in_range = dx * dx + dz * dz <= effective_range * effective_range;
    let bearing = simmath::atan2(rx, ry);
    let facing = facing_deg.to_radians();
    let arc_offset = super::arc_geometry::signed_difference(bearing, facing).abs();
    let in_arc = arc_offset <= fire_arc_deg.to_radians() * 0.5;
    crate::core::messages::WeaponTargetGeometry {
        range,
        arc_offset_deg: arc_offset.to_degrees(),
        in_range,
        in_arc,
    }
}

#[cfg(test)]
#[path = "mount_tests.rs"]
mod tests;
