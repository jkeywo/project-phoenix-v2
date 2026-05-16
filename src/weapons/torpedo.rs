/// Pure-Rust torpedo mechanics.
///
/// This module is platform-agnostic and Bevy-free. It models a torpedo system
/// with three tubes:
///   - Fore port (90° fire arc centred forward-port)
///   - Fore starboard (90° fire arc centred forward-starboard)
///   - Aft (90° fire arc centred aft)
///
/// Each tube has a configurable load time. When a torpedo is launched it
/// tracks a target UUID with a limited turn rate. If the target is gone the
/// torpedo flies straight. Torpedoes expire after a configurable lifespan.
///
/// Coordinate system: same as `ship_physics` / `radar` — XZ plane, Y-up.
/// Ship forward is −Z when yaw = 0.

use std::f32::consts::{PI, FRAC_PI_2};

// ── Tube identifier ────────────────────────────────────────────────────────

/// Which torpedo tube is being addressed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TorpedoTubeId {
    ForePort,
    ForeStarboard,
    Aft,
}

// ── Configuration ──────────────────────────────────────────────────────────

/// Tuning knobs for the torpedo system.
#[derive(Clone, Debug)]
pub struct TorpedoConfig {
    /// Total torpedoes available (shared pool).
    pub count: u32,
    /// Hull damage on impact.
    pub damage_hull: i32,
    /// Shield damage on impact.
    pub damage_shields: i32,
    /// Travel speed in world units per second.
    pub speed: f32,
    /// Maximum turn rate in radians per second (homing).
    pub turn_rate: f32,
    /// Seconds until an un-hit torpedo expires.
    pub lifespan: f32,
    /// Default tube reload time in seconds.
    pub load_time: f32,
}

impl Default for TorpedoConfig {
    fn default() -> Self {
        Self {
            count: 10,
            damage_hull: 50,
            damage_shields: 5,
            speed: 30.0,
            turn_rate: PI / 4.0, // 45°/s
            lifespan: 20.0,
            load_time: 10.0,
        }
    }
}

// ── In-flight torpedo ──────────────────────────────────────────────────────

/// A single torpedo in flight.
#[derive(Clone, Debug)]
pub struct Torpedo {
    /// Stable identifier (UUID string).
    pub uuid: String,
    /// Current world-space position.
    pub x: f32,
    pub z: f32,
    /// Current heading in radians (same convention as ship yaw).
    pub heading: f32,
    /// Remaining lifespan in seconds.
    pub lifespan_remaining: f32,
    /// Target entity UUID, if homing. `None` → fly straight.
    pub target_uuid: Option<String>,
}

impl Torpedo {
    /// Advance the torpedo by `dt` seconds.
    ///
    /// If `target_pos` is `Some((tx, tz))` and `target_uuid` is set, the
    /// torpedo steers toward the target at up to `turn_rate` rad/s.
    /// Otherwise it flies straight.
    pub fn tick(&mut self, dt: f32, target_pos: Option<(f32, f32)>, config: &TorpedoConfig) {
        // Homing
        if self.target_uuid.is_some() {
            if let Some((tx, tz)) = target_pos {
                let dx = tx - self.x;
                let dz = tz - self.z;
                let desired = (dx).atan2(-dz); // atan2(x, -z) gives yaw convention
                let delta = angle_diff(desired, self.heading);
                let max_turn = config.turn_rate * dt;
                self.heading += delta.clamp(-max_turn, max_turn);
            }
            // If target_pos is None (target destroyed), fly straight — no heading change
        }

        // Move forward
        let cos_h = self.heading.cos();
        let sin_h = self.heading.sin();
        // forward is (-z) direction when heading=0: dx = sin(heading), dz = -cos(heading)
        self.x += sin_h * config.speed * dt;
        self.z -= cos_h * config.speed * dt;

        // Age
        self.lifespan_remaining = (self.lifespan_remaining - dt).max(0.0);
    }

    /// Returns `true` if the torpedo has expired (lifespan reached zero).
    pub fn is_expired(&self) -> bool {
        self.lifespan_remaining <= 0.0
    }
}

// ── Torpedo tube ───────────────────────────────────────────────────────────

