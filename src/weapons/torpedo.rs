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
        }
    }
}

// Ã¢â€â‚¬Ã¢â€â‚¬ In-flight torpedo Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬

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
    /// Each entry is `(tube_id, uuid, x, z, heading)`.
    pub burst_launched: Vec<(String, String, f32, f32, f32)>,
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
    pub launch_z: f32,
    pub launch_heading: f32,
    pub target_uuid: Option<String>,
    pub source_uuid: Option<String>,
}

impl TorpedoSystem {
    /// Construct a torpedo system with the three legacy tubes
    /// (`fore_port`, `fore_starboard`, `aft`) for test convenience.
    /// Production code should use [`Self::from_configs`].
    pub fn new(config: TorpedoConfig) -> Self {
        let count = config.count;
        let load_time = config.load_time;
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
                target_count: 0,
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
        let lifespan = self.config.lifespan;
        let burst_interval = self.config.burst_interval_secs;
        let Some(tube) = self.tube_mut(tube_id) else {
            return LaunchResult::UnknownTube;
        };
        if tube.loaded_count == 0 {
            return LaunchResult::TubeNotLoaded;
        }
        let count = tube.loaded_count;
        tube.mark_fired(); // sets loaded_count = 0, load_state = Unloaded
        let shield_pierce = self.config.shield_pierce;
        // Fire the first torpedo immediately.
        self.in_flight.push(Torpedo {
            uuid: uuid.clone(),
            x: launch_x,
            z: launch_z,
            heading: launch_heading,
            lifespan_remaining: lifespan,
            target_uuid: target_uuid.clone(),
            source_uuid: source_uuid.clone(),
            shield_pierce,
        });
        let count_remaining = count - 1;
        // Schedule the remaining burst torpedoes.
        if count_remaining > 0 {
            // Remove any existing burst state for this tube.
            self.burst_states.retain(|b| b.tube_id != tube_id);
            self.burst_states.push(TubeBurstState {
                tube_id: tube_id.to_string(),
                pending: count_remaining,
                timer: burst_interval,
                launch_x,
                launch_z,
                launch_heading,
                target_uuid,
                source_uuid,
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
        target_positions: &std::collections::HashMap<String, (f32, f32)>,
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
        let mut burst_events: Vec<(String, String, f32, f32, f32)> = Vec::new();
        let mut completed_bursts: Vec<usize> = Vec::new();
        for (i, burst) in self.burst_states.iter_mut().enumerate() {
            burst.timer -= dt;
            if burst.timer <= 0.0 && burst.pending > 0 {
                let uuid = next_uuid();
                burst_torpedoes.push(Torpedo {
                    uuid: uuid.clone(),
                    x: burst.launch_x,
                    z: burst.launch_z,
                    heading: burst.launch_heading,
                    lifespan_remaining: lifespan,
                    target_uuid: burst.target_uuid.clone(),
                    source_uuid: burst.source_uuid.clone(),
                    shield_pierce,
                });
                burst_events.push((
                    burst.tube_id.clone(),
                    uuid,
                    burst.launch_x,
                    burst.launch_z,
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
            volley_max: 1,
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
        let targets: HashMap<String, (f32, f32)> = HashMap::new();
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
        let r = sys.launch("fore_port", "t1".into(), 0.0, 0.0, 0.0, None, None);
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
        sys.launch("fore_port", "t1".into(), 0.0, 0.0, 0.0, None, None);
        assert_eq!(sys.in_flight.len(), 1);
        assert_eq!(sys.in_flight[0].uuid, "t1");
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
        sys.launch("fore_port", "t1".into(), 0.0, 0.0, 0.0, None, None);
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
        sys.launch("fore_port", "t1".into(), 0.0, 0.0, 0.0, None, None);
        assert!(!sys.tube("fore_port").unwrap().is_loaded());
        assert_eq!(
            sys.tube("fore_port").unwrap().load_state,
            TubeLoadState::Unloaded
        );
        // Disable auto-management on this tube: the test verifies that a
        // manual launch does NOT trigger an automatic reload on its own.
        // (Auto-management only reloads when target_count > loaded_count.)
        sys.tube_mut("fore_port").unwrap().target_count = 0;
        let targets: HashMap<String, (f32, f32)> = HashMap::new();
        sys.tick(sys.config.load_time, &targets, &mut no_uuid);
        assert_eq!(
            sys.tube("fore_port").unwrap().load_state,
            TubeLoadState::Unloaded
        );
    }

    #[test]
    fn launch_from_unloaded_tube_returns_not_loaded() {
        let mut sys = default_system();
        let r = sys.launch("fore_port", "t1".into(), 0.0, 0.0, 0.0, None, None);
        assert_eq!(r, LaunchResult::TubeNotLoaded);
    }

    #[test]
    fn launch_from_unknown_tube_returns_unknown() {
        let mut sys = default_system();
        let r = sys.launch("dorsal", "t1".into(), 0.0, 0.0, 0.0, None, None);
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
        let targets: HashMap<String, (f32, f32)> = HashMap::new();
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
        let mut sys = loaded_system();
        sys.launch("fore_port", "t1".into(), 0.0, 0.0, 0.0, None, None);
        let initial = sys.in_flight[0].heading;
        let targets: HashMap<String, (f32, f32)> = HashMap::new();
        sys.tick(0.1, &targets, &mut no_uuid);
        assert_eq!(sys.in_flight[0].heading, initial);
    }

    #[test]
    fn torpedo_moves_forward_in_straight_flight() {
        let mut sys = loaded_system();
        sys.launch("fore_port", "t1".into(), 0.0, 0.0, 0.0, None, None);
        let targets: HashMap<String, (f32, f32)> = HashMap::new();
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
            Some("enemy".into()),
            None,
        );
        let mut targets = HashMap::new();
        targets.insert("enemy".into(), (20.0_f32, 0.0_f32));
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
            Some("enemy".into()),
            None,
        );
        let mut targets = HashMap::new();
        targets.insert("enemy".into(), (20.0_f32, 0.0_f32));
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
            Some("enemy".into()),
            None,
        );
        let targets: HashMap<String, (f32, f32)> = HashMap::new();
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
            Some("target-a".into()),
            None,
        );
        let mut targets = HashMap::new();
        targets.insert("target-a".into(), (100.0_f32, 0.0_f32)); // hard right
        targets.insert("target-b".into(), (0.0_f32, -100.0_f32)); // straight ahead

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
        sys.launch("fore_port", "t1".into(), 0.0, 0.0, 0.0, None, None);
        let targets: HashMap<String, (f32, f32)> = HashMap::new();
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
        sys.launch("fore_port", "t1".into(), 0.0, 0.0, 0.0, None, None);
        let targets: HashMap<String, (f32, f32)> = HashMap::new();
        let r = sys.tick(4.9, &targets, &mut no_uuid);
        assert!(!r.expired.contains(&"t1".to_string()));
        assert_eq!(sys.in_flight.len(), 1);
    }

    #[test]
    fn collision_removes_torpedo_and_returns_damage() {
        let mut sys = loaded_system();
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
    fn tube_loads_after_manual_load_time() {
        let mut config = TorpedoConfig::default();
        config.load_time = 10.0;
        let tubes = vec![cfg("fore_port", -30.0, 90.0)];
        let mut sys = TorpedoSystem::from_configs(&tubes, config);
        assert!(sys.start_load("fore_port"));
        assert!(!sys.tube("fore_port").unwrap().is_loaded());
        let targets: HashMap<String, (f32, f32)> = HashMap::new();
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
        let targets: HashMap<String, (f32, f32)> = HashMap::new();
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
        // Detonation radius 1, target radius 10, distance 9 Ã¢â€ â€™ should hit.
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
        sys.launch(
            "fore_port",
            "t1".into(),
            0.0,
            0.0,
            0.0,
            /*target_uuid*/ None,
            /*source_uuid*/ None,
        );
        let targets = vec![("raider".to_string(), 0.0, -3.0, 1.0)];
        let hits = sys.find_detonation_hits(&targets);
        assert_eq!(hits, vec![("t1".to_string(), "raider".to_string())]);
    }

    #[test]
    fn find_detonation_hits_handles_multiple_torpedoes_independently() {
        let mut sys = detonation_system(2.0);
        sys.launch("fore_port", "t1".into(), 0.0, 0.0, 0.0, None, None);
        // Manually push a second torpedo so the test can focus on detonation
        // matching rather than tube load state.
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
            ("a".to_string(), 1.0, 0.0, 0.0),     // close to t1
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

    // Ã¢â€â‚¬Ã¢â€â‚¬ Volley mechanics (issue #632) Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬

    fn volley_cfg(id: &str, volley_max: u32) -> TorpedoTubeConfig {
        TorpedoTubeConfig {
            id: id.into(),
            facing_deg: 0.0,
            fire_arc_deg: 180.0,
            load_time: None,
            marker: None,
            volley_max,
        }
    }

    #[test]
    fn volley_max_defaults_to_1_on_standard_tube() {
        let sys = default_system();
        assert_eq!(sys.tube("fore_port").unwrap().volley_max, 1);
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
        let targets: HashMap<String, (f32, f32)> = HashMap::new();
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
        let result = sys.launch("t1", "uuid-0".into(), 0.0, 0.0, 0.0, None, None);
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
        sys.launch("t1", "uuid-0".into(), 0.0, 0.0, 0.0, None, None);
        assert_eq!(sys.in_flight.len(), 1);

        let targets: HashMap<String, (f32, f32)> = HashMap::new();
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
        let result = sys.launch("t1", "uuid-0".into(), 0.0, 0.0, 0.0, None, None);
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
        let targets: HashMap<String, (f32, f32)> = HashMap::new();
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
        let targets: HashMap<String, (f32, f32)> = HashMap::new();
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
}
