//! The shared structured debug-payload schema (PRD #1144).
//!
//! # This module IS the schema
//!
//! Every debug surface in the observability pipeline — the station-activity
//! chart in this slice, and the scenario-state / AI-state surfaces the later
//! slices (#1147–#1152) add — is a plain `serde` struct produced by a read-only
//! `collect` projection off authoritative state and carried to a consumer as
//! JSON. There are no pre-formatted strings on this pipeline: a text overlay is
//! the *legacy* `debug_overlay` shape this pipeline replaces.
//!
//! ## Conventions the later slices copy
//!
//! 1. **One module, one schema.** Every surface's payload struct lives here in
//!    `crate::debug::payload`, not scattered next to the projector that fills it.
//!    A consumer (the dock, the headless report, the GM console M6 Live
//!    Inspector) imports one module to know the whole wire vocabulary.
//! 2. **Every top-level payload carries [`DEBUG_SCHEMA_VERSION`].** A consumer
//!    reads the version first and refuses or adapts to a mismatch rather than
//!    silently misreading a payload from a newer host. Additive, serde-tolerant
//!    field growth does not bump it; a rename or a removed/retyped field does.
//! 3. **`serde` derives, never hand-rolled JSON.** The one place the structs
//!    become JSON is [`crate::core::codec`] (AGENTS.md Key Constraint 1 confines
//!    `serde_json` there). Each surface adds one `encode_*` there; nothing else
//!    imports `serde_json`.
//! 4. **Deterministic ordering.** Collections are emitted in a stable order
//!    (sorted keys, oldest→newest time series) so two hosts folding the same
//!    state produce byte-identical JSON — the property the GM inspector and any
//!    diff tooling lean on.
//!
//! Nothing in here is authoritative simulation state: these are read-only
//! projections, so producing one can never move the #894 digest (see
//! `crate::debug::station_activity` for the determinism guard).

use serde::{Deserialize, Serialize};

/// The wire-schema version every top-level debug payload stamps.
///
/// See convention 2 in the module docs. Bump on a breaking shape change to any
/// payload in this module; a consumer compares it against the version it was
/// built for.
pub const DEBUG_SCHEMA_VERSION: u32 = 1;

/// Which control source issued the admitted commands counted in a bucket.
///
/// The wire form of [`crate::ship::control_source::ControlSource`], kept as a
/// separate type so the payload schema does not depend on the internals of the
/// ship control model and so the JSON field names are stable regardless of how
/// the simulation spells the source. `Offline` is present for completeness — a
/// command admitted against an offline system cannot happen (admission refuses
/// it), so the count stays zero, but the slot keeps the schema uniform across
/// all three sources.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActivitySource {
    Human,
    Ai,
    Offline,
}

impl From<crate::ship::control_source::ControlSource> for ActivitySource {
    fn from(source: crate::ship::control_source::ControlSource) -> Self {
        use crate::ship::control_source::ControlSource;
        match source {
            ControlSource::Human => ActivitySource::Human,
            ControlSource::Ai => ActivitySource::Ai,
            ControlSource::Offline => ActivitySource::Offline,
        }
    }
}

/// One station's admitted-command tally for one time bucket, split by control
/// source.
///
/// A "few integers per station", per the PRD: the always-on counters cost this
/// much and no more.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StationActivityEntry {
    /// The `StationId` string this tally is for.
    pub station: String,
    /// Commands admitted while the station was human-crewed.
    pub human: u32,
    /// Commands admitted while the station was AI-crewed (Backfill or NPC).
    pub ai: u32,
    /// Commands admitted while the station was offline. Always zero in practice
    /// (admission refuses an offline system); kept for schema uniformity.
    pub offline: u32,
}

impl StationActivityEntry {
    /// Add one admitted command from `source` to this entry.
    pub fn record(&mut self, source: ActivitySource) {
        match source {
            ActivitySource::Human => self.human = self.human.saturating_add(1),
            ActivitySource::Ai => self.ai = self.ai.saturating_add(1),
            ActivitySource::Offline => self.offline = self.offline.saturating_add(1),
        }
    }

    /// Total commands across all sources in this entry.
    pub fn total(&self) -> u32 {
        self.human
            .saturating_add(self.ai)
            .saturating_add(self.offline)
    }
}

/// One time bucket: every station's tally over one `bucket_ticks`-long window.
///
/// `stations` is sorted by station id so the JSON is deterministic.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StationActivityBucket {
    /// The `SimTick` at which this bucket opened (`bucket_index * bucket_ticks`).
    pub start_tick: u64,
    /// Per-station tallies, sorted by [`StationActivityEntry::station`].
    pub stations: Vec<StationActivityEntry>,
}

