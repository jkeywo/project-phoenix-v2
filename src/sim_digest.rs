//! The canonical authoritative-state digest (issue #901), folded exactly as
//! issue #894's record — `pasm/spec/architecture/deterministic-simulation.yaml`
//! — decided it should be.
//!
//! # What the record binds, and what this module does about it
//!
//! * **Fold order is `(namespace, tick, seq)`, compared as a tuple of numbers,
//!   never as a rendered string.** [`FoldKey`] is that tuple. Its `Ord` is the
//!   derived field order, so `namespace` groups first and the numeric pair
//!   orders within a namespace. [`FoldKey::from_world_id`] parses a minted id
//!   (issue #907) into that pair and falls back to `(0, 0)` plus the raw string
//!   for anything unminted — which, since #907, is asteroids and nothing else.
//!   The parse is `world_id::WorldId::parse`, the same definition the renderer
//!   uses, so the format has one owner. A mint renders as a **version-8 uuid
//!   whose bits are the tuple**, not as a readable `tick-seq` string: a world
//!   id's uuid shape turned out to be load-bearing (`ai::AiWorldEntity::uuid`
//!   is a real `Uuid`, and comms uses "parses as a uuid?" to tell an entity
//!   from a synthetic sender), and `world_id`'s module docs carry that finding
//!   in full. The version nibble is what distinguishes a mint from a v4 rock.
//! * **Namespaces fold in a fixed declared sequence and are never merged.**
//!   [`Namespace`] is `world_id::IdNamespace` — the mint's own enum, not a copy
//!   — and its discriminants *are* that sequence: `Entity` then `Asteroid`. A
//!   further namespace appends a variant; it must never be inserted in the
//!   middle, because that reorders every id that sorts near it and invalidates
//!   every digest ever recorded. The mint already declares two the fold does
//!   not walk (`Message`, `Projectile`), which is the append rule working.
//! * **Never ECS entity handle.** Nothing here iterates a Bevy query straight
//!   into the fold. Every walk collects, sorts by [`FoldKey`], and folds the
//!   sorted run. `Entity::index()` appears only as a same-id tiebreak, exactly
//!   as `handle_collisions` uses it.
//! * **Floats fold by canonical bit pattern, with no quantisation.** See
//!   [`canon_f32`]: NaN's ~16 million bit patterns collapse to one payload and
//!   `-0.0` folds as `+0.0`. Rounding first would report the wrong tick a split
//!   happened on, which is the one thing a per-tick digest exists to answer.
//! * **The fold point is inside the `RenderInterp` bracket** — after `SimSet`
//!   has fully committed a tick, before any frame-time interpolation. This
//!   module is only ever called between `App::update()` calls (see
//!   `headless::replay`), which is that point.
//!
//! # What is folded, and what is deferred — stated honestly
//!
//! The record's AC5 table is the authority on in/out. This implementation
//! covers the whole of its **IN** list plus everything `RunFingerprint` already
//! covered, and nothing from its **OUT**/exclusion list. Precisely:
//!
//! **Folded (the run-scope preamble):** `SimTick`; the absolute
//! `SimulationPaused` value; the applied canonical `GmActionJournal` prefix
//! (including attribution and idempotency keys); canonical `StationPuppets`
//! membership; the whole `SimRngState`
//! (master seed, its provenance, and every `SimStream`'s exact `Pcg32`
//! position) through `digest_postcard`, so a divergent *draw count* is caught
//! the tick it happens; `WorldIdMint`'s tick and per-namespace counters (issue
//! #907), the identity analogue of the same thing, so a divergent *spawn
//! count* is caught the tick it happens rather than on the tick the next id is
//! minted; `GamePhase`; `GameOverReason` (both the reason string
//! and the `Outcome`); `MissionReport`'s every row in authored order, all five
//! fields including the hidden score (issue #1344);
//! `CaptainPriorityBoost`'s every `(scope, objective)` pair
//! in sorted key order; and the `WorldResource` projection described below.
//!
//! **Folded (`EntityUuid` namespace, in `FoldKey` order):** every entity
//! carrying an `EntityUuid` — its id, then `ShipPhysics`' eight fields as bit
//! patterns, then `EntitySystemHull` per system in the hull's own stable
//! insertion order (`SystemId`, current, max), then `ShipRedAlert`.
//!
//! **Folded (infrastructure namespace, in `FoldKey` order — issue #1025):**
//! every entity carrying an `InfrastructureCondition` — its id, its condition
//! and ceiling as bit patterns, and each authored operational flag with its
//! current state. A host that disagreed about whether a skyhook can still
//! transfer disagrees about whether the mission is winnable, so this is
//! authoritative and folded. Its authored capacities are NOT folded: they never
//! move, and a divergence in them is a content divergence, which
//! `snapshot::content_digest` is the thing that catches. See
//! [`fold_infrastructure_namespace`] for why this one namespace folds *nothing*
//! when it is empty.
//!
//! **Folded (civilian namespace, in `FoldKey` order — issue #1028):** every
//! entity carrying a `CivilianTraffic` — its id, its lane, its leg, its
//! compliance state, the tick its current compliance stage is due on, and its
//! standing order as a verb plus a destination. A host that disagreed about
//! whether a hauler is complying disagrees about whether traffic control is
//! working, so this is authoritative and folded. The per-leg dwell tick is NOT
//! folded: it is re-derived from the same authored `hold_secs` on both hosts the
//! moment a leg is left. Empty-namespace rule as above.
//!
//! **Retired (weapons-hold namespace — issues #1041, #1398):** this fold
//! carried the id of every ship under a captain's weapons hold until #1398
//! retired that lever. Restraint is now a POWER order — a `weapons` group
//! commanded to level 0 — and reactor allocation is one of the entity-scope
//! rows this fold deliberately defers (see DEFERRED below), so the namespace
//! went with the state it folded rather than being re-pointed at power.
//! Removing it is digest-neutral for every run in which nobody held fire, by
//! the empty-namespace rule the namespace itself was built on: it folded
//! nothing at all unless some ship was actually holding.
//!
//! **Folded (tractor namespace, in `FoldKey` order — issue #1156):** every ship
//! whose tractor beam is holding a target — its id, its engaged flag and the
//! coupled target's uuid. A held target is a derelict, which carries no
//! `ShipPhysics`, so the entity namespace never folds its position: this
//! namespace is the only place a host records that a hulk is under tow, and two
//! hosts that disagreed about the grip disagree about where the hulk is. Empty-
//! namespace rule as above — a hull that authored a tractor and is holding
//! nothing folds nothing — see [`fold_tractor_namespace`].
//!
//! **Folded (scenario scope — issue #1086):** the scenario's own memory, in
//! five walks, between the run-scope preamble and the entity namespace. The
//! base world's `FlagStore` and every ACTIVE layer's, sorted by name and folded
//! with the layer's path, its `loader_path` and its POSITION in the activation
//! order (never the raw ordinal — see [`fold_scenario_flags`] for why the
//! payload cannot round-trip that); every trigger's authored identity (`id` plus
//! its condition's kind) and latch (`fired`, its `OnAllDestroyed` accumulation,
//! and whether a `repeat`'s cooldown stamp is set — see
//! [`fold_scenario_triggers`] for why the stamp's VALUE is not folded) with the
//! table's row count and each row's `origin_layer`;
//! the scripted `PendingCallbacks` queue and the queued `WorldEvent`s, both in
//! queue order because that order is what fires; the named records — entity
//! groups, the deadline table with its `armed` latch, the commitments ledger,
//! the evidence log, the workforce register; and the whole of `CommsState` —
//! inbox in inbox order (each message whole, its stored `sender_in_range` and
//! per-response `available` included), live dialogues sorted by message id, the
//! open-hail set and the pending scripted opens. Before this issue the fold
//! could agree about every hull in the world while the two hosts disagreed about
//! whether the mission had been won. Membership was decided by one test —
//! **scenario-scope** authoritative state a `PhoenixSnapshot` carries that the
//! fold ignored, which is narrower than it sounds and is qualified at
//! [`fold_scenario_scope`] — and the five walks take the empty-walk affordance
//! below, so a world with no scenario resources at all (the cross-target probe)
//! folds exactly as it did before. See [`fold_scenario_scope`], and
//! [`fold_scenario_records`] for the six `WorldContentRuntime` fields
//! deliberately left out — `name_to_uuid`, `observed_hull_fractions`,
//! `mission_clock_anchor_secs`, `pending_delayed_actions`,
//! `triggers.generation()` and `loaded_scenario_paths` — plus the authored
//! row fields that go with them, each with its reason.
//!
//! **Folded (`AsteroidUuid` namespace, in `FoldKey` order):** every asteroid's
//! id, its `Transform` translation as bit patterns (a rock's position is
//! authoritative — it is what a collision resolves against), and its
//! `EntitySystemHull` totals.
//!
//! **Folded (collision attribution):** every collision the run applied, as
//! `(victim uuid, damage, shield absorbed, hull damage)` in the order the
//! balance tracer saw them — the record's own AC5 line, and #896's fingerprint
//! design. This is read from `RunTelemetry`, which used to be why this module
//! lived under `headless`; issue #904 moved that resource to
//! `crate::core::telemetry` and this module out to the crate root, because a
//! digest that only exists on native cannot make a native↔wasm claim. Nothing
//! about the fold changed in the move. `crate::headless::digest` is an alias
//! for this module, so every existing path still resolves.
//!
//! **DEFERRED, and the digest may grow to cover it.** `WorldResource` folds as
//! a *projection*, not wholesale: `scenario_title`, `scenario_description`, and
//! per authored entity (sorted by uuid) the uuid, `position`, `yaw`,
//! `hull_fraction`, `shield_fraction`, `warp_out_remaining_secs` and
//! `objective_target`. The four authored **presentation** fields the record
//! names by hand — `colour`, `radar_icon`, `region_colour`, `radar_size` — are
//! deliberately left out, because `EntitySnapshot` is the record's stated
//! REJECTED shortcut and folding it whole would pull authored presentation into
//! the surface the type-shape constraint (AC4) exists to protect. The
//! `EntitySnapshot` geometry fields (`shape`, `radius`, `inner_radius`,
//! `half_extents`, `tags`) are authored-static and are also not folded today.
//!
//! Also deferred, honestly: per-*arc* shield hull (`ShipArcHull`), the
//! continuation-authoritative `ShieldsDamageHistory`,
//! `PendingShieldsThreatBearing`, `SensorsThreatState`, `CoordinationQueue`,
//! `ShipIntentNarration`, `RecentCombatActivity`, `ShipFrequencyHintState`,
//! `ShipPhaserFrequency`, `PendingTacticalFrequencyHint`, `LastSystemTiers`,
//! `PowerBrownoutState`, `ShieldsCoordinationState`, `AiHighFidelity`,
//! `LodTransitionTimer`, `ShipBoost`, `ShipImpulse`, `ShipSystemControlSources`,
//! `NavigationWaypoint`, `NavClearanceIssueState` and
//! `HelmWaypointClearance` components plus the `CoordinationEnqueueCursor`,
//! `NpcFrequencyMatchStates`, `CurrentPhaserMode` and `TrackedEntities`
//! resources. Snapshot format 14 carries their stable projections (including
//! process-local Entity keys projected through `EntityUuid`) together with the
//! exact `Time<Fixed>` overstep. Weapons state machines, power allocation,
//! modifier caches, the per-system blackboards, and
//! `WorldContentRuntime::pending_delayed_actions` (the one scenario field issue
//! #1086 could not take — the payload refuses to carry it, and folding what a
//! restore cannot reproduce would break the at-restore digest equality
//! `tests/snapshot_resume.rs` asserts; see [`fold_scenario_records`]) are
//! likewise deferred — **and the
//! `WorldResource` projection narrowing itself**: the record's reviewer table
//! lists `WorldResource` as IN unqualified, and this module is what actually
//! narrows that to the seven-field projection above rather than the resource
//! wholesale, so the narrowing belongs on this list too rather than only in
//! the paragraph above it. None of these is *excluded* by the record; they are
//! simply not folded yet (or, for the projection, folded less than the record's
//! own IN line reads). `tests/
//! authoritative_state_enumeration.rs` is the census ratchet that keeps their
//! classification honest, and this list is what a reviewer should read to know
//! the difference between "the record says no" and "this slice has not got to
//! it". Adding one is a re-blessing event under AC4.
//!
//! # Cross-instance comparability (issue #907 — closed)
//!
//! This module's claims used to be same-seed, *same-instance* claims only:
//! production world ids were `Uuid::new_v4()` strings, so the fold was stable
//! within one run and meaningless across two. Issue #907 closed that. Every
//! minted id is now `(namespace, tick, seq)` from `crate::world_id` — a
//! function of the logical tick and of the spawn order within it, both of which
//! #895 and #896 already pin — so two instances reaching tick T on the same
//! admitted inputs give the same entity the same id, and [`FoldKey`]'s numeric
//! fields are populated rather than defaulted.
//!
//! Two honest caveats remain, and neither is an instance-dependence:
//!
//! * **Asteroids key as `(0, 0)`.** `deterministic_cell_uuid` derives a rock's
//!   id from its cell coordinates, which is constraint 8's design and has to
//!   stay that way (a rock respawning must come back with the id it had). Those
//!   ids are cross-instance identical — they are a pure function of position —
//!   they are simply not *numeric*, so they sort on the string tiebreak. The
//!   `AsteroidUuid` namespace fold is therefore stable, just not tick-ordered.
//! * **The fold is only as comparable as its inputs.** Identical ids make the
//!   fold *contents and order* agree; two instances still have to have admitted
//!   the same commands and run the same schedule to agree on everything else.
//!   That is #895/#896/#899's ground, not this module's.
//!
//! # Type-shape pinning
//!
//! The moment this fold runs, every type it folds has its field order and enum
//! variant order pinned. `digest_postcard` is used only where a serde shape is
//! deliberately pinned (`SimRngState`, `GamePhase`, `Outcome`); everything else
//! folds field-by-field through [`fold_u64`]/[`fold_f32`]/[`fold_str`], which
//! makes the pinned surface visible at the call site rather than implied by a
//! `derive`.

