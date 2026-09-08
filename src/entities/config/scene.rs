//! Entity schema: scene. Public paths remain in the parent module.
use super::*;

// ── Leaf scene-shape types moved from former entities/map_config.rs (PRD #341) ──
// These describe entity-template physical/visual properties consumed by
// EntityConfig (one-per-template) and by steroids::spawner. They are not
// world-tree concerns and so live alongside the entity-template schema rather
// than in world::config.

/// Global configuration block (deterministic seed, lobby metadata, and the
/// shared AI-helm sim-tick rate, all surfaced through WorldConfig).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct GlobalConfig {
    /// Master seed for deterministic generation, feeding `SimRng` (issue
    /// #837). `None` — the key omitted — means "draw one from the OS"; it is
    /// not defaulted to a constant, because a constant here would make the
    /// random tier of the seed-precedence chain unreachable.
    #[serde(default)]
    pub seed: Option<u64>,
    /// Display name shown in the lobby title bar.
    #[serde(default)]
    pub title: Option<String>,
    /// Short description shown below the title in the lobby.
    #[serde(default)]
    pub description: Option<String>,
    /// Fixed rate (Hz) of the LOGICAL SIMULATION tick (issue #895, PRD #849).
    ///
    /// The rate `Time<Fixed>` steps at — every `SimSet` system, in every host
    /// (browser and headless alike), advances on this clock rather than on the
    /// rendered frame. The serde default of 60 Hz matches what the browser
    /// host effectively ran at while the sim was frame-driven, and headless'
    /// `DEFAULT_HZ`, so an unauthored world does not change pace.
    ///
    /// Must be a whole multiple of [`Self::ai_tick_hz`]: the AI decision
    /// cadence is derived from this tick by counting (see
    /// [`Self::sim_ticks_per_ai_tick`]), and `world::config::parse_world`
    /// rejects a non-commensurate pair the same way it rejects a bad
    /// `ai_tick_hz / ai_snapshot_hz` ratio.
    ///
    /// Must also be at least [`MIN_SIM_TICK_HZ`] (30 Hz), which `parse_world`
    /// enforces the same way. Below that floor the helm integrator's
    /// `HELM_AI_MAX_DT_SECS` cap would silently shorten every step: the ship
    /// under-integrates, and two hosts on different authored rates diverge.
    /// A slower rate is a content error at load, not a quiet loss of fidelity.
    ///
    /// Must be at most [`MAX_SIM_TICK_HZ`] (240 Hz), which `parse_world` also
    /// enforces. Above that ceiling the number of `FixedUpdate` steps a
    /// single lagged frame can unpack into (`Time<Virtual>::max_delta` /
    /// timestep) grows large enough to wedge the host — a faster rate is a
    /// content error at load, not a quiet performance cliff.
    #[serde(default = "default_sim_tick_hz")]
    pub sim_tick_hz: f32,
    /// Peer-local autosave cadence in simulation seconds (issue #865).
    ///
    /// Persistence is a local side effect, but every simulation peer schedules
    /// its rolling autosave from this deterministic clock. The interval must
    /// therefore convert to a positive whole number of [`Self::sim_tick_hz`]
    /// ticks; `world::config::parse_world` rejects values that would require
    /// rounding rather than letting peers choose their own boundary.
    #[serde(default = "default_autosave_interval_secs")]
    pub autosave_interval_secs: f32,
    /// Fixed rate (Hz) of the ONE shared AI decision tick (issue #889).
    ///
    /// Gates every AI policy host — the six per-axis helm systems, the seven
    /// weapons/shields/power deciders, and (through the derived slower cadence
    /// below) Captain and Sensors — decoupling AI decision cadence from the
    /// host's frame rate (issues #803, #889; PRD #620). The default matches the
    /// old `AiLateralThrustTimer` period.
    ///
    /// Authored as `ai_tick_hz`. The pre-#889 key `ai_helm_tick_hz` remains a
    /// serde alias — every shipped world TOML authors it — because the rate was
    /// never helm-specific in anything but name.
    #[serde(default = "default_ai_tick_hz", alias = "ai_helm_tick_hz")]
    pub ai_tick_hz: f32,
    /// Rate (Hz) of the DERIVED slower AI cadence: the `WorldSnapshot` /
    /// doctrine-blackboard rebuild and the two policy hosts that read them
    /// (Captain, Sensors).
    ///
    /// Before #889 this was a hardcoded 10 Hz `Timer` in `ai/server.rs` — a
    /// designer-tunable decision rate living as a Rust literal, and a second AI
    /// clock free to drift out of phase with the base one. It is now expressed
    /// as authored data and realised as an integer multiple of [`Self::ai_tick_hz`]
    /// (see [`Self::snapshot_every_ticks`]); a non-integer relationship between
    /// the two is rejected by `world::config::parse_world`.
    #[serde(default = "default_ai_snapshot_hz")]
    pub ai_snapshot_hz: f32,
    /// Hull fraction at or below which a seat's intent narration announces
    /// that the ship is breaking off (issue #879).
    ///
    /// Authored so the "we are pulling out" advisory fires where a designer
    /// says it should rather than at a Rust literal. This is the NARRATION
    /// threshold — what the crew is told — and is deliberately independent of
    /// the authored helm doctrine that decides whether the ship actually
    /// disengages; that lives in the movement fragments as a policy guard.
    #[serde(default = "default_intent_break_off_hull_fraction")]
    pub intent_break_off_hull_fraction: f32,
    /// How long (simulation seconds) a landed hit — shields or hull — keeps a
    /// ship's doctrine `attacked` condition true (issue #1010).
    ///
    /// `attacked` used to read the `LastShipAttacker` latch, which clears only
    /// on death or when the ship's red alert stands down. A hull's captain
    /// stand-down does release it (`combat_window_secs`, 10 s on every Harrow),
    /// but not while the fight continues — the captain's `secs_since_combat`
    /// fact counts the hull's own return fire, so a Harrow shooting back holds
    /// its own alert up and a `not_attacked`-gated raid stayed retired for as
    /// long as anything loitered nearby. This window governs the doctrine gate
    /// DIRECTLY: a raid yields to self-defence while the ship is being hit and
    /// resumes after a reprieve of this length, whatever the alert posture is
    /// doing. Authored so a designer can say how long a hull holds a grudge
    /// without a recompile — see `objectives::attacked_recently`.
    #[serde(default = "default_attacked_memory_secs")]
    pub attacked_memory_secs: f32,
    /// How long (simulation seconds) one station-activity debug bucket spans
    /// (issue #1145, PRD #1144).
    ///
    /// The always-on station-activity counters tally admitted commands per
    /// station per this time chunk, split by control source. Authored so a
    /// crew-control designer can widen or narrow the chart's resolution without a
    /// recompile — the serde default is the only sanctioned hardcoded copy of it
    /// (AGENTS.md #11), and it IS the shipped tuning: no world TOML authors the
    /// key. Converted to an integer tick count at `sim_tick_hz` by
    /// `debug::station_activity::StationActivityTracker::configure`.
    #[serde(default = "default_station_activity_bucket_secs")]
    pub station_activity_bucket_secs: f32,
    /// How many recent fires the trigger-fire-history debug recorder keeps per
    /// trigger (issue #1151, PRD #1144).
    ///
    /// When the scenario-state debug surface is on, a bounded ring records each
    /// trigger's recent fires with the predicate values observed at each, so an
    /// author can reconstruct why a beat fired early, late, or not at all. This
    /// is the ring depth — the bound that keeps a session that runs for hours
    /// from leaking. Read-only diagnostic capture into a `Presentation`-class
    /// resource, so it never moves the #894 digest whatever the depth. The serde
    /// default is the only sanctioned hardcoded copy (AGENTS.md #11); no world
    /// TOML authors the key, it IS the shipped tuning.
    #[serde(default = "default_trigger_fire_history_depth")]
    pub trigger_fire_history_depth: u32,
    /// How many damage/destruction rows the browser GM activity feed retains
    /// (issue #1297, PRD #930).
    ///
    /// The feed is a peer-local presentation projection over unconditional
    /// balance events. This authored bound keeps a long-running facilitated
    /// session from growing page or simulation memory without limit. The serde
    /// default is the one sanctioned hardcoded copy (AGENTS.md #11); worlds
    /// may tune the depth without changing the event stream or authoritative
    /// reducer state.
    #[serde(default = "default_gm_activity_history_depth")]
    pub gm_activity_history_depth: u32,
    /// The lockstep input delay a FLEET playing this mission agrees on, in
    /// logical ticks (issue #1116).
    ///
    /// A command a crew issues on tick *T* applies on tick *T + this*, on every
    /// host in the fleet. It is what buys agreement: a peer may be up to this
    /// many ticks behind before anybody has to wait, so it is the mission's
    /// latency budget expressed in the only unit the simulation has.
    ///
    /// **It applies only to a fleet.** A single host has nobody to wait for and
    /// runs at zero, which is `command_admission::log::CommandDelay`'s default
    /// and the only value `crate::lockstep::join_fleet` ever gives a lone host.
    /// So authoring this cannot slow down single-player play.
    ///
    /// It is authored rather than constant because it is a property of the
    /// MISSION, not of the engine: a scenario meant for a group on one LAN can
    /// afford a shorter delay than one meant for players on separate mobile
    /// networks, and the trade — input latency against how often the fleet
    /// stalls — is the author's to make. AGENTS.md rule 7 records this as the
    /// deliberate amendment it always said a non-zero delay would be.
    ///
    /// Validated at world load (`world::config::parse_world`), because a wrong
    /// value here is a permanent stall or a desync rather than a balance change.
    #[serde(default = "default_command_delay_ticks")]
    pub command_delay_ticks: u32,
}

