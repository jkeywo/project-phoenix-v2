// Pure Rust module for managing mission objectives.
// No Bevy dependency. Owns all objective state for the running simulation.
//
// PRD #342: legacy multi-scenario layering is gone. Objectives live for the
// duration of the session. Completed/failed objectives are retained until
// explicitly cleared.
//
// The public surface is intentionally narrow:
//   - `ObjectiveManager::add` — register a new active objective (backward compat)
//   - `ObjectiveManager::add_full` — register with directive + utility config
//   - `ObjectiveManager::add_full_with_params` — the above plus a table of
//     runtime values to interpolate into the objective's text

//   - `ObjectiveManager::complete` — transition active → completed
//   - `ObjectiveManager::fail` — transition active → failed
//   - `ObjectiveManager::sorted_snapshots` — sorted view (mandatory first)
//   - `ObjectiveManager::scored_pool` — utility-scored pool for AI (issue #571)
//   - `ObjectiveManager::is_dirty` / `ObjectiveManager::mark_clean` — change tracking
//     so callers can push `ObjectiveSummary` only on change

use crate::messages::{
    AiDirective, ObjectiveSnapshot, ObjectiveSource, ObjectiveStatus, ScoredObjective, StationId,
    SystemAffinity,
};
use crate::ship::config::StationStanceConfig;
use std::collections::BTreeMap;

// ── Utility scoring types ──────────────────────────────────────────────────

/// A condition-weighted modifier added to a utility score when the condition
/// evaluates to true at scoring time.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ConditionModifier {
    /// Condition name: `"red_alert"`, `"hull_below"`, `"hull_above"`, `"attacked"`, `"not_attacked"`.
    pub condition: String,
    /// Optional numeric threshold (required for `hull_below` / `hull_above`).
    pub threshold: Option<f32>,
    /// Weight added to (or subtracted from) the score when the condition is true.
    pub weight: f32,
}

/// A veto condition. When the condition evaluates to **false** the objective's
/// score is forced to 0 and it is never selected by the AI.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ZeroGateCondition {
    /// Condition name: `"red_alert"`, `"hull_below"`, `"hull_above"`, `"attacked"`, `"not_attacked"`.
    pub condition: String,
    /// Optional numeric threshold (required for `hull_below` / `hull_above`).
    pub threshold: Option<f32>,
}

/// TOML-authored utility configuration for an objective (issue #571).
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct UtilityConfig {
    /// Base score before modifiers. Mandatory objectives receive an extra
    /// `MANDATORY_BONUS` on top of this.
    pub base_priority: f32,
    /// Condition-weighted score modifiers, applied when their condition is true.
    pub modifiers: Vec<ConditionModifier>,
    /// Veto conditions. Any gate whose condition evaluates to `false` forces the
    /// final score to 0 (never selected by AI).
    pub zero_gates: Vec<ZeroGateCondition>,
}

/// Extra score added when an objective is marked `mandatory = true`.
const MANDATORY_BONUS: f32 = 10.0;

/// Snapshot of world conditions evaluated when scoring the objective pool.
/// Derived from published blackboards + world registry (no live Bevy reads).
#[derive(Clone, Debug, Default)]
pub struct WorldConditions {
    /// Whether the ship is currently in red alert.
    pub red_alert: bool,
    /// Current hull integrity fraction [0.0, 1.0].
    pub hull_fraction: f32,
    /// Whether something landed a hit on the ship — shields or hull — recently
    /// enough that it still counts as under attack. See [`attacked_recently`]
    /// over [`last_landed_hit_secs`], which both publish sites derive this from.
    pub attacked: bool,
}

/// When something last LANDED a hit on this ship: the more recent of hull
/// damage taken and hostile fire an arc absorbed. The pure fold both `attacked`
/// publish sites reduce a ship's combat activity through before calling
/// [`attacked_recently`], so neither can pick a different set of readings.
///
/// Hull damage alone is not enough. `RecentCombatActivity::last_damage_taken`
/// is written only when the hull TOTAL actually drops
/// (`ship::combat_activity::update_combat_activity`), so fire a shield eats
/// never reaches it — it lands in `last_hostile_fire_taken` instead.
/// `assets/entities/station_axiom.toml` shoots 5 dps in 4 s bursts with no
/// `shield_pierce` at a Harrow's single 90 hp arc regenerating 2/s: the burst
/// (20 dmg) barely outpaces the regen over the same cycle (16 dmg over 8 s),
/// netting only ~4 hp/cycle, so shield-absorbed fire dominates the opening of
/// an engagement and hull damage does not register until sustained pressure
/// collapses the arc (roughly three minutes of continuous fire). Reading
/// damage alone would leave that Harrow flying its raid while the station
/// shot at it for most of a short engagement, which is the behaviour the old
/// per-beam `LastShipAttacker` signal did get right.
///
/// `last_weapon_fired` is deliberately NOT folded in: firing your own guns is
/// not being attacked, and folding it would make a `not_attacked` gate veto
/// itself the moment the hull opened fire and hold the veto for as long as it
/// kept firing. (The captain's `secs_since_combat` red-alert fact DOES fold all
/// three — it asks "is this ship in a fight", which is a different question.)
pub fn last_landed_hit_secs(
    last_damage_taken_secs: Option<f32>,
    last_hostile_fire_taken_secs: Option<f32>,
) -> Option<f32> {
    match (last_damage_taken_secs, last_hostile_fire_taken_secs) {
        (Some(damage), Some(fire)) => Some(damage.max(fire)),
        (Some(damage), None) => Some(damage),
        (None, fire) => fire,
    }
}

