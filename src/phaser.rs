/// Pure-Rust phaser bank mechanics.
///
/// This module is platform-agnostic and Bevy-free. It models two independent
/// phaser banks (port and starboard), each with:
///   - A 270° fire arc (the 90° blind cone is on the *opposite* side).
///   - A narrower 240° auto-fire arc (30° margin from the edge of the fire arc).
///   - A cooldown timer.
///   - Manual or Auto firing mode (shared across both banks).
///
/// Arc geometry
/// ─────────────
/// We reuse the ship-local coordinate system from `radar.rs`:
///   radar_x = dot((dx,dz), right)   > 0 means starboard
///   radar_y = dot((dx,dz), forward) > 0 means ahead
///
/// For the **port bank** the 90° blind cone is centred on pure starboard
/// (radar_x > 0, |angle from right| < 45°).  Equivalently, the target is in
/// the blind cone when `radar_x > |radar_y|` (using the L∞ approximation of a
/// 90° sector).
///
/// For the **starboard bank** the blind cone is on the port side: `radar_x < -|radar_y|`.
///
/// For the auto-fire arc (240°, blind cone = 120°):
///   Port bank blind: `radar_x > |radar_y| * tan(30°)`  (i.e. within 60° of right)
///   Starboard bank blind: `radar_x < -|radar_y| * tan(30°)`
///
/// `tan(45°) = 1.0`, `tan(30°) ≈ 0.5774`.

use std::f32::consts::PI;

/// Which phaser bank is being addressed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PhaserBankId {
    Port,
    Starboard,
}

/// Firing mode shared by both banks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum PhaserMode {
    /// Banks fire automatically when a target is in arc, in range, and off cooldown.
    #[default]
    Auto,
    /// Operator must press the fire button explicitly.
    Manual,
}

/// Configuration for a single phaser bank (and for the system as a whole).
#[derive(Clone, Debug)]
pub struct PhaserConfig {
    /// Seconds the bank must wait between shots.
    pub cooldown_secs: f32,
    /// Effective range (world units) used by auto-fire decisions.
    pub auto_fire_range: f32,
    /// Full fire-arc half-angle in degrees (used for manual fire validity).
    /// PRD: 270° total arc → 135° each side from the bank centre.
    pub fire_arc_deg: f32,
    /// Auto-fire arc in degrees (narrower; PRD: 240° → 120° each side).
    pub auto_arc_deg: f32,
}

impl Default for PhaserConfig {
    fn default() -> Self {
        Self {
            cooldown_secs: 3.0,
            auto_fire_range: 40.0,
            fire_arc_deg: 270.0,
            auto_arc_deg: 240.0,
        }
    }
}

/// State for a single phaser bank.
#[derive(Clone, Debug)]
pub struct PhaserBank {
    pub id: PhaserBankId,
    /// Remaining cooldown in seconds. 0.0 means ready to fire.
    pub cooldown_remaining: f32,
}

impl PhaserBank {
    pub fn new(id: PhaserBankId) -> Self {
        Self { id, cooldown_remaining: 0.0 }
    }

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
    pub fn fire(&mut self, config: &PhaserConfig) -> bool {
        if !self.is_ready() {
            return false;
        }
        self.cooldown_remaining = config.cooldown_secs;
        true
    }
}

/// The full phaser system: two banks sharing a mode setting.
#[derive(Clone, Debug)]
pub struct PhaserSystem {
    pub port: PhaserBank,
    pub starboard: PhaserBank,
    pub mode: PhaserMode,
    pub config: PhaserConfig,
}

impl PhaserSystem {
    pub fn new(config: PhaserConfig) -> Self {
        Self {
            port: PhaserBank::new(PhaserBankId::Port),
            starboard: PhaserBank::new(PhaserBankId::Starboard),
            mode: PhaserMode::default(),
            config,
        }
    }

    /// Advance both banks' cooldown timers.
    pub fn tick(&mut self, dt: f32) {
        self.port.tick(dt);
        self.starboard.tick(dt);
    }

    /// Set the firing mode.
    pub fn set_mode(&mut self, mode: PhaserMode) {
        self.mode = mode;
    }

    /// Attempt to fire a specific bank manually.  Returns `true` if the shot
    /// was accepted.  Does NOT check target arc/range — that is the caller's
    /// responsibility for manual fire.
    pub fn fire_manual(&mut self, bank_id: PhaserBankId) -> bool {
        match bank_id {
            PhaserBankId::Port => self.port.fire(&self.config),
            PhaserBankId::Starboard => self.starboard.fire(&self.config),
        }
    }