use bevy::prelude::*;
use vellum_digest::{digest_postcard, fnv1a, fold_digest, FOLD_SEED};

use crate::civilian::{CivilianState, CivilianTraffic};
use crate::comms::server::{CommsInboxRes, CommsRuntime};
use crate::console::command::server::ShipStationStances;
use crate::console::repair::external_server::ExternalRepairDispatch;
use crate::core::balance::BalanceEvent;
use crate::core::messages::{CommsPriority, GamePhase};
use crate::core::telemetry::RunTelemetry;
use crate::dock::DockControl;
use crate::entities::spawner::{EntitySystemHull, EntityUuid};
use crate::infrastructure::{InfrastructureCondition, InfrastructureState};
use crate::lobby::WorldResource;
use crate::security::ShipSecurityTeams;
use crate::server_app::{AsteroidUuid, CaptainPriorityBoost, GameOverReason};
use crate::ship::damage::SystemHull;
use crate::ship::state::{ShipPhysics, ShipRedAlert};
use crate::sim_rng::SimRng;
use crate::sim_tick::SimTick;
use crate::tractor::TractorBeam;
use crate::umbilical::TransferUmbilical;
use crate::world::content::WorldEvent;
use crate::world::flags::FlagStore;
use crate::world::server::{WorldContentRuntime, WorldLayerMap, WorldScriptRuntime};

/// The declared namespace sequence. **Append only** — see the module docs.
///
/// Namespace membership is part of the minted id itself, so no id can collide
/// across namespaces and the sort key and the fold grouping come from the same
/// value. Since issue #907 that is literally true rather than aspirational:
/// this *is* `world_id::IdNamespace`, the enum the mint stamps into the id, not
/// a parallel copy of it that could drift out of agreement with it.
///
/// The mint declares two namespaces this module never folds (`Message`,
/// `Projectile`). That is the append rule working as intended: a namespace
/// nothing folds simply never appears in a fold.
pub use crate::world_id::IdNamespace as Namespace;

/// The fold's sort key: `(namespace, tick, seq)` compared as a tuple of
/// numbers, with the raw id as a final tiebreak.
///
/// `Ord` is the derived field order, which *is* the policy. The `id` tail is
/// not part of the declared key — it is the deterministic tiebreak that keeps
/// the sort total for ids that are *not* tick-scoped counters. Since issue #907
/// that is one population and one only: asteroids, whose ids come from
/// `deterministic_cell_uuid` and are a pure function of the rock's cell
/// coordinates, so they are cross-instance identical without being numeric (see
/// `world_id`'s module docs for why they must stay coordinate-derived). For
/// every minted id, `tick`/`seq` decide the comparison before `id` is reached.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct FoldKey {
    pub namespace: Namespace,
    pub tick: u64,
    pub seq: u64,
    pub id: String,
}

impl FoldKey {
    /// Build a key from a world id string.
    ///
    /// A minted id (issue #907) parses into the numeric pair via
    /// `world_id::WorldId::parse`, which is the single definition of the format
    /// — this module does not carry a second parser that could disagree with
    /// the renderer. Anything else keys as `(0, 0)` and sorts on the raw
    /// string: a v4 asteroid uuid, an authored literal, a test fixture.
    ///
    /// Note what this deliberately does NOT do: render `tick`/`seq` back into a
    /// string to sort on. The record is explicit that a naive `"{tick}-{seq}"`
    /// render sorts `"10-1"` before `"2-1"`, silently making fold order a
    /// function of elapsed ticks.
    ///
    /// The `namespace` argument stays, rather than being taken from the parsed
    /// id, because it is the *caller's* declaration of which walk this key
    /// belongs to — an unminted id (an asteroid, an authored literal) has no
    /// namespace of its own to read, and a minted id landing in the wrong walk
    /// must group where the walk says, not where the string says.
    pub fn from_world_id(namespace: Namespace, id: &str) -> Self {
        let parsed = crate::world_id::WorldId::parse(id);
        Self {
            namespace,
            tick: parsed.map(|w| w.tick).unwrap_or(0),
            seq: parsed.map(|w| w.seq).unwrap_or(0),
            id: id.to_string(),
        }
    }
}

/// The payload every NaN folds as.
///
/// One value for all ~16 million quiet/signalling NaN bit patterns, so two
/// instances that both produced "not a number" agree even if they produced a
/// different *flavour* of it.
const CANONICAL_NAN: u32 = 0x7fc0_0000;

/// Canonicalise a float to the bit pattern it folds as.
///
/// No quantisation, ever — see the module docs. Two adjustments only: NaN
/// collapses to [`CANONICAL_NAN`], and `-0.0` folds as `+0.0` (the two compare
/// equal but differ in bits, so folding them apart would report a divergence
/// where none exists).
pub fn canon_f32(value: f32) -> u32 {
    if value.is_nan() {
        CANONICAL_NAN
    } else if value == 0.0 {
        0.0f32.to_bits()
    } else {
        value.to_bits()
    }
}

/// Fold a `u64` into the accumulator.
pub fn fold_u64(acc: u64, value: u64) -> u64 {
    fold_digest(acc, fnv1a(&value.to_le_bytes()))
}

/// Fold a float by its canonical bit pattern.
pub fn fold_f32(acc: u64, value: f32) -> u64 {
    fold_u64(acc, u64::from(canon_f32(value)))
}

/// Fold a string by its bytes.
pub fn fold_str(acc: u64, value: &str) -> u64 {
    fold_digest(acc, fnv1a(value.as_bytes()))
}

/// Fold an `i64` by its two's-complement bytes.
///
/// Its own helper rather than an `as u64` at each call site: a flag counter and
/// a workforce disposition are both signed, and a cast written out five times is
/// five chances to write one of them unsigned.
fn fold_i64(acc: u64, value: i64) -> u64 {
    fold_digest(acc, fnv1a(&value.to_le_bytes()))
}

/// Fold an optional string as a present/absent marker plus its bytes.
///
/// The marker is what keeps `None` and `Some("")` apart — an unowned scenario
/// row (base world) and one owned by a layer whose path is empty are different
/// states, and a bare `unwrap_or_default()` would fold them alike.
fn fold_optional_str(acc: u64, value: Option<&str>) -> u64 {
    match value {
        Some(text) => fold_str(fold_u64(acc, 1), text),
        None => fold_u64(acc, 0),
    }
}

/// Fold an optional number as a present/absent marker plus its value.
///
/// [`fold_optional_str`]'s reason, one step sharper: an authored
/// `ai_weight = 0` and an unauthored one are different instructions to the
/// Backfill picker (forbidden versus "this is not a weighted decision"), so
/// folding them alike would hide exactly the disagreement worth catching.
fn fold_optional_u64(acc: u64, value: Option<u64>) -> u64 {
    match value {
        Some(number) => fold_u64(fold_u64(acc, 1), number),
        None => fold_u64(acc, 0),
    }
}

/// Fold a value whose serde shape is deliberately pinned.
pub fn fold_serde<T: serde::Serialize>(acc: u64, value: &T) -> u64 {
    fold_digest(acc, digest_postcard(value))
}

/// Compute the canonical authoritative-state digest for `app`'s current state.
///
/// Call this only between `App::update()` calls — the `RenderInterp` bracket
/// (see the module docs). An app with no `RunTelemetry` folds that resource's
/// "absent" marker rather than failing — which is what lets the cross-target
/// probe (`crate::cross_target_probe`, issue #904) fold through the very same
/// function a headless run does, on a target where `headless` does not exist.
pub fn state_digest(app: &App) -> u64 {
    world_digest(app.world())
}

/// [`state_digest`] against a bare `World`.
///
/// Takes `&World`, not `&mut World`, which is what lets
/// `vellum_replay::Simulation::digest(&self)` be implemented at all. Every walk
/// goes through `World::try_query`, whose `None` — a component type this world
/// never registered — folds as its own marker rather than panicking, so a
/// bare-`App` fixture produces a digest instead of a crash.
///
/// Resources are read through `get_resource` for the same reason, and an absent
/// resource folds as a distinct marker so "absent" and "present and empty" are
/// never the same number.
///
/// **The scenario scope is the one exception, deliberately** (issue #1086).
/// [`fold_scenario_scope`]'s five walks take the empty-walk affordance
/// [`fold_infrastructure_namespace`] established: with nothing to say they fold
/// nothing at all, so for them "absent" and "present and empty" ARE the same
/// number. That is safe because the four containers involved are unconditionally
/// `init_resource`'d by the plugins that own them — `WorldContentRuntime` and
/// `WorldLayerMap` in `world::server`, `CommsRuntime` and `CommsInboxRes` in
/// `comms::server` — so on any real host the distinction is unreachable, and the
/// fifth, `WorldScriptRuntime`, is present exactly when the world (or one of its
/// layers) authored a `[script]` block, which is content every peer reads the
/// same way. What the affordance buys is the compatibility claim: a world that
/// registers no scenario resource at all — the cross-target probe, which builds
/// its world from Rust literals — folds to precisely the number it folded before
/// the widening, so `tests/fixtures/cross-target-ledger.json` is untouched.
pub fn world_digest(world: &World) -> u64 {
    let mut acc = FOLD_SEED;
    for (_, fold) in FOLD_STAGES {
        acc = fold(world, acc);
    }
    acc
}