/// Whether a hit landed recently enough that the ship still counts as under
/// attack (issue #1010). Take `last_hit_secs` from [`last_landed_hit_secs`] —
/// a hit that connected, shields or hull.
///
/// `attacked` used to read the `LastShipAttacker` latch, which is set on the
/// first beam that connects and cleared only when the ship dies or when its red
/// alert stands down (`server_app::clear_last_attacker_on_red_alert_off`).
/// Every Harrow hull DOES author a captain stand-down —
/// `combat_window_secs = 10.0` in `ship_harrow_cruiser.toml` — so the latch is
/// releasable in principle. What it is not is releasable DURING a fight: the
/// captain's `secs_since_combat` fact folds the hull's OWN weapon fire in
/// alongside damage and hostile fire, so a Harrow returning fire keeps resetting
/// its own stand-down clock, red alert never drops, and the latch never clears.
/// (Hulls that author an alert-on-hostile rule hold the alert up on mere contact
/// as well — `alliance_courier.toml`'s priority-5 rule; a Harrow authors none.)
/// So with a player ship loitering nearby, `combat_test.toml`'s
/// `not_attacked`-gated `assault-starbase` stayed retired for as long as the
/// loitering lasted — the raid the scenario is named for never resumed, which
/// is what the playtest saw.
///
/// Recency decays instead: the gate closes on the landed hit and reopens once
/// `window_secs` of simulation time pass with no further one. Both times are
/// `Time::elapsed_secs()` read inside `FixedUpdate` — SIM seconds off the fixed
/// clock, never a wall clock (AGENTS.md #7) — and `window_secs` is authored as
/// `[global] attacked_memory_secs`.
///
/// The two windows are separate on purpose. The per-hull captain
/// `combat_window_secs` governs ALERT POSTURE; the global `attacked_memory_secs`
/// governs this doctrine gate directly, which is what decouples resuming a raid
/// from the red-alert/`LastShipAttacker` chain that could not release while the
/// shooting continued.
///
/// A ship nothing has hit (`None`) is not under attack. A non-positive window
/// degenerates to "never under attack", which is the honest reading of a
/// designer authoring a zero-length memory.
pub fn attacked_recently(last_hit_secs: Option<f32>, now_secs: f32, window_secs: f32) -> bool {
    match last_hit_secs {
        Some(last) => now_secs - last < window_secs,
        None => false,
    }
}

fn evaluate_condition(condition: &str, threshold: Option<f32>, cond: &WorldConditions) -> bool {
    match condition {
        "red_alert" => cond.red_alert,
        "hull_below" => cond.hull_fraction < threshold.unwrap_or(0.3),
        "hull_above" => cond.hull_fraction > threshold.unwrap_or(0.3),
        "attacked" => cond.attacked,
        "not_attacked" => !cond.attacked,
        _ => false,
    }
}

impl UtilityConfig {
    /// Compute the utility score given current world conditions.
    ///
    /// Returns 0.0 if any zero-gate condition evaluates to `false`.
    pub fn score(&self, mandatory: bool, cond: &WorldConditions) -> f32 {
        for gate in &self.zero_gates {
            if !evaluate_condition(&gate.condition, gate.threshold, cond) {
                return 0.0;
            }
        }
        let mut score = self.base_priority;
        if mandatory {
            score += MANDATORY_BONUS;
        }
        for m in &self.modifiers {
            if evaluate_condition(&m.condition, m.threshold, cond) {
                score += m.weight;
            }
        }
        score.max(0.0)
    }
}

/// Derive which ship systems care about a given directive kind.
pub fn directive_relevance(directive: &AiDirective) -> Vec<SystemAffinity> {
    match directive {
        AiDirective::None => vec![],
        AiDirective::Destroy { .. } => {
            vec![
                SystemAffinity::Helm,
                SystemAffinity::Weapons,
                SystemAffinity::Captain,
            ]
        }
        // Dock (issue #1028) joins the travel directives: it names a place to
        // be, its destination is resolved by the same navigation-objective path
        // Destroy's is, and the hull flies it with the same waypoint hand-off.
        // Deliberately NOT `Weapons` — a civilian berthing at a depot is not
        // acquiring it.
        AiDirective::Patrol { .. }
        | AiDirective::Reach { .. }
        | AiDirective::Retreat { .. }
        | AiDirective::Dock { .. } => {
            vec![SystemAffinity::Helm]
        }
        // Hail is a Comms action: the Backfill Comms AI (issue #753) consumes
        // these from its local scored pool and issues the same typed `Hail`
        // command a human Comms officer sends. No consumer filters Hail on
        // `Captain`, so Comms is the sole relevant affinity.
        AiDirective::Hail { .. } => vec![SystemAffinity::Comms],
        // The tractor operate directives (issue #1162): each routes to the
        // owning system's affinity — Engineering, which owns the tractor. The
        // seat decides the concrete `EngageTractor`/`ReleaseTractor`, so these
        // live upstream of admission and command-level symmetry holds. The
        // Weapons Tactical selector ALSO reads them (through its own
        // `objective-operate` source) to lock the named target, but that is not
        // an affinity — the tractor is Engineering's, not Weapons'.
        AiDirective::Tow { .. } | AiDirective::Stabilise { .. } | AiDirective::Escort { .. } => {
            vec![SystemAffinity::Engineering]
        }
        // Transfer is a two-seat chain (issue #1162): Helm docks (the dock is
        // Helm-owned) and Engineering runs the umbilical over the mated dock.
        // Both seats are relevant so each backfilled host sees the directive.
        AiDirective::Transfer { .. } => {
            vec![SystemAffinity::Helm, SystemAffinity::Engineering]
        }
        // FieldRepair routes to Repair, which owns the external dispatch. The
        // Weapons Tactical selector reads it too (to lock the ally), for the
        // same reason as the tractor verbs above.
        AiDirective::FieldRepair { .. } => vec![SystemAffinity::Repair],
    }
}