/// The station-activity surface's whole payload: a bounded time series of
/// per-station, per-source admitted-command counts.
///
/// Produced by `crate::debug::station_activity::StationActivityTracker::report`,
/// encoded to JSON by `crate::core::codec::encode_station_activity`, and read by
/// the dock chart (`gui/station-activity-chart.js`).
///
/// Not `Eq`: `bucket_secs` is an `f32`. `PartialEq` is enough for the tests that
/// compare payloads.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StationActivityPayload {
    /// [`DEBUG_SCHEMA_VERSION`] at the time the host produced this payload.
    pub schema_version: u32,
    /// The bucket length in `SimTick`s the counters actually used.
    pub bucket_ticks: u64,
    /// The authored bucket length in simulation seconds (for the chart's axis
    /// label). Derived from `[global] station_activity_bucket_secs`.
    pub bucket_secs: f32,
    /// Buckets oldest→newest, ending with the in-progress bucket.
    pub buckets: Vec<StationActivityBucket>,
}

impl Default for StationActivityPayload {
    fn default() -> Self {
        Self {
            schema_version: DEBUG_SCHEMA_VERSION,
            bucket_ticks: 0,
            bucket_secs: 0.0,
            buckets: Vec::new(),
        }
    }
}

// ── AI-state surface: the doctrine pool per ship (issue #1149) ───────────────
//
// The AI observability surface. The scored-objective doctrine pool crosses the
// wire on every ship's viewscreen blackboard each tick and is rendered nowhere
// today; this makes it diagnostic, so a tuner can see *why* the AI picked what
// it picked. The projector that fills these lives in `crate::debug::ai_state`;
// the field VALUES (directive label, resolved target, chosen top directive)
// reuse the `crate::ai::decision_trace` helpers #1146 built, so the trace a log
// line carries and the surface a dock renders never drift.

/// The AI-state debug surface's whole payload (issue #1149, PRD #1144).
///
/// Two independent sub-surfaces of the one AI payload (the PRD's "doctrine pool
/// plus per-host fine-system policy view"):
///
/// * `ships` — the per-ship scored-objective doctrine pool (issue #1149).
/// * `hosts` — the per-host fine-system policy-machine view (issue #1152): for
///   each stateful fine-system AI host, its current state, private memory, the
///   last transition it committed and the guard blocking the transition it did
///   not take. Added AFTER `ships`, so growing the struct never reshapes the
///   doctrine part or any type below it — additive, serde-tolerant growth that
///   does not bump [`DEBUG_SCHEMA_VERSION`] (payload convention 2).
///
/// Not `Eq`: the candidate scores and memory readings are floats. `PartialEq`
/// is enough for the tests that compare payloads.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AiStatePayload {
    /// [`DEBUG_SCHEMA_VERSION`] at the time the host produced this payload.
    pub schema_version: u32,
    /// The `SimTick` at which this snapshot was projected.
    pub tick: u64,
    /// One entry per AI-controlled ship, sorted by `(ship, uuid)` so two hosts
    /// folding the same world produce byte-identical JSON.
    pub ships: Vec<ShipDoctrine>,
    /// One entry per stateful fine-system AI host, sorted by
    /// `(ship, uuid, host)` for the same byte-identical wire order (issue
    /// #1152). Empty when no AI ship runs a stateful policy machine.
    #[serde(default)]
    pub hosts: Vec<HostPolicyView>,
}

impl Default for AiStatePayload {
    fn default() -> Self {
        Self {
            schema_version: DEBUG_SCHEMA_VERSION,
            tick: 0,
            ships: Vec::new(),
            hosts: Vec::new(),
        }
    }
}

// ── Damage-log surface (issue #1150) ─────────────────────────────────────────
//
// The structured form of the legacy `debug_overlay::DamageLog::format` text
// stream. The always-on ring buffer (`debug_overlay::DamageLog`) is the data
// source — the analogue of `AdmittedCommands` for the station-activity surface
// — and this payload is a read-only projection of it, newest event first.

/// One recorded damage event.
///
/// The wire form of a `debug_overlay::DamageLogEntry`. `shield_arc` is `None`
/// when shields were bypassed or absent (the legacy text rendered that as an
/// em-dash); the dock decides how to show the absence.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DamageEntry {
    /// Human-readable damage source (asteroid uuid, region uuid, weapon name).
    pub source: String,
    /// Shield arc label hit, or `None` when shields were bypassed / absent.
    pub shield_arc: Option<String>,
    /// Total damage amount before shield absorption (hull + shield combined).
    pub amount: f32,
}

/// The damage surface's whole payload: the recent damage events, newest first.
///
/// Not `Eq`: `DamageEntry::amount` is an `f32`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DamageDebugPayload {
    /// [`DEBUG_SCHEMA_VERSION`] at the time the host produced this payload.
    pub schema_version: u32,
    /// Recent damage events, newest first (the ring buffer's own order).
    pub entries: Vec<DamageEntry>,
}

impl Default for DamageDebugPayload {
    fn default() -> Self {
        Self {
            schema_version: DEBUG_SCHEMA_VERSION,
            entries: Vec::new(),
        }
    }
}

