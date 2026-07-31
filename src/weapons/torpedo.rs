//! Pure-Rust torpedo mechanics.
//!
//! This module is platform-agnostic and Bevy-free. The torpedo system holds
//! a `Vec<TorpedoTube>` whose contents come from `[[torpedoes.tubes]]` in
//! the ship entity TOML. Each tube has a TOML-defined `id`,
//! `facing_deg`, and `fire_arc_deg`. Ammunition is a single shared pool
//! (`[torpedoes] count`).
//!
//! When a torpedo is launched it tracks a target UUID with a limited turn
//! rate. If the target is gone the torpedo flies straight. Torpedoes expire
//! after a configurable lifespan.
//!
//! Coordinate system: same as `ship_physics` / `radar` Ã¢â‚¬â€ XZ plane, Y-up.
//! Ship forward is Ã¢Ë†â€™Z when yaw = 0.

use crate::simmath;
use crate::weapons::pattern::BarrelPattern;
use std::f32::consts::PI;

/// String identifier for a torpedo tube (matches the `id` field in TOML).
pub type TorpedoTubeId = String;

// Ã¢â€â‚¬Ã¢â€â‚¬ Configuration Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬

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
    /// Default tube load time in seconds.
    pub load_time: f32,
    /// Proximity-detonation radius in world units. A torpedo explodes when
    /// its centre comes within `detonation_radius + target_radius` of any
    /// entity. Independent of homing Ã¢â‚¬â€ an un-locked torpedo still detonates
    /// on contact.
    pub detonation_radius: f32,
    /// Fraction of the `damage_shields` payload that bypasses the shield
    /// system entirely and adds to hull damage. Default `0.0` Ã¢â‚¬â€ all
    /// `damage_shields` is mitigated by the facing shield quadrant.
    /// `damage_hull` is unaffected (it always hits hull by design).
    /// Clamped to `[0.0, 1.0]` at apply time.
    pub shield_pierce: f32,
    /// Interval in seconds between successive torpedo launches in a burst
    /// volley (issue #632). Default `0.3`.
    pub burst_interval_secs: f32,
    /// Ship-wide default for how many rounds an AI-operated crew keeps loaded
    /// per tube (`[torpedoes] ai_volley_target`). `None` → each tube falls
    /// back to its own `volley_max`. Consumed only by
    /// [`TorpedoSystem::from_configs`] when it resolves
    /// [`TorpedoTube::ai_target_count`].
    pub ai_volley_target: Option<u32>,
}

impl Default for TorpedoConfig {
    fn default() -> Self {
        Self {
            count: 10,
            damage_hull: 50,
            damage_shields: 5,
            speed: 15.0,
            turn_rate: PI / 4.0,
            lifespan: 20.0,
            load_time: 10.0,
            detonation_radius: 5.0,
            shield_pierce: 0.0,
            burst_interval_secs: 0.3,
            ai_volley_target: None,
        }
    }
}

// Ã¢â€â‚¬Ã¢â€â‚¬ In-flight torpedo Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬

#[derive(Clone, Debug)]
pub struct Torpedo {
    pub uuid: String,
    pub x: f32,
    /// Vertical (altitude) position in world space. `0.0` for a torpedo fired
    /// by (and at) a Planar hull, so the 3D maths below collapses exactly to
    /// the pre-#768 XZ behaviour (issue #768).
    pub y: f32,
    pub z: f32,
    pub heading: f32,
    /// Vertical steering angle in radians: the torpedo's climb/descent pitch
    /// relative to the horizontal plane (issue #768). `0.0` = level flight,
    /// positive = climbing (+Y). Rate-limited toward the target's elevation by
    /// the same `TorpedoConfig::turn_rate` clamp the yaw uses, so there is no
    /// new gameplay constant. Stays exactly `0.0` while the target is level
    /// (or absent), collapsing movement to the 2D case bit-for-bit.
    pub pitch: f32,
    pub lifespan_remaining: f32,
    pub target_uuid: Option<String>,
    /// UUID of the entity that fired this torpedo. Used by
    /// [`TorpedoSystem::find_detonation_hits`] to prevent a torpedo from
    /// detonating on its launcher (the torpedo spawns at the launcher's
    /// centre, well within any reasonable detonation radius).
    pub source_uuid: Option<String>,
    /// Id of the tube that launched it, carried so balance telemetry can
    /// attribute damage per-tube. The tube is out of scope by detonation
    /// time, and `TorpedoConfig` is ship-wide, so this has to ride along.
    pub tube_id: String,
    /// Snapshot of the firing tube's `shield_pierce` at launch time, so
    /// detonation logic can split `damage_shields` between absorbed and
    /// pierced portions without re-resolving the source config. Clamped
    /// to `[0.0, 1.0]` at apply time.
    pub shield_pierce: f32,
}

impl Torpedo {
    pub fn tick(&mut self, dt: f32, target_pos: Option<(f32, f32, f32)>, config: &TorpedoConfig) {
        if self.target_uuid.is_some() {
            if let Some((tx, ty, tz)) = target_pos {
                let dx = tx - self.x;
                let dz = tz - self.z;
                let desired = simmath::atan2(dx, -dz);
                let delta = angle_diff(desired, self.heading);
                let max_turn = config.turn_rate * dt;
                self.heading += delta.clamp(-max_turn, max_turn);
                // Vertical steering (issue #768): rotate the climb pitch toward
                // the target's elevation, rate-limited by the SAME turn_rate
                // clamp as the yaw. `pitch` lives in `[-pi/2, pi/2]` (the range
                // of `atan2(dy, horizontal)`), so no angle wrapping is needed.
                // When the target is level with the torpedo (`dy == 0`),
                // `desired_pitch` is exactly `0.0` and pitch never leaves 0 —
                // the movement below then collapses to the 2D case.
                let dy = ty - self.y;
                let horizontal = (dx * dx + dz * dz).sqrt();
                let desired_pitch = simmath::atan2(dy, horizontal);
                let dpitch = (desired_pitch - self.pitch).clamp(-max_turn, max_turn);
                self.pitch += dpitch;
            }
        }
        let cos_h = simmath::cos(self.heading);
        let sin_h = simmath::sin(self.heading);
        // `cos_p == 1.0` and `sin_p == 0.0` exactly while pitch is 0, so the XZ
        // integration is bit-identical to the pre-#768 planar path and Y stays
        // put (issue #768, AC3).
        let cos_p = simmath::cos(self.pitch);
        let sin_p = simmath::sin(self.pitch);
        self.x += sin_h * cos_p * config.speed * dt;
        self.z -= cos_h * cos_p * config.speed * dt;
        self.y += sin_p * config.speed * dt;
        self.lifespan_remaining = (self.lifespan_remaining - dt).max(0.0);
    }

    pub fn is_expired(&self) -> bool {
        self.lifespan_remaining <= 0.0
    }
}

// Ã¢â€â‚¬Ã¢â€â‚¬ Torpedo tube Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬

/// Load state for a single torpedo tube.
#[derive(Clone, Debug, PartialEq)]
pub enum TubeLoadState {
    Unloaded,
    Loading { remaining: f32, total: f32 },
    Loaded,
    Unloading { remaining: f32, total: f32 },
}

impl TubeLoadState {
    /// Completion fraction in `[0.0, 1.0]`: 0 = just started, 1 = done.
    pub fn progress(&self) -> f32 {
        match self {
            TubeLoadState::Unloaded => 0.0,
            TubeLoadState::Loaded => 1.0,
            TubeLoadState::Loading { remaining, total }
            | TubeLoadState::Unloading { remaining, total } => {
                if *total <= 0.0 {
                    1.0
                } else {
                    1.0 - remaining / total
                }
            }
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            TubeLoadState::Unloaded => "unloaded",
            TubeLoadState::Loading { .. } => "loading",
            TubeLoadState::Loaded => "loaded",
            TubeLoadState::Unloading { .. } => "unloading",
        }
    }
}

#[derive(Clone, Debug)]
pub struct TorpedoTube {
    /// Tube identifier from TOML (e.g. `"fore_port"`, `"aft"`).
    pub id: TorpedoTubeId,
    /// Centre of the tube's fire arc, degrees clockwise from ship-forward.
    pub facing_deg: f32,
    /// Total fire-arc width in degrees.
    pub fire_arc_deg: f32,
    /// Current load state for the in-progress load/unload operation.
    /// When `loaded_count < volley_max`, this tracks the next torpedo
    /// being loaded. When `loaded_count > target_count`, it tracks the
    /// torpedo being unloaded.
    pub load_state: TubeLoadState,
    /// Seconds to load or unload one torpedo.
    pub load_time: f32,
    /// Maximum number of torpedoes this tube can hold at once (from TOML).
    pub volley_max: u32,
    /// Number of torpedoes currently loaded and ready to fire.
    pub loaded_count: u32,
    /// Desired number of loaded torpedoes (0..=volley_max). The tube
    /// loads/unloads toward this value automatically.
    pub target_count: u32,
    /// The `target_count` an AI-operated crew asks this tube to sit at,
    /// resolved from TOML (`[[torpedoes.tubes]] ai_target_count`, then
    /// `[torpedoes] ai_volley_target`, then `volley_max`).
    ///
    /// Read only by `console_ai::server::ai_torpedo_load`, which turns it
    /// into the same `SetTorpedoVolleyTarget` command a human console sends —
    /// it is never applied to `target_count` directly, so AI and human
    /// loading share one code path.
    pub ai_target_count: u32,
    /// Authored barrel-marker names (issue #766). Empty ⇒ one implicit barrel
    /// = the tube's single `marker`. A [`pattern`](Self::pattern) step
    /// addresses these by index. Threaded from `TorpedoTubeConfig::barrels`.
    pub barrels: Vec<String>,
    /// Timed multi-barrel firing pattern (issue #766). Governs only WHICH
    /// authored barrel each launched round leaves from, and the order barrels
    /// cycle through the volley — NOT how many rounds fire. `loaded_count`, the
    /// magazine, and the burst cadence stay the sole authority over the count.
    pub pattern: BarrelPattern,
    /// Barrel indices that fired on the most recently launched round (issue
    /// #766). One entry per shot (torpedoes fire one-per-burst). Empty when the
    /// tube has never fired. Surfaced on the wire for the Tactical indicator.
    pub active_barrels: Vec<u32>,
    /// 1-based index of the pattern step the most recently launched round came
    /// from (0 when none has fired). With [`Self::pattern_len`] this renders as
    /// "step N/M" for a patterned tube.
    pub pattern_step: u32,
}

impl TorpedoTube {
    /// True when at least one torpedo is loaded and ready to fire.
    pub fn is_loaded(&self) -> bool {
        self.loaded_count > 0
    }

    /// Fraction of the in-progress load/unload operation, `[0.0, 1.0]`.
    /// Returns 0.0 when idle (no active operation in progress).
    pub fn load_progress(&self) -> f32 {
        self.load_state.progress()
    }

    /// Advance the in-progress load/unload timer by `dt` seconds.
    /// Returns `LoadTickOutcome` indicating any state transitions.
    pub fn tick(&mut self, dt: f32) -> LoadTickOutcome {
        let prev = self.load_state.clone();
        self.load_state = match self.load_state.clone() {
            TubeLoadState::Loading { remaining, total } => {
                let r = (remaining - dt).max(0.0);
                if r <= 0.0 {
                    TubeLoadState::Loaded
                } else {
                    TubeLoadState::Loading {
                        remaining: r,
                        total,
                    }
                }
            }
            TubeLoadState::Unloading { remaining, total } => {
                let r = (remaining - dt).max(0.0);
                if r <= 0.0 {
                    TubeLoadState::Unloaded
                } else {
                    TubeLoadState::Unloading {
                        remaining: r,
                        total,
                    }
                }
            }
            other => other,
        };
        match (&prev, &self.load_state) {
            (TubeLoadState::Loading { .. }, TubeLoadState::Loaded) => LoadTickOutcome::LoadedOne,
            (TubeLoadState::Unloading { .. }, TubeLoadState::Unloaded) => {
                LoadTickOutcome::UnloadedOne
            }
            _ => LoadTickOutcome::None,
        }
    }

