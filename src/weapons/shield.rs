/// Shield system for the ship. Consists of one or more `ShieldFacing` arcs.
///
/// By default ships have four facings (Fore / Port / Aft / Starboard), each
/// spanning 90°. The number of arcs is configurable: fewer arcs means wider
/// but fewer facings.
///
/// ## Hit detection
/// The facing that absorbs a hit is determined by the *attacker bearing*
/// expressed as an angle **relative to the ship's own yaw** in the range
/// `(-π, π]`, measured anti-clockwise from the ship's forward (+Z) axis.
/// Each facing covers an equal arc of the full circle starting from the
/// forward direction (angle 0).
///
/// ## Offline mechanic
/// When a facing's HP reaches 0 it goes offline for `offline_duration`
/// seconds. While offline it absorbs no damage; any hit to that facing
/// passes straight through to the hull. The facing recharges back to
/// `max_hp` once the timer expires.
///
/// ## Regen
/// Each facing regenerates `regen_per_sec` HP per second while online.
/// Regen is capped at `max_hp`.
use std::f32::consts::TAU;

/// A snapshot of a single shield facing, suitable for serialisation and UI.
#[derive(Clone, Debug, PartialEq)]
pub struct ShieldFacingSnapshot {
    /// Human-readable label (e.g. "Fore", "Starboard").
    pub label: String,
    pub hp: i32,
    pub max_hp: i32,
    pub online: bool,
    /// Remaining offline seconds (0.0 when online).
    pub offline_remaining: f32,
    /// Whether this facing is the currently focused arc.
    pub is_focused: bool,
}

/// Configuration for the entire shield system.
#[derive(Clone, Debug)]
pub struct ShieldConfig {
    /// Number of equally-spaced facings. Must be ≥ 1.
    pub num_facings: usize,
    pub max_hp: i32,
    pub regen_per_sec: f32,
    /// How long (seconds) a facing stays offline after its HP is depleted.
    pub offline_duration: f32,
}

impl Default for ShieldConfig {
    fn default() -> Self {
        Self {
            num_facings: 4,
            max_hp: 100,
            regen_per_sec: 2.0,
            offline_duration: 10.0,
        }
    }
}

/// Configuration for shield focus bonuses and penalties.
///
/// When the Shields console operator focuses one facing:
/// - That facing gets `bonus_max_hp` extra capacity and `bonus_regen` regen.
/// - The other three facings lose `penalty_max_hp` capacity and `penalty_regen` regen.
/// - Non-focused facings decay HP at `decay_rate` per second when above their
///   reduced maximum.
#[derive(Clone, Debug)]
pub struct ShieldFocusConfig {
    /// Extra max HP applied to the focused facing.
    pub bonus_max_hp: i32,
    /// Extra regen per second applied to the focused facing.
    pub bonus_regen: f32,
    /// Max HP subtracted from each non-focused facing.
    pub penalty_max_hp: i32,
    /// Regen per second subtracted from each non-focused facing.
    pub penalty_regen: f32,
    /// HP per second decay applied to non-focused facings when above reduced max.
    pub decay_rate: f32,
}

impl Default for ShieldFocusConfig {
    fn default() -> Self {
        Self {
            bonus_max_hp: 50,
            bonus_regen: 5.0,
            penalty_max_hp: 25,
            penalty_regen: 1.0,
            decay_rate: 10.0,
        }
    }
}

/// A single shield facing arc.
#[derive(Clone, Debug)]
pub struct ShieldFacing {
    pub label: String,
    pub hp: i32,
    pub max_hp: i32,
    pub regen_per_sec: f32,
    pub offline_duration: f32,
    /// Remaining seconds of offline time. 0.0 means the facing is online.
    pub offline_remaining: f32,
    /// Whether this facing is the currently focused arc.
    pub is_focused: bool,
    /// Sub-integer regen accumulator. Carries fractional HP across frames so
    /// that regen rates below 1 HP/frame are applied correctly.
    hp_frac: f32,
}