/// A single torpedo tube with load-time tracking.
#[derive(Clone, Debug)]
pub struct TorpedoTube {
    pub id: TorpedoTubeId,
    /// Seconds remaining until this tube is ready again. 0.0 = loaded.
    pub reload_remaining: f32,
    /// Half-angle of the fire arc in radians. Torpedo may only be launched
    /// when the tube is aimed within this arc.
    pub arc_half_rad: f32,
    /// Centre heading offset from ship forward (in radians).
    /// e.g. ForePort = −45°, ForeStarboard = +45°, Aft = ±180°.
    pub centre_offset_rad: f32,
}

impl TorpedoTube {
    pub fn new(id: TorpedoTubeId, _load_time_override: Option<f32>, _default_load_time: f32) -> Self {
        let (arc_half_rad, centre_offset_rad) = match id {
            TorpedoTubeId::ForePort => (FRAC_PI_2 / 2.0, -PI / 4.0),       // 45° arc, -45° offset
            TorpedoTubeId::ForeStarboard => (FRAC_PI_2 / 2.0, PI / 4.0),   // 45° arc, +45° offset
            TorpedoTubeId::Aft => (FRAC_PI_2 / 2.0, PI),                    // 45° arc, 180° offset
        };
        Self {
            id,
            reload_remaining: 0.0, // starts loaded
            arc_half_rad,
            centre_offset_rad,
        }
    }

    /// Returns `true` if the tube is loaded (ready to fire).
    pub fn is_loaded(&self) -> bool {
        self.reload_remaining <= 0.0
    }

    /// Advance the reload timer by `dt` seconds.
    pub fn tick(&mut self, dt: f32) {
        self.reload_remaining = (self.reload_remaining - dt).max(0.0);
    }

    /// Begin reloading (after a torpedo is launched).
    pub fn start_reload(&mut self, load_time: f32) {
        self.reload_remaining = load_time;
    }

    /// Returns `true` if a target at the given bearing (radians from ship
    /// forward, where +right = positive) is within this tube's fire arc.
    pub fn is_in_arc(&self, bearing_rad: f32) -> bool {
        let diff = angle_diff(bearing_rad, self.centre_offset_rad).abs();
        diff <= self.arc_half_rad
    }
}

// ── Torpedo system ─────────────────────────────────────────────────────────

/// The complete torpedo system: three tubes, shared torpedo pool, in-flight
/// torpedoes.
#[derive(Clone, Debug)]
pub struct TorpedoSystem {
    pub fore_port: TorpedoTube,
    pub fore_starboard: TorpedoTube,
    pub aft: TorpedoTube,
    pub config: TorpedoConfig,
    /// Number of torpedoes remaining in the magazine.
    pub torpedoes_remaining: u32,
    /// All currently in-flight torpedoes.
    pub in_flight: Vec<Torpedo>,
}

/// Result of a launch attempt.
#[derive(Clone, Debug, PartialEq)]
pub enum LaunchResult {
    /// Torpedo was successfully launched. Contains the torpedo UUID.
    Launched { uuid: String },
    /// The tube is still reloading.
    TubeNotLoaded,
    /// The magazine is empty.
    NoTorpedoes,
}

/// Result of calling `tick` — lists torpedoes that expired or hit something.
#[derive(Clone, Debug, Default)]
pub struct TorpedoTickResult {
    /// UUIDs of torpedoes that expired this tick (lifespan → 0).
    pub expired: Vec<String>,
}

impl TorpedoSystem {
    pub fn new(config: TorpedoConfig) -> Self {
        let count = config.count;
        Self {
            fore_port: TorpedoTube::new(TorpedoTubeId::ForePort, None, config.load_time),
            fore_starboard: TorpedoTube::new(TorpedoTubeId::ForeStarboard, None, config.load_time),
            aft: TorpedoTube::new(TorpedoTubeId::Aft, None, config.load_time),
            config,
            torpedoes_remaining: count,
            in_flight: Vec::new(),
        }
    }