/// Serde default for [`GlobalConfig::intent_break_off_hull_fraction`]: half
/// hull. The only sanctioned hardcoded gameplay value is a TOML-parse fallback
/// (AGENTS.md #11).
fn default_intent_break_off_hull_fraction() -> f32 {
    0.5
}

/// Serde default for [`GlobalConfig::attacked_memory_secs`]: eight seconds.
/// The only sanctioned hardcoded gameplay value is a TOML-parse fallback
/// (AGENTS.md #11).
///
/// `[ai]` The eight-second figure is AI-origin tuning, chosen as long enough
/// that a hull under sustained fire never flickers back to its raid between
/// volleys and short enough that a single stray hit costs the raid seconds
/// rather than the run. The marker that RATIFICATION reads is the `[ai] `
/// rationale bullet on `objective-doctrine-score-policy` in
/// `pasm/spec/architecture/objectives.yaml` (AGENTS.md "AI-origin decisions");
/// this note is the pointer to it, not the record itself.
fn default_attacked_memory_secs() -> f32 {
    8.0
}

/// Serde default for [`GlobalConfig::station_activity_bucket_secs`]: fifteen
/// seconds (issue #1145). The only sanctioned hardcoded copy of the shipped
/// bucket length (AGENTS.md #11) — a TOML-parse fallback.
fn default_station_activity_bucket_secs() -> f32 {
    15.0
}

