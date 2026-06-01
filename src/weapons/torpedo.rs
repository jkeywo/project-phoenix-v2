//! Pure-Rust torpedo mechanics.
//!
//! This module is platform-agnostic and Bevy-free. The torpedo system holds
//! a `Vec<TorpedoTube>` whose contents come from `[[torpedoes.tubes]]` in
//! `assets/entities/player_ship.toml`. Each tube has a TOML-defined `id`,
//! `facing_deg`, and `fire_arc_deg`. Ammunition is a single shared pool
//! (`[torpedoes] count`).
//!
//! When a torpedo is launched it tracks a target UUID with a limited turn
//! rate. If the target is gone the torpedo flies straight. Torpedoes expire
//! after a configurable lifespan.
//!
//! Coordinate system: same as `ship_physics` / `radar` — XZ plane, Y-up.
//! Ship forward is −Z when yaw = 0.

use std::f32::consts::PI;

/// String identifier for a torpedo tube (matches the `id` field in TOML).
pub type TorpedoTubeId = String;

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
    /// Proximity-detonation radius in world units. A torpedo explodes when
    /// its centre comes within `detonation_radius + target_radius` of any
    /// entity. Independent of homing — an un-locked torpedo still detonates
    /// on contact.
    pub detonation_radius: f32,
    /// Fraction of the `damage_shields` payload that bypasses the shield
    /// system entirely and adds to hull damage. Default `0.0` — all
    /// `damage_shields` is mitigated by the facing shield quadrant.
    /// `damage_hull` is unaffected (it always hits hull by design).
    /// Clamped to `[0.0, 1.0]` at apply time.
    pub shield_pierce: f32,
}

impl Default for TorpedoConfig {
    fn default() -> Self {
        Self {
            count: 10,
            damage_hull: 50,
            damage_shields: 5,
            speed: 30.0,
            turn_rate: PI / 4.0,
            lifespan: 20.0,
            load_time: 10.0,
            detonation_radius: 5.0,
            shield_pierce: 0.0,
        }
    }
}

// ── In-flight torpedo ──────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct Torpedo {
    pub uuid: String,
    pub x: f32,
    pub z: f32,
    pub heading: f32,
    pub lifespan_remaining: f32,
    pub target_uuid: Option<String>,
    /// UUID of the entity that fired this torpedo. Used by
    /// [`TorpedoSystem::find_detonation_hits`] to prevent a torpedo from
    /// detonating on its launcher (the torpedo spawns at the launcher's
    /// centre, well within any reasonable detonation radius).
    pub source_uuid: Option<String>,
    /// Snapshot of the firing tube's `shield_pierce` at launch time, so
    /// detonation logic can split `damage_shields` between absorbed and
    /// pierced portions without re-resolving the source config. Clamped
    /// to `[0.0, 1.0]` at apply time.
    pub shield_pierce: f32,
}

impl Torpedo {
    pub fn tick(&mut self, dt: f32, target_pos: Option<(f32, f32)>, config: &TorpedoConfig) {
        if self.target_uuid.is_some() {
            if let Some((tx, tz)) = target_pos {
                let dx = tx - self.x;
                let dz = tz - self.z;
                let desired = (dx).atan2(-dz);
                let delta = angle_diff(desired, self.heading);
                let max_turn = config.turn_rate * dt;
                self.heading += delta.clamp(-max_turn, max_turn);
            }
        }
        let cos_h = self.heading.cos();
        let sin_h = self.heading.sin();
        self.x += sin_h * config.speed * dt;
        self.z -= cos_h * config.speed * dt;
        self.lifespan_remaining = (self.lifespan_remaining - dt).max(0.0);
    }

    pub fn is_expired(&self) -> bool {
        self.lifespan_remaining <= 0.0
    }
}

// ── Torpedo tube ───────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct TorpedoTube {
    /// Tube identifier from TOML (e.g. `"fore_port"`, `"aft"`).
    pub id: TorpedoTubeId,
    /// Centre of the tube's fire arc, degrees clockwise from ship-forward.
    pub facing_deg: f32,
    /// Total fire-arc width in degrees.
    pub fire_arc_deg: f32,
    /// Seconds remaining until this tube is ready again. 0.0 = loaded.
    pub reload_remaining: f32,
}

