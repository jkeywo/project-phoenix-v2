//! The authoritative world snapshot (issue #862) — phoenix's half of a save.
//!
//! # What this module owns, and what it deliberately does not
//!
//! Phoenix supplies exactly one thing: the **payload**. [`PhoenixSnapshot`] is
//! the captured authoritative state, and [`capture`]/[`restore`] are the two
//! walks that get it out of and back into a live ECS world.
//!
//! Everything around it comes from `vellum-save` and is not re-invented here:
//!
//! | concern | who answers it |
//! |---|---|
//! | the envelope a save is written in | `vellum_save::Run` |
//! | the three version dimensions | `vellum_save::Versions` |
//! | "why won't this load?" | `vellum_save::Moved` |
//! | where the bytes live | `vellum_save::Store` |
//! | "did the restore reproduce the capture?" | `vellum_save::verify` |
//!
//! There is no phoenix envelope, no phoenix version field, and no phoenix
//! compatibility validator, and that is a constraint rather than an omission: a
//! second validator would be a second, quietly disagreeing answer to a question
//! `Versions::check` has already settled, and a host shown a phoenix-invented
//! status would learn less than one shown `Moved` — which names *which*
//! dimension moved and to what.
//!
//! # What the payload covers, and why exactly that
//!
//! The boundary is issue #894's authoritative-state record, and this module
//! does not get to have its own opinion about where it runs. `world_digest`
//! (`crate::sim_digest`) is that record made executable — it is the enumerated
//! list of what a divergence is *defined over* — so the payload is built to be
//! the same list, walked in the same order:
//!
//! | `world_digest` folds | [`PhoenixSnapshot`] carries |
//! |---|---|
//! | `SimTick` | [`PhoenixSnapshot::tick`] |
//! | `SimRng`'s six stream positions | [`PhoenixSnapshot::rng`] (`SimRngState`) |
//! | `WorldIdMint`'s tick + per-namespace counters | [`PhoenixSnapshot::mint`] |
//! | `GamePhase` | [`PhoenixSnapshot::phase`] |
//! | `GameOverReason` (reason + `Outcome`) | [`PhoenixSnapshot::game_over`] |
//! | `CaptainPriorityBoost`'s sorted pairs | [`PhoenixSnapshot::captain_boosts`] |
//! | the `WorldResource` projection | [`PhoenixSnapshot::world`] (the whole `WorldData`) |
//! | the `EntityUuid` namespace | [`PhoenixSnapshot::entities`] |
//! | the `AsteroidUuid` namespace | [`PhoenixSnapshot::asteroids`] |
//! | collision attribution from `RunTelemetry` | [`PhoenixSnapshot::collisions`] |
//!
//! Two entries are deliberately *wider* than the fold rather than equal to it,
//! and both are widened toward the record, never away from it:
//!
//! * `world` stores the whole `WorldData`, not the seven-field projection the
//!   digest narrows to. The record lists `WorldResource` as IN unqualified; the
//!   narrowing is `sim_digest`'s own honest under-coverage, and a payload that
//!   copied the narrowing would drop authored geometry a resumed session still
//!   has to broadcast to its clients.
//! * `asteroids` carries each rock's config path, orientation and shield
//!   pierce alongside the position the digest folds, and
//!   [`PhoenixSnapshot::asteroid_window`] carries the streamer's own progress.
//!   The record's asteroid namespace is about rocks that *exist*; on a world
//!   with streamed belts, which rocks exist is a fact about the streamer, and a
//!   restore that could not rebuild one would be short of exactly the rocks the
//!   capture's digest counted.
//! * `flags` (the world `FlagStore`s, base and per-layer) is in because a
//!   scenario's trigger state is what makes a bounded Combat Test *bounded* —
//!   `wave_3_cleared` is authoritative even though nothing folds it yet. It is
//!   named by this issue's own acceptance criteria, and `FlagStore` gained
//!   serde for exactly this.
//!
//! **Excluded, and the exclusion is the design.** Browser UI state, PeerJS
//! sessions, renderer caches, client projections, and raw ECS `Entity` handles
//! are all absent. Every per-entity row here is keyed by its `EntityUuid` or
//! `AsteroidUuid` string, never by a handle — the same discipline the command
//! log applied when it refused to record session tokens (#898), for a related
//! reason: a handle is a slot in *this* process's ECS, so a stored handle is a
//! number that means something different every time it is read.
//!
//! # Wider than the digest, and why that is not a re-blessing
//!
//! The payload also carries the **weapon and repair state machines** that AC2
//! names by hand: [`WeaponState`] (live beams with their fractional damage
//! debt, per-bank phaser cooldowns, tube contents and load timers, torpedoes in
//! flight, pending burst volleys, per-arc shield hull) and [`RepairState`]
//! (each team's slot, the request queue, the "already told the crew" latch).
//! Alongside them it carries the AI state a continuation turns out to hang off
//! just as hard: [`PhoenixSnapshot::ai_policy_clock`], each ship's
//! [`RecoveryHistory`] windows, its [`EntityState::patrol_cursors`], and its
//! frozen [`EntityState::blackboards`]. Every one of those was found the same
//! way — by measuring where a restored world stopped agreeing with the live one
//! and asking why, rather than by reasoning about what a save "should" hold.
//!
//! None of that is folded by `world_digest`, and none of it moves the digest by
//! being here. That distinction is the whole reason this is allowed:
//! `sim_digest` is the definition of what a *divergence* is, and widening it is
//! a re-blessing event under #894's AC4. A payload is the definition of what a
//! *continuation* needs, and the two are not the same list. A restored ship
//! whose beams were extinguished, whose tubes were emptied and whose repair
//! teams were sent home stands at a matching digest and then behaves
//! differently on the very next tick — which is a divergence the digest
//! discovers a tick late and cannot explain.
//!
//! # Honestly *still* not covered, and what that costs
//!
//! Not captured today: power allocation, modifier caches, the doctrine
//! objective evaluator's own per-ship derivation (which is what still bounds a
//! low-LOD ship's continuation — see `tests/snapshot_resume.rs`), and rapier's
//! own rigid-body velocities (phoenix's `ShipPhysics` is restored; the solver's
//! internal state is not).
//!
//! A restore reproduces the captured *digest* exactly — that is what
//! [`vellum_save::verify`] checks — and then continues from state that is
//! complete over the record, complete over the machines above, and default over
//! everything else. `tests/snapshot_resume.rs` measures how far that carries
//! and writes the number down rather than tuning around it.
//!
//! # A restore is not a `spawn`
//!
//! [`restore`] does not build a world from nothing. It is handed a world that
//! the *same scenario* has already bootstrapped — `Run::scenario` and
//! `Run::seed` are what say which — and it overwrites that world's
//! authoritative state with the capture's. Entities are matched by uuid;
//! anything the bootstrap spawned that the capture did not have is despawned,
//! and anything the capture had that the bootstrap did not spawn is reported as
//! a [`RestoreGap`] rather than silently skipped. A silent gap is the failure
//! mode worth engineering against: it restores *most* of a world and then
//! diverges for a reason nothing in the save points at.

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use vellum_save::{Ledger, Run, Snapshot, Versions};

use crate::asteroid_lifecycle::{AsteroidData, AsteroidEntityMap, AsteroidWindow};
use crate::balance::{BalanceEvent, StampedBalanceEvent, VictimKind, WEAPON_KIND_COLLISION};
use crate::command_admission::log::LoggedCommand;
use crate::console::repair::server::{RepairQueueEntry, RepairRequestQueue, ShipRepairTeams};
use crate::console::weapons::beam::{
    ActiveBeam, ActiveBeamSlot, LastShipAttacker, PhaserCooldown, TacticalRadarSelection,
};
use crate::console::weapons::torpedo::TorpedoSystemResource;
use crate::core::telemetry::RunTelemetry;
use crate::entity_spawner::{EntityShipArcHull, EntitySystemHull, EntityUuid};
use crate::lobby::WorldResource;
use crate::messages::{GamePhase, SystemId, TeamSlot, WorldData};
use crate::server_app::{AsteroidUuid, CaptainPriorityBoost, GameOverReason};
use crate::ship::components::LastHelmInput;
use crate::ship::components::RepairHumanAlerted;
use crate::ship::helm::{
    BoostCommand, ImpulseCommand, LateralThrustInput, SteeringInput, ThrustInput,
    VerticalThrustInput,
};
use crate::ship::helm_ai::{
    HelmBoostAiPolicyState, HelmEnginesAiPolicyState, HelmSteeringAiPolicyState,
};
use crate::ship::impulse::ImpulsePhase;
use crate::ship::state::{ShipPhysics, ShipRedAlert};
use crate::sim_rng::{SimRng, SimRngState};
use crate::sim_tick::SimTick;
use crate::torpedo::{Torpedo, TubeBurstState, TubeLoadState};
use crate::world::flags::FlagStore;
use crate::world::server::WorldContentRuntime;
use crate::world_id::{WorldIdMint, WorldIdMintState};

// ── The three version dimensions ─────────────────────────────────────────────

/// The payload's byte layout, bumped by hand whenever [`PhoenixSnapshot`]'s
/// shape changes in a way an older save cannot be read as.
///
/// This is a *phoenix* constant that `vellum_save::Versions` carries, not a
/// phoenix version field: the comparison, the ordering of the three checks, and
/// the refusal all belong to `Versions::check`.
pub const SNAPSHOT_FORMAT: u32 = 1;

/// The simulation, as a string because "0.1-pre" says more in a bug report than
/// "1" and because nothing compares these for order.
///
/// Bump it whenever the simulation's rules change, however slightly. A save
/// recorded under other rules is not a save this build can honour, and the only
/// available honesty is to refuse it — `Run` has no migration hook and is not
/// meant to.
pub const SIMULATION_RULES: &str = "0.1";

/// The authored data, computed rather than remembered.
///
/// Phoenix has no repo-wide assets digest today (searched for: the perf module
/// inventories asset *sizes*, not contents, and nothing else hashes authored
/// data), so the scenario's own TOML text is what stands in. That is the
/// narrowest honest choice available: a save is of one scenario, and a scenario
/// whose text has changed is a scenario this save's ticks did not happen in.
///
/// `fnv1a` rather than a phoenix-local hash, because `vellum-digest` is already
/// the fleet's digest primitive and a second one would be a second answer.
pub fn content_digest(scenario_toml: &str) -> u64 {
    vellum_digest::fnv1a(scenario_toml.as_bytes())
}

/// The three dimensions this build writes and reads saves against.
pub fn versions(scenario_toml: &str) -> Versions {
    Versions::new(
        SNAPSHOT_FORMAT,
        SIMULATION_RULES,
        content_digest(scenario_toml),
    )
}

/// The stored artifact's full type: `vellum-save`'s envelope, phoenix's payload.
///
/// Named because it appears in four signatures and spelling it out invites the
/// two parameters being swapped. `LoggedCommand` is the log's element type even
/// though this issue always stores an empty log — the type is what makes #849's
/// continuation log a filled-in field rather than a new artifact.
pub type StoredRun = Run<LoggedCommand, PhoenixSnapshot>;

// ── The payload ──────────────────────────────────────────────────────────────