/// The fold, in order, with each stage named.
///
/// [`world_digest`] is the sum of these and nothing else, so the list cannot
/// drift out of step with what it folds — adding a namespace here is what adds
/// it to the digest.
type FoldStage = (&'static str, fn(&World, u64) -> u64);
const FOLD_STAGES: &[FoldStage] = &[
    ("run", fold_run_scope),
    ("scenario", fold_scenario_scope),
    ("entity", fold_entity_namespace),
    ("infrastructure", fold_infrastructure_namespace),
    ("civilian", fold_civilian_namespace),
    ("station-stances", fold_station_stances_namespace),
    ("tractor", fold_tractor_namespace),
    ("dock", fold_dock_namespace),
    ("external-repair", fold_external_repair_namespace),
    ("umbilical", fold_umbilical_namespace),
    ("asteroid", fold_asteroid_namespace),
    ("collisions", fold_collisions),
    ("security", fold_security_namespace),
];

/// The running accumulator after each named stage of the fold.
///
/// A divergence diagnostic (issue #1116). "Two hosts disagree at tick 240" is
/// where an investigation starts and not where it can usefully stop; comparing
/// these two lists says *which scope* they disagree in — the entity namespace,
/// the scenario's flags and scheduled work, the station stances — which is the
/// difference between a bisection and a look.
///
/// Cheap to call and free not to: it is exactly the work [`world_digest`]
/// already does, and nothing calls it on a healthy tick.
///
/// It is **not** a second digest and must never become one. The accumulator is
/// threaded through the same stages in the same order, so the last entry here
/// is always `world_digest`'s answer — a property
/// [`the_stage_breakdown_ends_where_the_digest_does`] pins.
pub fn digest_stages(world: &World) -> Vec<(&'static str, u64)> {
    let mut acc = FOLD_SEED;
    FOLD_STAGES
        .iter()
        .map(|(name, fold)| {
            acc = fold(world, acc);
            (*name, acc)
        })
        .collect()
}

/// The first named scope in which two hosts' folds disagree.
///
/// `None` when they agree. The stages are cumulative, so the first difference
/// is the first scope that actually diverged — every later one inherits it.
pub fn first_divergent_scope(mine: &World, theirs: &[(&'static str, u64)]) -> Option<&'static str> {
    digest_stages(mine)
        .into_iter()
        .zip(theirs.iter())
        .find(|((_, a), (_, b))| a != b)
        .map(|((name, _), _)| name)
}

/// The run-scope preamble: tick, typed GM pause/action frontier, RNG, phase,
/// ending, captain boosts, world.
fn fold_run_scope(world: &World, mut acc: u64) -> u64 {
    acc = fold_u64(acc, world.get_resource::<SimTick>().map_or(0, |t| t.0));

    // Typed GM control becomes current authoritative state at its application
    // boundary, not when a transport happens to deliver a future owner commit.
    // Fold only the actually-applied canonical prefix; every field in that
    // prefix (operator
    // attribution and correlation included) remains part of the replay/
    // idempotency fact. Future commits are retained by the journal/snapshot but
    // cannot create a receipt-timing digest split between honest peers.
    // SimulationPaused stays separate because trusted solo-host pause remains a
    // valid writer.
    acc = match world.get_resource::<crate::gm_action::SimulationPaused>() {
        Some(paused) => fold_serde(acc, paused),
        None => fold_str(acc, "simulation-paused:absent"),
    };
    acc = match world.get_resource::<crate::gm_action::GmActionJournal>() {
        Some(journal) => fold_serde(
            acc,
            &(
                journal.initial_paused(),
                journal.applied_prefix(),
                journal.applied_results(),
                journal.recovery_generations_through(
                    world.get_resource::<SimTick>().map_or(0, |tick| tick.0),
                ),
            ),
        ),
        None => fold_str(acc, "gm-action-journal:absent"),
    };
    acc = match world.get_resource::<crate::gm_puppet::StationPuppets>() {
        Some(puppets) => fold_serde(acc, puppets),
        None => fold_str(acc, "gm-station-puppets:absent"),
    };
    acc = match world.get_resource::<crate::gm_puppet::PendingGmStationCommands>() {
        Some(commands) => fold_serde(acc, commands),
        None => fold_str(acc, "gm-station-commands:absent"),
    };

    // SimRng: the FULL state, not a probe draw. `RunFingerprint` takes one draw
    // per stream because it has no serde shape to lean on; the record puts
    // `SimRng` itself in the fold precisely because #897 moved it onto `Pcg32`,
    // which is `Serialize`. Folding the state also leaves the generators
    // untouched, so taking a digest cannot perturb the run it is measuring —
    // which a probe draw would.
    acc = match world.get_resource::<SimRng>() {
        Some(rng) => fold_serde(acc, &rng.state()),
        None => fold_str(acc, "sim-rng:absent"),
    };

    // WorldIdMint (issue #907): the identity analogue of the `SimRng` fold
    // above, and folded for the identical reason. `SimRng`'s stream positions
    // are in so a divergent DRAW COUNT is caught the tick it happens; the
    // mint's per-namespace counters are in so a divergent SPAWN COUNT is too.
    // Without it, two instances that spawned a different number of things on
    // one tick agree until the *next* id is minted, and the divergence gets
    // reported a tick late — on a run of anything, a tick late is a different
    // window and a different suspect. The tick it is scoped to is folded
    // separately (`SimTick`, above), so this contributes the counters.
    acc = match world.get_resource::<crate::world_id::WorldIdMint>() {
        Some(mint) => {
            acc = fold_u64(acc, mint.tick());
            for namespace in crate::world_id::IdNamespace::ALL {
                acc = fold_u64(acc, mint.minted_so_far(namespace));
            }
            acc
        }
        None => fold_str(acc, "world-id-mint:absent"),
    };

    acc = match world.get_resource::<State<GamePhase>>() {
        Some(phase) => fold_serde(acc, phase.get()),
        None => fold_str(acc, "game-phase:absent"),
    };

    acc = match world.get_resource::<GameOverReason>() {
        // Both halves: the free-form reason string AND the structured
        // `Outcome`. Two instances reaching GameOver for different reasons on
        // the same seed is a correctness bug, and so is reaching it with the
        // same words for opposite sides. `Outcome` folds through its own
        // `as_str` label rather than `digest_postcard` because it is not
        // `Serialize` — one fewer type whose variant order is pinned surface,
        // at the cost of nothing: the labels are already this run's report
        // vocabulary.
        Some(GameOverReason(reason, outcome)) => {
            let acc = fold_str(acc, reason.as_deref().unwrap_or("\u{0}none"));
            fold_str(acc, outcome.map_or("\u{0}none", |o| o.as_str()))
        }
        None => fold_str(acc, "game-over-reason:absent"),
    };

    // The structured post-mission report (issue #1344). Every field of every
    // row, in the report's own authored order — never sorted, because the ORDER
    // is authored content: two instances that agree on the rows but disagree on
    // their order would show two different reports and must not share a digest.
    // The hidden score folds like everything else; it is authoritative state
    // that merely never reaches a player.
    acc = match world.get_resource::<crate::core::report::MissionReport>() {
        Some(report) => {
            let mut acc = fold_u64(acc, report.rows().len() as u64);
            for row in report.rows() {
                acc = fold_str(acc, &row.id);
                acc = fold_str(acc, &row.heading_id);
                acc = fold_str(acc, &row.outcome_id);
                acc = fold_str(acc, row.state.as_str());
                acc = fold_i64(acc, i64::from(row.score));
            }
            acc
        }
        None => fold_str(acc, "mission-report:absent"),
    };

    // Sorted by scope key, never HashMap iteration order. `boosts_sorted`
    // already returns its pairs sorted, so re-sorting here was a no-op left
    // over from before that accessor existed.
    acc = match world.get_resource::<CaptainPriorityBoost>() {
        Some(boosts) => {
            let pairs = boosts.boosts_sorted();
            let mut acc = fold_u64(acc, pairs.len() as u64);
            for (scope, objective) in pairs {
                acc = fold_str(acc, scope);
                acc = fold_str(acc, objective);
            }
            acc
        }
        None => fold_str(acc, "captain-priority-boost:absent"),
    };

    match world.get_resource::<WorldResource>() {
        Some(world_res) => fold_world_projection(acc, world_res),
        None => fold_str(acc, "world-resource:absent"),
    }
}

/// `WorldResource`'s authoritative projection — see the module docs for exactly
/// which fields are in and which are deferred, and why the four authored
/// presentation fields are not folded.
fn fold_world_projection(mut acc: u64, world: &WorldResource) -> u64 {
    acc = fold_str(acc, &world.0.scenario_title);
    acc = fold_str(acc, &world.0.scenario_description);

    let mut entities: Vec<_> = world.0.entities.iter().collect();
    // Authored order is a function of world-file parse order plus streamed
    // asteroid pushes, so it is sorted by uuid rather than walked as-is.
    entities.sort_by(|a, b| a.uuid.cmp(&b.uuid));
    acc = fold_u64(acc, entities.len() as u64);
    for snapshot in entities {
        acc = fold_str(acc, &snapshot.uuid);
        acc = fold_optional_triple(acc, snapshot.position);
        acc = fold_optional_f32(acc, snapshot.yaw);
        acc = fold_optional_f32(acc, snapshot.hull_fraction);
        acc = fold_optional_f32(acc, snapshot.shield_fraction);
        acc = fold_optional_f32(acc, snapshot.warp_out_remaining_secs);
        acc = fold_u64(acc, u64::from(snapshot.objective_target));
    }
    acc
}

fn fold_optional_f32(acc: u64, value: Option<f32>) -> u64 {
    match value {
        Some(v) => fold_f32(fold_u64(acc, 1), v),
        None => fold_u64(acc, 0),
    }
}

fn fold_optional_triple(acc: u64, value: Option<[f32; 3]>) -> u64 {
    match value {
        Some(v) => v.iter().fold(fold_u64(acc, 1), |acc, c| fold_f32(acc, *c)),
        None => fold_u64(acc, 0),
    }
}

// ── Scenario scope (issue #1086) ─────────────────────────────────────────────

/// The **scenario** the run is playing: its flags, its trigger latches, its
/// scheduled future work, its named records, and the conversation it is in the
/// middle of having.
///
/// # Why this exists
///
/// Until issue #1086 the fold walked entities and asteroids and *nothing the
/// scenario itself remembers*. Two hosts could agree, byte for byte, on where
/// every hull was and how much of each was left, while disagreeing about
/// whether the mission had been won: one with `lyra_clear` set and the other
/// without, one with a `ctx.schedule.after(..)` callback queued and the other
/// with it already spent, one holding an open dialogue the other had answered.
/// `p2p-design-deltas.yaml`'s `p2p-delta-scenario-state-must-ride-the-join`
/// states the consequence plainly — a cross-peer hash exchange (#1118) cannot
/// detect scenario-state divergence until the fold covers it — and the same
/// hole made the headless determinism sweeps blind to the loudest class of
/// divergence a scripted mission can produce.
///
/// # What decides membership
///
/// **Scenario-scope** authoritative state a
/// [`crate::snapshot::PhoenixSnapshot`] carries that the fold ignored. That test
/// is deliberate rather than convenient: the payload's field list is the one
/// place in this repo that has already argued, field by field, about what a
/// resumed run cannot re-derive, and anything on it is by construction something
/// a second host cannot re-derive either.
///
/// The **scenario-scope** qualifier is load-bearing and not decoration. The
/// payload carries plenty of ENTITY-scope state this walk does not reach — the
/// weapons rows, the reactor allocation, the repair and control rows, the helm
/// pass surface, the scan reading, the spawn recipes — and none of it was
/// considered and rejected here. It is deferred, and the module docs' DEFERRED
/// paragraph is where it is accounted for. What THIS test leaves out inside its
/// own scope is listed at [`fold_scenario_records`].
///
/// # Cheapness, and the empty-walk affordance
///
/// This runs once per digest sample (and once per save and per restore) rather
/// than once per logical tick — no Bevy schedule registers it, and its callers
/// are `headless::replay`'s sampler, `cross_target_probe`, the resume tests, and
/// `server::bridge`'s save/restore pair. A per-tick exchange is #1118's to
/// introduce. Every walk is nonetheless a borrow-and-fold: no clone of the
/// inbox, no serialisation of a whole structure, and a sort only where the
/// runtime's own container is a `HashMap`/`HashSet` whose iteration order must
/// never reach a fold (the flag stores, the trigger accumulations, the entity
/// groups, the live dialogues). Everything else is already an ordered
/// `Vec`/`BTreeSet` whose order is *load-bearing* — a `PendingCallbacks` queue
/// fires in its own order, and sorting the fold would hide a reordering that
/// changes what the mission does.
///
/// When #1118 does put this on the per-tick path, the cheap encodings are
/// already available and do not change a folded number: `FlagStore` and
/// `entity_groups` can fold through a cached sorted key list invalidated by a
/// generation counter — the pattern `triggers.generation()` already
/// establishes for the trigger table — and the layer vector can be folded from a
/// scratch buffer rather than a fresh `Vec` per call.
///
/// Each of the five walks takes [`fold_infrastructure_namespace`]'s
/// empty-walk affordance: with nothing to say it folds **nothing at all**, not
/// even a marker. That is what keeps a scenario-free world — the cross-target
/// probe (`crate::cross_target_probe`), which builds its world from Rust
/// literals and registers no scenario resource whatsoever — folding to exactly
/// the number it folded before this issue, so its committed ledger and the
/// browser half of that claim are untouched by this widening.
fn fold_scenario_scope(world: &World, mut acc: u64) -> u64 {
    acc = fold_scenario_flags(world, acc);
    acc = fold_scenario_triggers(world, acc);
    acc = fold_scenario_schedule(world, acc);
    acc = fold_scenario_records(world, acc);
    fold_comms_scope(world, acc)
}

/// The base world's [`FlagStore`] and every active layer's, in activation order.
///
/// A flag is the scenario's memory: `when = "…"` predicates gate triggers on it,
/// scripted handlers read and write it, and the campaign projection is built
/// from it. Two hosts that disagree about one counter disagree about which
/// triggers may still fire.
///
/// The **layer** half is folded with its identity and its PLACE, not merely its
/// flags, because the composition is what decides which triggers exist: a host
/// that loaded a supporting world its peer did not has a different set of
/// triggers, a different `parent:` chain and a different `FlagStore` to resolve
/// names against. Only ACTIVE layers are folded — a failed-load sentinel
/// occupies a path to suppress retries and is explicitly not part of the
/// composition a snapshot recreates.
///
/// # The place is the ENUMERATE INDEX, never the runtime ordinal
///
/// `WorldRuntime::activation_order` is a live counter — `max(active) + 1` at
/// each load — so unloading a layer from the middle leaves the survivors with
/// GAPPED ordinals, and nothing puts them back. The payload deliberately does
/// not carry it: [`crate::snapshot::LayerFlags`] stores `path`, `loader_path`,
/// the declared entity uuids and the flags, and says in terms that "the vector
/// position is the stored order". `reconcile_world_layers` rebuilds a resumed
/// composition by loading the saved paths in that vector order, so the resumed
/// layers are renumbered from one.
///
/// Folding the raw ordinal therefore made a live run with gapped ordinals fail
/// its OWN restore's digest equality, reported to the player as save corruption
/// (`snapshot::restore`). The enumerate index of the already-sorted vector is
/// exactly what the payload's vector position stores and exactly what the
/// restore reproduces, so it keeps every cross-peer claim — a layer loaded on
/// one host and not the other still moves the digest, and so does a reordered
/// composition — without claiming a number the payload cannot round-trip.
///
/// `loader_path` folds beside the path because it is the other half of the key
/// the reconcile compares on, it is payload-carried, and it is what
/// `layered_flag_chain` resolves every `parent:`-prefixed flag name through: two
/// hosts that agreed on the layer set but disagreed about who loaded what
/// resolve the same flag write into different stores.
///
/// Both stores sort by name: a `FlagStore` is a `HashMap` and says so, and
/// "set" is its whole vocabulary — `set_flag_value(name, 0)` removes the entry
/// rather than storing a zero — so an unset name and a cleared one fold alike,
/// which is what every other reader already believes.
fn fold_scenario_flags(world: &World, mut acc: u64) -> u64 {
    let base = world
        .get_resource::<WorldContentRuntime>()
        .map(|runtime| sorted_flags(&runtime.flags))
        .unwrap_or_default();

    type LayerRow<'a> = (u64, &'a str, Option<&'a str>, Vec<(&'a str, i64)>);
    let mut layers: Vec<LayerRow<'_>> = world
        .get_resource::<WorldLayerMap>()
        .map(|map| {
            map.0
                .iter()
                .filter(|(_, layer)| layer.is_active)
                .map(|(path, layer)| {
                    (
                        layer.activation_order,
                        path.as_str(),
                        layer.loader_path.as_deref(),
                        sorted_flags(&layer.flags),
                    )
                })
                .collect()
        })
        .unwrap_or_default();

    if base.is_empty() && layers.is_empty() {
        return acc;
    }
    // Activation order first, path as the tiebreak — the same key
    // `snapshot::capture_layer_flags` sorts the payload's topology by, so the
    // two orders cannot drift. The ordinal decides the SORT and is then dropped;
    // what folds is the resulting position. See the doc above.
    layers.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(b.1)));

    acc = fold_str(acc, "scenario-flags");
    acc = fold_flag_store(acc, &base);
    acc = fold_u64(acc, layers.len() as u64);
    for (index, (_order, path, loader_path, flags)) in layers.into_iter().enumerate() {
        acc = fold_u64(acc, index as u64);
        acc = fold_str(acc, path);
        acc = fold_optional_str(acc, loader_path);
        acc = fold_flag_store(acc, &flags);
    }
    acc
}

