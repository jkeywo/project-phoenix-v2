use crate::core::messages::{
    AiDirective, ObjectiveSnapshot, ObjectiveSource, ObjectiveStatus, ScoredObjective, StationId,
};
use crate::ship::config::StationStanceConfig;
pub use phoenix_sim_contracts::directive;
pub use phoenix_sim_contracts::objective_utility::*;
use std::collections::BTreeMap;
/// Replace the shared entity metadata's global Objective hint with the exact
/// recipient's current projection. Never write this presentation back to
/// WorldData: its snapshot/digest must remain identical on every host.
pub fn project_entity_targets(
    entities: &[crate::core::messages::EntitySnapshot],
    objectives: &[ObjectiveSnapshot],
) -> Vec<crate::core::messages::EntitySnapshot> {
    let targets: std::collections::HashSet<&str> = objectives
        .iter()
        .filter(|objective| objective.status == ObjectiveStatus::Active && !objective.unassigned)
        .flat_map(|objective| objective.targets.iter().map(String::as_str))
        .collect();
    entities
        .iter()
        .map(|entity| {
            let mut projected = entity.clone();
            projected.objective_target = [
                Some(entity.uuid.as_str()),
                entity.id.as_deref(),
                entity.name.as_deref(),
            ]
            .into_iter()
            .flatten()
            .any(|name| targets.contains(name));
            projected
        })
        .collect()
}

/// Exact authored and lifecycle state, in insertion order in the manager.
/// Presentation transition buffers are deliberately stored separately.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ObjectiveRecord {
    id: String,
    text: String,
    /// Values interpolated into `text`'s `{placeholder}` tokens on the client.
    /// See `messages::TEXT_PARAMS_SUFFIX`. Empty for every objective that names
    /// a figure-free string.
    text_params: BTreeMap<String, String>,
    mandatory: bool,
    status: ObjectiveStatus,
    targets: Vec<String>,
    /// Intended ship UUIDs. Empty preserves legacy all-ship visibility.
    recipients: Vec<String>,
    /// Mission-altitude AI directive for this objective.
    directive: AiDirective,
    /// TOML-authored utility scoring configuration.
    utility: UtilityConfig,
    /// Whether this originated from a mission trigger or standing doctrine.
    source: ObjectiveSource,
    /// An objective-specific Command stance this objective contributes to a
    /// named target Station while it is `Active` (issue #1110).
    ///
    /// `Some((station, stance))` lends the Station a temporary authored stance —
    /// exposed only through [`ObjectiveManager::active_station_stances`], which
    /// filters on `status == Active`, so completing, failing or removing the
    /// objective withdraws it immediately. Never mutates the target Station's
    /// permanent catalogue; the Command consumers merge it in at read time. Most
    /// objectives author none and carry `None`, keeping their record and wire
    /// snapshot unchanged.
    command_stance: Option<(StationId, StationStanceConfig)>,
}

/// A read-only view over one objective for the scenario-state debug surface
/// (issue #1148). Borrows the record so [`ObjectiveManager::debug_views`] can
/// project without cloning; the debug projector maps it into the owned
/// `crate::debug::payload::ScenarioObjective` it puts on the wire.
#[derive(Clone, Copy, Debug)]
pub struct ObjectiveDebugView<'a> {
    /// Stable objective id.
    pub id: &'a str,
    /// Active / Completed / Failed.
    pub status: &'a ObjectiveStatus,
    /// Whether the mission requires this objective.
    pub mandatory: bool,
    /// The authored base priority — the "score" the debug objective table shows,
    /// before the mandatory bonus and any per-tick condition modifiers.
    pub base_priority: f32,
    /// The mission-altitude AI directive attached to this objective.
    pub directive: &'a AiDirective,
}