/// The top-scored ACTIVE directive relevant to `affinity` whose kind `wanted`
/// accepts, from a viewscreen scored-objective pool (issue #1162).
///
/// The pool is already sorted descending by score (see `scored_pool`), so the
/// first match is the top one. Pure and Bevy-free (AGENTS.md rule 10), so the
/// four backfill operate hosts — tractor, umbilical, dock and external repair —
/// share ONE selection rule and it can be unit-tested without an `App`. A
/// zero-score objective is skipped exactly as the other AI-facing consumers skip
/// it, so a boosted or condition-changed directive re-activates without the pool
/// being republished.
pub fn top_operate_directive(
    scored: &[ScoredObjective],
    affinity: SystemAffinity,
    wanted: impl Fn(&AiDirective) -> bool,
) -> Option<&AiDirective> {
    scored.iter().find_map(|o| {
        (o.score > 0.0 && o.relevance.contains(&affinity) && wanted(&o.directive))
            .then_some(&o.directive)
    })
}

/// The target a tractor operate directive (`Tow`/`Stabilise`/`Escort`) names, or
/// `None` for any other directive (issue #1162). The tractor host's `wanted`
/// predicate, factored out so the host and its tests read one rule.
pub fn tractor_directive_target(directive: &AiDirective) -> Option<&str> {
    match directive {
        AiDirective::Tow { target }
        | AiDirective::Stabilise { target }
        | AiDirective::Escort { target } => Some(target.as_str()),
        _ => None,
    }
}

/// The target a `Transfer` directive names, or `None` (issue #1162). Shared by
/// the Helm dock host and the Engineering umbilical host — the two seats of the
/// resupply chain.
pub fn transfer_directive_target(directive: &AiDirective) -> Option<&str> {
    match directive {
        AiDirective::Transfer { target } => Some(target.as_str()),
        _ => None,
    }
}

/// The target a `FieldRepair` directive names, or `None` (issue #1162). The
/// external-repair dispatch host's predicate.
pub fn field_repair_directive_target(directive: &AiDirective) -> Option<&str> {
    match directive {
        AiDirective::FieldRepair { target } => Some(target.as_str()),
        _ => None,
    }
}

/// Shared player-facing visibility filter (`objective-visibility-policy`, #752).
///
/// A scored objective is shown on a **player-facing** panel (Captain, Comms)
/// when it is a mission objective — always visible regardless of score — or when
/// it is a doctrine objective with a currently positive utility score. Doctrine
/// objectives sitting at score 0 (e.g. an unmet zero-gate) are hidden until
/// conditions or a Captain boost lift them above zero.
///
/// This is deliberately NOT used by the AI-facing pool: the AI keeps zero-score
/// objectives in view and skips them at consumption time (`plan_helm_travel`
/// filters `score > 0.0`), so a boost or a changed condition can re-activate a
/// directive without re-publishing the pool.
pub fn is_visible_objective(o: &ScoredObjective) -> bool {
    o.source == ObjectiveSource::Mission || o.score > 0.0
}

// ── Internal record ────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
struct ObjectiveRecord {
    id: String,
    text: String,
    /// Values interpolated into `text`'s `{placeholder}` tokens on the client.
    /// See `messages::TEXT_PARAMS_SUFFIX`. Empty for every objective that names
    /// a figure-free string.
    text_params: BTreeMap<String, String>,
    mandatory: bool,
    status: ObjectiveStatus,
    targets: Vec<String>,
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

// ── Manager ────────────────────────────────────────────────────────────────

/// Manages the full lifecycle of mission objectives.
#[derive(Clone, Debug, Default)]
pub struct ObjectiveManager {
    objectives: Vec<ObjectiveRecord>,
    dirty: bool,
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
        self.objectives.push(ObjectiveRecord {
            id,
            text: text.into(),
            text_params,
            mandatory,
            status: ObjectiveStatus::Active,
            targets,
            directive,
            utility,
            source,
            command_stance,
        });
        self.dirty = true;
        true
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
            self.dirty = true;
            true
        } else {
            false
        }
    }

    /// Remove the objective with `id` entirely (issue #751).
    ///
    /// Unlike `fail`/`complete` (which transition status but keep the record),
    /// this drops the record so a world layer's objectives disappear when the
    /// layer unloads. Returns `true` if a record was removed.
    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.objectives.len();
        self.objectives.retain(|o| o.id != id);
        let removed = self.objectives.len() != before;
        if removed {
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
        id: r.id.clone(),
        text: r.text.clone(),
        text_params: r.text_params.clone(),
        mandatory: r.mandatory,
        status: r.status.clone(),
        targets: r.targets.clone(),
        source: r.source.clone(),
    }
}

