//! Pure-Rust phaser bank mechanics.
//!
//! This module is platform-agnostic and Bevy-free. The phaser system holds a
//! `Vec<PhaserBank>` whose contents come from
//! a ship entity TOML (e.g. `assets/entities/alliance_battleship.toml`, the `[[weapons_console.phaser_banks]]`
//! array). Each bank carries its own facing, fire arc, auto-fire arc, and
//! optional per-bank beam range.
//!
//! Arc geometry
//! ─────────────
//! We use the ship-local coordinate system from `radar.rs`:
//!   radar_x = dx * cos(yaw) + dz * sin(yaw)   > 0 means starboard
//!   radar_y = dx * sin(yaw) - dz * cos(yaw)   > 0 means ahead
//!
//! Bearing-from-forward is `atan2(radar_x, radar_y)`:
//!   0       → directly forward
//!   +π/2    → directly starboard
//!   ±π      → directly aft
//!   −π/2    → directly port
//!
//! A target is "in arc" for a bank with `facing_deg` and `arc_deg` when
//! `angle_diff(bearing, facing_rad).abs() <= arc_deg.to_radians() / 2`.

use crate::simmath;
use std::f32::consts::PI;

/// String identifier for a phaser bank (matches the `id` field in TOML).
pub type PhaserBankId = String;

/// Firing mode shared by all banks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum PhaserMode {
    /// Banks fire automatically when a target is in the auto arc, in range,
    /// and off cooldown.
    #[default]
    Auto,
    /// Operator must press the fire button explicitly.
    Manual,
}

/// State for a single phaser bank.
#[derive(Clone, Debug)]
pub struct PhaserBank {
    /// Bank identifier from TOML (e.g. `"port"`, `"starboard"`).
    pub id: PhaserBankId,
    /// Centre of the bank's fire arc, degrees clockwise from ship-forward.
    /// 0 = forward, +90 = starboard, ±180 = aft, −90 = port.
    pub facing_deg: f32,
    /// Total fire-arc width in degrees (manual-fire arc).
    pub fire_arc_deg: f32,
    /// Total auto-fire arc width in degrees (must be ≤ `fire_arc_deg`).
    pub auto_arc_deg: f32,
    /// Effective beam range for this bank in world units.
    pub beam_range: f32,
    /// Remaining cooldown in seconds. 0.0 means ready to fire.
    pub cooldown_remaining: f32,
}

impl PhaserBank {
    /// Returns `true` if the bank is not on cooldown.
    pub fn is_ready(&self) -> bool {
        self.cooldown_remaining <= 0.0
    }

    /// Advance the cooldown timer by `dt` seconds.
    pub fn tick(&mut self, dt: f32) {
        self.cooldown_remaining = (self.cooldown_remaining - dt).max(0.0);
    }

    /// Trigger a shot: resets the cooldown.  Returns `true` if the shot was
    /// accepted (bank was ready), `false` if still on cooldown.
    pub fn fire(&mut self, cooldown_secs: f32) -> bool {
        if !self.is_ready() {
            return false;
        }
        self.cooldown_remaining = cooldown_secs;
        true
    }
}

/// The full phaser system: a list of banks sharing a single mode + cooldown.
#[derive(Clone, Debug)]
pub struct PhaserSystem {
    pub banks: Vec<PhaserBank>,
    pub mode: PhaserMode,
    /// Cooldown after firing (same for every bank — sourced from
    /// `[weapons_console] cooldown_secs`).
    pub cooldown_secs: f32,
}

impl PhaserSystem {
    /// Build a phaser system from the parsed TOML bank configs.
    ///
    /// `fallback_beam_range` is used for any bank whose own `beam_range` is
    /// `0.0` (the TOML default), matching `[weapons_console] beam_range`.
    pub fn from_configs(
        banks: &[crate::entities::config::PhaserBankConfig],
        cooldown_secs: f32,
        fallback_beam_range: f32,
    ) -> Self {
        let banks = banks
            .iter()
            .map(|c| PhaserBank {
                id: c.id.clone(),
                facing_deg: c.facing_deg,
                fire_arc_deg: c.fire_arc_deg,
                auto_arc_deg: c.auto_arc_deg,
                beam_range: if c.beam_range > 0.0 {
                    c.beam_range
                } else {
                    fallback_beam_range
                },
                cooldown_remaining: 0.0,
            })
            .collect();
        Self {
            banks,
            mode: PhaserMode::default(),
            cooldown_secs,
        }
    }

    /// Lookup a bank by id.
    pub fn bank(&self, id: &str) -> Option<&PhaserBank> {
        self.banks.iter().find(|b| b.id == id)
    }

    /// Mutable lookup by id.
    pub fn bank_mut(&mut self, id: &str) -> Option<&mut PhaserBank> {
        self.banks.iter_mut().find(|b| b.id == id)
    }

    /// Advance every bank's cooldown timer.
    pub fn tick(&mut self, dt: f32) {
        for b in &mut self.banks {
            b.tick(dt);
        }
    }