/// Serde default for [`GlobalConfig::autosave_interval_secs`]: thirty
/// simulation seconds (issue #865). The only sanctioned hardcoded copy of the
/// shipped cadence (AGENTS.md #11) — a TOML-parse fallback.
fn default_autosave_interval_secs() -> f32 {
    30.0
}

/// Serde default for [`GlobalConfig::trigger_fire_history_depth`]: sixteen fires
/// per trigger (issue #1151). The only sanctioned hardcoded copy of the shipped
/// ring depth (AGENTS.md #11) — a TOML-parse fallback. Sixteen is deep enough to
/// show a repeat trigger's recent rhythm and shallow enough to stay a "few
/// records per trigger" bound.
fn default_trigger_fire_history_depth() -> u32 {
    16
}

/// Serde default for [`GlobalConfig::gm_activity_history_depth`]: 128 rows
/// (issue #1297). The only sanctioned hardcoded copy of the shipped GM feed
/// bound (AGENTS.md #11) -- a TOML-parse fallback.
fn default_gm_activity_history_depth() -> u32 {
    128
}

/// Serde default for [`GlobalConfig::command_delay_ticks`]: six logical ticks
/// (issue #1116). The only sanctioned hardcoded copy of the shipped fleet delay
/// (AGENTS.md #11) — a TOML-parse fallback.
///
/// `[ai]` Six is AI-origin tuning. At the default `sim_tick_hz = 60` it is
/// 100 ms, chosen as a round number in the unit that actually matters (wall
/// time on the wire, not ticks): it covers a one-way WebRTC hop over broadband
/// or a good mobile link with room to spare, while staying inside the ~100 ms
/// band where added input latency is not felt as sluggishness on a bridge
/// console — these are second-scale orders (set a heading, raise shields), not
/// twitch aim. A fleet on one LAN could halve it; one spread across mobile
/// networks should raise it, and will see stalls named in the host log if it
/// has not. Ratification: this is a starting value from measurement of the
/// medium rather than of play, and the first fleet playtest is what should
/// confirm or move it.
pub fn default_command_delay_ticks() -> u32 {
    6
}