/// One `EntityUuid`-bearing entity's authoritative state.
///
/// Keyed by the uuid string, never by an ECS handle — see the module docs.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct EntityState {
    pub uuid: String,
    /// `ShipPhysics`' eight fields, in the order the digest folds them:
    /// `x, y, z, yaw, forward_speed, roll, lateral_speed, vertical_speed`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub physics: Option<[f32; 8]>,
    /// `(SystemId, current, max)` per system, in the hull's own stable
    /// insertion order — the same walk `fold_hull` takes. The tier thresholds
    /// and display names are NOT stored: they are authored config the fresh
    /// world rebuilds from TOML, and storing them would put authored data in a
    /// save that a content-digest change is already meant to invalidate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hull: Option<Vec<(String, f32, f32)>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub red_alert: Option<bool>,
    /// The helm axes as they stood at the capture — see [`ControlState`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control: Option<ControlState>,
    /// The weapon state machines — see [`WeaponState`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weapons: Option<WeaponState>,
    /// The repair crew — see [`RepairState`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repair: Option<RepairState>,
    /// `ObjectiveCursors` as `(objective id, waypoint index, settled)`.
    ///
    /// Where a patrolling ship is *around its route*, which is not derivable
    /// from where it is in space: a route crosses itself, and the cursor is the
    /// only thing that says which leg the ship is on. A wave NPC restored at
    /// index 0 steers for the start of a lap it was halfway around — and
    /// `simulate_low_lod_ships` snaps its yaw straight at that waypoint, so the
    /// error is instant and total rather than a drift.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub patrol_cursors: Vec<(String, u32, bool)>,
    /// `ShipSystemBlackboards`, sorted by system id — see the type note below.
    ///
    /// # Why a blackboard is authoritative and not a client projection
    ///
    /// AC5 excludes client projections, and a blackboard is *broadcast* to a
    /// console, which makes this look like the same thing. It is not, and the
    /// difference is which direction the arrow points: the console copy is a
    /// projection OF this map, and this map is the **frozen cross-system read
    /// surface** the ship's own AI decides from. `helm_shared_target_view`
    /// reads the Viewscreen blackboard's `combat_lock` and `science_target`
    /// deliberately through this freeze rather than off Tactical's live
    /// selection (issue #829), precisely so a cross-system read cannot reach
    /// another system's synchronous state. Restoring the wire copies to the
    /// browser is not this field's job; restoring what the helm reads next tick
    /// is.
    ///
    /// The measured consequence of leaving it out is written down in
    /// `tests/snapshot_resume.rs`: a resumed ship whose blackboards were the
    /// *bootstrap's* found its own target lock naming a ship its frozen view
    /// had never seen, resolved no travel target, cleared its recovery windows
    /// and fell out of `torpedo_run` into `acquire` on the first AI tick.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blackboards: Vec<(String, crate::messages::SystemBlackboard)>,
}

/// A ship's **weapon state machines**, named by this issue's AC2.
///
/// Every field here is a machine that is *mid-something* at the capture tick,
/// and a resumed ship whose machines came back cold is a different ship: a beam
/// that was two thirds through its burn stops burning, a bank that was on
/// cooldown is free to fire, a tube that was three seconds into a load is
/// empty, and a shield arc that had taken a broadside is whole again. Each of
/// those changes what happens on the *first* tick after a restore, which is
/// exactly the window a digest match at the instant of restore cannot see.
///
/// # Runtime only, never authored
///
/// The rule [`EntityState::hull`] states is kept here throughout: what is
/// stored is what the run *changed*, never what the TOML said. A tube's
/// `facing_deg`, `fire_arc_deg`, `volley_max`, `load_time`, barrel names and
/// firing pattern are all authored, the fresh world rebuilt them, and a save
/// that disagreed about them is a save the content-version gate refuses. Only
/// the load state, the counts and the last-fired markers travel.
///
/// # Written as scalars
///
/// For [`ControlState`]'s reason: a `derive` on `TubeLoadState` would silently
/// make its variant *order* stored surface, and a scalar written out at the
/// call site makes that commitment visible where it is made.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct WeaponState {
    /// Every live phaser beam as `(bank, target uuid, remaining_secs,
    /// damage_accumulator)`, in the bank order `ActiveBeam` already keeps.
    ///
    /// `damage_accumulator` is in the tuple deliberately. It is the fractional
    /// damage carried between ticks so that 5 HP/s applies accurately at any
    /// frame rate; a beam restored without it is a beam whose sub-tick debt was
    /// forgiven, and `ActiveBeam::restore_live_banks` exists so that a restore
    /// cannot round-trip through `start` and lose it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub beams: Vec<(String, String, f32, f32)>,
    /// `(bank, remaining_secs)` for every bank still cooling, in bank order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub phaser_cooldowns: Vec<(String, f32)>,
    /// Every tube's contents and load machine, in the tube order the hull
    /// authored.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tubes: Vec<TubeState>,
    /// The shared magazine. `None` when this ship has no torpedo system at all,
    /// which is how a hull with no tubes stays distinguishable from one that
    /// has shot itself dry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub torpedoes_remaining: Option<u32>,
    /// Torpedoes in the air, which are authoritative in the plainest sense —
    /// they are moving, steering and about to detonate on somebody.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub torpedoes_in_flight: Vec<TorpedoInFlight>,
    /// Volleys part-way through their burst cadence.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bursts: Vec<BurstState>,
    /// Per-arc shield hull as `(arc id, current, max)` in the arc order the
    /// TOML declared — `ShipArcHull`'s own iteration order, which it keeps
    /// separately from its map for exactly this determinism reason.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub arc_hull: Vec<(String, f32, f32)>,
}

/// One torpedo tube's runtime state.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TubeState {
    pub id: String,
    /// `TubeLoadState` as `0` = `Unloaded`, `1` = `Loading`, `2` = `Loaded`,
    /// `3` = `Unloading`. Anything else restores as `Unloaded` rather than
    /// panicking, for [`ControlState::impulse_phase`]'s reason.
    pub load_phase: u8,
    /// The `Loading`/`Unloading` timer, `(remaining, total)`. Zeroes for the
    /// two settled phases.
    pub load_timer: [f32; 2],
    pub loaded_count: u32,
    pub target_count: u32,
    /// Barrel indices the most recently launched round left from, and the
    /// 1-based pattern step it came from. Both are read to render the Tactical
    /// indicator and to pick the *next* barrel, so a tube restored without them
    /// resumes its firing pattern from the wrong place.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub active_barrels: Vec<u32>,
    pub pattern_step: u32,
}

/// One torpedo mid-flight.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TorpedoInFlight {
    pub uuid: String,
    pub position: [f32; 3],
    pub heading: f32,
    pub pitch: f32,
    pub lifespan_remaining: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_uuid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_uuid: Option<String>,
    pub tube_id: String,
    /// The firing tube's `shield_pierce` as it stood at launch. Carried by the
    /// round rather than re-resolved at detonation, so it has to be stored with
    /// it.
    pub shield_pierce: f32,
}

/// One tube's pending burst.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BurstState {
    pub tube_id: String,
    pub pending: u32,
    pub timer: f32,
    /// The launch origin and heading captured at fire time; every shot of the
    /// volley leaves from here, so it is state and not a derivation.
    pub launch: [f32; 3],
    pub launch_heading: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_uuid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_uuid: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub barrel_origins: Vec<[f32; 3]>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub barrel_sequence: Vec<(u32, u32)>,
    pub next_shot_index: u32,
}

/// A ship's **repair state**, the other half of AC2's "weapon/repair state".
///
/// A repair team is a timer with a destination. Restored idle, every team that
/// was three seconds into a five-second walk arrives late — or never, because
/// the dispatch that sent it is not re-issued — and the systems they were
/// mending stop mending. The queue and the alert latch travel with them for the
/// same reason: a queue restored empty re-raises requests the capture had
/// already spent, and `RepairHumanAlerted` is precisely the latch that stops a
/// crew being told twice.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RepairState {
    /// One `TeamSlot` per team, in slot order.
    ///
    /// The single place this payload stores a *type* rather than scalars, and
    /// the exception is narrow on purpose: `TeamSlot` is already a wire type
    /// with derived serde that the client renders from, so its variant order is
    /// stored surface the repository had committed to before this module
    /// existed. Copying it into scalars here would not remove that commitment,
    /// only duplicate it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub teams: Vec<TeamSlot>,
    /// Pending requests as `(station id, label, tier, deficit)`, in the
    /// severity order the queue keeps.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub queue: Vec<(String, String, crate::damage::DamageTier, f32)>,
    /// The "this crew has already been told" latch, as `(system id, tier)`
    /// sorted by id — the component is a `HashMap`, and a payload may not
    /// inherit its iteration order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub alerted: Vec<(String, crate::damage::DamageTier)>,
}

/// A ship's **active control state**: what its helm was being told to do.
///
/// Named by this issue's acceptance criteria, and not optional in practice even
/// though `world_digest` does not fold it. `ShipPhysics` records where a ship
/// *is* and how fast it is going; these six axes record what it is being asked
/// to do next, and `integrate_ship_physics` reads them on the very first step
/// after a restore. A resumed ship without them keeps its captured velocity and
/// then immediately coasts, which reads as a divergence one frame after a
/// restore that was otherwise exact — the first thing this slice's continuation
/// test caught.
///
/// Stored as plain scalars rather than by giving `ImpulsePhase`, `ImpulseState`
/// and `BoostState` serde derives. The reason is the type-shape constraint
/// `sim_digest` documents: a `derive` on an enum silently makes its variant
/// *order* stored surface, and a scalar written out at the call site makes that
/// commitment visible where it is made. Three fewer types become save format.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ControlState {
    pub thrust: f32,
    pub steering: f32,
    pub lateral: f32,
    pub vertical: f32,
    pub boost: bool,
    /// `ImpulsePhase` as `0` = `Idle`, `1` = `Charging`, `2` = `Active`.
    /// Anything else restores as `Idle` rather than panicking — an unknown
    /// phase is a save from a build that had one this one does not, and the
    /// content/format gate is what refuses that, not a `match` arm here.
    pub impulse_phase: u8,
    /// `LastHelmInput`'s `(thrust, steering, lateral)`. Distinct from the three
    /// axes above: those are the *desired* input, this is what the integrator
    /// last actually applied, and the helm AI's rate limiting reads the
    /// difference.
    pub last_helm: [f32; 3],
    /// `TacticalRadarSelection` — the uuid this ship's Tactical radar is locked
    /// on, or `None`.
    ///
    /// Targeting is radar-owned, and the lock is what every downstream decision
    /// hangs off: a restored ship without it has no target, so its helm AI
    /// steers nowhere and its weapons hold fire. That is a whole ship behaving
    /// differently, one tick after a restore whose digest matched exactly —
    /// which is precisely the class of silent gap this payload exists to close.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_lock: Option<String>,
    /// `LastShipAttacker` — who last shot this ship, the AI's fallback target.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_attacker: Option<String>,
    /// The three stateful helm policies' runtime state, in the fixed order
    /// `(engines, steering, boost)` — see [`PolicyState`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub helm_policies: Option<[PolicyState; 3]>,
    /// `HelmRecoveryHistory` — see [`RecoveryHistory`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub helm_recovery: Option<RecoveryHistory>,
}

