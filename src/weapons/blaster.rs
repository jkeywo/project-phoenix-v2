//! Pure-Rust blaster projectile mechanics.
//!
//! This module is platform-agnostic and Bevy-free. Each blaster bank has a
//! `BlasterBankConfig` (loaded from TOML) and a `BlasterSystem` runtime that
//! tracks in-flight projectiles and volley state.
//!
//! Coordinate system: same as `ship_physics` / `torpedo` — XZ plane, Y-up.
//! Ship forward is −Z when yaw = 0. Heading convention: atan2(dx, -dz).
//!
//! ## Linear motion prediction
//!
//! At fire time the bank computes where a target *will be* when the projectile
//! arrives (assuming constant velocity). Given:
//!
//!   - target current position: (tx, tz)
//!   - target velocity: (tvx, tvz)
//!   - projectile speed: `speed`
//!
//! The estimated intercept time is `distance / speed`; predicted position is
//! `(tx + tvx * t, tz + tvz * t)`. The projectile is oriented toward this
//! predicted position and flies straight — no mid-flight correction.

use std::f32::consts::PI;

// ── Configuration ──────────────────────────────────────────────────────────

/// TOML-loaded configuration for one blaster bank instance.
///
/// `serde(default)` on optional-feel fields so the struct can grow with
/// future TOML fields without a `deny_unknown_fields` compile error.
#[derive(Clone, Debug)]
pub struct BlasterBankConfig {
    /// Bank identifier (matches the TOML `id` field, e.g. `"fore"`, `"aft"`).
    pub id: String,
    /// Centre of the fire arc in degrees clockwise from ship-forward (0 = fore).
    pub facing_deg: f32,
    /// Total fire arc width in degrees.
    pub fire_arc_deg: f32,
    /// Number of projectiles in one volley.
    pub volley_count: u32,
    /// Delay between successive projectile launches within a volley (seconds).
    pub volley_interval_secs: f32,
    /// Cooldown applied after a full volley completes (seconds).
    pub cooldown_secs: f32,
    /// Charge time before firing begins (0 = instant; >0 reserved for a later
    /// issue — hold-to-fire behaviour).
    pub charge_time_secs: f32,
    /// Projectile travel speed in world units per second.
    pub projectile_speed: f32,
    /// Proximity hit radius for each projectile (world units).
    pub collision_radius: f32,
    /// Visual scale hint for the client renderer (reserved for a later issue).
    pub visual_scale: f32,
    /// Hull + shield damage applied on hit.
    pub damage: i32,
    /// Fraction `[0.0, 1.0]` of damage that bypasses shields entirely.
    pub shield_pierce: f32,
    /// Recoil impulse magnitude (reserved for a later issue, default 0).
    pub recoil_impulse: f32,
    /// Screenshake magnitude (reserved for a later issue, default 0).
    pub screenshake_magnitude: f32,
    /// Optional rig-marker name for mount point resolution.
    pub marker: Option<String>,
    /// Maximum range in world units. Projectile lifespan is computed as
    /// `range / projectile_speed` at fire time.
    pub range: f32,
}

impl Default for BlasterBankConfig {
    fn default() -> Self {
        Self {
            id: String::new(),
            facing_deg: 0.0,
            fire_arc_deg: 90.0,
            volley_count: 3,
            volley_interval_secs: 0.15,
            cooldown_secs: 3.0,
            charge_time_secs: 0.0,
            projectile_speed: 40.0,
            collision_radius: 1.5,
            visual_scale: 1.0,
            damage: 20,
            shield_pierce: 0.0,
            recoil_impulse: 0.0,
            screenshake_magnitude: 0.0,
            marker: None,
            range: 35.0,
        }
    }
}

// ── In-flight projectile ───────────────────────────────────────────────────

/// A single blaster bolt in flight.
#[derive(Clone, Debug)]
pub struct BlasterProjectile {
    /// Unique identifier for this projectile (UUID string).
    pub id: String,
    /// World X position.
    pub x: f32,
    /// World Z position.
    pub z: f32,
    /// Heading in radians. Convention: atan2(dx, -dz) — ship forward is -Z.
    pub heading: f32,
    /// Travel speed (world units / second).
    pub speed: f32,
    /// Seconds remaining before this projectile expires.
    pub lifespan_remaining: f32,
    /// Proximity hit radius (world units).
    pub collision_radius: f32,
    /// Damage applied on hit.
    pub damage: i32,
    /// Shield-pierce fraction `[0.0, 1.0]`.
    pub shield_pierce: f32,
    /// UUID of the entity that fired this projectile (excluded from hit
    /// detection so a blaster can't self-hit).
    pub source_uuid: String,
}