    /// Attempt to launch a torpedo from the given tube.
    ///
    /// `uuid` — caller-supplied identifier for the new torpedo.
    /// `launch_x`, `launch_z` — ship position (launch origin).
    /// `launch_heading` — initial heading in radians (typically tube direction).
    /// `target_uuid` — optional homing target.
    pub fn launch(
        &mut self,
        tube_id: TorpedoTubeId,
        uuid: String,
        launch_x: f32,
        launch_z: f32,
        launch_heading: f32,
        target_uuid: Option<String>,
    ) -> LaunchResult {
        if self.torpedoes_remaining == 0 {
            return LaunchResult::NoTorpedoes;
        }
        let tube = self.tube_mut(tube_id);
        if !tube.is_loaded() {
            return LaunchResult::TubeNotLoaded;
        }
        let load_time = self.config.load_time;
        let lifespan = self.config.lifespan;
        let tube = self.tube_mut(tube_id);
        tube.start_reload(load_time);

        self.torpedoes_remaining -= 1;
        self.in_flight.push(Torpedo {
            uuid: uuid.clone(),
            x: launch_x,
            z: launch_z,
            heading: launch_heading,
            lifespan_remaining: lifespan,
            target_uuid,
        });
        LaunchResult::Launched { uuid }
    }

    /// Advance all tubes and in-flight torpedoes by `dt` seconds.
    ///
    /// `target_positions` maps target UUID strings to current world position.
    /// Returns a `TorpedoTickResult` listing expired torpedoes.
    pub fn tick(
        &mut self,
        dt: f32,
        target_positions: &std::collections::HashMap<String, (f32, f32)>,
    ) -> TorpedoTickResult {
        self.fore_port.tick(dt);
        self.fore_starboard.tick(dt);
        self.aft.tick(dt);

        let config = &self.config;
        let mut expired = Vec::new();
        for t in &mut self.in_flight {
            let pos = t.target_uuid.as_ref().and_then(|id| target_positions.get(id)).copied();
            t.tick(dt, pos, config);
            if t.is_expired() {
                expired.push(t.uuid.clone());
            }
        }
        self.in_flight.retain(|t| !t.is_expired());
        TorpedoTickResult { expired }
    }

    /// Resolve a torpedo hitting something.  Removes the torpedo from
    /// in-flight and returns the hull damage to apply (or `None` if the
    /// torpedo UUID was not found).
    pub fn handle_collision(&mut self, torpedo_uuid: &str) -> Option<i32> {
        if let Some(pos) = self.in_flight.iter().position(|t| t.uuid == torpedo_uuid) {
            self.in_flight.remove(pos);
            Some(self.config.damage_hull)
        } else {
            None
        }
    }

    // ── private helpers ───────────────────────────────────────────────────

    fn tube_mut(&mut self, id: TorpedoTubeId) -> &mut TorpedoTube {
        match id {
            TorpedoTubeId::ForePort => &mut self.fore_port,
            TorpedoTubeId::ForeStarboard => &mut self.fore_starboard,
            TorpedoTubeId::Aft => &mut self.aft,
        }
    }
}

// ── private math helpers ───────────────────────────────────────────────────