/// The host-side bounded range windows a ship's helm policies read through
/// `fact(safe_distance_held)` and the pressed detector.
///
/// `HelmPolicyRuntime`'s own docs call its five components "one thing", and the
/// payload had three of them. The two it did not have are not alike, and only
/// one of them is state: `HelmPassSurface` is republished from scratch by
/// `ai_policy_state_tick` every AI tick, so a restored ship rebuilds it on its
/// first tick and storing it would store a derivation. These windows are the
/// opposite — they are an *accumulation* over the last N shared AI ticks, and
/// there is no tick on which they are recomputed from the world. A ship
/// restored without them has held its safe distance for zero samples, which is
/// a different answer to a question its transitions are gated on.
///
/// The capacities are authored (`safe_distance_window_ticks`,
/// `pressed_window_ticks`) and re-applied every tick from config, so they are
/// stored only so the samples can be replayed into a window of the right size
/// before the first tick re-authors it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RecoveryHistory {
    /// The uuid both windows were measured against. A target switch clears
    /// them, so restoring the samples without the identity they belong to
    /// would credit a new threat with distance held against the old one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    /// The level window (`safe_distance_held`), oldest sample first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ranges: Vec<f64>,
    pub ranges_capacity: u32,
    /// The trend window (the pressed detector), oldest sample first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub separation: Vec<f64>,
    pub separation_capacity: u32,
}

/// One stateful AI policy's runtime state (issue #882's `AiPolicyRuntimeState`).
///
/// Captured because a cold policy runtime is a ship that behaves differently:
/// the state id and `entered_at_secs` are what `state_time` is measured
/// against, so a restored ship whose policy was reset evaluates every
/// time-gated transition from zero and takes a different branch on the first
/// tick after the restore. That is the second silent gap this slice's
/// continuation test caught, after the helm axes.
///
/// Written out field-by-field rather than by deriving serde on
/// `AiPolicyRuntimeState` itself, for the reason [`ControlState`] gives:
/// `AiPolicyMemory` already carries serde (added for this payload), and the two
/// scalars beside it do not need a third type's shape pinned as save format to
/// travel.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PolicyState {
    /// The currently-entered state id.
    pub current: String,
    /// The tick-derived clock reading `current` was entered at.
    pub entered_at_secs: f64,
    /// The fine system's typed private memory.
    pub memory: crate::world::flags::AiPolicyMemory,
}

/// One asteroid's authoritative state.
///
/// A rock's position is authoritative because it is what a collision resolves
/// against — the digest's own reason for folding it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AsteroidState {
    pub uuid: String,
    pub translation: [f32; 3],
    /// The rock's orientation, `[x, y, z, w]`. Not folded by the digest — a
    /// tumbling rock collides the same either way — but stored because a
    /// restore now *spawns* rocks, and a spawned rock with a default rotation
    /// is a visibly different rock from the one that was saved.
    #[serde(default)]
    pub rotation: [f32; 4],
    /// `(SystemId, current, max)`, as [`EntityState::hull`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hull: Option<Vec<(String, f32, f32)>>,
    /// The rock's entity TOML path, and the reason it is here is [`restore`]'s
    /// spawn path: to *build* a missing rock the restore needs its collider,
    /// mesh, tags and radar appearance, and all four are read from this file
    /// rather than stored. It is joined from the streaming window's slot data,
    /// which is the only place a rock's config path survives after spawn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_path: Option<String>,
    /// The rock's `AsteroidShieldPierce`, which is per-*field* tuning resolved
    /// at spawn from whichever contribution the composed evaluator picked. That
    /// makes it neither authored-per-rock nor recomputable without re-running
    /// the evaluator, so it travels with the rock.
    #[serde(default)]
    pub shield_pierce: f32,
}

/// The asteroid streamer's own progress, and the reason AC1's world needs it.
///
/// Combat Test's belts are *streamed*: a rock exists when the player's cell
/// window covers it, so a fresh app bootstrapped at the spawn point has a
/// different rock population than a capture taken after the player has flown
/// somewhere. Restoring the rocks without restoring the window that owns them
/// closes half the gap and opens a worse one — the streamer would still believe
/// it was anchored where the fresh boot left it, and the very next
/// `update_asteroid_window` tick would full-rebuild the belt out from under the
/// restore.
///
/// With the anchor, the player cell and the composition key all put back, that
/// tick recomputes the same cell from the restored ship's position, finds it
/// unchanged, and returns without touching anything. The streamer resumes
/// rather than restarting.
///
/// Cosmetic slots are **not** here: they hold raw `Entity` handles, which this
/// payload never stores (see the module docs), and they are set dressing with
/// no uuid, no hull and no collider. [`restore`] clears them so the streamer
/// repopulates them on its next scroll.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AsteroidWindowState {
    pub arena_gx: i32,
    pub arena_gz: i32,
    pub despawn_cells: u32,
    pub spawn_cells: u32,
    pub resolution: f32,
    /// The player's lattice cell as of the last streamer tick. `None` before
    /// the first one has run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub player_grid: Option<(i32, i32)>,
    /// The fingerprint of the contribution set the window's contents were built
    /// from. Restored so the streamer does not read its own live fields as a
    /// composition change and rebuild.
    pub composition_key: u64,
    pub needs_init: bool,
    /// The occupied slots, sorted by `(z, x)` so the payload is byte-stable.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub slots: Vec<WindowSlot>,
}

/// One occupied ring-buffer slot.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct WindowSlot {
    pub x: u32,
    pub z: u32,
    pub uuid: String,
    pub config_path: String,
    pub hp: i32,
    pub max_hp: i32,
    pub y: f32,
}

/// One collision the run applied, in the shape the digest attributes it.
///
/// Only collisions are stored, not the whole `RunTelemetry` stream: the rest of
/// that resource is a *report* artifact (message counts, ndjson lines, name
/// tables), and a report is something a resumed run rebuilds rather than
/// something it inherits. Collisions are here because `fold_collisions` puts
/// them in the authoritative fold — #896's finding that contact attribution is
/// the part of physics a divergence shows up in first.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CollisionRecord {
    pub tick: u64,
    pub sim_t: f64,
    pub victim: String,
    pub victim_is_asteroid: bool,
    pub amount: f32,
    pub shield_absorbed: f32,
    pub hull_damage: f32,
}

/// One world layer's flag store, keyed by the layer's TOML path.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LayerFlags {
    pub path: String,
    pub flags: FlagStore,
}

/// Captured authoritative world state: everything issue #894's record says a
/// divergence is defined over, at one tick.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PhoenixSnapshot {
    /// The logical tick the capture was taken between. Mirrors
    /// `vellum_save::Snapshot::tick`, which is the envelope's copy; this is the
    /// resource's own value, and [`restore`] writes it back.
    pub tick: u64,
    pub rng: Option<SimRngState>,
    pub mint: Option<WorldIdMintState>,
    pub phase: Option<GamePhase>,
    /// `(reason, outcome label)`. The outcome is a label rather than the
    /// `Outcome` enum because `Outcome` is not `Serialize` — and the labels are
    /// already this run's report vocabulary, so nothing is lost and one fewer
    /// enum's variant order becomes stored surface.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game_over: Option<(Option<String>, Option<String>)>,
    /// `(scope, objective)` pairs in sorted key order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub captain_boosts: Vec<(String, String)>,
    /// The whole `WorldResource` payload — see the module docs for why this is
    /// wider than the digest's projection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub world: Option<WorldData>,
    /// The base world's flag store.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flags: Option<FlagStore>,
    /// Per-layer flag stores, sorted by path so the payload is byte-stable.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub layer_flags: Vec<LayerFlags>,
    /// `AiPolicyTickClock` — the tick-derived clock every stateful AI policy
    /// measures `state_time` against (issue #882's AC4).
    ///
    /// One `f64`, and the most load-bearing one in this payload.
    /// [`PolicyState::entered_at_secs`] is a reading *of this clock*, so
    /// restoring the policies without it hands every ship a state entered three
    /// seconds into a clock that now reads a sixteenth of a second:
    /// `memory_at` clamps the resulting negative `state_time` to zero, every
    /// time-gated transition evaluates as though the state had only just been
    /// entered, and the next AI tick walks a different edge. That was the whole
    /// of the tick-2 divergence this slice first measured and attributed to
    /// cold weapon machines — it was not the weapons. It was two ships falling
    /// out of `torpedo_run` and `inbound` back into `acquire`, one tick after a
    /// restore whose digest had matched exactly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ai_policy_clock: Option<f64>,
    /// Sorted by uuid — a payload must not inherit ECS iteration order any more
    /// than the digest may.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entities: Vec<EntityState>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub asteroids: Vec<AsteroidState>,
    /// The streamer's window over those rocks — see [`AsteroidWindowState`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asteroid_window: Option<AsteroidWindowState>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub collisions: Vec<CollisionRecord>,
}

// ── Capture ──────────────────────────────────────────────────────────────────

/// Walk a live world and take its authoritative state.
///
/// Takes `&World`, not `&mut World`, for the same reason `world_digest` does:
/// capturing must not perturb the run it is capturing. Every read goes through
/// `get_resource`/`try_query`, so a bare-`App` fixture with half the world
/// unregistered produces a partial payload rather than a panic.
///
/// Call this between `App::update()` calls — outside `SimSet`, at a tick
/// boundary. `SimRng::state`'s own docs say why: mid-tick, some systems for the
/// step have drawn and others have not, so "all six streams right now" is not a
/// point any system agrees on.
pub fn capture(world: &World) -> PhoenixSnapshot {
    PhoenixSnapshot {
        tick: world.get_resource::<SimTick>().map_or(0, |t| t.0),
        rng: world.get_resource::<SimRng>().map(SimRng::state),
        mint: world.get_resource::<WorldIdMint>().map(WorldIdMint::state),
        phase: world
            .get_resource::<State<GamePhase>>()
            .map(|s| s.get().clone()),
        game_over: world
            .get_resource::<GameOverReason>()
            .map(|reason| (reason.0.clone(), reason.1.map(|o| o.as_str().to_string()))),
        captain_boosts: world
            .get_resource::<CaptainPriorityBoost>()
            .map(|boosts| {
                boosts
                    .boosts_sorted()
                    .into_iter()
                    .map(|(scope, objective)| (scope.to_string(), objective.to_string()))
                    .collect()
            })
            .unwrap_or_default(),
        world: world.get_resource::<WorldResource>().map(|w| w.0.clone()),
        flags: world
            .get_resource::<WorldContentRuntime>()
            .map(|rt| rt.flags.clone()),
        layer_flags: capture_layer_flags(world),
        ai_policy_clock: world
            .get_resource::<crate::ship::helm_ai::AiPolicyTickClock>()
            .map(|clock| clock.0),
        entities: capture_entities(world),
        asteroids: capture_asteroids(world),
        asteroid_window: capture_asteroid_window(world),
        collisions: capture_collisions(world),
    }
}