impl BlasterProjectile {
    /// Advance position by `dt` seconds and decrement lifespan.
    pub fn tick(&mut self, dt: f32) {
        self.x += self.heading.sin() * self.speed * dt;
        self.z -= self.heading.cos() * self.speed * dt;
        self.lifespan_remaining = (self.lifespan_remaining - dt).max(0.0);
    }

    /// True when this projectile has expired (lifespan exhausted).
    pub fn is_expired(&self) -> bool {
        self.lifespan_remaining <= 0.0
    }
}

// ── Volley runtime state ───────────────────────────────────────────────────

/// Runtime state for the volley + cooldown cycle of one blaster bank.
#[derive(Clone, Debug, Default)]
pub struct BlasterVolleyState {
    /// How many more projectiles remain to be launched in the current volley.
    pub pending_volley: u32,
    /// Countdown timer (seconds) before the next projectile in the volley fires.
    pub volley_timer: f32,
    /// True while the bank is waiting for the post-volley cooldown to expire.
    pub on_cooldown: bool,
    /// Seconds remaining on the cooldown timer (0 when ready).
    pub cooldown_remaining: f32,
    /// True while the bank is in the charge phase (hold-to-fire, issue #636).
    /// Only used when `BlasterBankConfig::charge_time_secs > 0`.
    pub charging: bool,
    /// Seconds elapsed in the current charge phase.
    pub charge_elapsed: f32,
}

// ── Blaster system ─────────────────────────────────────────────────────────

/// Runtime state for one blaster bank.
#[derive(Clone, Debug)]
pub struct BlasterSystem {
    /// TOML-derived config for this bank.
    pub config: BlasterBankConfig,
    /// In-flight projectiles fired from this bank.
    pub in_flight: Vec<BlasterProjectile>,
    /// Volley + cooldown cycle state.
    pub volley: BlasterVolleyState,
}

impl BlasterSystem {
    /// Create a new `BlasterSystem` with the given config.
    pub fn new(config: BlasterBankConfig) -> Self {
        Self {
            config,
            in_flight: Vec::new(),
            volley: BlasterVolleyState::default(),
        }
    }

    /// True when the bank can accept a new fire/charge command (not on
    /// cooldown, not mid-volley, and not already charging).
    pub fn is_fire_ready(&self) -> bool {
        !self.volley.on_cooldown && self.volley.pending_volley == 0 && !self.volley.charging
    }

    /// Start a new volley. Resets the volley timer so the first projectile
    /// fires on the next `tick` call (or immediately via the caller).
    ///
    /// Does nothing if the bank is already on cooldown or has a volley
    /// in progress.
    ///
    /// Note: `fire_arc_deg` is stored in `config` but is NOT enforced during
    /// launch — arc enforcement is deferred to a future issue. All projectiles
    /// are aimed at the predicted intercept regardless of the arc boundary.
    pub fn request_fire(&mut self) -> bool {
        if self.config.volley_count == 0 {
            return false;
        }
        if !self.is_fire_ready() {
            return false;
        }
        self.volley.pending_volley = self.config.volley_count;
        self.volley.volley_timer = 0.0; // fire immediately on first tick
        true
    }

    /// Begin the charge phase for a hold-to-fire bank (issue #636).
    ///
    /// When `charge_time_secs == 0` this delegates directly to `request_fire`
    /// (instant-fire path — unchanged behaviour for Destroyer blasters).
    ///
    /// When `charge_time_secs > 0` this sets `charging = true` and resets
    /// `charge_elapsed` to 0 if the bank is currently `is_fire_ready()`.
    ///
    /// Returns `true` when the bank accepted the command.
    pub fn request_charge_start(&mut self) -> bool {
        if self.config.charge_time_secs <= 0.0 {
            return self.request_fire();
        }
        if !self.is_fire_ready() {
            return false;
        }
        self.volley.charging = true;
        self.volley.charge_elapsed = 0.0;
        true
    }

    /// Cancel an in-progress charge phase (issue #636).
    ///
    /// Resets `charging` and `charge_elapsed` to zero with no cooldown and
    /// no ammo consumed. Safe to call even when not charging (no-op).
    pub fn request_charge_cancel(&mut self) {
        self.volley.charging = false;
        self.volley.charge_elapsed = 0.0;
    }

    /// Charge phase completion fraction in `[0.0, 1.0]` (issue #636).
    ///
    /// Returns `0.0` when `charge_time_secs == 0` (instant-fire banks never
    /// show a charge bar).
    pub fn charge_progress(&self) -> f32 {
        if self.config.charge_time_secs <= 0.0 {
            return 0.0;
        }
        (self.volley.charge_elapsed / self.config.charge_time_secs).clamp(0.0, 1.0)
    }