/// One flag store's set pairs, sorted by name — never `HashMap` order.
fn sorted_flags(store: &FlagStore) -> Vec<(&str, i64)> {
    let mut pairs: Vec<(&str, i64)> = store.iter().collect();
    pairs.sort_unstable_by_key(|(name, _)| *name);
    pairs
}

fn fold_flag_store(mut acc: u64, pairs: &[(&str, i64)]) -> u64 {
    acc = fold_u64(acc, pairs.len() as u64);
    for (name, value) in pairs {
        acc = fold_str(acc, name);
        acc = fold_i64(acc, *value);
    }
    acc
}

/// Every live trigger's **latch**, in `triggers` order.
///
/// The single-shot `fired` flag and the `OnAllDestroyed` accumulation — two of
/// the three fields a run moves, which is what
/// [`crate::snapshot::TriggerRuntimeState`] carries. The trigger itself
/// (condition, actions, `when`, `repeat`, `cooldown_secs`) is authored config
/// both hosts rebuild from the same content, and `snapshot::content_digest` is
/// what answers for content.
///
/// The row count is folded, and the `origin_layer` tag with each row, so a
/// table that changed SHAPE — a supporting layer loaded on one host and not the
/// other, which appends scripted triggers into the middle of this vec — moves
/// the digest immediately rather than only once one of the new triggers fires.
/// Index is positional and therefore never folded as a number: it is the order
/// itself that is being pinned.
///
/// # Each row's authored IDENTITY folds with its latch
///
/// A latch keyed only by position is exactly the hazard
/// `WorldContentRuntime::triggers.generation()` was introduced to name: since
/// a layer can be unloaded from the MIDDLE of this vec, "same length" stopped
/// implying "same triggers", and `TriggerFireRecorder` got that wrong by
/// believing it. Count plus `origin_layer` does not close it either — a reshape
/// that swapped one base-world trigger for another leaves both unchanged. So
/// each row also folds the authored `id` (`None` for an anonymous trigger, kept
/// apart from `Some("")` by [`fold_optional_str`]) and its condition's kind as a
/// small integer written out at the call site, for [`fold_world_event`]'s
/// reason. That is identity, not content: the condition's *fields* stay
/// `snapshot::content_digest`'s to answer for, and a resumed world rebuilds both
/// from the same TOML, so the pair round-trips through a restore untouched.
///
/// The generation counter itself is NOT folded. It is a function of load
/// HISTORY — how many reshapes this host has seen — rather than of state, so two
/// hosts that reached the same composition by different routes would disagree
/// about a number neither the payload carries nor the simulation reads.
///
/// The accumulation sorts because it is a `HashSet` in the runtime; the payload
/// sorts it for the same reason. Nothing else here needs a sort — the table's
/// order is a deterministic replay of the same load on every host.
///
/// # `last_fired_elapsed` folds as present-or-absent, never as its value
///
/// The third field is the mission-elapsed reading a `repeat` trigger's
/// `cooldown_secs` is measured from, and it is the one piece of scenario state
/// a restore **reconstructs** rather than copies. `restore_scenario` rebuilds
/// `mission_clock_anchor_secs` as `now - captured_elapsed` in `f32`, so every
/// later reading in a resumed world can land an ULP away from the live one's:
/// measured on `assets/worlds/probe_evidence.toml`, a trigger that fired at
/// `5.0` in the live run stamps `5.0000005` in the resumed one. Folding the
/// value would therefore report a divergence between a run and its own resume
/// that no simulation ever made, and this module forbids the quantisation that
/// would paper over it.
///
/// Losing it costs nothing a peer needs. `None` versus `Some` is folded, which
/// is what a `ResetTrigger` moves; and two hosts that fired the same repeat
/// trigger on *different ticks* have already disagreed about that trigger's
/// EFFECTS — the flags it set, the entities it spawned, the threads it opened —
/// every one of which this scope now folds, on the earlier of the two ticks.
/// The stamp would restate a divergence a tick after something else caught it.
fn fold_scenario_triggers(world: &World, mut acc: u64) -> u64 {
    let Some(runtime) = world.get_resource::<WorldContentRuntime>() else {
        return acc;
    };
    if runtime.triggers.is_empty() {
        return acc;
    }

    acc = fold_str(acc, "scenario-triggers");
    acc = fold_u64(acc, runtime.triggers.len() as u64);
    for state in runtime.triggers.iter() {
        acc = fold_optional_str(acc, state.trigger.id.as_deref());
        acc = fold_u64(acc, trigger_condition_code(&state.trigger.condition));
        acc = fold_u64(acc, u64::from(state.fired));
        acc = fold_optional_str(acc, state.origin_layer.as_deref());
        acc = fold_u64(acc, u64::from(state.last_fired_elapsed.is_some()));
        acc = fold_u64(acc, state.seen_destroyed.len() as u64);
        if !state.seen_destroyed.is_empty() {
            let mut seen: Vec<&str> = state.seen_destroyed.iter().map(String::as_str).collect();
            seen.sort_unstable();
            for name in seen {
                acc = fold_str(acc, name);
            }
        }
    }
    // The GM-operable half (issue #1301), folded ONLY when a world authors one.
    // A trigger's control set is authored config that `snapshot::content_digest`
    // answers for, exactly as the condition's fields are; what a RUN moves is
    // which Fires are armed and not yet run, and that is what folds. Keeping the
    // whole block behind an emptiness check is what leaves every existing
    // world's digest at the value it had before this issue, so no cross-target
    // ledger needed re-blessing.
    if !runtime.pending_gm_event_fires.is_empty() {
        acc = fold_str(acc, "scenario-gm-event-fires");
        acc = fold_u64(acc, runtime.pending_gm_event_fires.len() as u64);
        // A `BTreeSet`, so this walk is already the sorted one every peer makes.
        for id in &runtime.pending_gm_event_fires {
            acc = fold_str(acc, id);
        }
    }
    acc
}

/// A trigger condition's KIND as a small integer, matched at the call site.
///
/// Half of a trigger row's authored identity — see [`fold_scenario_triggers`].
/// Written out rather than folded through a `derive`, for [`fold_world_event`]'s
/// reason: `TriggerCondition` is scenario vocabulary whose variant order this
/// fold has no business pinning. The condition's own fields are content and stay
/// out; only which KIND of trigger occupies this row is folded.
fn trigger_condition_code(condition: &crate::world::config::TriggerCondition) -> u64 {
    use crate::world::config::TriggerCondition as C;
    match condition {
        C::OnDestroyed { .. } => 0,
        C::OnAllDestroyed { .. } => 1,
        C::OnAttacked { .. } => 2,
        C::OnHullBelow { .. } => 3,
        C::OnTimer { .. } => 4,
        C::OnHailed { .. } => 5,
        C::OnFlagSet { .. } => 6,
        C::OnFlagCleared { .. } => 7,
        C::OnWorldLoaded => 8,
        C::OnEnteredRegion { .. } => 9,
        C::OnExitedRegion { .. } => 10,
        C::OnWaypointReached { .. } => 11,
        C::Manual => 12,
    }
}

/// The scenario's queued FUTURE work: scripted `after(n, |ctx| …)` callbacks and
/// the world events queued for the next tick's trigger pass.
///
/// # Why the queue order is folded as-is
///
/// `PendingCallbacks` is a `Vec` whose order is the order the scripts scheduled
/// into it, and `tick_script_callbacks` fires the due entries in exactly that
/// order — so their effects apply in that order too. Sorting the fold would hide
/// the one divergence that matters most here: two hosts holding the same set of
/// callbacks in a different order will run the same mission differently on the
/// tick they come due. The payload refuses to sort it for the identical reason
/// ([`crate::snapshot::ScenarioState::script_callbacks`]), and this is the fold
/// half of that decision.
///
/// `pending_world_events` is the same shape one step earlier: it is non-empty
/// exactly when the previous tick's delayed actions or script callbacks queued a
/// chaining event for this one, and that is a tick a capture can land on.
///
/// The event's variant folds as a small integer tag written out at the call site
/// rather than through a `derive` — `WorldEvent` is scenario vocabulary whose
/// variant order this fold has no business pinning, and the tags below match the
/// ones [`crate::snapshot::WorldEventRecord`] already writes, so the payload and
/// the fold cannot disagree about what a queued event *is*.
fn fold_scenario_schedule(world: &World, mut acc: u64) -> u64 {
    let callbacks = world
        .get_resource::<WorldScriptRuntime>()
        .map(|script| script.pending_callbacks.0.as_slice())
        .unwrap_or_default();
    let events = world
        .get_resource::<WorldContentRuntime>()
        .map(|runtime| runtime.pending_world_events.as_slice())
        .unwrap_or_default();

    if callbacks.is_empty() && events.is_empty() {
        return acc;
    }

    acc = fold_str(acc, "scenario-schedule");
    acc = fold_u64(acc, callbacks.len() as u64);
    for call in callbacks {
        acc = fold_scheduled_call(acc, call);
    }
    acc = fold_u64(acc, events.len() as u64);
    for event in events {
        acc = fold_world_event(acc, event);
    }
    acc
}

/// One `ScheduledCall` — `(fire_tick, script_path, fn_name, origin_layer)`, the
/// stable key the payload stores and the deadline table retracts against.
fn fold_scheduled_call(mut acc: u64, call: &crate::world::script::schedule::ScheduledCall) -> u64 {
    acc = fold_u64(acc, call.fire_tick);
    acc = fold_str(acc, &call.script_path);
    acc = fold_str(acc, &call.fn_name);
    fold_optional_str(acc, call.origin_layer.as_deref())
}

/// One queued `WorldEvent`, as a tag plus its own fields — see
/// [`fold_scenario_schedule`] for why the tag is written by hand.
fn fold_world_event(mut acc: u64, event: &WorldEvent) -> u64 {
    match event {
        WorldEvent::Destroyed { uuid } => {
            acc = fold_u64(acc, 0);
            fold_str(acc, uuid)
        }
        WorldEvent::Attacked {
            uuid,
            attacker_uuid,
        } => {
            acc = fold_u64(acc, 1);
            fold_str(fold_str(acc, uuid), attacker_uuid)
        }
        WorldEvent::HullDroppedBelow {
            uuid,
            previous_fraction,
            current_fraction,
        } => {
            acc = fold_u64(acc, 2);
            acc = fold_str(acc, uuid);
            fold_f32(fold_f32(acc, *previous_fraction), *current_fraction)
        }
        WorldEvent::TimerElapsed { elapsed_secs } => {
            acc = fold_u64(acc, 3);
            fold_f32(acc, *elapsed_secs)
        }
        WorldEvent::Hailed { target_uuid } => {
            acc = fold_u64(acc, 4);
            fold_str(acc, target_uuid)
        }
        WorldEvent::FlagSet { name, origin_layer } => {
            acc = fold_u64(acc, 5);
            fold_optional_str(fold_str(acc, name), origin_layer.as_deref())
        }
        WorldEvent::FlagCleared { name, origin_layer } => {
            acc = fold_u64(acc, 6);
            fold_optional_str(fold_str(acc, name), origin_layer.as_deref())
        }
        WorldEvent::WorldLoaded => fold_u64(acc, 7),
        WorldEvent::EnteredRegion { uuid } => {
            acc = fold_u64(acc, 8);
            fold_str(acc, uuid)
        }
        WorldEvent::ExitedRegion { uuid } => {
            acc = fold_u64(acc, 9);
            fold_str(acc, uuid)
        }
        WorldEvent::WaypointReached { uuid, waypoint } => {
            acc = fold_u64(acc, 10);
            fold_str(fold_str(acc, uuid), waypoint)
        }
    }
}