/// The largest fleet delay a world may author: two seconds at the default tick
/// rate.
///
/// Not a balance ceiling — a taste one. Beyond about this, a helm order lands
/// so long after the key that a crew stops attributing the movement to their
/// own input, which is a worse failure than the stalls a shorter delay causes.
pub const MAX_COMMAND_DELAY_TICKS: u32 = 120;

impl Default for GlobalConfig {
    fn default() -> Self {
        Self {
            seed: None,
            title: None,
            description: None,
            sim_tick_hz: default_sim_tick_hz(),
            autosave_interval_secs: default_autosave_interval_secs(),
            ai_tick_hz: default_ai_tick_hz(),
            ai_snapshot_hz: default_ai_snapshot_hz(),
            intent_break_off_hull_fraction: default_intent_break_off_hull_fraction(),
            attacked_memory_secs: default_attacked_memory_secs(),
            station_activity_bucket_secs: default_station_activity_bucket_secs(),
            trigger_fire_history_depth: default_trigger_fire_history_depth(),
            gm_activity_history_depth: default_gm_activity_history_depth(),
            command_delay_ticks: default_command_delay_ticks(),
        }
    }
}

impl GlobalConfig {
    /// Convert the authored autosave interval into exact logical ticks.
    ///
    /// `None` means the interval is non-finite, non-positive, too large for a
    /// `u64`, or falls between tick boundaries. In particular this never rounds
    /// an authored duration onto a nearby tick: that would let the declared
    /// cadence and the deterministic capture boundary disagree.
    pub fn checked_autosave_interval_ticks(&self) -> Option<u64> {
        let tick_hz = f64::from(self.sim_tick_hz);
        let interval_secs = f64::from(self.autosave_interval_secs);
        if !(tick_hz.is_finite()
            && tick_hz > 0.0
            && interval_secs.is_finite()
            && interval_secs > 0.0)
        {
            return None;
        }

        let ticks = tick_hz * interval_secs;
        if !ticks.is_finite() || ticks < 1.0 || ticks.fract() != 0.0 || ticks >= u64::MAX as f64 {
            return None;
        }
        Some(ticks as u64)
    }

    /// The number of base AI ticks per slower snapshot tick.
    ///
    /// `None` when the authored pair is not a positive integer relationship —
    /// e.g. `ai_tick_hz = 25` against `ai_snapshot_hz = 10` gives 2.5, which is
    /// a content error rather than something to silently round. Callers on the
    /// hot path use [`Self::snapshot_every_ticks`], which is only reachable
    /// after `parse_world` has rejected that case.
    pub fn checked_snapshot_every_ticks(&self) -> Option<u32> {
        if !(self.ai_tick_hz.is_finite() && self.ai_tick_hz > 0.0) {
            return None;
        }
        if !(self.ai_snapshot_hz.is_finite() && self.ai_snapshot_hz > 0.0) {
            return None;
        }
        let ratio = self.ai_tick_hz / self.ai_snapshot_hz;
        let rounded = ratio.round();
        if rounded < 1.0 || (ratio - rounded).abs() > SNAPSHOT_RATIO_EPSILON {
            return None;
        }
        Some(rounded as u32)
    }

