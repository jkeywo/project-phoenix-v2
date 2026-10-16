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
//! Coordinate system: same as `ship_physics` / `radar` — XZ plane, Y-up.
//! Ship forward is −Z when yaw = 0.

use crate::resources::{reserve_requirements, ResourceRequirement, Stock};
use crate::simmath;
use crate::weapons::pattern::BarrelPattern;
use phoenix_model::guided_guidance::{GuidanceObservation, GuidanceSource};
pub use phoenix_model::guided_round::GuidedRoundProfile;
use std::f32::consts::PI;

/// String identifier for a torpedo tube (matches the `id` field in TOML).
pub type TorpedoTubeId = String;

/// Physical work requested by tube execution. The runtime resolves reserves;
/// this mechanism owns ammunition and cadence only.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TorpedoWork<'a> {
    Progress { tube: &'a str, dt: f32 },
    Transfer,
    Launch { tube: &'a str },
}

// ── Configuration ──────────────────────────────────────────────────────────

/// Tuning knobs for the torpedo system.
#[derive(Clone, Debug)]
pub struct TorpedoConfig {
    /// Total torpedoes available (shared pool).
    pub count: u32,
    pub initial_count: Option<u32>,
    /// What the round delivers when the arc it strikes is DOWN — landing in
    /// full on the hull, with no screen in the way to absorb or pierce
    /// (issue #929).
    ///
    /// This is the payload the fleet's torpedo doctrine is written around, and
    /// it is authored an order of magnitude above [`Self::damage_shields`]: the
    /// tubes' `fact(target_facing_shields) <= 0` launch gate trades patience for
    /// a payload, and that ratio is what makes waiting worth it.
    pub damage_hull: i32,
    /// What the round delivers when the arc it strikes is UP (issue #929).
    ///
    /// Offered to that facing through the same seam a beam takes — split by
    /// [`Self::shield_pierce`], the remainder absorbed by the arc, any overflow
    /// past its remaining HP spilling to hull. A torpedo does NOT ignore a live
    /// screen; a hull that authors `0` here is declaring that a round spent into
    /// one is a round wasted.
    pub damage_shields: i32,
    /// Physical interception durability carried by each launched round.
    pub durability: f32,
    pub seeker: Option<phoenix_model::guided_guidance::SeekerProfile>,
    pub receiver: Option<phoenix_model::guided_guidance::GuidanceReceiverProfile>,
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
    /// entity. Independent of homing — an un-locked torpedo still detonates
    /// on contact.
    pub detonation_radius: f32,
    /// Fraction of the `damage_shields` payload that bypasses the shield
    /// system entirely and adds to hull damage. Default `0.0` — all
    /// `damage_shields` is mitigated by the facing shield quadrant.
    /// Clamped to `[0.0, 1.0]` at apply time.
    ///
    /// It is a lever on the shields-UP payload only, and always was: the
    /// shields-down payload (`damage_hull`) meets no screen, so there is nothing
    /// there for it to get past (issue #929 — before that pass `damage_hull`
    /// bypassed the shield system unconditionally, which is a different thing
    /// entirely and is no longer true).
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
            initial_count: None,
            damage_hull: 50,
            damage_shields: 5,
            durability: 1.0,
            seeker: None,
            receiver: None,
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

// ── In-flight torpedo ──────────────────────────────────────────────────────

impl TorpedoConfig {
    pub fn round_profile(
        &self,
        impact: phoenix_model::damage::DamageProfile,
    ) -> GuidedRoundProfile {
        GuidedRoundProfile {
            marking: None,
            speed: self.speed,
            turn_rate: self.turn_rate,
            lifespan: self.lifespan,
            detonation_radius: self.detonation_radius,
            damage_hull: self.damage_hull,
            damage_shields: self.damage_shields,
            shield_pierce: self.shield_pierce,
            impact,
            durability: self.durability,
            seeker: self.seeker,
            receiver: self.receiver,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Torpedo {
    /// Actual previous pose for this Physics/Damage interval only. The normal
    /// collision owner clears it after relative CCD; saves/folds keep current
    /// canonical pose rather than a competing flight/history copy.
    pub previous_position: Option<[f32; 3]>,
    /// Actual live travel duration paired with this unsaved collision segment.
    pub collision_interval_secs: Option<f32>,
    /// One actual held measurement, retaining its original sample and source.
    pub last_guidance: Option<GuidanceObservation>,
    pub profile: GuidedRoundProfile,
    pub durability_remaining: f32,
    pub strike_damage_bonus: f32,
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
}

impl Torpedo {
    /// Private physical receiver seam. Caller has proved source ownership,
    /// dated Console permission, actual paired link and paid source work.
    /// No independent seeker may use this to acquire another target.
    pub fn receive_guidance(&mut self, observation: Option<&GuidanceObservation>) -> bool {
        if self.profile.receiver.is_none()
            || observation.is_some_and(|reading| {
                reading.validate(u64::MAX).is_err()
                    || !matches!(reading.source, GuidanceSource::Console { .. })
            })
        {
            return false;
        }
        self.target_uuid = observation.map(|reading| reading.target_uuid.clone());
        self.last_guidance = observation.cloned();
        true
    }

    /// External illumination supplies an observation only for existing intent;
    /// it cannot retarget or revive a stopped round. Producer validates actual
    /// logical time, paid emission and finite physical link before this seam.
    pub fn assist_guidance(
        &mut self,
        observation: &phoenix_model::guided_guidance::GuidanceObservation,
    ) -> bool {
        if !self
            .profile
            .receiver
            .is_some_and(|receiver| receiver.accept_external_illumination)
            || !matches!(
                observation.source,
                phoenix_model::guided_guidance::GuidanceSource::Console { .. }
            )
        {
            return false;
        }
        self.accept_guidance(observation)
    }

    /// Structural/intent acceptance only: the physical producer or link must
    /// validate the original sample and receipt frontier against its actual
    /// logical tick first. A forwarded receipt can postdate its original sample.
    pub fn accept_guidance(&mut self, observation: &GuidanceObservation) -> bool {
        if self.target_uuid.as_deref() != Some(observation.target_uuid.as_str())
            || observation.validate(u64::MAX).is_err()
            || self
                .last_guidance
                .as_ref()
                .is_some_and(|held| held.sampled_tick > observation.sampled_tick)
            || match observation.source {
                GuidanceSource::Seeker => self.profile.seeker.is_none(),
                GuidanceSource::Console { .. } => self.profile.receiver.is_none(),
            }
        {
            return false;
        }
        self.last_guidance = Some(observation.clone());
        true
    }

    pub fn tick(&mut self, dt: f32, observation: Option<&GuidanceObservation>) {
        if let Some(observation) = observation {
            self.accept_guidance(observation);
        }
        let target_pos = self
            .last_guidance
            .as_ref()
            .filter(|held| self.target_uuid.as_deref() == Some(held.target_uuid.as_str()))
            .map(|held| held.position);
        let dt = if dt.is_finite() && dt > 0.0 && self.lifespan_remaining.is_finite() {
            dt.min(self.lifespan_remaining.max(0.0))
        } else {
            0.0
        };
        if dt > 0.0 {
            self.previous_position = Some([self.x, self.y, self.z]);
            self.collision_interval_secs = Some(dt);
        }
        if self.target_uuid.is_some() {
            if let Some([tx, ty, tz]) = target_pos {
                let dx = tx - self.x;
                let dz = tz - self.z;
                let desired = simmath::atan2(dx, -dz);
                let delta = angle_diff(desired, self.heading);
                let max_turn = self.profile.turn_rate * dt;
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
        self.x += sin_h * cos_p * self.profile.speed * dt;
        self.z -= cos_h * cos_p * self.profile.speed * dt;
        self.y += sin_p * self.profile.speed * dt;
        self.lifespan_remaining = (self.lifespan_remaining - dt).max(0.0);
    }

    pub fn is_expired(&self) -> bool {
        self.lifespan_remaining <= 0.0
    }
}

// ── Torpedo tube ───────────────────────────────────────────────────────────

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
    pub marking: Option<phoenix_model::marking::MarkingPayloadConfig>,
    pub targets: Option<Vec<phoenix_model::tags::EntityTag>>,
    pub impact: phoenix_model::damage::DamageProfile,
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
    pub load_priority: u16,
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

    /// Start unloading: from Loaded → Unloading; cancel an in-progress load → Unloaded.
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
        super::pattern::barrel_sequence(&self.pattern)
    }

    /// True if `bearing_rad` (radians from ship-forward, +right = positive)
    /// is within this tube's fire arc.
    pub fn is_in_arc(&self, bearing_rad: f32) -> bool {
        super::mount::bearing_in_arc(bearing_rad, self.facing_deg, self.fire_arc_deg)
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

// ── Torpedo system ─────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct TorpedoSystem {
    pub tubes: Vec<TorpedoTube>,
    pub loader_groups: Vec<super::shared_loaders::LoaderGroupConfig>,
    /// Current derived work receipt only; never used by the executor or restored.
    pub loader_wait: std::collections::BTreeMap<String, phoenix_model::shared_loader::LoaderWait>,
    pub config: TorpedoConfig,
    /// Physical rounds in the magazine, including claimed rounds being loaded.
    pub magazine: Stock,
    pub in_flight: Vec<Torpedo>,
    /// Per-tube burst-fire pending state (issue #632). Entries are removed
    /// when their `pending` count reaches 0.
    pub burst_states: Vec<TubeBurstState>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum LaunchResult {
    MissingMarkingIdentity,
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
    /// Causal first emitted round identity; not another mint or launch cursor.
    pub launch_uuid: String,
    pub profile: GuidedRoundProfile,
    /// Original emission cadence; current equipment cannot retime this volley.
    pub burst_interval_secs: f32,
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
        let initial = config.initial_count.unwrap_or(count);
        let load_time = config.load_time;
        // Legacy tubes are all `volley_max: 1`, so the ship-wide AI default
        // clamps to at most one round per tube.
        let ai_target_count = config.ai_volley_target.unwrap_or(1).min(1);
        let tubes = vec![
            TorpedoTube {
                marking: None,
                targets: None,
                impact: phoenix_model::damage::DamageProfile::default(),
                id: "fore_port".to_string(),
                facing_deg: -30.0,
                fire_arc_deg: 90.0,
                load_state: TubeLoadState::Unloaded,
                load_time,
                load_priority: 100,
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
                marking: None,
                targets: None,
                impact: phoenix_model::damage::DamageProfile::default(),
                id: "fore_starboard".to_string(),
                facing_deg: 30.0,
                fire_arc_deg: 90.0,
                load_state: TubeLoadState::Unloaded,
                load_time,
                load_priority: 100,
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
                marking: None,
                targets: None,
                impact: phoenix_model::damage::DamageProfile::default(),
                id: "aft".to_string(),
                facing_deg: 180.0,
                fire_arc_deg: 90.0,
                load_state: TubeLoadState::Unloaded,
                load_time,
                load_priority: 100,
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
            loader_groups: Vec::new(),
            loader_wait: Default::default(),
            config,
            magazine: Stock::new(i64::from(initial), i64::from(count))
                .expect("u32 ammunition is bounded"),
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
                marking: c.marking.clone(),
                targets: c.targets.clone(),
                impact: c.impact,
                id: c.id.clone(),
                facing_deg: c.facing_deg,
                fire_arc_deg: c.fire_arc_deg,
                load_state: TubeLoadState::Unloaded,
                load_time: c.load_time.unwrap_or(global_load_time),
                load_priority: c.load_priority,
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
        let initial = config.initial_count.unwrap_or(count);
        Self {
            tubes,
            loader_groups: Vec::new(),
            loader_wait: Default::default(),
            config,
            magazine: Stock::new(i64::from(initial), i64::from(count))
                .expect("u32 ammunition is bounded"),
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

    /// Physical rounds aboard: magazine level already includes Loading claims;
    /// ready/Unloading rounds remain in tubes and queued burst rounds remain
    /// committed onboard until launched. In-flight rounds are excluded.
    pub fn rounds_aboard(&self) -> u32 {
        self.magazine.level as u32
            + self.tubes.iter().map(|t| t.loaded_count).sum::<u32>()
            + self
                .burst_states
                .iter()
                .map(|burst| burst.pending)
                .sum::<u32>()
    }

    /// Available ammunition retains the old magazine readout: claims disappear
    /// from availability as soon as loading starts.
    pub fn torpedoes_remaining(&self) -> u32 {
        self.magazine.available() as u32
    }

    pub fn magazine_claim_id(tube_id: &str) -> String {
        format!("torpedo-load:{tube_id}")
    }

    /// Ready rounds belong to tubes but still occupy this ship's ammunition room.
    /// Loading rounds are already retained within the magazine's physical level.
    pub fn magazine_headroom(&self) -> i64 {
        self.magazine
            .headroom()
            .saturating_sub(
                self.tubes
                    .iter()
                    .map(|t| i64::from(t.loaded_count))
                    .sum::<i64>(),
            )
            .saturating_sub(
                self.burst_states
                    .iter()
                    .map(|burst| i64::from(burst.pending))
                    .sum::<i64>(),
            )
            .max(0)
    }

    pub fn replenish(&mut self, amount: i64) -> i64 {
        self.magazine
            .adjust(amount.max(0).min(self.magazine_headroom()))
    }

    /// Check canonical claims against the tube work they support on restore.
    pub fn validate_magazine(&self) -> Result<(), String> {
        self.magazine.validate()?;
        if self.magazine.ceiling != i64::from(self.config.count) {
            return Err("magazine ceiling does not match authored ammunition capacity".into());
        }
        for tube in &self.tubes {
            let claimed = self
                .magazine
                .reserved
                .get(&Self::magazine_claim_id(&tube.id))
                .copied();
            let expected = matches!(tube.load_state, TubeLoadState::Loading { .. }).then_some(1);
            if claimed != expected {
                return Err(format!(
                    "magazine claim does not match loading tube '{}'",
                    tube.id
                ));
            }
        }
        if self.magazine.reserved.len()
            != self
                .tubes
                .iter()
                .filter(|t| matches!(t.load_state, TubeLoadState::Loading { .. }))
                .count()
        {
            return Err("magazine contains an unknown tube claim".into());
        }
        let mut burst_tubes = std::collections::BTreeSet::new();
        for burst in &self.burst_states {
            let tube = self
                .tube(&burst.tube_id)
                .ok_or("burst references unknown tube")?;
            if burst.pending == 0
                || burst.pending >= tube.volley_max
                || !burst_tubes.insert(&burst.tube_id)
            {
                return Err("invalid committed burst ammunition".into());
            }
        }
        if self.magazine.level
            + self.magazine.incoming_total()
            + self
                .tubes
                .iter()
                .map(|t| i64::from(t.loaded_count))
                .sum::<i64>()
            + self
                .burst_states
                .iter()
                .map(|burst| i64::from(burst.pending))
                .sum::<i64>()
            > self.magazine.ceiling
        {
            return Err("magazine and tube rounds exceed ammunition capacity".into());
        }
        Ok(())
    }

    /// Start loading a torpedo into the given tube.
    ///
    /// Claims one torpedo from the shared pool. Returns `false` if the pool
    /// is empty, the tube id is unknown, the tube is not in the `Unloaded`
    /// state, or the tube is already at `volley_max` capacity.
    pub fn start_load(&mut self, tube_id: &str) -> bool {
        self.start_load_with_work(tube_id, &mut || true)
    }

    pub fn start_load_with_work(
        &mut self,
        tube_id: &str,
        admit: &mut impl FnMut() -> bool,
    ) -> bool {
        if !self.claim_magazine_round_with_work(tube_id, admit) {
            return false;
        }
        if self.start_load_reserved(tube_id) {
            true
        } else {
            self.release_magazine_round(tube_id);
            false
        }
    }

    /// Start loading a torpedo into the given tube *without* consuming from
    /// the shared magazine (issue #512).
    ///
    /// Used after the magazine has granted this tube a next-stage claim through
    /// the channel-2 `ClaimTorpedoRound` transaction.
    ///
    /// Returns `false` if the tube id is unknown, the tube is not in the
    /// `Unloaded` state, or the tube is at `volley_max` capacity. Does NOT
    /// claim another round; it requires the matching pregranted reservation.
    pub fn start_load_reserved(&mut self, tube_id: &str) -> bool {
        let Some(tube) = self.tubes.iter_mut().find(|t| t.id == tube_id) else {
            return false;
        };
        if tube.load_state != TubeLoadState::Unloaded
            || tube.loaded_count >= tube.volley_max
            || self
                .magazine
                .reserved
                .get(&Self::magazine_claim_id(tube_id))
                != Some(&1)
        {
            return false;
        }
        tube.start_load();
        true
    }

    /// Atomically claim the next round for an eligible tube. A repeated claim
    /// cannot reserve a second round, and loading tubes cannot claim again.
    pub fn claim_magazine_round(&mut self, tube_id: &str) -> bool {
        self.claim_magazine_round_with_work(tube_id, &mut || true)
    }

    pub fn claim_magazine_round_with_work(
        &mut self,
        tube_id: &str,
        admit: &mut impl FnMut() -> bool,
    ) -> bool {
        let Some(tube) = self.tube(tube_id) else {
            return false;
        };
        if tube.load_state != TubeLoadState::Unloaded || tube.loaded_count >= tube.volley_max {
            return false;
        }
        if let Some(claimed) = self
            .magazine
            .reserved
            .get(&Self::magazine_claim_id(tube_id))
        {
            return *claimed == 1; // Idempotent receipt; the original claim already paid.
        }
        if self.magazine.available() < 1 || !admit() {
            return false;
        }
        reserve_requirements(
            std::slice::from_mut(&mut self.magazine),
            &[ResourceRequirement {
                stock: 0,
                claim: Self::magazine_claim_id(tube_id),
                amount: 1,
            }],
        )
        .is_ok()
    }

    pub fn release_magazine_round(&mut self, tube_id: &str) -> bool {
        self.magazine
            .release(&Self::magazine_claim_id(tube_id))
            .is_some()
    }

    /// Start unloading (or cancel a load in progress).
    ///
    /// If the tube is currently `Loading`, cancels immediately and returns the
    /// torpedo to the magazine.
    ///
    /// If the tube is `Unloaded` and `loaded_count > 0`, begins the unloading
    /// timer for one torpedo — the torpedo is returned to the magazine when
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
            self.release_magazine_round(tube_id);
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
        self.launch_with_barrels_faction(
            tube_id,
            uuid,
            barrel_origins,
            launch_x,
            launch_y,
            launch_z,
            launch_heading,
            target_uuid,
            source_uuid,
            None,
        )
    }

    /// The actual runtime supplies the source's real launch faction. Marker
    /// identity freezes before any ammunition/reservation mutation.
    #[allow(clippy::too_many_arguments)]
    pub fn launch_with_barrels_faction(
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
        faction: Option<uuid::Uuid>,
    ) -> LaunchResult {
        let burst_interval = self.config.burst_interval_secs;
        let Some(idx) = self.tubes.iter().position(|t| t.id == tube_id) else {
            return LaunchResult::UnknownTube;
        };
        if self.tubes[idx].loaded_count == 0
            || self
                .burst_states
                .iter()
                .any(|burst| burst.tube_id == tube_id && burst.pending > 0)
        {
            return LaunchResult::TubeNotLoaded;
        }
        let count = self.tubes[idx].loaded_count;
        let mut profile = self.config.round_profile(self.tubes[idx].impact);
        if let Some(recipe) = &self.tubes[idx].marking {
            let Some(faction) = faction.filter(|id| !id.is_nil()) else {
                return LaunchResult::MissingMarkingIdentity;
            };
            if source_uuid.as_ref().is_none_or(|id| id.is_empty()) || recipe.validate().is_err() {
                return LaunchResult::MissingMarkingIdentity;
            }
            profile.marking = Some(phoenix_model::marking::FrozenMarkingPayload {
                recipe: recipe.clone(),
                faction,
            });
            profile.damage_hull = 0;
            profile.damage_shields = 0;
            profile.shield_pierce = 0.0;
            profile.detonation_radius = 0.0;
            profile.impact = Default::default();
        }
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

        // A partial ready volley can fire while its next round is loading.
        // Firing resets that activity, so release its unfinished claim rather
        // than losing a physical round or leaving an orphan reservation.
        if matches!(self.tubes[idx].load_state, TubeLoadState::Loading { .. }) {
            self.release_magazine_round(tube_id);
        }
        self.tubes[idx].mark_fired(); // sets loaded_count = 0, load_state = Unloaded

        // Round 0: the immediate launch.
        let ((origin_x, origin_y, origin_z), barrel0, step0) = resolve(0);
        self.tubes[idx].active_barrels = vec![barrel0];
        self.tubes[idx].pattern_step = step0;
        self.in_flight.push(Torpedo {
            previous_position: None,
            collision_interval_secs: None,
            last_guidance: None,
            profile: profile.clone(),
            durability_remaining: profile.durability,
            strike_damage_bonus: 0.0,
            uuid: uuid.clone(),
            x: origin_x,
            y: origin_y,
            z: origin_z,
            heading: launch_heading,
            pitch: 0.0,
            lifespan_remaining: profile.lifespan,
            target_uuid: target_uuid.clone(),
            source_uuid: source_uuid.clone(),
            tube_id: tube_id.to_string(),
        });

        let count_remaining = count - 1;
        // Schedule the remaining burst torpedoes — count bounded by loaded_count.
        if count_remaining > 0 {
            self.burst_states.retain(|b| b.tube_id != tube_id);
            self.burst_states.push(TubeBurstState {
                launch_uuid: uuid.clone(),
                profile,
                burst_interval_secs: burst_interval,
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
        guidance: &std::collections::HashMap<String, GuidanceObservation>,
        next_uuid: &mut impl FnMut() -> String,
    ) -> TorpedoTickResult {
        self.tick_with_work(dt, guidance, &mut |_| 1.0, next_uuid)
    }

    /// Work funding affects preparation and launch only. A waiting burst keeps
    /// its rounds and target; launched torpedoes continue real-time flight.
    pub fn tick_with_work(
        &mut self,
        dt: f32,
        guidance: &std::collections::HashMap<String, GuidanceObservation>,
        work: &mut impl FnMut(TorpedoWork<'_>) -> f32,
        next_uuid: &mut impl FnMut() -> String,
    ) -> TorpedoTickResult {
        let mut result = self.advance_with_work(dt, guidance, work, next_uuid);
        result.expired = self.settle_expired();
        result
    }

    /// Physics advancement leaves final clipped intervals available to Damage.
    /// The normal standalone wrapper settles expiry immediately afterwards.
    pub fn advance_with_work(
        &mut self,
        dt: f32,
        guidance: &std::collections::HashMap<String, GuidanceObservation>,
        work: &mut impl FnMut(TorpedoWork<'_>) -> f32,
        next_uuid: &mut impl FnMut() -> String,
    ) -> TorpedoTickResult {
        self.advance_with_loader_work(dt, guidance, 1.0, work, next_uuid)
    }

    pub fn advance_with_loader_work(
        &mut self,
        dt: f32,
        guidance: &std::collections::HashMap<String, GuidanceObservation>,
        magazine_operation: f32,
        work: &mut impl FnMut(TorpedoWork<'_>) -> f32,
        next_uuid: &mut impl FnMut() -> String,
    ) -> TorpedoTickResult {
        let mut result = TorpedoTickResult::default();
        self.loader_wait.clear();
        let mut loader_budgets = self.loader_budgets(dt, magazine_operation);
        let order = self.loader_order();
        let grouped: std::collections::BTreeMap<_, _> = self
            .tubes
            .iter()
            .filter_map(|tube| {
                self.loader_group(&tube.id)
                    .map(|group| (tube.id.clone(), group.id.clone()))
            })
            .collect();

        // ── Tube load/unload progression ──────────────────────────────────
        let mut unload_completions: u32 = 0;
        for index in order {
            let t = &mut self.tubes[index];
            let budget = grouped
                .get(&t.id)
                .and_then(|id| loader_budgets.get(id))
                .copied()
                .unwrap_or(f32::INFINITY);
            let work_dt = if matches!(
                t.load_state,
                TubeLoadState::Loading { .. } | TubeLoadState::Unloading { .. }
            ) {
                let phase_dt = match t.load_state {
                    TubeLoadState::Loading { remaining, .. }
                    | TubeLoadState::Unloading { remaining, .. } => {
                        dt.min(remaining.max(0.0)).min(budget)
                    }
                    _ => 0.0,
                };
                phase_dt
                    * work(TorpedoWork::Progress {
                        tube: &t.id,
                        dt: phase_dt,
                    })
                    .clamp(0.0, 1.0)
            } else {
                0.0
            };
            if grouped.contains_key(&t.id)
                && matches!(
                    t.load_state,
                    TubeLoadState::Loading { .. } | TubeLoadState::Unloading { .. }
                )
                && work_dt == 0.0
            {
                use phoenix_model::shared_loader::LoaderWait;
                self.loader_wait.insert(
                    t.id.clone(),
                    if magazine_operation <= 0.0 {
                        LoaderWait::MagazineUnavailable
                    } else if budget <= 0.0 {
                        LoaderWait::SharedHandling
                    } else {
                        LoaderWait::TubeWork
                    },
                );
            }
            if let Some(group) = grouped.get(&t.id) {
                if let Some(budget) = loader_budgets.get_mut(group) {
                    *budget = (*budget - work_dt).max(0.0);
                }
            }
            let awaiting_completion = matches!(t.load_state,
                TubeLoadState::Loading { remaining, .. } | TubeLoadState::Unloading { remaining, .. } if remaining <= 0.0);
            let unloading_total = match t.load_state {
                TubeLoadState::Unloading { total, .. } => Some(total),
                _ => None,
            };
            let outcome = if work_dt > 0.0 || awaiting_completion {
                t.tick(work_dt)
            } else {
                LoadTickOutcome::None
            };
            match outcome {
                LoadTickOutcome::LoadedOne => {
                    if self
                        .magazine
                        .consume(&Self::magazine_claim_id(&t.id))
                        .is_some()
                    {
                        t.loaded_count += 1;
                    }
                }
                LoadTickOutcome::UnloadedOne => {
                    if work(TorpedoWork::Transfer) <= 0.0 {
                        t.load_state = TubeLoadState::Unloading {
                            remaining: 0.0,
                            total: unloading_total.expect("unload completion has a duration"),
                        };
                        continue;
                    }
                    if t.loaded_count > 0 {
                        t.loaded_count -= 1;
                    }
                    unload_completions += 1;
                }
                LoadTickOutcome::None => {}
            }
        }
        // Return unloaded torpedoes to the magazine.
        self.magazine.adjust(i64::from(unload_completions));

        // ── Auto-load toward target_count ─────────────────────────────────
        let n_tubes = self.tubes.len();
        for i in 0..n_tubes {
            let tube = &self.tubes[i];
            if !grouped.contains_key(&tube.id)
                && tube.target_count > 0
                && !matches!(
                    tube.load_state,
                    TubeLoadState::Loading { .. } | TubeLoadState::Unloading { .. }
                )
                && tube.loaded_count < tube.target_count
                && tube.loaded_count < tube.volley_max
                && self.torpedoes_remaining() > 0
            {
                // start_load() requires Unloaded state, so reset from Loaded first.
                let previous = self.tubes[i].load_state.clone();
                self.tubes[i].load_state = TubeLoadState::Unloaded;
                let id = self.tubes[i].id.clone();
                if !self.start_load_with_work(&id, &mut || work(TorpedoWork::Transfer) > 0.0) {
                    self.tubes[i].load_state = previous;
                }
            }
        }

        // ── Auto-unload toward target_count ───────────────────────────────
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

        // ── Burst-fire timer ──────────────────────────────────────────────
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
            if burst.timer <= 0.0
                && burst.pending > 0
                && work(TorpedoWork::Launch {
                    tube: &burst.tube_id,
                }) > 0.0
            {
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
                    previous_position: None,
                    collision_interval_secs: None,
                    last_guidance: None,
                    profile: burst.profile.clone(),
                    durability_remaining: burst.profile.durability,
                    strike_damage_bonus: 0.0,
                    uuid: uuid.clone(),
                    x: origin_x,
                    y: origin_y,
                    z: origin_z,
                    heading: burst.launch_heading,
                    pitch: 0.0,
                    lifespan_remaining: burst.profile.lifespan,
                    target_uuid: burst.target_uuid.clone(),
                    source_uuid: burst.source_uuid.clone(),
                    tube_id: burst.tube_id.clone(),
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
                    burst.timer = burst.burst_interval_secs;
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

        // ── In-flight torpedo movement ────────────────────────────────────
        for t in &mut self.in_flight {
            // The map is keyed by actual round, never by a world target. Each
            // entry is a permitted measurement from a finite producer.
            t.tick(dt, guidance.get(&t.uuid));
        }
        result
    }

    /// Called after physical PDC and endpoint impacts. Expired survivors cannot
    /// be saved or advanced into a later interval.
    pub fn settle_expired(&mut self) -> Vec<String> {
        let expired = self
            .in_flight
            .iter()
            .filter(|round| round.is_expired())
            .map(|round| round.uuid.clone())
            .collect();
        self.in_flight.retain(|round| !round.is_expired());
        expired
    }

    /// Close the actual collision interval after ordinary Damage. No payload,
    /// guidance intent, fuel, durability or current physical pose is changed.
    pub fn finish_collision_interval(&mut self) {
        for round in &mut self.in_flight {
            round.previous_position = None;
            round.collision_interval_secs = None;
        }
    }

    /// Resolve a paid physical interception against the actual vector owner.
    /// Returns remaining durability, or None for an already absent round.
    /// Intercepted rounds are removed once, without detonating their payload.
    pub fn apply_interception(&mut self, uuid: &str, damage: f32) -> Result<Option<f32>, String> {
        if !damage.is_finite() || damage <= 0.0 {
            return Err("interception damage must be finite and positive".into());
        }
        let Some(index) = self.in_flight.iter().position(|round| round.uuid == uuid) else {
            return Ok(None);
        };
        let round = &self.in_flight[index];
        round.profile.validate()?;
        let final_live_interval = round.lifespan_remaining == 0.0
            && round.previous_position.is_some()
            && round
                .collision_interval_secs
                .is_some_and(|seconds| seconds.is_finite() && seconds > 0.0);
        if !final_live_interval {
            round
                .profile
                .validate_active(round.lifespan_remaining, round.durability_remaining)?;
        } else if !round.durability_remaining.is_finite()
            || round.durability_remaining <= 0.0
            || round.durability_remaining > round.profile.durability
        {
            return Err("intercepted final interval has invalid remaining durability".into());
        }
        let remaining = (round.durability_remaining - damage).max(0.0);
        if remaining == 0.0 {
            self.in_flight.remove(index);
        } else {
            self.in_flight[index].durability_remaining = remaining;
        }
        Ok(Some(remaining))
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
        let damage = |base: i32| {
            if removed.strike_damage_bonus == 0.0 {
                base
            } else {
                (base as f32 * (1.0 + removed.strike_damage_bonus)).round() as i32
            }
        };
        Some(TorpedoDetonation {
            marking: removed.profile.marking,
            impact: removed.profile.impact,
            incoming: [
                simmath::sin(removed.heading),
                -simmath::cos(removed.heading),
            ],
            damage_hull: damage(removed.profile.damage_hull),
            damage_shields: damage(removed.profile.damage_shields),
            shield_pierce: removed.profile.shield_pierce,
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
/// according to its own target model, and the two damage fields are
/// ALTERNATIVES rather than two components of one payload (issue #929):
///
/// - Resolve the facing arc the round struck, from [`Self::impact_x`] /
///   [`Self::impact_z`] and the victim's own position and yaw
///   ([`hp_facing_bearing`](crate::weapons::shield::ShieldSystem::hp_facing_bearing)).
/// - Arc UP: deliver `damage_shields`, split by `shield_pierce` with
///   [`split_damage_for_pierce`](crate::ship::damage::split_damage_for_pierce)
///   and the absorbed remainder handed to that facing.
/// - Arc DOWN (or a victim with no shields at all): deliver `damage_hull`,
///   whole, to the hull.
#[derive(Clone, Debug, PartialEq)]
pub struct TorpedoDetonation {
    pub marking: Option<phoenix_model::marking::FrozenMarkingPayload>,
    pub impact: phoenix_model::damage::DamageProfile,
    /// Arrival azimuth is retained before removing the guided projectile.
    /// Near-vertical approaches use its stored azimuth, never the launcher.
    pub incoming: [f32; 2],
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

impl TorpedoDetonation {
    /// Preserve the guided round's distinct shields-up/down payloads and the
    /// launch-time pierce snapshot, after its existing strike bonus is applied.
    pub fn delivery(&self) -> super::mount::DamageDelivery {
        super::mount::DamageDelivery {
            impact: self.impact,
            damage_hull: self.damage_hull as f32,
            damage_shields: self.damage_shields as f32,
            shield_pierce: self.shield_pierce,
        }
    }
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
        let mut hits = Vec::new();
        for torpedo in &self.in_flight {
            if torpedo.profile.marking.is_some() {
                continue;
            }
            let mut best: Option<(f32, &String)> = None;
            for (uuid, tx, ty, tz, radius) in targets {
                if torpedo.source_uuid.as_ref() == Some(uuid) {
                    continue;
                }
                let dx = tx - torpedo.x;
                let dy = ty - torpedo.y;
                let dz = tz - torpedo.z;
                let dist_sq = dx * dx + dy * dy + dz * dz;
                let threshold = torpedo.profile.detonation_radius + radius;
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

// ── private math helpers ───────────────────────────────────────────────────

use super::arc_geometry::signed_difference as angle_diff;

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "torpedo_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "torpedo_logistics_tests.rs"]
mod logistics_tests;

#[cfg(test)]
#[path = "torpedo_frozen_tests.rs"]
mod frozen_tests;