    /// Start loading from the Unloaded state (manual load).
    pub fn start_load(&mut self) {
        if self.load_state == TubeLoadState::Unloaded {
            let t = self.load_time;
            self.load_state = TubeLoadState::Loading {
                remaining: t,
                total: t,
            };
        }
    }

    /// Start unloading: from Loaded Ã¢â€ â€™ Unloading; cancel an in-progress load Ã¢â€ â€™ Unloaded.
    pub fn start_unload(&mut self) {
        match &self.load_state {
            TubeLoadState::Loaded => {
                let t = self.load_time;
                self.load_state = TubeLoadState::Unloading {
                    remaining: t,
                    total: t,
                };
            }
            TubeLoadState::Loading { .. } => {
                self.load_state = TubeLoadState::Unloaded;
            }
            _ => {}
        }
    }

    /// Mark this tube empty after firing all loaded torpedoes.
    pub fn mark_fired(&mut self) {
        self.loaded_count = 0;
        self.load_state = TubeLoadState::Unloaded;
    }

    /// Number of authored barrels: the barrel-marker count, or `1` (the
    /// implicit single barrel = `marker`) when none are authored.
    pub fn barrel_count(&self) -> usize {
        if self.barrels.is_empty() {
            1
        } else {
            self.barrels.len()
        }
    }

    /// Total number of authored pattern steps (0 when the tube has no
    /// multi-barrel pattern — the single-barrel/backward-compat case). Surfaced
    /// on the wire so the Tactical indicator only shows a step count for a
    /// genuine patterned tube.
    pub fn pattern_len(&self) -> u32 {
        self.pattern.len() as u32
    }

