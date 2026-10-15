//! One paid physical bearing per System, with no target/AI/inventory copy.
pub use phoenix_model::turret::TurretConfig;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TurretState {
    /// Offset from the authored home bearing. Limited sectors never wrap
    /// through their forbidden back; full-circle mounts use [-180,180).
    pub offset_deg: f64,
    pub last_tick: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TurretStep {
    pub moved_deg: f64,
    pub at_aim: bool,
}

fn wrap(degrees: f64) -> f64 {
    (degrees + 180.0).rem_euclid(360.0) - 180.0
}

impl TurretState {
    pub fn validate(&self, config: &TurretConfig, tick: u64) -> Result<(), String> {
        config.validate()?;
        let half = f64::from(config.traverse_arc_deg) * 0.5;
        if !self.offset_deg.is_finite()
            || self.offset_deg.abs() > half
            || config.traverse_arc_deg == 360.0 && self.offset_deg >= 180.0
            || self.last_tick.is_some_and(|last| last > tick)
            || self.last_tick.is_none() && self.offset_deg != 0.0
        {
            return Err("invalid turret bearing or continuation frontier".into());
        }
        Ok(())
    }

    pub fn bearing_deg(&self, config: &TurretConfig) -> f32 {
        wrap(f64::from(config.facing_deg) + self.offset_deg) as f32
    }

    pub fn ready(&self, config: &TurretConfig, desired_deg: f32) -> bool {
        if !desired_deg.is_finite() {
            return false;
        }
        let desired = wrap(f64::from(desired_deg) - f64::from(config.facing_deg));
        let reachable = config.traverse_arc_deg == 360.0
            || desired.abs() <= f64::from(config.traverse_arc_deg) * 0.5;
        reachable && wrap(desired - self.offset_deg).abs() <= f64::from(config.aim_tolerance_deg)
    }

    pub fn movement_deg(
        &self,
        config: &TurretConfig,
        desired_deg: Option<f32>,
    ) -> Result<f64, String> {
        if desired_deg.is_some_and(|bearing| !bearing.is_finite()) {
            return Err("turret aim must be finite".into());
        }
        let desired = desired_deg.map_or(0.0, |bearing| {
            wrap(f64::from(bearing) - f64::from(config.facing_deg))
        });
        Ok(if config.traverse_arc_deg == 360.0 {
            wrap(desired - self.offset_deg)
        } else {
            desired.clamp(
                -f64::from(config.traverse_arc_deg) * 0.5,
                f64::from(config.traverse_arc_deg) * 0.5,
            ) - self.offset_deg
        })
    }

    /// The caller stages actual local energy with this state. Only funded
    /// elapsed time moves the mount; there is no accrued movement backlog.
    pub fn advance(
        &mut self,
        config: &TurretConfig,
        tick: u64,
        dt: f32,
        enabled: bool,
        desired_deg: Option<f32>,
        funded_fraction: f32,
    ) -> Result<TurretStep, String> {
        self.validate(config, tick)?;
        if !dt.is_finite()
            || dt < 0.0
            || !funded_fraction.is_finite()
            || !(0.0..=1.0).contains(&funded_fraction)
        {
            return Err(
                "turret step requires finite nonnegative duration and funded fraction in [0,1]"
                    .into(),
            );
        }
        let distance = self.movement_deg(config, desired_deg)?;
        let at_aim = |state: &Self| desired_deg.is_some_and(|bearing| state.ready(config, bearing));
        if self.last_tick == Some(tick) {
            return Ok(TurretStep {
                moved_deg: 0.0,
                at_aim: at_aim(self),
            });
        }
        let allowed = if enabled {
            f64::from(dt) * f64::from(funded_fraction) * f64::from(config.traverse_deg_per_sec)
        } else {
            0.0
        };
        let moved = distance.abs().min(allowed).copysign(distance);
        self.offset_deg += moved;
        if config.traverse_arc_deg == 360.0 {
            self.offset_deg = wrap(self.offset_deg);
        }
        self.last_tick = Some(tick);
        Ok(TurretStep {
            moved_deg: moved.abs(),
            at_aim: at_aim(self),
        })
    }

    /// Existing static geometry still supplies range and firing-sector limits;
    /// finite traversal supplies the additional actual-aim readiness gate.
    pub fn geometry(
        &self,
        config: &TurretConfig,
        mount: &super::mount::MountConfig,
        target: [f32; 2],
        own: [f32; 2],
        yaw: f32,
    ) -> crate::core::messages::WeaponTargetGeometry {
        let mut result = mount.geometry(target[0], target[1], own[0], own[1], yaw);
        let (x, y) = super::mount::ship_local(target[0], target[1], own[0], own[1], yaw);
        let desired = crate::simmath::atan2(x, y).to_degrees();
        result.in_arc &= self.ready(config, desired);
        result.arc_offset_deg =
            wrap(f64::from(desired) - f64::from(self.bearing_deg(config))).abs() as f32;
        result
    }
}

#[cfg(test)]
#[path = "turret_tests.rs"]
mod tests;