/// The scenario's named records: entity groups, the deadline table (#1024), the
/// commitments ledger (#1029), the evidence log (#1031) and the workforce
/// register (#1035).
///
/// Every one of these is authoritative, is carried whole by the payload, and was
/// unfolded before issue #1086 — the "authoritative but not folded" bucket
/// `pasm/spec/architecture/world-files.yaml` names for the dossier evidence
/// store, taken as a group. A host that disagreed about whether a promise was
/// kept, whether a deadline was cancelled, whether the crew had learned
/// something or whether a side is still out is playing a different mission.
///
/// The `armed` latches on the deadline table and the workforce register are
/// folded beside their rows, because they are the fields that decide whether the
/// arming systems run again — an empty-but-armed table and an empty-and-unarmed
/// one behave differently on the very next tick.
///
/// # What is deliberately NOT folded here, and why
///
/// * **Authored, immutable row fields** — a deadline's `label`/`visible`, a
///   workforce side's `label`, an evidence entry's authored strings that were
///   never a run's to move. Content is `snapshot::content_digest`'s to answer
///   for, which is the standing division [`fold_infrastructure_namespace`] makes
///   for authored capacities and [`fold_tractor_namespace`] for coupling terms.
/// * **`WorldContentRuntime::name_to_uuid`** — the payload carries it because a
///   *restore*-spawned entity has no re-run behind it to rebuild the name from.
///   Two live hosts do: they re-run the same spawns, and a divergent spawn is
///   already caught the tick it happens by the mint's per-namespace counters and
///   by the entity namespace's own ids. Folding it would fold the same
///   divergence a second time, at the cost of sorting a map every tick.
/// * **`WorldContentRuntime::observed_hull_fractions`** — a one-tick-lagged copy
///   of hull state the entity namespace already folds per system.
/// * **`WorldContentRuntime::mission_clock_anchor_secs`** — a reading of
///   `Time<Fixed>`, which is a function of the folded `SimTick`. It is also the
///   one scenario field a restore *reconstructs* rather than copies (`anchor =
///   now - captured_elapsed`), so folding it would compare an `f32`
///   round-trip rather than a state.
/// * **`WorldContentRuntime::pending_delayed_actions`** — authoritative, and the
///   one genuine hole this issue leaves. The payload deliberately does not carry
///   it: storing it means giving `TriggerAction`'s 22 variants a serde derive and
///   pinning six authored-config types' shape as save format, which
///   `snapshot::ScenarioState` refuses. Folding what a restore cannot reproduce
///   would make `tests/snapshot_resume.rs`'s at-restore digest equality fail for
///   any world that used it, so the fold follows the payload. No shipped world
///   authors `action_delays`, so the queue is empty everywhere today; closing it
///   properly belongs with the issue that widens the payload.
/// * **`WorldContentRuntime::triggers.generation()`** — declared at its own
///   definition as a cache-invalidation token for index-keyed observers rather
///   than authoritative state. It counts this host's load HISTORY, not its
///   state; [`fold_scenario_triggers`] closes the hazard it names by folding
///   each row's authored identity instead.
/// * **`WorldContentRuntime::loaded_scenario_paths`** — and this one is left out
///   *honestly rather than comfortably*. It is genuinely behaviour-bearing: it
///   is the dedup set `apply_world_layer_changes` inserts into on both a
///   successful and a failed load, and a host that had it and its peer did not
///   would short-circuit a later `LoadWorld` the peer performed. The reason it
///   stays out is the membership rule and nothing better: the payload does not
///   carry it, so a resumed world rebuilds the set from the loads the reconcile
///   actually replays, and folding what a restore cannot reproduce would break
///   the at-restore digest equality for exactly the worlds that unload a layer.
///   The active-layer walk in [`fold_scenario_flags`] covers the composition it
///   dedupes against, which is why nothing shipped today can diverge on it
///   invisibly — but a failed-load sentinel's path is in this set and in no
///   fold, so it is the first candidate for a widening once the payload carries
///   it (#1118's hardening).
fn fold_scenario_records(world: &World, mut acc: u64) -> u64 {
    let Some(runtime) = world.get_resource::<WorldContentRuntime>() else {
        return acc;
    };
    let deadlines = &runtime.deadlines;
    let workforce = &runtime.workforce;
    if runtime.entity_groups.is_empty()
        && deadlines.records.is_empty()
        && !deadlines.armed
        && runtime.commitments.is_empty()
        && runtime.evidence.is_empty()
        && workforce.records.is_empty()
        && !workforce.armed
    {
        return acc;
    }

    acc = fold_str(acc, "scenario-records");

    // Groups: a `HashMap` of `HashSet`s in the runtime, so both levels sort.
    let mut groups: Vec<(&str, Vec<&str>)> = runtime
        .entity_groups
        .iter()
        .map(|(group, members)| {
            let mut names: Vec<&str> = members.iter().map(String::as_str).collect();
            names.sort_unstable();
            (group.as_str(), names)
        })
        .collect();
    groups.sort_unstable_by_key(|(group, _)| *group);
    acc = fold_u64(acc, groups.len() as u64);
    for (group, members) in groups {
        acc = fold_str(acc, group);
        acc = fold_u64(acc, members.len() as u64);
        for name in members {
            acc = fold_str(acc, name);
        }
    }

    // Deadlines, in authored order — never sorted, for the payload's reason.
    acc = fold_u64(acc, u64::from(deadlines.armed));
    acc = fold_u64(acc, deadlines.records.len() as u64);
    for record in &deadlines.records {
        acc = fold_str(acc, &record.id);
        acc = fold_optional_str(acc, record.origin_layer.as_deref());
        acc = fold_u64(acc, record.due_tick);
        acc = fold_str(acc, record.state.as_str());
        acc = match &record.armed {
            Some(call) => fold_scheduled_call(fold_u64(acc, 1), call),
            None => fold_u64(acc, 0),
        };
    }

    // Promises, in the order they were made.
    acc = fold_u64(acc, runtime.commitments.records.len() as u64);
    for record in &runtime.commitments.records {
        acc = fold_str(acc, &record.id);
        acc = fold_str(acc, &record.made_to);
        acc = fold_str(acc, &record.terms);
        acc = fold_str(acc, &record.resolves_when);
        acc = fold_str(acc, record.state.as_str());
        acc = fold_u64(acc, record.made_at_tick);
        acc = match record.resolved_at_tick {
            Some(tick) => fold_u64(fold_u64(acc, 1), tick),
            None => fold_u64(acc, 0),
        };
    }

    // Findings, in the order they were made.
    acc = fold_u64(acc, runtime.evidence.entries.len() as u64);
    for entry in &runtime.evidence.entries {
        acc = fold_str(acc, &entry.subject_uuid);
        acc = fold_str(acc, &entry.text);
        acc = fold_str(acc, entry.provenance.as_str());
        acc = fold_u64(acc, entry.gathered_at_tick);
    }

    // The labour dispute, in authored order.
    acc = fold_u64(acc, u64::from(workforce.armed));
    acc = fold_u64(acc, workforce.records.len() as u64);
    for record in &workforce.records {
        acc = fold_str(acc, &record.id);
        acc = fold_u64(acc, u64::from(record.on_strike));
        acc = fold_i64(acc, record.disposition);
    }
    acc
}

/// The conversation the scenario is in the middle of having — the whole of
/// [`crate::snapshot::CommsState`].
///
/// The inbox in inbox order, every live dialogue sorted by the message id that
/// keys it, the open-hail set (already ordered — it is a `BTreeSet`), and the
/// scripted `open_comms` requests queued but not yet materialised into threads.
///
/// Two hosts that disagree here disagree about what the crew can answer, and the
/// pending-open queue is sharper still: `open_scripted_comms_threads` drains it
/// front-to-back and mints a message id per request from the tick-scoped
/// [`crate::world_id::WorldIdMint`], so a reordered queue hands different threads
/// different ids. Queue order is therefore folded as-is, for
/// [`fold_scenario_schedule`]'s reason.
///
/// # The inbox folds WHOLE, `sender_in_range` and `available` included
///
/// Those two used to be excluded here on the grounds that
/// `update_comms_range_flags` rewrote them every tick. It does not, and never
/// did: that system (`src/comms/server.rs`) writes only `CommsRuntime`'s
/// `contacts`, `range_flags`, `range_active`, `needs_broadcast` and its
/// `open_hails` pruning, and touches no `CommsMessage` at all. The per-message
/// stamping happens on CLONES — `broadcast_comms_state` stamps
/// `CommsInbox::messages()`, and `publish_comms_blackboard` stamps its own copy
/// — so the STORED reading is written once, at injection, and then carried
/// unchanged for the life of the message. `CommsState::inbox` stores the whole
/// `CommsMessage` verbatim, so a restore reproduces both fields exactly and
/// folding them cannot break the at-restore digest equality
/// `tests/snapshot_resume.rs` asserts.
///
/// That they fold is exactly why the injection stamp may not be a `LocalShip`
/// reading — and it was one until issue #1343. `current_sender_in_range`
/// measures from the hull THIS host projects, so on a fleet whose hulls are not
/// equidistant from the sender two peers stamped different values into a field
/// folded verbatim, from the tick the message was injected. Both injection sites
/// (`open_scripted_comms_threads`, and `handle_respond_to_message`'s follow-up
/// node) now stamp
/// [`crate::comms::server::sender_in_range_for_fleet`]: the union over every
/// `FleetSlotOf` hull, which is the reading that matches a fleet-SHARED inbox and
/// which every host computes identically. Whether a given hull may ANSWER stays
/// a per-hull question, asked of `CommsRuntime::fleet_range_flags`.
///
/// They are worth folding rather than merely safe to fold. A derelict under tow
/// carries no `ShipPhysics`, so the entity namespace folds nothing about where
/// it is; a stored `sender_in_range` taken against it is a fact the entity walk
/// cannot restate. And the field is authoritative in its own right — the Comms
/// response router refuses a reply whose message reads out of range — so two
/// hosts that disagreed about it disagree about what the crew may say.
///
/// # What is still left out, and why
///
/// `CommsRuntime`'s `contacts`, `range_flags` and `range_active` — and the
/// per-fleet-slot `fleet_range_flags` / `fleet_range_active` #1343 added beside
/// them — are genuinely
/// re-derived: `update_comms_range_flags` rebuilds all five every tick from the
/// live hailable entities and the ship and entity transforms the entity
/// namespace already folds, which is the same call
/// [`crate::snapshot::CommsState`] makes when it declines to carry them.
/// `needs_broadcast` and `last_broadcast_host` are broadcast BOOKKEEPING rather
/// than scenario state — which network peer was last sent a `CommsState`, and
/// whether one is owed — and the restore re-establishes both (it sets
/// `needs_broadcast` unconditionally, because after a restore it is
/// unconditionally true). Everything else the payload stores verbatim is folded
/// verbatim.
fn fold_comms_scope(world: &World, mut acc: u64) -> u64 {
    let inbox = world.get_resource::<CommsInboxRes>();
    let comms = world.get_resource::<CommsRuntime>();
    let opens = world
        .get_resource::<WorldScriptRuntime>()
        .map(|script| script.pending_comms_opens.as_slice())
        .unwrap_or_default();

    let inbox_len = inbox.map_or(0, |inbox| inbox.0.len());
    let dialogue_len = comms.map_or(0, |comms| comms.active_dialogues.len());
    let hail_len = comms.map_or(0, |comms| comms.open_hails.len());
    let pending_ai_len = comms.map_or(0, |comms| comms.pending_ai_responses.len());
    if inbox_len == 0
        && dialogue_len == 0
        && hail_len == 0
        && pending_ai_len == 0
        && opens.is_empty()
    {
        return acc;
    }

    acc = fold_str(acc, "comms-scope");

    acc = fold_u64(acc, inbox_len as u64);
    if let Some(inbox) = inbox {
        for message in inbox.0.iter() {
            acc = fold_str(acc, &message.id);
            acc = fold_str(acc, &message.thread_id);
            acc = fold_str(acc, &message.sender_uuid);
            acc = fold_str(acc, &message.sender_name);
            acc = fold_str(acc, &message.subject);
            acc = fold_str(acc, &message.body);
            acc = fold_text_params(acc, &message.body_params);
            acc = fold_u64(acc, u64::from(message.sender_in_range));
            acc = fold_u64(acc, message.responses.len() as u64);
            for response in &message.responses {
                acc = fold_str(acc, &response.text);
                acc = fold_u64(acc, u64::from(response.important));
                acc = fold_u64(acc, u64::from(response.available));
            }
            acc = match message.selected_response {
                Some(index) => fold_u64(fold_u64(acc, 1), index as u64),
                None => fold_u64(acc, 0),
            };
            acc = fold_u64(acc, u64::from(message.is_read));
            acc = fold_u64(acc, u64::from(message.is_orphaned));
            acc = fold_u64(acc, u64::from(message.is_urgent));
            acc = fold_u64(acc, comms_priority_code(message.priority));
        }
    }

    acc = fold_u64(acc, dialogue_len as u64);
    if let Some(comms) = comms {
        // `active_dialogues` is a `HashMap` keyed by message id; the payload
        // sorts on that key and so does this.
        let mut dialogues: Vec<(&str, &crate::comms::content::ActiveDialogue)> = comms
            .active_dialogues
            .iter()
            .map(|(id, dialogue)| (id.as_str(), dialogue))
            .collect();
        dialogues.sort_unstable_by_key(|(message_id, _)| *message_id);
        for (message_id, dialogue) in dialogues {
            acc = fold_str(acc, message_id);
            acc = fold_str(acc, &dialogue.thread_id);
            acc = fold_str(acc, &dialogue.current_node.body);
            acc = fold_text_params(acc, &dialogue.current_node.body_params);
            acc = fold_u64(acc, dialogue.current_node.responses.len() as u64);
            for response in &dialogue.current_node.responses {
                acc = fold_str(acc, &response.text);
                acc = fold_u64(acc, u64::from(response.important));
                // The Backfill choice metadata (issue #1343). Folded because it
                // decides what an unmanned console reaches for and when: two
                // peers holding different weights for the same live node will
                // answer it differently, which is a divergence the tick it
                // happens rather than the tick they disagree about the outcome.
                acc = fold_optional_u64(acc, response.ai.weight.map(u64::from));
                acc = fold_optional_u64(acc, response.ai.delay_seconds.map(u64::from));
            }
            acc = fold_str(acc, &dialogue.script.script_path);
            acc = fold_optional_str(acc, dialogue.script.origin_layer.as_deref());
            acc = fold_str(acc, &dialogue.script.node_fn);
            acc = fold_u64(acc, dialogue.script.on_pick.len() as u64);
            for on_pick in &dialogue.script.on_pick {
                acc = fold_str(acc, on_pick);
            }
        }

        acc = fold_u64(acc, hail_len as u64);
        for target in &comms.open_hails {
            acc = fold_str(acc, target);
        }

        // The unmanned console's running weighted decisions (issue #1343),
        // already in message-id order — a `BTreeMap`, chosen for exactly this.
        // Two peers that agree about every open conversation but disagree about
        // WHEN one of them gets answered have diverged, and this is the tick
        // that says so rather than the tick the answer lands.
        acc = fold_u64(acc, pending_ai_len as u64);
        for (key, record) in &comms.pending_ai_responses {
            // The fleet slot first, because the key's own ordering is slot-first
            // and because "which hull is waiting" is half of what two peers must
            // agree about — a fleet whose two consoles swapped waits folds the
            // same message ids and is still diverged.
            acc = fold_u64(acc, u64::from(key.host.0));
            acc = fold_str(acc, &key.message_id);
            acc = fold_u64(acc, record.due_tick);
            acc = fold_u64(acc, record.response_fingerprint);
        }
    } else {
        acc = fold_u64(acc, 0);
        acc = fold_u64(acc, 0);
    }

    acc = fold_u64(acc, opens.len() as u64);
    for open in opens {
        acc = fold_str(acc, &open.from);
        acc = fold_str(acc, &open.root_fn);
        acc = fold_optional_str(acc, open.display_name.as_deref());
        acc = fold_optional_str(acc, open.thread_id.as_deref());
        acc = fold_u64(acc, comms_priority_code(open.priority));
        acc = fold_u64(acc, u64::from(open.urgent));
        acc = fold_str(acc, &open.script_path);
        acc = fold_optional_str(acc, open.origin_layer.as_deref());
    }
    acc
}