/// Which mutation an [`ObjectiveTransition`] records.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjectiveTransitionKind {
    /// A new `Active` objective was inserted.
    Posted,
    /// An `Active` objective became `Completed`.
    Completed,
    /// An `Active` objective became `Failed`.
    Failed,
    /// The record was dropped entirely (a world layer unloading its own).
    Removed,
}

/// One mutation of the objective set, logged in the order it happened.
///
/// Exists because the mission timeline (issue #1338) asks for *transitions*, and
/// a status field can only report the state a tick ENDED in. An objective posted
/// and completed inside one tick — a handler that adds it and a deadline
/// callback that resolves it, both on the same fixed tick — is one status field
/// and two story beats. Diffing the snapshot can only ever see the second.
///
/// The record's fields are copied in rather than referenced by id because the
/// drain happens after the tick: an objective posted and then REMOVED in one
/// tick has no record left to look up, and its posting still happened.
///
/// Non-authoritative by construction: nothing in the fixed tick reads this log,
/// `sim_digest`/`snapshot` do not walk it, and
/// [`crate::narrative::emit_scenario_narrative`] drains it in full every tick.
#[derive(Clone, Debug, PartialEq)]
pub struct ObjectiveTransition {
    /// The objective's stable id.
    pub id: String,
    /// What happened to it.
    pub kind: ObjectiveTransitionKind,
    /// The objective's `strings.csv` text id, carried so a posting that no
    /// longer has a record can still be reported in full.
    pub text: String,
    /// Whether the mission requires it.
    pub mandatory: bool,
    /// The objective's authored targets, in authored order.
    pub targets: Vec<String>,
}

/// Manages the full lifecycle of mission objectives.
#[derive(Clone, Debug, Default)]
pub struct ObjectiveManager {
    objectives: Vec<ObjectiveRecord>,
    dirty: bool,
    /// Every mutation since the last [`ObjectiveManager::drain_transitions`],
    /// in the order it was made — see [`ObjectiveTransition`]. Drained once per
    /// fixed tick by the narrative recorder; a build with no recorder (a bare
    /// `App` unit test) simply never reads it, and it grows only on actual
    /// objective mutations, which are authored and few.
    transitions: Vec<ObjectiveTransition>,
    /// Presentation-only baseline for the narrative diff after a restore.
    /// Captured at restoration, so later real transitions are still emitted.
    restored_statuses: Option<BTreeMap<String, ObjectiveStatus>>,
}

impl ObjectiveManager {
    /// Create an empty `ObjectiveManager`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a new `Active` objective (backward-compatible; directive defaults to `None`).
    ///
    /// If an objective with this `id` already exists it is **not** duplicated;
    /// the call is a no-op and returns `false`. Returns `true` when the
    /// objective was newly inserted.
    pub fn add(
        &mut self,
        id: impl Into<String>,
        text: impl Into<String>,
        mandatory: bool,
        targets: Vec<String>,
    ) -> bool {
        self.add_full(
            id,
            text,
            mandatory,
            targets,
            AiDirective::default(),
            UtilityConfig::default(),
            ObjectiveSource::default(),
        )
    }

    /// Add a new `Active` objective with full directive + utility config.
    ///
    /// If an objective with this `id` already exists it is **not** duplicated;
    /// the call is a no-op and returns `false`. Returns `true` when inserted.
    pub fn add_full(
        &mut self,
        id: impl Into<String>,
        text: impl Into<String>,
        mandatory: bool,
        targets: Vec<String>,
        directive: AiDirective,
        utility: UtilityConfig,
        source: ObjectiveSource,
    ) -> bool {
        self.add_full_with_params(
            id,
            text,
            BTreeMap::new(),
            mandatory,
            targets,
            directive,
            utility,
            source,
            None,
        )
    }