    /// [`Self::checked_snapshot_every_ticks`] with the parse-time default
    /// applied, for the run-condition system that cannot return an error.
    pub fn snapshot_every_ticks(&self) -> u32 {
        self.checked_snapshot_every_ticks()
            .unwrap_or_else(|| (default_ai_tick_hz() / default_ai_snapshot_hz()).round() as u32)
    }

    /// The number of logical sim ticks per shared AI decision tick (issue
    /// #895): the AI cadence is derived from [`SimTick`](crate::sim_tick::SimTick)
    /// by counting, so `sim_tick_hz / ai_tick_hz` must be a positive integer.
    ///
    /// `None` when it is not — a content error `parse_world` rejects, exactly
    /// like [`Self::checked_snapshot_every_ticks`].
    pub fn checked_sim_ticks_per_ai_tick(&self) -> Option<u32> {
        if !(self.sim_tick_hz.is_finite() && self.sim_tick_hz > 0.0) {
            return None;
        }
        if !(self.ai_tick_hz.is_finite() && self.ai_tick_hz > 0.0) {
            return None;
        }
        let ratio = self.sim_tick_hz / self.ai_tick_hz;
        let rounded = ratio.round();
        if rounded < 1.0 || (ratio - rounded).abs() > SNAPSHOT_RATIO_EPSILON {
            return None;
        }
        Some(rounded as u32)
    }

    /// [`Self::checked_sim_ticks_per_ai_tick`] with the parse-time default
    /// applied, for the cadence system that cannot return an error.
    pub fn sim_ticks_per_ai_tick(&self) -> u32 {
        self.checked_sim_ticks_per_ai_tick()
            .unwrap_or_else(|| (default_sim_tick_hz() / default_ai_tick_hz()).round() as u32)
    }
}

/// Tolerance on the `ai_tick_hz / ai_snapshot_hz` ratio. Both are authored as
/// `f32`, so an exactly-integer relationship such as 30/10 can land a few ULPs
/// off; 2.5 is nowhere near this band.
const SNAPSHOT_RATIO_EPSILON: f32 = 1e-4;

/// Floor on the authored [`GlobalConfig::sim_tick_hz`], enforced by
/// `world::config::parse_world` (issue #895).
///
/// Derived from — not merely matching — the helm integrator's
/// `HELM_AI_MAX_DT_SECS` cap: a sim tick longer than that cap would be
/// silently shortened by it, so the sim would under-integrate and two hosts
/// on different rates would produce different trajectories from the same
/// commands. Keeping the floor tied to the constant means the two can never
/// drift apart.
pub const MIN_SIM_TICK_HZ: f32 = 1.0 / crate::ship::components::HELM_AI_MAX_DT_SECS;

/// Ceiling on the authored [`GlobalConfig::sim_tick_hz`], enforced by
/// `world::config::parse_world` (re-review of issue #895 — the floor above
/// had no matching upper bound).
///
/// `Time<Virtual>`'s `max_delta` (250 ms) bounds how much wall-clock lag a
/// single rendered frame can absorb, but the NUMBER of `FixedUpdate` steps
/// that lag unpacks into is `max_delta / timestep`: an unbounded rate lets a
/// fat-fingered or hostile TOML (`sim_tick_hz = 100000`) demand ~25 000
/// fixed steps back-to-back inside one frame, starving everything else on
/// the host thread and making the browser or headless runner appear to
/// hang. 240 Hz keeps that worst case to 60 steps — generous headroom above
/// the shipped 60 Hz default and well past the fastest cadence any current
/// design work asks for — while still catching authored rates nobody could
/// mean.
pub const MAX_SIM_TICK_HZ: f32 = 240.0;

fn default_sim_tick_hz() -> f32 {
    60.0
}

fn default_ai_tick_hz() -> f32 {
    30.0
}

fn default_ai_snapshot_hz() -> f32 {
    10.0
}