impl ShieldFacing {
    fn new(
        label: impl Into<String>,
        max_hp: i32,
        regen_per_sec: f32,
        offline_duration: f32,
    ) -> Self {
        Self {
            label: label.into(),
            hp: max_hp,
            max_hp,
            regen_per_sec,
            offline_duration,
            offline_remaining: 0.0,
            is_focused: false,
            hp_frac: 0.0,
        }
    }

    /// Whether this facing is currently active (not offline).
    pub fn is_online(&self) -> bool {
        self.offline_remaining <= 0.0
    }

    /// Apply `amount` damage to this facing.
    ///
    /// Returns the damage that passed through to the hull:
    /// - If the facing is offline, all damage passes through.
    /// - If the facing absorbs the hit and HP drops to 0, the facing goes offline
    ///   and any overflow damage passes through to the hull.
    pub fn apply_damage(&mut self, amount: i32) -> i32 {
        if !self.is_online() {
            // Facing is down — all damage passes to hull.
            return amount;
        }
        if amount <= self.hp {
            self.hp -= amount;
            if self.hp == 0 {
                self.offline_remaining = self.offline_duration;
            }
            0 // no hull passthrough
        } else {
            // Overflow: shield absorbs what it has, rest bleeds through.
            let overflow = amount - self.hp;
            self.hp = 0;
            self.offline_remaining = self.offline_duration;
            overflow
        }
    }

    /// Advance the facing by `dt` seconds: tick offline timer and regen.
    pub fn tick(&mut self, dt: f32) {
        if !self.is_online() {
            self.offline_remaining = (self.offline_remaining - dt).max(0.0);
            if self.is_online() {
                // Just came back online — restore to full HP and clear accumulator.
                self.hp = self.max_hp;
                self.hp_frac = 0.0;
            }
        } else {
            // Regen while online, accumulating fractional HP across frames.
            self.hp_frac += self.regen_per_sec * dt;
            let whole = self.hp_frac as i32;
            if whole > 0 {
                self.hp = (self.hp + whole).min(self.max_hp);
                self.hp_frac -= whole as f32;
            }
            // Keep hp_frac from drifting if already at max.
            if self.hp >= self.max_hp {
                self.hp_frac = 0.0;
            }
        }
    }

    pub fn snapshot(&self) -> ShieldFacingSnapshot {
        ShieldFacingSnapshot {
            label: self.label.clone(),
            hp: self.hp,
            max_hp: self.max_hp,
            online: self.is_online(),
            offline_remaining: self.offline_remaining,
            is_focused: self.is_focused,
        }
    }
}

/// Default facing labels for 1, 2, 3, or 4 arcs.
///
/// Facings are indexed going **counter-clockwise** (anti-clockwise) from forward:
/// Fore(0) → Port(1) → Aft(2) → Starboard(3)
fn default_label(index: usize, num_facings: usize) -> String {
    match num_facings {
        1 => "All".to_string(),
        2 => match index {
            0 => "Fore".to_string(),
            _ => "Aft".to_string(),
        },
        3 => match index {
            0 => "Fore".to_string(),
            1 => "Port".to_string(),
            _ => "Starboard".to_string(),
        },
        4 => match index {
            0 => "Fore".to_string(),
            1 => "Port".to_string(),
            2 => "Aft".to_string(),
            _ => "Starboard".to_string(),
        },
        _ => format!("Arc {}", index),
    }
}

/// The complete shield system, owning all facings.
pub struct ShieldSystem {
    pub facings: Vec<ShieldFacing>,
    /// Which facing (index) is currently focused by the Shields console.
    /// `None` means no focus (all facings at base values).
    pub focused_facing: Option<usize>,
    /// Configuration for focus bonus/penalty/decay.
    pub focus_config: ShieldFocusConfig,
    /// Base max HP per facing before focus modifiers.
    pub base_max_hp: i32,
    /// Base regen per second before focus modifiers.
    pub base_regen_per_sec: f32,
}

impl ShieldSystem {
    /// Create a new shield system from the given config.
    pub fn new(config: &ShieldConfig) -> Self {
        assert!(config.num_facings >= 1, "num_facings must be >= 1");
        let facings = (0..config.num_facings)
            .map(|i| {
                ShieldFacing::new(
                    default_label(i, config.num_facings),
                    config.max_hp,
                    config.regen_per_sec,
                    config.offline_duration,
                )
            })
            .collect();
        Self {
            facings,
            focused_facing: None,
            focus_config: ShieldFocusConfig::default(),
            base_max_hp: config.max_hp,
            base_regen_per_sec: config.regen_per_sec,
        }
    }