// ── Scenario-state surface (issue #1148) ────────────────────────────────────
//
// A read-only projection of the running scenario's working state — the flags
// table, the objective table, the pending triggers with their eligibility, the
// delayed-action and deadline queues, the commitments board and the comms
// dossier — off `crate::world::server::WorldContentRuntime` and the objective
// manager. It answers "why didn't the story beat fire?" without reading a
// snapshot save or a digest hash. Rhai runtime internals (handler
// registrations, op budgets) are deliberately NOT here (PRD #1144 out-of-scope).
//
// The two enum-shaped fields that ARE the acceptance criteria's "status" and
// "directive" reuse the canonical wire vocabulary in [`crate::core::messages`]
// ([`ObjectiveStatus`], [`AiDirective`]) rather than re-spelling it: those are
// already the objective summary's wire types, so a consumer that reads the
// captain panel and one that reads this surface see the same shapes. The
// domain-local state words (a deadline's / commitment's state, a finding's
// provenance) travel as their documented `as_str()` labels so this schema does
// not depend on the `crate::world` module internals that own them.

pub use crate::core::messages::{AiDirective, ObjectiveStatus};

/// One entry in the world flag / counter store.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScenarioFlag {
    /// The flag name.
    pub name: String,
    /// Its counter value. A boolean flag reads as `1`; an unset flag never
    /// appears (the store drops zeroes), so every entry here is a set flag.
    pub value: i64,
}

/// One mission objective: id, status, its authored resting priority, and the
/// AI directive attached to it.
///
/// `base_priority` is the acceptance criteria's "score" — the objective's
/// authored importance to the mission before any per-tick utility conditions
/// apply. The LIVE per-ship scored-objective pool (utility conditions folded in)
/// is the AI-state surface's job, not this one, so this stays a pure projection
/// off the objective manager with no `WorldConditions` to evaluate against.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScenarioObjective {
    /// Stable objective id.
    pub id: String,
    /// Active / Completed / Failed.
    pub status: ObjectiveStatus,
    /// Whether the mission requires this objective.
    pub mandatory: bool,
    /// The authored base priority (before the mandatory bonus and any condition
    /// modifiers) — the "score" the objective table shows.
    pub base_priority: f32,
    /// The mission-altitude AI directive this objective carries.
    pub directive: AiDirective,
}

/// One atom of a trigger's `condition` / `when`, paired with the value the
/// fire recorder observed for it at fire time (issue #1151).
///
/// The [`atom`](Self::atom) string quotes the SAME vocabulary
/// [`ScenarioTrigger::condition`] / [`ScenarioTrigger::when`] render in — a
/// `flag(ready)` in the `when` string reads back as a `flag(ready)` atom here —
/// so an author reads a fire against the predicate they wrote. The
/// [`value`](Self::value) is that atom's observed reading rendered to a string:
/// `true` / `false` for a `flag(...)`, the counter reading (e.g. `5`) for a
/// `counter(...)`, or `n/a` for an AI-policy `fact` / `history` atom that a world
/// trigger's gate never uses and that carries no flag-store reading here.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PredicateValue {
    /// The atom, rendered in the #1148 `condition` / `when` string vocabulary.
    pub atom: String,
    /// Its observed value at fire time, rendered as a string.
    pub value: String,
}

/// One recorded trigger fire: when it fired, and the predicate atom values the
/// recorder observed at that moment (issue #1151).
///
/// The bounded per-trigger fire ring pushes one of these each time a trigger
/// fires, so an author can reconstruct why a beat fired early, late, or not at
/// all: [`fired_secs`](Self::fired_secs) answers the timing question the
/// `condition` poses (fired at 32.5s when the timer was authored for 30s), and
/// [`predicate_values`](Self::predicate_values) answers why the `when` gate held
/// (and which flag-referencing condition atom held) at that fire.
///
/// Not `Eq`: `fired_secs` is an `f32`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TriggerFire {
    /// World-elapsed seconds at which the trigger fired — its
    /// `last_fired_elapsed` at the recorded fire.
    pub fired_secs: f32,
    /// The flag-store atom values observed for this fire, drawn from the
    /// trigger's flag-referencing `condition` atom (if any) first, then its
    /// `when` gate's atoms in left-to-right tree order. Empty for a gateless
    /// event/entity trigger, whose whole story is `fired_secs`.
    pub predicate_values: Vec<PredicateValue>,
}