/// Configuration for the grid-based asteroid spawner.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct GridConfig {
    pub resolution: f32,
    #[serde(default = "default_fill_gameplay")]
    pub fill_gameplay: f32,
    #[serde(default = "default_fill_cosmetic")]
    pub fill_cosmetic: f32,
    #[serde(default)]
    pub uniformity: f32,
    #[serde(default = "default_noise_freq")]
    pub noise_freq: f32,
    #[serde(default = "default_noise_octaves")]
    pub noise_octaves: u32,
    #[serde(default = "default_density_noise_freq")]
    pub density_noise_freq: f32,
    #[serde(default = "default_density_noise_octaves")]
    pub density_noise_octaves: u32,
    #[serde(default)]
    pub jitter: f32,
    #[serde(default)]
    pub cosmetic_y_offset: f32,
    #[serde(default = "default_gameplay_y_variance")]
    pub gameplay_y_variance: f32,
    #[serde(default = "default_spawn_cells")]
    pub spawn_cells: u32,
    #[serde(default = "default_despawn_cells")]
    pub despawn_cells: u32,
}

fn default_fill_gameplay() -> f32 {
    0.4
}
fn default_fill_cosmetic() -> f32 {
    0.15
}
fn default_noise_freq() -> f32 {
    0.02
}
fn default_noise_octaves() -> u32 {
    3
}
fn default_density_noise_freq() -> f32 {
    0.01
}
fn default_density_noise_octaves() -> u32 {
    2
}
fn default_gameplay_y_variance() -> f32 {
    0.5
}
fn default_spawn_cells() -> u32 {
    10
}
fn default_despawn_cells() -> u32 {
    12
}

/// Shape variant for an asteroid field.
///
/// When the TOML schema omits `shape`, the field defaults to the historical
/// behaviour (cell-centre distance check against `inner_radius`/`outer_radius`,
/// which produces a disc/annulus depending on whether `inner_radius` is zero).
///
/// `Torus` selects the explicit annulus eligibility test: a cell is admitted
/// if its XZ bounding box overlaps the annulus `[inner_radius, outer_radius]`
/// around the world origin. Cells whose bounding box lies fully inside
/// `inner_radius` or whose nearest corner is beyond `outer_radius` are
/// rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AsteroidFieldShape {
    /// Annulus / ring-belt eligibility based on `inner_radius` and
    /// `outer_radius`. Cells whose bounding box overlaps the annulus
    /// are admitted.
    Torus,
}

/// One authored asteroid type in a field's type list, with its relative
/// rarity weight (issue #946).
///
/// Two spellings, both valid TOML in the same array:
///
/// ```toml
/// asteroid_type_paths = [
///     "assets/entities/asteroid_common_1_small.toml",
///     { path = "assets/entities/asteroid_rare_1_small.toml", weight = 0.01 },
/// ]
/// ```
///
/// The bare-string form is the pre-#946 schema and still parses — it means
/// exactly `weight = 1.0` — so every field TOML written before rarity
/// existed keeps working untouched.
///
/// Weights are **relative within one list**, not probabilities: an entry at
/// `0.1` is drawn a tenth as often as an entry at `1.0` beside it. Nothing
/// in Rust knows what "common", "uncommon" or "rare" mean; the rarity tiers
/// are entirely a property of the numbers a designer authors here, so a new
/// tier needs no code change. Non-positive weights clamp to zero, and a list
/// whose weights are *all* zero falls back to a uniform draw rather than
/// erasing the field (the same degenerate-authoring guard the field-level
/// `weight` uses).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum AsteroidTypeRef {
    /// `"assets/entities/foo.toml"` — an unweighted entry, i.e. weight 1.0.
    Path(String),
    /// `{ path = "assets/entities/foo.toml", weight = 0.1 }`.
    Weighted {
        path: String,
        #[serde(default = "default_asteroid_type_weight")]
        weight: f32,
    },
}

impl AsteroidTypeRef {
    /// The entity template this entry points at.
    pub fn path(&self) -> &str {
        match self {
            Self::Path(path) => path,
            Self::Weighted { path, .. } => path,
        }
    }

    /// The authored rarity weight; `1.0` for the bare-string form.
    pub fn weight(&self) -> f32 {
        match self {
            Self::Path(_) => default_asteroid_type_weight(),
            Self::Weighted { weight, .. } => *weight,
        }
    }
}