/// A `{placeholder}` table, in its own key order — it is a `BTreeMap`, so the
/// order is already the sorted one every host agrees on.
fn fold_text_params(mut acc: u64, params: &std::collections::BTreeMap<String, String>) -> u64 {
    acc = fold_u64(acc, params.len() as u64);
    for (key, value) in params {
        acc = fold_str(acc, key);
        acc = fold_str(acc, value);
    }
    acc
}

/// A comms priority as a small integer, matched at the call site.
///
/// Written out rather than folded through a `derive`, for
/// [`fold_world_event`]'s reason and `GameOverReason`'s: a wire enum's variant
/// order is not this fold's to pin.
fn comms_priority_code(priority: CommsPriority) -> u64 {
    match priority {
        CommsPriority::Routine => 0,
        CommsPriority::Urgent => 1,
        CommsPriority::Critical => 2,
    }
}

/// Every `EntityUuid`-bearing entity, in [`FoldKey`] order.
fn fold_entity_namespace(world: &World, mut acc: u64) -> u64 {
    type EntityRow = (
        FoldKey,
        bevy::ecs::entity::EntityIndex,
        Option<ShipPhysics>,
        Option<SystemHull>,
        Option<bool>,
    );
    let Some(mut query) = world.try_query::<(
        Entity,
        &EntityUuid,
        Option<&ShipPhysics>,
        Option<&EntitySystemHull>,
        Option<&ShipRedAlert>,
    )>() else {
        return fold_str(acc, "entity-namespace:unregistered");
    };
    let mut rows: Vec<EntityRow> = query
        .iter(world)
        .map(|(entity, uuid, physics, hull, alert)| {
            (
                FoldKey::from_world_id(Namespace::Entity, &uuid.0),
                entity.index(),
                physics.copied(),
                hull.map(|h| h.0.clone()),
                alert.map(|a| a.0),
            )
        })
        .collect();
    // `entity.index()` is the SAME-KEY tiebreak only, never the primary key —
    // the pattern `handle_collisions` established in #896.
    rows.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));

    acc = fold_u64(acc, rows.len() as u64);
    for (key, _, physics, hull, alert) in rows {
        acc = fold_str(acc, &key.id);
        acc = fold_physics(acc, physics.as_ref());
        acc = fold_hull(acc, hull.as_ref());
        acc = match alert {
            Some(active) => fold_u64(fold_u64(acc, 1), u64::from(active)),
            None => fold_u64(acc, 0),
        };
    }
    acc
}

/// Every civilian craft's traffic state (issue #1028), in [`FoldKey`] order, in
/// its own namespace.
///
/// Its lane, its leg, its standing order and where it stands with that order.
/// Two hosts that disagreed about whether a hauler is complying would disagree
/// about whether a mission's traffic control is working, so this is
/// authoritative and folded.
///
/// The **due tick** is folded and the dwell tick is not, and that asymmetry is
/// deliberate: the due tick is the thing the machine compares against every tick
/// to decide when to answer, so two hosts holding different ones would answer on
/// different ticks. The dwell is re-derived from the same authored `hold_secs`
/// on both hosts the moment a leg is left; folding it would add a second copy of
/// a number that is already implied by the leg and the lane.
///
/// Empty-namespace rule as [`fold_infrastructure_namespace`], for the same
/// reason and with the same expiry: no shipped world authors `[civilian]`
/// traffic, so folding a row count for all of them would move every committed
/// world digest over state none of them carry.
fn fold_civilian_namespace(world: &World, mut acc: u64) -> u64 {
    let Some(mut query) = world.try_query::<(Entity, &EntityUuid, &CivilianTraffic)>() else {
        return acc;
    };
    let mut rows: Vec<(FoldKey, bevy::ecs::entity::EntityIndex, CivilianState)> = query
        .iter(world)
        .map(|(entity, uuid, traffic)| {
            (
                FoldKey::from_world_id(Namespace::Entity, &uuid.0),
                entity.index(),
                traffic.0.clone(),
            )
        })
        .collect();
    if rows.is_empty() {
        return acc;
    }
    rows.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));

    acc = fold_str(acc, "civilian-namespace");
    acc = fold_u64(acc, rows.len() as u64);
    for (key, _, state) in rows {
        acc = fold_str(acc, &key.id);
        acc = fold_str(acc, state.route().unwrap_or_default());
        acc = fold_u64(acc, state.leg() as u64);
        acc = fold_str(acc, state.compliance().as_str());
        acc = fold_u64(acc, state.due_tick());
        // The order, as the two strings a console reads it by. Folding the
        // typed enum would need a serialiser here; the verb and its destination
        // are the whole of what distinguishes one order from another.
        match state.order() {
            None => acc = fold_u64(acc, 0),
            Some(order) => {
                acc = fold_u64(acc, 1);
                acc = fold_str(acc, order.kind().as_str());
                acc = fold_str(acc, &civilian_order_destination(order));
            }
        }
    }
    acc
}

/// Where an order sends a craft, as one string: a route id, an anchor name, a
/// structure name, or nothing for a hold.
fn civilian_order_destination(order: &crate::civilian::CivilianOrder) -> String {
    use crate::civilian::CivilianOrder;
    match order {
        CivilianOrder::Hold => String::new(),
        CivilianOrder::Divert { route, anchor } => {
            route.clone().or_else(|| anchor.clone()).unwrap_or_default()
        }
        CivilianOrder::Dock { structure } => structure.clone(),
    }
}

/// Every ship carrying a stored Command stance selection (issue #1107), in
/// [`FoldKey`] order, in its own namespace.
///
/// Authoritative and folded: a stored stance is a human Command operator's
/// standing order, it persists tick to tick, and it changes what the directed
/// Station's weapons AI does — two hosts that disagreed about which stance a
/// hull is under would disagree about whether that hull's guns open up. It is
/// NOT re-derived each tick from digest-free inputs the way `HumanSeekingHosts`
/// and `VisitingStationHosts` are (those recompute from `ShipConfig` +
/// sessions + control sources every tick and so are excluded as `derived`); a
/// selection lands here only when a `SetStationStance` command is admitted and
/// stays until a later order or the AI operator clears it. So a divergent
/// selection has to be caught on the tick it happens — the same rationale the
/// retired weapons-hold namespace folded on (issues #1041, #1398).
///
/// # Only the ships that carry a selection, and per-station in id order
///
/// The empty-namespace rule of [`fold_infrastructure_namespace`]. EMPTY is the
/// load-bearing default: a hull nobody commands carries an empty
/// `ShipStationStances` and folds nothing at all here, so a run in which no
/// stance is ever selected and a run recorded before the lever existed *are the
/// same authoritative state* and fold to the same number — which is the
/// byte-identical property the whole slice is built to. Every ship is
/// spawn-inserted with an empty map (`entities::spawner`), so folding a row for
/// all of them would move every committed world digest over a lever none of
/// those runs pull.
///
/// Each selecting ship folds its per-station selections in `StationId` order —
/// the map's own iteration order is `HashMap` order, never stable across
/// instances — so two hosts fold the same selections to the same number
/// whatever order the entries were inserted in.
fn fold_station_stances_namespace(world: &World, mut acc: u64) -> u64 {
    let Some(mut query) = world.try_query::<(Entity, &EntityUuid, &ShipStationStances)>() else {
        // A world that never registered the component carries no selection — the
        // empty case above, not a distinct one.
        return acc;
    };
    let mut rows: Vec<(
        FoldKey,
        bevy::ecs::entity::EntityIndex,
        Vec<(String, String)>,
    )> = query
        .iter(world)
        .filter(|(_, _, stances)| !stances.0.is_empty())
        .map(|(entity, uuid, stances)| {
            let mut selections: Vec<(String, String)> = stances
                .0
                .iter()
                .map(|(station, stance)| (station.0.clone(), stance.clone()))
                .collect();
            selections.sort_by(|a, b| a.0.cmp(&b.0));
            (
                FoldKey::from_world_id(Namespace::Entity, &uuid.0),
                entity.index(),
                selections,
            )
        })
        .collect();
    if rows.is_empty() {
        return acc;
    }
    rows.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));

    acc = fold_str(acc, "station-stances-namespace");
    acc = fold_u64(acc, rows.len() as u64);
    for (key, _, selections) in rows {
        acc = fold_str(acc, &key.id);
        acc = fold_u64(acc, selections.len() as u64);
        for (station, stance) in selections {
            acc = fold_str(acc, &station);
            acc = fold_str(acc, &stance);
        }
    }
    acc
}