fn capture_layer_flags(world: &World) -> Vec<LayerFlags> {
    let Some(layers) = world.get_resource::<crate::world::server::WorldLayerMap>() else {
        return Vec::new();
    };
    let mut rows: Vec<LayerFlags> = layers
        .0
        .iter()
        .map(|(path, runtime)| LayerFlags {
            path: path.clone(),
            flags: runtime.flags.clone(),
        })
        .collect();
    rows.sort_by(|a, b| a.path.cmp(&b.path));
    rows
}

fn hull_rows(hull: &crate::damage::SystemHull) -> Vec<(String, f32, f32)> {
    hull.iter()
        .map(|(id, entry)| (id.0.clone(), entry.current, entry.max))
        .collect()
}

/// The helm axes, in a query of their own.
///
/// Separate from [`capture_entities`]' walk because Bevy's query tuples do not
/// stretch that far, and joined back by uuid rather than by handle — the same
/// rule the rest of this module keeps.
fn capture_controls(world: &World) -> Vec<(String, ControlState)> {
    // A row is emitted only for an entity that actually carries the helm axes.
    // The distinction is load-bearing: `map_or(0.0, ..)` over an absent
    // component and a genuinely centred stick produce the same numbers, so
    // without this an entity with no helm at all would be captured as one
    // holding neutral — and `ready_to_restore` would then have no way to tell
    // that a freshly-spawned ship had not yet been given its controls.
    let Some(mut query) = world.try_query::<(
        &EntityUuid,
        Option<&ThrustInput>,
        Option<&SteeringInput>,
        Option<&LateralThrustInput>,
        Option<&VerticalThrustInput>,
        Option<&BoostCommand>,
        Option<&ImpulseCommand>,
        Option<&LastHelmInput>,
        Option<&TacticalRadarSelection>,
        Option<&LastShipAttacker>,
        Option<&HelmEnginesAiPolicyState>,
        Option<&HelmSteeringAiPolicyState>,
        Option<&HelmBoostAiPolicyState>,
        Option<&crate::ship::helm_ai::HelmRecoveryHistory>,
    )>() else {
        return Vec::new();
    };
    query
        .iter(world)
        .filter(|(_, thrust, ..)| thrust.is_some())
        .map(
            |(
                uuid,
                thrust,
                steering,
                lateral,
                vertical,
                boost,
                impulse,
                last,
                lock,
                attacker,
                engines_policy,
                steering_policy,
                boost_policy,
                recovery,
            )| {
                (
                    uuid.0.clone(),
                    ControlState {
                        thrust: thrust.map_or(0.0, |t| t.0),
                        steering: steering.map_or(0.0, |s| s.0),
                        lateral: lateral.map_or(0.0, |l| l.0),
                        vertical: vertical.map_or(0.0, |v| v.0),
                        boost: boost.is_some_and(|b| b.0),
                        impulse_phase: impulse.map_or(0, |i| match i.0 {
                            ImpulsePhase::Idle => 0,
                            ImpulsePhase::Charging => 1,
                            ImpulsePhase::Active => 2,
                        }),
                        last_helm: last.map_or([0.0; 3], |l| [l.thrust, l.steering, l.lateral]),
                        target_lock: lock.and_then(|l| l.0.clone()),
                        last_attacker: attacker.and_then(|a| a.0.clone()),
                        helm_policies: Some([
                            policy_state(engines_policy.map(|p| &p.0)),
                            policy_state(steering_policy.map(|p| &p.0)),
                            policy_state(boost_policy.map(|p| &p.0)),
                        ]),
                        helm_recovery: recovery.map(|r| RecoveryHistory {
                            target: r.target.map(|t| t.to_string()),
                            ranges: r.ranges.iter().collect(),
                            ranges_capacity: r.ranges.capacity() as u32,
                            separation: r.separation.iter().collect(),
                            separation_capacity: r.separation.capacity() as u32,
                        }),
                    },
                )
            },
        )
        .collect()
}

fn policy_state(runtime: Option<&crate::ai::policy::AiPolicyRuntimeState>) -> PolicyState {
    runtime.map_or_else(PolicyState::default, |r| PolicyState {
        current: r.current.clone(),
        entered_at_secs: r.entered_at_secs,
        memory: r.memory.clone(),
    })
}

fn apply_policy_state(runtime: &mut crate::ai::policy::AiPolicyRuntimeState, stored: &PolicyState) {
    runtime.current = stored.current.clone();
    runtime.entered_at_secs = stored.entered_at_secs;
    runtime.memory = stored.memory.clone();
}

/// The weapon state machines and the repair crew, in a query of their own.
///
/// Separate from [`capture_entities`] for [`capture_controls`]' reason — Bevy's
/// query tuples do not stretch that far — and joined back by uuid rather than
/// by handle, which is the rule the whole module keeps.
type WeaponRepairRow = (
    String,
    Option<WeaponState>,
    Option<RepairState>,
    Vec<(String, crate::messages::SystemBlackboard)>,
    Vec<(String, u32, bool)>,
);