/// One trigger's pending state and current eligibility.
///
/// "Eligibility" is two independent facts the author needs to tell a waiting
/// beat from a dead one: whether the trigger is still ARMED to fire
/// ([`pending`](Self::pending)), and whether its `when` predicate currently
/// HOLDS ([`when_holds`](Self::when_holds)). A beat that never fires despite its
/// event arriving is usually a `when` that never became true.
///
/// # Fire history (#1151)
///
/// [`fire_history`](Self::fire_history) is the bounded record of this trigger's
/// recent fires with the predicate values observed at each. It is additive on
/// #1148's schema — a new `#[serde(default)]` field that neither reshapes the
/// payload nor bumps [`DEBUG_SCHEMA_VERSION`], so an older consumer, or a payload
/// from a host that predates #1151, still deserialises. The projector's rendered
/// `condition` / `when` strings are the vocabulary those per-fire records quote
/// their predicate values against.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScenarioTrigger {
    /// The authored trigger id, or `None` for an anonymous trigger.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// The firing condition, rendered to a stable compact string
    /// (e.g. `on_timer(after_secs=30)`, `on_flag_set(alarm)`).
    pub condition: String,
    /// The `when` predicate gate, rendered to a stable string, or `None` when
    /// the trigger has no gate (always eligible).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<String>,
    /// Whether the trigger re-arms after firing (`repeat = true`).
    pub repeat: bool,
    /// Whether the trigger has already fired at least once.
    pub fired: bool,
    /// Whether the trigger is still armed to fire: for a once-only trigger,
    /// `!fired`; for a `repeat` trigger, always `true`.
    pub pending: bool,
    /// Whether the `when` predicate evaluates true against the current flags
    /// (always `true` when there is no `when` gate). A pending trigger with
    /// `when_holds == false` is armed but waiting on its gate.
    pub when_holds: bool,
    /// World-elapsed seconds at which the trigger last fired, or `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_fired_secs: Option<f32>,
    /// This trigger's recent fires with the predicate values observed at each
    /// (issue #1151), oldest first, bounded per trigger. Empty when the trigger
    /// has not fired since capture began. Always emitted (not skipped) so a
    /// consumer always finds the field, even for a never-fired trigger; additive
    /// and `#[serde(default)]` so a pre-#1151 payload without it still decodes.
    #[serde(default)]
    pub fire_history: Vec<TriggerFire>,
}

/// One action queued for deferred dispatch off a trigger's `action_delays`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScenarioDelayedAction {
    /// The action kind, rendered to a stable compact string
    /// (e.g. `set_world_flag(alarm)`, `add_objective(rescue)`).
    pub action: String,
    /// The entity name the action was raised for, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entity: Option<String>,
    /// World-elapsed seconds at which the action is due to dispatch.
    pub fire_at_secs: f32,
}

/// One mission deadline's live state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScenarioDeadline {
    /// The authored deadline id.
    pub id: String,
    /// The crew-facing `strings.csv` label id (empty when the deadline authored
    /// none). Only meaningful when `visible`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
    /// Whether the crew sees a countdown for this deadline.
    pub visible: bool,
    /// The absolute `SimTick` the deadline is due.
    pub due_tick: u64,
    /// `pending` / `fired` / `cancelled` — the deadline table's own state label.
    pub state: String,
}

/// One promise on the commitments board.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScenarioCommitment {
    /// The promise id.
    pub id: String,
    /// The party it was made to (an entity or faction name, unresolved).
    pub made_to: String,
    /// `strings.csv` id for the terms.
    pub terms: String,
    /// `strings.csv` id for what would count as keeping it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub resolves_when: String,
    /// `open` / `kept` / `broken` — the ledger's own state label.
    pub state: String,
    /// The `SimTick` the promise was made on.
    pub made_at_tick: u64,
    /// The `SimTick` it was resolved on, or `None` while still open.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_at_tick: Option<u64>,
}

/// One finding on the comms dossier.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScenarioDossierEntry {
    /// The subject's UUID.
    pub subject_uuid: String,
    /// `strings.csv` id for what was learned.
    pub text: String,
    /// `scan` / `dialogue` / `records` / `briefing` — how the crew learned it.
    pub provenance: String,
    /// The `SimTick` it was learned on.
    pub gathered_at_tick: u64,
}

/// The scenario-state surface's whole payload (issue #1148).
///
/// Produced by `crate::debug::scenario::collect_scenario_state`, encoded to JSON
/// by `crate::core::codec::encode_scenario_state`, and read by the dock panel
/// (`gui/scenario-state-panel.js`) and the headless report. Every collection is
/// emitted in a deterministic order (flags sorted by name; every other list in
/// its authored / insertion order, which is already deterministic) so two hosts
/// folding the same state produce byte-identical JSON.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScenarioStatePayload {
    /// [`DEBUG_SCHEMA_VERSION`] at the time the host produced this payload.
    pub schema_version: u32,
    /// The world flag / counter store, sorted by flag name.
    pub flags: Vec<ScenarioFlag>,
    /// The mission objectives, mandatory first then optional (the objective
    /// manager's own sorted order).
    pub objectives: Vec<ScenarioObjective>,
    /// Every authored trigger with its pending state and eligibility.
    pub triggers: Vec<ScenarioTrigger>,
    /// The delayed-action queue, in dispatch order.
    pub delayed_actions: Vec<ScenarioDelayedAction>,
    /// The mission deadline queue, in authored order.
    pub deadlines: Vec<ScenarioDeadline>,
    /// The commitments board, oldest promise first.
    pub commitments: Vec<ScenarioCommitment>,
    /// The comms dossier, oldest finding first.
    pub dossier: Vec<ScenarioDossierEntry>,
}

impl Default for ScenarioStatePayload {
    /// A version-stamped empty payload — what a bare fixture with no world
    /// loaded projects, and the base the collector fills. Stamps the schema
    /// version like [`StationActivityPayload`] so a `..Default::default()` can
    /// never leak a version-0 payload onto the wire.
    fn default() -> Self {
        Self {
            schema_version: DEBUG_SCHEMA_VERSION,
            flags: Vec::new(),
            objectives: Vec::new(),
            triggers: Vec::new(),
            delayed_actions: Vec::new(),
            deadlines: Vec::new(),
            commitments: Vec::new(),
            dossier: Vec::new(),
        }
    }
}