    /// Check whether a target at the given world position is within a bank's
    /// **full** fire arc (270°) and within `auto_fire_range`.
    ///
    /// Returns `true` if fire is valid.
    pub fn is_in_fire_arc(
        &self,
        bank_id: PhaserBankId,
        target_x: f32,
        target_z: f32,
        ship_x: f32,
        ship_z: f32,
        ship_yaw: f32,
    ) -> bool {
        let (radar_x, radar_y) = ship_local(target_x, target_z, ship_x, ship_z, ship_yaw);
        let range_sq = (target_x - ship_x).powi(2) + (target_z - ship_z).powi(2);
        if range_sq > self.config.auto_fire_range.powi(2) {
            return false;
        }
        in_arc(radar_x, radar_y, bank_id, self.config.fire_arc_deg)
    }

    /// Check whether a target is within the **auto-fire** arc (240°) AND range.
    pub fn is_in_auto_arc(
        &self,
        bank_id: PhaserBankId,
        target_x: f32,
        target_z: f32,
        ship_x: f32,
        ship_z: f32,
        ship_yaw: f32,
    ) -> bool {
        let (radar_x, radar_y) = ship_local(target_x, target_z, ship_x, ship_z, ship_yaw);
        let range_sq = (target_x - ship_x).powi(2) + (target_z - ship_z).powi(2);
        if range_sq > self.config.auto_fire_range.powi(2) {
            return false;
        }
        in_arc(radar_x, radar_y, bank_id, self.config.auto_arc_deg)
    }

    /// Auto-fire: attempt to fire each bank if mode is `Auto`, target is in the
    /// auto arc, and the bank is off cooldown.
    ///
    /// Returns `(port_fired, starboard_fired)`.
    pub fn auto_fire(
        &mut self,
        target_x: f32,
        target_z: f32,
        ship_x: f32,
        ship_z: f32,
        ship_yaw: f32,
    ) -> (bool, bool) {
        if self.mode != PhaserMode::Auto {
            return (false, false);
        }
        let port_ok = self.is_in_auto_arc(PhaserBankId::Port, target_x, target_z, ship_x, ship_z, ship_yaw);
        let star_ok = self.is_in_auto_arc(PhaserBankId::Starboard, target_x, target_z, ship_x, ship_z, ship_yaw);
        let config = self.config.clone();
        let port_fired = port_ok && self.port.fire(&config);
        let star_fired = star_ok && self.starboard.fire(&config);
        (port_fired, star_fired)
    }
}

// ── private helpers ────────────────────────────────────────────────────────

/// Convert a world-space target to ship-local (radar) coordinates.
fn ship_local(target_x: f32, target_z: f32, ship_x: f32, ship_z: f32, ship_yaw: f32) -> (f32, f32) {
    let dx = target_x - ship_x;
    let dz = target_z - ship_z;
    let cos_y = ship_yaw.cos();
    let sin_y = ship_yaw.sin();
    let radar_x = dx * cos_y + dz * sin_y;
    let radar_y = dx * sin_y - dz * cos_y;
    (radar_x, radar_y)
}