impl From<&str> for AsteroidTypeRef {
    fn from(path: &str) -> Self {
        Self::Path(path.to_string())
    }
}

impl From<String> for AsteroidTypeRef {
    fn from(path: String) -> Self {
        Self::Path(path)
    }
}

fn default_asteroid_type_weight() -> f32 {
    1.0
}

/// Configuration for an asteroid field.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct AsteroidFieldConfig {
    pub inner_radius: f32,
    pub outer_radius: f32,
    pub density: f32,
    /// Relative weight of this field's contribution to the world's composed
    /// density evaluator (#913). Every authored asteroid-field entity feeds
    /// one shared evaluator; where several fields cover the same lattice
    /// cell their densities and fill thresholds blend proportionally to
    /// weight, and the spawned rock's tuning (type lists, jitter, rotation,
    /// shield pierce) comes from one contributing field picked by the same
    /// weights. `1.0` (the default) makes all fields equal partners. `0.0`
    /// removes a field's influence wherever a positively-weighted field
    /// also covers the cell; if every covering field is zero-weighted the
    /// blend falls back to uniform so a field can never author itself into
    /// a divide-by-zero.
    #[serde(default = "default_field_weight")]
    pub weight: f32,
    #[serde(default = "default_spawn_distance")]
    pub spawn_distance: f32,
    #[serde(default = "default_despawn_distance")]
    pub despawn_distance: f32,
    /// Gameplay (targetable, hulled) asteroid types this field may spawn,
    /// each with its relative rarity weight. See [`AsteroidTypeRef`].
    #[serde(default)]
    pub asteroid_type_paths: Vec<AsteroidTypeRef>,
    /// Backdrop asteroid types for the two cosmetic layers, same shape.
    #[serde(default)]
    pub cosmetic_type_paths: Vec<AsteroidTypeRef>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub grid: Option<GridConfig>,
    /// Fraction of asteroid-collision damage that bypasses shields and
    /// applies directly to the hull. Default `0.0` — asteroid impacts are
    /// fully absorbed by the facing shield quadrant (matching pre-#414
    /// behaviour). Clamped to `[0.0, 1.0]` at apply time.
    #[serde(default)]
    pub shield_pierce: f32,
    /// Optional shape variant. When `None`, the historical cell-centre
    /// distance eligibility test is used. When `Some(Torus)`, cells are
    /// admitted iff their XZ bounding box overlaps the annulus
    /// `[inner_radius, outer_radius]`. See [`AsteroidFieldShape`].
    #[serde(default)]
    pub shape: Option<AsteroidFieldShape>,
    /// Optional world anchor name. When present, the field's eligibility
    /// region and per-asteroid spawn positions are translated so the
    /// `[inner_radius, outer_radius]` annulus is centred on the named
    /// anchor's world position instead of the world origin. The anchor
    /// is resolved against `WorldConfig.anchors` at spawn time; the
    /// resolved offset is written into `anchor_offset`. If the anchor
    /// name is not present in the world's anchor table, the field falls
    /// back to the world origin (`anchor_offset = [0, 0, 0]`) and a
    /// warning is logged.
    #[serde(default)]
    pub anchor: Option<String>,
    /// Resolved world-space offset for the anchor referenced by `anchor`.
    /// Defaults to `[0, 0, 0]` (world origin) when no anchor is set or
    /// the named anchor could not be resolved. Not serialised — derived
    /// at spawn time.
    #[serde(skip)]
    pub anchor_offset: [f32; 3],
    /// Maximum random rotation applied to each spawned asteroid, in degrees.
    /// `[x, y, z]` → ±x° pitch, ±y° roll, ±z° yaw. When `None` (default),
    /// asteroids spawn with no rotation. Set e.g. `[30, 30, 180]` for mild
    /// tilt with full spin freedom.
    #[serde(default)]
    pub random_rotation: Option<[f32; 3]>,
}

fn default_field_weight() -> f32 {
    1.0
}

fn default_spawn_distance() -> f32 {
    150.0
}
fn default_despawn_distance() -> f32 {
    250.0
}