    /// The flattened, offset-ordered barrel firing sequence for one volley
    /// (issue #766). Each entry is `(barrel_index, step_number)` where
    /// `step_number` is the 1-based position of the owning step after sorting
    /// the pattern by `offset_secs`.
    ///
    /// A step listing several barrels contributes each of them, in order, at
    /// the same `step_number` — successive rounds of a volley draw their origin
    /// from consecutive entries (cycling when the volley is longer than the
    /// sequence). This is the ORIGIN map only: it decides which barrel a round
    /// leaves from, never how many rounds a volley fires (that stays
    /// `loaded_count`).
    ///
    /// An empty pattern yields a single implicit barrel `0` at step `0`, so the
    /// backward-compat single-barrel tube resolves every round to `marker`.
    pub fn barrel_sequence(&self) -> Vec<(u32, u32)> {
        if self.pattern.is_empty() {
            return vec![(0, 0)];
        }
        let mut steps: Vec<&crate::weapons::pattern::BarrelPatternStep> =
            self.pattern.iter().collect();
        steps.sort_by(|a, b| {
            a.offset_secs
                .partial_cmp(&b.offset_secs)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let mut seq = Vec::new();
        for (i, step) in steps.iter().enumerate() {
            for &b in &step.barrels {
                seq.push((b, (i + 1) as u32));
            }
        }
        if seq.is_empty() {
            seq.push((0, 0));
        }
        seq
    }

    /// True if `bearing_rad` (radians from ship-forward, +right = positive)
    /// is within this tube's fire arc.
    pub fn is_in_arc(&self, bearing_rad: f32) -> bool {
        let half = (self.fire_arc_deg.to_radians()) * 0.5;
        let facing = self.facing_deg.to_radians();
        angle_diff(bearing_rad, facing).abs() <= half
    }
}

/// Outcome of a single `TorpedoTube::tick` call.
#[derive(Clone, Debug, PartialEq)]
pub enum LoadTickOutcome {
    None,
    /// A loading operation completed; caller should increment `loaded_count`.
    LoadedOne,
    /// An unloading operation completed; caller should decrement `loaded_count`
    /// and return one torpedo to the magazine.
    UnloadedOne,
}

// Ã¢â€â‚¬Ã¢â€â‚¬ Torpedo system Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬

#[derive(Clone, Debug)]
pub struct TorpedoSystem {
    pub tubes: Vec<TorpedoTube>,
    pub config: TorpedoConfig,
    pub torpedoes_remaining: u32,
    pub in_flight: Vec<Torpedo>,
    /// Per-tube burst-fire pending state (issue #632). Entries are removed
    /// when their `pending` count reaches 0.
    pub burst_states: Vec<TubeBurstState>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum LaunchResult {
    /// First torpedo of a burst was launched. `count_remaining` is the number
    /// of additional torpedoes still to be fired (0 for a single shot).
    Launched {
        uuid: String,
        count_remaining: u32,
    },
    TubeNotLoaded,
    NoTorpedoes,
    UnknownTube,
}

#[derive(Clone, Debug, Default)]
pub struct TorpedoTickResult {
    pub expired: Vec<String>,
    /// Torpedoes launched by burst-fire this tick.
    /// Each entry is `(tube_id, uuid, x, y, z, heading)` (issue #768 threads Y).
    pub burst_launched: Vec<(String, String, f32, f32, f32, f32)>,
}

/// Pending burst-fire state for a single tube. Stored on `TorpedoSystem` so
/// the pure-module `tick` can drive the burst without Bevy.
#[derive(Clone, Debug, Default)]
pub struct TubeBurstState {
    pub tube_id: String,
    /// Torpedoes remaining to be fired in the current burst (0 = idle).
    pub pending: u32,
    /// Countdown timer before the next burst shot fires.
    pub timer: f32,
    /// Parameters captured at fire time for the burst shots.
    pub launch_x: f32,
    /// Vertical launch origin captured at fire time (issue #768). `0.0` for a
    /// Planar hull, so burst shots stay on the play plane.
    pub launch_y: f32,
    pub launch_z: f32,
    pub launch_heading: f32,
    pub target_uuid: Option<String>,
    pub source_uuid: Option<String>,
    /// World-XYZ origin per authored barrel index (issue #766/#768), captured at
    /// fire time from the tube's barrel markers. Empty for a legacy single-barrel
    /// launch, in which case burst shots fall back to
    /// `launch_x`/`launch_y`/`launch_z`.
    pub barrel_origins: Vec<(f32, f32, f32)>,
    /// The tube's flattened `(barrel_index, step_number)` firing sequence
    /// (issue #766), captured at fire time. Each burst shot draws its origin
    /// from the next entry, cycling. Empty ⇒ legacy single-origin burst.
    pub barrel_sequence: Vec<(u32, u32)>,
    /// Index of the next volley round to fire (0-based across the whole volley;
    /// the immediate launch was round 0, so the first burst shot is round 1).
    /// Indexes `barrel_sequence` modulo its length.
    pub next_shot_index: u32,
}

impl TorpedoSystem {
    /// Construct a torpedo system with the three legacy tubes
    /// (`fore_port`, `fore_starboard`, `aft`) for test convenience.
    /// Production code should use [`Self::from_configs`].
    pub fn new(config: TorpedoConfig) -> Self {
        let count = config.count;
        let load_time = config.load_time;
        // Legacy tubes are all `volley_max: 1`, so the ship-wide AI default
        // clamps to at most one round per tube.
        let ai_target_count = config.ai_volley_target.unwrap_or(1).min(1);
        let tubes = vec![
            TorpedoTube {
                id: "fore_port".to_string(),
                facing_deg: -30.0,
                fire_arc_deg: 90.0,
                load_state: TubeLoadState::Unloaded,
                load_time,
                volley_max: 1,
                loaded_count: 0,
                target_count: 0,
                ai_target_count,
                barrels: Vec::new(),
                pattern: Vec::new(),
                active_barrels: Vec::new(),
                pattern_step: 0,
            },
            TorpedoTube {
                id: "fore_starboard".to_string(),
                facing_deg: 30.0,
                fire_arc_deg: 90.0,
                load_state: TubeLoadState::Unloaded,
                load_time,
                volley_max: 1,
                loaded_count: 0,
                target_count: 0,
                ai_target_count,
                barrels: Vec::new(),
                pattern: Vec::new(),
                active_barrels: Vec::new(),
                pattern_step: 0,
            },
            TorpedoTube {
                id: "aft".to_string(),
                facing_deg: 180.0,
                fire_arc_deg: 90.0,
                load_state: TubeLoadState::Unloaded,
                load_time,
                volley_max: 1,
                loaded_count: 0,
                target_count: 0,
                ai_target_count,
                barrels: Vec::new(),
                pattern: Vec::new(),
                active_barrels: Vec::new(),
                pattern_step: 0,
            },
        ];
        Self {
            tubes,
            config,
            torpedoes_remaining: count,
            in_flight: Vec::new(),
            burst_states: Vec::new(),
        }
    }

    /// Build a torpedo system from the parsed TOML tube configs.
    pub fn from_configs(
        tubes: &[crate::entities::config::TorpedoTubeConfig],
        config: TorpedoConfig,
    ) -> Self {
        let global_load_time = config.load_time;
        let global_ai_target = config.ai_volley_target;
        let tubes = tubes
            .iter()
            .map(|c| TorpedoTube {
                id: c.id.clone(),
                facing_deg: c.facing_deg,
                fire_arc_deg: c.fire_arc_deg,
                load_state: TubeLoadState::Unloaded,
                load_time: c.load_time.unwrap_or(global_load_time),
                volley_max: c.volley_max,
                loaded_count: 0,
                // Tubes start empty for everyone — human and AI alike. The AI
                // reaches `ai_target_count` only by sending the same
                // `SetTorpedoVolleyTarget` command a console sends, so a
                // human-crewed tube stays exactly as the player left it.
                target_count: 0,
                ai_target_count: c
                    .ai_target_count
                    .or(global_ai_target)
                    .unwrap_or(c.volley_max)
                    .min(c.volley_max),
                barrels: c.barrels.clone(),
                pattern: c.pattern.clone(),
                active_barrels: Vec::new(),
                pattern_step: 0,
            })
            .collect();
        let count = config.count;
        Self {
            tubes,
            config,
            torpedoes_remaining: count,
            in_flight: Vec::new(),
            burst_states: Vec::new(),
        }
    }

    pub fn tube(&self, id: &str) -> Option<&TorpedoTube> {
        self.tubes.iter().find(|t| t.id == id)
    }

    pub fn tube_mut(&mut self, id: &str) -> Option<&mut TorpedoTube> {
        self.tubes.iter_mut().find(|t| t.id == id)
    }

    /// How many MORE rounds must come out of the magazine before every tube is
    /// at `volley_max` — i.e. before a ship-wide `tubes_full` could read true
    /// (issue #791 follow-up).
    ///
    /// Rounds already claimed for an in-progress load are NOT counted again:
    /// the magazine is decremented when a load *starts* (see `start_load` and
    /// the auto-load block in [`Self::tick`]), so a `Loading` tube is already
    /// paid for and its round is simply not in `loaded_count` yet. Counting it
    /// twice would make a ship with exactly enough rounds look one short for
    /// the whole of every load cycle.
    ///
    /// A hull with no tubes returns 0. That is arithmetically right and
    /// deliberately not the answer to "can this ship fill its tubes" — the
    /// caller that asks THAT question has to rule out a tubeless hull itself,
    /// because `all`-over-nothing is vacuously true here as it is everywhere.
    pub fn salvo_shortfall(&self) -> u32 {
        self.tubes
            .iter()
            .map(|t| {
                let in_progress = u32::from(matches!(t.load_state, TubeLoadState::Loading { .. }));
                t.volley_max
                    .saturating_sub(t.loaded_count)
                    .saturating_sub(in_progress)
            })
            .sum()
    }

    /// Start loading a torpedo into the given tube.
    ///
    /// Consumes one torpedo from the shared pool. Returns `false` if the pool
    /// is empty, the tube id is unknown, the tube is not in the `Unloaded`
    /// state, or the tube is already at `volley_max` capacity.
    pub fn start_load(&mut self, tube_id: &str) -> bool {
        if self.torpedoes_remaining == 0 {
            return false;
        }
        let idx = self.tubes.iter().position(|t| t.id == tube_id);
        let Some(idx) = idx else {
            return false;
        };
        if self.tubes[idx].load_state != TubeLoadState::Unloaded {
            return false;
        }
        if self.tubes[idx].loaded_count >= self.tubes[idx].volley_max {
            return false;
        }
        self.torpedoes_remaining -= 1;
        self.tubes[idx].start_load();
        true
    }

    /// Start loading a torpedo into the given tube *without* consuming from
    /// the shared magazine (issue #512).
    ///
    /// Used by the Bevy `handle_torpedo_magazine_inter_system` consumer after
    /// the magazine has already been decremented via the channel-2
    /// [`crate::messages::InterSystemPayload::ClaimTorpedoRound`] transaction.
    /// The caller is responsible for having granted the round.
    ///
    /// Returns `false` if the tube id is unknown, the tube is not in the
    /// `Unloaded` state, or the tube is at `volley_max` capacity. Does NOT
    /// check magazine stock.
    pub fn start_load_reserved(&mut self, tube_id: &str) -> bool {
        let idx = self.tubes.iter().position(|t| t.id == tube_id);
        let Some(idx) = idx else {
            return false;
        };
        if self.tubes[idx].load_state != TubeLoadState::Unloaded {
            return false;
        }
        if self.tubes[idx].loaded_count >= self.tubes[idx].volley_max {
            return false;
        }
        self.tubes[idx].start_load();
        true
    }

    /// Consume one round from the shared magazine without touching any tube
    /// (issue #512). Used by the magazine consumer to enact a
    /// [`crate::messages::InterSystemPayload::ClaimTorpedoRound`]. Returns
    /// `false` when the magazine is empty (claim refused).
    pub fn claim_magazine_round(&mut self) -> bool {
        if self.torpedoes_remaining == 0 {
            return false;
        }
        self.torpedoes_remaining -= 1;
        true
    }

    /// Start unloading (or cancel a load in progress).
    ///
    /// If the tube is currently `Loading`, cancels immediately and returns the
    /// torpedo to the magazine.
    ///
    /// If the tube is `Unloaded` and `loaded_count > 0`, begins the unloading
    /// timer for one torpedo Ã¢â‚¬â€ the torpedo is returned to the magazine when
    /// the timer completes in [`Self::tick`].
    ///
    /// Returns `false` if the tube id is unknown, or there is nothing to unload
    /// (idle and `loaded_count == 0`), or a previous unload is already in
    /// progress.
    pub fn start_unload(&mut self, tube_id: &str) -> bool {
        let idx = self.tubes.iter().position(|t| t.id == tube_id);
        let Some(idx) = idx else {
            return false;
        };
        let tube = &self.tubes[idx];
        let is_loading = matches!(tube.load_state, TubeLoadState::Loading { .. });
        let is_unloading = matches!(tube.load_state, TubeLoadState::Unloading { .. });
        if is_loading {
            // Cancel in-progress load, return torpedo to pool.
            self.tubes[idx].start_unload();
            self.torpedoes_remaining += 1;
            true
        } else if is_unloading {
            false
        } else if self.tubes[idx].loaded_count > 0 {
            // Start unloading one of the ready torpedoes.
            let t = self.tubes[idx].load_time;
            self.tubes[idx].load_state = TubeLoadState::Unloading {
                remaining: t,
                total: t,
            };
            true
        } else {
            false
        }
    }

    /// Fire all loaded torpedoes from the specified tube as a burst.
    ///
    /// The first torpedo is added to `in_flight` immediately. If `loaded_count
    /// > 1`, a `TubeBurstState` entry is pushed so subsequent torpedoes fire
    /// at `burst_interval_secs` cadence during `tick` calls.
    ///
    /// Returns `LaunchResult::TubeNotLoaded` when `loaded_count == 0`.
    /// After a successful launch `loaded_count` is set to 0 and `load_state`
    /// reset to `Unloaded`.
    #[allow(clippy::too_many_arguments)]
    pub fn launch(
        &mut self,
        tube_id: &str,
        uuid: String,
        launch_x: f32,
        launch_y: f32,
        launch_z: f32,
        launch_heading: f32,
        target_uuid: Option<String>,
        source_uuid: Option<String>,
    ) -> LaunchResult {
        // Legacy single-origin launch: no authored barrel origins, so every
        // round leaves from `launch_x`/`launch_y`/`launch_z` (ship centre or
        // single marker) exactly as before issue #766.
        self.launch_with_barrels(
            tube_id,
            uuid,
            &[],
            launch_x,
            launch_y,
            launch_z,
            launch_heading,
            target_uuid,
            source_uuid,
        )
    }

    /// Patterned launch (issue #766). Identical to [`Self::launch`] except the
    /// caller supplies one resolved world-XZ origin per authored barrel index
    /// (`barrel_origins`, resolved by the Bevy driver from the tube's rig
    /// markers). Each volley round draws its origin from the tube's flattened
    /// barrel sequence — the immediate launch from the first sequence entry,
    /// and every burst shot from the next, cycling.
    ///
    /// The pattern governs ONLY the origin (and barrel order); it never changes
    /// the number of rounds fired. That stays `loaded_count`: a two-barrel
    /// simultaneous step with only one round loaded fires exactly one torpedo,
    /// the magazine is untouched (spend already happened at load time), and the
    /// burst count is bounded by `loaded_count - 1`.
    ///
    /// Timing rule (documented per issue #766): `burst_interval_secs` remains
    /// the SOLE cadence between rounds. The pattern's `offset_secs` orders the
    /// barrel sequence (steps fire in ascending-offset order) but does not
    /// retime the burst — keeping the round count and cadence authoritative.
    ///
    /// An empty `barrel_origins` (or a tube with no pattern) reproduces the
    /// legacy single-origin launch.
    #[allow(clippy::too_many_arguments)]
    pub fn launch_with_barrels(
        &mut self,
        tube_id: &str,
        uuid: String,
        barrel_origins: &[(f32, f32, f32)],
        launch_x: f32,
        launch_y: f32,
        launch_z: f32,
        launch_heading: f32,
        target_uuid: Option<String>,
        source_uuid: Option<String>,
    ) -> LaunchResult {
        let lifespan = self.config.lifespan;
        let burst_interval = self.config.burst_interval_secs;
        let shield_pierce = self.config.shield_pierce;
        let Some(idx) = self.tubes.iter().position(|t| t.id == tube_id) else {
            return LaunchResult::UnknownTube;
        };
        if self.tubes[idx].loaded_count == 0 {
            return LaunchResult::TubeNotLoaded;
        }
        let count = self.tubes[idx].loaded_count;
        // Resolve the tube's barrel firing sequence up-front (origin map only —
        // never affects `count`).
        let sequence = self.tubes[idx].barrel_sequence();
        // Resolve the origin + (barrel, step) for a given 0-based volley round.
        let resolve = |round: u32| -> ((f32, f32, f32), u32, u32) {
            let (barrel, step) = sequence[(round as usize) % sequence.len()];
            let origin = barrel_origins
                .get(barrel as usize)
                .copied()
                .or_else(|| barrel_origins.first().copied())
                .unwrap_or((launch_x, launch_y, launch_z));
            (origin, barrel, step)
        };

        self.tubes[idx].mark_fired(); // sets loaded_count = 0, load_state = Unloaded

        // Round 0: the immediate launch.
        let ((origin_x, origin_y, origin_z), barrel0, step0) = resolve(0);
        self.tubes[idx].active_barrels = vec![barrel0];
        self.tubes[idx].pattern_step = step0;
        self.in_flight.push(Torpedo {
            uuid: uuid.clone(),
            x: origin_x,
            y: origin_y,
            z: origin_z,
            heading: launch_heading,
            pitch: 0.0,
            lifespan_remaining: lifespan,
            target_uuid: target_uuid.clone(),
            source_uuid: source_uuid.clone(),
            tube_id: tube_id.to_string(),
            shield_pierce,
        });

        let count_remaining = count - 1;
        // Schedule the remaining burst torpedoes — count bounded by loaded_count.
        if count_remaining > 0 {
            self.burst_states.retain(|b| b.tube_id != tube_id);
            self.burst_states.push(TubeBurstState {
                tube_id: tube_id.to_string(),
                pending: count_remaining,
                timer: burst_interval,
                launch_x,
                launch_y,
                launch_z,
                launch_heading,
                target_uuid,
                source_uuid,
                barrel_origins: barrel_origins.to_vec(),
                barrel_sequence: sequence,
                next_shot_index: 1,
            });
        }
        LaunchResult::Launched {
            uuid,
            count_remaining,
        }
    }

    pub fn tick(
        &mut self,
        dt: f32,
        target_positions: &std::collections::HashMap<String, (f32, f32, f32)>,
        next_uuid: &mut impl FnMut() -> String,
    ) -> TorpedoTickResult {
        let mut result = TorpedoTickResult::default();

        // Ã¢â€â‚¬Ã¢â€â‚¬ Tube load/unload progression Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬
        let mut unload_completions: u32 = 0;
        for t in &mut self.tubes {
            let outcome = t.tick(dt);
            match outcome {
                LoadTickOutcome::LoadedOne => {
                    t.loaded_count += 1;
                }
                LoadTickOutcome::UnloadedOne => {
                    if t.loaded_count > 0 {
                        t.loaded_count -= 1;
                    }
                    unload_completions += 1;
                }
                LoadTickOutcome::None => {}
            }
        }
        // Return unloaded torpedoes to the magazine.
        self.torpedoes_remaining += unload_completions;

        // Ã¢â€â‚¬Ã¢â€â‚¬ Auto-load toward target_count Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬
        let n_tubes = self.tubes.len();
        for i in 0..n_tubes {
            let tube = &self.tubes[i];
            if tube.target_count > 0
                && !matches!(
                    tube.load_state,
                    TubeLoadState::Loading { .. } | TubeLoadState::Unloading { .. }
                )
                && tube.loaded_count < tube.target_count
                && tube.loaded_count < tube.volley_max
                && self.torpedoes_remaining > 0
            {
                self.torpedoes_remaining -= 1;
                // start_load() requires Unloaded state, so reset from Loaded first
                self.tubes[i].load_state = TubeLoadState::Unloaded;
                self.tubes[i].start_load();
            }
        }

        // Ã¢â€â‚¬Ã¢â€â‚¬ Auto-unload toward target_count Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬
        for i in 0..n_tubes {
            let tube = &self.tubes[i];
            if !matches!(
                tube.load_state,
                TubeLoadState::Loading { .. } | TubeLoadState::Unloading { .. }
            ) && tube.loaded_count > tube.target_count
            {
                let lt = tube.load_time;
                self.tubes[i].load_state = TubeLoadState::Unloading {
                    remaining: lt,
                    total: lt,
                };
            }
        }

        // Ã¢â€â‚¬Ã¢â€â‚¬ Burst-fire timer Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬
        let lifespan = self.config.lifespan;
        let shield_pierce = self.config.shield_pierce;
        let burst_interval = self.config.burst_interval_secs;
        // Collect burst launches to avoid split borrows.
        let mut burst_torpedoes: Vec<Torpedo> = Vec::new();
        let mut burst_events: Vec<(String, String, f32, f32, f32, f32)> = Vec::new();
        let mut completed_bursts: Vec<usize> = Vec::new();
        // Per-tube pattern-state updates (tube_id, barrel, step) to apply after
        // the burst loop — the tubes and burst_states both live on `self`, so a
        // deferred pass avoids a split borrow (issue #766).
        let mut pattern_updates: Vec<(String, u32, u32)> = Vec::new();
        for (i, burst) in self.burst_states.iter_mut().enumerate() {
            burst.timer -= dt;
            if burst.timer <= 0.0 && burst.pending > 0 {
                let uuid = next_uuid();
                // Resolve this burst shot's origin from the barrel sequence
                // captured at fire time (issue #766). A legacy single-origin
                // burst (empty sequence/origins) falls back to launch_x/z.
                let (origin_x, origin_y, origin_z, barrel, step) =
                    if burst.barrel_sequence.is_empty() {
                        (burst.launch_x, burst.launch_y, burst.launch_z, 0u32, 0u32)
                    } else {
                        let (barrel, step) = burst.barrel_sequence
                            [(burst.next_shot_index as usize) % burst.barrel_sequence.len()];
                        let (ox, oy, oz) = burst
                            .barrel_origins
                            .get(barrel as usize)
                            .copied()
                            .or_else(|| burst.barrel_origins.first().copied())
                            .unwrap_or((burst.launch_x, burst.launch_y, burst.launch_z));
                        (ox, oy, oz, barrel, step)
                    };
                burst.next_shot_index += 1;
                pattern_updates.push((burst.tube_id.clone(), barrel, step));
                burst_torpedoes.push(Torpedo {
                    uuid: uuid.clone(),
                    x: origin_x,
                    y: origin_y,
                    z: origin_z,
                    heading: burst.launch_heading,
                    pitch: 0.0,
                    lifespan_remaining: lifespan,
                    target_uuid: burst.target_uuid.clone(),
                    source_uuid: burst.source_uuid.clone(),
                    tube_id: burst.tube_id.clone(),
                    shield_pierce,
                });
                burst_events.push((
                    burst.tube_id.clone(),
                    uuid,
                    origin_x,
                    origin_y,
                    origin_z,
                    burst.launch_heading,
                ));
                burst.pending -= 1;
                if burst.pending > 0 {
                    burst.timer = burst_interval;
                } else {
                    completed_bursts.push(i);
                }
            }
        }
        self.in_flight.extend(burst_torpedoes);
        result.burst_launched.extend(burst_events);
        // Reflect the most recent burst shot's barrel/step on the tube so the
        // blackboard/Tactical indicator tracks the active patterned attack.
        for (tube_id, barrel, step) in pattern_updates {
            if let Some(tube) = self.tubes.iter_mut().find(|t| t.id == tube_id) {
                tube.active_barrels = vec![barrel];
                tube.pattern_step = step;
            }
        }
        // Remove completed burst states (in reverse to preserve indices).
        for &i in completed_bursts.iter().rev() {
            self.burst_states.remove(i);
        }

        // Ã¢â€â‚¬Ã¢â€â‚¬ In-flight torpedo movement Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬
        let config = &self.config;
        for t in &mut self.in_flight {
            let pos = t
                .target_uuid
                .as_ref()
                .and_then(|id| target_positions.get(id))
                .copied();
            t.tick(dt, pos, config);
            if t.is_expired() {
                result.expired.push(t.uuid.clone());
            }
        }
        self.in_flight.retain(|t| !t.is_expired());
        result
    }

    pub fn handle_collision(&mut self, torpedo_uuid: &str) -> Option<i32> {
        self.handle_collision_full(torpedo_uuid)
            .map(|d| d.damage_hull)
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
            source_uuid: removed.source_uuid,
            tube_id: removed.tube_id,
            impact_x: removed.x,
            impact_y: removed.y,
            impact_z: removed.z,
        })
    }

    /// Set the volley target count for the given tube, clamped to
    /// `[0, tube.volley_max]`. Returns `false` if the tube id is unknown.
    pub fn set_volley_target(&mut self, tube_id: &str, count: u32) -> bool {
        let Some(tube) = self.tube_mut(tube_id) else {
            return false;
        };
        tube.target_count = count.min(tube.volley_max);
        true
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
#[derive(Clone, Debug, PartialEq)]
pub struct TorpedoDetonation {
    pub damage_hull: i32,
    pub damage_shields: i32,
    pub shield_pierce: f32,
    /// UUID of the ship that launched the torpedo, carried through from
    /// [`Torpedo::source_uuid`] so the damage system can attribute the hit.
    /// `None` for torpedoes launched by the legacy resource-only test paths.
    pub source_uuid: Option<String>,
    /// Id of the tube that fired it, carried through from [`Torpedo::tube_id`]
    /// so balance telemetry can report the tube rather than a generic
    /// `"torpedo"` label.
    pub tube_id: String,
    /// Where the torpedo was when it went off, carried through from
    /// [`Torpedo::x`] / [`Torpedo::z`].
    ///
    /// # Why the caller needs this
    ///
    /// Shield arcs are directional: `ShieldSystem::apply_damage` routes a hit
    /// to a facing via `facing_index_for_bearing`, so it needs the bearing the
    /// blow arrived on. The torpedo is removed from `in_flight` right here, so
    /// by the time the damage system runs there is nothing left to ask — the
    /// impact point has to ride along with the detonation, exactly as
    /// `source_uuid` and `tube_id` already do. Callers pair it with the
    /// victim's own position and yaw via
    /// [`attacker_bearing_relative`](crate::shield::attacker_bearing_relative).
    ///
    /// It is the *torpedo's* position, not the firing ship's: a homing torpedo
    /// curves, and the arc it meets is the one it is nose-on to at detonation.
    pub impact_x: f32,
    /// Vertical position at detonation (issue #768). `0.0` for a torpedo that
    /// stayed on the play plane, so shield routing is unchanged for Planar
    /// engagements.
    pub impact_y: f32,
    pub impact_z: f32,
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
    /// `targets` is a slice of `(uuid, x, y, z, radius)` tuples (issue #768
    /// threads the target Y so vertical separation counts toward the 3D
    /// distance). With every `y == 0` the `dy` term vanishes and the check
    /// reduces exactly to the pre-#768 XZ comparison.
    pub fn find_detonation_hits(
        &self,
        targets: &[(String, f32, f32, f32, f32)],
    ) -> Vec<(String, String)> {
        let det = self.config.detonation_radius;
        let mut hits = Vec::new();
        for torpedo in &self.in_flight {
            let mut best: Option<(f32, &String)> = None;
            for (uuid, tx, ty, tz, radius) in targets {
                if torpedo.source_uuid.as_ref() == Some(uuid) {
                    continue;
                }
                let dx = tx - torpedo.x;
                let dy = ty - torpedo.y;
                let dz = tz - torpedo.z;
                let dist_sq = dx * dx + dy * dy + dz * dz;
                let threshold = det + radius;
                if dist_sq <= threshold * threshold
                    && best.map(|(d, _)| dist_sq < d).unwrap_or(true)
                {
                    best = Some((dist_sq, uuid));
                }
            }
            if let Some((_, uuid)) = best {
                hits.push((torpedo.uuid.clone(), uuid.clone()));
            }
        }
        hits
    }
}

// Ã¢â€â‚¬Ã¢â€â‚¬ private math helpers Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬

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

// Ã¢â€â‚¬Ã¢â€â‚¬ Tests Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬

#[cfg(test)]
mod tests {
    #![allow(clippy::field_reassign_with_default)]

    use super::*;
    use crate::entities::config::TorpedoTubeConfig;
    use std::collections::HashMap;

    fn cfg(id: &str, facing_deg: f32, fire_arc_deg: f32) -> TorpedoTubeConfig {
        TorpedoTubeConfig {
            id: id.into(),
            facing_deg,
            fire_arc_deg,
            load_time: None,
            marker: None,
            barrels: Vec::new(),
            pattern: Vec::new(),
            volley_max: 1,
            ai_target_count: None,
            ai: None,
        }
    }

    fn no_uuid() -> String {
        "test-uuid".to_string()
    }

    fn default_system() -> TorpedoSystem {
        let tubes = vec![
            cfg("fore_port", -30.0, 90.0),
            cfg("fore_starboard", 30.0, 90.0),
            cfg("aft", 180.0, 90.0),
        ];
        TorpedoSystem::from_configs(&tubes, TorpedoConfig::default())
    }

    fn load_tube(sys: &mut TorpedoSystem, id: &str) {
        let load_time = sys.tube(id).unwrap().load_time;
        // Set target_count = 1 before loading so the auto-unload logic
        // inside tick() does not immediately drain the torpedo we are about
        // to load (auto-unload fires when loaded_count > target_count).
        sys.tube_mut(id).unwrap().target_count = 1;
        assert!(sys.start_load(id));
        let targets: HashMap<String, (f32, f32, f32)> = HashMap::new();
        sys.tick(load_time, &targets, &mut no_uuid);
    }

    fn loaded_system() -> TorpedoSystem {
        let mut sys = default_system();
        load_tube(&mut sys, "fore_port");
        load_tube(&mut sys, "fore_starboard");
        load_tube(&mut sys, "aft");
        sys
    }

    #[test]
    fn tubes_start_unloaded() {
        let sys = default_system();
        assert!(sys.tubes.iter().all(|tube| !tube.is_loaded()));
        assert!(sys
            .tubes
            .iter()
            .all(|tube| tube.load_state == TubeLoadState::Unloaded));
    }

    #[test]
    fn launch_returns_launched_with_uuid() {
        let mut sys = loaded_system();
        let r = sys.launch("fore_port", "t1".into(), 0.0, 0.0, 0.0, 0.0, None, None);
        assert_eq!(
            r,
            LaunchResult::Launched {
                uuid: "t1".into(),
                count_remaining: 0
            }
        );
    }

    #[test]
    fn launch_adds_torpedo_to_in_flight() {
        let mut sys = loaded_system();
        sys.launch("fore_port", "t1".into(), 0.0, 0.0, 0.0, 0.0, None, None);
        assert_eq!(sys.in_flight.len(), 1);
        assert_eq!(sys.in_flight[0].uuid, "t1");
    }

    #[test]
    fn salvo_shortfall_counts_only_rounds_not_yet_committed() {
        // Three tubes at volley_max 1, all empty: three rounds short.
        let mut sys = default_system();
        assert_eq!(sys.salvo_shortfall(), 3);

        // A round already claimed for an in-progress load is NOT counted again —
        // `start_load` has already taken it out of the magazine, so counting the
        // gap it has not yet filled would charge for it twice and make a ship
        // with exactly enough rounds look one short for every load cycle.
        sys.tubes[0].target_count = 1;
        assert!(sys.start_load("fore_port"));
        assert_eq!(sys.salvo_shortfall(), 2);

        // And once it lands, the shortfall stays where it was: the round moved
        // from `Loading` into `loaded_count`, it did not appear from nowhere.
        let load_time = sys.tube("fore_port").unwrap().load_time;
        sys.tick(load_time, &HashMap::new(), &mut no_uuid);
        assert_eq!(sys.tube("fore_port").unwrap().loaded_count, 1);
        assert_eq!(sys.salvo_shortfall(), 2);
    }

    #[test]
    fn salvo_shortfall_is_zero_for_a_full_battery_and_for_no_tubes() {
        assert_eq!(loaded_system().salvo_shortfall(), 0);

        let mut config = TorpedoConfig::default();
        config.count = 0;
        let tubeless = TorpedoSystem::from_configs(&[], config);
        assert_eq!(
            tubeless.salvo_shortfall(),
            0,
            "a hull with no tubes is short of nothing — callers asking `can I fill \
             my tubes` must rule the tubeless case out themselves"
        );
    }

    #[test]
    fn start_load_decrements_torpedo_count() {
        let mut sys = default_system();
        assert_eq!(sys.torpedoes_remaining, 10);
        assert!(sys.start_load("fore_port"));
        assert_eq!(sys.torpedoes_remaining, 9);
    }

    #[test]
    fn start_load_fails_when_no_torpedoes() {
        let mut config = TorpedoConfig::default();
        config.count = 0;
        let tubes = vec![cfg("fore_port", -30.0, 90.0)];
        let mut sys = TorpedoSystem::from_configs(&tubes, config);
        assert!(!sys.start_load("fore_port"));
        assert_eq!(sys.torpedoes_remaining, 0);
    }

    // Ã¢â€â‚¬Ã¢â€â‚¬ channel-2 magazine claim helpers (issue #512) Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬

    #[test]
    fn claim_magazine_round_decrements_when_available() {
        let mut sys = default_system();
        assert_eq!(sys.torpedoes_remaining, 10);
        assert!(sys.claim_magazine_round());
        assert_eq!(sys.torpedoes_remaining, 9);
    }

    #[test]
    fn claim_magazine_round_returns_false_when_empty() {
        let mut config = TorpedoConfig::default();
        config.count = 0;
        let tubes = vec![cfg("fore_port", -30.0, 90.0)];
        let mut sys = TorpedoSystem::from_configs(&tubes, config);
        assert!(!sys.claim_magazine_round());
        assert_eq!(sys.torpedoes_remaining, 0);
    }

    #[test]
    fn start_load_reserved_begins_loading_without_touching_magazine() {
        let mut sys = default_system();
        assert_eq!(sys.torpedoes_remaining, 10);
        assert!(sys.start_load_reserved("fore_port"));
        assert_eq!(
            sys.torpedoes_remaining, 10,
            "reserved load must not touch the magazine counter (caller decremented already)"
        );
        assert!(matches!(
            sys.tube("fore_port").unwrap().load_state,
            TubeLoadState::Loading { .. }
        ));
    }

    #[test]
    fn start_load_reserved_fails_for_unknown_tube() {
        let mut sys = default_system();
        assert!(!sys.start_load_reserved("dorsal"));
    }

    #[test]
    fn start_load_reserved_fails_when_tube_not_unloaded() {
        let mut sys = default_system();
        assert!(sys.start_load_reserved("fore_port"));
        // Second call must fail because tube is now Loading.
        assert!(!sys.start_load_reserved("fore_port"));
    }

    #[test]
    fn launch_does_not_change_torpedo_count() {
        let mut sys = loaded_system();
        sys.launch("fore_port", "t1".into(), 0.0, 0.0, 0.0, 0.0, None, None);
        // Count was decremented at load time, not at launch
        assert_eq!(sys.torpedoes_remaining, 7);
    }

    #[test]
    fn start_load_fails_for_unknown_tube() {
        let mut sys = default_system();
        assert!(!sys.start_load("dorsal"));
        assert_eq!(sys.torpedoes_remaining, 10);
    }

    #[test]
    fn launch_leaves_tube_unloaded_until_manual_load() {
        let mut sys = loaded_system();
        sys.launch("fore_port", "t1".into(), 0.0, 0.0, 0.0, 0.0, None, None);
        assert!(!sys.tube("fore_port").unwrap().is_loaded());
        assert_eq!(
            sys.tube("fore_port").unwrap().load_state,
            TubeLoadState::Unloaded
        );
        // Disable auto-management on this tube: the test verifies that a
        // manual launch does NOT trigger an automatic reload on its own.
        // (Auto-management only reloads when target_count > loaded_count.)
        sys.tube_mut("fore_port").unwrap().target_count = 0;
        let targets: HashMap<String, (f32, f32, f32)> = HashMap::new();
        sys.tick(sys.config.load_time, &targets, &mut no_uuid);
        assert_eq!(
            sys.tube("fore_port").unwrap().load_state,
            TubeLoadState::Unloaded
        );
    }

    #[test]
    fn launch_from_unloaded_tube_returns_not_loaded() {
        let mut sys = default_system();
        let r = sys.launch("fore_port", "t1".into(), 0.0, 0.0, 0.0, 0.0, None, None);
        assert_eq!(r, LaunchResult::TubeNotLoaded);
    }

    #[test]
    fn launch_from_unknown_tube_returns_unknown() {
        let mut sys = default_system();
        let r = sys.launch("dorsal", "t1".into(), 0.0, 0.0, 0.0, 0.0, None, None);
        assert_eq!(r, LaunchResult::UnknownTube);
    }

    #[test]
    fn unload_cancelling_load_returns_torpedo_to_pool() {
        let mut sys = default_system();
        assert!(sys.start_load("fore_port"));
        assert_eq!(sys.torpedoes_remaining, 9);
        assert!(sys.start_unload("fore_port"));
        assert_eq!(sys.torpedoes_remaining, 10);
    }

    #[test]
    fn start_unload_loaded_tube_starts_timer_torpedo_returned_on_completion() {
        let mut sys = default_system();
        load_tube(&mut sys, "fore_port");
        assert_eq!(sys.torpedoes_remaining, 9);
        assert!(sys.start_unload("fore_port"));
        assert_eq!(sys.torpedoes_remaining, 9); // not returned yet
                                                // Disable auto-management so the manual-unload test is isolated: after
                                                // the timer fires we expect exactly one torpedo to return to the pool
                                                // and no auto-reload to start.
        sys.tube_mut("fore_port").unwrap().target_count = 0;
        let targets: HashMap<String, (f32, f32, f32)> = HashMap::new();
        sys.tick(
            sys.tube("fore_port").unwrap().load_time,
            &targets,
            &mut no_uuid,
        );
        assert_eq!(sys.torpedoes_remaining, 10); // returned after timer
    }

    #[test]
    fn start_unload_on_unloaded_tube_does_nothing() {
        let mut sys = default_system();
        assert!(!sys.start_unload("fore_port"));
        assert_eq!(sys.torpedoes_remaining, 10);
    }

    #[test]
    fn start_unload_on_unloading_tube_does_nothing() {
        let mut sys = default_system();
        load_tube(&mut sys, "fore_port");
        assert!(sys.start_unload("fore_port"));
        // Already unloading Ã¢â‚¬â€ second call does nothing
        assert!(!sys.start_unload("fore_port"));
    }

    #[test]
    fn start_unload_unknown_tube_returns_false() {
        let mut sys = default_system();
        assert!(!sys.start_unload("dorsal"));
    }

    #[test]
    fn can_launch_from_all_three_tubes_independently() {
        let mut sys = loaded_system();
        let r1 = sys.launch("fore_port", "t1".into(), 0.0, 0.0, 0.0, 0.0, None, None);
        let r2 = sys.launch(
            "fore_starboard",
            "t2".into(),
            0.0,
            0.0,
            0.0,
            0.0,
            None,
            None,
        );
        let r3 = sys.launch("aft", "t3".into(), 0.0, 0.0, 0.0, 0.0, None, None);
        assert!(matches!(r1, LaunchResult::Launched { .. }));
        assert!(matches!(r2, LaunchResult::Launched { .. }));
        assert!(matches!(r3, LaunchResult::Launched { .. }));
        assert_eq!(sys.in_flight.len(), 3);
    }

    #[test]
    fn torpedo_with_no_target_flies_straight() {
        let mut sys = loaded_system();
        sys.launch("fore_port", "t1".into(), 0.0, 0.0, 0.0, 0.0, None, None);
        let initial = sys.in_flight[0].heading;
        let targets: HashMap<String, (f32, f32, f32)> = HashMap::new();
        sys.tick(0.1, &targets, &mut no_uuid);
        assert_eq!(sys.in_flight[0].heading, initial);
    }

    #[test]
    fn torpedo_moves_forward_in_straight_flight() {
        let mut sys = loaded_system();
        sys.launch("fore_port", "t1".into(), 0.0, 0.0, 0.0, 0.0, None, None);
        let targets: HashMap<String, (f32, f32, f32)> = HashMap::new();
        sys.tick(1.0, &targets, &mut no_uuid);
        let t = &sys.in_flight[0];
        assert!(t.x.abs() < 0.01);
        assert!(t.z < 0.0);
    }

    #[test]
    fn torpedo_homes_toward_target() {
        let mut sys = loaded_system();
        sys.launch(
            "fore_port",
            "t1".into(),
            0.0,
            0.0,
            0.0,
            0.0,
            Some("enemy".into()),
            None,
        );
        let mut targets = HashMap::new();
        targets.insert("enemy".into(), (20.0_f32, 0.0_f32, 0.0_f32));
        let h0 = sys.in_flight[0].heading;
        sys.tick(0.1, &targets, &mut no_uuid);
        assert!(sys.in_flight[0].heading > h0);
    }

    #[test]
    fn torpedo_turn_rate_is_limited() {
        let mut config = TorpedoConfig::default();
        config.turn_rate = PI / 4.0;
        let tubes = vec![cfg("fore_port", -30.0, 90.0)];
        let mut sys = TorpedoSystem::from_configs(&tubes, config);
        load_tube(&mut sys, "fore_port");
        sys.launch(
            "fore_port",
            "t1".into(),
            0.0,
            0.0,
            0.0,
            0.0,
            Some("enemy".into()),
            None,
        );
        let mut targets = HashMap::new();
        targets.insert("enemy".into(), (20.0_f32, 0.0_f32, 0.0_f32));
        sys.tick(1.0, &targets, &mut no_uuid);
        assert!(sys.in_flight[0].heading <= PI / 4.0 + 0.001);
    }

    #[test]
    fn torpedo_flies_straight_when_target_destroyed() {
        let mut sys = loaded_system();
        sys.launch(
            "fore_port",
            "t1".into(),
            0.0,
            0.0,
            0.0,
            0.0,
            Some("enemy".into()),
            None,
        );
        let targets: HashMap<String, (f32, f32, f32)> = HashMap::new();
        let h0 = sys.in_flight[0].heading;
        sys.tick(0.5, &targets, &mut no_uuid);
        assert_eq!(sys.in_flight[0].heading, h0);
    }

    #[test]
    fn torpedo_target_uuid_locked_at_launch_and_never_updated() {
        // Fire at "target-a". Then tick with positions for both "target-a"
        // (far right) and a new "target-b" (straight ahead). The torpedo must
        // keep homing toward "target-a", never re-routing to "target-b", and
        // its stored target_uuid must remain "target-a" throughout.
        let mut sys = loaded_system();
        sys.launch(
            "fore_port",
            "t1".into(),
            0.0,
            0.0,
            0.0,
            0.0,
            Some("target-a".into()),
            None,
        );
        let mut targets = HashMap::new();
        targets.insert("target-a".into(), (100.0_f32, 0.0_f32, 0.0_f32)); // hard right
        targets.insert("target-b".into(), (0.0_f32, 0.0_f32, -100.0_f32)); // straight ahead

        let h0 = sys.in_flight[0].heading;
        sys.tick(0.1, &targets, &mut no_uuid);

        // The torpedo must have turned right (toward target-a), not stayed straight.
        assert!(
            sys.in_flight[0].heading > h0,
            "should home toward target-a (rightward turn)"
        );
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
        load_tube(&mut sys, "fore_port");
        sys.launch("fore_port", "t1".into(), 0.0, 0.0, 0.0, 0.0, None, None);
        let targets: HashMap<String, (f32, f32, f32)> = HashMap::new();
        let r = sys.tick(5.1, &targets, &mut no_uuid);
        assert!(r.expired.contains(&"t1".to_string()));
        assert_eq!(sys.in_flight.len(), 0);
    }

    #[test]
    fn torpedo_not_expired_before_lifespan() {
        let mut config = TorpedoConfig::default();
        config.lifespan = 5.0;
        let tubes = vec![cfg("fore_port", -30.0, 90.0)];
        let mut sys = TorpedoSystem::from_configs(&tubes, config);
        load_tube(&mut sys, "fore_port");
        sys.launch("fore_port", "t1".into(), 0.0, 0.0, 0.0, 0.0, None, None);
        let targets: HashMap<String, (f32, f32, f32)> = HashMap::new();
        let r = sys.tick(4.9, &targets, &mut no_uuid);
        assert!(!r.expired.contains(&"t1".to_string()));
        assert_eq!(sys.in_flight.len(), 1);
    }

    #[test]
    fn collision_removes_torpedo_and_returns_damage() {
        let mut sys = loaded_system();
        sys.launch("fore_port", "t1".into(), 0.0, 0.0, 0.0, 0.0, None, None);
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
    fn tube_loads_after_manual_load_time() {
        let mut config = TorpedoConfig::default();
        config.load_time = 10.0;
        let tubes = vec![cfg("fore_port", -30.0, 90.0)];
        let mut sys = TorpedoSystem::from_configs(&tubes, config);
        assert!(sys.start_load("fore_port"));
        assert!(!sys.tube("fore_port").unwrap().is_loaded());
        let targets: HashMap<String, (f32, f32, f32)> = HashMap::new();
        sys.tick(10.0, &targets, &mut no_uuid);
        assert!(sys.tube("fore_port").unwrap().is_loaded());
    }

    #[test]
    fn tube_not_loaded_before_manual_load_time_expires() {
        let mut config = TorpedoConfig::default();
        config.load_time = 10.0;
        let tubes = vec![cfg("fore_port", -30.0, 90.0)];
        let mut sys = TorpedoSystem::from_configs(&tubes, config);
        assert!(sys.start_load("fore_port"));
        let targets: HashMap<String, (f32, f32, f32)> = HashMap::new();
        sys.tick(9.9, &targets, &mut no_uuid);
        assert!(!sys.tube("fore_port").unwrap().is_loaded());
    }

    // Ã¢â€â‚¬Ã¢â€â‚¬ proximity detonation Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬

    fn detonation_system(detonation_radius: f32) -> TorpedoSystem {
        let mut config = TorpedoConfig::default();
        config.detonation_radius = detonation_radius;
        let tubes = vec![cfg("fore_port", -30.0, 90.0)];
        let mut sys = TorpedoSystem::from_configs(&tubes, config);
        load_tube(&mut sys, "fore_port");
        sys
    }

    #[test]
    fn find_detonation_hits_returns_empty_when_no_targets_in_range() {
        let mut sys = detonation_system(5.0);
        sys.launch("fore_port", "t1".into(), 0.0, 0.0, 0.0, 0.0, None, None);
        // Target far away with small radius.
        let targets = vec![("enemy".to_string(), 100.0, 0.0, 100.0, 1.0)];
        let hits = sys.find_detonation_hits(&targets);
        assert!(hits.is_empty());
    }

    #[test]
    fn find_detonation_hits_reports_target_within_detonation_radius() {
        let mut sys = detonation_system(5.0);
        sys.launch("fore_port", "t1".into(), 0.0, 0.0, 0.0, 0.0, None, None);
        // Target at (0, -4): distance 4, threshold 5+0 = 5.
        let targets = vec![("enemy".to_string(), 0.0, 0.0, -4.0, 0.0)];
        let hits = sys.find_detonation_hits(&targets);
        assert_eq!(hits, vec![("t1".to_string(), "enemy".to_string())]);
    }

    #[test]
    fn find_detonation_hits_includes_target_radius_in_threshold() {
        // Detonation radius 1, target radius 10, distance 9 Ã¢â€ â€™ should hit.
        let mut sys = detonation_system(1.0);
        sys.launch("fore_port", "t1".into(), 0.0, 0.0, 0.0, 0.0, None, None);
        let targets = vec![("rock".to_string(), 0.0, 0.0, -9.0, 10.0)];
        let hits = sys.find_detonation_hits(&targets);
        assert_eq!(hits, vec![("t1".to_string(), "rock".to_string())]);
    }

    #[test]
    fn find_detonation_hits_picks_nearest_when_multiple_in_range() {
        let mut sys = detonation_system(50.0);
        sys.launch("fore_port", "t1".into(), 0.0, 0.0, 0.0, 0.0, None, None);
        let targets = vec![
            ("far".to_string(), 0.0, 0.0, -40.0, 0.0),
            ("near".to_string(), 0.0, 0.0, -5.0, 0.0),
        ];
        let hits = sys.find_detonation_hits(&targets);
        assert_eq!(hits, vec![("t1".to_string(), "near".to_string())]);
    }

    #[test]
    fn find_detonation_hits_detonates_unlocked_torpedo_on_contact() {
        // Bug repro: shot without a target lock should still explode.
        let mut sys = detonation_system(5.0);
        sys.launch(
            "fore_port",
            "t1".into(),
            0.0,
            0.0,
            0.0,
            0.0,
            /*target_uuid*/ None,
            /*source_uuid*/ None,
        );
        let targets = vec![("raider".to_string(), 0.0, 0.0, -3.0, 1.0)];
        let hits = sys.find_detonation_hits(&targets);
        assert_eq!(hits, vec![("t1".to_string(), "raider".to_string())]);
    }

    #[test]
    fn find_detonation_hits_handles_multiple_torpedoes_independently() {
        let mut sys = detonation_system(2.0);
        sys.launch("fore_port", "t1".into(), 0.0, 0.0, 0.0, 0.0, None, None);
        // Manually push a second torpedo so the test can focus on detonation
        // matching rather than tube load state.
        sys.in_flight.push(Torpedo {
            uuid: "t2".into(),
            x: 100.0,
            y: 0.0,
            z: 100.0,
            heading: 0.0,
            pitch: 0.0,
            lifespan_remaining: 10.0,
            target_uuid: None,
            source_uuid: None,
            tube_id: "fore_port".into(),
            shield_pierce: 0.0,
        });
        let targets = vec![
            ("a".to_string(), 1.0, 0.0, 0.0, 0.0),     // close to t1
            ("b".to_string(), 101.0, 0.0, 100.0, 0.0), // close to t2
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
            0.0,
            None,
            Some("player-ship".into()),
        );
        // Player ship sitting right on top of the torpedo, plus a raider
        // also in range further out.
        let targets = vec![
            ("player-ship".to_string(), 0.0, 0.0, 0.0, 5.0),
            ("raider".to_string(), 0.0, 0.0, -3.0, 1.0),
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
            0.0,
            None,
            Some("player-ship".into()),
        );
        let targets = vec![("player-ship".to_string(), 0.0, 0.0, 0.0, 5.0)];
        let hits = sys.find_detonation_hits(&targets);
        assert!(hits.is_empty());
    }

    // Ã¢â€â‚¬Ã¢â€â‚¬ Volley mechanics (issue #632) Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬

    fn volley_cfg(id: &str, volley_max: u32) -> TorpedoTubeConfig {
        TorpedoTubeConfig {
            id: id.into(),
            facing_deg: 0.0,
            fire_arc_deg: 180.0,
            load_time: None,
            marker: None,
            barrels: Vec::new(),
            pattern: Vec::new(),
            volley_max,
            ai_target_count: None,
            ai: None,
        }
    }

    #[test]
    fn volley_max_defaults_to_1_on_standard_tube() {
        let sys = default_system();
        assert_eq!(sys.tube("fore_port").unwrap().volley_max, 1);
    }

    /// The AI's standing volley target is TOML, not a constant, and a hull
    /// that says nothing gets "keep the tube as full as it can".
    #[test]
    fn ai_target_count_defaults_to_volley_max() {
        let sys = TorpedoSystem::from_configs(&[volley_cfg("t1", 3)], TorpedoConfig::default());
        assert_eq!(sys.tube("t1").unwrap().ai_target_count, 3);
        // The tube still starts empty — the AI has to *ask* for the load.
        assert_eq!(sys.tube("t1").unwrap().target_count, 0);
    }

    #[test]
    fn ai_target_count_reads_the_ship_wide_default() {
        let config = TorpedoConfig {
            ai_volley_target: Some(2),
            ..Default::default()
        };
        let sys = TorpedoSystem::from_configs(&[volley_cfg("t1", 3)], config);
        assert_eq!(sys.tube("t1").unwrap().ai_target_count, 2);
    }

    #[test]
    fn per_tube_ai_target_count_overrides_the_ship_wide_default_and_clamps() {
        let config = TorpedoConfig {
            ai_volley_target: Some(2),
            ..Default::default()
        };
        let mut low = volley_cfg("low", 3);
        low.ai_target_count = Some(1);
        let mut greedy = volley_cfg("greedy", 3);
        greedy.ai_target_count = Some(9);
        let sys = TorpedoSystem::from_configs(&[low, greedy], config);
        assert_eq!(sys.tube("low").unwrap().ai_target_count, 1);
        assert_eq!(
            sys.tube("greedy").unwrap().ai_target_count,
            3,
            "a per-tube ai_target_count above volley_max clamps to what fits"
        );
    }

    #[test]
    fn set_volley_target_clamps_to_volley_max() {
        let mut sys = TorpedoSystem::from_configs(&[volley_cfg("t1", 3)], TorpedoConfig::default());
        assert!(sys.set_volley_target("t1", 5)); // 5 > 3, should clamp
        assert_eq!(sys.tube("t1").unwrap().target_count, 3);
    }

    #[test]
    fn set_volley_target_returns_false_for_unknown_tube() {
        let mut sys = default_system();
        assert!(!sys.set_volley_target("dorsal", 1));
    }

    #[test]
    fn auto_load_toward_target_count() {
        // Set target_count=2, volley_max=2. Tick twice through load_time.
        // Should end up with loaded_count==2 and torpedoes_remaining decremented by 2.
        let mut config = TorpedoConfig::default();
        config.count = 10;
        config.load_time = 5.0;
        let mut sys = TorpedoSystem::from_configs(&[volley_cfg("t1", 2)], config);
        sys.set_volley_target("t1", 2);
        let targets: HashMap<String, (f32, f32, f32)> = HashMap::new();
        // First tick starts loading torpedo #1.
        sys.tick(0.0, &targets, &mut no_uuid);
        assert_eq!(sys.torpedoes_remaining, 9);
        assert!(matches!(
            sys.tube("t1").unwrap().load_state,
            TubeLoadState::Loading { .. }
        ));
        // Tick past load_time: torpedo #1 finishes, torpedo #2 starts immediately.
        sys.tick(5.0, &targets, &mut no_uuid);
        assert_eq!(sys.tube("t1").unwrap().loaded_count, 1);
        assert_eq!(sys.torpedoes_remaining, 8); // second load started
                                                // Tick past load_time again: torpedo #2 finishes.
        sys.tick(5.0, &targets, &mut no_uuid);
        assert_eq!(sys.tube("t1").unwrap().loaded_count, 2);
        // No more auto-loads: loaded_count == target_count.
        assert_eq!(sys.torpedoes_remaining, 8);
    }

    #[test]
    fn fire_volley_fires_first_torpedo_immediately_rest_in_burst() {
        let mut config = TorpedoConfig::default();
        config.count = 10;
        config.load_time = 1.0;
        config.burst_interval_secs = 0.3;
        let tubes = vec![volley_cfg("t1", 3)];
        let mut sys = TorpedoSystem::from_configs(&tubes, config);
        // Load 3 torpedoes manually.
        sys.torpedoes_remaining -= 3;
        let tube = sys.tube_mut("t1").unwrap();
        tube.loaded_count = 3;
        let result = sys.launch("t1", "uuid-0".into(), 0.0, 0.0, 0.0, 0.0, None, None);
        assert!(matches!(
            result,
            LaunchResult::Launched {
                count_remaining: 2,
                ..
            }
        ));
        assert_eq!(sys.in_flight.len(), 1);
        assert_eq!(sys.burst_states.len(), 1);
        assert_eq!(sys.burst_states[0].pending, 2);
    }

    #[test]
    fn burst_launches_remaining_torpedoes_at_interval() {
        let mut config = TorpedoConfig::default();
        config.count = 10;
        config.burst_interval_secs = 0.3;
        let tubes = vec![volley_cfg("t1", 3)];
        let mut sys = TorpedoSystem::from_configs(&tubes, config);
        sys.torpedoes_remaining -= 3;
        let tube = sys.tube_mut("t1").unwrap();
        tube.loaded_count = 3;
        sys.launch("t1", "uuid-0".into(), 0.0, 0.0, 0.0, 0.0, None, None);
        assert_eq!(sys.in_flight.len(), 1);

        let targets: HashMap<String, (f32, f32, f32)> = HashMap::new();
        let mut uuid_counter = 1u32;
        let mut next = || {
            let s = format!("uuid-{uuid_counter}");
            uuid_counter += 1;
            s
        };
        // Tick past first burst interval: should fire torpedo #2.
        sys.tick(0.3, &targets, &mut next);
        assert_eq!(
            sys.in_flight.len(),
            2,
            "torpedo #2 should fire after interval"
        );
        // Tick past second interval: torpedo #3.
        sys.tick(0.3, &targets, &mut next);
        assert_eq!(
            sys.in_flight.len(),
            3,
            "torpedo #3 should fire after second interval"
        );
        assert!(sys.burst_states.is_empty(), "burst state should be cleared");
    }

    #[test]
    fn fire_with_partial_load_fires_what_is_loaded() {
        let mut config = TorpedoConfig::default();
        config.count = 10;
        let tubes = vec![volley_cfg("t1", 4)];
        let mut sys = TorpedoSystem::from_configs(&tubes, config);
        // Only 2 of 4 are loaded.
        sys.torpedoes_remaining -= 2;
        let tube = sys.tube_mut("t1").unwrap();
        tube.loaded_count = 2;
        let result = sys.launch("t1", "uuid-0".into(), 0.0, 0.0, 0.0, 0.0, None, None);
        assert!(matches!(
            result,
            LaunchResult::Launched {
                count_remaining: 1,
                ..
            }
        ));
        assert_eq!(sys.tube("t1").unwrap().loaded_count, 0);
    }

    #[test]
    fn auto_unload_when_target_count_decremented() {
        let mut config = TorpedoConfig::default();
        config.count = 10;
        config.load_time = 1.0;
        let tubes = vec![volley_cfg("t1", 3)];
        let mut sys = TorpedoSystem::from_configs(&tubes, config);
        // Start with 2 loaded, target_count = 2 (auto-managed mode).
        sys.torpedoes_remaining -= 2;
        {
            let tube = sys.tube_mut("t1").unwrap();
            tube.loaded_count = 2;
            tube.target_count = 2;
        }
        // Drop target to 1 â†’ should auto-unload one torpedo.
        sys.set_volley_target("t1", 1);
        let targets: HashMap<String, (f32, f32, f32)> = HashMap::new();
        // First tick starts unloading one.
        sys.tick(0.0, &targets, &mut no_uuid);
        assert!(matches!(
            sys.tube("t1").unwrap().load_state,
            TubeLoadState::Unloading { .. }
        ));
        // Complete the unload: loaded_count goes from 2 to 1.
        sys.tick(1.0, &targets, &mut no_uuid);
        assert_eq!(sys.tube("t1").unwrap().loaded_count, 1);
        // target_count == loaded_count now â†’ no more auto-unload.
        sys.tick(0.0, &targets, &mut no_uuid);
        assert_eq!(
            sys.tube("t1").unwrap().loaded_count,
            1,
            "should stop at target_count=1"
        );
        assert_eq!(
            sys.torpedoes_remaining, 9,
            "one torpedo returned to magazine"
        );
    }

    #[test]
    fn auto_unload_when_target_set_to_zero() {
        // Regression for issue #632: setting target_count=0 must drain all
        // loaded torpedoes back to the magazine automatically.
        let mut config = TorpedoConfig::default();
        config.count = 10;
        config.load_time = 1.0;
        let tubes = vec![volley_cfg("t1", 3)];
        let mut sys = TorpedoSystem::from_configs(&tubes, config);
        // Pre-load 2 torpedoes directly (bypass timer for test clarity).
        sys.torpedoes_remaining -= 2;
        {
            let tube = sys.tube_mut("t1").unwrap();
            tube.loaded_count = 2;
            tube.target_count = 0; // target is already 0
        }
        let targets: HashMap<String, (f32, f32, f32)> = HashMap::new();
        // First tick should start unloading the first torpedo.
        sys.tick(0.0, &targets, &mut no_uuid);
        assert!(
            matches!(
                sys.tube("t1").unwrap().load_state,
                TubeLoadState::Unloading { .. }
            ),
            "tube should be unloading after target_count set to 0"
        );
        // Complete the first unload.
        sys.tick(1.0, &targets, &mut no_uuid);
        assert_eq!(sys.tube("t1").unwrap().loaded_count, 1);
        // Next tick starts unloading the second torpedo.
        sys.tick(0.0, &targets, &mut no_uuid);
        assert!(matches!(
            sys.tube("t1").unwrap().load_state,
            TubeLoadState::Unloading { .. }
        ));
        // Complete the second unload.
        sys.tick(1.0, &targets, &mut no_uuid);
        assert_eq!(sys.tube("t1").unwrap().loaded_count, 0);
        assert_eq!(
            sys.tube("t1").unwrap().load_state,
            TubeLoadState::Unloaded,
            "tube should be fully unloaded"
        );
        assert_eq!(
            sys.torpedoes_remaining, 10,
            "both torpedoes returned to magazine"
        );
    }

    // ── Patterned multi-barrel attacks (issue #766) ──────────────────────────

    use crate::weapons::pattern::BarrelPatternStep;

    fn barrel_step(barrels: &[u32], offset: f32) -> BarrelPatternStep {
        BarrelPatternStep {
            barrels: barrels.to_vec(),
            offset_secs: offset,
        }
    }

    fn patterned_cfg(
        id: &str,
        barrels: Vec<String>,
        pattern: Vec<BarrelPatternStep>,
        volley_max: u32,
    ) -> TorpedoTubeConfig {
        TorpedoTubeConfig {
            id: id.into(),
            facing_deg: 0.0,
            fire_arc_deg: 180.0,
            load_time: None,
            marker: None,
            barrels,
            pattern,
            volley_max,
            ai_target_count: None,
            ai: None,
        }
    }

    /// Pre-load `n` rounds into `tube` directly, decrementing the magazine to
    /// mirror the load-time spend (so a later `launch` proves it does NOT spend
    /// again).
    fn preload(sys: &mut TorpedoSystem, tube: &str, n: u32) {
        sys.torpedoes_remaining -= n;
        sys.tube_mut(tube).unwrap().loaded_count = n;
    }

    fn burst_next() -> impl FnMut() -> String {
        let mut i = 100u32;
        move || {
            i += 1;
            format!("burst-{i}")
        }
    }

    #[test]
    fn patterned_alternating_launches_from_barrels_in_sequence() {
        // Two barrels, two steps at increasing offsets → the volley's rounds
        // leave from barrel 0 then barrel 1.
        let mut config = TorpedoConfig::default();
        config.count = 10;
        config.burst_interval_secs = 0.3;
        let cfg = patterned_cfg(
            "t1",
            vec!["b0".into(), "b1".into()],
            vec![barrel_step(&[0], 0.0), barrel_step(&[1], 0.5)],
            2,
        );
        let mut sys = TorpedoSystem::from_configs(&[cfg], config);
        preload(&mut sys, "t1", 2);

        // Distinct origins per barrel so a torpedo's X identifies its barrel.
        let origins = [(10.0, 0.0, 0.0), (20.0, 0.0, 0.0)];
        let r =
            sys.launch_with_barrels("t1", "u0".into(), &origins, 0.0, 0.0, 0.0, 0.0, None, None);
        assert!(matches!(
            r,
            LaunchResult::Launched {
                count_remaining: 1,
                ..
            }
        ));
        // Immediate round from barrel 0.
        assert_eq!(sys.in_flight.len(), 1);
        assert!((sys.in_flight[0].x - 10.0).abs() < 1e-4, "barrel 0 origin");
        assert_eq!(sys.tube("t1").unwrap().active_barrels, vec![0]);
        assert_eq!(sys.tube("t1").unwrap().pattern_step, 1);

        // Burst shot fires from barrel 1 after the burst interval.
        let targets: HashMap<String, (f32, f32, f32)> = HashMap::new();
        let mut next = burst_next();
        sys.tick(0.3, &targets, &mut next);
        assert_eq!(sys.in_flight.len(), 2);
        let burst = sys.in_flight.iter().find(|t| t.uuid != "u0").unwrap();
        assert!((burst.x - 20.0).abs() < 1e-4, "barrel 1 origin");
        assert_eq!(sys.tube("t1").unwrap().active_barrels, vec![1]);
        assert_eq!(sys.tube("t1").unwrap().pattern_step, 2);
    }

    #[test]
    fn patterned_simultaneous_launches_from_multiple_barrels() {
        // One step listing several barrels → consecutive rounds leave from each
        // listed barrel. With two loaded, both barrels are used.
        let mut config = TorpedoConfig::default();
        config.count = 10;
        config.burst_interval_secs = 0.3;
        let cfg = patterned_cfg(
            "t1",
            vec!["b0".into(), "b1".into()],
            vec![barrel_step(&[0, 1], 0.0)],
            2,
        );
        let mut sys = TorpedoSystem::from_configs(&[cfg], config);
        preload(&mut sys, "t1", 2);

        let origins = [(10.0, 0.0, 0.0), (20.0, 0.0, 0.0)];
        sys.launch_with_barrels("t1", "u0".into(), &origins, 0.0, 0.0, 0.0, 0.0, None, None);
        assert!((sys.in_flight[0].x - 10.0).abs() < 1e-4, "barrel 0 origin");

        let targets: HashMap<String, (f32, f32, f32)> = HashMap::new();
        let mut next = burst_next();
        sys.tick(0.3, &targets, &mut next);
        assert_eq!(sys.in_flight.len(), 2, "both barrels fire");
        let burst = sys.in_flight.iter().find(|t| t.uuid != "u0").unwrap();
        assert!((burst.x - 20.0).abs() < 1e-4, "barrel 1 origin");
    }

    /// AC3: a two-barrel simultaneous step with only ONE round loaded fires
    /// exactly one torpedo and leaves the magazine untouched. The pattern never
    /// invents rounds — `loaded_count` is the count authority.
    #[test]
    fn patterned_simultaneous_step_with_one_loaded_fires_exactly_one() {
        let mut config = TorpedoConfig::default();
        config.count = 10;
        let cfg = patterned_cfg(
            "t1",
            vec!["b0".into(), "b1".into()],
            vec![barrel_step(&[0, 1], 0.0)],
            2,
        );
        let mut sys = TorpedoSystem::from_configs(&[cfg], config);
        preload(&mut sys, "t1", 1);
        let mag_before = sys.torpedoes_remaining;

        let origins = [(10.0, 0.0, 0.0), (20.0, 0.0, 0.0)];
        let r =
            sys.launch_with_barrels("t1", "u0".into(), &origins, 0.0, 0.0, 0.0, 0.0, None, None);
        assert!(
            matches!(
                r,
                LaunchResult::Launched {
                    count_remaining: 0,
                    ..
                }
            ),
            "a step listing 2 barrels must NOT fire 2 when only 1 is loaded"
        );
        assert_eq!(sys.in_flight.len(), 1, "exactly one torpedo fired");
        assert!(
            sys.burst_states.is_empty(),
            "no burst scheduled for 1 round"
        );
        assert!(
            (sys.in_flight[0].x - 10.0).abs() < 1e-4,
            "first barrel only"
        );
        assert_eq!(
            sys.torpedoes_remaining, mag_before,
            "launch must not spend from the magazine (spend happens at load)"
        );
    }

    /// AC3: the burst count stays bounded by `loaded_count` even when the
    /// pattern is short — the barrel sequence cycles for origins but never
    /// extends the volley.
    #[test]
    fn patterned_burst_count_bounded_by_loaded_count() {
        let mut config = TorpedoConfig::default();
        config.count = 10;
        config.burst_interval_secs = 0.3;
        // Single-step, single-barrel pattern; two rounds loaded.
        let cfg = patterned_cfg("t1", vec!["b0".into()], vec![barrel_step(&[0], 0.0)], 3);
        let mut sys = TorpedoSystem::from_configs(&[cfg], config);
        preload(&mut sys, "t1", 2);
        let mag_before = sys.torpedoes_remaining;

        let origins = [(10.0, 0.0, 0.0)];
        let r =
            sys.launch_with_barrels("t1", "u0".into(), &origins, 0.0, 0.0, 0.0, 0.0, None, None);
        assert!(matches!(
            r,
            LaunchResult::Launched {
                count_remaining: 1,
                ..
            }
        ));
        let targets: HashMap<String, (f32, f32, f32)> = HashMap::new();
        let mut next = burst_next();
        // Drive well past several burst intervals.
        sys.tick(0.3, &targets, &mut next);
        sys.tick(0.3, &targets, &mut next);
        sys.tick(0.3, &targets, &mut next);
        assert_eq!(
            sys.in_flight.len(),
            2,
            "exactly loaded_count torpedoes fire — no more"
        );
        assert!(sys.burst_states.is_empty());
        assert_eq!(
            sys.torpedoes_remaining, mag_before,
            "magazine untouched by patterned firing"
        );
    }

    #[test]
    fn legacy_launch_without_barrels_uses_ship_centre_origin() {
        // Back-compat: no barrels/pattern authored → every round leaves from the
        // passed launch origin exactly as before issue #766.
        let mut config = TorpedoConfig::default();
        config.count = 10;
        config.burst_interval_secs = 0.3;
        let mut sys = TorpedoSystem::from_configs(&[volley_cfg("t1", 2)], config);
        preload(&mut sys, "t1", 2);
        sys.launch("t1", "u0".into(), 7.0, 0.0, -3.0, 0.0, None, None);
        assert!((sys.in_flight[0].x - 7.0).abs() < 1e-4);
        assert!((sys.in_flight[0].z - (-3.0)).abs() < 1e-4);
        // Legacy tube reports no pattern → no step indicator.
        assert_eq!(sys.tube("t1").unwrap().pattern_len(), 0);
        let targets: HashMap<String, (f32, f32, f32)> = HashMap::new();
        let mut next = burst_next();
        sys.tick(0.3, &targets, &mut next);
        let burst = sys.in_flight.iter().find(|t| t.uuid != "u0").unwrap();
        assert!((burst.x - 7.0).abs() < 1e-4, "burst from ship centre too");
    }

    // ── Full-3D torpedo flight (issue #768) ──────────────────────────────────

    /// A torpedo fired level at a target ABOVE it climbs: its `y` and `pitch`
    /// both increase as it homes toward the target's altitude. Vertical
    /// separation therefore changes guidance (AC2).
    #[test]
    fn torpedo_climbs_toward_target_above() {
        let mut sys = loaded_system();
        sys.launch(
            "fore_port",
            "t1".into(),
            0.0,
            0.0,
            0.0,
            0.0,
            Some("enemy".into()),
            None,
        );
        // Target dead ahead (−Z) but 40 m up.
        let mut targets = HashMap::new();
        targets.insert("enemy".into(), (0.0_f32, 40.0_f32, -40.0_f32));
        assert_eq!(sys.in_flight[0].y, 0.0);
        assert_eq!(sys.in_flight[0].pitch, 0.0);
        sys.tick(0.5, &targets, &mut no_uuid);
        assert!(
            sys.in_flight[0].pitch > 0.0,
            "pitch should tilt up toward the higher target"
        );
        assert!(
            sys.in_flight[0].y > 0.0,
            "torpedo should gain altitude climbing toward the target"
        );
    }

    /// Mirror of the climb case: a target BELOW drives a descent (negative
    /// pitch, decreasing `y`).
    #[test]
    fn torpedo_descends_toward_target_below() {
        let mut sys = loaded_system();
        sys.launch(
            "fore_port",
            "t1".into(),
            0.0,
            0.0,
            0.0,
            0.0,
            Some("enemy".into()),
            None,
        );
        let mut targets = HashMap::new();
        targets.insert("enemy".into(), (0.0_f32, -40.0_f32, -40.0_f32));
        sys.tick(0.5, &targets, &mut no_uuid);
        assert!(sys.in_flight[0].pitch < 0.0, "pitch should tilt down");
        assert!(sys.in_flight[0].y < 0.0, "torpedo should lose altitude");
    }

    /// The vertical steering is rate-limited by the SAME `turn_rate` clamp as
    /// the yaw: over one second the pitch cannot exceed `turn_rate` radians even
    /// when the target sits straight overhead (desired pitch = +π/2).
    #[test]
    fn vertical_steering_is_rate_limited_by_turn_rate() {
        let mut config = TorpedoConfig::default();
        config.turn_rate = PI / 4.0;
        let tubes = vec![cfg("fore_port", -30.0, 90.0)];
        let mut sys = TorpedoSystem::from_configs(&tubes, config);
        load_tube(&mut sys, "fore_port");
        sys.launch(
            "fore_port",
            "t1".into(),
            0.0,
            0.0,
            0.0,
            0.0,
            Some("enemy".into()),
            None,
        );
        // Target directly overhead → desired pitch is +π/2, far beyond one
        // second of turn budget.
        let mut targets = HashMap::new();
        targets.insert("enemy".into(), (0.0_f32, 1000.0_f32, 0.0_f32));
        sys.tick(1.0, &targets, &mut no_uuid);
        assert!(
            sys.in_flight[0].pitch <= PI / 4.0 + 1e-4,
            "pitch climb per second must be clamped to turn_rate"
        );
    }

    /// Vertical separation changes collision: same XZ, a large ΔY leaves the
    /// torpedo OUTSIDE the 3D detonation sphere (no hit), while a small ΔY is
    /// inside it (hit). AC2 / AC1.
    #[test]
    fn vertical_separation_governs_3d_collision() {
        let mut sys = detonation_system(5.0);
        sys.launch("fore_port", "t1".into(), 0.0, 0.0, 0.0, 0.0, None, None);
        // Same XZ as the torpedo (0,0), radius 0. ΔY = 20 ≫ det radius 5 → miss.
        let far_up = vec![("blimp".to_string(), 0.0, 20.0, 0.0, 0.0)];
        assert!(
            sys.find_detonation_hits(&far_up).is_empty(),
            "a torpedo 20 m below a target at the same XZ must NOT detonate"
        );
        // ΔY = 3 < det radius 5 → hit.
        let near_up = vec![("blimp".to_string(), 0.0, 3.0, 0.0, 0.0)];
        assert_eq!(
            sys.find_detonation_hits(&near_up),
            vec![("t1".to_string(), "blimp".to_string())],
            "within the 3D radius the torpedo detonates"
        );
    }

    /// The detonation payload carries the torpedo's vertical impact position, so
    /// a torpedo that climbed reports a non-zero `impact_y` (AC1: 3D detonation).
    #[test]
    fn detonation_carries_vertical_impact_point() {
        let mut sys = detonation_system(5.0);
        sys.launch(
            "fore_port",
            "t1".into(),
            0.0,
            0.0,
            0.0,
            0.0,
            Some("enemy".into()),
            None,
        );
        // Fly a while homing at a high target so the torpedo gains altitude.
        let mut targets = HashMap::new();
        targets.insert("enemy".into(), (0.0_f32, 100.0_f32, -60.0_f32));
        sys.tick(1.0, &targets, &mut no_uuid);
        sys.tick(1.0, &targets, &mut no_uuid);
        let expected_y = sys.in_flight[0].y;
        assert!(expected_y > 0.0, "torpedo should have climbed");
        let det = sys.handle_collision_full("t1").unwrap();
        assert!(
            (det.impact_y - expected_y).abs() < 1e-4,
            "impact_y must equal the torpedo's altitude at detonation"
        );
    }

    /// Patterned launch preserves the barrel marker's Y: each authored barrel
    /// origin is a full 3D point, so a round leaving a raised barrel spawns at
    /// that altitude (AC4: patterned origins carry Y).
    #[test]
    fn patterned_origins_carry_barrel_y() {
        let mut config = TorpedoConfig::default();
        config.count = 10;
        config.burst_interval_secs = 0.3;
        let cfg = patterned_cfg(
            "t1",
            vec!["b0".into(), "b1".into()],
            vec![barrel_step(&[0], 0.0), barrel_step(&[1], 0.5)],
            2,
        );
        let mut sys = TorpedoSystem::from_configs(&[cfg], config);
        preload(&mut sys, "t1", 2);
        // Distinct Y per barrel so a torpedo's altitude identifies its barrel.
        let origins = [(10.0, 2.0, 0.0), (20.0, -3.0, 0.0)];
        sys.launch_with_barrels("t1", "u0".into(), &origins, 0.0, 0.0, 0.0, 0.0, None, None);
        assert!(
            (sys.in_flight[0].y - 2.0).abs() < 1e-4,
            "immediate round keeps barrel 0's Y"
        );
        let targets: HashMap<String, (f32, f32, f32)> = HashMap::new();
        let mut next = burst_next();
        sys.tick(0.3, &targets, &mut next);
        let burst = sys.in_flight.iter().find(|t| t.uuid != "u0").unwrap();
        assert!(
            (burst.y - (-3.0)).abs() < 1e-4,
            "burst round keeps barrel 1's Y"
        );
    }

    /// AC3 planar collapse: with every Y at 0, the 3D distance check reduces
    /// EXACTLY to the 2D one. This target sits 6 m astern (ΔZ only) with the
    /// detonation radius at 5 — a boundary the 2D check also missed — proving no
    /// spurious `dy` term crept in.
    #[test]
    fn planar_collision_matches_2d_when_all_y_zero() {
        let mut sys = detonation_system(5.0);
        sys.launch("fore_port", "t1".into(), 0.0, 0.0, 0.0, 0.0, None, None);
        // distance 6 > 5 → miss, exactly as the pure XZ check would decide.
        let miss = vec![("e".to_string(), 0.0, 0.0, -6.0, 0.0)];
        assert!(sys.find_detonation_hits(&miss).is_empty());
        // distance 4 < 5 → hit.
        let hit = vec![("e".to_string(), 0.0, 0.0, -4.0, 0.0)];
        assert_eq!(
            sys.find_detonation_hits(&hit),
            vec![("t1".to_string(), "e".to_string())]
        );
    }
}