    /// Set the focused facing by index, or `None` to clear focus.
    /// Recalculates each facing's effective max_hp, regen, and is_focused flag.
    pub fn set_focused_facing(&mut self, facing: Option<usize>) {
        self.focused_facing = facing;
        self.recalculate_focus();
    }

    /// Recalculate effective max_hp, regen_per_sec, and is_focused for all facings
    /// based on the current `focused_facing`.
    fn recalculate_focus(&mut self) {
        let fc = &self.focus_config;
        for (i, facing) in self.facings.iter_mut().enumerate() {
            if self.focused_facing == Some(i) {
                // Focused arc: bonus
                facing.max_hp = self.base_max_hp + fc.bonus_max_hp;
                facing.regen_per_sec = self.base_regen_per_sec + fc.bonus_regen;
                facing.is_focused = true;
            } else if self.focused_facing.is_some() {
                // Another arc is focused: penalty on this arc
                facing.max_hp = (self.base_max_hp - fc.penalty_max_hp).max(0);
                facing.regen_per_sec = (self.base_regen_per_sec - fc.penalty_regen).max(0.0);
                facing.is_focused = false;
            } else {
                // No focus: restore base values
                facing.max_hp = self.base_max_hp;
                facing.regen_per_sec = self.base_regen_per_sec;
                facing.is_focused = false;
            }
            // Clamp HP to effective max_hp (but we don't reduce it here — decay does that over time)
        }
    }

    /// Determine the facing index hit by an attacker at `bearing_relative` radians
    /// (angle relative to the ship's own yaw, in (-π, π], anti-clockwise positive).
    ///
    /// Facing 0 is centred on forward (bearing 0). Facing indices increase
    /// **clockwise** when viewed from above: Fore(0) → Port(1) → Aft(2) → Starboard(3).
    /// Port is at bearing -π/2 and Starboard at +π/2.
    pub fn facing_index_for_bearing(&self, bearing_relative: f32) -> usize {
        let n = self.facings.len() as f32;
        // Negate so that going clockwise (Port = -π/2) increases the index.
        let angle = (-bearing_relative).rem_euclid(TAU);
        // Shift so facing 0 is centred on 0 (forward).
        let shifted = (angle + TAU / (2.0 * n)).rem_euclid(TAU);
        let idx = (shifted / (TAU / n)) as usize;
        idx.min(self.facings.len() - 1)
    }

    /// Apply `amount` damage from `bearing_relative` (radians relative to ship yaw).
    ///
    /// Returns the hull passthrough damage (0 if shields fully absorbed the hit).
    pub fn apply_damage(&mut self, amount: i32, bearing_relative: f32) -> i32 {
        let idx = self.facing_index_for_bearing(bearing_relative);
        self.facings[idx].apply_damage(amount)
    }

    /// Apply `amount` damage uniformly across all shield facings.
    ///
    /// Each facing receives an equal share; any remainder is distributed
    /// one-at-a-time to the first `amount % N` facings. Returns the total
    /// hull passthrough damage (sum of overflow from each facing).
    pub fn apply_uniform_damage(&mut self, amount: i32) -> i32 {
        let n = self.facings.len();
        if n == 0 {
            return amount;
        }
        let base = amount / n as i32;
        let rem = amount % n as i32;
        let mut total_leak = 0i32;
        for i in 0..n {
            let facing_amount = base + if (i as i32) < rem { 1 } else { 0 };
            total_leak += self.facings[i].apply_damage(facing_amount);
        }
        total_leak
    }

