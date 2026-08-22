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
/// Today it carries only the per-ship doctrine pool (`ships`). Issue #1152 adds
/// the per-host fine-system policy view as its OWN `hosts` field alongside
/// `ships` — the two are independent sub-surfaces of the one AI payload (the
/// PRD's "doctrine pool plus per-host fine-system policy view"), so #1152 can
/// grow this struct without reshaping the doctrine part or any type below it.
///
/// Not `Eq`: the candidate scores are `f32`. `PartialEq` is enough for the
/// tests that compare payloads.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AiStatePayload {
    /// [`DEBUG_SCHEMA_VERSION`] at the time the host produced this payload.
    pub schema_version: u32,
    /// The `SimTick` at which this snapshot was projected.
    pub tick: u64,
    /// One entry per AI-controlled ship, sorted by `(ship, uuid)` so two hosts
    /// folding the same world produce byte-identical JSON.
    pub ships: Vec<ShipDoctrine>,
    // #1152 adds `hosts: Vec<HostPolicyView>` here, after `ships`.
}

impl Default for AiStatePayload {
    fn default() -> Self {
        Self {
            schema_version: DEBUG_SCHEMA_VERSION,
            tick: 0,
            ships: Vec::new(),
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

/// One trigger's pending state and current eligibility.
///
/// "Eligibility" is two independent facts the author needs to tell a waiting
/// beat from a dead one: whether the trigger is still ARMED to fire
/// ([`pending`](Self::pending)), and whether its `when` predicate currently
/// HOLDS ([`when_holds`](Self::when_holds)). A beat that never fires despite its
/// event arriving is usually a `when` that never became true.
///
/// # Extension point for #1151
///
/// Issue #1151 (trigger fire history with predicate values) adds a
/// `fire_history: Vec<...>` field to THIS struct. It is a named-field struct
/// precisely so that is additive — a new field with `#[serde(default)]` does not
/// reshape the payload and does not bump [`DEBUG_SCHEMA_VERSION`]. The
/// projector's rendered `condition` / `when` strings are already the vocabulary
/// #1151's per-fire records quote predicate values against.
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
}
