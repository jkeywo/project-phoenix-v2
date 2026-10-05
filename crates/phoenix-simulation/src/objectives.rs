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
//   - `ObjectiveManager::drain_transitions` — the ordered per-tick log of every
//     mutation the mutators above made, for the mission-timeline recorder (#1338)

use crate::core::messages::{
    AiDirective, ObjectiveSnapshot, ObjectiveSource, ObjectiveStatus, ScoredObjective, StationId,
    SystemAffinity,
};
use crate::ship::config::StationStanceConfig;
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

/// Canonical authoring vocabulary and validation for objective Directives.
/// Entity doctrine and World actions keep their existing TOML field names, but
/// both adapt into this one typed contract before a runtime `AiDirective` is
/// built (issue #1268).
pub mod directive;

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
        // A civilian order belongs to Navigation. It is not a Helm travel goal:
        // the ordered craft's own doctrine and helm fly the resulting route.
        AiDirective::Order { .. } => vec![SystemAffinity::Navigation],
        // Scan is a Sensors action (issue #1139). The Backfill Sensors host
        // consumes it and emits the ordinary admitted ScanTarget command; no
        // other affinity gets the directive and the scan applier stays shared.
        AiDirective::Scan { .. } => vec![SystemAffinity::Sensors],
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
        // Secure routes to Security, which owns the teams (issue #1346). Its own
        // affinity rather than Weapons': which station owns the Security System is
        // a hull's authoring decision, so the directive names the system and the
        // seat that holds it decides the concrete `DispatchSecurityTeam`.
        // Deliberately NOT a Helm or Weapons goal — a team crossing to a burning
        // compartment is not an acquisition.
        AiDirective::Secure { .. } => vec![SystemAffinity::Security],
        // Rescue routes to Engineering, which owns the transporter (issue #1348).
        // Deliberately NOT the Weapons Tactical selector like the tractor verbs:
        // the transporter names its OWN discovered contact rather than resolving
        // through the combat lock, so a rescue never pulls a weapons lock onto
        // the civilians it is saving.
        AiDirective::Rescue { .. } => vec![SystemAffinity::Engineering],
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

/// The target a `Rescue` directive names, or `None` for any other directive
/// (issue #1348). The transporter host's own-kind test, factored out so the host
/// and its tests read one rule. The tractor-versus-rescue precedence is settled
/// through [`engineering_seat_operate_target`] (the shared seat), not here.
pub fn rescue_directive_target(directive: &AiDirective) -> Option<&str> {
    match directive {
        AiDirective::Rescue { target } => Some(target.as_str()),
        _ => None,
    }
}

/// The target of any directive that occupies the single Engineering backfill
/// seat — a tractor verb (`Tow`/`Stabilise`/`Escort`) or the transporter's
/// `Rescue` — or `None` for anything else (issue #1348).
///
/// The tractor and the rescue transporter are distinct systems but share one
/// Engineering seat: one pair of hands. Both backfill hosts pass THIS predicate
/// to [`top_operate_directive`], so they rank the tractor and rescue orders
/// against the ONE scored pool and the seat resolves to a single winner. Each
/// host then keeps only its own kind of that winner (via
/// [`tractor_directive_target`] / [`rescue_directive_target`]): when a tractor
/// obligation outscores a rescue the transporter host sees `None` and stands
/// down, and when a rescue outscores the tractor the tractor host stands down —
/// so a higher-scored life-saving stabilisation defers the rescue exactly as the
/// acceptance criterion requires, and neither ever runs while the other holds the
/// seat.
pub fn engineering_seat_operate_target(directive: &AiDirective) -> Option<&str> {
    tractor_directive_target(directive).or_else(|| rescue_directive_target(directive))
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

/// The target a `Secure` directive names, or `None` (issue #1346). The Security
/// backfill host's predicate: a target it names has its authored Security work
/// promoted to `urgent_objective` in the host's ranking.
pub fn secure_directive_target(directive: &AiDirective) -> Option<&str> {
    match directive {
        AiDirective::Secure { target } => Some(target.as_str()),
        _ => None,
    }
}

/// The civilian and route named by an `Order` directive, or `None` for any
/// other directive (issue #1141). Kept pure so authoring, selection and the
/// Navigation host share the same payload projection.
pub fn order_directive(directive: &AiDirective) -> Option<(&str, &str)> {
    match directive {
        AiDirective::Order { target, route } => Some((target.as_str(), route.as_str())),
        _ => None,
    }
}

/// The target a `Scan` directive names, or `None` for another directive
/// (issue #1139). Shared by the Sensors host and its pure selection tests.
pub fn scan_directive_target(directive: &AiDirective) -> Option<&str> {
    match directive {
        AiDirective::Scan { target } => Some(target.as_str()),
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

// ── Transition log (issue #1338) ───────────────────────────────────────────

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

// ── Manager ────────────────────────────────────────────────────────────────

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
    pub(crate) fn take_restored_statuses(&mut self) -> Option<BTreeMap<String, ObjectiveStatus>> {
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

// ── Unit Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "objectives_tests.rs"]
mod tests;