    /// Add a new `Active` objective whose text carries runtime values.
    ///
    /// The widest door, and the only one that inserts. `add` and `add_full` are
    /// this with progressively more defaults, the same way `add` was already
    /// `add_full` with an empty directive and utility — so there is one insert
    /// site rather than three, and a field added to `ObjectiveRecord` cannot be
    /// missed at two of them.
    ///
    /// `text_params` is empty for every objective that names a figure-free
    /// string, which is what keeps its `ObjectiveSnapshot` byte-identical on the
    /// wire (`skip_serializing_if`).
    ///
    /// If an objective with this `id` already exists it is **not** duplicated;
    /// the call is a no-op and returns `false`. Returns `true` when inserted.
    #[allow(clippy::too_many_arguments)] // one arg per record field
    pub fn add_full_with_params(
        &mut self,
        id: impl Into<String>,
        text: impl Into<String>,
        text_params: BTreeMap<String, String>,
        mandatory: bool,
        targets: Vec<String>,
        directive: AiDirective,
        utility: UtilityConfig,
        source: ObjectiveSource,
        command_stance: Option<(StationId, StationStanceConfig)>,
    ) -> bool {
        let id = id.into();
        if self.objectives.iter().any(|o| o.id == id) {
            return false;
        }
        let text = text.into();
        // Logged BEFORE the move into the record, and only on the branch that
        // actually inserts — a duplicate id is a no-op and no beat (issue
        // #1338).
        self.transitions.push(ObjectiveTransition {
            id: id.clone(),
            kind: ObjectiveTransitionKind::Posted,
            text: text.clone(),
            mandatory,
            targets: targets.clone(),
        });
        self.objectives.push(ObjectiveRecord {
            id,
            text,
            text_params,
            mandatory,
            status: ObjectiveStatus::Active,
            targets,
            recipients: Vec::new(),
            directive,
            utility,
            source,
            command_stance,
        });
        self.dirty = true;
        true
    }

    /// Log one transition off the record it happened to (issue #1338).
    fn log_transition(rec: &ObjectiveRecord, kind: ObjectiveTransitionKind) -> ObjectiveTransition {
        ObjectiveTransition {
            id: rec.id.clone(),
            kind,
            text: rec.text.clone(),
            mandatory: rec.mandatory,
            targets: rec.targets.clone(),
        }
    }

    /// Take the ordered log of every mutation since the last drain (issue
    /// #1338).
    ///
    /// The mission-timeline recorder's primary input: it reports one event per
    /// entry, so an objective posted and resolved inside a single fixed tick
    /// produces both beats and not just the terminal one. Draining (rather than
    /// reading) is what keeps the log a per-tick buffer rather than a growing
    /// second copy of the objective set.
    pub fn drain_transitions(&mut self) -> Vec<ObjectiveTransition> {
        std::mem::take(&mut self.transitions)
    }

    /// The Command stances currently contributed by `Active` objectives
    /// (issue #1110), each paired with the target Station it is lent to.
    ///
    /// Filtering on `status == Active` is the whole removal mechanism: the same
    /// gate the AI-facing `scored_pool` uses. Completing or failing an objective
    /// moves it out of `Active`, and [`remove`](Self::remove) deletes the record
    /// outright, so any of the three drops the contribution here on the very next
    /// read — the Command consumers stop exposing the stance and reconcile any
    /// selection of it away. An objective that authored no stance contributes
    /// nothing.
    pub fn active_station_stances(&self) -> Vec<(StationId, StationStanceConfig)> {
        self.objectives
            .iter()
            .filter(|o| o.status == ObjectiveStatus::Active)
            .filter_map(|o| o.command_stance.clone())
            .collect()
    }

    /// Transition an `Active` objective to `Completed`.
    ///
    /// Returns `true` if the objective was found and transitioned.
    /// If the objective does not exist or is not `Active`, returns `false`.
    pub fn complete(&mut self, id: &str) -> bool {
        if let Some(rec) = self
            .objectives
            .iter_mut()
            .find(|o| o.id == id && o.status == ObjectiveStatus::Active)
        {
            rec.status = ObjectiveStatus::Completed;
            let transition = Self::log_transition(rec, ObjectiveTransitionKind::Completed);
            self.transitions.push(transition);
            self.dirty = true;
            true
        } else {
            false
        }
    }