/// Every ship whose tractor beam is holding a target (issue #1156), in
/// [`FoldKey`] order, in its own namespace.
///
/// # What is folded, and why it has to be
///
/// A ship's engaged state and its coupled target, and nothing else. This is the
/// authoritative divergence signal a resume has to survive: a held target is a
/// *derelict*, and a derelict carries no `ShipPhysics`, so its position is NOT
/// folded by the entity namespace at all — the only place a host records that a
/// hulk is being dragged across the map is right here. Two hosts that disagreed
/// about whether a tractor still has a grip disagree about where that hulk is,
/// with nothing else to catch it.
///
/// The authored coupling terms are content, which `snapshot::content_digest` is
/// answerable for, so `range`, `coupling_offset` and `min_power_level` are not
/// folded; the last refusal is a projection the next tick re-derives and is left
/// out for the same reason the tow leaves its stall reason's derivations out.
///
/// The empty-walk affordance is [`fold_infrastructure_namespace`]'s, and it does the
/// same real work: a hull that authored a `[tractor]` table and is holding
/// nothing folds NOTHING — not even a row — so a shipped hull can gain a tractor
/// without moving any committed world's digest.
fn fold_tractor_namespace(world: &World, mut acc: u64) -> u64 {
    let Some(mut query) = world.try_query::<(Entity, &EntityUuid, &TractorBeam)>() else {
        // A world that never registered the component runs no tractor — the
        // empty case, not a distinct one.
        return acc;
    };
    let mut rows: Vec<(FoldKey, bevy::ecs::entity::EntityIndex, bool, String)> = query
        .iter(world)
        .filter_map(|(entity, uuid, beam)| {
            beam.coupled_target.as_ref().map(|target| {
                (
                    FoldKey::from_world_id(Namespace::Entity, &uuid.0),
                    entity.index(),
                    beam.engaged,
                    target.clone(),
                )
            })
        })
        .collect();
    if rows.is_empty() {
        return acc;
    }
    rows.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));

    acc = fold_str(acc, "tractor-namespace");
    acc = fold_u64(acc, rows.len() as u64);
    for (key, _, engaged, target) in rows {
        acc = fold_str(acc, &key.id);
        acc = fold_u64(acc, engaged as u64);
        acc = fold_str(acc, &target);
    }
    acc
}

/// Every ship DOCKED to another hull (issue #1159), in [`FoldKey`] order, in its
/// own namespace.
///
/// # What is folded, and why it has to be
///
/// The docked relationship — the docker's key and the uuid of the hull it is
/// mated to — for every ship actually docked. This is a relationship between two
/// hulls that nothing else in the digest records: while a mid-approach docker's
/// divergence shows in its own folded position, the DOCKED FACT (which two hulls
/// are joined) is authoritative state the umbilical (#1160) gates on, and a host
/// that thought two hulls were mated when they were not disagrees about what the
/// umbilical may bridge. The authored `[dock]` terms are content, which
/// `content_digest` answers for, and the approach/undock motion is already caught
/// by the ship's folded position, so neither is folded here.
///
/// The empty-walk affordance is [`fold_tractor_namespace`]'s, and does the same
/// real work: a hull that authored a `[dock]` table and is NOT docked folds
/// NOTHING — not even a row — so a shipped hull can gain docking without moving
/// any committed world's digest. The moment one ship is docked the row count is
/// in the accumulator like everyone else's.
fn fold_dock_namespace(world: &World, mut acc: u64) -> u64 {
    let Some(mut query) = world.try_query::<(Entity, &EntityUuid, &DockControl)>() else {
        // A world that never registered the component runs no docks — the empty
        // case, not a distinct one.
        return acc;
    };
    let mut rows: Vec<(FoldKey, bevy::ecs::entity::EntityIndex, String)> = query
        .iter(world)
        .filter_map(|(entity, uuid, control)| {
            control.docked_partner().map(|target| {
                (
                    FoldKey::from_world_id(Namespace::Entity, &uuid.0),
                    entity.index(),
                    target.to_string(),
                )
            })
        })
        .collect();
    if rows.is_empty() {
        return acc;
    }
    rows.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));

    acc = fold_str(acc, "dock-namespace");
    acc = fold_u64(acc, rows.len() as u64);
    for (key, _, target) in rows {
        acc = fold_str(acc, &key.id);
        acc = fold_str(acc, &target);
    }
    acc
}

/// Every ship dispatching a repair team abroad (issue #1161), in [`FoldKey`]
/// order, in its own namespace.
///
/// # What is folded, and why it has to be
///
/// The uuid of the target a team is working abroad, for every ship actually
/// dispatching one. This is the authoritative divergence signal a resume has to
/// survive with the tractor's own reason turned to repair: two hosts that
/// disagreed about whether a team was still over there would disagree about
/// whether the ally's condition is climbing and whether this hull has a team
/// free for its own sweep. The authored reach and rate are content, which
/// `snapshot::content_digest` is answerable for, so neither is folded; the last
/// refusal is a projection the next tick re-derives and is left out for the same
/// reason the tractor leaves its refusal out.
///
/// The empty-walk affordance is [`fold_infrastructure_namespace`]'s, exactly as
/// [`fold_tractor_namespace`] takes it: a hull that authored
/// `[repair.external_dispatch]` and is dispatching nobody folds NOTHING — not
/// even a row — so a shipped hull can gain the capability without moving any
/// committed world's digest.
fn fold_external_repair_namespace(world: &World, mut acc: u64) -> u64 {
    let Some(mut query) = world.try_query::<(Entity, &EntityUuid, &ExternalRepairDispatch)>()
    else {
        // A world that never registered the component runs no external dispatch
        // — the empty case, not a distinct one.
        return acc;
    };
    let mut rows: Vec<(FoldKey, bevy::ecs::entity::EntityIndex, String, u8)> = query
        .iter(world)
        .filter_map(|(entity, uuid, dispatch)| {
            dispatch.dispatched_target.as_ref().map(|target| {
                (
                    FoldKey::from_world_id(Namespace::Entity, &uuid.0),
                    entity.index(),
                    target.clone(),
                    dispatch.team_idx,
                )
            })
        })
        .collect();
    if rows.is_empty() {
        return acc;
    }
    rows.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));

    acc = fold_str(acc, "external-repair-namespace");
    acc = fold_u64(acc, rows.len() as u64);
    for (key, _, target, team_idx) in rows {
        acc = fold_str(acc, &key.id);
        acc = fold_str(acc, &target);
        // WHICH team is abroad, beside the target it is working (issue #1386).
        // It has to fold for the reason the target does: two hosts that disagree
        // about it disagree about which slot this hull has free for its own
        // damage-control sweep, and therefore about where the next internal
        // dispatch lands. The empty-walk affordance is untouched — a ship
        // dispatching nobody still folds nothing at all, so `dispatched_target`
        // folds exactly as it did.
        acc = fold_u64(acc, u64::from(team_idx));
    }
    acc
}

/// Every ship whose transfer umbilical is RUNNING (issue #1160), in [`FoldKey`]
/// order, in its own namespace.
///
/// # What is folded, and why it has to be
///
/// The running intent, for every ship whose umbilical is actively flowing. This
/// is the authoritative divergence signal a resume must survive that nothing else
/// catches: the capacity the flow moves lands on the two hulls' infrastructure
/// ledgers (which fold through their own path), but WHETHER the flow is running —
/// the state that decides whether more capacity moves next tick — is authoritative
/// state two hosts must agree on, exactly as the dock folds the docked FACT and
/// the tractor the engaged FACT. The authored terms are content, which
/// `content_digest` answers for; the carry and the last refusal are projections
/// the next tick re-derives, so none of them is folded.
///
/// The empty-walk affordance is [`fold_dock_namespace`]'s, and does the same real
/// work: a hull that authored an `[umbilical]` table and is NOT running folds
/// NOTHING — not even a row — so a shipped hull can gain an umbilical without
/// moving any committed world's digest. The moment one umbilical runs the row
/// count is in the accumulator like everyone else's.
fn fold_umbilical_namespace(world: &World, mut acc: u64) -> u64 {
    let Some(mut query) = world.try_query::<(Entity, &EntityUuid, &TransferUmbilical)>() else {
        // A world that never registered the component runs no umbilicals — the
        // empty case, not a distinct one.
        return acc;
    };
    let mut rows: Vec<(FoldKey, bevy::ecs::entity::EntityIndex)> = query
        .iter(world)
        .filter(|(_, _, umbilical)| umbilical.running)
        .map(|(entity, uuid, _)| {
            (
                FoldKey::from_world_id(Namespace::Entity, &uuid.0),
                entity.index(),
            )
        })
        .collect();
    if rows.is_empty() {
        return acc;
    }
    rows.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));

    acc = fold_str(acc, "umbilical-namespace");
    acc = fold_u64(acc, rows.len() as u64);
    for (key, _) in rows {
        acc = fold_str(acc, &key.id);
    }
    acc
}

/// Every ship with a Security team OUT (issue #1346), in [`FoldKey`] order, in
/// its own namespace.
///
/// # What is folded, and why it has to be
///
/// Each committed team's index, state and target, for every ship with anybody
/// abroad. This is the authoritative divergence signal a resume must survive that
/// nothing else catches: what a completed action leaves behind lands on the world
/// flag store (which folds through the scenario scope), but WHICH teams are out,
/// where, and how far along, is the state that decides whether the next tick
/// completes the work at all — exactly as the umbilical folds its running fact
/// and the dock the docked one. The authored terms are content, which
/// `content_digest` answers for; the risk and the last refusal are projections the
/// next tick re-derives, so neither is folded.
///
/// The elapsed phase clock is a different case, and the difference matters. It is
/// accumulated authoritative state — `tick_security_teams` adds this tick's delta
/// to it and nothing reconstructs it from anything else, which is precisely why
/// `SecuritySaveState` has to carry it. It is left UNFOLDED deliberately, not
/// because it is derived: folding it would make every tick of a live assignment a
/// fresh digest, which reports a rate rather than a fact. Two hosts whose clocks
/// drift still diverge here — one tick later, when a clock crosses a phase
/// boundary and the team's folded state changes underneath it.
///
/// The empty-walk affordance is [`fold_umbilical_namespace`]'s, and does the same
/// real work: a hull that authored `[security]` and has every team home folds
/// NOTHING — not even a row — so a shipped hull can gain Security teams without
/// moving any committed world's digest. The moment one team crosses over, the row
/// count is in the accumulator like everyone else's.
fn fold_security_namespace(world: &World, mut acc: u64) -> u64 {
    let Some(mut query) = world.try_query::<(Entity, &EntityUuid, &ShipSecurityTeams)>() else {
        // A world that never registered the component musters nobody — the empty
        // case, not a distinct one.
        return acc;
    };
    let mut rows: Vec<(
        FoldKey,
        bevy::ecs::entity::EntityIndex,
        Vec<(u8, &'static str, String)>,
    )> = query
        .iter(world)
        .filter_map(|(entity, uuid, security)| {
            let committed: Vec<(u8, &'static str, String)> = security
                .teams
                .iter()
                .enumerate()
                .filter(|(_, team)| team.is_committed())
                .map(|(index, team)| {
                    (
                        index as u8,
                        team.state.as_str(),
                        team.target.clone().unwrap_or_default(),
                    )
                })
                .collect();
            (!committed.is_empty()).then(|| {
                (
                    FoldKey::from_world_id(Namespace::Entity, &uuid.0),
                    entity.index(),
                    committed,
                )
            })
        })
        .collect();
    if rows.is_empty() {
        return acc;
    }
    rows.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));

    acc = fold_str(acc, "security-namespace");
    acc = fold_u64(acc, rows.len() as u64);
    for (key, _, committed) in rows {
        acc = fold_str(acc, &key.id);
        acc = fold_u64(acc, committed.len() as u64);
        for (index, state, target) in committed {
            acc = fold_u64(acc, index as u64);
            acc = fold_str(acc, state);
            acc = fold_str(acc, &target);
        }
    }
    acc
}