    /// Advance the volley timer by `dt` seconds.
    ///
    /// When `volley_timer` reaches 0 and `pending_volley > 0`, one projectile
    /// is launched and returned (for the caller to broadcast). When the volley
    /// completes (`pending_volley == 0`), the cooldown starts. Expired
    /// projectiles are also pruned here.
    ///
    /// Returns a list of `LaunchEvent`s — one per projectile launched this
    /// tick. The caller is responsible for obtaining the fire position and
    /// emitting server messages.
    pub fn tick(
        &mut self,
        dt: f32,
        shooter_x: f32,
        shooter_z: f32,
        shooter_yaw: f32,
        target_x: f32,
        target_z: f32,
        target_vx: f32,
        target_vz: f32,
        source_uuid: &str,
        next_uuid: &mut impl FnMut() -> String,
    ) -> Vec<LaunchEvent> {
        // Prune expired projectiles.
        self.in_flight.retain(|p| !p.is_expired());

        // Tick all live projectiles.
        for p in self.in_flight.iter_mut() {
            p.tick(dt);
        }

        // ── Charge phase (issue #636) ────────────────────────────────────
        if self.volley.charging {
            self.volley.charge_elapsed += dt;
            if self.volley.charge_elapsed >= self.config.charge_time_secs {
                // Charge complete — transition to volley start.
                self.volley.charging = false;
                self.volley.charge_elapsed = 0.0;
                if self.config.volley_count > 0 {
                    self.volley.pending_volley = self.config.volley_count;
                    self.volley.volley_timer = 0.0;
                }
            } else {
                // Still charging — no projectile launch yet.
                return Vec::new();
            }
        }

        // Tick cooldown.
        if self.volley.on_cooldown {
            self.volley.cooldown_remaining = (self.volley.cooldown_remaining - dt).max(0.0);
            if self.volley.cooldown_remaining <= 0.0 {
                self.volley.on_cooldown = false;
            }
            return Vec::new();
        }

        // No volley pending.
        if self.volley.pending_volley == 0 {
            return Vec::new();
        }

        // Count down the inter-shot timer.
        self.volley.volley_timer -= dt;
        if self.volley.volley_timer > 0.0 {
            return Vec::new();
        }

        // Fire one projectile.
        let uuid = next_uuid();
        let heading = predict_intercept_heading(
            shooter_x,
            shooter_z,
            target_x,
            target_z,
            target_vx,
            target_vz,
            self.config.projectile_speed,
            shooter_yaw,
            self.config.facing_deg,
        );
        let lifespan = self.config.range / self.config.projectile_speed;
        let projectile = BlasterProjectile {
            id: uuid.clone(),
            x: shooter_x,
            z: shooter_z,
            heading,
            speed: self.config.projectile_speed,
            lifespan_remaining: lifespan,
            collision_radius: self.config.collision_radius,
            damage: self.config.damage,
            shield_pierce: self.config.shield_pierce,
            source_uuid: source_uuid.to_string(),
        };
        let event = LaunchEvent {
            projectile_id: uuid,
            x: projectile.x,
            z: projectile.z,
            heading: projectile.heading,
        };
        self.in_flight.push(projectile);

        self.volley.pending_volley -= 1;
        // Schedule the next shot.
        if self.volley.pending_volley > 0 {
            self.volley.volley_timer = self.config.volley_interval_secs;
        } else {
            // Volley complete — start cooldown.
            self.volley.on_cooldown = true;
            self.volley.cooldown_remaining = self.config.cooldown_secs;
            self.volley.volley_timer = 0.0;
        }

        vec![event]
    }

    /// Check every live projectile against a list of possible targets.
    ///
    /// Returns a vec of `(projectile_id, target_uuid)` for every hit detected.
    /// The hit projectile is NOT removed here — call `consume_hit` to remove
    /// it after broadcasting the hit event.
    ///
    /// `targets`: `&[(uuid, x, z, radius)]`. The projectile's own
    /// `source_uuid` is excluded to prevent self-hits.
    pub fn find_hits(&self, targets: &[(String, f32, f32, f32)]) -> Vec<(String, String)> {
        let mut hits = Vec::new();
        for projectile in &self.in_flight {
            for (uuid, tx, tz, radius) in targets {
                if uuid == &projectile.source_uuid {
                    continue;
                }
                let dx = tx - projectile.x;
                let dz = tz - projectile.z;
                let dist = (dx * dx + dz * dz).sqrt();
                if dist <= projectile.collision_radius + radius {
                    hits.push((projectile.id.clone(), uuid.clone()));
                    break; // one hit per projectile
                }
            }
        }
        hits
    }