    /// Transition an `Active` objective to `Failed`.
    ///
    /// Returns `true` if the objective was found and transitioned.
    /// If the objective does not exist or is not `Active`, returns `false`.
    pub fn fail(&mut self, id: &str) -> bool {
        if let Some(rec) = self
            .objectives
            .iter_mut()
            .find(|o| o.id == id && o.status == ObjectiveStatus::Active)
        {
            rec.status = ObjectiveStatus::Failed;
            let transition = Self::log_transition(rec, ObjectiveTransitionKind::Failed);
            self.transitions.push(transition);
            self.dirty = true;
            true
        } else {
            false
        }
    }

    /// Authored targets for one retained objective.
    ///
    /// This narrow read is used by transition observers at the mutation seam;
    /// it deliberately exposes neither the private record nor mutable access.
    pub fn targets(&self, id: &str) -> Option<&[String]> {
        self.objectives
            .iter()
            .find(|objective| objective.id == id)
            .map(|objective| objective.targets.as_slice())
    }

    /// Status of a retained objective, including terminal records.
    pub fn status(&self, id: &str) -> Option<&ObjectiveStatus> {
        self.objectives
            .iter()
            .find(|o| o.id == id)
            .map(|o| &o.status)
    }

    /// Immutable recipient scope, distinct from the objective's subject targets.
    pub fn recipients(&self, id: &str) -> Option<&[String]> {
        self.objectives
            .iter()
            .find(|o| o.id == id)
            .map(|o| o.recipients.as_slice())
    }

    /// Stamp the scope immediately after a successful authored activation.
    /// Only the trusted activation seam calls this; existing records are never
    /// re-scoped by a GM action. Duplicate additions must not call this method.
    pub fn set_recipients(&mut self, id: &str, mut recipients: Vec<String>) -> bool {
        let Some(record) = self
            .objectives
            .iter_mut()
            .find(|o| o.id == id && o.status == ObjectiveStatus::Active && o.recipients.is_empty())
        else {
            return false;
        };
        recipients.sort();
        recipients.dedup();
        record.recipients = recipients;
        self.dirty = true;
        true
    }

    /// Whether this ship has an `Active` objective **explicitly scoped to it**.
    ///
    /// Deliberately stricter than [`Self::is_for_ship`], which also answers
    /// `true` for an unscoped record because legacy all-ship visibility is the
    /// right answer for a *display*. This is the question "has anybody given
    /// this hull a job", and a mission line addressed to everyone has given no
    /// particular hull anything. Read by the GM idle-NPC advisory
    /// ([`crate::gm_attention`]), which would otherwise fall silent for every
    /// NPC in the world the moment a scenario posted its first objective.
    pub fn has_active_for_ship(&self, ship: &str) -> bool {
        self.objectives.iter().any(|objective| {
            objective.status == ObjectiveStatus::Active
                && objective.recipients.iter().any(|id| id == ship)
        })
    }

    /// Whether a ship is an intended recipient. Missing identity sees only
    /// legacy unscoped objectives, never another ship's private assignment.
    pub fn is_for_ship(&self, id: &str, ship: &str) -> bool {
        self.recipients(id)
            .is_some_and(|scope| scope.is_empty() || scope.iter().any(|id| id == ship))
    }

    /// Durable records in their authoritative insertion order.
    pub fn records(&self) -> &[ObjectiveRecord] {
        &self.objectives
    }

    /// Restore exact state without manufacturing lifecycle events or score.
    pub fn restore_records(&mut self, records: Vec<ObjectiveRecord>) {
        self.restored_statuses = Some(
            records
                .iter()
                .map(|record| (record.id.clone(), record.status.clone()))
                .collect(),
        );
        self.objectives = records;
        self.transitions.clear();
        self.dirty = true;
    }