fn capture_weapons_and_repair(world: &World) -> Vec<WeaponRepairRow> {
    let Some(mut query) = world.try_query::<(
        &EntityUuid,
        Option<&ActiveBeam>,
        Option<&PhaserCooldown>,
        Option<&TorpedoSystemResource>,
        Option<&EntityShipArcHull>,
        Option<&ShipRepairTeams>,
        Option<&RepairRequestQueue>,
        Option<&RepairHumanAlerted>,
        Option<&crate::server_app::ShipSystemBlackboards>,
        Option<&crate::ai::server::ObjectiveCursors>,
    )>() else {
        return Vec::new();
    };
    query
        .iter(world)
        .map(
            |(
                uuid,
                beam,
                cooldown,
                torpedoes,
                arcs,
                teams,
                queue,
                alerted,
                blackboards,
                cursors,
            )| {
                // A row is emitted only for an entity that carries at least one
                // of these, for `capture_controls`' reason: an all-defaults
                // `WeaponState` and a genuinely idle one are the same bytes, so
                // storing one for every entity would make an asteroid look like
                // a ship with its weapons cold.
                let weapons =
                    (beam.is_some() || cooldown.is_some() || torpedoes.is_some() || arcs.is_some())
                        .then(|| weapon_state(beam, cooldown, torpedoes, arcs));
                let repair = (teams.is_some() || queue.is_some() || alerted.is_some())
                    .then(|| repair_state(teams, queue, alerted));
                let mut boards: Vec<(String, crate::messages::SystemBlackboard)> = blackboards
                    .map(|b| {
                        b.0.iter()
                            .map(|(id, board)| (id.0.clone(), board.clone()))
                            .collect()
                    })
                    .unwrap_or_default();
                // The component is a `HashMap` on purpose (see its own docs);
                // a payload may not inherit that order.
                boards.sort_by(|a, b| a.0.cmp(&b.0));
                let cursors = cursors
                    .map(|c| {
                        c.0.iter()
                            .map(|cursor| {
                                (
                                    cursor.objective_id.clone(),
                                    cursor.index() as u32,
                                    cursor.settled(),
                                )
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                (uuid.0.clone(), weapons, repair, boards, cursors)
            },
        )
        .collect()
}

fn weapon_state(
    beam: Option<&ActiveBeam>,
    cooldown: Option<&PhaserCooldown>,
    torpedoes: Option<&TorpedoSystemResource>,
    arcs: Option<&EntityShipArcHull>,
) -> WeaponState {
    let system = torpedoes.map(|t| &t.0);
    WeaponState {
        beams: beam
            .map(|b| {
                b.live_banks()
                    .map(|(bank, slot)| {
                        (
                            bank.clone(),
                            slot.target_uuid.clone(),
                            slot.remaining_secs,
                            slot.damage_accumulator,
                        )
                    })
                    .collect()
            })
            .unwrap_or_default(),
        phaser_cooldowns: cooldown
            .map(PhaserCooldown::active_banks_sorted)
            .unwrap_or_default(),
        tubes: system
            .map(|s| {
                s.tubes
                    .iter()
                    .map(|tube| {
                        let (load_phase, load_timer) = match &tube.load_state {
                            TubeLoadState::Unloaded => (0, [0.0, 0.0]),
                            TubeLoadState::Loading { remaining, total } => {
                                (1, [*remaining, *total])
                            }
                            TubeLoadState::Loaded => (2, [0.0, 0.0]),
                            TubeLoadState::Unloading { remaining, total } => {
                                (3, [*remaining, *total])
                            }
                        };
                        TubeState {
                            id: tube.id.clone(),
                            load_phase,
                            load_timer,
                            loaded_count: tube.loaded_count,
                            target_count: tube.target_count,
                            active_barrels: tube.active_barrels.clone(),
                            pattern_step: tube.pattern_step,
                        }
                    })
                    .collect()
            })
            .unwrap_or_default(),
        torpedoes_remaining: system.map(|s| s.torpedoes_remaining),
        torpedoes_in_flight: system
            .map(|s| {
                s.in_flight
                    .iter()
                    .map(|t| TorpedoInFlight {
                        uuid: t.uuid.clone(),
                        position: [t.x, t.y, t.z],
                        heading: t.heading,
                        pitch: t.pitch,
                        lifespan_remaining: t.lifespan_remaining,
                        target_uuid: t.target_uuid.clone(),
                        source_uuid: t.source_uuid.clone(),
                        tube_id: t.tube_id.clone(),
                        shield_pierce: t.shield_pierce,
                    })
                    .collect()
            })
            .unwrap_or_default(),
        bursts: system
            .map(|s| {
                s.burst_states
                    .iter()
                    .map(|b| BurstState {
                        tube_id: b.tube_id.clone(),
                        pending: b.pending,
                        timer: b.timer,
                        launch: [b.launch_x, b.launch_y, b.launch_z],
                        launch_heading: b.launch_heading,
                        target_uuid: b.target_uuid.clone(),
                        source_uuid: b.source_uuid.clone(),
                        barrel_origins: b
                            .barrel_origins
                            .iter()
                            .map(|(x, y, z)| [*x, *y, *z])
                            .collect(),
                        barrel_sequence: b.barrel_sequence.clone(),
                        next_shot_index: b.next_shot_index,
                    })
                    .collect()
            })
            .unwrap_or_default(),
        arc_hull: arcs
            .map(|a| {
                a.0.iter()
                    .map(|(id, entry)| (id.to_string(), entry.current, entry.max))
                    .collect()
            })
            .unwrap_or_default(),
    }
}

fn repair_state(
    teams: Option<&ShipRepairTeams>,
    queue: Option<&RepairRequestQueue>,
    alerted: Option<&RepairHumanAlerted>,
) -> RepairState {
    let mut alerted_rows: Vec<(String, crate::damage::DamageTier)> = alerted
        .map(|a| {
            a.0.iter()
                .map(|(system, tier)| (system.clone(), *tier))
                .collect()
        })
        .unwrap_or_default();
    alerted_rows.sort_by(|a, b| a.0.cmp(&b.0));
    RepairState {
        teams: teams.map(|t| t.0.slots().to_vec()).unwrap_or_default(),
        queue: queue
            .map(|q| {
                q.entries
                    .iter()
                    .map(|e| {
                        (
                            e.station_id.clone(),
                            e.station_label.clone(),
                            e.tier,
                            e.deficit,
                        )
                    })
                    .collect()
            })
            .unwrap_or_default(),
        alerted: alerted_rows,
    }
}

fn capture_entities(world: &World) -> Vec<EntityState> {
    let controls = capture_controls(world);
    let machines = capture_weapons_and_repair(world);
    let Some(mut query) = world.try_query::<(
        &EntityUuid,
        Option<&ShipPhysics>,
        Option<&EntitySystemHull>,
        Option<&ShipRedAlert>,
    )>() else {
        return Vec::new();
    };
    let mut rows: Vec<EntityState> = query
        .iter(world)
        .map(|(uuid, physics, hull, alert)| EntityState {
            control: controls
                .iter()
                .find(|(id, _)| id == &uuid.0)
                .map(|(_, state)| state.clone()),
            weapons: machines
                .iter()
                .find(|(id, ..)| id == &uuid.0)
                .and_then(|(_, weapons, ..)| weapons.clone()),
            repair: machines
                .iter()
                .find(|(id, ..)| id == &uuid.0)
                .and_then(|(_, _, repair, ..)| repair.clone()),
            blackboards: machines
                .iter()
                .find(|(id, ..)| id == &uuid.0)
                .map(|(_, _, _, boards, _)| boards.clone())
                .unwrap_or_default(),
            patrol_cursors: machines
                .iter()
                .find(|(id, ..)| id == &uuid.0)
                .map(|(.., cursors)| cursors.clone())
                .unwrap_or_default(),
            uuid: uuid.0.clone(),
            physics: physics.map(|p| {
                [
                    p.x,
                    p.y,
                    p.z,
                    p.yaw,
                    p.forward_speed,
                    p.roll,
                    p.lateral_speed,
                    p.vertical_speed,
                ]
            }),
            hull: hull.map(|h| hull_rows(&h.0)),
            red_alert: alert.map(|a| a.0),
        })
        .collect();
    rows.sort_by(|a, b| a.uuid.cmp(&b.uuid));
    rows
}

fn capture_asteroids(world: &World) -> Vec<AsteroidState> {
    // The streaming window is the only place a live rock's config path
    // survives — nothing on the entity carries it — and [`restore`] needs it to
    // rebuild a rock the target world never streamed.
    let config_paths: Vec<(String, String)> = world
        .get_resource::<AsteroidWindow>()
        .map(|window| {
            window
                .slots
                .iter()
                .flatten()
                .flatten()
                .map(|data| (data.uuid.clone(), data.config_path.clone()))
                .collect()
        })
        .unwrap_or_default();

    let Some(mut query) = world.try_query::<(
        &AsteroidUuid,
        Option<&Transform>,
        Option<&EntitySystemHull>,
        Option<&crate::server_app::AsteroidShieldPierce>,
    )>() else {
        return Vec::new();
    };
    let mut rows: Vec<AsteroidState> = query
        .iter(world)
        .map(|(uuid, transform, hull, pierce)| {
            let t = transform.map(|t| t.translation).unwrap_or(Vec3::ZERO);
            let r = transform.map(|t| t.rotation).unwrap_or(Quat::IDENTITY);
            AsteroidState {
                config_path: config_paths
                    .iter()
                    .find(|(id, _)| id == &uuid.0)
                    .map(|(_, path)| path.clone()),
                uuid: uuid.0.clone(),
                translation: [t.x, t.y, t.z],
                rotation: [r.x, r.y, r.z, r.w],
                hull: hull.map(|h| hull_rows(&h.0)),
                shield_pierce: pierce.map_or(0.0, |p| p.0),
            }
        })
        .collect();
    rows.sort_by(|a, b| a.uuid.cmp(&b.uuid));
    rows
}

fn capture_asteroid_window(world: &World) -> Option<AsteroidWindowState> {
    let window = world.get_resource::<AsteroidWindow>()?;
    let mut slots = Vec::new();
    for (z, row) in window.slots.iter().enumerate() {
        for (x, slot) in row.iter().enumerate() {
            let Some(data) = slot else { continue };
            slots.push(WindowSlot {
                x: x as u32,
                z: z as u32,
                uuid: data.uuid.clone(),
                config_path: data.config_path.clone(),
                hp: data.hp,
                max_hp: data.max_hp,
                y: data.y,
            });
        }
    }
    Some(AsteroidWindowState {
        arena_gx: window.arena_gx,
        arena_gz: window.arena_gz,
        despawn_cells: window.despawn_cells,
        spawn_cells: window.spawn_cells,
        resolution: window.resolution,
        player_grid: window.player_grid,
        composition_key: window.composition_key,
        needs_init: window.needs_init,
        slots,
    })
}

fn capture_collisions(world: &World) -> Vec<CollisionRecord> {
    let Some(telemetry) = world.get_resource::<RunTelemetry>() else {
        return Vec::new();
    };
    telemetry
        .balance_events
        .iter()
        .filter_map(|stamped| match &stamped.event {
            BalanceEvent::DamageApplied {
                weapon,
                victim,
                victim_kind,
                amount,
                shield_absorbed,
                hull_damage,
                ..
            } if weapon == WEAPON_KIND_COLLISION => Some(CollisionRecord {
                tick: stamped.tick,
                sim_t: stamped.sim_t,
                victim: victim.clone(),
                victim_is_asteroid: matches!(victim_kind, VictimKind::Asteroid),
                amount: *amount,
                shield_absorbed: *shield_absorbed,
                hull_damage: *hull_damage,
            }),
            _ => None,
        })
        .collect()
}

// ── The stored artifact ──────────────────────────────────────────────────────

/// Build the `vellum_save::Run` a save is written as.
///
/// The log is empty and the ledger holds only the capture: this is a saved
/// game, not yet a recording. `vellum-save`'s own
/// `a_snapshot_with_an_empty_log_is_a_saved_game_that_verifies` is the shape
/// this mirrors, and the continuation log is #849's to fill in — at which point
/// nothing here changes, because `Run` already has the field.
///
/// `digest` is the caller's, deliberately: it is `sim_digest::world_digest` of
/// the same world at the same instant, and taking it here would mean this
/// module deciding when a digest is meaningful. That decision belongs to the
/// caller who knows it is standing between `update()` calls.
pub fn run_for(
    payload: PhoenixSnapshot,
    digest: u64,
    seed: u64,
    scenario: impl Into<String>,
    versions: Versions,
) -> StoredRun {
    let tick = payload.tick;
    Run {
        versions,
        scenario: scenario.into(),
        seed,
        snapshot: Some(Snapshot {
            tick,
            digest,
            state: payload,
        }),
        commands: Vec::new(),
        ledger: Ledger {
            every: 0,
            samples: Vec::new(),
            final_tick: tick,
            final_digest: digest,
        },
    }
}

/// The slot a host's one save lives in.
///
/// `vellum_save::is_slot` is what decides whether a name is usable at all, and
/// it is checked by the backends rather than trusted — this is just phoenix's
/// default choice of name.
pub const DEFAULT_SLOT: &str = "autosave";

/// The `localStorage` namespace the browser backend keys under.
///
/// Namespaced because two games served from one origin — which is exactly what
/// a GitHub Pages account is — must not read or overwrite each other's saves.
pub const STORAGE_NAMESPACE: &str = "phoenix";

/// Why a stored save did not become a resumable session.
///
/// [`LoadRefusal::Moved`] carries `vellum_save::Moved` **unchanged**, and its
/// `Display` is that type's own sentence. That is the acceptance criterion, not
/// a convenience: a phoenix-worded status would be a second answer to a
/// question the version gate has already answered, and it would lose the one
/// thing the gate exists to report — *which* dimension moved, and to what.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoadRefusal {
    /// Nothing is stored in that slot. A first run has no save, which is not an
    /// error.
    Empty,
    /// The store itself would not answer.
    Unreadable(String),
    /// The bytes are not a `Run` this build can parse.
    Unparsable(String),
    /// The version gate refused it.
    Moved(vellum_save::Moved),
}

impl std::fmt::Display for LoadRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadRefusal::Empty => f.write_str("there is no save in that slot"),
            LoadRefusal::Unreadable(why) => write!(f, "the save could not be read: {why}"),
            LoadRefusal::Unparsable(why) => write!(f, "the save could not be parsed: {why}"),
            // Verbatim. See the type's docs.
            LoadRefusal::Moved(moved) => write!(f, "{moved}"),
        }
    }
}

/// Write a run to a slot, through `vellum-save`'s store and nothing else.
pub fn save_to<S: vellum_save::Store>(
    store: &S,
    slot: &str,
    run: &StoredRun,
) -> Result<(), String> {
    let text = run.to_ron().map_err(|e| e.to_string())?;
    store.write(slot, &text).map_err(|e| e.to_string())
}

/// Read a run back and put it through the version gate before anything is
/// activated.
///
/// The gate runs *here*, before a single component is written, because that
/// ordering is the whole reason it exists: restoring first and refusing second
/// would mean a host had already half-adopted a world it is about to be told it
/// cannot have.
pub fn load_from<S: vellum_save::Store>(
    store: &S,
    slot: &str,
    current: &Versions,
) -> Result<StoredRun, LoadRefusal> {
    let text = store
        .read(slot)
        .map_err(|e| LoadRefusal::Unreadable(e.to_string()))?
        .ok_or(LoadRefusal::Empty)?;
    let run = StoredRun::from_ron(&text).map_err(|e| LoadRefusal::Unparsable(e.to_string()))?;
    run.versions.check(current).map_err(LoadRefusal::Moved)?;
    Ok(run)
}

// ── Restore ──────────────────────────────────────────────────────────────────

/// Something the capture named that the bootstrapped world did not have.
///
/// Reported rather than skipped. A restore that quietly drops a ship produces a
/// world that looks right and diverges for a reason nothing in the save points
/// at, which is strictly worse than a restore that says what it could not do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RestoreGap {
    /// A captured `EntityUuid` no entity in the target world carries.
    MissingEntity(String),
    /// A captured `AsteroidUuid` no asteroid in the target world carries.
    MissingAsteroid(String),
    /// The captured `SimRngState` has a different number of streams than this
    /// build declares — a save from before a stream was added. Refused rather
    /// than mapped by position, which would hand one call site another's
    /// sequence.
    RngStreamsMoved,
}

impl std::fmt::Display for RestoreGap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RestoreGap::MissingEntity(uuid) => {
                write!(f, "the world has no entity `{uuid}` to restore into")
            }
            RestoreGap::MissingAsteroid(uuid) => {
                write!(f, "the world has no asteroid `{uuid}` to restore into")
            }
            RestoreGap::RngStreamsMoved => f.write_str(
                "this save's generator streams do not match this build's; \
                 mapping them by position would misroute a call site's sequence",
            ),
        }
    }
}

/// What a restore actually managed to do.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RestoreReport {
    pub entities_restored: usize,
    pub asteroids_restored: usize,
    /// Entities the bootstrap spawned that the capture did not have. Despawned
    /// — a resumed world must not carry a ship the save never saw.
    pub despawned: usize,
    pub gaps: Vec<RestoreGap>,
}