// ── Modifier surface (issue #1150) ───────────────────────────────────────────
//
// The structured form of the legacy `ShipModifiers::format_debug` text stream,
// for the LocalShip. Three labelled sections — flags, float modifiers, integer
// modifiers — each an entry list sorted by name so the JSON is deterministic
// (convention 4). Built by `crate::modifiers::ShipModifiers::debug_payload`,
// which owns the private-field access the projection needs.

/// One active boolean modifier flag and the sources that set it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModifierFlagEntry {
    /// The `FlagKind` name (its `Debug` spelling).
    pub flag: String,
    /// The sources holding this flag active, sorted.
    pub sources: Vec<String>,
}

/// One source's additive bonus to a float modifier slot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FloatContribution {
    /// The rendered `ModifierSource` (e.g. `PowerGroup(helm)`, `Region(1a2b3c4d)`).
    pub source: String,
    /// The additive bonus this source contributes (positive buff, negative debuff).
    pub bonus: f32,
}

/// One float modifier slot: its computed multiplier and per-source breakdown.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FloatModifierEntry {
    /// The `ModifierSlot` name (its `Debug` spelling).
    pub slot: String,
    /// The cached multiplier the simulation applies for this slot.
    pub multiplier: f32,
    /// Per-source additive contributions, sorted by rendered source.
    pub contributions: Vec<FloatContribution>,
}

/// One source's additive bonus to an integer modifier slot.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntContribution {
    /// The rendered `ModifierSource`.
    pub source: String,
    /// The additive integer bonus this source contributes.
    pub bonus: i32,
}

/// One integer modifier slot: its summed total and per-source breakdown.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntModifierEntry {
    /// The `IntModifierSlot` name (its `Debug` spelling).
    pub slot: String,
    /// The summed total across all active sources.
    pub sum: i32,
    /// Per-source additive contributions, sorted by rendered source.
    pub contributions: Vec<IntContribution>,
}

/// The modifier surface's whole payload for the LocalShip.
///
/// Every section is empty when the LocalShip has no modifiers, or when there is
/// no LocalShip at all (a headless run) — the payload is always produced so the
/// dock and the determinism guard have something to read.
///
/// Not `Eq`: `FloatModifierEntry::multiplier` is an `f32`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModifierDebugPayload {
    /// [`DEBUG_SCHEMA_VERSION`] at the time the host produced this payload.
    pub schema_version: u32,
    /// Active boolean flags, sorted by flag name.
    pub flags: Vec<ModifierFlagEntry>,
    /// Active float modifier slots, sorted by slot name.
    pub float_modifiers: Vec<FloatModifierEntry>,
    /// Active integer modifier slots, sorted by slot name.
    pub int_modifiers: Vec<IntModifierEntry>,
}

impl Default for ModifierDebugPayload {
    fn default() -> Self {
        Self {
            schema_version: DEBUG_SCHEMA_VERSION,
            flags: Vec::new(),
            float_modifiers: Vec::new(),
            int_modifiers: Vec::new(),
        }
    }
}

/// One AI-controlled ship's doctrine-pool projection.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ShipDoctrine {
    /// The ship's display name (`EntityName`), or `"<unnamed>"`.
    pub ship: String,
    /// The ship's stable entity uuid, when it carries one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uuid: Option<String>,
    /// The directive the ship is acting on this tick — the highest positively
    /// scored objective that carries a real directive — or `None` when the pool
    /// is empty or everything gated out to zero. Mirrors what the helm/weapons
    /// AI actually serve (`decision_trace::top_directive`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chosen: Option<DoctrineChoice>,
    /// Every candidate objective in the pool, sorted by descending score then
    /// id, so the ordering is deterministic and the winner reads first.
    pub candidates: Vec<DoctrineCandidate>,
}

/// The directive a ship is acting on — the resolved winner of its pool.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DoctrineChoice {
    /// The winning objective's id.
    pub id: String,
    /// A compact label for the directive kind and the entity/anchor it names,
    /// e.g. `"Destroy(Ashrender)"` (`decision_trace::directive_label`).
    pub directive: String,
    /// The resolved target/anchor the directive names, or `None` for a
    /// target-less directive (`decision_trace::directive_target`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    /// The winner's utility score.
    pub score: f32,
}

/// One scored candidate objective in a ship's doctrine pool.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DoctrineCandidate {
    /// Stable objective id (`ScoredObjective::id`).
    pub id: String,
    /// Computed utility score (`0.0` = gated-out / inactive).
    pub score: f32,
    /// `"Mission"` or `"Doctrine"` — where the objective came from.
    pub source: String,
    /// The ship systems this directive is relevant to (`Helm`, `Weapons`, …),
    /// as their debug names.
    pub relevance: Vec<String>,
    /// The directive kind and target, as a compact label
    /// (`decision_trace::directive_label`).
    pub directive: String,
    /// The resolved target/anchor the directive names, or `None` for a
    /// target-less directive (`decision_trace::directive_target`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    /// Whether the underlying objective is mandatory.
    pub mandatory: bool,
    /// The objective's status (`"Active"` / `"Completed"` / `"Failed"`).
    pub status: String,
}

