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
            barrels: Vec::new(),
            pattern: Vec::new(),
            range: 35.0,
        })
    }

    fn no_target() -> (f32, f32, f32, f32) {
        (100.0, 0.0, 100.0, 0.0)
    }

    fn tick_system(sys: &mut BlasterSystem, dt: f32, uuids: &mut Vec<String>) -> Vec<LaunchEvent> {
        // Enough origins for any barrel index the tests use; a single-barrel
        // bank only ever reads index 0.
        let origins = [(0.0, 0.0), (0.0, 0.0), (0.0, 0.0), (0.0, 0.0)];
        let (tx, tz, tvx, tvz) = no_target();
        let mut idx = 0usize;
        let events = sys.tick(
            dt,
            &origins,
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
        // First shot fires immediately (schedule step 0 is at offset 0).
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

    // ── Patterned multi-barrel attacks (issue #765) ─────────────────────────

    use crate::weapons::pattern::BarrelPatternStep;

    fn step(barrels: &[u32], offset: f32) -> BarrelPatternStep {
        BarrelPatternStep {
            barrels: barrels.to_vec(),
            offset_secs: offset,
        }
    }

    fn make_pattern_system(barrels: Vec<String>, pattern: Vec<BarrelPatternStep>) -> BlasterSystem {
        BlasterSystem::new(BlasterBankConfig {
            id: "multi".to_string(),
            volley_interval_secs: 0.1,
            cooldown_secs: 2.0,
            projectile_speed: 40.0,
            range: 35.0,
            barrels,
            pattern,
            ..BlasterBankConfig::default()
        })
    }

    /// Tick with per-barrel origins so the emitted events prove which barrel
    /// fired (by its distinct world X).
    fn tick_with_origins(
        sys: &mut BlasterSystem,
        dt: f32,
        origins: &[(f32, f32)],
    ) -> Vec<LaunchEvent> {
        let mut idx = 0usize;
        sys.tick(
            dt,
            origins,
            0.0,
            100.0,
            0.0,
            0.0,
            0.0,
            "shooter",
            &mut || {
                idx += 1;
                format!("p-{idx}")
            },
        )
    }

    #[test]
    fn alternating_pattern_fires_barrels_in_sequence() {
        // Two barrels, two steps at increasing offsets → alternating fire.
        let sys_barrels = vec!["b0".to_string(), "b1".to_string()];
        let pattern = vec![step(&[0], 0.0), step(&[1], 0.2)];
        let mut sys = make_pattern_system(sys_barrels, pattern);
        assert!(sys.request_fire());
        assert_eq!(sys.volley.pending_volley, 2, "two scheduled steps");

        // Distinct origins per barrel so the event's X identifies the barrel.
        let origins = [(10.0, 0.0), (20.0, 0.0)];

        // First tick fires only barrel 0 (offset 0).
        let e1 = tick_with_origins(&mut sys, 0.01, &origins);
        assert_eq!(e1.len(), 1, "step 0 fires one barrel");
        assert_eq!(e1[0].barrel, 0);
        assert!((e1[0].x - 10.0).abs() < 1e-4, "barrel 0 origin");

        // Not yet at 0.2s → no fire.
        let e2 = tick_with_origins(&mut sys, 0.1, &origins);
        assert_eq!(e2.len(), 0, "barrel 1 not due until 0.2s");

        // Past 0.2s → barrel 1 fires.
        let e3 = tick_with_origins(&mut sys, 0.1, &origins);
        assert_eq!(e3.len(), 1, "step 1 fires one barrel");
        assert_eq!(e3[0].barrel, 1);
        assert!((e3[0].x - 20.0).abs() < 1e-4, "barrel 1 origin");

        // Volley done → cooldown.
        assert!(sys.volley.on_cooldown);
    }

    #[test]
    fn simultaneous_step_fires_multiple_barrels_one_tick() {
        // A single step listing several barrels fires them together on one tick.
        let sys_barrels = vec!["b0".to_string(), "b1".to_string(), "b2".to_string()];
        let pattern = vec![step(&[0, 2], 0.0)];
        let mut sys = make_pattern_system(sys_barrels, pattern);
        assert!(sys.request_fire());

        let origins = [(10.0, 0.0), (20.0, 0.0), (30.0, 0.0)];
        let e1 = tick_with_origins(&mut sys, 0.01, &origins);
        assert_eq!(e1.len(), 2, "two barrels fire simultaneously");
        let mut barrels: Vec<u32> = e1.iter().map(|e| e.barrel).collect();
        barrels.sort();
        assert_eq!(barrels, vec![0, 2]);
        // Distinct origins prove the events come from distinct barrels.
        let xs: Vec<f32> = e1.iter().map(|e| e.x).collect();
        assert!(xs.contains(&10.0) && xs.contains(&30.0), "{xs:?}");
        // Single-step pattern → volley completes immediately.
        assert!(sys.volley.on_cooldown);
    }

    #[test]
    fn backward_compat_single_barrel_unchanged() {
        // No barrels + no pattern → identical to the legacy uniform volley:
        // volley_count shots from the single origin, one per tick.
        let mut sys = make_system(); // volley_count 3, interval 0.1
        assert!(sys.request_fire());
        assert_eq!(sys.volley.pending_volley, 3);
        let origins = [(5.0, 0.0)];

        let e1 = tick_with_origins(&mut sys, 0.001, &origins);
        assert_eq!(e1.len(), 1);
        assert_eq!(e1[0].barrel, 0, "implicit single barrel is index 0");
        assert!((e1[0].x - 5.0).abs() < 1e-4);

        assert_eq!(tick_with_origins(&mut sys, 0.05, &origins).len(), 0);
        assert_eq!(tick_with_origins(&mut sys, 0.1, &origins).len(), 1);
        assert_eq!(tick_with_origins(&mut sys, 0.1, &origins).len(), 1);
        assert!(sys.volley.on_cooldown);
    }

    #[test]
    fn bank_state_reflects_active_pattern() {
        let sys_barrels = vec!["b0".to_string(), "b1".to_string()];
        let pattern = vec![step(&[0], 0.0), step(&[1], 0.2)];
        let mut sys = make_pattern_system(sys_barrels, pattern);
        sys.request_fire();
        let origins = [(10.0, 0.0), (20.0, 0.0)];
        tick_with_origins(&mut sys, 0.01, &origins);

        let state = sys.bank_state(0.0, 0.0, 0.0, Some((0.0, -10.0)), true);
        assert_eq!(state.pattern_len, 2, "two-step pattern");
        assert_eq!(state.pattern_step, 1, "on step 1 after first fire");
        assert_eq!(state.active_barrels, vec![0], "barrel 0 just fired");
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
        let expected_x = x0 + simmath::sin(heading) * proj.speed * dt;
        let expected_z = z0 - simmath::cos(heading) * proj.speed * dt;

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
        let straight_at = simmath::atan2(20.0_f32, -(-20.0_f32)); // atan2(dx, -dz)
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

    // ── The closed-form intercept ───────────────────────────────────────────

    /// The solver is EXACT, on a geometry whose answer is a whole number.
    ///
    /// Asserted on the intercept POINT rather than only on the heading, because
    /// a heading assertion cannot tell "led correctly" from "led in the right
    /// direction": the first-order estimate this replaces also produces a
    /// heading to starboard of a starboard-crossing target, and is wrong.
    #[test]
    fn intercept_solution_is_exact_for_a_hand_computable_geometry() {
        // Shooter at the origin. Target 100 units dead ahead (−Z), crossing to
        // starboard at 30 u/s. Bolt speed 50.
        //
        //   t = d / sqrt(c² − v²) = 100 / sqrt(2500 − 900) = 100 / 40 = 2.5 s
        //
        // so the intercept sits at (75, −100): a 3-4-5 triangle scaled by 25,
        // whose hypotenuse of 125 is exactly 50 × 2.5. Every figure here is a
        // whole number on purpose — a solver that is merely CLOSE fails it.
        let t = solve_intercept_time(0.0, -100.0, 30.0, 0.0, 50.0).expect("an intercept exists");
        assert!((t - 2.5).abs() < 1e-3, "intercept time {t}, expected 2.5");

        let px = 30.0 * t;
        let pz = -100.0_f32;
        assert!((px - 75.0).abs() < 1e-2, "intercept x {px}, expected 75");

        // ...and the bolt really can BE there then: the aim point is exactly
        // `speed × t` from the muzzle, which is the identity the whole solve is
        // for and the one the first-order estimate cannot satisfy.
        let flight = (px * px + pz * pz).sqrt();
        assert!(
            (flight - 125.0).abs() < 1e-2,
            "the intercept must be reachable: {flight} units of flight, expected 125"
        );

        // The launch heading agrees with the point.
        let h = predict_intercept_heading(0.0, 0.0, 0.0, -100.0, 30.0, 0.0, 50.0, 0.0, 0.0);
        let expected = simmath::atan2(px, -pz);
        assert!(
            (h - expected).abs() < 1e-4,
            "heading {h}, expected {expected}"
        );

        // And it leads FURTHER than the estimate it replaces: that one solved
        // t = 100/50 = 2 s and aimed at (60, −100) — fifteen units behind a
        // target flying a perfectly straight line.
        let first_order = 100.0_f32 / 50.0;
        assert!(
            t > first_order,
            "the exact solution ({t} s) must lead further than the first-order \
             estimate ({first_order} s) that under-led every crossing target"
        );
    }

    /// Zero relative velocity must reduce EXACTLY to "aim at the target" — no
    /// residual lead, no special case in the caller.
    #[test]
    fn a_stationary_target_reduces_to_aiming_at_it() {
        let t = solve_intercept_time(0.0, -120.0, 0.0, 0.0, 40.0).expect("an intercept exists");
        assert!((t - 3.0).abs() < 1e-4, "t = d/c = 3, got {t}");

        // Off-axis and off-origin, so this is not passing on a lucky zero.
        let h = predict_intercept_heading(10.0, 10.0, 40.0, -20.0, 0.0, 0.0, 40.0, 0.0, 0.0);
        let straight_at = simmath::atan2(40.0_f32 - 10.0, -(-20.0_f32 - 10.0));
        assert!(
            (h - straight_at).abs() < 1e-5,
            "heading {h} must be the bearing to a stationary target ({straight_at})"
        );
    }

    /// `a ≈ 0`: the target moves at exactly the projectile's speed and the
    /// quadratic degenerates to a line. Solved as a line — never divided by the
    /// vanishing coefficient.
    #[test]
    fn a_target_at_exactly_the_projectile_speed_degenerates_to_a_linear_solve() {
        // Closing head-on: bolt 40 u/s out, target 40 u/s in, 100 units apart.
        // They meet at the midpoint after 100 / 80 = 1.25 s.
        let t = solve_intercept_time(0.0, -100.0, 0.0, 40.0, 40.0)
            .expect("a CLOSING match-speed target is still catchable");
        assert!((t - 1.25).abs() < 1e-3, "intercept time {t}, expected 1.25");
        let meet = -100.0 + 40.0 * t;
        assert!(
            (meet + 50.0).abs() < 1e-2,
            "they must meet at the midpoint, got z = {meet}"
        );

        // Running away at the bolt's own speed is never caught...
        assert!(
            solve_intercept_time(0.0, -100.0, 0.0, -40.0, 40.0).is_none(),
            "a target receding at exactly the projectile's speed has no intercept"
        );
        // ...and neither is one crossing square at it (`b` vanishes too, which
        // is the sub-case that would divide by zero if it were not caught).
        assert!(
            solve_intercept_time(0.0, -100.0, 40.0, 0.0, 40.0).is_none(),
            "a square crosser at exactly the projectile's speed has no intercept"
        );
        // The caller still produces a real heading in both.
        for vx in [40.0_f32, 0.0] {
            let vz = if vx == 0.0 { -40.0 } else { 0.0 };
            let h = predict_intercept_heading(0.0, 0.0, 0.0, -100.0, vx, vz, 40.0, 0.0, 0.0);
            assert!(h.is_finite(), "heading must never be NaN, got {h}");
        }
    }

    /// No real positive root: the target outruns the projectile. The caller
    /// degrades to the first-order estimate — it must not aim at a `NaN`.
    #[test]
    fn a_target_that_outruns_the_projectile_has_no_intercept_and_no_nan() {
        // Crossing square at twice the bolt's speed: negative discriminant.
        assert!(
            solve_intercept_time(0.0, -100.0, 60.0, 0.0, 30.0).is_none(),
            "no intercept exists against a square crosser at twice the bolt speed"
        );

        let h = predict_intercept_heading(0.0, 0.0, 0.0, -100.0, 60.0, 0.0, 30.0, 0.0, 0.0);
        assert!(
            h.is_finite(),
            "aiming at a NaN is the failure mode this case exists to prevent, got {h}"
        );
        // The documented degradation, asserted so it cannot silently become
        // something else: the first-order estimate, t = |rel| / speed.
        let t_est = 100.0_f32 / 30.0;
        let expected = simmath::atan2(60.0 * t_est, 100.0);
        assert!(
            (h - expected).abs() < 1e-4,
            "heading {h} must fall back to the first-order estimate ({expected})"
        );

        // Nothing here keys off "faster" — only off "reachable". The SAME
        // over-speed target on a converging course is still solved exactly:
        // 100 units closing at 30 + 60 = 90 u/s.
        let closing = solve_intercept_time(0.0, -100.0, 0.0, 60.0, 30.0)
            .expect("a head-on target is reachable at any speed");
        assert!(
            (closing - 100.0 / 90.0).abs() < 1e-3,
            "intercept time {closing}, expected {}",
            100.0 / 90.0
        );
    }

    /// The target is sitting on the muzzle: the intercept is *now*, and there is
    /// no direction to derive from a zero-length vector.
    #[test]
    fn a_target_on_the_shooter_intercepts_now_and_falls_back_to_the_bank_facing() {
        assert_eq!(
            solve_intercept_time(0.0, 0.0, 12.0, -5.0, 40.0),
            Some(0.0),
            "a target already on the shooter is intercepted this instant"
        );

        let h = predict_intercept_heading(7.0, -3.0, 7.0, -3.0, 12.0, -5.0, 40.0, 0.4, 30.0);
        let expect = normalise_angle(0.4 + 30.0_f32.to_radians());
        assert!(
            (h - expect).abs() < 1e-5,
            "heading {h} must fall back to the bank facing ({expect})"
        );
    }

    /// Zero (or negative) projectile speed: nothing was fired, so there is
    /// nothing to solve. The heading half is
    /// `predict_intercept_heading_zero_speed_falls_back`.
    #[test]
    fn zero_projectile_speed_has_no_intercept() {
        assert!(solve_intercept_time(0.0, -100.0, 5.0, 0.0, 0.0).is_none());
        assert!(solve_intercept_time(0.0, -100.0, 5.0, 0.0, -3.0).is_none());
    }

    /// Issue #792, AC4/AC7: the battleship's artillery bolt is aimed ONCE, at
    /// launch, and a target that changes course afterwards evades it.
    ///
    /// Driven off the SHIPPED hull's authored bank rather than a fabricated one,
    /// so it fails on the content as well as on the code: slow the bolt down and
    /// this gets easier, speed it up and it eventually stops being true at all.
    ///
    /// Both halves are asserted, and the first is what makes the second mean
    /// something. A target that HOLDS its course is hit — the prediction works —
    /// and the identical shot against a target that reverses misses, with the
    /// projectile's heading provably unchanged across the whole flight. Without
    /// the control half a miss would be indistinguishable from a bolt that was
    /// never aimed properly to begin with.
    fn warhawk_artillery_bank() -> BlasterSystem {
        let cfg = crate::entity_config::EntityConfig::from_toml(include_str!(
            "../../assets/entities/ship_harrow_warhawk.toml"
        ))
        .expect("the shipped battleship hull must parse");
        let bank = cfg
            .weapons_console
            .as_ref()
            .expect("the battleship declares [weapons_console]")
            .blaster_banks
            .iter()
            .max_by(|a, b| a.range.total_cmp(&b.range))
            .expect("the battleship carries an artillery bank")
            .clone();
        BlasterSystem::new(bank.to_runtime())
    }

    /// The battleship's authored `artillery_hold_range` — the distance the hull
    /// actually stops at and shoots from. Read off the shipped hull rather than
    /// written here so a retune of the gun line moves these tests with it.
    fn warhawk_hold_range() -> f32 {
        crate::entity_config::EntityConfig::from_toml(include_str!(
            "../../assets/entities/ship_harrow_warhawk.toml"
        ))
        .expect("the shipped battleship hull must parse")
        .helm_console
        .expect("the battleship declares [helm_console]")
        .steering_ai
        .expect("...and authors [helm_console.steering_ai]")
        .param[crate::ship::helm_ai::ARTILLERY_HOLD_RANGE_PARAM]
    }

    /// A capital-ship-sized target, so a miss in these tests is a miss by a wide
    /// margin rather than by a rounding error.
    const TARGET_RADIUS: f32 = 5.0;

    /// Fire one artillery bolt at a target `range` units dead ahead crossing at
    /// `+launch_vx`, then let the target run at `vx_after` for the bolt's whole
    /// life. Returns whether the bolt ever found it, and asserts the heading
    /// never changes on the way.
    fn artillery_shot_finds_target(range: f32, launch_vx: f32, vx_after: f32) -> bool {
        artillery_shot_finds_moving_target(range, (launch_vx, 0.0), (vx_after, 0.0))
    }

    /// As above, but the target may close or open as well as cross: it starts
    /// `range` units dead ahead moving at `launch_v`, and flies `after_v` for
    /// the bolt's whole life. `+z` is *toward* the shooter, so a positive
    /// `after_v.1` is a closing target.
    fn artillery_shot_finds_moving_target(
        range: f32,
        launch_v: (f32, f32),
        after_v: (f32, f32),
    ) -> bool {
        let dt = 1.0 / 30.0;

        let mut sys = warhawk_artillery_bank();
        let mut counter = 0_u32;
        let mut next_uuid = || {
            counter += 1;
            format!("bolt-{counter}")
        };
        let mut tx = 0.0_f32;
        let mut tz = -range;

        sys.request_fire();
        let launched = sys.tick(
            dt,
            &[(0.0, 0.0)],
            0.0,
            tx,
            tz,
            launch_v.0,
            launch_v.1,
            "shooter",
            &mut next_uuid,
        );
        assert_eq!(launched.len(), 1, "one bolt per volley on this bank");
        let heading = sys.in_flight[0].heading;

        // Fly it out. The target now runs at `after_v`, which the bolt has no
        // way of learning: nothing in `BlasterProjectile::tick` reads a target.
        for _ in 0..2000 {
            tx += after_v.0 * dt;
            tz += after_v.1 * dt;
            sys.tick(
                dt,
                &[(0.0, 0.0)],
                0.0,
                tx,
                tz,
                after_v.0,
                after_v.1,
                "shooter",
                &mut next_uuid,
            );
            let Some(bolt) = sys.in_flight.first() else {
                return false; // expired without finding anything
            };
            assert_eq!(
                bolt.heading, heading,
                "the launch heading must be frozen: a bolt that re-aimed would be \
                 homing, which is exactly what AC4 forbids"
            );
            if !sys
                .find_hits(&[("target".to_string(), tx, tz, TARGET_RADIUS)])
                .is_empty()
            {
                return true;
            }
        }
        panic!("the bolt must expire inside its own lifespan");
    }

    #[test]
    fn a_target_that_changes_course_evades_the_artillery_bolt() {
        const RANGE: f32 = 150.0;
        // The crossing speed the control half is calibrated against, and the
        // number this test was RE-AUTHORED around. It used to be 10 u/s, chosen
        // because the shipped lead was first-order — `t_est` measured the flight
        // time to where the target was standing, not to where it would be — so
        // it under-led every crosser and only a slow one made "held course hits"
        // a fair control rather than a coin toss about the estimator.
        //
        // With a closed-form intercept the control is honest at a speed the game
        // actually ships: 20 u/s is the cruise of the pirate raider and the
        // Alliance courier. It is not the FASTEST shipped cruise (the Harrow
        // destroyer's 26) — see
        // `the_bolts_authored_range_caps_the_crossing_speed_a_lead_can_reach`
        // for why that one is out of reach, which is a content limit rather than
        // an estimator limit and belongs in its own test.
        const CROSSING: f32 = 20.0;

        // Holding course: the launch-time prediction puts the bolt where the
        // target will be, and it connects.
        assert!(
            artillery_shot_finds_target(RANGE, CROSSING, CROSSING),
            "a target that keeps its course must be hit — otherwise the miss below \
             proves nothing about course changes"
        );

        // Reversing after launch: the bolt flies to an intercept the target
        // abandoned, and misses.
        assert!(
            !artillery_shot_finds_target(RANGE, CROSSING, -CROSSING),
            "a target that reverses after launch must evade a bolt already in the \
             air"
        );

        // ...and simply stopping is enough. The evasion is not about outrunning
        // the shot, it is about the shot being committed to a course the target
        // no longer flies.
        assert!(
            !artillery_shot_finds_target(RANGE, CROSSING, 0.0),
            "a target that merely stops must also evade the committed bolt"
        );
    }

    /// The Harrow destroyer's authored `max_speed` — the fastest cruise the game
    /// ships. Read off the hull so a retune of the fleet moves these tests with
    /// it rather than leaving them asserting against a number nobody ships.
    fn destroyer_cruise() -> f32 {
        crate::entity_config::EntityConfig::from_toml(include_str!(
            "../../assets/entities/ship_harrow_destroyer.toml"
        ))
        .expect("the destroyer hull must parse")
        .helm_console
        .expect("the destroyer declares [helm_console]")
        .max_speed
    }

    /// Closest a straight bolt fired from the origin toward `aim` ever comes to
    /// a target that starts at `start` and holds `vel`, within the bolt's own
    /// lifespan.
    ///
    /// Both bodies fly straight, so the separation is a quadratic in `t` and the
    /// minimum is exact — no sampling, no missed frame. Clamped to `[0, life]`
    /// because a bolt that has expired is not a threat however close it would
    /// have come afterwards.
    fn closest_approach(
        start: (f32, f32),
        vel: (f32, f32),
        aim: (f32, f32),
        speed: f32,
        bolt_range: f32,
    ) -> f32 {
        let aim_len = (aim.0 * aim.0 + aim.1 * aim.1).sqrt();
        assert!(aim_len > 0.0, "an aim point must be a direction");
        // Separation D(t) = start + (vel − bolt_velocity)·t.
        let rx = vel.0 - speed * aim.0 / aim_len;
        let rz = vel.1 - speed * aim.1 / aim_len;
        let quad = rx * rx + rz * rz;
        let life = bolt_range / speed;
        let t = if quad > 0.0 {
            (-(start.0 * rx + start.1 * rz) / quad).clamp(0.0, life)
        } else {
            0.0
        };
        let dx = start.0 + rx * t;
        let dz = start.1 + rz * t;
        (dx * dx + dz * dz).sqrt()
    }

    /// The property the exact solver exists to deliver: at the range the
    /// battleship actually holds, the fastest hull the game ships flying a
    /// perfectly straight line is HIT — and the estimate this solver replaced
    /// would have missed the same shot by a wide margin.
    ///
    /// Every figure is read off shipped content: the range from the battleship's
    /// own `artillery_hold_range`, the target's speed from the Harrow
    /// destroyer's authored `max_speed`, the envelope and hit radius from the
    /// bank. Nothing is a geometry invented to pass.
    ///
    /// ## Why a closing diagonal and not a square-on crosser
    ///
    /// A square-on crosser cannot carry this test, and the reason is content
    /// rather than aiming. At the authored hold range the bolt's own `range`
    /// admits square-on leads only up to about 15 u/s (see
    /// `the_bolts_authored_range_caps_the_crossing_speed_a_lead_can_reach`,
    /// which pins that cap), and below that cap the first-order estimate's error
    /// is still inside the 8-unit hit radius — so a square-on version of this
    /// test cannot fail even with the solver reverted, and would be asserting
    /// nothing. A closing component shortens the intercept, which both puts the
    /// destroyer's full cruise back inside the envelope and opens the lead angle
    /// far enough that the two estimators visibly disagree. It is also the more
    /// honest geometry: hulls close as they cross rather than sliding sideways
    /// forever.
    #[test]
    fn a_straight_line_crosser_is_hit_at_the_shipped_hold_range() {
        let bank = warhawk_artillery_bank().config;
        let hold = warhawk_hold_range();
        // A 45 deg closing diagonal at the destroyer's authored cruise: the
        // speed is the tuning's, the split between crossing and closing is this
        // test's.
        let component = destroyer_cruise() / std::f32::consts::SQRT_2;
        let vel = (component, component);

        let t = solve_intercept_time(0.0, -hold, vel.0, vel.1, bank.projectile_speed)
            .expect("a hull slower than the bolt always has an intercept");
        let flight = bank.projectile_speed * t;

        // Asserted, not assumed: the shot must sit COMFORTABLY inside the bolt's
        // envelope. A version of this test that passed on the last 1% of `range`
        // would flip on a one-unit retune of `range` or `artillery_hold_range`,
        // for a reason that has nothing to do with the aiming it is about.
        assert!(
            flight < bank.range * 0.9,
            "this test needs real headroom against the bolt's envelope: the \
             intercept is {flight} units of flight against a {} unit range, \
             which is inside the last tenth. Re-pick the geometry rather than \
             letting a retune of `range` or `artillery_hold_range` decide \
             whether an aiming test passes.",
            bank.range
        );

        assert!(
            artillery_shot_finds_moving_target(hold, vel, vel),
            "a hull holding a straight course at its own authored cruise must be \
             hit at the battleship's own hold range ({hold} units): a target \
             that does nothing must not be able to beat an artillery piece aimed \
             at it"
        );

        // ── The counterfactual, asserted rather than claimed in prose ────────
        //
        // The estimate this solver replaced measured the flight time to where
        // the target was STANDING (`t = |r| / c`) rather than to where it would
        // be. Firing this same shot on that aim point misses, and misses wide —
        // which is what earns this test its keep. If it ever stops missing, the
        // test has quietly stopped discriminating.
        let hit_radius = TARGET_RADIUS + bank.collision_radius;
        let start = (0.0_f32, -hold);
        let solved_miss = closest_approach(
            start,
            vel,
            (vel.0 * t, -hold + vel.1 * t),
            bank.projectile_speed,
            bank.range,
        );
        assert!(
            solved_miss < hit_radius,
            "sanity: the solved aim point must be a hit, got {solved_miss} \
             units of closest approach against a {hit_radius} unit radius"
        );

        let t_first_order = hold / bank.projectile_speed;
        let first_order_miss = closest_approach(
            start,
            vel,
            (vel.0 * t_first_order, -hold + vel.1 * t_first_order),
            bank.projectile_speed,
            bank.range,
        );
        assert!(
            first_order_miss > 2.0 * hit_radius,
            "the first-order estimate must MISS this shot and miss it wide, or \
             this test proves nothing about the solver: closest approach \
             {first_order_miss} units against a {hit_radius} unit hit radius. If \
             the estimate now connects here, re-derive the geometry — do not \
             delete this assert and leave the docstring above claiming a \
             discrimination that is gone."
        );

        // The other control: the identical solved shot against a target that
        // changes its mind still misses, so the hit above is a lead and not a
        // bolt that happens to fly through where the target started.
        assert!(
            !artillery_shot_finds_moving_target(hold, vel, (-vel.0, vel.1)),
            "and a course change after launch must still evade it"
        );
    }

    /// A FINDING about the shipped tuning, pinned rather than left in prose.
    ///
    /// The bolt's authored `range` — where it stops existing — caps how far the
    /// lead can be thrown, independently of the solver. At the hull's own hold
    /// range the straight-line distance to the target already eats most of the
    /// envelope, so a crosser fast enough to need a long lead puts the intercept
    /// beyond where the bolt can reach and it expires in flight.
    ///
    /// This is NOT the estimator: it is `range` (200), `artillery_hold_range`
    /// (180) and `projectile_speed` (35) as authored. Nothing here is fixed by
    /// aiming better, which is exactly why it is asserted separately — a future
    /// widening of the envelope should have to come and change this test on
    /// purpose.
    #[test]
    fn the_bolts_authored_range_caps_the_crossing_speed_a_lead_can_reach() {
        let bank = warhawk_artillery_bank().config;
        let hold = warhawk_hold_range();

        // The crossing speed whose intercept lands exactly on the bolt's range:
        //   c·t = range  and  t = hold / sqrt(c² − v²)  ⇒  v = c·sqrt(1 − (hold/range)²)
        let reach = bank.projectile_speed * (1.0 - (hold / bank.range).powi(2)).sqrt();
        assert!(
            reach > 0.0 && reach < bank.projectile_speed,
            "precondition: a reachable crossing speed must resolve, got {reach}"
        );

        // The Harrow destroyer — the fastest hull the game ships — cruises past
        // it, so at the authored hold range it cannot be led SQUARE-ON at all.
        // (Give it a closing component and the intercept comes back inside the
        // envelope; `a_straight_line_crosser_is_hit_at_the_shipped_hold_range`
        // is that shot.)
        let destroyer_cruise = destroyer_cruise();
        assert!(
            destroyer_cruise > reach,
            "FINDING: at the authored hold range ({hold}) the bolt's own range \
             ({}) admits leads only up to a {reach} u/s crosser, and the fastest \
             shipped hull cruises at {destroyer_cruise}. Aiming cannot fix this — \
             the envelope has to.",
            bank.range
        );

        // ...and the mechanism, so the number above cannot be mistaken for a
        // solver failure: the solved intercept is real, it is simply out of reach.
        let t = solve_intercept_time(0.0, -hold, destroyer_cruise, 0.0, bank.projectile_speed)
            .expect("the intercept EXISTS — the destroyer is slower than the bolt");
        let flight = bank.projectile_speed * t;
        assert!(
            flight > bank.range,
            "the intercept is {flight} units out, which must exceed the bolt's \
             {} unit range for the cap above to be the reason",
            bank.range
        );

        // The constructive half: just inside the cap the shot connects, so this
        // is a boundary and not a blanket failure.
        assert!(
            artillery_shot_finds_target(hold, reach * 0.9, reach * 0.9),
            "a crosser inside the reachable band ({} u/s) must still be hit",
            reach * 0.9
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