impl RestoreReport {
    /// Whether every captured row found a home. A clean restore is the only
    /// one whose digest can be expected to match the capture's.
    pub fn is_complete(&self) -> bool {
        self.gaps.is_empty()
    }
}

/// Write a captured payload back over a bootstrapped world.
///
/// The world handed in must already be the *same scenario*, freshly built and
/// run far enough that its authored entities exist — see the module docs on why
/// this is an overwrite and not a spawn. `Run::scenario` and `Run::seed` are
/// what tell a host which world that is.
pub fn restore(world: &mut World, snapshot: &PhoenixSnapshot) -> RestoreReport {
    let mut report = RestoreReport::default();

    restore_entities(world, snapshot, &mut report);
    restore_asteroids(world, snapshot, &mut report);
    restore_run_scope(world, snapshot, &mut report);
    rebuild_ai_world_snapshot(world);

    report
}

/// Rebuild the AI's `WorldSnapshot` from the world this restore just wrote.
///
/// The one derivation a restore has to force, and the reason is the *cadence*
/// rather than the data. `build_world_snapshot` runs under
/// `run_if(ai_snapshot_ready)`, a latch that is a pure function of `SimTick`
/// (issue #895's anchor) — and [`restore_run_scope`] has just moved `SimTick`
/// to the capture's. So the resumed world's next arm lands on the same tick the
/// live world's does, which is right, but every tick *between* the restore and
/// that arm is spent steering from whatever snapshot the bootstrap happened to
/// leave behind — for a fresh app stopped the moment its roster appeared, an
/// empty one.
///
/// The measured consequence, and the last one this slice's continuation test
/// found: the resumed ships' radar-gated world view held no contacts at all, so
/// `seed_helm_travel_facts` resolved no target, `HelmRecoveryHistory` cleared
/// itself on the target switch, and both machines fell out of `torpedo_run` and
/// `inbound` into `acquire` on the first AI tick after a restore whose digest
/// had matched exactly.
///
/// Rebuilt rather than *stored*: the snapshot is a pure function of the world
/// at its tick, and after this restore the world already stands at that tick.
/// Putting a derivation in the save would be storing an answer the payload can
/// recompute — and one that a later build might compute differently.
fn rebuild_ai_world_snapshot(world: &mut World) {
    use bevy::ecs::system::RunSystemOnce;
    // Errors are swallowed on purpose: a bare-`App` fixture with the AI plugin
    // absent has no `WorldSnapshot` to rebuild, and that is the same
    // "partial world, partial restore" contract `capture` keeps.
    let _ = world.run_system_once(crate::ai::server::build_world_snapshot);
}

/// The run-scope resources, written last so the entity walks above cannot see a
/// half-updated tick.
fn restore_run_scope(world: &mut World, snapshot: &PhoenixSnapshot, report: &mut RestoreReport) {
    world.insert_resource(SimTick(snapshot.tick));

    if let Some(state) = snapshot.rng.clone() {
        match SimRng::from_state(state) {
            Some(rng) => world.insert_resource(rng),
            None => report.gaps.push(RestoreGap::RngStreamsMoved),
        }
    }

    if let Some(state) = snapshot.mint.clone() {
        world.insert_resource(WorldIdMint::from_state(state));
    }

    let mut restored_phase_entry: Option<GamePhase> = None;
    if let Some(phase) = snapshot.phase.clone() {
        // `State::new` rather than `NextState`: a queued transition applies on
        // the next `StateTransition`, which is a step this restore has not run
        // yet and must not depend on. The captured phase is where the world
        // already *is*.
        //
        // But `State::new` also skips `OnEnter`/`OnExit` entirely (issue #934),
        // and for a phase actually changing under this restore that silently
        // drops whatever that phase's entry effects do. Note the transition
        // here — before the direct write below — and let
        // `run_restored_phase_entry_effects` decide, once the rest of this
        // function has finished writing the resources those effects read.
        let previous = world
            .get_resource::<State<GamePhase>>()
            .map(|s| s.get().clone());
        if previous.as_ref() != Some(&phase) {
            restored_phase_entry = Some(phase.clone());
        }
        world.insert_resource(State::new(phase));
    }

    if let Some((reason, outcome)) = snapshot.game_over.clone() {
        let outcome = outcome
            .as_deref()
            .and_then(|label| crate::balance::Outcome::parse(label).ok());
        world.insert_resource(GameOverReason(reason, outcome));
    }

    if !snapshot.captain_boosts.is_empty() {
        // `toggle` on an empty store inserts; there is no bulk setter and this
        // needs none — `CaptainPriorityBoost::default()` is empty by
        // construction, so one toggle per pair reproduces the map exactly.
        let mut boosts = CaptainPriorityBoost::default();
        for (scope, objective) in &snapshot.captain_boosts {
            boosts.toggle(scope, objective);
        }
        world.insert_resource(boosts);
    }

    if let Some(data) = snapshot.world.clone() {
        world.insert_resource(WorldResource(data));
    }

    if let Some(flags) = snapshot.flags.clone() {
        if let Some(mut runtime) = world.get_resource_mut::<WorldContentRuntime>() {
            runtime.flags = flags;
        }
    }

    if let Some(secs) = snapshot.ai_policy_clock {
        world.insert_resource(crate::ship::helm_ai::AiPolicyTickClock(secs));
    }

    if !snapshot.layer_flags.is_empty() {
        if let Some(mut layers) = world.get_resource_mut::<crate::world::server::WorldLayerMap>() {
            for layer in &snapshot.layer_flags {
                if let Some(runtime) = layers.0.get_mut(&layer.path) {
                    runtime.flags = layer.flags.clone();
                }
            }
        }
    }

    restore_collisions(world, snapshot);

    // Last, now that every resource an entry effect might read (`GameOverReason`
    // above included) carries the restored value.
    if let Some(phase) = restored_phase_entry {
        run_restored_phase_entry_effects(world, phase);
    }
}

/// Re-run the observable entry effects of a restored phase transition that the
/// direct `State::new` write above skips (issue #934).
///
/// Not a blanket "run every restored phase's `OnEnter`" — that is wrong for
/// `InProgress` specifically. `ready_to_restore` (below) gates every restore on
/// the fresh app's own roster already existing, which only happens after that
/// app ran its *own* `OnEnter(InProgress)` for its own game start — the mint,
/// the spawns, the command-log reset. Re-running that schedule here would
/// re-spawn and re-reset exactly what the entity/asteroid restore above just
/// wrote. So `InProgress` gets nothing, on purpose. `Lobby` has no `OnEnter`
/// registered at all. `Loading` does (`broadcast_loading_start`), but it is a
/// transient phase a resumed run has no business landing in — capture refuses
/// to record it (see the guard where `capture` is called) — so there is
/// nothing to re-enter here either.
///
/// `GameOver` is the case the issue was filed for: a fresh app still
/// `InProgress` restoring a captured `GameOver` needs `on_game_over_enter`
/// (`server_app.rs`) and `push_game_over_hud_state`
/// (`server/viewscreen_border.rs`) to run, or the host never emits the
/// `GameOver` message and the HUD never leaves its live state. Both are
/// audited safe to re-run: they only *read* `GameOverReason` (restored above,
/// before this call) and *write* an outbox message / HUD resource — neither
/// spawns, despawns, or resets anything the rest of this restore depends on.
/// Run via `OnEnter(GameOver)` itself rather than by naming the two systems,
/// so a future addition to that schedule is covered by construction — but that
/// also means a system landing in `OnEnter(GameOver)` later needs this same
/// audit before it can be trusted here.
fn run_restored_phase_entry_effects(world: &mut World, phase: GamePhase) {
    if phase == GamePhase::GameOver {
        let _ = world.try_run_schedule(OnEnter(GamePhase::GameOver));
    }
}

fn restore_collisions(world: &mut World, snapshot: &PhoenixSnapshot) {
    let Some(mut telemetry) = world.get_resource_mut::<RunTelemetry>() else {
        return;
    };
    // Every non-collision event the bootstrap produced goes too. The resumed
    // run's telemetry is the capture's, not the capture's plus whatever the
    // bootstrap happened to generate on its way to the restore point.
    telemetry.balance_events.clear();
    for record in &snapshot.collisions {
        telemetry.balance_events.push(StampedBalanceEvent {
            tick: record.tick,
            sim_t: record.sim_t,
            event: BalanceEvent::DamageApplied {
                attacker: None,
                victim: record.victim.clone(),
                victim_kind: if record.victim_is_asteroid {
                    VictimKind::Asteroid
                } else {
                    VictimKind::Ship
                },
                weapon: WEAPON_KIND_COLLISION.to_string(),
                amount: record.amount,
                shield_absorbed: record.shield_absorbed,
                hull_damage: record.hull_damage,
                system_hit: None,
            },
        });
    }
}