impl ScenarioStatePayload {
    /// A version-stamped empty payload; the base the collector fills.
    pub fn empty() -> Self {
        Self::default()
    }
}

// ── Entity-behavior surface (issue #1150) ────────────────────────────────────
//
// The structured form of the legacy `write_entity_debug_state` table: every
// AI-driven entity's name, position and current Tactical lock. Sorted by name
// so the JSON is deterministic regardless of ECS iteration order.

/// One AI-driven entity's behavior row.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EntityBehaviorEntry {
    /// The entity's display name, or `<unnamed>`.
    pub name: String,
    /// World position.
    pub x: f32,
    pub y: f32,
    pub z: f32,
    /// The ship's authoritative Tactical lock, or `none` (issue #702).
    pub target: String,
}

/// The entity-behavior surface's whole payload.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EntityBehaviorPayload {
    /// [`DEBUG_SCHEMA_VERSION`] at the time the host produced this payload.
    pub schema_version: u32,
    /// AI-driven entities, sorted by name.
    pub entries: Vec<EntityBehaviorEntry>,
}

impl Default for EntityBehaviorPayload {
    fn default() -> Self {
        Self {
            schema_version: DEBUG_SCHEMA_VERSION,
            entries: Vec::new(),
        }
    }
}

// ── Entity-inspector surface (issue #1150) ───────────────────────────────────
//
// The structured form of the legacy `update_entity_inspector` block: the player
// ship's position, per-system hull and per-arc shields, plus every non-asteroid
// world entity's name, tags, position, distance, faction, hull, comms
// hailability and AI target. World entities are sorted by distance from the
// player (then name) so the JSON is deterministic.

/// One system's hull HP on the player ship.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InspectorHullEntry {
    /// The `SystemId` string.
    pub system: String,
    /// Current hull HP.
    pub current: f32,
    /// Maximum hull HP.
    pub max: f32,
}

/// One shield arc's state on the player ship.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InspectorShieldFacing {
    /// The arc label (e.g. `Fore`, `Port`).
    pub label: String,
    /// Current arc HP.
    pub hp: i32,
    /// Maximum arc HP.
    pub max_hp: i32,
    /// Whether the arc is currently offline (recovering).
    pub offline: bool,
    /// Whether this arc is the focused (reinforced) one.
    pub focused: bool,
}

/// The player ship's inspector block.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InspectorPlayer {
    /// Planar position.
    pub x: f32,
    pub z: f32,
    /// Per-system hull, in the ship's declared system order.
    pub hull: Vec<InspectorHullEntry>,
    /// Per-arc shields.
    pub shields: Vec<InspectorShieldFacing>,
}

/// One inspected world entity.
///
/// The optional fields mirror the legacy overlay, which only printed a line when
/// the matching component was present: `faction` iff the entity had a faction,
/// the `hull_*` pair iff it had hull, the `comms_*` trio iff it had a comms
/// range, `ai_target` iff it carried a Tactical selection (its value is `none`
/// when the selection is empty).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InspectorEntity {
    /// The entity's display name.
    pub name: String,
    /// The entity's tags.
    pub tags: Vec<String>,
    /// Planar position.
    pub x: f32,
    pub z: f32,
    /// Distance from the player ship.
    pub distance: f32,
    /// Faction display name, if the entity has a faction.
    pub faction: Option<String>,
    /// Current total hull, if the entity has hull.
    pub hull_current: Option<f32>,
    /// Maximum total hull, if the entity has hull.
    pub hull_max: Option<f32>,
    /// Whether the entity is hailable, if it has a comms range (always `true`
    /// today — the presence of a range is what makes it hailable).
    pub comms_hailable: Option<bool>,
    /// Whether the player is within comms range, if the entity has one.
    pub comms_in_range: Option<bool>,
    /// The entity's comms range, if it has one.
    pub comms_range: Option<f32>,
    /// The entity's Tactical lock (`none` when empty), if it carries one.
    pub ai_target: Option<String>,
}

/// The entity-inspector surface's whole payload.
///
/// `player` is `None` when there is no LocalShip (a headless run); `entities`
/// is still produced. The payload is always emitted so the dock and the
/// determinism guard have something to read.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EntityInspectorPayload {
    /// [`DEBUG_SCHEMA_VERSION`] at the time the host produced this payload.
    pub schema_version: u32,
    /// The player ship block, if a LocalShip exists.
    pub player: Option<InspectorPlayer>,
    /// World entities, sorted by distance from the player (then name).
    pub entities: Vec<InspectorEntity>,
}

impl Default for EntityInspectorPayload {
    fn default() -> Self {
        Self {
            schema_version: DEBUG_SCHEMA_VERSION,
            player: None,
            entities: Vec::new(),
        }
    }
}