    /// Advance all facings by `dt` seconds (regen + offline timers + focus decay).
    ///
    /// Non-focused facings whose HP exceeds their (reduced) effective max_hp decay at
    /// `focus_config.decay_rate` per second until they reach the cap.  While decaying
    /// toward the reduced maximum, regen is suppressed so the transition is gradual
    /// (rather than snapping to max_hp in a single tick).
    pub fn tick(&mut self, dt: f32) {
        for (i, facing) in self.facings.iter_mut().enumerate() {
            let is_decaying = self.focused_facing != Some(i) && facing.hp > facing.max_hp;

            if is_decaying {
                // Apply focus decay toward effective max_hp (no regen while decaying).
                // Accumulate fractional decay in hp_frac (negative while decaying) so
                // sub-integer rates are applied correctly across frames.
                facing.hp_frac -= self.focus_config.decay_rate * dt;
                let whole = facing.hp_frac.abs() as i32;
                if whole > 0 {
                    let target = facing.max_hp;
                    facing.hp = (facing.hp - whole).max(target);
                    facing.hp_frac += whole as f32; // remove the consumed integer part
                }
                // Clear accumulator once we've reached (or gone below) the reduced max.
                if facing.hp <= facing.max_hp {
                    facing.hp = facing.max_hp;
                    facing.hp_frac = 0.0;
                }
                // Still tick offline timer if applicable.
                if !facing.is_online() {
                    facing.offline_remaining = (facing.offline_remaining - dt).max(0.0);
                }
            } else {
                facing.tick(dt);
            }
        }
    }

    /// Snapshot all facings for broadcast.
    pub fn snapshot(&self) -> Vec<ShieldFacingSnapshot> {
        self.facings.iter().map(|f| f.snapshot()).collect()
    }
}

impl Default for ShieldSystem {
    fn default() -> Self {
        let mut sys = Self::new(&ShieldConfig::default());
        sys.focus_config = ShieldFocusConfig::default();
        sys
    }
}

