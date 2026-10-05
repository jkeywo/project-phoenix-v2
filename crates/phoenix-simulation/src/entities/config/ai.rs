//! Entity schema: ai. Public paths remain in the parent module.
use super::*;

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
    pub(crate) unrecognised_fields: std::collections::BTreeMap<String, toml::Value>,
    /// Base utility score before modifiers.
    #[serde(default)]
    pub base_priority: f32,
    /// Veto conditions — force score to 0 when the condition evaluates to false.
    #[serde(default)]
    pub zero_gates: Vec<crate::objectives::ZeroGateCondition>,
    /// Additive score modifiers applied when their condition is true.
    #[serde(default)]
    pub modifiers: Vec<crate::objectives::ConditionModifier>,
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
    pub fn effective_use_impulse(&self, directive: &crate::core::messages::AiDirective) -> bool {
        self.use_impulse.unwrap_or(!matches!(
            directive,
            crate::core::messages::AiDirective::Patrol { .. }
        ))
    }
}

/// Adapt the unchanged entity-doctrine TOML shape into the one typed Directive
/// contract shared with World `add_objective` actions (issue #1268).
pub fn authored_doctrine_directive(
    doctrine: &DoctrineObjective,
) -> crate::objectives::directive::AuthoredDirective {
    use crate::objectives::directive::{AuthoredDirective, DirectiveField};

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
        crate::objectives::directive::interpret(&authored_doctrine_directive(entry)).map_err(
            |error| {
                format!(
                    "doctrine '{}': {}",
                    entry.id,
                    error.describe(crate::objectives::directive::DirectiveSurface::Doctrine)
                )
            },
        )?;
    }
    Ok(())
}

fn default_doctrine_target_speed() -> f32 {
    0.8
}

fn default_maintain_range() -> f32 {
    25.0
}

/// Configuration for an AI behaviour controller attached to an entity.
///
/// The FSM (AiState/TransitionConfig) is dissolved in issue #572. Behaviour is
/// now driven by a list of `DoctrineObjective`s scored by the viewscreen
/// aggregator and interpreted per-system via operate functions.
/// AI profile section: aggression and sensor range for NPC ship AI.
/// Maps to [ai_profile] in entity TOML.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AiProfileConfig {
    pub aggression: f32,
    pub sensor_range: f32,
    /// Low-LOD dead-reckoning fallback (issue #933): fraction of this hull's
    /// authored `max_speed` that a demoted ship's frozen exit speed decays
    /// toward when it has no route to steer by. Defaults to a sane non-zero
    /// cruise fraction rather than `0.0` — the issue's intent is that the
    /// decay is *on* out of the box, not an opt-in a designer has to remember.
    #[serde(default = "default_low_lod_cruise_fraction")]
    pub low_lod_cruise_fraction: f32,
    /// Rate (world-units/s²) at which the low-LOD fallback moves a ship's
    /// speed toward `low_lod_cruise_fraction * max_speed`. Bidirectional
    /// despite the name: it is the *decay* rate for a hull demoted mid-boost
    /// (issue #933) and equally the *ramp* rate for a parked hull getting under
    /// way again when a completed route diverts it onto a scored `Destroy`
    /// (issue #1012). Only route-following acceleration uses a different rate
    /// (`LOW_LOD_ACCEL_PER_SEC` in `ai::server`, a fixed 10 u/s²).
    #[serde(default = "default_low_lod_speed_decay_per_sec")]
    pub low_lod_speed_decay_per_sec: f32,
    /// Fraction of this hull's authored `max_yaw_rate` the low-LOD fallback
    /// may spend turning a standing `Destroy` directive's dead-reckoned
    /// heading back toward its named target, once that target resolves in
    /// the (possibly stale) `WorldSnapshot`. Issue #933.
    #[serde(default = "default_low_lod_turn_rate_fraction")]
    pub low_lod_turn_rate_fraction: f32,
}

/// See [`AiProfileConfig::low_lod_cruise_fraction`].
pub(crate) fn default_low_lod_cruise_fraction() -> f32 {
    0.5
}

/// See [`AiProfileConfig::low_lod_speed_decay_per_sec`].
pub(crate) fn default_low_lod_speed_decay_per_sec() -> f32 {
    8.0
}

/// See [`AiProfileConfig::low_lod_turn_rate_fraction`].
pub(crate) fn default_low_lod_turn_rate_fraction() -> f32 {
    0.5
}