// ── AI-state surface: the per-host policy machine view (issue #1152) ─────────
//
// The second AI observability sub-surface. Where `ShipDoctrine` above answers
// "why did the ship pick this directive", this answers "what is the ship's
// stateful fine-system policy machine doing" — the black box #882 introduced.
// One entry per stateful fine-system AI host (the helm Engines/Steering/Boost
// axes today: the hosts that carry an `AiPolicyRuntimeState`), organised by the
// `crate::entities::ai_flag_hosts` registry's own host names rather than a new
// parallel index. The projector that fills these lives in
// `crate::debug::ai_state`; the last-transition and blocked-transition it reads
// are recorded read-only by `tick_policy_machine` and are never folded into the
// #894 digest.

/// One stateful fine-system AI host's policy-machine projection (issue #1152).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HostPolicyView {
    /// The owning ship's display name (`EntityName`), or `"<unnamed>"`.
    pub ship: String,
    /// The ship's stable entity uuid, when it carries one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uuid: Option<String>,
    /// The AI host's name from the registry (`ai_flag_hosts::AiHost::system`),
    /// e.g. `"Helm boost"` — so the view is keyed off the same registry that
    /// names every evaluation site, not a new parallel index.
    pub host: String,
    /// The currently-entered policy state id.
    pub state: String,
    /// The tick-derived clock reading at which `state` was entered
    /// (`AiPolicyRuntimeState::entered_at_secs`). Combined with the payload's
    /// `tick` this reads as "how long in this state"; kept as the raw datum so
    /// two hosts fold identically.
    pub entered_at_secs: f64,
    /// This host's private memory bag, sorted by key so the wire order is
    /// deterministic.
    pub memory: Vec<HostMemoryEntry>,
    /// The most recent transition this machine committed, or `None` before it
    /// has taken one since a reset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_transition: Option<HostTransitionView>,
    /// The outgoing transition the machine considered and did not take on the
    /// most recent tick, with the guard blocking it — or `None` when every
    /// outgoing guard is satisfied or the state has no transitions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_transition: Option<HostBlockedView>,
}

/// One `(name, value)` reading from a host's private memory bag (issue #1152).
///
/// Not `Eq`: memory readings are `f64`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HostMemoryEntry {
    /// The memory slot name, as an author writes it in `memory(name)`.
    pub key: String,
    /// The current reading.
    pub value: f64,
}

/// A transition the machine committed, as projected for the surface (issue
/// #1152).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HostTransitionView {
    /// The state the machine left.
    pub from: String,
    /// The state it entered.
    pub to: String,
    /// The guard that fired, rendered as authored (`Predicate::render`).
    pub guard: String,
    /// The tick-derived clock reading at which it committed.
    pub at_secs: f64,
}

/// The outgoing transition the machine considered and rejected, with the guard
/// blocking it (issue #1152).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HostBlockedView {
    /// The current state the machine would leave.
    pub from: String,
    /// The state the blocked transition would enter.
    pub to: String,
    /// The guard that is not yet satisfied, rendered as authored.
    pub guard: String,
}

// ── Console input-to-feedback latency surface (issue #1169) ──────────────────
//
// The one surface on this pipeline whose data does not come only from the
// simulation host: two of its three `LatencySurface`s are measured by a CLIENT,
// on the client's own clock, and reported up through
// `ClientMessage::ReportConsoleLatency`. The wire sample type therefore lives in
// `crate::core::messages` (it crosses the wire) and is re-exported here, exactly
// as `AiDirective` / `ObjectiveStatus` are for the scenario surface — payload
// convention 1 is "one module to import to know the vocabulary", not "every type
// is declared here".
//
// Everything below is a DURATION. No wall-clock timestamp appears in this schema,
// in `ConsoleLatencyTracker`, or anywhere a digest can reach: see
// `crate::debug::console_latency` for why that is a hard constraint rather than
// a style choice.

pub use crate::core::messages::{ConsoleLatencySample, LatencySurface};

/// One segment's distribution over the samples in the tracker's window
/// (issue #1169).
///
/// p50/p75/max rather than a mean: the polish bar PRD #1144 sets is "frequent
/// actions acknowledge within ~100 ms", which is a claim about the *tail* a
/// player actually notices. A mean hides the tail; p75 and max are where a
/// stutter shows up. Percentiles are nearest-rank over the retained window
/// (see [`crate::debug::console_latency`]), which is the same definition
/// `vellum-perf` uses, so a number here and a number in a perf capture of the
/// same run mean the same thing.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LatencySummary {
    /// How many samples this distribution was computed over.
    pub count: u32,
    /// Median.
    pub p50_ms: f32,
    /// 75th percentile.
    pub p75_ms: f32,
    /// The worst sample retained.
    pub max_ms: f32,
}