impl TorpedoTube {
    pub fn is_loaded(&self) -> bool {
        self.reload_remaining <= 0.0
    }

    pub fn tick(&mut self, dt: f32) {
        self.reload_remaining = (self.reload_remaining - dt).max(0.0);
    }

    pub fn start_reload(&mut self, load_time: f32) {
        self.reload_remaining = load_time;
    }

    /// True if `bearing_rad` (radians from ship-forward, +right = positive)
    /// is within this tube's fire arc.
    pub fn is_in_arc(&self, bearing_rad: f32) -> bool {
        let half = (self.fire_arc_deg.to_radians()) * 0.5;
        let facing = self.facing_deg.to_radians();
        angle_diff(bearing_rad, facing).abs() <= half
    }
}

// ── Torpedo system ─────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct TorpedoSystem {
    pub tubes: Vec<TorpedoTube>,
    pub config: TorpedoConfig,
    pub torpedoes_remaining: u32,
    pub in_flight: Vec<Torpedo>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum LaunchResult {
    Launched { uuid: String },
    TubeNotLoaded,
    NoTorpedoes,
    UnknownTube,
}

#[derive(Clone, Debug, Default)]
pub struct TorpedoTickResult {
    pub expired: Vec<String>,
}

impl TorpedoSystem {
    /// Construct a torpedo system with the three legacy tubes
    /// (`fore_port`, `fore_starboard`, `aft`) for test convenience.
    /// Production code should use [`Self::from_configs`].
    pub fn new(config: TorpedoConfig) -> Self {
        let count = config.count;
        let tubes = vec![
            TorpedoTube { id: "fore_port".to_string(), facing_deg: -30.0, fire_arc_deg: 90.0, reload_remaining: 0.0 },
            TorpedoTube { id: "fore_starboard".to_string(), facing_deg: 30.0, fire_arc_deg: 90.0, reload_remaining: 0.0 },
            TorpedoTube { id: "aft".to_string(), facing_deg: 180.0, fire_arc_deg: 90.0, reload_remaining: 0.0 },
        ];
        Self {
            tubes,
            config,
            torpedoes_remaining: count,
            in_flight: Vec::new(),
        }
    }

    /// Build a torpedo system from the parsed TOML tube configs.
    pub fn from_configs(
        tubes: &[crate::entities::config::TorpedoTubeConfig],
        config: TorpedoConfig,
    ) -> Self {
        let tubes = tubes
            .iter()
            .map(|c| TorpedoTube {
                id: c.id.clone(),
                facing_deg: c.facing_deg,
                fire_arc_deg: c.fire_arc_deg,
                reload_remaining: 0.0,
            })
            .collect();
        let count = config.count;
        Self {
            tubes,
            config,
            torpedoes_remaining: count,
            in_flight: Vec::new(),
        }
    }

    pub fn tube(&self, id: &str) -> Option<&TorpedoTube> {
        self.tubes.iter().find(|t| t.id == id)
    }

    pub fn tube_mut(&mut self, id: &str) -> Option<&mut TorpedoTube> {
        self.tubes.iter_mut().find(|t| t.id == id)
    }

    pub fn launch(
        &mut self,
        tube_id: &str,
        uuid: String,
        launch_x: f32,
        launch_z: f32,
        launch_heading: f32,
        target_uuid: Option<String>,
        source_uuid: Option<String>,
    ) -> LaunchResult {
        if self.torpedoes_remaining == 0 {
            return LaunchResult::NoTorpedoes;
        }
        let load_time = self.config.load_time;
        let lifespan = self.config.lifespan;
        let Some(tube) = self.tube_mut(tube_id) else {
            return LaunchResult::UnknownTube;
        };
        if !tube.is_loaded() {
            return LaunchResult::TubeNotLoaded;
        }
        tube.start_reload(load_time);
        self.torpedoes_remaining -= 1;
        let shield_pierce = self.config.shield_pierce;
        self.in_flight.push(Torpedo {
            uuid: uuid.clone(),
            x: launch_x,
            z: launch_z,
            heading: launch_heading,
            lifespan_remaining: lifespan,
            target_uuid,
            source_uuid,
            shield_pierce,
        });
        LaunchResult::Launched { uuid }
    }

