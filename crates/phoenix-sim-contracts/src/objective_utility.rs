//! Shared directive utility over borrowed facts; no Objective storage.
use phoenix_model::messages::{AiDirective, SystemAffinity};

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
pub const MANDATORY_BONUS: f32 = 10.0;

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
