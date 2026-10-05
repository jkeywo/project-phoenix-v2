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
//!   - target velocity: `v` = (tvx, tvz)
//!   - shooter position: (sx, sz), projectile speed `c`
//!   - relative position `r` = (tx − sx, tz − sz)
//!
//! the intercept time is the *exact* solution of
//!
//! ```text
//!     |r + v·t| = c·t
//! ```
//!
//! which expands to the quadratic
//!
//! ```text
//!     (|v|² − c²)·t²  +  2(r·v)·t  +  |r|²  =  0
//! ```
//!
//! [`solve_intercept_time`] takes its smallest positive real root; the
//! predicted position is `(tx + tvx·t, tz + tvz·t)`. The projectile is oriented
//! toward that point and flies straight — no mid-flight correction.
//!
//! This is a true lead solution, *not* the first-order estimate `t ≈ |r| / c`
//! that measures flight time to where the target is standing rather than to
//! where it will be. The first-order form systematically under-leads any
//! crossing target (it is short by a factor of `cos(asin(|v|/c))` at square-on
//! crossing), and the error grows with both range and crossing speed. It
//! survives only as the degenerate fallback below.

use crate::simmath;
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
    /// Optional rig-marker name for mount point resolution. In the
    /// single-barrel (backward-compat) case this is the sole projectile origin.
    pub marker: Option<String>,
    /// Authored barrel-marker names (issue #765). Empty ⇒ one implicit barrel
    /// = `marker`. A [`pattern`](Self::pattern) step addresses these by index.
    pub barrels: Vec<String>,
    /// Timed multi-barrel firing pattern (issue #765). Empty ⇒ the uniform
    /// `volley_count` volley from the single implicit barrel (unchanged).
    pub pattern: crate::weapons::pattern::BarrelPattern,
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
            barrels: Vec::new(),
            pattern: Vec::new(),
            range: 35.0,
        }
    }
}

impl BlasterBankConfig {
    /// Effective barrel count: the authored barrel-marker count, or `1` (the
    /// implicit single barrel = `marker`) when none are authored.
    pub fn barrel_count(&self) -> usize {
        if self.barrels.is_empty() {
            1
        } else {
            self.barrels.len()
        }
    }