/// One (surface, action) pair's latency distributions (issue #1169).
///
/// Every segment is `Option` because **which segments exist depends on the
/// path**, and an absent segment must read as absent rather than as zero:
///
/// | surface | `input_to_send` | `send_to_ack` | `input_to_ack` | `admit_to_broadcast` |
/// |---|---|---|---|---|
/// | `BrowserHost` / `PhoneConsole` | yes | yes | yes (derived) | no |
/// | `SimHost` | no | no | no | yes |
///
/// A client cannot observe the host's internal schedule and the host cannot
/// observe a client's input event, so neither invents the other's numbers.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ActionLatencyEntry {
    /// Which surface measured these samples.
    pub surface: LatencySurface,
    /// The action label. A console action name (`fire_phaser`) for a client
    /// surface; a `SystemControlPayload` variant name (`FirePhaser`) for
    /// `SimHost`, which never sees the client's vocabulary.
    pub action: String,
    /// Samples retained for this entry — the largest segment count, so a
    /// consumer can tell a well-populated entry from a single tap.
    pub count: u32,
    /// Input event → transport hand-off (client surfaces only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_to_send: Option<LatencySummary>,
    /// Transport hand-off → issuing surface handed fresh state (client surfaces
    /// only). The whole round trip, transport included.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub send_to_ack: Option<LatencySummary>,
    /// The end-to-end number the ~100 ms bar is about: the per-sample sum of the
    /// two above, summarised (client surfaces only). Summarising the sums rather
    /// than summing the summaries — p75(a+b) is not p75(a)+p75(b).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_to_ack: Option<LatencySummary>,
    /// The simulation host's own service window: wall time from the tick's
    /// command admission to the end of the tick's `SimSet::Broadcast`
    /// (`SimHost` only). This is the slice of `send_to_ack` the host is
    /// responsible for; whatever the two disagree by is transport plus client
    /// work, which nothing here measures directly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub admit_to_broadcast: Option<LatencySummary>,
}

/// The console-latency surface's whole payload (issue #1169).
///
/// Produced by `crate::debug::console_latency::ConsoleLatencyTracker::report`,
/// encoded by `crate::core::codec::encode_console_latency`, read by the dock
/// panel (`gui/console-latency-panel.js`) and embedded in the headless run
/// report. `actions` is sorted by `(surface, action)` so two hosts folding the
/// same samples produce byte-identical JSON (payload convention 4).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConsoleLatencyPayload {
    /// [`DEBUG_SCHEMA_VERSION`] at the time the host produced this payload.
    pub schema_version: u32,
    /// One entry per (surface, action), sorted.
    pub actions: Vec<ActionLatencyEntry>,
}

impl Default for ConsoleLatencyPayload {
    fn default() -> Self {
        Self {
            schema_version: DEBUG_SCHEMA_VERSION,
            actions: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ship::control_source::ControlSource;

    #[test]
    fn activity_source_maps_from_every_control_source() {
        assert_eq!(
            ActivitySource::from(ControlSource::Human),
            ActivitySource::Human
        );
        assert_eq!(ActivitySource::from(ControlSource::Ai), ActivitySource::Ai);
        assert_eq!(
            ActivitySource::from(ControlSource::Offline),
            ActivitySource::Offline
        );
    }

    #[test]
    fn entry_records_into_the_matching_source_and_totals() {
        let mut entry = StationActivityEntry::default();
        entry.record(ActivitySource::Human);
        entry.record(ActivitySource::Human);
        entry.record(ActivitySource::Ai);
        assert_eq!(entry.human, 2);
        assert_eq!(entry.ai, 1);
        assert_eq!(entry.offline, 0);
        assert_eq!(entry.total(), 3);
    }

    #[test]
    fn default_payload_carries_the_current_schema_version() {
        assert_eq!(
            StationActivityPayload::default().schema_version,
            DEBUG_SCHEMA_VERSION
        );
    }

    #[test]
    fn default_ai_state_payload_carries_the_current_schema_version() {
        let payload = AiStatePayload::default();
        assert_eq!(payload.schema_version, DEBUG_SCHEMA_VERSION);
        assert!(payload.ships.is_empty());
    }

    #[test]
    fn default_console_latency_payload_carries_the_current_schema_version() {
        let payload = ConsoleLatencyPayload::default();
        assert_eq!(payload.schema_version, DEBUG_SCHEMA_VERSION);
        assert!(payload.actions.is_empty());
    }

    /// A `SimHost` entry must not serialise empty client segments as zeroed
    /// distributions — an absent segment is absent, not "0 ms".
    #[test]
    fn absent_latency_segments_are_omitted_rather_than_zeroed() {
        let entry = ActionLatencyEntry {
            surface: LatencySurface::SimHost,
            action: "FirePhaser".into(),
            count: 3,
            input_to_send: None,
            send_to_ack: None,
            input_to_ack: None,
            admit_to_broadcast: Some(LatencySummary {
                count: 3,
                p50_ms: 1.0,
                p75_ms: 2.0,
                max_ms: 3.0,
            }),
        };
        let json = crate::core::codec::encode_console_latency(&ConsoleLatencyPayload {
            schema_version: DEBUG_SCHEMA_VERSION,
            actions: vec![entry],
        });
        assert!(json.contains("admit_to_broadcast"), "{json}");
        assert!(
            !json.contains("input_to_send"),
            "an unmeasured segment must be omitted, not zeroed: {json}"
        );
    }
}
