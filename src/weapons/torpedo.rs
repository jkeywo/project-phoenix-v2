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

    /// Every round this ship still HAS: the magazine, plus the rounds already
    /// moved out of it into the tubes (issue #943).
    ///
    /// `torpedoes_remaining` alone is the rounds left to *reload* with, not the
    /// rounds aboard: the magazine is debited when a load STARTS
    /// ([`Self::start_load`], [`Self::claim_magazine_round`], the auto-load
    /// block in [`Self::tick`]), so a round parked in a tube has already left
    /// the counter. A hull whose tube doctrine keeps its tubes topped up
    /// therefore reads permanently short by its parked volley — the
    /// `alliance_destroyer` parks 3 of its 12 — and anything that rations
    /// rounds against that number strands the parked ones for ever.
    ///
    /// The three states are counted exactly once each, which is why this is a
    /// method and not a call-site sum:
    /// * `Loaded`/`Unloading` rounds sit in `loaded_count` (an `Unloading` tube
    ///   keeps its round there until [`Self::tick`] completes the unload and
    ///   moves it back to the magazine);
    /// * a `Loading` tube's round is in NEITHER field — already debited from
    ///   the magazine, not yet in `loaded_count` — so it is counted here from
    ///   the state itself, the same "already paid for" reading
    ///   [`Self::salvo_shortfall`] takes;
    /// * everything else is in `torpedoes_remaining`.
    ///
    /// Rounds in flight are NOT counted: they have been spent.
    pub fn rounds_aboard(&self) -> u32 {
        self.torpedoes_remaining
            + self
                .tubes
                .iter()
                .map(|t| {
                    t.loaded_count
                        + u32::from(matches!(t.load_state, TubeLoadState::Loading { .. }))
                })
                .sum::<u32>()
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
    /// [`crate::core::messages::InterSystemPayload::ClaimTorpedoRound`] transaction.
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
    /// [`crate::core::messages::InterSystemPayload::ClaimTorpedoRound`]. Returns
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
///   [`split_damage_for_pierce`](crate::ship::damage::split_damage_for_pierce)
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
    /// [`attacker_bearing_relative`](crate::weapons::shield::attacker_bearing_relative).
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
#[path = "torpedo_tests.rs"]
mod tests;