    /// Consume the restored baseline before reading new lifecycle transitions.
    /// This observer state is neither captured nor folded into simulation state.
    pub fn take_restored_statuses(&mut self) -> Option<BTreeMap<String, ObjectiveStatus>> {
        self.restored_statuses.take()
    }

    /// The ordinary sorted projection restricted to an intended ship.
    pub fn snapshots_for(&self, ship: &str) -> Vec<ObjectiveSnapshot> {
        self.sorted_snapshots()
            .into_iter()
            .filter(|o| self.is_for_ship(&o.id, ship))
            .collect()
    }

    /// Active mission directives available to this ship.
    pub fn scored_pool_for(
        &self,
        conditions: &WorldConditions,
        ship: &str,
    ) -> Vec<ScoredObjective> {
        self.scored_pool_with_boost_for(conditions, None, ship)
    }

    /// Ship-scoped scoring, with that ship's Captain priority selection.
    pub fn scored_pool_with_boost_for(
        &self,
        conditions: &WorldConditions,
        boost: Option<&str>,
        ship: &str,
    ) -> Vec<ScoredObjective> {
        self.scored_pool_with_boost(conditions, boost)
            .into_iter()
            .filter(|o| self.is_for_ship(&o.id, ship))
            .collect()
    }

    /// Objective-contributed Command stances restricted to an intended ship.
    pub fn active_station_stances_for(&self, ship: &str) -> Vec<(StationId, StationStanceConfig)> {
        self.objectives
            .iter()
            .filter(|o| o.status == ObjectiveStatus::Active && self.is_for_ship(&o.id, ship))
            .filter_map(|o| o.command_stance.clone())
            .collect()
    }

    /// Same projection with the owning definition id retained, for consumers
    /// that replace a legacy definition with a ship-effective named instance.
    pub fn active_station_stances_with_ids_for(
        &self,
        ship: &str,
    ) -> Vec<(String, StationId, StationStanceConfig)> {
        self.objectives
            .iter()
            .filter(|o| o.status == ObjectiveStatus::Active && self.is_for_ship(&o.id, ship))
            .filter_map(|o| {
                o.command_stance
                    .clone()
                    .map(|(station, stance)| (o.id.clone(), station, stance))
            })
            .collect()
    }

    /// Remove the objective with `id` entirely (issue #751).
    ///
    /// Unlike `fail`/`complete` (which transition status but keep the record),
    /// this drops the record so a world layer's objectives disappear when the
    /// layer unloads. Returns `true` if a record was removed.
    pub fn remove(&mut self, id: &str) -> bool {
        // Logged off the record BEFORE it is dropped (issue #1338): the timeline
        // recorder needs the objective's own fields to reconcile a posting that
        // happened earlier in the same tick, and after the retain there is
        // nothing left to read them from.
        let doomed: Vec<ObjectiveTransition> = self
            .objectives
            .iter()
            .filter(|o| o.id == id)
            .map(|o| Self::log_transition(o, ObjectiveTransitionKind::Removed))
            .collect();
        let before = self.objectives.len();
        self.objectives.retain(|o| o.id != id);
        let removed = self.objectives.len() != before;
        if removed {
            self.transitions.extend(doomed);
            self.dirty = true;
        }
        removed
    }

    /// Returns a sorted snapshot of all objectives: mandatory first (in
    /// insertion order), then optional (in insertion order).
    ///
    /// This is the slice that should be packed into `ObjectiveSummary`.
    pub fn sorted_snapshots(&self) -> Vec<ObjectiveSnapshot> {
        let mandatory: Vec<_> = self
            .objectives
            .iter()
            .filter(|o| o.mandatory)
            .map(record_to_snapshot)
            .collect();
        let optional: Vec<_> = self
            .objectives
            .iter()
            .filter(|o| !o.mandatory)
            .map(record_to_snapshot)
            .collect();
        mandatory.into_iter().chain(optional).collect()
    }