    /// Remove the projectile with the given id (after a hit). Returns the
    /// projectile data needed by the damage system.
    pub fn consume_hit(&mut self, projectile_id: &str) -> Option<HitData> {
        if let Some(pos) = self.in_flight.iter().position(|p| p.id == projectile_id) {
            let p = self.in_flight.remove(pos);
            Some(HitData {
                damage: p.damage,
                shield_pierce: p.shield_pierce,
                source_uuid: p.source_uuid,
            })
        } else {
            None
        }
    }

    /// Snapshot state for the `WeaponsUpdate` broadcast.
    ///
    /// `target` is the shooter's frozen combat-lock target position (world XZ),
    /// or `None` when no target is locked. Range/arc/no-target blocking is
    /// derived here from the bank's own `range`/`facing_deg`/`fire_arc_deg`
    /// config using the same [`crate::weapons::phaser::target_geometry`] the
    /// fire path uses, so the readiness contract cannot diverge from the fire
    /// gate (issue #764). `is_online` reflects hull-damage offline state.
    pub fn bank_state(
        &self,
        ship_x: f32,
        ship_z: f32,
        ship_yaw: f32,
        target: Option<(f32, f32)>,
        is_online: bool,
    ) -> crate::core::messages::BlasterBankState {
        let geometry = target.map(|(tx, tz)| {
            crate::weapons::phaser::target_geometry(
                tx,
                tz,
                ship_x,
                ship_z,
                ship_yaw,
                self.config.range,
                self.config.facing_deg,
                self.config.fire_arc_deg,
            )
        });
        // "Loading" for a blaster is the charge phase or an in-progress volley:
        // both mean the bank cannot accept a fresh fire command yet.
        let loading = self.volley.charging || self.volley.pending_volley > 0;
        let readiness = crate::core::messages::WeaponReadiness::evaluate(
            is_online,
            self.volley.on_cooldown,
            loading,
            false,
            geometry,
        );
        crate::core::messages::BlasterBankState {
            id: self.config.id.clone(),
            fire_ready: self.is_fire_ready(),
            on_cooldown: self.volley.on_cooldown,
            cooldown_remaining: self.volley.cooldown_remaining,
            pending_volley: self.volley.pending_volley,
            charge_progress: self.charge_progress(),
            has_charge: self.config.charge_time_secs > 0.0,
            readiness,
        }
    }
}

// ── Output types ────────────────────────────────────────────────────────────

/// Returned by `BlasterSystem::tick` once per projectile launched.
#[derive(Clone, Debug)]
pub struct LaunchEvent {
    pub projectile_id: String,
    pub x: f32,
    pub z: f32,
    pub heading: f32,
}

/// Projectile stats returned by `consume_hit` for the damage system.
#[derive(Clone, Debug)]
pub struct HitData {
    pub damage: i32,
    pub shield_pierce: f32,
    /// UUID of the ship that fired the bolt. Carried through so the damage
    /// system can attribute the hit — it has no other route back to the
    /// shooter once the projectile is consumed.
    pub source_uuid: String,
}

// ── Helpers ─────────────────────────────────────────────────────────────────

/// Convert a `f32` heading in radians to the range `(-PI, PI]`.
fn normalise_angle(a: f32) -> f32 {
    let mut r = a;
    while r > PI {
        r -= 2.0 * PI;
    }
    while r <= -PI {
        r += 2.0 * PI;
    }
    r
}

