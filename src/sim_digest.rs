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
//! **Folded (the run-scope preamble):** `SimTick`; the whole `SimRngState`
//! (master seed, its provenance, and every `SimStream`'s exact `Pcg32`
//! position) through `digest_postcard`, so a divergent *draw count* is caught
//! the tick it happens; `WorldIdMint`'s tick and per-namespace counters (issue
//! #907), the identity analogue of the same thing, so a divergent *spawn
//! count* is caught the tick it happens rather than on the tick the next id is
//! minted; `GamePhase`; `GameOverReason` (both the reason string
//! and the `Outcome`); `CaptainPriorityBoost`'s every `(scope, objective)` pair
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
//! **Folded (weapons-hold namespace, in `FoldKey` order — issue #1041):** the
//! id of every ship currently under a captain's weapons hold, and nothing else —
//! the state is one boolean and the namespace carries only the ships for which
//! it is true. A host that disagreed about whether a hull had been ordered to
//! hold fire would disagree about whether its guns may open up at all. Empty-
//! namespace rule as above, and see [`fold_weapons_hold_namespace`] for why the
//! released ships are left out rather than folded as zeroes.
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
//! modifier caches and the per-system blackboards are likewise deferred —
//! **and the
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
use crate::console::command::server::ShipStationStances;
use crate::console::repair::external_server::ExternalRepairDispatch;
use crate::core::balance::BalanceEvent;
use crate::core::messages::GamePhase;
use crate::core::telemetry::RunTelemetry;
use crate::dock::DockControl;
use crate::entities::spawner::{EntitySystemHull, EntityUuid};
use crate::infrastructure::{InfrastructureCondition, InfrastructureState};
use crate::lobby::WorldResource;
use crate::server_app::{AsteroidUuid, CaptainPriorityBoost, GameOverReason};
use crate::ship::damage::SystemHull;
use crate::ship::state::{ShipPhysics, ShipRedAlert, ShipWeaponsHold};
use crate::sim_rng::SimRng;
use crate::sim_tick::SimTick;
use crate::tractor::TractorBeam;
use crate::umbilical::TransferUmbilical;

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
pub fn world_digest(world: &World) -> u64 {
    let mut acc = FOLD_SEED;
    acc = fold_run_scope(world, acc);
    acc = fold_entity_namespace(world, acc);
    acc = fold_infrastructure_namespace(world, acc);
    acc = fold_civilian_namespace(world, acc);
    acc = fold_weapons_hold_namespace(world, acc);
    acc = fold_station_stances_namespace(world, acc);
    acc = fold_tractor_namespace(world, acc);
    acc = fold_dock_namespace(world, acc);
    acc = fold_external_repair_namespace(world, acc);
    acc = fold_umbilical_namespace(world, acc);
    acc = fold_asteroid_namespace(world, acc);
    fold_collisions(world, acc)
}