/// Overwrite each captured entity's state, despawning anything the capture did
/// not have.
fn restore_entities(world: &mut World, snapshot: &PhoenixSnapshot, report: &mut RestoreReport) {
    let Some(mut query) = world.try_query::<(Entity, &EntityUuid)>() else {
        return;
    };
    let present: Vec<(Entity, String)> = query
        .iter(world)
        .map(|(entity, uuid)| (entity, uuid.0.clone()))
        .collect();

    let mut surplus = Vec::new();
    let mut matched: Vec<(Entity, &EntityState)> = Vec::new();
    for (entity, uuid) in &present {
        match snapshot.entities.iter().find(|row| &row.uuid == uuid) {
            Some(row) => matched.push((*entity, row)),
            None => surplus.push(*entity),
        }
    }
    for row in &snapshot.entities {
        if !present.iter().any(|(_, uuid)| uuid == &row.uuid) {
            report
                .gaps
                .push(RestoreGap::MissingEntity(row.uuid.clone()));
        }
    }

    let writes: Vec<(Entity, EntityState)> = matched
        .into_iter()
        .map(|(entity, row)| (entity, row.clone()))
        .collect();
    report.entities_restored = writes.len();

    for (entity, row) in writes {
        let mut entity_mut = world.entity_mut(entity);
        if let Some(p) = row.physics {
            if let Some(mut physics) = entity_mut.get_mut::<ShipPhysics>() {
                physics.x = p[0];
                physics.y = p[1];
                physics.z = p[2];
                physics.yaw = p[3];
                physics.forward_speed = p[4];
                physics.roll = p[5];
                physics.lateral_speed = p[6];
                physics.vertical_speed = p[7];
            }
            // The renderer and the physics solver both read `Transform`, so a
            // restored ship that only moved its `ShipPhysics` would sit in one
            // place and be drawn in another until the next helm integration.
            //
            // The ROTATION is written for a sharper reason than drawing:
            // `build_helm_ai_surfaces_frame` reads a target's facing straight
            // off `Transform::rotation`, not off its `ShipPhysics`. A resumed
            // world that restored only the translation therefore had every ship
            // steering against a target whose heading was the *bootstrap's* —
            // and a Harrow inbound on a bearing of pi was read as facing 0.
            //
            // Derived here rather than stored, and by the same expression
            // `physics_systems::apply_ship_physics` uses, so the two can only
            // ever agree: the transform is a projection of `ShipPhysics`, and a
            // save that stored it separately could contradict the thing it is a
            // projection of.
            if let Some(mut transform) = entity_mut.get_mut::<Transform>() {
                transform.translation = Vec3::new(p[0], p[1], p[2]);
                transform.rotation = Quat::from_euler(bevy::math::EulerRot::YXZ, -p[3], 0.0, p[5]);
            }
        }
        if let Some(rows) = &row.hull {
            if let Some(mut hull) = entity_mut.get_mut::<EntitySystemHull>() {
                apply_hull(&mut hull.0, rows);
            }
        }
        if let Some(active) = row.red_alert {
            if let Some(mut alert) = entity_mut.get_mut::<ShipRedAlert>() {
                alert.0 = active;
            }
        }
        if let Some(control) = &row.control {
            if let Some(mut thrust) = entity_mut.get_mut::<ThrustInput>() {
                thrust.0 = control.thrust;
            }
            if let Some(mut steering) = entity_mut.get_mut::<SteeringInput>() {
                steering.0 = control.steering;
            }
            if let Some(mut lateral) = entity_mut.get_mut::<LateralThrustInput>() {
                lateral.0 = control.lateral;
            }
            if let Some(mut vertical) = entity_mut.get_mut::<VerticalThrustInput>() {
                vertical.0 = control.vertical;
            }
            if let Some(mut boost) = entity_mut.get_mut::<BoostCommand>() {
                boost.0 = control.boost;
            }
            if let Some(mut impulse) = entity_mut.get_mut::<ImpulseCommand>() {
                impulse.0 = match control.impulse_phase {
                    1 => ImpulsePhase::Charging,
                    2 => ImpulsePhase::Active,
                    // Including anything this build does not recognise — see
                    // `ControlState::impulse_phase`.
                    _ => ImpulsePhase::Idle,
                };
            }
            if let Some(mut last) = entity_mut.get_mut::<LastHelmInput>() {
                last.thrust = control.last_helm[0];
                last.steering = control.last_helm[1];
                last.lateral = control.last_helm[2];
            }
            if let Some(policies) = &control.helm_policies {
                if let Some(mut state) = entity_mut.get_mut::<HelmEnginesAiPolicyState>() {
                    apply_policy_state(&mut state.0, &policies[0]);
                }
                if let Some(mut state) = entity_mut.get_mut::<HelmSteeringAiPolicyState>() {
                    apply_policy_state(&mut state.0, &policies[1]);
                }
                if let Some(mut state) = entity_mut.get_mut::<HelmBoostAiPolicyState>() {
                    apply_policy_state(&mut state.0, &policies[2]);
                }
            }
            if let Some(stored) = &control.helm_recovery {
                if let Some(mut history) =
                    entity_mut.get_mut::<crate::ship::helm_ai::HelmRecoveryHistory>()
                {
                    history.target = stored
                        .target
                        .as_deref()
                        .and_then(|t| uuid::Uuid::parse_str(t).ok());
                    history.ranges.set_capacity(stored.ranges_capacity as usize);
                    history.ranges.clear();
                    for sample in &stored.ranges {
                        history.ranges.push(*sample);
                    }
                    history
                        .separation
                        .set_capacity(stored.separation_capacity as usize);
                    history.separation.clear();
                    for sample in &stored.separation {
                        history.separation.push(*sample);
                    }
                }
            }
            if let Some(mut lock) = entity_mut.get_mut::<TacticalRadarSelection>() {
                lock.0 = control.target_lock.clone();
            }
            if let Some(mut attacker) = entity_mut.get_mut::<LastShipAttacker>() {
                // `set_if_neq` semantics matter here: `LastShipAttacker`'s
                // change detection is the rising-edge latch behind
                // `on_entity_attacked` triggers, so a blind write on restore
                // would re-fire a scenario trigger the capture had already
                // spent.
                let restored = control.last_attacker.clone();
                if attacker.0 != restored {
                    attacker.0 = restored;
                }
            }
        }
        if let Some(weapons) = &row.weapons {
            apply_weapons(&mut entity_mut, weapons);
        }
        if let Some(repair) = &row.repair {
            apply_repair(&mut entity_mut, repair);
        }
        if !row.patrol_cursors.is_empty() {
            if let Some(mut cursors) = entity_mut.get_mut::<crate::ai::server::ObjectiveCursors>() {
                cursors.0 = row
                    .patrol_cursors
                    .iter()
                    .map(|(id, index, settled)| {
                        crate::ai::patrol_cursor::PatrolCursor::restored(
                            id.clone(),
                            *index as usize,
                            *settled,
                        )
                    })
                    .collect();
            }
        }
        if !row.blackboards.is_empty() {
            if let Some(mut boards) =
                entity_mut.get_mut::<crate::server_app::ShipSystemBlackboards>()
            {
                boards.0 = row
                    .blackboards
                    .iter()
                    .map(|(id, board)| (SystemId(id.clone()), board.clone()))
                    .collect();
            }
        }
    }

    report.despawned += surplus.len();
    for entity in surplus {
        if let Ok(entity_mut) = world.get_entity_mut(entity) {
            entity_mut.despawn();
        }
    }
}

/// Put a ship's weapon state machines back mid-cycle.
///
/// Every write here is a wholesale replacement rather than a merge, and that is
/// the point: a fresh app's bootstrap ran its own tubes and its own beams on
/// the way to the restore point, and merging would leave the resumed ship
/// carrying a shot the capture never fired.
fn apply_weapons(entity: &mut EntityWorldMut<'_>, stored: &WeaponState) {
    if let Some(mut beam) = entity.get_mut::<ActiveBeam>() {
        beam.restore_live_banks(stored.beams.iter().map(
            |(bank, target, remaining, accumulator)| {
                (
                    bank.clone(),
                    ActiveBeamSlot {
                        target_uuid: target.clone(),
                        remaining_secs: *remaining,
                        damage_accumulator: *accumulator,
                    },
                )
            },
        ));
    }
    if let Some(mut cooldown) = entity.get_mut::<PhaserCooldown>() {
        cooldown.restore_banks(stored.phaser_cooldowns.iter().cloned());
    }
    if let Some(mut arcs) = entity.get_mut::<EntityShipArcHull>() {
        for (id, current, _max) in &stored.arc_hull {
            arcs.0.set_hp(id, *current);
        }
    }
    if let Some(mut torpedoes) = entity.get_mut::<TorpedoSystemResource>() {
        let system = &mut torpedoes.0;
        if let Some(remaining) = stored.torpedoes_remaining {
            system.torpedoes_remaining = remaining;
        }
        for tube in system.tubes.iter_mut() {
            let Some(row) = stored.tubes.iter().find(|t| t.id == tube.id) else {
                // A tube the save does not mention is left alone rather than
                // emptied, for `apply_hull`'s reason: an unmentioned tube is a
                // save written against a different hull, and the content digest
                // is what refuses that.
                continue;
            };
            tube.load_state = match row.load_phase {
                1 => TubeLoadState::Loading {
                    remaining: row.load_timer[0],
                    total: row.load_timer[1],
                },
                2 => TubeLoadState::Loaded,
                3 => TubeLoadState::Unloading {
                    remaining: row.load_timer[0],
                    total: row.load_timer[1],
                },
                // Including anything this build does not recognise — see
                // `TubeState::load_phase`.
                _ => TubeLoadState::Unloaded,
            };
            tube.loaded_count = row.loaded_count;
            tube.target_count = row.target_count;
            tube.active_barrels = row.active_barrels.clone();
            tube.pattern_step = row.pattern_step;
        }
        system.in_flight = stored
            .torpedoes_in_flight
            .iter()
            .map(|t| Torpedo {
                uuid: t.uuid.clone(),
                x: t.position[0],
                y: t.position[1],
                z: t.position[2],
                heading: t.heading,
                pitch: t.pitch,
                lifespan_remaining: t.lifespan_remaining,
                target_uuid: t.target_uuid.clone(),
                source_uuid: t.source_uuid.clone(),
                tube_id: t.tube_id.clone(),
                shield_pierce: t.shield_pierce,
            })
            .collect();
        system.burst_states = stored
            .bursts
            .iter()
            .map(|b| TubeBurstState {
                tube_id: b.tube_id.clone(),
                pending: b.pending,
                timer: b.timer,
                launch_x: b.launch[0],
                launch_y: b.launch[1],
                launch_z: b.launch[2],
                launch_heading: b.launch_heading,
                target_uuid: b.target_uuid.clone(),
                source_uuid: b.source_uuid.clone(),
                barrel_origins: b
                    .barrel_origins
                    .iter()
                    .map(|o| (o[0], o[1], o[2]))
                    .collect(),
                barrel_sequence: b.barrel_sequence.clone(),
                next_shot_index: b.next_shot_index,
            })
            .collect();
    }
}

/// Put a ship's repair crew back where it was standing.
fn apply_repair(entity: &mut EntityWorldMut<'_>, stored: &RepairState) {
    if let Some(mut teams) = entity.get_mut::<ShipRepairTeams>() {
        teams.0.restore_slots(&stored.teams);
    }
    if let Some(mut queue) = entity.get_mut::<RepairRequestQueue>() {
        queue.entries = stored
            .queue
            .iter()
            .map(
                |(station_id, station_label, tier, deficit)| RepairQueueEntry {
                    station_id: station_id.clone(),
                    station_label: station_label.clone(),
                    tier: *tier,
                    deficit: *deficit,
                },
            )
            .collect();
    }
    if let Some(mut alerted) = entity.get_mut::<RepairHumanAlerted>() {
        alerted.0 = stored.alerted.iter().cloned().collect();
    }
}