/// Compute the launch heading toward the predicted intercept position.
///
/// Uses linear motion prediction: `t_est = distance / speed`, then
/// `predicted = (tx + tvx * t_est, tz + tvz * t_est)`. If `speed` is zero
/// or the predicted position coincides with the shooter, falls back to the
/// bank's facing direction relative to `shooter_yaw`.
///
/// Returns a heading in radians following the project's convention:
/// `atan2(dx, -dz)` where `+Z` is backwards.
pub fn predict_intercept_heading(
    sx: f32,
    sz: f32,
    tx: f32,
    tz: f32,
    tvx: f32,
    tvz: f32,
    speed: f32,
    shooter_yaw: f32,
    facing_deg: f32,
) -> f32 {
    let fallback = normalise_angle(shooter_yaw + facing_deg.to_radians());

    if speed <= 0.0 {
        return fallback;
    }

    let dx0 = tx - sx;
    let dz0 = tz - sz;
    let dist = (dx0 * dx0 + dz0 * dz0).sqrt();
    let t_est = dist / speed;

    let px = tx + tvx * t_est;
    let pz = tz + tvz * t_est;

    let dx = px - sx;
    let dz = pz - sz;
    if dx * dx + dz * dz < 1e-6 {
        return fallback;
    }
    dx.atan2(-dz)
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_system() -> BlasterSystem {
        BlasterSystem::new(BlasterBankConfig {
            id: "fore".to_string(),
            facing_deg: 0.0,
            fire_arc_deg: 90.0,
            volley_count: 3,
            volley_interval_secs: 0.1,
            cooldown_secs: 2.0,
            charge_time_secs: 0.0,
            projectile_speed: 40.0,
            collision_radius: 1.5,
            visual_scale: 1.0,
            damage: 20,
            shield_pierce: 0.0,
            recoil_impulse: 0.0,
            screenshake_magnitude: 0.0,
            marker: None,
            range: 35.0,
        })
    }

    fn no_target() -> (f32, f32, f32, f32) {
        (100.0, 0.0, 100.0, 0.0)
    }

    fn tick_system(sys: &mut BlasterSystem, dt: f32, uuids: &mut Vec<String>) -> Vec<LaunchEvent> {
        let (tx, tz, tvx, tvz) = no_target();
        let mut idx = 0usize;
        let events = sys.tick(
            dt,
            0.0,
            0.0,
            0.0,
            tx,
            tz,
            tvx,
            tvz,
            "shooter-uuid",
            &mut || {
                idx += 1;
                let id = format!("proj-{}", uuids.len() + idx);
                id
            },
        );
        for e in &events {
            uuids.push(e.projectile_id.clone());
        }
        events
    }

    #[test]
    fn fire_ready_initially() {
        let sys = make_system();
        assert!(sys.is_fire_ready());
    }

    #[test]
    fn request_fire_starts_volley() {
        let mut sys = make_system();
        assert!(sys.request_fire());
        assert_eq!(sys.volley.pending_volley, 3);
        // Can't fire again while volley pending.
        assert!(!sys.request_fire());
    }

    #[test]
    fn volley_launches_correct_count() {
        let mut sys = make_system();
        sys.request_fire();

        let mut uuids = Vec::new();
        // First shot fires immediately (volley_timer starts at 0).
        let e1 = tick_system(&mut sys, 0.001, &mut uuids);
        assert_eq!(e1.len(), 1, "first tick should launch 1 projectile");

        // Before interval expires, no more shots.
        let e2 = tick_system(&mut sys, 0.05, &mut uuids);
        assert_eq!(e2.len(), 0);

        // After interval, second shot.
        let e3 = tick_system(&mut sys, 0.1, &mut uuids);
        assert_eq!(e3.len(), 1);

        // After interval, third (final) shot.
        let e4 = tick_system(&mut sys, 0.1, &mut uuids);
        assert_eq!(e4.len(), 1);

        assert_eq!(
            uuids.len(),
            3,
            "three projectiles should have been launched"
        );
        // After volley, enters cooldown.
        assert!(sys.volley.on_cooldown);
        assert!(!sys.is_fire_ready());
    }

    #[test]
    fn cooldown_expires_and_ready_again() {
        let mut sys = make_system();
        sys.request_fire();

        let mut uuids = Vec::new();
        // Fire all 3 shots (3 ticks of 0.15 each — enough to clear 0.1 interval).
        for _ in 0..3 {
            tick_system(&mut sys, 0.15, &mut uuids);
        }
        assert!(sys.volley.on_cooldown);

        // Tick through the 2 s cooldown.
        tick_system(&mut sys, 2.1, &mut uuids);
        assert!(!sys.volley.on_cooldown);
        assert!(sys.is_fire_ready());
    }

    #[test]
    fn projectile_flies_straight() {
        let mut sys = make_system();
        sys.request_fire();
        let mut uuids = Vec::new();
        tick_system(&mut sys, 0.001, &mut uuids);

        let proj = &sys.in_flight[0];
        let heading = proj.heading;
        let x0 = proj.x;
        let z0 = proj.z;

        // One second of travel.
        let dt = 1.0;
        let expected_x = x0 + heading.sin() * proj.speed * dt;
        let expected_z = z0 - heading.cos() * proj.speed * dt;

        let mut p = proj.clone();
        p.tick(dt);
        assert!((p.x - expected_x).abs() < 1e-4);
        assert!((p.z - expected_z).abs() < 1e-4);
    }

    #[test]
    fn find_hits_returns_matching_target() {
        let mut sys = make_system();
        sys.request_fire();
        let mut uuids = Vec::new();
        tick_system(&mut sys, 0.001, &mut uuids);

        // Move the projectile directly onto the target.
        let proj = sys.in_flight.first_mut().unwrap();
        proj.x = 10.0;
        proj.z = 5.0;

        let targets = vec![("other-entity".to_string(), 10.0_f32, 5.0_f32, 1.0_f32)];
        let hits = sys.find_hits(&targets);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].1, "other-entity");
    }

    #[test]
    fn find_hits_excludes_source_uuid() {
        let mut sys = make_system();
        sys.request_fire();
        let mut uuids = Vec::new();
        tick_system(&mut sys, 0.001, &mut uuids);

        // Projectile at origin; source ship is also at origin.
        let targets = vec![("shooter-uuid".to_string(), 0.0_f32, 0.0_f32, 100.0_f32)];
        let hits = sys.find_hits(&targets);
        assert_eq!(hits.len(), 0, "should not self-hit");
    }

    #[test]
    fn consume_hit_removes_projectile() {
        let mut sys = make_system();
        sys.request_fire();
        let mut uuids = Vec::new();
        tick_system(&mut sys, 0.001, &mut uuids);

        let pid = sys.in_flight[0].id.clone();
        let hit = sys.consume_hit(&pid);
        assert!(hit.is_some());
        assert!(sys.in_flight.is_empty());
    }

    #[test]
    fn projectile_expires_after_lifespan() {
        let mut sys = make_system();
        // speed=40, range=35 → lifespan=0.875 s
        sys.request_fire();
        let mut uuids = Vec::new();
        tick_system(&mut sys, 0.001, &mut uuids);

        let proj = sys.in_flight.first_mut().unwrap();
        proj.tick(1.0); // well past lifespan
        assert!(proj.is_expired());
    }

    #[test]
    fn predict_intercept_heading_stationary_target() {
        // Target directly ahead (+Z direction from shooter at origin with yaw=0
        // means target is at (0, 10) in XZ. Ship forward = -Z so "ahead" in
        // world space is z < shooter_z, i.e. z = -10).
        let h = predict_intercept_heading(0.0, 0.0, 0.0, -10.0, 0.0, 0.0, 40.0, 0.0, 0.0);
        // atan2(0, -(-10)) = atan2(0, 10) = 0 → straight ahead.
        assert!(
            (h - 0.0).abs() < 0.01,
            "heading should be ~0 for target ahead, got {h}"
        );
    }

    #[test]
    fn predict_intercept_heading_leads_moving_target() {
        // Target moving left-to-right across the shooter's field of view at
        // 5 units/s. Shooter at (0, 0), projectile speed 40 units/s, target
        // at (20, -20) at the moment of firing, moving in +X.
        // Distance = sqrt(20² + 20²) ≈ 28.28, t_est ≈ 28.28 / 40 ≈ 0.707s
        // Predicted X = 20 + 5 * 0.707 ≈ 23.54
        // The heading should point slightly ahead of the target's current
        // position — i.e. a heading angle larger than straight-at.
        let straight_at = (20.0_f32).atan2(-(-20.0_f32)); // atan2(dx, -dz)
        let h = predict_intercept_heading(0.0, 0.0, 20.0, -20.0, 5.0, 0.0, 40.0, 0.0, 0.0);
        assert!(
            (h - straight_at).abs() > 0.01,
            "heading ({h}) must differ from straight-at ({straight_at}) when target moves"
        );
        // With +X velocity, the intercept heading should be > straight_at
        // (more clockwise in the atan2(dx, -dz) convention).
        assert!(
            h > straight_at,
            "heading ({h}) must lead rightward-moving target (>{straight_at})"
        );
    }

    #[test]
    fn predict_intercept_heading_zero_speed_falls_back() {
        // When speed is 0 or negative, must return the facing direction
        // (shooter_yaw + facing_deg) irrespective of target velocity.
        let h = predict_intercept_heading(0.0, 0.0, 10.0, -10.0, 100.0, 0.0, 0.0, 0.0, 0.0);
        assert!(
            (h - 0.0).abs() < 0.01,
            "heading should be fallback ~0 when speed=0, got {h}"
        );
    }

    #[test]
    fn bank_state_reflects_current_state() {
        let sys = make_system();
        let state = sys.bank_state(0.0, 0.0, 0.0, None, true);
        assert!(state.fire_ready);
        assert!(!state.on_cooldown);
        assert_eq!(state.pending_volley, 0);
    }

    // ── Readiness contract (issue #764) ──────────────────────────────────────
    use crate::core::messages::WeaponBlockReason;

    /// Bank in range + arc, off cooldown, online → Ready with populated geometry.
    #[test]
    fn bank_state_ready_when_in_range_and_arc() {
        // fore bank, facing 0°, 90° arc, range 35. Target straight ahead at -Z,
        // 10 units away (ship forward is -Z at yaw 0).
        let sys = make_system();
        let state = sys.bank_state(0.0, 0.0, 0.0, Some((0.0, -10.0)), true);
        assert!(state.readiness.ready);
        assert_eq!(state.readiness.blocking_reason, WeaponBlockReason::Ready);
        let range = state.readiness.target_range.expect("range present");
        assert!((range - 10.0).abs() < 0.01, "range {range}");
        let arc = state.readiness.target_arc.expect("arc present");
        assert!(arc.abs() < 0.01, "arc offset {arc}");
    }

    #[test]
    fn bank_state_no_target_blocks() {
        let sys = make_system();
        let state = sys.bank_state(0.0, 0.0, 0.0, None, true);
        assert!(!state.readiness.ready);
        assert_eq!(state.readiness.blocking_reason, WeaponBlockReason::NoTarget);
        assert!(state.readiness.target_range.is_none());
    }

    #[test]
    fn bank_state_out_of_range_blocks() {
        // Target dead ahead but 100 units away — bank range is 35.
        let sys = make_system();
        let state = sys.bank_state(0.0, 0.0, 0.0, Some((0.0, -100.0)), true);
        assert_eq!(
            state.readiness.blocking_reason,
            WeaponBlockReason::OutOfRange
        );
        assert!(!state.readiness.ready);
    }

    #[test]
    fn bank_state_out_of_arc_blocks() {
        // Target in range (10 units) but directly behind (+Z) — outside the
        // fore bank's 90° arc centred on 0°.
        let sys = make_system();
        let state = sys.bank_state(0.0, 0.0, 0.0, Some((0.0, 10.0)), true);
        assert_eq!(state.readiness.blocking_reason, WeaponBlockReason::OutOfArc);
        assert!(!state.readiness.ready);
    }

    #[test]
    fn bank_state_offline_blocks_even_with_valid_target() {
        let sys = make_system();
        let state = sys.bank_state(0.0, 0.0, 0.0, Some((0.0, -10.0)), false);
        assert_eq!(state.readiness.blocking_reason, WeaponBlockReason::Offline);
        assert!(!state.readiness.ready);
        // Geometry is still populated while offline.
        assert!(state.readiness.target_range.is_some());
    }

    #[test]
    fn bank_state_cooldown_blocks() {
        let mut sys = make_system();
        sys.volley.on_cooldown = true;
        sys.volley.cooldown_remaining = 1.5;
        let state = sys.bank_state(0.0, 0.0, 0.0, Some((0.0, -10.0)), true);
        assert_eq!(state.readiness.blocking_reason, WeaponBlockReason::Cooldown);
    }

    #[test]
    fn bank_state_charging_reports_loading() {
        let mut sys = make_charging_system();
        assert!(sys.request_charge_start());
        let state = sys.bank_state(0.0, 0.0, 0.0, Some((0.0, -10.0)), true);
        assert_eq!(state.readiness.blocking_reason, WeaponBlockReason::Loading);
    }

    #[test]
    fn request_fire_zero_volley_count_returns_false() {
        let mut sys = BlasterSystem::new(BlasterBankConfig {
            volley_count: 0,
            ..BlasterBankConfig::default()
        });
        assert!(
            !sys.request_fire(),
            "request_fire must return false when volley_count == 0"
        );
        assert_eq!(sys.volley.pending_volley, 0, "pending_volley must remain 0");
    }

    // ── Charge mechanic tests (issue #636) ───────────────────────────────────

    fn make_charging_system() -> BlasterSystem {
        BlasterSystem::new(BlasterBankConfig {
            id: "heavy".to_string(),
            charge_time_secs: 2.0,
            volley_count: 2,
            volley_interval_secs: 0.1,
            cooldown_secs: 3.0,
            ..BlasterBankConfig::default()
        })
    }

    #[test]
    fn charge_start_on_instant_fire_bank_delegates_to_request_fire() {
        // charge_time_secs == 0 → request_charge_start behaves as request_fire.
        let mut sys = make_system();
        assert!(sys.request_charge_start());
        // Volley should be armed immediately, no charging state.
        assert!(
            !sys.volley.charging,
            "instant-fire bank must not set charging"
        );
        assert_eq!(
            sys.volley.pending_volley, 3,
            "instant-fire bank must arm volley immediately"
        );
    }

    #[test]
    fn charge_start_sets_charging_flag_for_charge_bank() {
        let mut sys = make_charging_system();
        assert!(sys.request_charge_start());
        assert!(sys.volley.charging);
        assert_eq!(sys.volley.charge_elapsed, 0.0);
        assert_eq!(
            sys.volley.pending_volley, 0,
            "volley must not be armed until charge completes"
        );
    }

    #[test]
    fn is_fire_ready_false_while_charging() {
        let mut sys = make_charging_system();
        sys.request_charge_start();
        assert!(
            !sys.is_fire_ready(),
            "is_fire_ready must be false while charging"
        );
    }

    #[test]
    fn charge_start_rejected_when_already_charging() {
        let mut sys = make_charging_system();
        assert!(sys.request_charge_start());
        assert!(
            !sys.request_charge_start(),
            "second charge_start while charging must be rejected"
        );
    }

    #[test]
    fn charge_cancel_resets_state_with_no_cooldown() {
        let mut sys = make_charging_system();
        sys.request_charge_start();
        // Tick a bit so elapsed advances.
        let mut uuids = Vec::new();
        tick_system(&mut sys, 0.5, &mut uuids);
        assert!(sys.volley.charging);

        sys.request_charge_cancel();
        assert!(!sys.volley.charging);
        assert_eq!(sys.volley.charge_elapsed, 0.0);
        assert!(!sys.volley.on_cooldown, "cancel must not start cooldown");
        assert_eq!(sys.volley.pending_volley, 0, "cancel must not arm a volley");
        assert!(sys.is_fire_ready(), "bank must be ready again after cancel");
    }

    #[test]
    fn charge_cancel_is_noop_when_not_charging() {
        let mut sys = make_charging_system();
        // Calling cancel when idle must not corrupt state.
        sys.request_charge_cancel();
        assert!(!sys.volley.charging);
        assert!(sys.is_fire_ready());
    }

    #[test]
    fn charge_progress_zero_for_instant_fire_bank() {
        let sys = make_system(); // charge_time_secs == 0
        assert_eq!(
            sys.charge_progress(),
            0.0,
            "instant-fire banks must always report 0 charge_progress"
        );
    }

    #[test]
    fn charge_progress_increases_while_charging() {
        let mut sys = make_charging_system();
        sys.request_charge_start();
        let mut uuids = Vec::new();
        tick_system(&mut sys, 1.0, &mut uuids); // halfway through 2-second charge
        let progress = sys.charge_progress();
        assert!(
            (progress - 0.5).abs() < 0.01,
            "charge_progress must be ~0.5 after 1s of 2s charge, got {progress}"
        );
    }

    #[test]
    fn charge_progress_clamped_at_one() {
        let mut sys = make_charging_system();
        // Manually push elapsed past charge_time_secs.
        sys.volley.charging = true;
        sys.volley.charge_elapsed = 10.0;
        assert_eq!(
            sys.charge_progress(),
            1.0,
            "charge_progress must be clamped to 1.0"
        );
    }

    #[test]
    fn charge_completes_and_fires_volley() {
        let mut sys = make_charging_system();
        sys.request_charge_start();

        let mut uuids = Vec::new();
        // Tick just under charge time — still charging, no projectile.
        let early = tick_system(&mut sys, 1.9, &mut uuids);
        assert_eq!(early.len(), 0, "no projectile before charge completes");
        assert!(sys.volley.charging);

        // Tick past the charge threshold — charge completes, volley begins.
        let completed = tick_system(&mut sys, 0.2, &mut uuids);
        assert_eq!(
            completed.len(),
            1,
            "one projectile on the tick charge completes"
        );
        assert!(
            !sys.volley.charging,
            "charging must be cleared after completion"
        );
    }

    #[test]
    fn no_projectile_while_charging() {
        let mut sys = make_charging_system();
        sys.request_charge_start();

        let mut uuids = Vec::new();
        // Multiple ticks, all within charge window.
        for _ in 0..5 {
            let events = tick_system(&mut sys, 0.3, &mut uuids);
            assert_eq!(events.len(), 0, "must not fire during charge phase");
        }
    }

    #[test]
    fn bank_state_includes_charge_progress_and_has_charge() {
        let sys = make_charging_system();
        let state = sys.bank_state(0.0, 0.0, 0.0, None, true);
        assert_eq!(state.charge_progress, 0.0);
        assert!(
            state.has_charge,
            "has_charge must be true when charge_time_secs > 0"
        );

        let instant = make_system();
        let istate = instant.bank_state(0.0, 0.0, 0.0, None, true);
        assert_eq!(istate.charge_progress, 0.0);
        assert!(
            !istate.has_charge,
            "has_charge must be false for instant-fire bank"
        );
    }
}