// ── Unit Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── issue #1162: operate-directive relevance + selection ────────────────

    #[test]
    fn tractor_verbs_route_to_engineering_only() {
        for d in [
            AiDirective::Tow { target: "t".into() },
            AiDirective::Stabilise { target: "t".into() },
            AiDirective::Escort { target: "t".into() },
        ] {
            assert_eq!(
                directive_relevance(&d),
                vec![SystemAffinity::Engineering],
                "tractor verbs route to Engineering, the tractor's owner"
            );
        }
    }

    #[test]
    fn transfer_routes_to_both_helm_and_engineering() {
        assert_eq!(
            directive_relevance(&AiDirective::Transfer { target: "t".into() }),
            vec![SystemAffinity::Helm, SystemAffinity::Engineering],
            "Transfer is a two-seat chain: Helm docks, Engineering runs the umbilical"
        );
    }

    #[test]
    fn field_repair_routes_to_repair_only() {
        assert_eq!(
            directive_relevance(&AiDirective::FieldRepair { target: "t".into() }),
            vec![SystemAffinity::Repair],
        );
    }

    fn scored_dir(id: &str, score: f32, directive: AiDirective) -> ScoredObjective {
        let relevance = directive_relevance(&directive);
        ScoredObjective {
            id: id.into(),
            score,
            directive,
            source: ObjectiveSource::Mission,
            relevance,
            snapshot: ObjectiveSnapshot {
                id: id.into(),
                text: String::new(),
                text_params: BTreeMap::new(),
                mandatory: false,
                status: ObjectiveStatus::Active,
                targets: vec![],
                source: ObjectiveSource::Mission,
            },
        }
    }

    #[test]
    fn top_operate_directive_picks_the_first_relevant_positive_match() {
        // Pool is sorted descending by score (as `scored_pool` leaves it).
        let pool = vec![
            scored_dir(
                "tow",
                10.0,
                AiDirective::Tow {
                    target: "hulk".into(),
                },
            ),
            scored_dir(
                "tow2",
                5.0,
                AiDirective::Tow {
                    target: "other".into(),
                },
            ),
        ];
        let picked = top_operate_directive(&pool, SystemAffinity::Engineering, |d| {
            tractor_directive_target(d).is_some()
        });
        assert_eq!(picked.and_then(tractor_directive_target), Some("hulk"));
    }

    #[test]
    fn top_operate_directive_skips_zero_score_and_wrong_affinity() {
        let pool = vec![
            // Zero score — skipped even though the kind matches.
            scored_dir("z", 0.0, AiDirective::Tow { target: "a".into() }),
            // A FieldRepair (Repair affinity) is invisible to an Engineering query.
            scored_dir("fr", 9.0, AiDirective::FieldRepair { target: "b".into() }),
        ];
        assert!(
            top_operate_directive(&pool, SystemAffinity::Engineering, |d| {
                tractor_directive_target(d).is_some()
            })
            .is_none()
        );
        // The FieldRepair IS visible to a Repair query.
        assert_eq!(
            top_operate_directive(&pool, SystemAffinity::Repair, |d| {
                field_repair_directive_target(d).is_some()
            })
            .and_then(field_repair_directive_target),
            Some("b")
        );
    }

    #[test]
    fn transfer_reaches_both_seats_but_not_a_tractor_query() {
        let pool = vec![scored_dir(
            "xf",
            7.0,
            AiDirective::Transfer {
                target: "tender".into(),
            },
        )];
        // Both the Helm dock seat and the Engineering umbilical seat see it.
        assert_eq!(
            top_operate_directive(&pool, SystemAffinity::Helm, |d| {
                transfer_directive_target(d).is_some()
            })
            .and_then(transfer_directive_target),
            Some("tender")
        );
        assert_eq!(
            top_operate_directive(&pool, SystemAffinity::Engineering, |d| {
                transfer_directive_target(d).is_some()
            })
            .and_then(transfer_directive_target),
            Some("tender")
        );
        // But a tractor query (Engineering, tractor verbs) does NOT — Transfer
        // is not a tractor verb, so it never pulls a lock.
        assert!(
            top_operate_directive(&pool, SystemAffinity::Engineering, |d| {
                tractor_directive_target(d).is_some()
            })
            .is_none()
        );
    }

    #[test]
    fn empty_manager_returns_no_snapshots() {
        let mgr = ObjectiveManager::new();
        assert!(mgr.sorted_snapshots().is_empty());
    }

    #[test]
    fn empty_manager_is_not_dirty() {
        let mgr = ObjectiveManager::new();
        assert!(!mgr.is_dirty());
    }

    #[test]
    fn add_objective_appears_in_snapshots_as_active() {
        let mut mgr = ObjectiveManager::new();
        mgr.add("obj-1", "Destroy the convoy", true, vec![]);
        let snapshots = mgr.sorted_snapshots();
        assert_eq!(snapshots.len(), 1);
        assert_eq!(snapshots[0].id, "obj-1");
        assert_eq!(snapshots[0].text, "Destroy the convoy");
        assert!(snapshots[0].mandatory);
        assert_eq!(snapshots[0].status, ObjectiveStatus::Active);
    }

    #[test]
    fn add_objective_marks_dirty() {
        let mut mgr = ObjectiveManager::new();
        mgr.add("obj-1", "Text", false, vec![]);
        assert!(mgr.is_dirty());
    }

    #[test]
    fn mark_clean_clears_dirty_flag() {
        let mut mgr = ObjectiveManager::new();
        mgr.add("obj-1", "Text", false, vec![]);
        mgr.mark_clean();
        assert!(!mgr.is_dirty());
    }

    #[test]
    fn adding_duplicate_id_is_noop() {
        let mut mgr = ObjectiveManager::new();
        let first = mgr.add("obj-1", "First", true, vec![]);
        let second = mgr.add("obj-1", "Second", false, vec![]);
        assert!(first);
        assert!(!second);
        assert_eq!(mgr.sorted_snapshots().len(), 1);
        assert_eq!(mgr.sorted_snapshots()[0].text, "First");
    }

    #[test]
    fn mandatory_objectives_sort_before_optional() {
        let mut mgr = ObjectiveManager::new();
        mgr.add("opt-1", "Optional A", false, vec![]);
        mgr.add("man-1", "Mandatory A", true, vec![]);
        mgr.add("opt-2", "Optional B", false, vec![]);
        mgr.add("man-2", "Mandatory B", true, vec![]);

        let snaps = mgr.sorted_snapshots();
        assert_eq!(snaps.len(), 4);
        assert!(snaps[0].mandatory);
        assert!(snaps[1].mandatory);
        assert!(!snaps[2].mandatory);
        assert!(!snaps[3].mandatory);
        assert_eq!(snaps[0].id, "man-1");
        assert_eq!(snaps[1].id, "man-2");
        assert_eq!(snaps[2].id, "opt-1");
        assert_eq!(snaps[3].id, "opt-2");
    }

    #[test]
    fn complete_transitions_active_to_completed() {
        let mut mgr = ObjectiveManager::new();
        mgr.add("obj-1", "Destroy convoy", true, vec![]);
        mgr.mark_clean();

        let result = mgr.complete("obj-1");
        assert!(result);
        assert_eq!(mgr.sorted_snapshots()[0].status, ObjectiveStatus::Completed);
        assert!(mgr.is_dirty());
    }

    #[test]
    fn complete_returns_false_for_unknown_id() {
        let mut mgr = ObjectiveManager::new();
        let result = mgr.complete("nonexistent");
        assert!(!result);
        assert!(!mgr.is_dirty());
    }

    #[test]
    fn complete_returns_false_if_already_completed() {
        let mut mgr = ObjectiveManager::new();
        mgr.add("obj-1", "Text", true, vec![]);
        mgr.complete("obj-1");
        mgr.mark_clean();

        let result = mgr.complete("obj-1");
        assert!(!result);
        assert!(!mgr.is_dirty());
    }

    #[test]
    fn fail_transitions_active_to_failed() {
        let mut mgr = ObjectiveManager::new();
        mgr.add("obj-1", "Save the station", true, vec![]);
        mgr.mark_clean();

        let result = mgr.fail("obj-1");
        assert!(result);
        assert_eq!(mgr.sorted_snapshots()[0].status, ObjectiveStatus::Failed);
        assert!(mgr.is_dirty());
    }

    #[test]
    fn fail_returns_false_for_unknown_id() {
        let mut mgr = ObjectiveManager::new();
        assert!(!mgr.fail("ghost"));
        assert!(!mgr.is_dirty());
    }

    #[test]
    fn fail_returns_false_if_already_failed() {
        let mut mgr = ObjectiveManager::new();
        mgr.add("obj-1", "Text", true, vec![]);
        mgr.fail("obj-1");
        mgr.mark_clean();
        assert!(!mgr.fail("obj-1"));
        assert!(!mgr.is_dirty());
    }

    #[test]
    fn add_objective_stores_targets() {
        let mut mgr = ObjectiveManager::new();
        mgr.add(
            "obj-1",
            "Destroy Ironveil",
            true,
            vec!["Ironveil".to_string()],
        );
        let snaps = mgr.sorted_snapshots();
        assert_eq!(snaps[0].targets, vec!["Ironveil".to_string()]);
    }

    #[test]
    fn add_objective_stores_multiple_targets() {
        let mut mgr = ObjectiveManager::new();
        mgr.add(
            "obj-1",
            "Hail Axiom Station or Research Outpost",
            false,
            vec!["Axiom Station".to_string(), "Research Outpost".to_string()],
        );
        let snaps = mgr.sorted_snapshots();
        assert_eq!(
            snaps[0].targets,
            vec!["Axiom Station".to_string(), "Research Outpost".to_string()]
        );
    }

    #[test]
    fn add_objective_without_targets_is_empty() {
        let mut mgr = ObjectiveManager::new();
        mgr.add("obj-1", "Survive", true, vec![]);
        let snaps = mgr.sorted_snapshots();
        assert!(snaps[0].targets.is_empty());
    }

    #[test]
    fn add_objective_without_targets_is_empty_vec() {
        let mut mgr = ObjectiveManager::new();
        mgr.add("obj-1", "Survive", true, vec![]);
        let snaps = mgr.sorted_snapshots();
        assert!(snaps[0].targets.is_empty());
    }

    // ── scored_pool tests (issue #571) ─────────────────────────────────────

    #[test]
    fn scored_pool_empty_when_no_objectives() {
        let mgr = ObjectiveManager::new();
        assert!(mgr.scored_pool(&WorldConditions::default()).is_empty());
    }

    #[test]
    fn scored_pool_excludes_completed_and_failed() {
        let mut mgr = ObjectiveManager::new();
        mgr.add("a", "Active", false, vec![]);
        mgr.add("b", "Done", false, vec![]);
        mgr.complete("b");
        mgr.add("c", "Failed", false, vec![]);
        mgr.fail("c");
        let pool = mgr.scored_pool(&WorldConditions::default());
        assert_eq!(pool.len(), 1);
        assert_eq!(pool[0].id, "a");
    }

    #[test]
    fn scored_pool_base_priority_is_score() {
        let mut mgr = ObjectiveManager::new();
        mgr.add_full(
            "obj-1",
            "Patrol",
            false,
            vec![],
            AiDirective::Patrol {
                anchors: vec!["alpha".into()],
                loop_path: true,
            },
            UtilityConfig {
                base_priority: 40.0,
                ..Default::default()
            },
            ObjectiveSource::Mission,
        );
        let pool = mgr.scored_pool(&WorldConditions::default());
        assert!((pool[0].score - 40.0).abs() < f32::EPSILON);
    }

    #[test]
    fn mandatory_bonus_added_to_score() {
        let mut mgr = ObjectiveManager::new();
        mgr.add_full(
            "obj-1",
            "Mandatory patrol",
            true,
            vec![],
            AiDirective::default(),
            UtilityConfig {
                base_priority: 30.0,
                ..Default::default()
            },
            ObjectiveSource::Mission,
        );
        let pool = mgr.scored_pool(&WorldConditions::default());
        assert!((pool[0].score - (30.0 + MANDATORY_BONUS)).abs() < f32::EPSILON);
    }

    #[test]
    fn zero_gate_forces_score_to_zero_when_condition_false() {
        let mut mgr = ObjectiveManager::new();
        mgr.add_full(
            "obj-1",
            "Flee",
            false,
            vec![],
            AiDirective::default(),
            UtilityConfig {
                base_priority: 80.0,
                zero_gates: vec![ZeroGateCondition {
                    condition: "hull_below".into(),
                    threshold: Some(0.3),
                }],
                ..Default::default()
            },
            ObjectiveSource::Doctrine,
        );
        // Hull is 1.0 → hull_below(0.3) is false → gate fails → score = 0
        let pool = mgr.scored_pool(&WorldConditions {
            hull_fraction: 1.0,
            ..Default::default()
        });
        assert_eq!(pool[0].score, 0.0);
    }

    #[test]
    fn zero_gate_passes_when_condition_true() {
        let mut mgr = ObjectiveManager::new();
        mgr.add_full(
            "obj-1",
            "Flee",
            false,
            vec![],
            AiDirective::default(),
            UtilityConfig {
                base_priority: 80.0,
                zero_gates: vec![ZeroGateCondition {
                    condition: "hull_below".into(),
                    threshold: Some(0.3),
                }],
                ..Default::default()
            },
            ObjectiveSource::Doctrine,
        );
        // Hull is 0.2 → hull_below(0.3) is true → gate passes → full score
        let pool = mgr.scored_pool(&WorldConditions {
            hull_fraction: 0.2,
            ..Default::default()
        });
        assert!((pool[0].score - 80.0).abs() < f32::EPSILON);
    }

    #[test]
    fn modifier_adds_weight_when_condition_true() {
        let mut mgr = ObjectiveManager::new();
        mgr.add_full(
            "obj-1",
            "Attack",
            false,
            vec![],
            AiDirective::Destroy {
                target: "enemy".into(),
            },
            UtilityConfig {
                base_priority: 50.0,
                modifiers: vec![ConditionModifier {
                    condition: "red_alert".into(),
                    threshold: None,
                    weight: 20.0,
                }],
                ..Default::default()
            },
            ObjectiveSource::Doctrine,
        );
        let pool = mgr.scored_pool(&WorldConditions {
            red_alert: true,
            hull_fraction: 1.0,
            attacked: false,
        });
        assert!((pool[0].score - 70.0).abs() < f32::EPSILON);
    }

    #[test]
    fn modifier_skipped_when_condition_false() {
        let mut mgr = ObjectiveManager::new();
        mgr.add_full(
            "obj-1",
            "Attack",
            false,
            vec![],
            AiDirective::Destroy {
                target: "enemy".into(),
            },
            UtilityConfig {
                base_priority: 50.0,
                modifiers: vec![ConditionModifier {
                    condition: "red_alert".into(),
                    threshold: None,
                    weight: 20.0,
                }],
                ..Default::default()
            },
            ObjectiveSource::Doctrine,
        );
        let pool = mgr.scored_pool(&WorldConditions {
            red_alert: false,
            hull_fraction: 1.0,
            attacked: false,
        });
        assert!((pool[0].score - 50.0).abs() < f32::EPSILON);
    }

    #[test]
    fn scored_pool_sorted_descending_by_score() {
        let mut mgr = ObjectiveManager::new();
        mgr.add_full(
            "low",
            "Low priority",
            false,
            vec![],
            AiDirective::default(),
            UtilityConfig {
                base_priority: 10.0,
                ..Default::default()
            },
            ObjectiveSource::Doctrine,
        );
        mgr.add_full(
            "high",
            "High priority",
            false,
            vec![],
            AiDirective::default(),
            UtilityConfig {
                base_priority: 60.0,
                ..Default::default()
            },
            ObjectiveSource::Mission,
        );
        let pool = mgr.scored_pool(&WorldConditions::default());
        assert_eq!(pool[0].id, "high");
        assert_eq!(pool[1].id, "low");
    }

    #[test]
    fn captain_priority_selection_outranks_a_higher_authored_score() {
        let mut mgr = ObjectiveManager::new();
        mgr.add_full(
            "patrol",
            "Patrol",
            false,
            vec![],
            AiDirective::Patrol {
                anchors: vec!["alpha".into()],
                loop_path: true,
            },
            UtilityConfig {
                base_priority: 90.0,
                ..Default::default()
            },
            ObjectiveSource::Mission,
        );
        mgr.add_full(
            "destroy-priority",
            "Destroy priority target",
            false,
            vec!["priority-target".into()],
            AiDirective::Destroy {
                target: "priority-target".into(),
            },
            UtilityConfig {
                base_priority: 10.0,
                ..Default::default()
            },
            ObjectiveSource::Mission,
        );

        let pool =
            mgr.scored_pool_with_boost(&WorldConditions::default(), Some("destroy-priority"));

        assert_eq!(pool[0].id, "destroy-priority");
        assert_eq!(pool[0].score, f32::MAX);
    }

    #[test]
    fn patrol_directive_has_helm_relevance() {
        let mut mgr = ObjectiveManager::new();
        mgr.add_full(
            "p",
            "Patrol",
            false,
            vec![],
            AiDirective::Patrol {
                anchors: vec!["a".into()],
                loop_path: false,
            },
            UtilityConfig {
                base_priority: 1.0,
                ..Default::default()
            },
            ObjectiveSource::Mission,
        );
        let pool = mgr.scored_pool(&WorldConditions::default());
        assert_eq!(pool[0].relevance, vec![SystemAffinity::Helm]);
    }

    #[test]
    fn destroy_directive_has_helm_weapons_captain_relevance() {
        let mut mgr = ObjectiveManager::new();
        mgr.add_full(
            "d",
            "Destroy",
            false,
            vec![],
            AiDirective::Destroy {
                target: "target".into(),
            },
            UtilityConfig {
                base_priority: 1.0,
                ..Default::default()
            },
            ObjectiveSource::Mission,
        );
        let pool = mgr.scored_pool(&WorldConditions::default());
        assert_eq!(
            pool[0].relevance,
            vec![
                SystemAffinity::Helm,
                SystemAffinity::Weapons,
                SystemAffinity::Captain
            ]
        );
    }

    #[test]
    fn none_directive_has_no_relevance() {
        let mut mgr = ObjectiveManager::new();
        mgr.add("obj", "No directive", false, vec![]);
        let pool = mgr.scored_pool(&WorldConditions::default());
        assert!(pool[0].relevance.is_empty());
    }

    #[test]
    fn hail_directive_has_comms_relevance() {
        // Issue #753: Hail is a Comms action, so the Backfill Comms AI can
        // consume it from the scored pool by affinity.
        let mut mgr = ObjectiveManager::new();
        mgr.add_full(
            "h",
            "Hail",
            false,
            vec![],
            AiDirective::Hail {
                target: "Station Alpha".into(),
            },
            UtilityConfig {
                base_priority: 1.0,
                ..Default::default()
            },
            ObjectiveSource::Mission,
        );
        let pool = mgr.scored_pool(&WorldConditions::default());
        assert_eq!(pool[0].relevance, vec![SystemAffinity::Comms]);
    }

    // ── remove clears runtime record (issue #751/#752) ─────────────────────

    #[test]
    fn remove_drops_record_and_marks_dirty() {
        let mut mgr = ObjectiveManager::new();
        mgr.add("obj-1", "Text", true, vec![]);
        mgr.mark_clean();
        assert!(mgr.remove("obj-1"));
        assert!(mgr.sorted_snapshots().is_empty());
        assert!(mgr.scored_pool(&WorldConditions::default()).is_empty());
        assert!(mgr.is_dirty());
    }

    #[test]
    fn remove_unknown_id_is_noop() {
        let mut mgr = ObjectiveManager::new();
        mgr.add("obj-1", "Text", true, vec![]);
        mgr.mark_clean();
        assert!(!mgr.remove("ghost"));
        assert!(!mgr.is_dirty());
    }

    #[test]
    fn removed_id_can_be_re_added_fresh() {
        // After removal the id is free again: a re-add is a genuine insert, not
        // a dedup no-op — so an unloaded-then-reloaded layer re-registers its
        // objective cleanly (#752 lifecycle).
        let mut mgr = ObjectiveManager::new();
        mgr.add("obj-1", "First", true, vec![]);
        mgr.remove("obj-1");
        assert!(mgr.add("obj-1", "Second", false, vec![]));
        let snaps = mgr.sorted_snapshots();
        assert_eq!(snaps.len(), 1);
        assert_eq!(snaps[0].text, "Second");
    }

    // ── objective-contributed Command stances (issue #1110) ────────────────

    fn objective_stance() -> (StationId, StationStanceConfig) {
        (
            StationId("tactical".into()),
            StationStanceConfig {
                id: "objective-escort".into(),
                label: String::new(),
                kind: crate::ship::config::StanceKind::Standard,
                high_alert: true,
                persist_behind_human: true,
                ai_engaged: false,
            },
        )
    }

    fn add_with_stance(
        mgr: &mut ObjectiveManager,
        id: &str,
        stance: Option<(StationId, StationStanceConfig)>,
    ) {
        mgr.add_full_with_params(
            id,
            "text",
            BTreeMap::new(),
            false,
            vec![],
            AiDirective::None,
            UtilityConfig::default(),
            ObjectiveSource::Mission,
            stance,
        );
    }

    #[test]
    fn a_contributed_stance_is_exposed_only_while_active() {
        let mut mgr = ObjectiveManager::new();
        add_with_stance(&mut mgr, "escort", Some(objective_stance()));
        // Active → the contribution is exposed against its target Station.
        assert_eq!(mgr.active_station_stances(), vec![objective_stance()]);
    }

    #[test]
    fn an_objective_without_a_stance_contributes_nothing() {
        let mut mgr = ObjectiveManager::new();
        add_with_stance(&mut mgr, "plain", None);
        assert!(mgr.active_station_stances().is_empty());
    }

    #[test]
    fn completing_the_objective_withdraws_its_contribution() {
        let mut mgr = ObjectiveManager::new();
        add_with_stance(&mut mgr, "escort", Some(objective_stance()));
        assert!(mgr.complete("escort"));
        assert!(
            mgr.active_station_stances().is_empty(),
            "a completed objective no longer contributes its stance",
        );
    }

    #[test]
    fn failing_the_objective_withdraws_its_contribution() {
        let mut mgr = ObjectiveManager::new();
        add_with_stance(&mut mgr, "escort", Some(objective_stance()));
        assert!(mgr.fail("escort"));
        assert!(
            mgr.active_station_stances().is_empty(),
            "a failed objective no longer contributes its stance",
        );
    }

    #[test]
    fn removing_the_objective_withdraws_its_contribution() {
        // Invalidation (`remove`) deletes the record outright, so its
        // contribution disappears the same way completion/failure withdraw it.
        let mut mgr = ObjectiveManager::new();
        add_with_stance(&mut mgr, "escort", Some(objective_stance()));
        assert!(mgr.remove("escort"));
        assert!(mgr.active_station_stances().is_empty());
    }

    // ── is_visible_objective (objective-visibility-policy, #752) ────────────

    fn scored(id: &str, source: ObjectiveSource, base: f32) -> ScoredObjective {
        let mut mgr = ObjectiveManager::new();
        mgr.add_full(
            id,
            "text",
            false,
            vec![],
            AiDirective::None,
            UtilityConfig {
                base_priority: base,
                ..Default::default()
            },
            source,
        );
        mgr.scored_pool(&WorldConditions::default())
            .into_iter()
            .next()
            .unwrap()
    }

    #[test]
    fn mission_objective_is_visible_even_at_zero_score() {
        let o = scored("m", ObjectiveSource::Mission, 0.0);
        assert_eq!(o.score, 0.0);
        assert!(is_visible_objective(&o));
    }

    #[test]
    fn doctrine_objective_hidden_at_zero_score_visible_when_positive() {
        let zero = scored("d0", ObjectiveSource::Doctrine, 0.0);
        assert!(!is_visible_objective(&zero));
        let positive = scored("d1", ObjectiveSource::Doctrine, 5.0);
        assert!(is_visible_objective(&positive));
    }

    #[test]
    fn scored_pool_is_total_ordered_on_equal_scores_by_insertion() {
        // Equal scores keep insertion order under the stable `total_cmp` sort,
        // giving a deterministic tiebreak the consumers rely on.
        let mut mgr = ObjectiveManager::new();
        mgr.add("first", "A", false, vec![]);
        mgr.add("second", "B", false, vec![]);
        let pool = mgr.scored_pool(&WorldConditions::default());
        assert_eq!(pool[0].id, "first");
        assert_eq!(pool[1].id, "second");
    }

    // ── attacked_recently (issue #1010) ────────────────────────────────────

    /// The boundary both publish sites share. It lives here, on the pure
    /// function, so the NPC aggregator and the LocalShip publisher cannot
    /// drift apart on where the window ends: they call this, and this is what
    /// is pinned.
    #[test]
    fn attacked_recency_window_opens_and_closes_on_the_boundary() {
        // Never damaged: nothing to decay from, so never under attack.
        assert!(!attacked_recently(None, 100.0, 8.0));
        // The hit itself, and everything strictly inside the window.
        assert!(attacked_recently(Some(100.0), 100.0, 8.0));
        assert!(attacked_recently(Some(100.0), 107.9, 8.0));
        // The far edge: at exactly the window the memory has expired, so the
        // raid resumes rather than hanging on for one more tick.
        assert!(!attacked_recently(Some(100.0), 108.0, 8.0));
        assert!(!attacked_recently(Some(100.0), 200.0, 8.0));
    }

    /// A designer authoring a zero (or negative) window means "no memory at
    /// all" — a hit must not read as an attack even on the tick it lands.
    #[test]
    fn a_non_positive_attacked_window_never_reads_as_attacked() {
        assert!(!attacked_recently(Some(100.0), 100.0, 0.0));
        assert!(!attacked_recently(Some(100.0), 100.0, -1.0));
    }

    /// Shield-absorbed fire IS being attacked. `last_damage_taken` only moves
    /// when the hull total drops, and shield-absorbed fire dominates early;
    /// hull damage lands only after sustained pressure collapses an arc — so a
    /// fold over hull damage alone would leave a Harrow under sustained
    /// station fire reading "not attacked" for most of a short engagement and
    /// flying its raid into the guns.
    #[test]
    fn shield_absorbed_hostile_fire_counts_as_a_landed_hit() {
        assert_eq!(last_landed_hit_secs(None, Some(100.0)), Some(100.0));
        assert!(attacked_recently(
            last_landed_hit_secs(None, Some(100.0)),
            103.0,
            8.0
        ));
    }

    /// The fold takes the MORE RECENT of the two readings either way round, so
    /// a hull hit long ago and skimmed a moment ago still reads as under
    /// attack (and not the other way about).
    #[test]
    fn the_landed_hit_fold_takes_the_more_recent_reading() {
        assert_eq!(last_landed_hit_secs(None, None), None);
        assert_eq!(last_landed_hit_secs(Some(100.0), None), Some(100.0));
        assert_eq!(last_landed_hit_secs(Some(100.0), Some(140.0)), Some(140.0));
        assert_eq!(last_landed_hit_secs(Some(140.0), Some(100.0)), Some(140.0));

        // Hull damage at 100, shields skimmed at 140, now 145: the stale hull
        // reading must not expire the window the fresh one keeps open.
        assert!(attacked_recently(
            last_landed_hit_secs(Some(100.0), Some(140.0)),
            145.0,
            8.0
        ));
    }
}
