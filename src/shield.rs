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
            regen_per_sec: 5.0,
            offline_duration: 10.0,
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
}

impl ShieldFacing {
    fn new(label: impl Into<String>, max_hp: i32, regen_per_sec: f32, offline_duration: f32) -> Self {
        Self {
            label: label.into(),
            hp: max_hp,
            max_hp,
            regen_per_sec,
            offline_duration,
            offline_remaining: 0.0,
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
                // Just came back online — restore to full HP.
                self.hp = self.max_hp;
            }
        } else {
            // Regen while online.
            let new_hp = (self.hp as f32 + self.regen_per_sec * dt) as i32;
            self.hp = new_hp.min(self.max_hp);
        }
    }

    pub fn snapshot(&self) -> ShieldFacingSnapshot {
        ShieldFacingSnapshot {
            label: self.label.clone(),
            hp: self.hp,
            max_hp: self.max_hp,
            online: self.is_online(),
            offline_remaining: self.offline_remaining,
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
        Self { facings }
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

    /// Advance all facings by `dt` seconds (regen + offline timers).
    pub fn tick(&mut self, dt: f32) {
        for facing in &mut self.facings {
            facing.tick(dt);
        }
    }

    /// Snapshot all facings for broadcast.
    pub fn snapshot(&self) -> Vec<ShieldFacingSnapshot> {
        self.facings.iter().map(|f| f.snapshot()).collect()
    }
}

impl Default for ShieldSystem {
    fn default() -> Self {
        Self::new(&ShieldConfig::default())
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
    let wrapped = ((relative + std::f32::consts::PI).rem_euclid(tau)) - std::f32::consts::PI;
    wrapped
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
        f.tick(2.0);        // +20 → hp = 80
        assert_eq!(f.hp, 80);
    }

    #[test]
    fn regen_does_not_exceed_max_hp() {
        let mut f = ShieldFacing::new("Fore", 100, 50.0, 10.0);
        f.apply_damage(10); // hp = 90
        f.tick(10.0);       // +500 → capped at 100
        assert_eq!(f.hp, 100);
    }

    #[test]
    fn no_regen_while_offline() {
        let mut f = ShieldFacing::new("Fore", 100, 50.0, 10.0);
        f.apply_damage(100); // offline
        f.tick(1.0);         // timer ticks, no regen
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
        let config = ShieldConfig { num_facings: 2, ..Default::default() };
        let s = ShieldSystem::new(&config);
        assert_eq!(s.facing_index_for_bearing(0.0), 0);   // fore
        assert_eq!(s.facing_index_for_bearing(PI), 1);     // aft
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
        let config = ShieldConfig { max_hp: 50, ..Default::default() };
        let mut s = ShieldSystem::new(&config);
        let passthrough = s.apply_damage(60, 0.0); // fore only has 50
        assert_eq!(passthrough, 10);
    }

    // ── ShieldSystem tick ────────────────────────────────────────────────────

    #[test]
    fn tick_regenerates_all_facings() {
        let mut s = ShieldSystem::default(); // regen 5/s
        s.apply_damage(20, 0.0); // fore: 80
        s.apply_damage(10, PI / 2.0); // starboard: 90
        s.tick(2.0); // +10 fore → 90, +10 starboard → 100
        assert_eq!(s.facings[0].hp, 90);
        assert_eq!(s.facings[3].hp, 100);
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
        let config = ShieldConfig { num_facings: 1, ..Default::default() };
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
}