/// The run-scope preamble: tick, RNG, phase, ending, captain boosts, world.
fn fold_run_scope(world: &World, mut acc: u64) -> u64 {
    acc = fold_u64(acc, world.get_resource::<SimTick>().map_or(0, |t| t.0));

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

/// Every ship currently under a **weapons hold** (issue #1041), in [`FoldKey`]
/// order, in its own namespace.
///
/// Authoritative and folded: two hosts that disagreed about whether a hull had
/// been ordered to hold fire would disagree about whether its guns are allowed
/// to open up at all, which is about as divergent as two hosts get.
///
/// # Only the ships that ARE holding
///
/// The empty-namespace rule of [`fold_infrastructure_namespace`], turned one
/// notch further: this walk folds nothing at all when no ship is holding, and
/// the rows it does fold are the held ships alone rather than a bit per ship.
/// The reason is the same and the argument is stronger here, because the state
/// is on EVERY ship rather than on a handful of authored structures. Folding a
/// released hold for every hull would have moved every committed world digest
/// the moment this slice landed, over a lever none of those runs pull — and the
/// acceptance criterion this slice is built to is precisely that Red Alert's
/// behaviour is unchanged while the hold is not engaged. A run in which nobody
/// holds fire and a run recorded before the lever existed *are the same
/// authoritative state*, so they fold to the same number.
///
/// It is not a hole. The moment one ship holds, its id is in the accumulator
/// and the count with it, so two hosts that disagree about whether ANY ship is
/// holding disagree about this namespace immediately.
fn fold_weapons_hold_namespace(world: &World, mut acc: u64) -> u64 {
    let Some(mut query) = world.try_query::<(Entity, &EntityUuid, &ShipWeaponsHold)>() else {
        // A world that never registered the component holds nothing — the empty
        // case above, not a distinct one.
        return acc;
    };
    let mut rows: Vec<(FoldKey, bevy::ecs::entity::EntityIndex)> = query
        .iter(world)
        .filter(|(_, _, hold)| hold.0)
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

    acc = fold_str(acc, "weapons-hold-namespace");
    acc = fold_u64(acc, rows.len() as u64);
    for (key, _) in rows {
        acc = fold_str(acc, &key.id);
    }
    acc
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
/// selection has to be caught on the tick it happens — the same rationale
/// [`fold_weapons_hold_namespace`] folds.
///
/// # Only the ships that carry a selection, and per-station in id order
///
/// The empty-namespace rule of [`fold_weapons_hold_namespace`]. EMPTY is the
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
    let mut rows: Vec<(FoldKey, bevy::ecs::entity::EntityIndex, String)> = query
        .iter(world)
        .filter_map(|(entity, uuid, dispatch)| {
            dispatch.dispatched_target.as_ref().map(|target| {
                (
                    FoldKey::from_world_id(Namespace::Entity, &uuid.0),
                    entity.index(),
                    target.clone(),
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
    for (key, _, target) in rows {
        acc = fold_str(acc, &key.id);
        acc = fold_str(acc, &target);
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
mod tests {
    use super::*;

    #[test]
    fn nan_flavours_fold_to_one_payload() {
        let quiet = f32::NAN;
        let other = f32::from_bits(0x7fc0_1234);
        assert!(other.is_nan(), "fixture must actually be a NaN");
        assert_eq!(canon_f32(quiet), canon_f32(other));
    }

    #[test]
    fn signed_zeroes_fold_alike_but_ordinary_values_do_not() {
        assert_eq!(canon_f32(0.0), canon_f32(-0.0));
        assert_ne!(canon_f32(1.0), canon_f32(-1.0));
    }

    /// No quantisation: the smallest representable difference must move the
    /// fold, or a digest reports the wrong tick a split happened on.
    #[test]
    fn one_ulp_moves_the_fold() {
        let a = 1.0f32;
        let b = f32::from_bits(a.to_bits() + 1);
        assert_ne!(fold_f32(FOLD_SEED, a), fold_f32(FOLD_SEED, b));
    }

    /// Namespaces group before ids, and the numeric pair orders within one —
    /// the whole of the fold-order policy, asserted rather than assumed.
    #[test]
    fn fold_keys_group_by_namespace_then_order_numerically() {
        let id = |tick, seq| crate::world_id::WorldId::new(Namespace::Entity, tick, seq).render();
        let (t2, t10, t10s2) = (id(2, 1), id(10, 1), id(10, 2));
        let mut keys = [
            FoldKey::from_world_id(Namespace::Asteroid, &t2),
            FoldKey::from_world_id(Namespace::Entity, &t10),
            FoldKey::from_world_id(Namespace::Entity, &t10s2),
            FoldKey::from_world_id(Namespace::Entity, &t2),
            FoldKey::from_world_id(Namespace::Asteroid, &t10),
        ];
        keys.sort();
        let rendered: Vec<_> = keys.iter().map(|k| (k.namespace, k.tick, k.seq)).collect();
        assert_eq!(
            rendered,
            vec![
                (Namespace::Entity, 2, 1),
                (Namespace::Entity, 10, 1),
                (Namespace::Entity, 10, 2),
                (Namespace::Asteroid, 2, 1),
                (Namespace::Asteroid, 10, 1),
            ],
            "namespaces group first, then the numeric pair orders within one"
        );
    }

    /// The failure the structured key exists to prevent, stated as its own
    /// assertion: an *unpadded* `tick-seq` rendering sorts 10 before 2, so a
    /// fold that sorted strings would reorder itself as the run got longer.
    /// `WorldId::render` is fixed-width hex, and the key compares numbers
    /// regardless of what the rendering does.
    #[test]
    fn the_naive_string_rendering_would_have_sorted_wrongly() {
        assert!("10-1" < "2-1");
        let a = crate::world_id::WorldId::new(Namespace::Entity, 2, 1);
        let b = crate::world_id::WorldId::new(Namespace::Entity, 10, 1);
        assert!(a < b, "the structured tuple orders numerically");
        assert!(a.render() < b.render(), "and so does the padded rendering");
    }

    /// A **v4** uuid must NOT be read as a mint. This is asteroids' live case,
    /// not a hypothetical: `deterministic_cell_uuid` ids stay v4-shaped by
    /// design (constraint 8), and since a mint is now uuid-shaped too, the
    /// version nibble is the entire difference between "fold this at tick 0 on
    /// its string" and "invent a tick and a sequence for a rock".
    #[test]
    fn a_v4_uuid_keys_as_zero_and_sorts_on_its_string() {
        let key = FoldKey::from_world_id(Namespace::Entity, "a1b2c3d4-0000-4000-8000-000000000001");
        assert_eq!((key.tick, key.seq), (0, 0));
        assert_eq!(key.id, "a1b2c3d4-0000-4000-8000-000000000001");
    }

    /// Register every component the fold walks, so `World::try_query` can see
    /// them even in a world where no entity happens to carry one.
    fn fold_world() -> World {
        let mut world = World::new();
        world.register_component::<EntityUuid>();
        world.register_component::<ShipPhysics>();
        world.register_component::<EntitySystemHull>();
        world.register_component::<ShipRedAlert>();
        world.register_component::<AsteroidUuid>();
        world.register_component::<Transform>();
        world.register_component::<InfrastructureCondition>();
        world.register_component::<CivilianTraffic>();
        world.register_component::<TractorBeam>();
        world.register_component::<TransferUmbilical>();
        world
    }

    fn spawn_ship(world: &mut World, uuid: &str, x: f32) {
        world.spawn((
            EntityUuid(uuid.to_string()),
            ShipPhysics {
                x,
                ..Default::default()
            },
            ShipRedAlert(false),
        ));
    }

    fn spawn_rock(world: &mut World, uuid: &str, x: f32) {
        world.spawn((
            AsteroidUuid(uuid.to_string()),
            Transform::from_xyz(x, 0.0, 0.0),
        ));
    }

    /// A structure with one threshold, at `condition` of 100 points.
    fn spawn_structure(world: &mut World, uuid: &str, condition: f32) {
        let config = crate::infrastructure::InfrastructureConfig {
            condition_max: 100.0,
            condition: Some(condition),
            thresholds: vec![crate::infrastructure::ThresholdConfig {
                label: None,
                flag: "transfer_capable".to_string(),
                capacity: None,
                fails_below: 0.4,
                restores_above: None,
            }],
            ..Default::default()
        };
        world.spawn((
            EntityUuid(uuid.to_string()),
            InfrastructureCondition(InfrastructureState::from_config(&config)),
        ));
    }

    /// A tug, optionally holding a named derelict on its tractor.
    fn spawn_tug(world: &mut World, uuid: &str, holding: Option<&str>) {
        let mut beam = TractorBeam::new(
            crate::tractor::TractorConfig {
                range: 600.0,
                coupling_offset: [0.0, 0.0, -120.0],
                min_power_level: 2,
                tow_load: crate::tractor::TowLoadCurve {
                    half_penalty_mass: 10_000.0,
                    max_penalty: 0.8,
                },
            },
            crate::core::messages::PowerGroupId("tractor".into()),
        );
        if let Some(target) = holding {
            beam.engaged = true;
            beam.coupled_target = Some(target.to_string());
        }
        world.spawn((EntityUuid(uuid.to_string()), beam));
    }

    /// **Issue #1156.** A world with no tractor holding folds to exactly the
    /// number it did before the namespace existed — and so does a hull that
    /// authored a tractor and is holding nothing. Only an ACTIVE grip moves it,
    /// and the coupled target is what moves it, because a held derelict carries
    /// no `ShipPhysics` and nothing else folds its position.
    #[test]
    fn only_a_held_tractor_moves_the_digest_and_the_target_is_what_moves_it() {
        let id = "00000000-0000-8000-8000-000000000001";

        let mut idle = fold_world();
        spawn_tug(&mut idle, id, None);
        assert_eq!(
            fold_tractor_namespace(&idle, FOLD_SEED),
            FOLD_SEED,
            "a tractor holding nothing folds nothing — a folded row here would have moved every \
             committed world's digest for state none of them carry"
        );

        let mut holding_a = fold_world();
        spawn_tug(&mut holding_a, id, Some("derelict-A"));
        let mut holding_a_again = fold_world();
        spawn_tug(&mut holding_a_again, id, Some("derelict-A"));
        let mut holding_b = fold_world();
        spawn_tug(&mut holding_b, id, Some("derelict-B"));

        assert_ne!(
            world_digest(&idle),
            world_digest(&holding_a),
            "engaging the tractor and taking a derelict under tow must move the digest — nothing \
             else records that the hulk is being dragged"
        );
        assert_eq!(
            world_digest(&holding_a),
            world_digest(&holding_a_again),
            "two hosts holding the same derelict must agree"
        );
        assert_ne!(
            world_digest(&holding_a),
            world_digest(&holding_b),
            "…and a host that thinks the beam has a different hulk must fold to a different number"
        );
    }

    /// A docker, optionally mated to a named berth.
    fn spawn_docker(world: &mut World, uuid: &str, docked_to: Option<&str>) {
        let mut control = DockControl::new(
            crate::ship::system_registry::dock_system_id(),
            crate::dock::DockConfig {
                range: 200.0,
                engage_distance: 400.0,
                approach_speed: 60.0,
                mate_tolerance: 4.0,
                undock_clear_distance: 120.0,
                min_power_level: 2,
            },
            crate::core::messages::PowerGroupId("dock".into()),
        );
        if let Some(target) = docked_to {
            control.engaged = true;
            control.docked = true;
            control.docking_target = Some(target.to_string());
        }
        world.spawn((EntityUuid(uuid.to_string()), control));
    }

    /// **Issue #1159.** A world with nobody docked folds to exactly the number it
    /// did before the namespace existed — and so does a hull that authored a dock
    /// and is not docked. Only a real MATE moves it, and which two hulls are
    /// joined is what moves it, because that relationship is folded nowhere else.
    #[test]
    fn only_a_docked_ship_moves_the_digest_and_the_partner_is_what_moves_it() {
        let id = "00000000-0000-8000-8000-000000000001";

        let mut idle = fold_world();
        spawn_docker(&mut idle, id, None);
        assert_eq!(
            fold_dock_namespace(&idle, FOLD_SEED),
            FOLD_SEED,
            "a dock mated to nothing folds nothing — a folded row here would have moved every \
             committed world's digest for state none of them carry"
        );

        let mut docked_a = fold_world();
        spawn_docker(&mut docked_a, id, Some("berth-A"));
        let mut docked_a_again = fold_world();
        spawn_docker(&mut docked_a_again, id, Some("berth-A"));
        let mut docked_b = fold_world();
        spawn_docker(&mut docked_b, id, Some("berth-B"));

        assert_ne!(
            world_digest(&idle),
            world_digest(&docked_a),
            "mating a dock must move the digest — nothing else records that the two hulls are joined"
        );
        assert_eq!(
            world_digest(&docked_a),
            world_digest(&docked_a_again),
            "two hosts docked to the same berth must agree"
        );
        assert_ne!(
            world_digest(&docked_a),
            world_digest(&docked_b),
            "…and a host that thinks the ship is docked to a different hull must fold to a \
             different number"
        );
    }

    /// An umbilical-carrying hull, optionally running.
    fn spawn_umbilical(world: &mut World, uuid: &str, running: bool) {
        let mut umbilical = TransferUmbilical::new(
            crate::umbilical::UmbilicalConfig {
                capacity: "reserve_fuel".into(),
                rate: 5.0,
                direction: crate::umbilical::UmbilicalDirection::Deliver,
                min_power_level: 2,
            },
            crate::core::messages::PowerGroupId("umbilical".into()),
        );
        umbilical.running = running;
        // The carry and the level projections are live state the fold must
        // ignore: two otherwise-identical running umbilicals mid-flow fold the
        // same, whatever their sub-tick carry.
        umbilical.carry = if running { 0.37 } else { 0.0 };
        umbilical.operator_level = Some(42);
        world.spawn((EntityUuid(uuid.to_string()), umbilical));
    }

    /// **Issue #1160.** A world with no umbilical running folds to exactly the
    /// number it did before the namespace existed — and so does a hull that
    /// authored an umbilical and is not running. Only a RUNNING flow moves it, and
    /// the carry/level projections never do, because they are re-derived every
    /// tick and a resume forgives the sub-tick debt.
    #[test]
    fn only_a_running_umbilical_moves_the_digest_and_the_carry_does_not() {
        let id = "00000000-0000-8000-8000-000000000001";

        let mut idle = fold_world();
        spawn_umbilical(&mut idle, id, false);
        assert_eq!(
            fold_umbilical_namespace(&idle, FOLD_SEED),
            FOLD_SEED,
            "an umbilical running nothing folds nothing — a folded row here would have moved \
             every committed world's digest for state none of them carry"
        );

        let mut running = fold_world();
        spawn_umbilical(&mut running, id, true);
        assert_ne!(
            world_digest(&idle),
            world_digest(&running),
            "starting the flow must move the digest — whether it runs is what decides the next \
             tick's capacity move, folded nowhere else"
        );

        // Two hosts mid-flow with different sub-tick carries still agree: the
        // carry is a projection the fold ignores.
        let mut running_again = fold_world();
        spawn_umbilical(&mut running_again, id, true);
        if let Some(mut u) = running_again
            .query::<&mut TransferUmbilical>()
            .iter_mut(&mut running_again)
            .next()
        {
            u.carry = 0.91;
            u.operator_level = Some(7);
        }
        assert_eq!(
            world_digest(&running),
            world_digest(&running_again),
            "two hosts running the same flow with different carries must fold the same"
        );
    }

    /// **Issue #1025.** A world with no infrastructure must fold to exactly the
    /// number it folded before the namespace existed.
    ///
    /// This is what keeps every committed world digest where it is. The
    /// namespace is a pure addition for worlds that have structures and a
    /// literal no-op for worlds that do not, and the two claims are the same
    /// claim: an empty walk and an absent feature are the same world state.
    #[test]
    fn the_infrastructure_namespace_is_a_no_op_for_a_world_that_has_none() {
        let mut world = fold_world();
        spawn_ship(&mut world, "00000000-0000-8000-8000-000000000001", 1.0);
        spawn_rock(&mut world, "a1b2c3d4-0000-4000-8000-000000000001", 2.0);
        assert_eq!(
            fold_infrastructure_namespace(&world, FOLD_SEED),
            FOLD_SEED,
            "an empty infrastructure walk must leave the accumulator untouched — a folded row \
             count here would have moved every world digest in the repository for state none \
             of those worlds carry"
        );
    }

    /// **Issue #1025.** Two structures that differ only in condition must fold
    /// to different numbers, and two that agree must not.
    #[test]
    fn a_structures_condition_and_flags_move_the_digest() {
        let mut intact = fold_world();
        spawn_structure(&mut intact, "00000000-0000-8000-8000-000000000001", 100.0);
        let mut same = fold_world();
        spawn_structure(&mut same, "00000000-0000-8000-8000-000000000001", 100.0);
        let mut degraded = fold_world();
        spawn_structure(&mut degraded, "00000000-0000-8000-8000-000000000001", 10.0);

        assert_eq!(
            world_digest(&intact),
            world_digest(&same),
            "two hosts holding the same structure in the same condition must agree"
        );
        assert_ne!(
            world_digest(&intact),
            world_digest(&degraded),
            "…and a structure degraded past its threshold — a different condition AND a \
             different operational flag — must not fold to the number an intact one does"
        );
    }

    /// **Issue #1027.** A depot's capacity LEVEL is folded, so two hosts that
    /// disagree about how much a transfer moved disagree about the digest.
    #[test]
    fn a_moved_capacity_moves_the_digest() {
        fn world_with(level: i64) -> World {
            let mut world = fold_world();
            let config = crate::infrastructure::InfrastructureConfig {
                capacities: vec![crate::infrastructure::CapacityConfig {
                    label: None,
                    id: "berths".to_string(),
                    amount: level,
                    ceiling: Some(40),
                }],
                ..Default::default()
            };
            world.spawn((
                EntityUuid("00000000-0000-8000-8000-000000000001".to_string()),
                InfrastructureCondition(InfrastructureState::from_config(&config)),
            ));
            world
        }
        assert_eq!(
            world_digest(&world_with(20)),
            world_digest(&world_with(20)),
            "two hosts holding the same depot at the same level must agree"
        );
        assert_ne!(
            world_digest(&world_with(20)),
            world_digest(&world_with(32)),
            "…and a host that thinks twelve more berths are free disagrees about whether the              transfer window can be met, which is the mission. Before #1027 a capacity could not              move and folding it would have been noise; now it can, and not folding it would be              a hole."
        );
    }

    /// **Issue #1025 / AC4.** Structure order must not reach the digest.
    #[test]
    fn structures_fold_in_uuid_order_whatever_order_they_spawned_in() {
        let mut forward = fold_world();
        spawn_structure(&mut forward, "00000000-0000-8000-8000-000000000001", 90.0);
        spawn_structure(&mut forward, "00000000-0000-8000-8000-000000000002", 20.0);
        let mut reverse = fold_world();
        spawn_structure(&mut reverse, "00000000-0000-8000-8000-000000000002", 20.0);
        spawn_structure(&mut reverse, "00000000-0000-8000-8000-000000000001", 90.0);
        assert_eq!(
            world_digest(&forward),
            world_digest(&reverse),
            "the fold is keyed on the minted id, not on archetype order"
        );
    }

    /// A civilian on `lane` at `leg`, optionally under an order.
    fn spawn_civilian(
        world: &mut World,
        uuid: &str,
        leg: usize,
        order: Option<crate::civilian::CivilianOrder>,
    ) {
        let mut state = CivilianState::from_config(&crate::civilian::CivilianConfig {
            route: Some("depot_run".into()),
            ..Default::default()
        });
        state.observe_leg(leg, None, 0, 60.0);
        if let Some(order) = order {
            state.receive_order(
                order,
                &crate::civilian::ComplianceDisposition::default(),
                0,
                60.0,
            );
        }
        world.spawn((EntityUuid(uuid.to_string()), CivilianTraffic(state)));
    }

    /// **Issue #1028.** A world with no civilian traffic must fold to exactly
    /// the number it folded before the namespace existed.
    ///
    /// The same claim the infrastructure namespace makes, for the same reason:
    /// an empty walk and an absent feature are the same world state, and a
    /// folded row count here would have moved every committed world digest over
    /// state none of those worlds carry.
    #[test]
    fn the_civilian_namespace_is_a_no_op_for_a_world_that_has_none() {
        let mut world = fold_world();
        spawn_ship(&mut world, "00000000-0000-8000-8000-000000000001", 1.0);
        spawn_rock(&mut world, "a1b2c3d4-0000-4000-8000-000000000001", 2.0);
        assert_eq!(
            fold_civilian_namespace(&world, FOLD_SEED),
            FOLD_SEED,
            "an empty civilian walk must leave the accumulator untouched"
        );
    }

    /// **Issue #1028.** A craft's lane position and its answer to an order both
    /// move the digest; two hosts that agree about both must agree.
    #[test]
    fn a_civilians_leg_and_its_compliance_move_the_digest() {
        const ID: &str = "00000000-0000-8000-8000-000000000001";
        let mut on_leg_one = fold_world();
        spawn_civilian(&mut on_leg_one, ID, 1, None);
        let mut same = fold_world();
        spawn_civilian(&mut same, ID, 1, None);
        let mut on_leg_two = fold_world();
        spawn_civilian(&mut on_leg_two, ID, 2, None);
        let mut ordered = fold_world();
        spawn_civilian(
            &mut ordered,
            ID,
            1,
            Some(crate::civilian::CivilianOrder::Hold),
        );
        let mut ordered_elsewhere = fold_world();
        spawn_civilian(
            &mut ordered_elsewhere,
            ID,
            1,
            Some(crate::civilian::CivilianOrder::divert_to_anchor(
                "holding_point",
            )),
        );

        assert_eq!(
            world_digest(&on_leg_one),
            world_digest(&same),
            "two hosts holding the same craft on the same leg must agree"
        );
        assert_ne!(
            world_digest(&on_leg_one),
            world_digest(&on_leg_two),
            "…and a craft one leg further round its circuit must not fold to the number the \
             one behind it does"
        );
        assert_ne!(
            world_digest(&on_leg_one),
            world_digest(&ordered),
            "an order taken is authoritative state: a craft that has been told to hold is not \
             the same craft as one that has not"
        );
        assert_ne!(
            world_digest(&ordered),
            world_digest(&ordered_elsewhere),
            "…and neither is one sent somewhere else — the verb AND its destination are what \
             distinguish two orders"
        );
    }

    /// **Issue #1028.** Craft order must not reach the digest.
    #[test]
    fn civilians_fold_in_uuid_order_whatever_order_they_spawned_in() {
        let mut forward = fold_world();
        spawn_civilian(
            &mut forward,
            "00000000-0000-8000-8000-000000000001",
            0,
            None,
        );
        spawn_civilian(
            &mut forward,
            "00000000-0000-8000-8000-000000000002",
            2,
            None,
        );
        let mut reverse = fold_world();
        spawn_civilian(
            &mut reverse,
            "00000000-0000-8000-8000-000000000002",
            2,
            None,
        );
        spawn_civilian(
            &mut reverse,
            "00000000-0000-8000-8000-000000000001",
            0,
            None,
        );
        assert_eq!(
            world_digest(&forward),
            world_digest(&reverse),
            "the fold is keyed on the minted id, not on archetype order"
        );
    }

    /// **AC4.** The same entities spawned in two different orders must produce
    /// the same digest.
    ///
    /// Bevy query iteration is archetype order — stable within one process, not
    /// across two instances that spawned entities in a different sequence. This
    /// is the test that would fail if the fold ever walked a query straight
    /// into the accumulator, and it is deliberately a *spawn-order* difference
    /// rather than a component-value one: everything about the two worlds is
    /// identical except the order the ECS happens to hold them in.
    #[test]
    fn the_fold_iterates_in_stable_world_id_order() {
        let mut forward = fold_world();
        spawn_ship(&mut forward, "ship-b", 2.0);
        spawn_ship(&mut forward, "ship-a", 1.0);
        spawn_rock(&mut forward, "rock-b", 20.0);
        spawn_rock(&mut forward, "rock-a", 10.0);

        let mut backward = fold_world();
        spawn_rock(&mut backward, "rock-a", 10.0);
        spawn_ship(&mut backward, "ship-a", 1.0);
        spawn_rock(&mut backward, "rock-b", 20.0);
        spawn_ship(&mut backward, "ship-b", 2.0);

        assert_eq!(
            world_digest(&forward),
            world_digest(&backward),
            "two worlds holding the same entities in a different spawn order \
             produced different digests — the fold is following ECS order \
             somewhere instead of world-id order"
        );
    }

    /// The other half of the same claim: the digest must still be *sensitive*.
    /// A fold that returned a constant would pass the test above trivially.
    #[test]
    fn a_moved_ship_moves_the_digest() {
        let mut before = fold_world();
        spawn_ship(&mut before, "ship-a", 1.0);
        let mut after = fold_world();
        spawn_ship(&mut after, "ship-a", 1.000_000_1);
        assert_ne!(world_digest(&before), world_digest(&after));
    }

    /// Namespaces are folded in a declared sequence and never merged, so an id
    /// that exists in both namespaces must not collapse into one fold position.
    #[test]
    fn the_same_id_in_two_namespaces_folds_twice() {
        let mut both = fold_world();
        spawn_ship(&mut both, "shared-id", 1.0);
        spawn_rock(&mut both, "shared-id", 1.0);

        let mut one = fold_world();
        spawn_ship(&mut one, "shared-id", 1.0);

        assert_ne!(world_digest(&both), world_digest(&one));
    }

    #[test]
    fn a_zero_interval_never_samples() {
        let ledger = DigestLedger::new(0);
        assert!(!ledger.samples(0));
        assert!(!ledger.samples(120));
    }

    #[test]
    fn the_same_tick_is_never_sampled_twice() {
        let mut ledger = DigestLedger::new(10);
        ledger.record(10, 1);
        ledger.record(10, 1);
        assert_eq!(ledger.checkpoints.len(), 1);
    }

    #[test]
    fn a_divergence_names_the_window_it_happened_in() {
        let mut recorded = DigestLedger::new(10);
        let mut replayed = DigestLedger::new(10);
        for (tick, digest) in [(10, 1), (20, 2), (30, 3)] {
            recorded.record(tick, digest);
            replayed.record(tick, if tick == 30 { 99 } else { digest });
        }
        let found = recorded.first_divergence(&replayed).expect("diverged");
        assert_eq!(found.tick, 30);
        assert_eq!(found.after, Some(20));
        assert!(
            !found.at_end,
            "a sampled-tick mismatch is not the end-of-run shape"
        );
    }

    /// A bare end-state mismatch with every sampled tick agreeing is a
    /// DIFFERENT claim from a sampled tick disagreeing, and must be reported
    /// as one: `at_end` is true, and the rendered message says "agreed through
    /// N; the final states differ" rather than "first disagree at tick N" —
    /// the latter would name a tick that never actually disagreed, since every
    /// checkpoint this ledger sampled matched.
    #[test]
    fn agreeing_ledgers_with_different_endings_still_locate_the_split() {
        let mut recorded = DigestLedger::new(10);
        let mut replayed = DigestLedger::new(10);
        recorded.record(10, 1);
        replayed.record(10, 1);
        recorded.final_digest = 7;
        replayed.final_digest = 8;
        let found = recorded.first_divergence(&replayed).expect("diverged");
        assert_eq!(found.after, Some(10));
        assert!(
            found.at_end,
            "every sampled checkpoint agreed; only the final digest differs"
        );
        let rendered = found.to_string();
        assert!(
            rendered.contains("every sampled tick agreed through 10"),
            "got {rendered:?}"
        );
        assert!(
            rendered.contains("the final states differ"),
            "got {rendered:?}"
        );
        assert!(
            !rendered.contains("first disagree at tick"),
            "the end-of-run shape must not claim a specific tick disagreed \
             when every sample it took actually agreed; got {rendered:?}"
        );
    }

    #[test]
    fn identical_ledgers_do_not_diverge() {
        let mut ledger = DigestLedger::new(10);
        ledger.record(10, 1);
        ledger.final_digest = 5;
        assert_eq!(ledger.first_divergence(&ledger.clone()), None);
    }
}
