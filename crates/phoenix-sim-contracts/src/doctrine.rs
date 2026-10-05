//! Authored standing directives shared by hulls and mission palettes.
use serde::{Deserialize, Serialize};

/// A single standing-doctrine objective declared in an entity's `[behaviour]` block.
///
/// Doctrine objectives replace the old FSM state/transition model: each entry
/// carries a typed `AiDirective`, a utility score (base priority + modifiers +
/// zero-gates), and an optional target speed for the helm to use when executing
/// the directive. The viewscreen aggregator scores these the same way it scores
/// mission objectives; per-system operate functions select the top-scoring
/// directive they can serve.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct DoctrineObjective {
    /// Stable identifier (e.g. `"patrol-sector"`, `"destroy-hostiles"`).
    pub id: String,
    /// Human-readable prose shown on the captain panel when active.
    ///
    /// Defaulted (issue #838): a doctrine directive authored *inline* — as a
    /// world `spawn_entity` override rather than in a ship template — routinely
    /// omits display prose (every wave in `combat_test.toml` does). Before this
    /// was `#[serde(default)]`, such an override reparsed with a "missing field
    /// `text`" error inside `dispatch_spawn_entity`'s round-trip (step 2b), and
    /// the failure was silently swallowed — discarding the *entire* override,
    /// faction and behaviour alike, and leaving the raw template. A hull with no
    /// template `[behaviour]` (e.g. `alliance_destroyer`) was left inert and,
    /// worse, kept its template faction, so a world-spawned "hostile" was
    /// neither hostile nor armed. An empty `text` is already a tested value in
    /// `score_doctrine_pool`; the captain panel simply shows no prose for it.
    #[serde(default)]
    pub text: String,
    /// Whether this objective blocks mission completion when active (usually `false` for doctrine).
    #[serde(default)]
    pub mandatory: bool,
    /// Directive kind: `"Patrol"`, `"Destroy"`, `"Reach"`, `"Retreat"`,
    /// `"Hail"`, `"Scan"`, `"Dock"`, `"Tow"`, `"Stabilise"`, `"Escort"`,
    /// `"Transfer"`, `"FieldRepair"`, `"Secure"`, `"Order"`, or absent for
    /// `None`.
    #[serde(default)]
    pub directive_kind: Option<String>,
    /// Anchor names for `Patrol` directives.
    #[serde(default)]
    pub directive_anchors: Vec<String>,
    /// Whether the patrol loops back to the first anchor after the last.
    #[serde(default)]
    pub directive_loop: bool,
    /// Named target for `Destroy` directives. Resolved by `ai_target_selection`
    /// (tier 1) onto the ship's `TacticalRadarSelection`, which is what the Helm and the
    /// firing systems then read.
    #[serde(default)]
    pub directive_target: Option<String>,
    /// Named anchor for `Reach` and `Retreat` directives.
    #[serde(default)]
    pub directive_anchor: Option<String>,
    /// Named structure for `Dock` directives (issue #1028): the authored world
    /// entity name of the thing to berth at. Resolved to a live UUID by the same
    /// `resolve_destroy_target` a `Destroy` target goes through, so it accepts a
    /// UUID too.
    #[serde(default)]
    pub directive_dock_target: Option<String>,
    /// Named target for `Hail` directives.
    #[serde(default)]
    pub directive_hail_target: Option<String>,
    /// Named target for `Scan` directives (issue #1139). Kept distinct from the
    /// Destroy and Hail fields so doctrine field-ownership mistakes fail at
    /// load rather than silently resolving to no scan subject.
    #[serde(default)]
    pub directive_scan_target: Option<String>,
    /// Named target for the operate verbs — `Tow`, `Stabilise`, `Escort`,
    /// `Transfer` and `FieldRepair` (issue #1162), plus `Secure` (issue #1346).
    /// One shared field for all six,
    /// the way `Reach`/`Retreat` share `directive_anchor`: each verb names a
    /// target the owning seat operates on, resolved to a live UUID the same way
    /// a `Destroy`/`Dock` target is.
    #[serde(default)]
    pub directive_operate_target: Option<String>,
    /// Named civilian and authored route for an `Order` directive (#1141).
    /// Both are required: Navigation emits a route divert for this target.
    #[serde(default)]
    pub directive_order_target: Option<String>,
    #[serde(default)]
    pub directive_order_route: Option<String>,
    /// Keys serde did not recognise on this doctrine entry. The shared
    /// Directive contract rejects them after deserialization so both doctrine
    /// and World actions fail loudly instead of silently discarding a typo.
    #[serde(default, flatten)]
    pub unrecognised_fields: std::collections::BTreeMap<String, toml::Value>,
    /// Base utility score before modifiers.
    #[serde(default)]
    pub base_priority: f32,
    /// Veto conditions — force score to 0 when the condition evaluates to false.
    #[serde(default)]
    pub zero_gates: Vec<crate::objective_utility::ZeroGateCondition>,
    /// Additive score modifiers applied when their condition is true.
    #[serde(default)]
    pub modifiers: Vec<crate::objective_utility::ConditionModifier>,
    /// Desired helm speed fraction [0, 1] when executing this directive.
    #[serde(default = "default_doctrine_target_speed")]
    pub target_speed: f32,
    /// Distance to maintain from the target (world units) for Destroy directives.
    /// The helm stops thrusting when closer than this.
    #[serde(default = "default_maintain_range")]
    pub maintain_range: f32,
    /// Whether the AI may engage impulse drive while executing this objective.
    /// When absent, defaults from the interpreted runtime Directive: `false`
    /// for Patrol and `true` for every other kind.
    #[serde(default)]
    pub use_impulse: Option<bool>,
}