/// High-fidelity bubble section: `[lod_bubble] radius = N`. An entity carrying
/// one projects a zone inside which NPCs stay promoted to full-fidelity AI, and
/// is itself always full-fidelity. See [`crate::ai::server::LodBubble`]. A player
/// hull may author one to size its zone; a stationary defended object (the
/// station) authors a smaller one so the raid sieging it runs in full even when
/// the player is elsewhere.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LodBubbleConfig {
    /// Bubble radius in world units.
    pub radius: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BehaviourConfig {
    /// Standing-doctrine objectives for this entity template.
    #[serde(default)]
    pub doctrine: Vec<DoctrineObjective>,
    /// Arrival radius in world units — closer than this counts as "reached waypoint".
    /// Defaults to [`crate::ai::WAYPOINT_ARRIVAL_RADIUS`] when absent.
    #[serde(default = "default_waypoint_arrival_radius")]
    pub waypoint_arrival_radius: f32,
    /// Extra clearance (world units) added on top of radii for collision avoidance.
    /// Defaults to [`crate::ai::AVOIDANCE_BUFFER`] when absent.
    #[serde(default = "default_avoidance_buffer")]
    pub avoidance_buffer: f32,
    /// Look-ahead horizon (seconds) for predictive collision avoidance.
    /// Defaults to [`crate::ai::AVOIDANCE_LOOK_AHEAD_SECS`] when absent.
    #[serde(default = "default_avoidance_look_ahead_secs")]
    pub avoidance_look_ahead_secs: f32,
    /// Speed fraction [0, 1] for the Channel-3 Navigation→Helm handoff
    /// fallthrough, used when no local Helm-relevant objective resolves but
    /// Navigation has set a waypoint the Helm is cleared to follow.
    /// Defaults to [`crate::ai::NAV_HANDOFF_SPEED`] when absent.
    #[serde(default = "default_nav_handoff_speed")]
    pub nav_handoff_speed: f32,
    /// Distance (world units) within which a docking intent switches from
    /// normal objective approach to the close-quarters docking manoeuvre
    /// (controlled reverse / lateral translation). Issue #742.
    /// Defaults to [`crate::ai::DOCKING_ENGAGE_DISTANCE`] when absent.
    #[serde(default = "default_docking_engage_distance")]
    pub docking_engage_distance: f32,
    /// Speed fraction `[0, 1]` capping the low-speed reverse / lateral
    /// translation of a docking close manoeuvre. Issue #742.
    /// Defaults to [`crate::ai::DOCKING_APPROACH_SPEED`] when absent.
    #[serde(default = "default_docking_approach_speed")]
    pub docking_approach_speed: f32,
    /// Authored ignore-smaller rule (issue #743): the shared hazard assessment
    /// skips a MOBILE hazard whose `size_rating` is below this ship's own scaled
    /// by this ratio. `0.0` (the default) disables the rule so every dangerous
    /// hazard is assessed; `1.0` ignores any *ship* strictly smaller than self.
    ///
    /// Never applies to static terrain (issue #958): an asteroid, station or
    /// planet is avoided at any relative size, because it cannot manoeuvre out
    /// of the way. The dynamic/static split reads the hazard's own authored
    /// [`ColliderConfig::movable`] fact.
    ///
    /// Defaults to [`crate::ai::HAZARD_IGNORE_SIZE_RATIO`] when absent.
    #[serde(default = "default_hazard_ignore_size_ratio")]
    pub hazard_ignore_size_ratio: f32,
    /// Authored shape of the avoidance severity ramp (issue #968): the exponent
    /// the spent share of `avoidance_buffer` is raised to. `1.0` is a straight
    /// line; the shipped `2.0` reacts gently while there is still room and hard
    /// once there is not. Both ends of the ramp are fixed by the model (`0.0` a
    /// full buffer clear, `1.0` at contact, at every obstacle size), so this
    /// only decides how a hull spends the distance in between — a gameplay trade
    /// between dodging early and holding a firing solution.
    /// Defaults to [`crate::ai::HAZARD_THREAT_EXPONENT`] when absent.
    #[serde(default = "default_hazard_threat_exponent")]
    pub hazard_threat_exponent: f32,
    /// Authored ceiling (radians) on how far a DEAD-RECKONED hull will hold its
    /// heading off its route bearing to clear an obstacle (issue #968). Only the
    /// low-LOD mover reads it: a high-fidelity hull steers through its helm
    /// actuators, where its authored `max_yaw_rate` bounds the turn instead.
    ///
    /// Defaults to [`crate::ai::LOW_LOD_AVOIDANCE_DEVIATION_RAD`] (a quarter
    /// turn) when absent. A quarter turn is the largest deviation that still
    /// makes progress past an obstacle: at 90° off the line to it the ship is
    /// flying the tangent, and beyond that it is heading back the way it came.
    #[serde(default = "default_low_lod_avoidance_deviation_rad")]
    pub low_lod_avoidance_deviation_rad: f32,
    /// Authored lateral-thrust sensitivity to the shared hazard surface (issue
    /// #743): the multiplier the fine lateral-thrust actuator applies to the
    /// hazard assessment's starboard repulsion before clamping to `[-1, 1]`.
    /// Defaults to [`crate::ai::LATERAL_HAZARD_SENSITIVITY`] when absent.
    #[serde(default = "default_lateral_hazard_sensitivity")]
    pub lateral_hazard_sensitivity: f32,
    /// Authored vertical-thrust sensitivity to the shared hazard surface (issue
    /// #744): the multiplier the vertical-thrust actuator applies to the shared
    /// assessment's moving-hazard threat before clamping to `[0, 1]`.
    /// Defaults to [`crate::ai::VERTICAL_HAZARD_SENSITIVITY`] when absent.
    #[serde(default = "default_vertical_hazard_sensitivity")]
    pub vertical_hazard_sensitivity: f32,
    /// Authored hazard-urgency threshold at or above which an imminent collision
    /// may TEMPORARILY override the ship's desired facing toward the escape
    /// direction (issue #780, AC4). Ordinary avoidance below this only bends
    /// travel and never touches facing. Defaults to
    /// [`crate::ai::IMMINENT_COLLISION_FACING_THRESHOLD`] (`1.0` — effectively
    /// off) when absent; the override is stateless and self-clears the tick
    /// urgency drops back under the threshold.
    #[serde(default = "default_imminent_collision_facing_threshold")]
    pub imminent_collision_facing_threshold: f32,
    // `retreat_hull_threshold` lived here until issue #702. It fed a synthetic
    // hull-triggered Retreat that could never win (0..1 score against doctrine's
    // tens) and always steered to world origin (its anchor was empty and the
    // `home_position` it fell back on was never seeded in production). Retreat
    // is now authored as ordinary doctrine, which is strictly more expressive —
    // a real anchor, a real priority, and any `zero_gates` combination rather
    // than one hardwired hull ramp:
    //
    //     [[behaviour.doctrine]]
    //     id               = "retreat-when-hurt"
    //     directive_kind   = "Retreat"
    //     directive_anchor = "pirate_haven"
    //     base_priority    = 100.0
    //     zero_gates       = [{ condition = "hull_below", threshold = 0.3 }]
}