    /// Compute and return the utility-scored pool of all **active** objectives.
    ///
    /// Each objective is scored against the supplied `WorldConditions`. Zero-gated
    /// objectives are included with `score = 0.0` so the AI can see them and
    /// skip them cleanly. The pool is sorted descending by score.
    pub fn scored_pool(&self, conditions: &WorldConditions) -> Vec<ScoredObjective> {
        self.scored_pool_with_boost(conditions, None)
    }

    /// Like `scored_pool` but applies an optional captain priority selection.
    ///
    /// A captain's selected objective must outrank every other active objective:
    /// it is an explicit command decision, not a small utility preference that
    /// a sufficiently large authored score may ignore. The selected objective
    /// therefore receives the greatest finite score before the deterministic
    /// sort. Keeping it finite preserves the wire codec's JSON number contract.
    pub fn scored_pool_with_boost(
        &self,
        conditions: &WorldConditions,
        boost: Option<&str>,
    ) -> Vec<ScoredObjective> {
        let mut pool: Vec<ScoredObjective> = self
            .objectives
            .iter()
            .filter(|o| o.status == ObjectiveStatus::Active)
            .map(|o| {
                let mut score = o.utility.score(o.mandatory, conditions);
                if let Some(boost_id) = boost {
                    if o.id == boost_id {
                        score = f32::MAX;
                    }
                }
                let relevance = directive_relevance(&o.directive);
                ScoredObjective {
                    id: o.id.clone(),
                    score,
                    directive: o.directive.clone(),
                    source: o.source.clone(),
                    relevance,
                    snapshot: record_to_snapshot(o),
                }
            })
            .collect();
        // `total_cmp` gives a total, deterministic order (no `NaN`-dependent
        // `Equal` fallback). Player-facing panels (captain, comms) filter this
        // pool through `is_visible_objective`, and the AI-facing viewscreen pool
        // re-sorts the unioned result with `total_cmp` too — the rng-determinism
        // guard depends on every scoring path being totally ordered (#752).
        pool.sort_by(|a, b| b.score.total_cmp(&a.score));
        pool
    }

    /// Read-only views over every objective for the scenario-state debug
    /// surface (issue #1148): each objective's id, status, whether it is
    /// mandatory, its authored base priority, and its AI directive.
    ///
    /// A borrowing projection rather than an extension of [`ObjectiveSnapshot`]:
    /// the wire snapshot the captain panel reads deliberately carries neither
    /// the directive nor the raw base priority, and this surface must not widen
    /// that player-facing payload. Mandatory objectives come first (the manager's
    /// insertion-ordered listing), so the debug table reads in the same order the
    /// captain panel does. Reads nothing dirty and mutates nothing — a pure
    /// projection off authoritative state.
    pub fn debug_views(&self) -> impl Iterator<Item = ObjectiveDebugView<'_>> {
        let mandatory = self.objectives.iter().filter(|o| o.mandatory);
        let optional = self.objectives.iter().filter(|o| !o.mandatory);
        mandatory.chain(optional).map(|o| ObjectiveDebugView {
            id: &o.id,
            status: &o.status,
            mandatory: o.mandatory,
            base_priority: o.utility.base_priority,
            directive: &o.directive,
        })
    }

    /// `true` when the objective list has changed since the last `mark_clean` call.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Reset the dirty flag. Call after broadcasting `ObjectiveSummary`.
    pub fn mark_clean(&mut self) {
        self.dirty = false;
    }
}

fn record_to_snapshot(r: &ObjectiveRecord) -> ObjectiveSnapshot {
    ObjectiveSnapshot {
        progress: None,
        unassigned: false,
        id: r.id.clone(),
        text: r.text.clone(),
        text_params: r.text_params.clone(),
        mandatory: r.mandatory,
        status: r.status.clone(),
        targets: r.targets.clone(),
        source: r.source.clone(),
    }
}