/// Overwrite the streamed belt: keep the rocks the capture knows, despawn the
/// ones it does not, and **spawn** the ones the target world never streamed.
///
/// The spawn half is what makes a restore authoritative over the belt rather
/// than merely corrective, and it is what Combat Test needs. A capture taken
/// after the player has flown somewhere names rocks whose cells the fresh app —
/// bootstrapped at the spawn point and stepped only far enough to raise its
/// roster — has never had in window. Reporting those as
/// [`RestoreGap::MissingAsteroid`]s and carrying on would leave the resumed
/// world short of exactly the rocks the capture's digest counted, so the digest
/// would not match and nothing in the save would say why.
///
/// A rock is spawned through `asteroid_lifecycle::rock_bundle` — the same
/// component set the streamer itself builds — so a restored rock and a streamed
/// one are the same entity, down to the `ColliderSection` that collision
/// avoidance reads an obstacle's size from.
fn restore_asteroids(world: &mut World, snapshot: &PhoenixSnapshot, report: &mut RestoreReport) {
    let Some(mut query) = world.try_query::<(Entity, &AsteroidUuid)>() else {
        return;
    };
    let present: Vec<(Entity, String)> = query
        .iter(world)
        .map(|(entity, uuid)| (entity, uuid.0.clone()))
        .collect();

    let mut surplus = Vec::new();
    let mut writes: Vec<(Entity, AsteroidState)> = Vec::new();
    for (entity, uuid) in &present {
        match snapshot.asteroids.iter().find(|row| &row.uuid == uuid) {
            Some(row) => writes.push((*entity, row.clone())),
            None => surplus.push((*entity, uuid.clone())),
        }
    }
    let missing: Vec<AsteroidState> = snapshot
        .asteroids
        .iter()
        .filter(|row| !present.iter().any(|(_, uuid)| uuid == &row.uuid))
        .cloned()
        .collect();

    report.asteroids_restored = writes.len();

    for (entity, row) in writes {
        let mut entity_mut = world.entity_mut(entity);
        if let Some(mut transform) = entity_mut.get_mut::<Transform>() {
            transform.translation =
                Vec3::new(row.translation[0], row.translation[1], row.translation[2]);
            // Not re-normalised. The stored quaternion came off a live
            // `Transform` and is already unit; normalising it again is a
            // divide that moves the low bits, and the capture this restore is
            // checked against folds bit patterns.
            transform.rotation = Quat::from_xyzw(
                row.rotation[0],
                row.rotation[1],
                row.rotation[2],
                row.rotation[3],
            );
        }
        if let Some(rows) = &row.hull {
            if let Some(mut hull) = entity_mut.get_mut::<EntitySystemHull>() {
                apply_hull(&mut hull.0, rows);
            }
        }
    }

    for (entity, uuid) in &surplus {
        if let Ok(entity_mut) = world.get_entity_mut(*entity) {
            entity_mut.despawn();
        }
        if let Some(mut map) = world.get_resource_mut::<AsteroidEntityMap>() {
            map.0.remove(uuid);
        }
    }
    report.despawned += surplus.len();

    for row in &missing {
        // Without a config path there is nothing to build the rock *from* — no
        // collider, no hull maximum, no mesh — so this stays the honest gap it
        // always was rather than becoming a rock with invented dimensions. In
        // practice it means a hand-placed rock the target world does not have,
        // which is a scenario difference the content digest is the answer to.
        let Some(config_path) = row.config_path.as_deref() else {
            report
                .gaps
                .push(RestoreGap::MissingAsteroid(row.uuid.clone()));
            continue;
        };
        let config = crate::asteroid_lifecycle::rock_config(config_path);
        let current_hp = row
            .hull
            .as_ref()
            .and_then(|rows| rows.first().map(|(_, current, _)| *current))
            .unwrap_or(config.max_hp);
        let mut spawned = world.spawn(crate::asteroid_lifecycle::rock_bundle(
            &row.uuid,
            &config,
            Vec3::new(row.translation[0], row.translation[1], row.translation[2]),
            Quat::from_xyzw(
                row.rotation[0],
                row.rotation[1],
                row.rotation[2],
                row.rotation[3],
            ),
            row.shield_pierce,
            current_hp,
        ));
        if let Some(mesh) = &config.mesh {
            spawned.insert(crate::entity_spawner::MeshSection(mesh.clone()));
        }
        let entity = spawned.id();
        if let Some(mut map) = world.get_resource_mut::<AsteroidEntityMap>() {
            map.0.insert(row.uuid.clone(), entity);
        }
        report.asteroids_restored += 1;
    }

    restore_asteroid_window(world, snapshot);
}

/// Put the streamer's own progress back, so its next tick resumes rather than
/// rebuilding. See [`AsteroidWindowState`].
fn restore_asteroid_window(world: &mut World, snapshot: &PhoenixSnapshot) {
    let Some(stored) = snapshot.asteroid_window.as_ref() else {
        return;
    };
    // Cosmetic handles belong to the app that spawned them, and the arena the
    // restore is about to install may not be the one their slots were indexed
    // against. They carry no uuid, no hull and no collider, so despawning them
    // costs a frame of set dressing and buys a window whose every remaining
    // handle is one this restore put there.
    let cosmetics: Vec<Entity> = world
        .get_resource::<AsteroidWindow>()
        .map(|window| {
            window
                .cosmetic_upper_slots
                .iter()
                .chain(window.cosmetic_lower_slots.iter())
                .flatten()
                .flatten()
                .copied()
                .collect()
        })
        .unwrap_or_default();
    for entity in cosmetics {
        if let Ok(entity_mut) = world.get_entity_mut(entity) {
            entity_mut.despawn();
        }
    }

    let Some(mut window) = world.get_resource_mut::<AsteroidWindow>() else {
        return;
    };
    let size = (2 * stored.despawn_cells + 1) as usize;
    window.slots = vec![vec![None; size]; size];
    window.cosmetic_upper_slots = vec![vec![None; size]; size];
    window.cosmetic_lower_slots = vec![vec![None; size]; size];
    for slot in &stored.slots {
        let (x, z) = (slot.x as usize, slot.z as usize);
        let Some(cell) = window.slots.get_mut(z).and_then(|row| row.get_mut(x)) else {
            continue;
        };
        *cell = Some(AsteroidData {
            uuid: slot.uuid.clone(),
            config_path: slot.config_path.clone(),
            hp: slot.hp,
            max_hp: slot.max_hp,
            y: slot.y,
        });
    }
    window.arena_gx = stored.arena_gx;
    window.arena_gz = stored.arena_gz;
    window.despawn_cells = stored.despawn_cells;
    window.spawn_cells = stored.spawn_cells;
    window.resolution = stored.resolution;
    window.player_grid = stored.player_grid;
    window.composition_key = stored.composition_key;
    window.needs_init = stored.needs_init;
}

/// Whether a bootstrapped world is far enough along to be restored into.
///
/// A fresh app does not have the scenario's ships at tick 0 — the lobby's
/// collective auto-start has to run first, and the world spawns on the phase
/// transition. So both callers that restore (the browser boot path and the
/// integration test) step the fresh app until this is true and only then
/// overwrite. It is the same question [`restore`] would otherwise answer too
/// late, as a list of [`RestoreGap::MissingEntity`]s.
pub fn ready_to_restore(world: &World, snapshot: &PhoenixSnapshot) -> bool {
    let Some(mut query) = world.try_query::<(&EntityUuid, Option<&ThrustInput>)>() else {
        return false;
    };
    // Both halves matter. A ship's `EntityUuid` appears at spawn, but its helm
    // axes are inserted a beat later, and a restore that fired in that window
    // found no `ThrustInput` to write to and silently left the ship coasting —
    // a world whose digest matched the capture exactly and diverged one tick
    // afterwards. Waiting for the controls is what closes it.
    let present: Vec<(&str, bool)> = query
        .iter(world)
        .map(|(uuid, thrust)| (uuid.0.as_str(), thrust.is_some()))
        .collect();
    let roster_ready = snapshot.entities.iter().all(|row| {
        present.iter().any(|(uuid, has_controls)| {
            *uuid == row.uuid && (*has_controls || row.control.is_none())
        })
    });
    roster_ready && belt_ready(world, snapshot)
}

/// Whether the target world's asteroid streamer has settled onto the same
/// composition the capture was taken against.
///
/// [`restore_asteroids`] is authoritative over the *rocks* — it spawns the ones
/// the capture names and despawns the ones it does not — but it cannot be
/// authoritative over a field entity that has not loaded yet.
/// `update_asteroid_window` recomputes its composition key from the live
/// `AsteroidFieldSection`s every tick, and a key that disagrees with the
/// window's is its signal that a world layer loaded or unloaded a field, which
/// it answers by clearing the window wholesale. Restoring into a world whose
/// fields were still arriving would therefore be undone by the very next tick:
/// the belt wiped, the digest silently short of every rock the capture counted.
///
/// So a capture whose streamer had settled waits for one whose streamer has
/// too. A capture with no streamed field at all — the duel arena — waits for
/// nothing, because there is nothing to disagree about.
fn belt_ready(world: &World, snapshot: &PhoenixSnapshot) -> bool {
    let Some(stored) = snapshot.asteroid_window.as_ref() else {
        return true;
    };
    if stored.needs_init {
        return true;
    }
    world
        .get_resource::<AsteroidWindow>()
        .is_some_and(|live| !live.needs_init && live.composition_key == stored.composition_key)
}

// ── Verification ─────────────────────────────────────────────────────────────

/// The `Sampling` simulation [`vellum_save::verify`] checks a restore against.
///
/// Deliberately tiny, and deliberately not `headless::replay::PhoenixSim`: that
/// type is native-only (it lives under the `headless` feature) and a browser
/// host is precisely the thing that has to be told its save will not load. This
/// adapter compiles on both targets because it does nothing but hold a world
/// and hash it.
///
/// # `apply` refuses, and that is not a stub
///
/// This issue's artifact has an **empty log** by construction — a snapshot with
/// no commands is a saved game, which is the whole of what #862 stores — so
/// `replay_into` never reaches `apply`. Refusing rather than pretending is the
/// honest encoding of that: if a log ever does arrive here, it arrived from
/// #849's continuation work, and the right answer is a named refusal rather
/// than a silent no-op that would make an unreplayed command look replayed.
/// When #849 lands, the verifier for a run *with* a log is `PhoenixSim`, which
/// already crosses the production admission boundary.
pub struct SavedGame<'a> {
    world: &'a World,
    ledger: Ledger,
}

/// Why [`SavedGame`] will not replay a command. See its docs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NoContinuationLog;

impl std::fmt::Display for NoContinuationLog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("a saved game carries no command log to replay (issue #849 adds one)")
    }
}

impl<'a> SavedGame<'a> {
    /// Wrap a restored world for verification.
    ///
    /// The ledger is empty on purpose. `verify` reads this side's ledger only
    /// to look for a *sampled* disagreement, and a saved game samples nothing;
    /// the numbers it actually compares — the capture digest and the final
    /// digest — both come from `run`, checked against `digest()` recomputed
    /// live. Handing the recorded digest in here would make the check confirm
    /// itself.
    pub fn new(world: &'a World) -> SavedGame<'a> {
        SavedGame {
            world,
            ledger: Ledger::default(),
        }
    }
}

impl vellum_replay::Simulation for SavedGame<'_> {
    type Command = LoggedCommand;
    type Rejection = NoContinuationLog;

    fn apply(&mut self, _command: &LoggedCommand) -> Result<(), NoContinuationLog> {
        Err(NoContinuationLog)
    }

    fn is_over(&self) -> bool {
        false
    }

    fn digest(&self) -> u64 {
        crate::sim_digest::world_digest(self.world)
    }
}

impl vellum_save::Sampling for SavedGame<'_> {
    fn ledger(&self) -> &Ledger {
        &self.ledger
    }

    /// A restored world already stands at the capture tick, so there is no tail
    /// to run out. Stepping here would need `&mut World` and a schedule, and
    /// would be running the simulation *inside* a verification — which is the
    /// one thing a check of "did the restore land?" must not do.
    fn advance_to(&mut self, _tick: u64) {}
}

/// Write captured per-system HP onto a hull the fresh world built from config.
///
/// `set_hp` rather than replacing the whole `SystemHull`: the tier
/// thresholds, display names and insertion order are authored config, and the
/// bootstrapped hull already has them right. A system the capture does not
/// mention is left alone rather than zeroed — an unmentioned system is a save
/// written against a different hull, which the content digest is what refuses.
fn apply_hull(hull: &mut crate::damage::SystemHull, rows: &[(String, f32, f32)]) {
    for (id, current, _max) in rows {
        hull.set_hp(&SystemId(id.clone()), *current);
    }
}