/// Every entity carrying an infrastructure condition track (issue #1025), in
/// [`FoldKey`] order, in its own namespace.
///
/// # Why this namespace folds nothing when it is empty
///
/// Every other walk here folds its row count first, so "no rows" is still a
/// number in the accumulator. That is the right shape when the population is a
/// permanent part of the simulation — "no asteroids" is a fact about the world
/// worth recording. This namespace entered while no shipped world authored
/// `[infrastructure]`; folding a zero then would have moved every committed
/// digest over absent state. That compatibility rule remains part of the fold:
/// a world with no infrastructure entities and a pre-feature world are the
/// same authoritative state and fold to the same number.
///
/// The moment one structure exists it is folded in full, and from then on the
/// count is in the accumulator like everyone else's — so this is a one-time
/// compatibility affordance, not a hole: two hosts that disagree about whether a
/// structure exists at all disagree about `rows.len()` as soon as either of them
/// has one.
fn fold_infrastructure_namespace(world: &World, mut acc: u64) -> u64 {
    let Some(mut query) = world.try_query::<(Entity, &EntityUuid, &InfrastructureCondition)>()
    else {
        // A world that never registered the component has no infrastructure —
        // the empty case above, not a distinct one.
        return acc;
    };
    let mut rows: Vec<(FoldKey, bevy::ecs::entity::EntityIndex, InfrastructureState)> = query
        .iter(world)
        .map(|(entity, uuid, condition)| {
            (
                FoldKey::from_world_id(Namespace::Entity, &uuid.0),
                entity.index(),
                condition.0.clone(),
            )
        })
        .collect();
    if rows.is_empty() {
        return acc;
    }
    rows.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));

    acc = fold_str(acc, "infrastructure-namespace");
    acc = fold_u64(acc, rows.len() as u64);
    for (key, _, state) in rows {
        acc = fold_str(acc, &key.id);
        acc = fold_f32(acc, state.condition());
        acc = fold_f32(acc, state.condition_max());
        acc = fold_u64(acc, state.flags().len() as u64);
        for (flag, held) in state.flags() {
            acc = fold_str(acc, flag);
            acc = fold_u64(acc, u64::from(held));
        }
        // A capacity-backed threshold is represented at runtime by the same
        // held flag above plus the live source level below. Its selector and
        // restore/failure lines are immutable authored content, owned by the
        // content digest rather than duplicated in this state fold.
        // Capacity LEVELS, since #1027 made them movable. Two hosts that
        // disagree about how many berths a depot has left disagree about
        // whether the transfer window can be met, which is the mission. The
        // ceiling is authored content and `content_digest` is answerable for
        // it, so only the level is folded.
        acc = fold_u64(acc, state.capacities().len() as u64);
        for capacity in state.capacities() {
            acc = fold_str(acc, &capacity.id);
            acc = fold_u64(acc, capacity.level as u64);
        }
    }
    acc
}

/// Every asteroid, in [`FoldKey`] order, in its own namespace after the
/// entities — never merged into one flat sorted run across namespaces.
fn fold_asteroid_namespace(world: &World, mut acc: u64) -> u64 {
    let Some(mut query) = world.try_query::<(
        Entity,
        &AsteroidUuid,
        Option<&Transform>,
        Option<&EntitySystemHull>,
    )>() else {
        return fold_str(acc, "asteroid-namespace:unregistered");
    };
    let mut rows: Vec<(
        FoldKey,
        bevy::ecs::entity::EntityIndex,
        Vec3,
        Option<(f32, f32)>,
    )> = query
        .iter(world)
        .map(|(entity, uuid, transform, hull)| {
            (
                FoldKey::from_world_id(Namespace::Asteroid, &uuid.0),
                entity.index(),
                transform.map(|t| t.translation).unwrap_or(Vec3::ZERO),
                hull.map(|h| (h.0.total_current(), h.0.total_max())),
            )
        })
        .collect();
    rows.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));

    acc = fold_u64(acc, rows.len() as u64);
    for (key, _, translation, hull) in rows {
        acc = fold_str(acc, &key.id);
        acc = fold_f32(acc, translation.x);
        acc = fold_f32(acc, translation.y);
        acc = fold_f32(acc, translation.z);
        acc = match hull {
            Some((current, max)) => fold_f32(fold_f32(fold_u64(acc, 1), current), max),
            None => fold_u64(acc, 0),
        };
    }
    acc
}

fn fold_physics(acc: u64, physics: Option<&ShipPhysics>) -> u64 {
    match physics {
        None => fold_u64(acc, 0),
        Some(p) => {
            let acc = fold_u64(acc, 1);
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
            .iter()
            .fold(acc, |acc, v| fold_f32(acc, *v))
        }
    }
}

/// Per-system hull, in the hull's own stable insertion order (`SystemHull` keeps
/// a parallel `order` vec for exactly this reason), not just the totals: two
/// runs can land on the same total having damaged different systems.
fn fold_hull(acc: u64, hull: Option<&SystemHull>) -> u64 {
    match hull {
        None => fold_u64(acc, 0),
        Some(hull) => {
            let mut acc = fold_u64(acc, 1);
            acc = fold_u64(acc, hull.iter().count() as u64);
            for (system_id, entry) in hull.iter() {
                acc = fold_str(acc, &system_id.0);
                acc = fold_f32(acc, entry.current);
                acc = fold_f32(acc, entry.max);
            }
            acc
        }
    }
}

/// Collision attribution, in the order the balance tracer saw them — the
/// record's AC5 line, and the part of `RunFingerprint` that is actually about
/// physics.
fn fold_collisions(world: &World, mut acc: u64) -> u64 {
    let Some(telemetry) = world.get_resource::<RunTelemetry>() else {
        return fold_str(acc, "run-telemetry:absent");
    };
    let collisions: Vec<_> = telemetry
        .balance_events
        .iter()
        .filter_map(|stamped| match &stamped.event {
            BalanceEvent::DamageApplied {
                weapon,
                victim,
                amount,
                shield_absorbed,
                hull_damage,
                ..
            } if weapon == crate::core::balance::WEAPON_KIND_COLLISION => {
                Some((victim.clone(), *amount, *shield_absorbed, *hull_damage))
            }
            _ => None,
        })
        .collect();

    acc = fold_u64(acc, collisions.len() as u64);
    for (victim, amount, shield_absorbed, hull_damage) in collisions {
        acc = fold_str(acc, &victim);
        acc = fold_f32(acc, amount);
        acc = fold_f32(acc, shield_absorbed);
        acc = fold_f32(acc, hull_damage);
    }
    acc
}

// ── The divergence ledger ────────────────────────────────────────────────────

/// One sampled digest and the tick it was taken on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Checkpoint {
    pub tick: u64,
    pub digest: u64,
}

/// Where two runs of the same log first stopped agreeing.
///
/// The `window` is the point of the whole mechanism: a bare end-state mismatch
/// says only "these two runs differ", which localises a bug to the entire run.
/// A checkpoint pair says "they agreed at tick `after`, and disagreed at tick
/// `tick`", which is a window to read a log over.
///
/// `at_end` is what keeps those two cases from being told the same story. A
/// sampled-tick mismatch (`at_end: false`) is "the state at tick `tick` already
/// disagreed" — `tick` is somewhere a divergence actually happened. Every
/// sampled checkpoint agreeing and the two runs still finishing on different
/// digests (`at_end: true`) is a DIFFERENT claim: nothing this ledger sampled
/// ever disagreed, and `tick` here is the *last agreed* checkpoint, not a tick
/// that itself diverged — the two runs parted ways somewhere in the unsampled
/// tail after it. Reporting both shapes through the same "digests first
/// disagree at tick N" sentence would say a specific tick disagreed when in
/// the second case none sampled ever did — self-contradictory, since `after`
/// and `tick` would then name the same checkpoint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Divergence {
    /// The first tick whose sampled digest disagreed. Meaningless as "the tick
    /// that disagreed" when `at_end` is true — see the field's own doc.
    pub tick: u64,
    /// The last tick both runs agreed on, if there was one. `None` means they
    /// disagreed at the very first sample.
    pub after: Option<u64>,
    /// True when every sampled checkpoint agreed and only the final digests
    /// differ — the run diverged somewhere after the last checkpoint, in the
    /// tail no sample covers. False for an ordinary sampled-tick mismatch.
    pub at_end: bool,
    pub recorded: u64,
    pub replayed: u64,
}

impl std::fmt::Display for Divergence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.at_end {
            return match self.after {
                Some(after) => write!(
                    f,
                    "every sampled tick agreed through {}; the final states differ: recorded {:#018x}, replayed {:#018x}",
                    after, self.recorded, self.replayed
                ),
                None => write!(
                    f,
                    "no ticks were sampled, and the final states differ: recorded {:#018x}, replayed {:#018x}",
                    self.recorded, self.replayed
                ),
            };
        }
        match self.after {
            Some(after) => write!(
                f,
                "digests first disagree at tick {} (last agreement tick {}): recorded {:#018x}, replayed {:#018x}",
                self.tick, after, self.recorded, self.replayed
            ),
            None => write!(
                f,
                "digests disagree from the first sample, tick {}: recorded {:#018x}, replayed {:#018x}",
                self.tick, self.recorded, self.replayed
            ),
        }
    }
}

/// The periodic digest samples a run took, plus the digest it ended on.
///
/// `interval` of `0` means sampling was off, in which case `checkpoints` is
/// empty and the ledger carries the final digest alone — the "0 disables it and
/// costs nothing" half of the design. Nothing computes a digest on a run that
/// did not ask for one.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DigestLedger {
    /// Sample every N logical ticks. `0` is off.
    pub interval: u64,
    pub checkpoints: Vec<Checkpoint>,
    /// The digest at the end of the run. Always recorded — a run that samples
    /// nothing still says where it finished.
    pub final_digest: u64,
    /// How many of the commands this run *submitted* across the production
    /// admission boundary never made it into the `CommandLog` — i.e. the
    /// authority gate refused them (issue #901 review). `PhoenixSim` computes
    /// this as submitted-minus-admitted at [`PhoenixSim::seal`][crate::
    /// headless::replay::PhoenixSim::seal] time: cheap (no per-command
    /// bookkeeping beyond a counter) and honest (it reads the same `CommandLog`
    /// a recording run writes down, rather than re-deriving authorization
    /// itself). A refusal used to be silent — a command that stopped being
    /// admitted between record and replay left no trace anywhere but a
    /// possibly-unnoticed `pwarn!` line. Comparing this field between a
    /// recorded and a replayed ledger names that a command no longer admits
    /// instead of leaving it to be inferred from a digest mismatch.
    pub refused: u64,
}

impl DigestLedger {
    pub fn new(interval: u64) -> Self {
        Self {
            interval,
            checkpoints: Vec::new(),
            final_digest: 0,
            refused: 0,
        }
    }

    /// Whether `tick` is a sampling tick. `interval == 0` is never.
    pub fn samples(&self, tick: u64) -> bool {
        self.interval != 0 && tick.is_multiple_of(self.interval)
    }

    /// Record a sample, unless one for `tick` is already the most recent.
    ///
    /// The guard matters because a frame can run zero or several fixed steps:
    /// the same `SimTick` can be observed at the top of two consecutive frames
    /// (the first frame establishes the time baseline and steps nothing), and a
    /// duplicate entry would shift every later index and make two identical
    /// runs' ledgers compare unequal.
    pub fn record(&mut self, tick: u64, digest: u64) {
        if self.checkpoints.last().is_some_and(|c| c.tick == tick) {
            return;
        }
        self.checkpoints.push(Checkpoint { tick, digest });
    }

    /// The digest this ledger sampled at `tick`, if it sampled one.
    ///
    /// Added by issue #1116 so a host can answer a peer's digest frame the
    /// moment it arrives — "did I fold the same thing at tick 300?" — rather
    /// than waiting until it has a whole ledger to compare.
    /// [`Self::first_divergence`] remains the comparator for two complete runs;
    /// this is the same question asked one sample at a time, off the same
    /// checkpoints, so the two can never disagree about what was sampled.
    pub fn digest_at(&self, tick: u64) -> Option<u64> {
        self.checkpoints
            .iter()
            .find(|c| c.tick == tick)
            .map(|c| c.digest)
    }

    /// Drop every sampled checkpoint at or before `tick`, keeping only later ones.
    ///
    /// Divergence recovery (#1118) calls this on every host's ledger once a
    /// recovery resolves: the samples at and before the recovery boundary are the
    /// divergent history the restore has just healed, so forgetting them keeps the
    /// same stale disagreement from re-triggering recovery while the post-boundary
    /// samples (which now agree) are retained.
    pub fn forget_through(&mut self, tick: u64) {
        self.checkpoints.retain(|c| c.tick > tick);
    }

    /// The first tick at which this ledger and `other` disagree.
    ///
    /// Pairs samples by *tick*, not by index, so two runs that sampled
    /// different tick sets still compare on the ticks they share. A tick only
    /// one of them sampled is not evidence of anything and is skipped. When
    /// every shared sample agrees, the final digests are compared and reported
    /// against the last agreed tick — so "they matched all the way through and
    /// then ended differently" is still a located answer rather than silence.
    pub fn first_divergence(&self, other: &Self) -> Option<Divergence> {
        let mut last_agreed = None;
        let mut theirs = other.checkpoints.iter().peekable();
        for mine in &self.checkpoints {
            // Skip any of theirs that this run never sampled.
            while theirs.peek().is_some_and(|c| c.tick < mine.tick) {
                theirs.next();
            }
            let Some(match_) = theirs.peek().filter(|c| c.tick == mine.tick) else {
                continue;
            };
            if match_.digest != mine.digest {
                return Some(Divergence {
                    tick: mine.tick,
                    after: last_agreed,
                    at_end: false,
                    recorded: mine.digest,
                    replayed: match_.digest,
                });
            }
            last_agreed = Some(mine.tick);
            theirs.next();
        }

        if self.final_digest != other.final_digest {
            return Some(Divergence {
                tick: self
                    .checkpoints
                    .last()
                    .map_or(0, |c| c.tick)
                    .max(other.checkpoints.last().map_or(0, |c| c.tick)),
                after: last_agreed,
                at_end: true,
                recorded: self.final_digest,
                replayed: other.final_digest,
            });
        }
        None
    }
}

#[cfg(test)]
#[path = "sim_digest_tests.rs"]
mod tests;