/// Returns `true` if `(radar_x, radar_y)` is inside the given total arc
/// (centred on the appropriate bank side).
///
/// The arc is centred on:
///   - Port bank  → left side (radar_x < 0), centred at angle 270° from forward
///                  (i.e. directly to port).  The *blind* cone is on the
///                  starboard side.
///   - Starboard bank → centred at 90° from forward (directly to starboard).
///                      Blind cone is on port side.
///
/// We compute the half-angle of the *blind cone*:
///   blind_half = (360° - arc_deg) / 2  →  for 270° → 45°, for 240° → 60°.
///
/// A point is in the blind cone when its angle from the bank's opposite
/// direction (pure starboard for port bank, pure port for starboard bank)
/// is less than `blind_half`.
///
/// Using radar coordinates:
///   For port bank, "opposite direction" = pure starboard = (+1, 0).
///     angle from (+1,0) < blind_half  ⟺  radar_x > cos(blind_half) * r
///     where r = sqrt(radar_x² + radar_y²)
///     ⟺  radar_x / r > cos(blind_half)
///     ⟺  radar_x > 0 AND radar_x² / (radar_x²+radar_y²) > cos²(blind_half)
///
/// Simplified: the point is in the blind cone iff:
///   atan2(|radar_y|, radar_x) < blind_half   (for port: must check x > 0)
fn in_arc(radar_x: f32, radar_y: f32, bank: PhaserBankId, arc_deg: f32) -> bool {
    let blind_half_deg = (360.0 - arc_deg) / 2.0;
    let blind_half_rad = blind_half_deg * PI / 180.0;
    let tan_blind = blind_half_rad.tan();

    match bank {
        PhaserBankId::Port => {
            // Blind cone centred on pure starboard (+x direction).
            // Point is in blind cone: x > 0 AND |y|/x < tan(blind_half)
            // i.e. x > 0 AND |y| < x * tan_blind
            !(radar_x > 0.0 && radar_y.abs() < radar_x * tan_blind)
        }
        PhaserBankId::Starboard => {
            // Blind cone centred on pure port (-x direction).
            // Point is in blind cone: x < 0 AND |y|/(-x) < tan(blind_half)
            // i.e. x < 0 AND |y| < (-x) * tan_blind
            !(radar_x < 0.0 && radar_y.abs() < (-radar_x) * tan_blind)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_system() -> PhaserSystem {
        PhaserSystem::new(PhaserConfig::default())
    }

    // ── Cooldown ──────────────────────────────────────────────────────────

    #[test]
    fn bank_is_ready_initially() {
        let sys = default_system();
        assert!(sys.port.is_ready());
        assert!(sys.starboard.is_ready());
    }

    #[test]
    fn firing_sets_cooldown() {
        let mut sys = default_system();
        sys.fire_manual(PhaserBankId::Port);
        assert!(!sys.port.is_ready());
    }

    #[test]
    fn tick_reduces_cooldown() {
        let mut sys = default_system();
        sys.fire_manual(PhaserBankId::Port);
        sys.tick(1.0);
        // Still on cooldown (default 3s)
        assert!(!sys.port.is_ready());
    }

    #[test]
    fn tick_to_zero_makes_bank_ready() {
        let mut sys = default_system();
        sys.fire_manual(PhaserBankId::Port);
        sys.tick(sys.config.cooldown_secs);
        assert!(sys.port.is_ready());
    }

    #[test]
    fn tick_does_not_go_negative() {
        let mut sys = default_system();
        sys.fire_manual(PhaserBankId::Port);
        sys.tick(100.0);
        assert_eq!(sys.port.cooldown_remaining, 0.0);
    }

    #[test]
    fn fire_while_on_cooldown_returns_false() {
        let mut sys = default_system();
        sys.fire_manual(PhaserBankId::Port);
        let result = sys.fire_manual(PhaserBankId::Port);
        assert!(!result);
    }

    #[test]
    fn banks_are_independent() {
        let mut sys = default_system();
        sys.fire_manual(PhaserBankId::Port);
        assert!(!sys.port.is_ready());
        assert!(sys.starboard.is_ready());
    }

    // ── Fire arc (270°) ───────────────────────────────────────────────────

    /// Ship at origin, yaw=0 (facing -Z). Target directly ahead is in arc for
    /// both banks.
    #[test]
    fn target_ahead_in_arc_for_both_banks() {
        let sys = default_system();
        assert!(sys.is_in_fire_arc(PhaserBankId::Port, 0.0, -20.0, 0.0, 0.0, 0.0));
        assert!(sys.is_in_fire_arc(PhaserBankId::Starboard, 0.0, -20.0, 0.0, 0.0, 0.0));
    }

    /// Target directly to port (radar_x = -20): in arc for port bank, in arc
    /// for starboard bank (starboard blind is the *port* side pure direction but
    /// only 90° cone, so port direction is still covered by starboard 270°).
    #[test]
    fn target_directly_to_port_in_fire_arc_for_port_bank() {
        let sys = default_system();
        // yaw=0, target at (-20, 0): directly to port
        assert!(sys.is_in_fire_arc(PhaserBankId::Port, -20.0, 0.0, 0.0, 0.0, 0.0));
    }

    /// Target directly to starboard: in arc for starboard bank.
    #[test]
    fn target_directly_to_starboard_in_fire_arc_for_starboard_bank() {
        let sys = default_system();
        // yaw=0, target at (+20, 0): directly to starboard
        assert!(sys.is_in_fire_arc(PhaserBankId::Starboard, 20.0, 0.0, 0.0, 0.0, 0.0));
    }

    /// Target directly to starboard (pure right, radar_x > 0, radar_y = 0):
    /// the port bank's blind cone covers exactly this direction → NOT in arc.
    #[test]
    fn target_directly_to_starboard_not_in_port_fire_arc() {
        let sys = default_system();
        // (20, 0) is pure starboard. For port bank with 270° arc, blind cone is
        // 45° each side of starboard → radar_y = 0 is exactly on boundary → blind.
        // (We treat boundary as excluded from firing, same as is_fire_ready.)
        assert!(!sys.is_in_fire_arc(PhaserBankId::Port, 20.0, 0.0, 0.0, 0.0, 0.0));
    }

    /// Target directly to port: NOT in fire arc for starboard bank.
    #[test]
    fn target_directly_to_port_not_in_starboard_fire_arc() {
        let sys = default_system();
        assert!(!sys.is_in_fire_arc(PhaserBankId::Starboard, -20.0, 0.0, 0.0, 0.0, 0.0));
    }

    /// Target behind (aft) is in arc for both banks (270° arcs cover aft).
    #[test]
    fn target_aft_in_fire_arc_for_both_banks() {
        let sys = default_system();
        // yaw=0, target at (0, +20): directly aft
        assert!(sys.is_in_fire_arc(PhaserBankId::Port, 0.0, 20.0, 0.0, 0.0, 0.0));
        assert!(sys.is_in_fire_arc(PhaserBankId::Starboard, 0.0, 20.0, 0.0, 0.0, 0.0));
    }

    /// Target out of range returns false even if in arc.
    #[test]
    fn target_out_of_range_not_in_fire_arc() {
        let sys = default_system();
        assert!(!sys.is_in_fire_arc(PhaserBankId::Port, 0.0, -100.0, 0.0, 0.0, 0.0));
    }

    // ── Auto-fire arc (240°) ──────────────────────────────────────────────

    /// Target slightly starboard-of-forward: in auto arc for both banks
    /// (well within 240°).
    #[test]
    fn target_slightly_starboard_of_forward_in_auto_arc_for_both() {
        let sys = default_system();
        // (5, -20): mostly ahead, slightly starboard
        assert!(sys.is_in_auto_arc(PhaserBankId::Port, 5.0, -20.0, 0.0, 0.0, 0.0));
        assert!(sys.is_in_auto_arc(PhaserBankId::Starboard, 5.0, -20.0, 0.0, 0.0, 0.0));
    }

    /// Target directly to starboard: NOT in auto arc for port bank (blind cone
    /// is 60° each side of starboard for 240° arc).
    #[test]
    fn target_directly_to_starboard_not_in_port_auto_arc() {
        let sys = default_system();
        assert!(!sys.is_in_auto_arc(PhaserBankId::Port, 20.0, 0.0, 0.0, 0.0, 0.0));
    }

    // ── Mode and auto-fire ────────────────────────────────────────────────

    #[test]
    fn default_mode_is_auto() {
        let sys = default_system();
        assert_eq!(sys.mode, PhaserMode::Auto);
    }

    #[test]
    fn set_mode_changes_mode() {
        let mut sys = default_system();
        sys.set_mode(PhaserMode::Manual);
        assert_eq!(sys.mode, PhaserMode::Manual);
    }

    /// In auto mode, a target in arc fires both ready banks.
    #[test]
    fn auto_fire_fires_banks_in_arc() {
        let mut sys = default_system();
        // Target directly ahead — both banks should fire.
        let (p, s) = sys.auto_fire(0.0, -20.0, 0.0, 0.0, 0.0);
        assert!(p, "port should have fired");
        assert!(s, "starboard should have fired");
    }

    /// In manual mode, auto_fire does nothing.
    #[test]
    fn auto_fire_does_nothing_in_manual_mode() {
        let mut sys = default_system();
        sys.set_mode(PhaserMode::Manual);
        let (p, s) = sys.auto_fire(0.0, -20.0, 0.0, 0.0, 0.0);
        assert!(!p);
        assert!(!s);
    }

    /// Auto-fire respects bank cooldown: a bank on cooldown does not fire.
    #[test]
    fn auto_fire_skips_bank_on_cooldown() {
        let mut sys = default_system();
        // Pre-fire port so it's on cooldown.
        sys.port.fire(&sys.config.clone());
        let (p, s) = sys.auto_fire(0.0, -20.0, 0.0, 0.0, 0.0);
        assert!(!p, "port on cooldown should not fire");
        assert!(s, "starboard should fire");
    }

    /// Auto-fire does not fire when target is out of range.
    #[test]
    fn auto_fire_out_of_range_does_not_fire() {
        let mut sys = default_system();
        let (p, s) = sys.auto_fire(0.0, -100.0, 0.0, 0.0, 0.0);
        assert!(!p);
        assert!(!s);
    }
}