/// Compute the bearing of an attacker position relative to the ship's yaw.
///
/// `attacker_x`, `attacker_z` — world-space position of the attacker (or the
///   point from which the hit originates).
/// `ship_x`, `ship_z` — world-space position of the ship.
/// `ship_yaw` — ship's current yaw in radians (0 = forward along –Z axis).
///
/// Returns the bearing in `(-π, π]` measured anti-clockwise from the ship's
/// forward direction, consistent with `facing_index_for_bearing`.
pub fn attacker_bearing_relative(
    attacker_x: f32,
    attacker_z: f32,
    ship_x: f32,
    ship_z: f32,
    ship_yaw: f32,
) -> f32 {
    // World-space direction from ship to attacker.
    let dx = attacker_x - ship_x;
    let dz = attacker_z - ship_z;

    // World-space bearing of the attacker (atan2 in XZ plane).
    // We use atan2(dx, -dz) so that "forward" (dx=0, dz<0) gives 0.
    let world_bearing = dx.atan2(-dz);

    // Subtract ship yaw to get bearing relative to the ship's own frame.
    // Then normalise to (-π, π].
    let relative = world_bearing - ship_yaw;
    let tau = std::f32::consts::TAU;

    ((relative + std::f32::consts::PI).rem_euclid(tau)) - std::f32::consts::PI
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    // ── ShieldFacing ─────────────────────────────────────────────────────────

    #[test]
    fn facing_starts_at_full_hp_and_online() {
        let f = ShieldFacing::new("Fore", 100, 5.0, 10.0);
        assert_eq!(f.hp, 100);
        assert!(f.is_online());
    }

    #[test]
    fn damage_reduces_hp() {
        let mut f = ShieldFacing::new("Fore", 100, 5.0, 10.0);
        let passthrough = f.apply_damage(30);
        assert_eq!(f.hp, 70);
        assert_eq!(passthrough, 0);
    }

    #[test]
    fn damage_that_depletes_facing_sends_overflow_to_hull() {
        let mut f = ShieldFacing::new("Fore", 50, 5.0, 10.0);
        let passthrough = f.apply_damage(70); // 70 > 50 max_hp
        assert_eq!(f.hp, 0);
        assert_eq!(passthrough, 20);
        assert!(!f.is_online());
    }

    #[test]
    fn exact_depletion_leaves_no_passthrough_but_goes_offline() {
        let mut f = ShieldFacing::new("Fore", 100, 5.0, 10.0);
        let passthrough = f.apply_damage(100);
        assert_eq!(passthrough, 0);
        assert!(!f.is_online());
    }

    #[test]
    fn offline_facing_passes_all_damage_to_hull() {
        let mut f = ShieldFacing::new("Fore", 100, 5.0, 10.0);
        f.apply_damage(100); // deplete → offline
        let passthrough = f.apply_damage(40);
        assert_eq!(passthrough, 40);
    }

    #[test]
    fn offline_timer_counts_down_via_tick() {
        let mut f = ShieldFacing::new("Fore", 100, 0.0, 10.0);
        f.apply_damage(100); // offline for 10s
        f.tick(4.0);
        assert!(!f.is_online());
        assert!((f.offline_remaining - 6.0).abs() < 1e-4);
    }

    #[test]
    fn facing_comes_back_online_at_full_hp_after_offline_duration() {
        let mut f = ShieldFacing::new("Fore", 100, 0.0, 10.0);
        f.apply_damage(100); // offline for 10s
        f.tick(10.0);
        assert!(f.is_online());
        assert_eq!(f.hp, 100);
    }

    #[test]
    fn regen_increases_hp_while_online() {
        let mut f = ShieldFacing::new("Fore", 100, 10.0, 10.0);
        f.apply_damage(40); // hp = 60
        f.tick(2.0); // +20 → hp = 80
        assert_eq!(f.hp, 80);
    }

    #[test]
    fn regen_does_not_exceed_max_hp() {
        let mut f = ShieldFacing::new("Fore", 100, 50.0, 10.0);
        f.apply_damage(10); // hp = 90
        f.tick(10.0); // +500 → capped at 100
        assert_eq!(f.hp, 100);
    }

    #[test]
    fn no_regen_while_offline() {
        let mut f = ShieldFacing::new("Fore", 100, 50.0, 10.0);
        f.apply_damage(100); // offline
        f.tick(1.0); // timer ticks, no regen
        assert_eq!(f.hp, 0);
    }

    // ── ShieldSystem facing index ────────────────────────────────────────────

    #[test]
    fn four_facings_default_layout() {
        let s = ShieldSystem::default(); // 4 facings
                                         // Forward (bearing 0) → facing 0 (Fore)
        assert_eq!(s.facing_index_for_bearing(0.0), 0);
        // 90° left (port, bearing -PI/2) → facing 1 (Port)
        assert_eq!(s.facing_index_for_bearing(-PI / 2.0), 1);
        // Directly aft (PI or -PI) → facing 2 (Aft)
        assert_eq!(s.facing_index_for_bearing(PI), 2);
        // 90° right (starboard, bearing +PI/2) → facing 3 (Starboard)
        assert_eq!(s.facing_index_for_bearing(PI / 2.0), 3);
    }

    #[test]
    fn two_facings_fore_aft_layout() {
        let config = ShieldConfig {
            num_facings: 2,
            ..Default::default()
        };
        let s = ShieldSystem::new(&config);
        assert_eq!(s.facing_index_for_bearing(0.0), 0); // fore
        assert_eq!(s.facing_index_for_bearing(PI), 1); // aft
                                                       // 45° to the left: still in the fore hemisphere
        assert_eq!(s.facing_index_for_bearing(-PI / 4.0), 0);
    }

    // ── ShieldSystem damage routing ──────────────────────────────────────────

    #[test]
    fn damage_routed_to_correct_facing() {
        let mut s = ShieldSystem::default();
        s.apply_damage(20, 0.0); // hits fore
        assert_eq!(s.facings[0].hp, 80);
        assert_eq!(s.facings[1].hp, 100); // port untouched
    }

    #[test]
    fn damage_passthrough_when_facing_depleted() {
        let config = ShieldConfig {
            max_hp: 50,
            ..Default::default()
        };
        let mut s = ShieldSystem::new(&config);
        let passthrough = s.apply_damage(60, 0.0); // fore only has 50
        assert_eq!(passthrough, 10);
    }

    // ── ShieldSystem tick ────────────────────────────────────────────────────

    #[test]
    fn tick_regenerates_all_facings() {
        let mut s = ShieldSystem::default(); // regen 2/s
        s.apply_damage(20, 0.0); // fore: 80
        s.apply_damage(10, PI / 2.0); // starboard: 90
        s.tick(2.0); // +4 fore → 84, +4 starboard → 94
        assert_eq!(s.facings[0].hp, 84);
        assert_eq!(s.facings[3].hp, 94);
    }

    // ── ShieldSystem snapshot ────────────────────────────────────────────────

    #[test]
    fn snapshot_returns_all_four_facings() {
        let s = ShieldSystem::default();
        let snaps = s.snapshot();
        assert_eq!(snaps.len(), 4);
        assert_eq!(snaps[0].label, "Fore");
        assert_eq!(snaps[1].label, "Port");
        assert_eq!(snaps[2].label, "Aft");
        assert_eq!(snaps[3].label, "Starboard");
    }

    #[test]
    fn snapshot_reflects_current_hp_and_online_status() {
        let mut s = ShieldSystem::default();
        s.apply_damage(100, 0.0); // deplete fore
        let snaps = s.snapshot();
        assert_eq!(snaps[0].hp, 0);
        assert!(!snaps[0].online);
        assert_eq!(snaps[1].hp, 100);
        assert!(snaps[1].online);
    }

    // ── configurable arcs ────────────────────────────────────────────────────

    #[test]
    fn single_facing_absorbs_all_bearings() {
        let config = ShieldConfig {
            num_facings: 1,
            ..Default::default()
        };
        let mut s = ShieldSystem::new(&config);
        s.apply_damage(10, 0.0);
        s.apply_damage(10, PI);
        s.apply_damage(10, PI / 2.0);
        assert_eq!(s.facings[0].hp, 70);
    }

    #[test]
    fn custom_config_max_hp_and_regen() {
        let config = ShieldConfig {
            num_facings: 2,
            max_hp: 200,
            regen_per_sec: 20.0,
            offline_duration: 5.0,
        };
        let s = ShieldSystem::new(&config);
        assert_eq!(s.facings.len(), 2);
        assert_eq!(s.facings[0].max_hp, 200);
        assert_eq!(s.facings[0].hp, 200);
    }

    // ── attacker_bearing_relative ────────────────────────────────────────────

    #[test]
    fn attacker_directly_ahead_gives_zero_bearing() {
        // Ship at origin, yaw = 0, attacker in front (negative Z)
        let b = attacker_bearing_relative(0.0, -10.0, 0.0, 0.0, 0.0);
        assert!(b.abs() < 1e-4, "expected ~0, got {b}");
    }

    #[test]
    fn attacker_directly_aft_gives_pi_bearing() {
        let b = attacker_bearing_relative(0.0, 10.0, 0.0, 0.0, 0.0);
        assert!((b.abs() - PI).abs() < 1e-4, "expected ~±π, got {b}");
    }

    #[test]
    fn attacker_to_starboard_gives_positive_bearing() {
        // Starboard is to the right; with yaw=0 forward=-Z, right = +X
        let b = attacker_bearing_relative(10.0, 0.0, 0.0, 0.0, 0.0);
        assert!((b - PI / 2.0).abs() < 1e-4, "expected ~+π/2, got {b}");
    }

    #[test]
    fn attacker_to_port_gives_negative_bearing() {
        let b = attacker_bearing_relative(-10.0, 0.0, 0.0, 0.0, 0.0);
        assert!((b + PI / 2.0).abs() < 1e-4, "expected ~-π/2, got {b}");
    }

    #[test]
    fn bearing_accounts_for_ship_yaw() {
        // Ship rotated 90° clockwise (yaw = +π/2).
        // Attacker is in the world's +X direction; relative to the ship that
        // should now be directly ahead.
        let b = attacker_bearing_relative(10.0, 0.0, 0.0, 0.0, PI / 2.0);
        assert!(b.abs() < 1e-4, "expected ~0, got {b}");
    }

    #[test]
    fn bearing_routes_to_fore_facing() {
        let s = ShieldSystem::default(); // 4 facings
                                         // Attacker straight ahead → Fore (index 0)
        let b = attacker_bearing_relative(0.0, -10.0, 0.0, 0.0, 0.0);
        assert_eq!(s.facing_index_for_bearing(b), 0);
    }

    #[test]
    fn bearing_routes_to_aft_facing() {
        let s = ShieldSystem::default();
        // Attacker straight behind → Aft (index 2)
        let b = attacker_bearing_relative(0.0, 10.0, 0.0, 0.0, 0.0);
        assert_eq!(s.facing_index_for_bearing(b), 2);
    }

    // ── Focus mechanics ────────────────────────────────────────────────────

    #[test]
    fn default_focused_facing_is_none() {
        let s = ShieldSystem::default();
        assert!(s.focused_facing.is_none());
        for f in &s.facings {
            assert!(!f.is_focused);
        }
    }

    #[test]
    fn set_focused_facing_toggles_the_focused_flag() {
        let mut s = ShieldSystem::default();
        s.set_focused_facing(Some(0)); // Focus Fore
        assert_eq!(s.focused_facing, Some(0));
        assert!(s.facings[0].is_focused);
        assert!(!s.facings[1].is_focused);
        assert!(!s.facings[2].is_focused);
        assert!(!s.facings[3].is_focused);
    }

    #[test]
    fn set_focused_facing_none_clears_focus() {
        let mut s = ShieldSystem::default();
        s.set_focused_facing(Some(0));
        assert!(s.facings[0].is_focused);
        s.set_focused_facing(None);
        assert!(s.focused_facing.is_none());
        for f in &s.facings {
            assert!(!f.is_focused);
        }
    }

    #[test]
    fn focused_facing_gets_bonus_max_hp_and_regen() {
        let mut s = ShieldSystem::default();
        assert_eq!(s.facings[0].max_hp, 100);
        s.set_focused_facing(Some(0));
        // Default focus config: bonus_max_hp=50, bonus_regen=5.0
        assert_eq!(s.facings[0].max_hp, 150);
        assert!((s.facings[0].regen_per_sec - 7.0).abs() < 1e-4);
    }

    #[test]
    fn non_focused_facings_get_penalty_max_hp_and_regen() {
        let mut s = ShieldSystem::default();
        s.set_focused_facing(Some(0)); // Focus Fore
                                       // Default: penalty_max_hp=25, penalty_regen=1.0
        assert_eq!(s.facings[1].max_hp, 75); // Port
        assert_eq!(s.facings[2].max_hp, 75); // Aft
        assert_eq!(s.facings[3].max_hp, 75); // Starboard
        assert!((s.facings[1].regen_per_sec - 1.0).abs() < 1e-4);
    }

    #[test]
    fn clearing_focus_restores_base_max_hp_and_regen_for_all() {
        let mut s = ShieldSystem::default();
        s.set_focused_facing(Some(0));
        assert_eq!(s.facings[0].max_hp, 150);
        assert_eq!(s.facings[1].max_hp, 75);
        // Simulate the focused facing having regen'd above base max_hp.
        s.facings[0].hp = 130;
        s.set_focused_facing(None);
        for f in &s.facings {
            assert_eq!(f.max_hp, 100);
            assert!((f.regen_per_sec - 2.0).abs() < 1e-4);
        }
        // HP is NOT snapped immediately — it decays gradually via tick().
        assert_eq!(
            s.facings[0].hp, 130,
            "HP should persist above max after clear"
        );
        s.tick(0.5); // decay_rate=10/s * 0.5s = 5 HP decay
        assert_eq!(s.facings[0].hp, 125);
        s.tick(3.0); // 125 - min(10*3, 125-100) = 100
        assert_eq!(s.facings[0].hp, 100);
        // Once at max, regen applies normally on subsequent ticks.
        s.tick(0.5); // regen 2/s → 100 + 1.0 = 101.0 → capped to 100
        assert_eq!(s.facings[0].hp, 100);
    }

    #[test]
    fn non_focused_facing_decays_when_above_reduced_max() {
        let mut s = ShieldSystem::default();
        // Damage fore (facing 0) so HP drops, then focus Port (facing 1).
        // Port becomes focused at 150 max_hp. Fore becomes non-focused at 75 max_hp.
        s.facings[1].hp = 130; // Port HP above base 100 (will become 75 effective max)
        s.facings[3].hp = 120; // Starboard HP above base 100
        s.set_focused_facing(Some(1)); // Focus Port

        // After recalculate_focus, facings 0,2,3 have effective max=75 with HP above that.
        // Port (focused) has effective max=150 with HP=130 (no decay).
        s.tick(0.5); // decay_rate=10/s * 0.5s = 5 HP decay

        assert!(!s.facings[0].is_focused);
        assert!(s.facings[1].is_focused);
        // Facing 0 (Fore): base was 100, but focus recalculate doesn't clamp HP.
        // After recalculate max_hp=75, HP=100 (above max). Decays at 10/s for 0.5s = 5.
        assert_eq!(s.facings[0].hp, 95);
        // Facing 3 (Starboard): HP was 120, max becomes 75. Decays 10/s for 0.5s = 5.
        assert_eq!(s.facings[3].hp, 115);
        // Facing 1 (Port, focused): normal tick (base 2.0 + bonus 5.0 = 7.0/s). 130 + 7.0*0.5 ≈ +3 → 133.
        assert_eq!(s.facings[1].hp, 133);
    }

    #[test]
    fn non_focused_facing_stops_decaying_when_at_or_below_reduced_max() {
        let mut s = ShieldSystem::default();
        // Fore (facing 0) at 80 HP, gets reduced max=75 when another arc focused.
        s.facings[0].hp = 80;
        s.set_focused_facing(Some(1)); // Focus Port
                                       // Fore max=75, HP=80 → 80 - 10*0.5 = 75 → should decay to exactly 75
        s.tick(0.5);
        assert_eq!(s.facings[0].hp, 75);
        // Next tick: HP=75 ≤ max=75 → no more decay
        s.tick(0.5);
        assert_eq!(s.facings[0].hp, 75);
    }

    #[test]
    fn snapshot_includes_is_focused() {
        let mut s = ShieldSystem::default();
        s.set_focused_facing(Some(0));
        let snaps = s.snapshot();
        assert!(snaps[0].is_focused);
        assert!(!snaps[1].is_focused);
        assert!(!snaps[2].is_focused);
        assert!(!snaps[3].is_focused);
    }

    #[test]
    fn focused_facing_not_subject_to_decay_rate() {
        let mut s = ShieldSystem::default();
        // Focus Fore so effective max becomes 150. HP above base can only
        // be reduced by the normal regen cap, not by focus decay.
        s.set_focused_facing(Some(0));
        s.facings[0].hp = 200;
        s.tick(0.5);
        // Normal regen tick caps at max_hp=150: (200 + 7.0*0.5 → +3).min(150) = 150
        assert_eq!(s.facings[0].hp, 150);
        // The decay code (which only targets non-focused facings) did not run,
        // confirming the focused arc does not get focus-decayed.
    }

    /// End-to-end TOML-driven wiring check: build the runtime `ShieldSystem`
    /// the same way `spawn_game_start_entities` does (parse player_ship.toml
    /// → ShieldsBaseConfig::to_runtime → ShieldSystem::new) and assert the
    /// facings reflect the TOML. Changing `max_hp = 100` to `max_hp = 999`
    /// in `[shields_console.base]` would fail this test.
    #[test]
    fn shield_system_reflects_player_ship_toml_shields_console_base_block() {
        let toml_str = include_str!("../../assets/entities/player_ship.toml");
        let config = crate::entity_config::EntityConfig::from_toml(toml_str)
            .expect("player_ship.toml must parse");
        let base = config
            .shields_console
            .expect("player_ship must declare [shields_console]")
            .base
            .expect("player_ship must declare [shields_console.base]");
        let shield_config = base.to_runtime();
        let system = ShieldSystem::new(&shield_config);
        // Each facing must take its max_hp from the TOML.
        assert_eq!(system.facings.len(), base.num_facings);
        for f in &system.facings {
            assert_eq!(f.max_hp, base.max_hp, "facing max_hp must match TOML");
            assert_eq!(f.hp, base.max_hp, "facing starts full");
            assert_eq!(f.regen_per_sec, base.regen_per_sec, "regen must match TOML");
            assert_eq!(
                f.offline_duration, base.offline_duration,
                "offline_duration must match TOML"
            );
        }
    }
}