    pub fn tick(
        &mut self,
        dt: f32,
        target_positions: &std::collections::HashMap<String, (f32, f32)>,
    ) -> TorpedoTickResult {
        for t in &mut self.tubes {
            t.tick(dt);
        }
        let config = &self.config;
        let mut expired = Vec::new();
        for t in &mut self.in_flight {
            let pos = t
                .target_uuid
                .as_ref()
                .and_then(|id| target_positions.get(id))
                .copied();
            t.tick(dt, pos, config);
            if t.is_expired() {
                expired.push(t.uuid.clone());
            }
        }
        self.in_flight.retain(|t| !t.is_expired());
        TorpedoTickResult { expired }
    }

    pub fn handle_collision(&mut self, torpedo_uuid: &str) -> Option<i32> {
        self.handle_collision_full(torpedo_uuid).map(|d| d.damage_hull)
    }

    /// Full detonation payload: hull damage, shield damage, and the firing
    /// tube's `shield_pierce` fraction (snapshot taken at launch). Removes
    /// the torpedo from `in_flight`. Returns `None` if `torpedo_uuid` is
    /// unknown.
    pub fn handle_collision_full(&mut self, torpedo_uuid: &str) -> Option<TorpedoDetonation> {
        let pos = self.in_flight.iter().position(|t| t.uuid == torpedo_uuid)?;
        let removed = self.in_flight.remove(pos);
        Some(TorpedoDetonation {
            damage_hull: self.config.damage_hull,
            damage_shields: self.config.damage_shields,
            shield_pierce: removed.shield_pierce,
        })
    }
}

/// Result of a successful torpedo detonation, returned by
/// [`TorpedoSystem::handle_collision_full`]. The caller applies the damage
/// split according to its own target model:
///
/// - `damage_hull` always lands on the hull (it is pierce-by-design).
/// - `damage_shields` is the shield-eligible portion. Use
///   [`split_damage_for_pierce`](crate::damage::split_damage_for_pierce)
///   with `shield_pierce` to compute the pierced vs absorbed split for it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TorpedoDetonation {
    pub damage_hull: i32,
    pub damage_shields: i32,
    pub shield_pierce: f32,
}

impl TorpedoSystem {
    /// Find proximity-detonation hits between in-flight torpedoes and the
    /// supplied target volumes. Returns `(torpedo_uuid, target_uuid)` pairs.
    ///
    /// A torpedo `T` hits target `E` when
    /// `distance(T, E) <= detonation_radius + target_radius`, with one
    /// exception: a torpedo never detonates on its own [`Torpedo::source_uuid`]
    /// (the entity that fired it). Without this, every torpedo would detonate
    /// on launch because it spawns at the firing ship's centre.
    ///
    /// Each torpedo can only hit one target per call (the nearest qualifying
    /// one); the caller is responsible for removing detonated torpedoes via
    /// [`Self::handle_collision`].
    ///
    /// `targets` is a slice of `(uuid, x, z, radius)` tuples.
    pub fn find_detonation_hits(
        &self,
        targets: &[(String, f32, f32, f32)],
    ) -> Vec<(String, String)> {
        let det = self.config.detonation_radius;
        let mut hits = Vec::new();
        for torpedo in &self.in_flight {
            let mut best: Option<(f32, &String)> = None;
            for (uuid, tx, tz, radius) in targets {
                if torpedo.source_uuid.as_ref() == Some(uuid) {
                    continue;
                }
                let dx = tx - torpedo.x;
                let dz = tz - torpedo.z;
                let dist_sq = dx * dx + dz * dz;
                let threshold = det + radius;
                if dist_sq <= threshold * threshold {
                    if best.map(|(d, _)| dist_sq < d).unwrap_or(true) {
                        best = Some((dist_sq, uuid));
                    }
                }
            }
            if let Some((_, uuid)) = best {
                hits.push((torpedo.uuid.clone(), uuid.clone()));
            }
        }
        hits
    }
}