/// Hand-written so `BehaviourConfig::default()` agrees with what serde
/// produces for a `[behaviour]` block that omits every optional field.
/// A derived `Default` would silently zero the tuning fields — a
/// `waypoint_arrival_radius` of `0.0` means "arrival requires landing on the
/// anchor exactly", which no NPC ever does.
impl Default for BehaviourConfig {
    fn default() -> Self {
        Self {
            doctrine: Vec::new(),
            waypoint_arrival_radius: default_waypoint_arrival_radius(),
            avoidance_buffer: default_avoidance_buffer(),
            avoidance_look_ahead_secs: default_avoidance_look_ahead_secs(),
            nav_handoff_speed: default_nav_handoff_speed(),
            docking_engage_distance: default_docking_engage_distance(),
            docking_approach_speed: default_docking_approach_speed(),
            hazard_ignore_size_ratio: default_hazard_ignore_size_ratio(),
            hazard_threat_exponent: default_hazard_threat_exponent(),
            low_lod_avoidance_deviation_rad: default_low_lod_avoidance_deviation_rad(),
            lateral_hazard_sensitivity: default_lateral_hazard_sensitivity(),
            vertical_hazard_sensitivity: default_vertical_hazard_sensitivity(),
            imminent_collision_facing_threshold: default_imminent_collision_facing_threshold(),
        }
    }
}

fn default_imminent_collision_facing_threshold() -> f32 {
    crate::ai::IMMINENT_COLLISION_FACING_THRESHOLD
}

fn default_waypoint_arrival_radius() -> f32 {
    crate::ai::WAYPOINT_ARRIVAL_RADIUS
}

fn default_avoidance_buffer() -> f32 {
    crate::ai::AVOIDANCE_BUFFER
}

fn default_nav_handoff_speed() -> f32 {
    crate::ai::NAV_HANDOFF_SPEED
}

fn default_avoidance_look_ahead_secs() -> f32 {
    crate::ai::AVOIDANCE_LOOK_AHEAD_SECS
}

fn default_hazard_threat_exponent() -> f32 {
    crate::ai::HAZARD_THREAT_EXPONENT
}

fn default_low_lod_avoidance_deviation_rad() -> f32 {
    crate::ai::LOW_LOD_AVOIDANCE_DEVIATION_RAD
}

fn default_docking_engage_distance() -> f32 {
    crate::ai::DOCKING_ENGAGE_DISTANCE
}

fn default_docking_approach_speed() -> f32 {
    crate::ai::DOCKING_APPROACH_SPEED
}

fn default_hazard_ignore_size_ratio() -> f32 {
    crate::ai::HAZARD_IGNORE_SIZE_RATIO
}

fn default_lateral_hazard_sensitivity() -> f32 {
    crate::ai::LATERAL_HAZARD_SENSITIVITY
}

fn default_vertical_hazard_sensitivity() -> f32 {
    crate::ai::VERTICAL_HAZARD_SENSITIVITY
}