impl DoctrineObjective {
    /// Resolved effective `use_impulse` value.
    /// Returns `self.use_impulse` if set; otherwise defaults to `false` for the
    /// already-interpreted Patrol directive and `true` for every other runtime
    /// directive. The raw authoring-kind catalogue stays owned by the shared
    /// Directive interpreter.
    pub fn effective_use_impulse(&self, directive: &phoenix_model::messages::AiDirective) -> bool {
        self.use_impulse.unwrap_or(!matches!(
            directive,
            phoenix_model::messages::AiDirective::Patrol { .. }
        ))
    }
}

/// Adapt the unchanged entity-doctrine TOML shape into the one typed Directive
/// contract shared with World `add_objective` actions (issue #1268).
pub fn authored_doctrine_directive(
    doctrine: &DoctrineObjective,
) -> crate::directive::AuthoredDirective {
    use crate::directive::{AuthoredDirective, DirectiveField};

    let mut authored = AuthoredDirective::new(doctrine.directive_kind.clone());
    authored.push_texts(
        DirectiveField::PatrolAnchors,
        doctrine.directive_anchors.clone(),
    );
    authored.push_bool(DirectiveField::PatrolLoop, doctrine.directive_loop);
    authored.push_text(
        DirectiveField::DoctrineDestroyTarget,
        doctrine.directive_target.clone(),
    );
    authored.push_text(DirectiveField::Anchor, doctrine.directive_anchor.clone());
    authored.push_text(
        DirectiveField::DoctrineHailTarget,
        doctrine.directive_hail_target.clone(),
    );
    authored.push_text(
        DirectiveField::DoctrineScanTarget,
        doctrine.directive_scan_target.clone(),
    );
    authored.push_text(
        DirectiveField::DoctrineDockTarget,
        doctrine.directive_dock_target.clone(),
    );
    authored.push_text(
        DirectiveField::DoctrineOperateTarget,
        doctrine.directive_operate_target.clone(),
    );
    authored.push_text(
        DirectiveField::DoctrineOrderTarget,
        doctrine.directive_order_target.clone(),
    );
    authored.push_text(
        DirectiveField::DoctrineOrderRoute,
        doctrine.directive_order_route.clone(),
    );
    authored.push_unknown_fields(doctrine.unrecognised_fields.keys().cloned());
    authored
}

/// Reject invalid doctrine through the shared Directive contract. The adapter
/// above owns only field projection; kind vocabulary, field ownership,
/// requirements, defaults and runtime conversion all live in
/// `objectives::directive`.
pub fn validate_doctrine_directives(doctrine: &[DoctrineObjective]) -> Result<(), String> {
    for entry in doctrine {
        crate::directive::interpret(&authored_doctrine_directive(entry)).map_err(|error| {
            format!(
                "doctrine '{}': {}",
                entry.id,
                error.describe(crate::directive::DirectiveSurface::Doctrine)
            )
        })?;
    }
    Ok(())
}

fn default_doctrine_target_speed() -> f32 {
    0.8
}

fn default_maintain_range() -> f32 {
    25.0
}