// ── private math helpers ───────────────────────────────────────────────────

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

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::config::TorpedoTubeConfig;
    use std::collections::HashMap;

    fn cfg(id: &str, facing_deg: f32, fire_arc_deg: f32) -> TorpedoTubeConfig {
        TorpedoTubeConfig {
            id: id.into(),
            facing_deg,
            fire_arc_deg,
        }
    }

    fn default_system() -> TorpedoSystem {
        let tubes = vec![
            cfg("fore_port", -30.0, 90.0),
            cfg("fore_starboard", 30.0, 90.0),
            cfg("aft", 180.0, 90.0),
        ];
        TorpedoSystem::from_configs(&tubes, TorpedoConfig::default())
    }

    #[test]
    fn launch_returns_launched_with_uuid() {
        let mut sys = default_system();
        let r = sys.launch("fore_port", "t1".into(), 0.0, 0.0, 0.0, None, None);
        assert_eq!(r, LaunchResult::Launched { uuid: "t1".into() });
    }

    #[test]
    fn launch_adds_torpedo_to_in_flight() {
        let mut sys = default_system();
        sys.launch("fore_port", "t1".into(), 0.0, 0.0, 0.0, None, None);
        assert_eq!(sys.in_flight.len(), 1);
        assert_eq!(sys.in_flight[0].uuid, "t1");
    }

    #[test]
    fn launch_decrements_torpedo_count() {
        let mut sys = default_system();
        sys.launch("fore_port", "t1".into(), 0.0, 0.0, 0.0, None, None);
        assert_eq!(sys.torpedoes_remaining, 9);
    }

    #[test]
    fn launch_starts_tube_reload() {
        let mut sys = default_system();
        sys.launch("fore_port", "t1".into(), 0.0, 0.0, 0.0, None, None);
        assert!(!sys.tube("fore_port").unwrap().is_loaded());
    }

    #[test]
    fn launch_from_unloaded_tube_returns_not_loaded() {
        let mut sys = default_system();
        sys.launch("fore_port", "t1".into(), 0.0, 0.0, 0.0, None, None);
        let r = sys.launch("fore_port", "t2".into(), 0.0, 0.0, 0.0, None, None);
        assert_eq!(r, LaunchResult::TubeNotLoaded);
    }

    #[test]
    fn launch_from_unknown_tube_returns_unknown() {
        let mut sys = default_system();
        let r = sys.launch("dorsal", "t1".into(), 0.0, 0.0, 0.0, None, None);
        assert_eq!(r, LaunchResult::UnknownTube);
    }

    #[test]
    fn launch_with_no_torpedoes_returns_no_torpedoes() {
        let mut config = TorpedoConfig::default();
        config.count = 0;
        let tubes = vec![cfg("fore_port", -30.0, 90.0)];
        let mut sys = TorpedoSystem::from_configs(&tubes, config);
        let r = sys.launch("fore_port", "t1".into(), 0.0, 0.0, 0.0, None, None);
        assert_eq!(r, LaunchResult::NoTorpedoes);
    }

    #[test]
    fn can_launch_from_all_three_tubes_independently() {
        let mut sys = default_system();
        let r1 = sys.launch("fore_port", "t1".into(), 0.0, 0.0, 0.0, None, None);
        let r2 = sys.launch("fore_starboard", "t2".into(), 0.0, 0.0, 0.0, None, None);
        let r3 = sys.launch("aft", "t3".into(), 0.0, 0.0, 0.0, None, None);
        assert!(matches!(r1, LaunchResult::Launched { .. }));
        assert!(matches!(r2, LaunchResult::Launched { .. }));
        assert!(matches!(r3, LaunchResult::Launched { .. }));
        assert_eq!(sys.in_flight.len(), 3);
    }

    #[test]
    fn torpedo_with_no_target_flies_straight() {
        let mut sys = default_system();
        sys.launch("fore_port", "t1".into(), 0.0, 0.0, 0.0, None, None);
        let initial = sys.in_flight[0].heading;
        let targets: HashMap<String, (f32, f32)> = HashMap::new();
        sys.tick(0.1, &targets);
        assert_eq!(sys.in_flight[0].heading, initial);
    }

    #[test]
    fn torpedo_moves_forward_in_straight_flight() {
        let mut sys = default_system();
        sys.launch("fore_port", "t1".into(), 0.0, 0.0, 0.0, None, None);
        let targets: HashMap<String, (f32, f32)> = HashMap::new();
        sys.tick(1.0, &targets);
        let t = &sys.in_flight[0];
        assert!(t.x.abs() < 0.01);
        assert!(t.z < 0.0);
    }

    #[test]
    fn torpedo_homes_toward_target() {
        let mut sys = default_system();
        sys.launch("fore_port", "t1".into(), 0.0, 0.0, 0.0, Some("enemy".into()), None);
        let mut targets = HashMap::new();
        targets.insert("enemy".into(), (20.0_f32, 0.0_f32));
        let h0 = sys.in_flight[0].heading;
        sys.tick(0.1, &targets);
        assert!(sys.in_flight[0].heading > h0);
    }

    #[test]
    fn torpedo_turn_rate_is_limited() {
        let mut config = TorpedoConfig::default();
        config.turn_rate = PI / 4.0;
        let tubes = vec![cfg("fore_port", -30.0, 90.0)];
        let mut sys = TorpedoSystem::from_configs(&tubes, config);
        sys.launch("fore_port", "t1".into(), 0.0, 0.0, 0.0, Some("enemy".into()), None);
        let mut targets = HashMap::new();
        targets.insert("enemy".into(), (20.0_f32, 0.0_f32));
        sys.tick(1.0, &targets);
        assert!(sys.in_flight[0].heading <= PI / 4.0 + 0.001);
    }

    #[test]
    fn torpedo_flies_straight_when_target_destroyed() {
        let mut sys = default_system();
        sys.launch("fore_port", "t1".into(), 0.0, 0.0, 0.0, Some("enemy".into()), None);
        let targets: HashMap<String, (f32, f32)> = HashMap::new();
        let h0 = sys.in_flight[0].heading;
        sys.tick(0.5, &targets);
        assert_eq!(sys.in_flight[0].heading, h0);
    }

    #[test]
    fn torpedo_target_uuid_locked_at_launch_and_never_updated() {
        // Fire at "target-a". Then tick with positions for both "target-a"
        // (far right) and a new "target-b" (straight ahead). The torpedo must
        // keep homing toward "target-a", never re-routing to "target-b", and
        // its stored target_uuid must remain "target-a" throughout.
        let mut sys = default_system();
        sys.launch("fore_port", "t1".into(), 0.0, 0.0, 0.0, Some("target-a".into()), None);
        let mut targets = HashMap::new();
        targets.insert("target-a".into(), (100.0_f32, 0.0_f32)); // hard right
        targets.insert("target-b".into(), (0.0_f32, -100.0_f32)); // straight ahead

        let h0 = sys.in_flight[0].heading;
        sys.tick(0.1, &targets);

        // The torpedo must have turned right (toward target-a), not stayed straight.
        assert!(sys.in_flight[0].heading > h0, "should home toward target-a (rightward turn)");
        // The stored target_uuid is still "target-a".
        assert_eq!(
            sys.in_flight[0].target_uuid.as_deref(),
            Some("target-a"),
            "target_uuid must not change after launch"
        );
    }

    #[test]
    fn torpedo_expires_after_lifespan() {
        let mut config = TorpedoConfig::default();
        config.lifespan = 5.0;
        let tubes = vec![cfg("fore_port", -30.0, 90.0)];
        let mut sys = TorpedoSystem::from_configs(&tubes, config);
        sys.launch("fore_port", "t1".into(), 0.0, 0.0, 0.0, None, None);
        let targets: HashMap<String, (f32, f32)> = HashMap::new();
        let r = sys.tick(5.1, &targets);
        assert!(r.expired.contains(&"t1".to_string()));
        assert_eq!(sys.in_flight.len(), 0);
    }

    #[test]
    fn torpedo_not_expired_before_lifespan() {
        let mut config = TorpedoConfig::default();
        config.lifespan = 5.0;
        let tubes = vec![cfg("fore_port", -30.0, 90.0)];
        let mut sys = TorpedoSystem::from_configs(&tubes, config);
        sys.launch("fore_port", "t1".into(), 0.0, 0.0, 0.0, None, None);
        let targets: HashMap<String, (f32, f32)> = HashMap::new();
        let r = sys.tick(4.9, &targets);
        assert!(!r.expired.contains(&"t1".to_string()));
        assert_eq!(sys.in_flight.len(), 1);
    }

    #[test]
    fn collision_removes_torpedo_and_returns_damage() {
        let mut sys = default_system();
        sys.launch("fore_port", "t1".into(), 0.0, 0.0, 0.0, None, None);
        let d = sys.handle_collision("t1");
        assert_eq!(d, Some(50));
        assert_eq!(sys.in_flight.len(), 0);
    }

    #[test]
    fn collision_with_unknown_uuid_returns_none() {
        let mut sys = default_system();
        let d = sys.handle_collision("nonexistent");
        assert_eq!(d, None);
    }

    #[test]
    fn tube_reloads_after_load_time() {
        let mut config = TorpedoConfig::default();
        config.load_time = 10.0;
        let tubes = vec![cfg("fore_port", -30.0, 90.0)];
        let mut sys = TorpedoSystem::from_configs(&tubes, config);
        sys.launch("fore_port", "t1".into(), 0.0, 0.0, 0.0, None, None);
        assert!(!sys.tube("fore_port").unwrap().is_loaded());
        let targets: HashMap<String, (f32, f32)> = HashMap::new();
        sys.tick(10.0, &targets);
        assert!(sys.tube("fore_port").unwrap().is_loaded());
    }

    #[test]
    fn tube_not_loaded_before_reload_time_expires() {
        let mut config = TorpedoConfig::default();
        config.load_time = 10.0;
        let tubes = vec![cfg("fore_port", -30.0, 90.0)];
        let mut sys = TorpedoSystem::from_configs(&tubes, config);
        sys.launch("fore_port", "t1".into(), 0.0, 0.0, 0.0, None, None);
        let targets: HashMap<String, (f32, f32)> = HashMap::new();
        sys.tick(9.9, &targets);
        assert!(!sys.tube("fore_port").unwrap().is_loaded());
    }

    // ── proximity detonation ──────────────────────────────────────────────

    fn detonation_system(detonation_radius: f32) -> TorpedoSystem {
        let mut config = TorpedoConfig::default();
        config.detonation_radius = detonation_radius;
        let tubes = vec![cfg("fore_port", -30.0, 90.0)];
        TorpedoSystem::from_configs(&tubes, config)
    }

    #[test]
    fn find_detonation_hits_returns_empty_when_no_targets_in_range() {
        let mut sys = detonation_system(5.0);
        sys.launch("fore_port", "t1".into(), 0.0, 0.0, 0.0, None, None);
        // Target far away with small radius.
        let targets = vec![("enemy".to_string(), 100.0, 100.0, 1.0)];
        let hits = sys.find_detonation_hits(&targets);
        assert!(hits.is_empty());
    }

    #[test]
    fn find_detonation_hits_reports_target_within_detonation_radius() {
        let mut sys = detonation_system(5.0);
        sys.launch("fore_port", "t1".into(), 0.0, 0.0, 0.0, None, None);
        // Target at (0, -4): distance 4, threshold 5+0 = 5.
        let targets = vec![("enemy".to_string(), 0.0, -4.0, 0.0)];
        let hits = sys.find_detonation_hits(&targets);
        assert_eq!(hits, vec![("t1".to_string(), "enemy".to_string())]);
    }

    #[test]
    fn find_detonation_hits_includes_target_radius_in_threshold() {
        // Detonation radius 1, target radius 10, distance 9 → should hit.
        let mut sys = detonation_system(1.0);
        sys.launch("fore_port", "t1".into(), 0.0, 0.0, 0.0, None, None);
        let targets = vec![("rock".to_string(), 0.0, -9.0, 10.0)];
        let hits = sys.find_detonation_hits(&targets);
        assert_eq!(hits, vec![("t1".to_string(), "rock".to_string())]);
    }

    #[test]
    fn find_detonation_hits_picks_nearest_when_multiple_in_range() {
        let mut sys = detonation_system(50.0);
        sys.launch("fore_port", "t1".into(), 0.0, 0.0, 0.0, None, None);
        let targets = vec![
            ("far".to_string(), 0.0, -40.0, 0.0),
            ("near".to_string(), 0.0, -5.0, 0.0),
        ];
        let hits = sys.find_detonation_hits(&targets);
        assert_eq!(hits, vec![("t1".to_string(), "near".to_string())]);
    }

    #[test]
    fn find_detonation_hits_detonates_unlocked_torpedo_on_contact() {
        // Bug repro: shot without a target lock should still explode.
        let mut sys = detonation_system(5.0);
        sys.launch("fore_port", "t1".into(), 0.0, 0.0, 0.0, /*target_uuid*/ None, /*source_uuid*/ None);
        let targets = vec![("raider".to_string(), 0.0, -3.0, 1.0)];
        let hits = sys.find_detonation_hits(&targets);
        assert_eq!(hits, vec![("t1".to_string(), "raider".to_string())]);
    }

    #[test]
    fn find_detonation_hits_handles_multiple_torpedoes_independently() {
        let mut sys = detonation_system(2.0);
        sys.launch("fore_port", "t1".into(), 0.0, 0.0, 0.0, None, None);
        // Move tube ready by skipping reload — instead launch via a fresh tube.
        // Manually push a second torpedo to avoid tube cooldown.
        sys.in_flight.push(Torpedo {
            uuid: "t2".into(),
            x: 100.0,
            z: 100.0,
            heading: 0.0,
            lifespan_remaining: 10.0,
            target_uuid: None,
            source_uuid: None,
            shield_pierce: 0.0,
        });
        let targets = vec![
            ("a".to_string(), 1.0, 0.0, 0.0), // close to t1
            ("b".to_string(), 101.0, 100.0, 0.0), // close to t2
        ];
        let hits = sys.find_detonation_hits(&targets);
        assert_eq!(hits.len(), 2);
        assert!(hits.contains(&("t1".to_string(), "a".to_string())));
        assert!(hits.contains(&("t2".to_string(), "b".to_string())));
    }

    #[test]
    fn find_detonation_hits_never_detonates_on_source_uuid() {
        // Regression: torpedoes spawn at the firing ship's centre. Without
        // source_uuid filtering, every torpedo would instantly detonate on
        // its launcher and never reach an actual target.
        let mut sys = detonation_system(5.0);
        sys.launch(
            "fore_port",
            "t1".into(),
            0.0,
            0.0,
            0.0,
            None,
            Some("player-ship".into()),
        );
        // Player ship sitting right on top of the torpedo, plus a raider
        // also in range further out.
        let targets = vec![
            ("player-ship".to_string(), 0.0, 0.0, 5.0),
            ("raider".to_string(), 0.0, -3.0, 1.0),
        ];
        let hits = sys.find_detonation_hits(&targets);
        // Should hit the raider, not the launcher.
        assert_eq!(hits, vec![("t1".to_string(), "raider".to_string())]);
    }

    #[test]
    fn find_detonation_hits_with_no_targets_in_range_returns_empty_even_if_source_present() {
        let mut sys = detonation_system(5.0);
        sys.launch(
            "fore_port",
            "t1".into(),
            0.0,
            0.0,
            0.0,
            None,
            Some("player-ship".into()),
        );
        let targets = vec![("player-ship".to_string(), 0.0, 0.0, 5.0)];
        let hits = sys.find_detonation_hits(&targets);
        assert!(hits.is_empty());
    }
}