    /// Set the firing mode.
    pub fn set_mode(&mut self, mode: PhaserMode) {
        self.mode = mode;
    }

    /// Attempt to fire a specific bank manually. Returns `true` if the shot
    /// was accepted. Does NOT check arc/range — that is the caller's job.
    pub fn fire_manual(&mut self, id: &str) -> bool {
        let cooldown = self.cooldown_secs;
        self.bank_mut(id).map(|b| b.fire(cooldown)).unwrap_or(false)
    }

    /// Check whether a target is within a bank's **full** fire arc and the
    /// bank's `beam_range`.
    pub fn is_in_fire_arc(
        &self,
        id: &str,
        target_x: f32,
        target_z: f32,
        ship_x: f32,
        ship_z: f32,
        ship_yaw: f32,
    ) -> bool {
        let Some(bank) = self.bank(id) else {
            return false;
        };
        let range_sq = (target_x - ship_x).powi(2) + (target_z - ship_z).powi(2);
        if range_sq > bank.beam_range.powi(2) {
            return false;
        }
        let (rx, ry) = ship_local(target_x, target_z, ship_x, ship_z, ship_yaw);
        in_arc(rx, ry, bank.facing_deg, bank.fire_arc_deg)
    }

    /// Check whether a target is within a bank's **auto-fire** arc and range.
    pub fn is_in_auto_arc(
        &self,
        id: &str,
        target_x: f32,
        target_z: f32,
        ship_x: f32,
        ship_z: f32,
        ship_yaw: f32,
    ) -> bool {
        let Some(bank) = self.bank(id) else {
            return false;
        };
        let range_sq = (target_x - ship_x).powi(2) + (target_z - ship_z).powi(2);
        if range_sq > bank.beam_range.powi(2) {
            return false;
        }
        let (rx, ry) = ship_local(target_x, target_z, ship_x, ship_z, ship_yaw);
        in_arc(rx, ry, bank.facing_deg, bank.auto_arc_deg)
    }

    /// Auto-fire: try each bank in order if mode is `Auto`, target is in the
    /// auto arc, and the bank is off cooldown. Returns the ids of banks that
    /// fired this tick.
    pub fn auto_fire(
        &mut self,
        target_x: f32,
        target_z: f32,
        ship_x: f32,
        ship_z: f32,
        ship_yaw: f32,
    ) -> Vec<PhaserBankId> {
        if self.mode != PhaserMode::Auto {
            return Vec::new();
        }
        let cooldown = self.cooldown_secs;
        let mut fired = Vec::new();
        // Collect ids whose auto-arc contains the target, then attempt fire.
        let candidate_ids: Vec<(String, bool)> = self
            .banks
            .iter()
            .map(|b| {
                let in_range_sq = (target_x - ship_x).powi(2) + (target_z - ship_z).powi(2)
                    <= b.beam_range.powi(2);
                let (rx, ry) = ship_local(target_x, target_z, ship_x, ship_z, ship_yaw);
                let in_a = in_range_sq && in_arc(rx, ry, b.facing_deg, b.auto_arc_deg);
                (b.id.clone(), in_a)
            })
            .collect();
        for (id, ok) in candidate_ids {
            if ok {
                if let Some(b) = self.bank_mut(&id) {
                    if b.fire(cooldown) {
                        fired.push(id);
                    }
                }
            }
        }
        fired
    }
}

// ── helpers ────────────────────────────────────────────────────────────────

/// Convert a world-space target to ship-local (radar) coordinates.
pub(crate) fn ship_local(
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

/// True if `(radar_x, radar_y)` is within `arc_deg/2` of the bank's facing.
pub fn in_arc(radar_x: f32, radar_y: f32, facing_deg: f32, arc_deg: f32) -> bool {
    let bearing = simmath::atan2(radar_x, radar_y);
    let facing = facing_deg.to_radians();
    let half = arc_deg.to_radians() * 0.5;
    angle_diff(bearing, facing).abs() <= half
}

/// Compute the shared [`crate::core::messages::WeaponTargetGeometry`] for one weapon
/// instance from a world-space target, the shooter's physics, and the weapon's
/// effective range + fire arc (issue #764).
///
/// Pure and Bevy-free: reuses `ship_local` + the same sq-dist range test and
/// `in_arc` bearing math the phaser/blaster fire paths already use, so the
/// readiness projection can never diverge from the authoritative fire gate.
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
    let arc_offset = angle_diff(bearing, facing).abs();
    let in_arc = arc_offset <= fire_arc_deg.to_radians() * 0.5;
    crate::core::messages::WeaponTargetGeometry {
        range,
        arc_offset_deg: arc_offset.to_degrees(),
        in_range,
        in_arc,
    }
}

/// Signed angular difference `a − b` wrapped to `[−π, π]`.
fn angle_diff(a: f32, b: f32) -> f32 {
    let mut d = a - b;
    while d > PI {
        d -= 2.0 * PI;
    }
    while d < -PI {
        d += 2.0 * PI;
    }
    d
}

#[cfg(test)]
#[path = "phaser_tests.rs"]
mod tests;