/// Returns the signed angular difference `a − b` wrapped to `[−π, π]`.
fn angle_diff(a: f32, b: f32) -> f32 {
    let mut d = a - b;
    while d > PI { d -= 2.0 * PI; }
    while d < -PI { d += 2.0 * PI; }
    d
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn default_system() -> TorpedoSystem {
        TorpedoSystem::new(TorpedoConfig::default())
    }

    // ── Launch logic ──────────────────────────────────────────────────────

    #[test]
    fn launch_returns_launched_with_uuid() {
        let mut sys = default_system();
        let result = sys.launch(TorpedoTubeId::ForePort, "t1".into(), 0.0, 0.0, 0.0, None);
        assert_eq!(result, LaunchResult::Launched { uuid: "t1".into() });
    }

    #[test]
    fn launch_adds_torpedo_to_in_flight() {
        let mut sys = default_system();
        sys.launch(TorpedoTubeId::ForePort, "t1".into(), 0.0, 0.0, 0.0, None);
        assert_eq!(sys.in_flight.len(), 1);
        assert_eq!(sys.in_flight[0].uuid, "t1");
    }

    #[test]
    fn launch_decrements_torpedo_count() {
        let mut sys = default_system();
        sys.launch(TorpedoTubeId::ForePort, "t1".into(), 0.0, 0.0, 0.0, None);
        assert_eq!(sys.torpedoes_remaining, 9);
    }

    #[test]
    fn launch_starts_tube_reload() {
        let mut sys = default_system();
        sys.launch(TorpedoTubeId::ForePort, "t1".into(), 0.0, 0.0, 0.0, None);
        assert!(!sys.fore_port.is_loaded());
    }

    #[test]
    fn launch_from_unloaded_tube_returns_not_loaded() {
        let mut sys = default_system();
        sys.launch(TorpedoTubeId::ForePort, "t1".into(), 0.0, 0.0, 0.0, None);
        let result = sys.launch(TorpedoTubeId::ForePort, "t2".into(), 0.0, 0.0, 0.0, None);
        assert_eq!(result, LaunchResult::TubeNotLoaded);
    }

    #[test]
    fn launch_with_no_torpedoes_returns_no_torpedoes() {
        let mut config = TorpedoConfig::default();
        config.count = 0;
        let mut sys = TorpedoSystem::new(config);
        let result = sys.launch(TorpedoTubeId::ForePort, "t1".into(), 0.0, 0.0, 0.0, None);
        assert_eq!(result, LaunchResult::NoTorpedoes);
    }

    #[test]
    fn can_launch_from_all_three_tubes_independently() {
        let mut sys = default_system();
        let r1 = sys.launch(TorpedoTubeId::ForePort, "t1".into(), 0.0, 0.0, 0.0, None);
        let r2 = sys.launch(TorpedoTubeId::ForeStarboard, "t2".into(), 0.0, 0.0, 0.0, None);
        let r3 = sys.launch(TorpedoTubeId::Aft, "t3".into(), 0.0, 0.0, 0.0, None);
        assert!(matches!(r1, LaunchResult::Launched { .. }));
        assert!(matches!(r2, LaunchResult::Launched { .. }));
        assert!(matches!(r3, LaunchResult::Launched { .. }));
        assert_eq!(sys.in_flight.len(), 3);
    }

    // ── Homing behaviour ──────────────────────────────────────────────────

    #[test]
    fn torpedo_with_no_target_flies_straight() {
        let mut sys = default_system();
        sys.launch(TorpedoTubeId::ForePort, "t1".into(), 0.0, 0.0, 0.0, None);
        let initial_heading = sys.in_flight[0].heading;
        let targets: HashMap<String, (f32, f32)> = HashMap::new();
        sys.tick(0.1, &targets);
        assert_eq!(sys.in_flight[0].heading, initial_heading);
    }

    #[test]
    fn torpedo_moves_forward_in_straight_flight() {
        let mut sys = default_system();
        // heading = 0 → forward = −Z direction
        sys.launch(TorpedoTubeId::ForePort, "t1".into(), 0.0, 0.0, 0.0, None);
        let targets: HashMap<String, (f32, f32)> = HashMap::new();
        sys.tick(1.0, &targets);
        let t = &sys.in_flight[0];
        // x should stay ~0, z should decrease (moved forward)
        assert!((t.x).abs() < 0.01, "x should not change: {}", t.x);
        assert!(t.z < -0.0, "z should decrease (forward): {}", t.z);
    }

    #[test]
    fn torpedo_homes_toward_target() {
        let mut sys = default_system();
        // Launch heading 0 (straight ahead = -Z). Target is directly to the right (+X, 0).
        // Torpedo should turn right (positive heading).
        sys.launch(TorpedoTubeId::ForePort, "t1".into(), 0.0, 0.0, 0.0, Some("enemy".into()));
        let mut targets = HashMap::new();
        targets.insert("enemy".into(), (20.0_f32, 0.0_f32));
        let heading_before = sys.in_flight[0].heading;
        sys.tick(0.1, &targets);
        let heading_after = sys.in_flight[0].heading;
        assert!(heading_after > heading_before, "torpedo should turn right toward target");
    }

    #[test]
    fn torpedo_turn_rate_is_limited() {
        let mut config = TorpedoConfig::default();
        config.turn_rate = PI / 4.0; // 45°/s
        let mut sys = TorpedoSystem::new(config);
        // Target directly to the right (90° turn needed).
        sys.launch(TorpedoTubeId::ForePort, "t1".into(), 0.0, 0.0, 0.0, Some("enemy".into()));
        let mut targets = HashMap::new();
        targets.insert("enemy".into(), (20.0_f32, 0.0_f32));
        sys.tick(1.0, &targets); // 1 second, max 45° turn
        let heading = sys.in_flight[0].heading;
        assert!(heading <= PI / 4.0 + 0.001, "should not exceed max turn rate: {}", heading);
    }

    #[test]
    fn torpedo_flies_straight_when_target_destroyed() {
        let mut sys = default_system();
        sys.launch(TorpedoTubeId::ForePort, "t1".into(), 0.0, 0.0, 0.0, Some("enemy".into()));
        // Target not provided (destroyed) — no position in map
        let targets: HashMap<String, (f32, f32)> = HashMap::new();
        let heading_before = sys.in_flight[0].heading;
        sys.tick(0.5, &targets);
        assert_eq!(sys.in_flight[0].heading, heading_before, "heading should not change");
    }

    // ── Expiration ────────────────────────────────────────────────────────

    #[test]
    fn torpedo_expires_after_lifespan() {
        let mut config = TorpedoConfig::default();
        config.lifespan = 5.0;
        let mut sys = TorpedoSystem::new(config);
        sys.launch(TorpedoTubeId::ForePort, "t1".into(), 0.0, 0.0, 0.0, None);
        let targets: HashMap<String, (f32, f32)> = HashMap::new();
        let result = sys.tick(5.1, &targets);
        assert!(result.expired.contains(&"t1".to_string()));
        assert_eq!(sys.in_flight.len(), 0);
    }

    #[test]
    fn torpedo_not_expired_before_lifespan() {
        let mut config = TorpedoConfig::default();
        config.lifespan = 5.0;
        let mut sys = TorpedoSystem::new(config);
        sys.launch(TorpedoTubeId::ForePort, "t1".into(), 0.0, 0.0, 0.0, None);
        let targets: HashMap<String, (f32, f32)> = HashMap::new();
        let result = sys.tick(4.9, &targets);
        assert!(!result.expired.contains(&"t1".to_string()));
        assert_eq!(sys.in_flight.len(), 1);
    }

    // ── Collision resolution ──────────────────────────────────────────────

    #[test]
    fn collision_removes_torpedo_and_returns_damage() {
        let mut sys = default_system();
        sys.launch(TorpedoTubeId::ForePort, "t1".into(), 0.0, 0.0, 0.0, None);
        let damage = sys.handle_collision("t1");
        assert_eq!(damage, Some(50)); // default damage_hull
        assert_eq!(sys.in_flight.len(), 0);
    }

    #[test]
    fn collision_with_unknown_uuid_returns_none() {
        let mut sys = default_system();
        let damage = sys.handle_collision("nonexistent");
        assert_eq!(damage, None);
    }

    // ── Tube reload ───────────────────────────────────────────────────────

    #[test]
    fn tube_reloads_after_load_time() {
        let mut config = TorpedoConfig::default();
        config.load_time = 10.0;
        let mut sys = TorpedoSystem::new(config);
        sys.launch(TorpedoTubeId::ForePort, "t1".into(), 0.0, 0.0, 0.0, None);
        assert!(!sys.fore_port.is_loaded());
        let targets: HashMap<String, (f32, f32)> = HashMap::new();
        sys.tick(10.0, &targets);
        assert!(sys.fore_port.is_loaded());
    }

    #[test]
    fn tube_not_loaded_before_reload_time_expires() {
        let mut config = TorpedoConfig::default();
        config.load_time = 10.0;
        let mut sys = TorpedoSystem::new(config);
        sys.launch(TorpedoTubeId::ForePort, "t1".into(), 0.0, 0.0, 0.0, None);
        let targets: HashMap<String, (f32, f32)> = HashMap::new();
        sys.tick(9.9, &targets);
        assert!(!sys.fore_port.is_loaded());
    }
}