    /// Resolve the firing schedule for one volley as an ordered list of
    /// `(barrels, at_secs)` steps, sorted by `at_secs`.
    ///
    /// When a [`pattern`](Self::pattern) is authored it drives the schedule
    /// verbatim (a step fires its barrels simultaneously; successive steps at
    /// increasing offsets alternate). When no pattern is authored the bank
    /// falls back to the uniform volley: `volley_count` shots of barrel `0`
    /// spaced `volley_interval_secs` apart — exactly the pre-#765 behaviour.
    pub fn firing_schedule(&self) -> Vec<(Vec<u32>, f32)> {
        if !self.pattern.is_empty() {
            let mut steps: Vec<(Vec<u32>, f32)> = self
                .pattern
                .iter()
                .map(|s| (s.barrels.clone(), s.offset_secs.max(0.0)))
                .collect();
            steps.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
            steps
        } else {
            (0..self.volley_count)
                .map(|i| (vec![0u32], i as f32 * self.volley_interval_secs))
                .collect()
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
        self.x += simmath::sin(self.heading) * self.speed * dt;
        self.z -= simmath::cos(self.heading) * self.speed * dt;
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
    /// How many pattern steps remain to be fired in the current volley. Named
    /// `pending_volley` for wire/back-compat: in a single-barrel bank one step
    /// == one projectile, so this still counts projectiles remaining.
    pub pending_volley: u32,
    /// The resolved firing schedule for the active volley: an ordered list of
    /// `(barrels, at_secs)` steps (issue #765). Empty when idle.
    pub schedule: Vec<(Vec<u32>, f32)>,
    /// Index of the next step in `schedule` to fire.
    pub next_step: usize,
    /// Seconds elapsed since the current volley (schedule) began.
    pub volley_elapsed: f32,
    /// Barrel indices that fired on the most recently emitted step (issue #765).
    /// Empty when idle. Surfaced on the blackboard for the Tactical indicator.
    pub active_barrels: Vec<u32>,
    /// 1-based index of the most recently fired step (0 when none fired yet).
    pub current_step: u32,
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
        // An empty schedule means nothing would fire (volley_count == 0 with no
        // pattern authored). A pattern-driven bank is armed even when
        // volley_count is 0, because the pattern — not volley_count — decides
        // what fires.
        if self.config.firing_schedule().is_empty() {
            return false;
        }
        if !self.is_fire_ready() {
            return false;
        }
        self.arm_volley();
        true
    }

    /// Arm a fresh volley from the config's firing schedule (issue #765).
    /// Resets the schedule cursor and elapsed timer so the first due step fires
    /// on the next `tick`. Shared by `request_fire` and charge completion.
    fn arm_volley(&mut self) {
        let schedule = self.config.firing_schedule();
        self.volley.pending_volley = schedule.len() as u32;
        self.volley.schedule = schedule;
        self.volley.next_step = 0;
        self.volley.volley_elapsed = 0.0;
        self.volley.active_barrels.clear();
        self.volley.current_step = 0;
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

    /// Advance the volley by `dt` seconds against the resolved firing schedule
    /// (issue #765).
    ///
    /// `barrel_origins` supplies one world-XZ position per barrel index — the
    /// caller resolves these from the bank's authored barrel markers (or the
    /// single `marker` in the backward-compat case). Every pattern step whose
    /// `at_secs` has been reached this tick fires: for each barrel index in the
    /// step a projectile is launched from `barrel_origins[index]`, producing
    /// ONE [`LaunchEvent`] per barrel. A step with several barrels fires them
    /// simultaneously; successive steps alternate. When the schedule is
    /// exhausted the post-volley cooldown starts. Expired projectiles are
    /// pruned here.
    ///
    /// The caller is responsible for emitting server messages from the returned
    /// events (each already carries its resolved origin and barrel index).
    pub fn tick(
        &mut self,
        dt: f32,
        barrel_origins: &[(f32, f32)],
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
                // Charge complete — arm the volley from the firing schedule.
                self.volley.charging = false;
                self.volley.charge_elapsed = 0.0;
                self.arm_volley();
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

        // Advance the volley clock and fire the next step if it is now due.
        //
        // At most ONE step fires per tick — this preserves the pre-#765
        // one-projectile-per-tick cadence of the uniform volley exactly (a
        // coarse `dt` spanning several offsets never batches successive steps
        // into a single tick). A single step still emits one event PER barrel,
        // so a simultaneous multi-barrel step fires together on one tick.
        self.volley.volley_elapsed += dt;
        let lifespan = self.config.range / self.config.projectile_speed;
        let mut events = Vec::new();
        let mut fired_barrels: Vec<u32> = Vec::new();

        if self.volley.next_step < self.volley.schedule.len()
            && self.volley.schedule[self.volley.next_step].1 <= self.volley.volley_elapsed
        {
            let barrels = self.volley.schedule[self.volley.next_step].0.clone();
            for &barrel in &barrels {
                // Origin resolves per barrel; falls back to the first supplied
                // origin (then the world origin) so a mis-sized origin slice
                // still fires from a plausible point rather than panicking.
                let (origin_x, origin_z) = barrel_origins
                    .get(barrel as usize)
                    .copied()
                    .or_else(|| barrel_origins.first().copied())
                    .unwrap_or((0.0, 0.0));
                let uuid = next_uuid();
                let heading = predict_intercept_heading(
                    origin_x,
                    origin_z,
                    target_x,
                    target_z,
                    target_vx,
                    target_vz,
                    self.config.projectile_speed,
                    shooter_yaw,
                    self.config.facing_deg,
                );
                self.in_flight.push(BlasterProjectile {
                    id: uuid.clone(),
                    x: origin_x,
                    z: origin_z,
                    heading,
                    speed: self.config.projectile_speed,
                    lifespan_remaining: lifespan,
                    collision_radius: self.config.collision_radius,
                    damage: self.config.damage,
                    shield_pierce: self.config.shield_pierce,
                    source_uuid: source_uuid.to_string(),
                });
                events.push(LaunchEvent {
                    projectile_id: uuid,
                    x: origin_x,
                    z: origin_z,
                    heading,
                    barrel,
                });
            }
            fired_barrels = barrels;
            self.volley.next_step += 1;
            self.volley.current_step = self.volley.next_step as u32;
            self.volley.pending_volley = self.volley.pending_volley.saturating_sub(1);
        }

        if !fired_barrels.is_empty() {
            self.volley.active_barrels = fired_barrels;
        }

        // Volley complete — start cooldown and clear the active-step indicator.
        if self.volley.pending_volley == 0 && self.volley.next_step >= self.volley.schedule.len() {
            self.volley.on_cooldown = true;
            self.volley.cooldown_remaining = self.config.cooldown_secs;
            self.volley.schedule.clear();
            self.volley.next_step = 0;
            self.volley.volley_elapsed = 0.0;
            self.volley.active_barrels.clear();
            self.volley.current_step = 0;
        }

        events
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
            active_barrels: self.volley.active_barrels.clone(),
            pattern_step: self.volley.current_step,
            // Only a genuine multi-barrel pattern reports a length; the uniform
            // single-barrel volley leaves this 0 so the client shows no
            // step indicator for legacy banks.
            pattern_len: if self.config.pattern.is_empty() {
                0
            } else {
                self.config.pattern.len() as u32
            },
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
    /// Zero-based barrel index this bolt left from (issue #765). `0` for a
    /// single-barrel/backward-compat bank. Lets telemetry and the caller
    /// attribute the shot to a specific authored barrel marker.
    pub barrel: u32,
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

/// Squared distance below which two XZ points are treated as coincident.
///
/// Not a gameplay value: a float-precision guard on a direction that is about
/// to be normalised by `atan2`, in the same units the callers already use.
const COINCIDENT_EPS_SQ: f32 = 1e-6;

/// Solve the intercept time for a projectile of speed `speed` fired from the
/// origin of `(rel_x, rel_z)` at a target moving at constant `(tvx, tvz)`.
///
/// `rel` is the target's position **relative to the shooter**. Returns the
/// smallest strictly-positive real `t` satisfying
///
/// ```text
///     |rel + v·t| = speed·t
/// ```
///
/// i.e. the first moment the projectile and the target can occupy the same
/// point, or `None` when no such moment exists.
///
/// Expanding the square gives `a·t² + b·t + c = 0` with
///
/// ```text
///     a = |v|² − speed²        b = 2·(rel · v)        c = |rel|²
/// ```
///
/// `c` is a squared length and therefore never negative, which is what makes
/// the usual case unambiguous: when the target is slower than the projectile
/// `a < 0`, the product of the roots `c/a` is ≤ 0, and exactly one root is
/// positive. A target *faster* than the projectile can still be intercepted
/// (head-on, or on a converging course) and then both roots may be positive —
/// the earlier one is the shot that connects, so the smallest positive root is
/// the right answer in every case rather than just the common one.
///
/// Degenerate cases, all of them reachable from live gameplay:
///
/// * **Target already on the shooter** (`|rel| ≈ 0`) — the intercept is *now*;
///   returns `Some(0.0)` rather than dividing through a zero-length vector.
/// * **`a ≈ 0`** — the target moves at exactly the projectile's speed, and the
///   quadratic collapses to the linear `b·t + c = 0`. Solved as a line, never
///   divided by the vanishing `a`. Its root `−c/b` is positive only while
///   `b < 0` (the target is closing); a target running alongside or away at
///   exactly the projectile's speed can never be caught, and yields `None`.
/// * **Negative discriminant** — the target outruns the projectile on this
///   geometry. `None`; the caller decides how to degrade.
/// * **No positive root** — every solution is in the past. `None`.
/// * **Zero or negative `speed`** — nothing was fired. `None`.
///
/// Pure and Bevy-free: used by the launch path (`BlasterSystem::tick`) and by
/// the AI's artillery bow facing (`crate::ai::plan_artillery_position`), which
/// must agree with it exactly or the gun and the bow point different ways.
pub fn solve_intercept_time(rel_x: f32, rel_z: f32, tvx: f32, tvz: f32, speed: f32) -> Option<f32> {
    if speed <= 0.0 {
        return None;
    }

    let c = rel_x * rel_x + rel_z * rel_z;
    if c <= COINCIDENT_EPS_SQ {
        // The target is sitting on the shooter: the intercept is this instant.
        return Some(0.0);
    }

    let a = tvx * tvx + tvz * tvz - speed * speed;
    let b = 2.0 * (rel_x * tvx + rel_z * tvz);

    // `a` vanishes when the target's speed equals the projectile's. The
    // threshold is scaled by `speed²` — `a`'s own units — so it means the same
    // thing at any scale, rather than "small in world units", which would
    // swallow whole solutions for a slow projectile.
    //
    // What it buys is exactly one thing: nothing divides by a vanishing `a`. It
    // does NOT make the quadratic branch exact just outside it. `a` is itself
    // `|v|² − speed²`, a difference of near-equal squares, so a hair outside the
    // threshold it carries barely any significant figures and the root computed
    // from it is wrong by orders of magnitude. That is deliberately left alone:
    // every `t` in that band is tens of thousands of seconds, four orders past
    // the ~5.7 s a shipped bolt lives, so the projectile has expired long before
    // the imprecision could reach anything observable.
    if a.abs() <= 1e-6 * speed * speed {
        if b >= 0.0 {
            // Not closing: a target matching the projectile's speed and not
            // coming toward it is never caught.
            return None;
        }
        let t = -c / b;
        return if t > 0.0 && t.is_finite() {
            Some(t)
        } else {
            None
        };
    }

    let disc = b * b - 4.0 * a * c;
    if disc < 0.0 {
        return None;
    }
    let root = disc.sqrt();

    // Numerically stable roots. The textbook `(-b ± √D) / 2a` cancels
    // catastrophically for the SMALLER root when `√D ≈ |b|`, i.e. when `|4ac|`
    // is small next to `b²`. Computing one root through `q` — where `b` and
    // `√D` are added with matching signs — and the other as `c/q` keeps both to
    // full precision.
    //
    // Which geometry is that? NOT a long-range one: with `c = |r|²` and
    // `b = 2(r·v)`,
    //
    //     4ac / b²  =  (|v|² − speed²) / (|v|²·cos²φ)
    //
    // where `φ` is the angle between `r` and `v`. `|r|` cancels out entirely, so
    // the ratio is scale-invariant — it reads the same at 180 units and at 2000.
    // Range has nothing to do with it.
    //
    // In the `a < 0` regime — the target slower than the bolt, which is every
    // case the game currently ships — the ratio is negative and its magnitude is
    // at least `speed²/|v|² − 1`. It only approaches zero as `|v|` approaches
    // `speed`, and the `a ≈ 0` branch above has already taken that over. So no
    // shipped shot can cancel here, and the stable form costs nothing.
    //
    // The regime that CAN cancel is `a > 0`: a target moving faster than the
    // bolt, closing, whose speed only just exceeds it, driving `4ac/b²` toward
    // zero from above. Both roots are positive there, and the smaller one — the
    // shot that actually connects — is precisely the one that would lose its
    // digits. Boost pushes hulls past 35 u/s, so this is reachable rather than
    // theoretical, which is why the stable form is worth keeping.
    let q = -0.5 * (b + if b < 0.0 { -root } else { root });
    let t1 = q / a;
    let t2 = if q != 0.0 { c / q } else { f32::INFINITY };

    let mut best = f32::INFINITY;
    if t1 > 0.0 && t1 < best {
        best = t1;
    }
    if t2 > 0.0 && t2 < best {
        best = t2;
    }
    if best.is_finite() {
        Some(best)
    } else {
        None
    }
}

/// Compute the launch heading toward the predicted intercept position.
///
/// Solves the closed-form intercept ([`solve_intercept_time`]) and aims at
/// `(tx + tvx·t, tz + tvz·t)` — the point the target and the bolt actually
/// share, not the point the target is standing on now.
///
/// Two fallbacks, and they are deliberately different:
///
/// * **No intercept exists** (the target outruns the bolt on this geometry) —
///   degrades to the first-order estimate `t = |rel| / speed`. It is not a
///   solution to anything, but it still points *ahead* of a fleeing target
///   along its course, which is a better last word than the bank's rest
///   facing. Never a `NaN`: `solve_intercept_time` returns `None` rather than
///   a root of a negative discriminant.
/// * **Nothing was fired** (`speed <= 0`), or the predicted point coincides
///   with the shooter — falls back to the bank's facing direction relative to
///   `shooter_yaw`. There is no meaningful direction to derive in either case.
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

    let rel_x = tx - sx;
    let rel_z = tz - sz;

    let t = solve_intercept_time(rel_x, rel_z, tvx, tvz, speed)
        .unwrap_or_else(|| (rel_x * rel_x + rel_z * rel_z).sqrt() / speed);

    let px = tx + tvx * t;
    let pz = tz + tvz * t;

    let dx = px - sx;
    let dz = pz - sz;
    if dx * dx + dz * dz < COINCIDENT_EPS_SQ {
        return fallback;
    }
    simmath::atan2(dx, -dz)
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "blaster_tests.rs"]
mod tests;
