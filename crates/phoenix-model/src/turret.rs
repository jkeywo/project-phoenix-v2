//! Finite physical traversal shared by components of one actual System.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TurretConfig {
    /// Ship-relative home bearing, clockwise from forward.
    pub facing_deg: f32,
    pub traverse_arc_deg: f32,
    pub traverse_deg_per_sec: f32,
    pub aim_tolerance_deg: f32,
}

impl TurretConfig {
    pub fn validate(&self) -> Result<(), String> {
        if !self.facing_deg.is_finite()
            || !(-180.0..=180.0).contains(&self.facing_deg)
            || !self.traverse_arc_deg.is_finite()
            || !(0.0 < self.traverse_arc_deg && self.traverse_arc_deg <= 360.0)
            || !self.traverse_deg_per_sec.is_finite()
            || self.traverse_deg_per_sec <= 0.0
            || !self.aim_tolerance_deg.is_finite()
            || !(0.0..=self.traverse_arc_deg * 0.5).contains(&self.aim_tolerance_deg)
        {
            return Err("turret requires facing in [-180,180], traverse arc in (0,360], positive finite speed and aim tolerance in [0,half arc]".into());
        }
        Ok(())
    }
}
