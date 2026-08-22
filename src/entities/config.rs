use crate::entities::ai_declaration_manifest::AiDeclarationMode;
use crate::entities::ai_flag_hosts as ai_hosts;
use crate::regions::effects::RegionEffectsConfig;
use crate::regions::shape::RegionShape;
use serde::de::Error as SerdeError;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

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
    /// Directive kind: `"Patrol"`, `"Destroy"`, `"Reach"`, `"Hail"`, or absent for `None`.
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
    /// Named target for the issue-#1162 operate verbs — `Tow`, `Stabilise`,
    /// `Escort`, `Transfer` and `FieldRepair`. One shared field for all five,
    /// the way `Reach`/`Retreat` share `directive_anchor`: each verb names a
    /// target the owning seat operates on, resolved to a live UUID the same way
    /// a `Destroy`/`Dock` target is.
    #[serde(default)]
    pub directive_operate_target: Option<String>,
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
    /// When absent, defaults to `true` for Reach and Destroy, `false` for Patrol.
    #[serde(default)]
    pub use_impulse: Option<bool>,
}

impl DoctrineObjective {
    /// Resolved effective `use_impulse` value.
    /// Returns `self.use_impulse` if set; otherwise defaults to `true` for
    /// Reach and Destroy directives, `false` for Patrol.
    pub fn effective_use_impulse(&self) -> bool {
        self.use_impulse
            .unwrap_or(!matches!(self.directive_kind.as_deref(), Some("Patrol")))
    }
}

/// Which directive kind reads each `directive_*` field, for error messages.
///
/// The parse side of this table is `parse_doctrine_directive`
/// (`src/ai/core.rs`): a field a directive kind does not read is simply never
/// looked at, which is why authoring the wrong one is silent.
const DIRECTIVE_FIELD_OWNERS: &[(&str, &str)] = &[
    ("directive_anchors", "Patrol"),
    ("directive_loop", "Patrol"),
    ("directive_target", "Destroy"),
    ("directive_anchor", "Reach / Retreat"),
    ("directive_hail_target", "Hail"),
    ("directive_dock_target", "Dock"),
    (
        "directive_operate_target",
        "Tow / Stabilise / Escort / Transfer / FieldRepair",
    ),
];

/// Reject a doctrine entry that authors a `directive_*` field belonging to a
/// *different* directive kind, or that omits one its own kind requires.
///
/// # Why this is a load-time error
///
/// `parse_doctrine_directive` reads exactly one field per kind and ignores the
/// rest. `assets/entities/ship_requiem_courier.toml` authored
/// `directive_anchors = ["destination"]` (the **Patrol** field, plural) on a
/// `Reach` directive; `Reach` reads `directive_anchor` (singular), so the anchor
/// resolved to `""`, `anchors.get("")` missed, and the courier's only goal never
/// resolved — a shipped hull with nothing to do and no diagnostic anywhere. It
/// is the same "silently reads as nothing" failure mode as an unvalidated
/// `fact(...)` name, and gets the same treatment: the entity fails to load
/// rather than reaching a live tick.
///
/// Absent-vs-default is the limit of what this can see: `directive_loop = false`
/// and `directive_anchors = []` are indistinguishable from omission, so only a
/// field carrying a real value is reported.
///
/// # The mission side has its own copy
///
/// A `add_objective` trigger action authors the same directive fields and can
/// make the same mistake, so `parse_directive` (`src/world/config.rs`) runs the
/// mirror of this check over `RawActionEntry`. It is a separate implementation
/// rather than a shared one because the two field sets differ: a mission
/// objective names its `Destroy`/`Hail` target with the shared `target` field,
/// where a doctrine entry has `directive_target` and `directive_hail_target`.
/// Keep the two in step — `DIRECTIVE_FIELD_OWNERS` exists on both sides.
pub fn validate_doctrine_directives(doctrine: &[DoctrineObjective]) -> Result<(), String> {
    for d in doctrine {
        let kind = d.directive_kind.as_deref();
        let (allowed, required): (&[&str], &[&str]) = match kind {
            None | Some("None") => (&[], &[]),
            Some("Patrol") => (&["directive_anchors", "directive_loop"], &[]),
            Some("Destroy") => (&["directive_target"], &[]),
            // Mirrors the world-side `parse_directive`, which already rejects a
            // `Reach`/`Retreat` mission objective with no `directive_anchor`.
            Some("Reach") | Some("Retreat") => (&["directive_anchor"], &["directive_anchor"]),
            Some("Hail") => (&["directive_hail_target"], &[]),
            // Issue #1028. Required for the same reason `Reach` requires its
            // anchor: a Dock with nothing to dock at resolves to no destination
            // and the hull silently never goes anywhere.
            Some("Dock") => (&["directive_dock_target"], &["directive_dock_target"]),
            // The issue-#1162 operate verbs. Each requires its shared
            // `directive_operate_target` for the same reason `Dock` requires its
            // target: an operate directive naming nothing resolves to no target
            // and the seat silently never operates.
            Some("Tow") | Some("Stabilise") | Some("Escort") | Some("Transfer")
            | Some("FieldRepair") => (&["directive_operate_target"], &["directive_operate_target"]),
            Some(other) => {
                return Err(format!(
                    "doctrine '{}': unknown directive_kind '{other}'; \
                     valid: Patrol, Destroy, Reach, Retreat, Hail, Dock, \
                     Tow, Stabilise, Escort, Transfer, FieldRepair",
                    d.id
                ))
            }
        };

        // Fields the author actually filled in with a value.
        let authored: Vec<&str> = [
            (!d.directive_anchors.is_empty()).then_some("directive_anchors"),
            d.directive_loop.then_some("directive_loop"),
            d.directive_target.is_some().then_some("directive_target"),
            d.directive_anchor
                .as_deref()
                .is_some_and(|a| !a.is_empty())
                .then_some("directive_anchor"),
            d.directive_hail_target
                .is_some()
                .then_some("directive_hail_target"),
            d.directive_dock_target
                .as_deref()
                .is_some_and(|t| !t.is_empty())
                .then_some("directive_dock_target"),
            d.directive_operate_target
                .as_deref()
                .is_some_and(|t| !t.is_empty())
                .then_some("directive_operate_target"),
        ]
        .into_iter()
        .flatten()
        .collect();

        // Misplaced fields are reported before missing ones: authoring the
        // neighbouring kind's field is the actual mistake, and "you set
        // `directive_anchors` on a Reach" is far more use to the author than
        // "a Reach needs a `directive_anchor`" when both are true at once.
        for field in &authored {
            if allowed.contains(field) {
                continue;
            }
            let owner = DIRECTIVE_FIELD_OWNERS
                .iter()
                .find(|(f, _)| f == field)
                .map(|(_, owner)| *owner)
                .unwrap_or("no");
            return Err(match kind {
                Some(k) => format!(
                    "doctrine '{}': `{field}` is read only for a {owner} directive, but \
                     directive_kind = \"{k}\", which reads {}. A field belonging to another \
                     directive kind is silently ignored, so it is rejected here instead.",
                    d.id,
                    if allowed.is_empty() {
                        "no directive field".to_string()
                    } else {
                        allowed
                            .iter()
                            .map(|f| format!("`{f}`"))
                            .collect::<Vec<_>>()
                            .join(" / ")
                    },
                ),
                None => format!(
                    "doctrine '{}': `{field}` is read only for a {owner} directive, but no \
                     directive_kind is authored, so nothing reads it.",
                    d.id,
                ),
            });
        }

        for field in required {
            if !authored.contains(field) {
                return Err(format!(
                    "doctrine '{}': directive_kind = \"{}\" requires a non-empty `{field}`",
                    d.id,
                    kind.unwrap_or("None"),
                ));
            }
        }
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

/// Shape variant for the `[mesh]` section of an entity TOML.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MeshShape {
    Sphere,
    Cuboid,
    Torus,
}

impl MeshShape {
    /// Parse the lowercase name TOML uses, or `None` for anything else —
    /// including the empty string, which is how the model viewer's panel says
    /// "this level is a GLB, not a shape".
    pub fn parse(name: &str) -> Option<MeshShape> {
        match name {
            "sphere" => Some(MeshShape::Sphere),
            "cuboid" => Some(MeshShape::Cuboid),
            "torus" => Some(MeshShape::Torus),
            _ => None,
        }
    }
}

/// Visual mesh definition for an entity.
///
/// Present in entity TOMLs as a `[mesh]` section. The renderer creates the
/// appropriate Bevy primitive and material from this data; entities without
/// a `[mesh]` section are not given a 3-D visual on the viewscreen.
///
/// When `model` is set, the renderer loads a GLB scene instead of creating a
/// procedural shape (the `shape`/`colour`/`radius`/etc. fields are ignored for
/// rendering but kept as fallback). `scale` and `rotation` are applied to both
/// paths.
///
/// **Level of detail is not authored here** (issue #914). The LOD ladder
/// belongs to the model, so it lives in the model's rig sidecar as
/// [`crate::entities::model_rig::ModelRig::lod`]; this section only names the model. The
/// flat fields above remain the fallback every level falls back to. A leftover
/// `[[mesh.lod]]` block is rejected by [`EntityConfig::from_toml`] with a
/// message naming the sidecar it belongs in.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeshConfig {
    /// Path to a .glb file, e.g. "assets/models/dynasty_destroyer.glb".
    /// When set, overrides the procedural shape rendering.
    #[serde(default)]
    pub model: Option<String>,
    /// Optional rig-sidecar variant name. The model's rig sidecar is looked
    /// up alongside the `.glb` as `<stem>.<variant>.toml`. When absent the
    /// reserved default name `"model"` is used (i.e. `<stem>.model.toml`).
    /// `Some("weathered")` selects `<stem>.weathered.toml`.
    #[serde(default)]
    pub variant: Option<String>,
    pub shape: MeshShape,
    /// RGB colour `[r, g, b]`, sRGB 0–1 — the renderer feeds it straight to
    /// `Color::srgb` (`procedural_mesh_material`).
    pub colour: Vec<f32>,
    /// Sphere radius, or torus major radius. Ignored for `cuboid`.
    #[serde(default)]
    pub radius: f32,
    /// Full XYZ dimensions of a `cuboid` mesh.
    #[serde(default)]
    pub size: Option<[f32; 3]>,
    /// Tube radius for a `torus` mesh.
    #[serde(default)]
    pub minor_radius: f32,
    /// Emissive multiplier (the renderer multiplies `colour` by this and feeds
    /// the result into `StandardMaterial::emissive`). When `None`, the renderer
    /// applies its own default (typically `0.4` for general-purpose entities).
    #[serde(default)]
    pub emissive: Option<f32>,
    /// Uniform scale multiplier applied to the entity's transform.
    /// Affects both GLB models and procedural shapes.
    #[serde(default = "default_mesh_scale")]
    pub scale: f32,
    /// Euler rotation [x, y, z] in radians applied to the entity's transform.
    /// Affects both GLB models and procedural shapes.
    #[serde(default)]
    pub rotation: [f32; 3],
}

fn default_mesh_scale() -> f32 {
    1.0
}

/// Hysteresis margin (world units) applied by [`select_lod`]. Once an entity is
/// showing a given level, the camera distance must move past the band boundary
/// by more than this margin before the level switches. This prevents rapid
/// flip-flopping when the camera hovers exactly on a boundary.
pub const LOD_HYSTERESIS_MARGIN: f32 = 5.0;

/// One distance band in a model rig sidecar's `[[lod]]` chain
/// ([`crate::entities::model_rig::ModelRig::lod`]).
///
/// Levels are declared near→far in ascending `max_distance` order. Each level
/// self-describes as either a GLB level (`model` set) or a procedural level
/// (`shape` set); a level with neither is invalid and is skipped by the
/// renderer. Every visual field is optional — when omitted, the renderer falls
/// back to the corresponding flat [`MeshConfig`] field of the *entity* that
/// named the model, so a level only needs to declare what differs from the
/// shared defaults.
///
/// The type stays here, next to [`select_lod`], because selection is entity
/// rendering logic; only the *authoring location* moved to the sidecar (#914).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct LodLevel {
    /// Upper bound (exclusive) of this level's camera-distance band. Level `i`
    /// covers `[bound(i-1), max_distance)`. The final (fallback) level omits
    /// `max_distance`, which is treated as `f32::INFINITY`.
    #[serde(default)]
    pub max_distance: Option<f32>,
    /// Path to a `.glb` file. When set, this is a GLB level and the procedural
    /// fields are ignored for this band.
    #[serde(default)]
    pub model: Option<String>,
    /// Optional rig-sidecar variant name for the GLB (see [`MeshConfig::variant`]).
    #[serde(default)]
    pub variant: Option<String>,
    /// Path to a billboard atlas `.png` for this band. When set, this is a
    /// **billboard level**: the renderer draws a single camera-facing quad
    /// textured from the atlas (a yaw ring of pre-rendered views of the model),
    /// picking the tile nearest the camera's heading relative to the entity.
    ///
    /// It is the far replacement for a procedural `shape` stand-in — a captured
    /// silhouette of the actual hull reads far better at 400+ than a coloured
    /// sphere — and, because the PNG loads long before a multi-MB GLB, it is
    /// also what shows while the near levels are still streaming in. Mutually
    /// exclusive with `model` and `shape`; the atlas is baked by the model
    /// viewer's capture tool (see `[lod.capture]`).
    #[serde(default)]
    pub billboard: Option<String>,
    /// Procedural shape for this band. Used only when `model` is `None`.
    #[serde(default)]
    pub shape: Option<MeshShape>,
    /// RGB colour `[r, g, b]`, sRGB 0–1 — the renderer feeds it straight to
    /// `Color::srgb` (`procedural_mesh_material`). Falls back to
    /// [`MeshConfig::colour`].
    #[serde(default)]
    pub colour: Option<Vec<f32>>,
    /// Sphere radius / torus major radius. Falls back to `MeshConfig::radius`.
    #[serde(default)]
    pub radius: Option<f32>,
    /// Cuboid full XYZ dimensions. Falls back to `MeshConfig::size`.
    #[serde(default)]
    pub size: Option<[f32; 3]>,
    /// Torus tube radius. Falls back to `MeshConfig::minor_radius`.
    #[serde(default)]
    pub minor_radius: Option<f32>,
    /// Emissive multiplier. Falls back to `MeshConfig::emissive`.
    #[serde(default)]
    pub emissive: Option<f32>,
    /// Euler `[x, y, z]` rotation in radians for a **procedural** level.
    ///
    /// Applied to the level's own visual, never to the entity: an entity's
    /// rotation is simulation state — physics owns it on anything that moves —
    /// so writing it here would fight the sim every time the level changed. A
    /// GLB level takes its orientation from its rig sidecar's `[base] rotation`
    /// instead, which is why this is ignored for one.
    ///
    /// It exists for the same reason a level has a `scale`: a sphere standing
    /// in for a hull wants to point the way the hull points.
    #[serde(default)]
    pub rotation: Option<[f32; 3]>,
    /// Non-uniform `[x, y, z]` scale for this level, multiplied onto the
    /// entity's own uniform [`MeshConfig::scale`].
    ///
    /// The flat `[mesh]` scale is a single number, which is all a model needs —
    /// but a procedural level is a *stand-in* for a model, and a sphere is the
    /// wrong shape for almost everything it stands in for. Three numbers turn
    /// that sphere into an ellipsoid roughly the proportions of the thing it
    /// replaces, which is the difference between a distant hull reading as a
    /// hull and reading as a ball.
    ///
    /// Applies to GLB levels too, for the same reason a level may override any
    /// other visual field. Omitted means `[1, 1, 1]` — the entity's own scale,
    /// unchanged — and every level recomputes it on switch, so moving between
    /// levels that do and do not declare one is symmetric.
    #[serde(default)]
    pub scale: Option<[f32; 3]>,
    /// How this level's `model` was decimated out of a source GLB (issue #919).
    /// Authored as a `[lod.generate]` sub-table. Build-time provenance only —
    /// see [`LodGeneration`]; the renderer never reads it.
    #[serde(default)]
    pub generate: Option<LodGeneration>,
    /// Whether this level's `model` ships a rig sidecar of its own — the
    /// ladder's *convention*, recorded at build time by whichever pipeline
    /// script wrote the ladder (issue: sidecar-probe-404).
    ///
    /// Only a GENERATED tier carries it: level 0's model IS the primary GLB, so
    /// its sidecar is the one being read right now, and a billboard level has no
    /// GLB to ask about.
    ///
    /// `None` means the sidecar predates the field — a mod pack's hand-authored
    /// ladder, chiefly. The renderer then falls back to probing the tier sidecar
    /// as it always did; see [`crate::entities::glb_visual::resolve_tier_parent_scale`].
    #[serde(default)]
    pub tier_rig: Option<TierRig>,
    /// How this level's `billboard` atlas was captured. Authored as a
    /// `[lod.capture]` sub-table. Build-time provenance only — see
    /// [`LodCapture`]; the renderer never reads it.
    #[serde(default)]
    pub capture: Option<LodCapture>,
}

/// Whether a generated LOD tier's `.glb` ships a rig sidecar beside it.
///
/// Two pipelines write ladders and they differ on this, which is the whole
/// reason [`crate::entities::glb_visual::tier_parent_scale`] exists. The fact
/// was previously *inferred at runtime* by reading the tier's sidecar and
/// seeing what came back — and on wasm "seeing what came back" is an HTTP
/// fetch, so every hull ladder in a browser session begged a 404 for a file
/// deliberately not shipped. It is knowable when the ladder is authored, so it
/// is authored.
///
/// Recorded per level rather than once per ladder because TOML root keys must
/// precede every table header, so a ladder-wide key could not live beside the
/// `[[lod]]` blocks that `scripts/viewer-lods.mjs` rewrites — and a field the
/// ladder writer does not rewrite is a field that goes stale. On the level it
/// rides the same `LEVEL_KEYS` round trip as `model` itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TierRig {
    /// No sidecar beside this tier's `.glb` — every ship hull, the starbase, the
    /// research outpost. The tier resolves an identity rig, so the parent
    /// transform owes it the whole `[base].scale`. Nothing may fetch its
    /// sidecar: there has never been one to fetch.
    Identity,
    /// A sidecar sits beside this tier's `.glb` carrying the primary's `[base]`
    /// rig verbatim — every asteroid class, written by
    /// `scripts/import-asteroids.mjs`. The tier applies the base scale itself,
    /// so the parent owes it none.
    Baked,
}

/// Decimation parameters that produced a generated LOD level's `.glb`
/// (issue #919), authored as a `[lod.generate]` sub-table on the level.
///
/// **Ignored at runtime.** Every field here is meaningless to the renderer: by
/// the time the game loads the ladder, the decimation has already happened and
/// the only thing that matters is the file on disk. It lives in the sidecar
/// anyway so the sidecar *fully* declares its ladder — the distances, the files,
/// and how those files come back if someone deletes them. The alternative was a
/// second list of ratios inside a build script, which is exactly the hardcoded
/// table this repo does not keep (Key Constraint 11).
///
/// `scripts/generate-lods.mjs` is the one reader: it plans a
/// simplify → resize run per declared level and records the result in
/// `scripts/lod-manifest.toml`. A level with no `[lod.generate]` is authored by
/// hand and the generator leaves it alone.
///
/// ```toml
/// [[lod]]
/// max_distance = 100.0
/// model = "assets/models/asteroid_common_1_lod1.glb"
///
/// [lod.generate]
/// source = "assets/models/asteroid_common_1.glb"
/// ratio = 0.25
/// error = 0.01
/// texture_size = 512
/// ```
///
/// Optional throughout, and deliberately so: the engine must never reject a
/// sidecar over a build-time key it does not use. Validation of the *values*
/// (a missing `source`, a ratio outside 0–1, two sidecars claiming the same
/// output with different parameters) belongs to the generator, which is where
/// the parameters mean something. `deny_unknown_fields` still applies, so a
/// misspelled key fails loudly rather than being silently dropped.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct LodGeneration {
    /// The `.glb` this level is decimated from — normally the ladder's own
    /// near level. Omitted means "the first GLB level of this chain".
    #[serde(default)]
    pub source: Option<String>,
    /// meshoptimizer target ratio (0–1) of vertices to keep.
    #[serde(default)]
    pub ratio: Option<f32>,
    /// meshoptimizer error limit, as a fraction of mesh radius.
    #[serde(default)]
    pub error: Option<f32>,
    /// Maximum texture dimension (px) after decimation. Omitted means the
    /// source's textures are carried over untouched.
    #[serde(default)]
    pub texture_size: Option<u32>,
    /// Voxel size for the optional Blender voxel-remesh pre-pass
    /// (`scripts/blender-voxel-remesh.py`), for meshes that decimate badly.
    /// Omitted means no pre-pass, which is the case for every shipped ladder.
    ///
    /// **In the model's own units — the raw GLB geometry, before `[base] scale`.**
    /// Every other number in this file (`max_distance`, `[extents]`) is
    /// post-scale world units, so on a rock scaled 4.2x the two are nothing
    /// alike: `1.0` against an extent of 8 looks small and in fact spans half
    /// the mesh, which remeshes the asteroid into a cube. Divide the extent by
    /// the base scale to see what the model measures, then take a small
    /// fraction of that (a sixty-fourth is a reasonable start).
    #[serde(default)]
    pub remesh_voxel_size: Option<f32>,
}

/// Capture parameters that produced a billboard level's atlas `.png`, authored
/// as a `[lod.capture]` sub-table on the level.
///
/// **Ignored at runtime**, exactly like [`LodGeneration`]: once the atlas is on
/// disk the renderer only needs the file. It lives in the sidecar so the ladder
/// *fully* declares how the atlas comes back — the model viewer's capture tool
/// (`src/viewer/capture.rs`, reached from the LOD panel) is the one reader, and
/// re-baking needs a GPU + the browser viewer, so like the Blender voxel pre-pass
/// this is a local step CI only re-hashes (`scripts/lod-manifest.toml`).
///
/// ```toml
/// [[lod]]
/// billboard = "assets/models/alliance_battleship_lod3.png"
/// scale = [11.3, 12.0, 1.0]   # world width/height of the quad
///
/// [lod.capture]
/// source = "assets/models/alliance_battleship.glb"
/// yaw_views = 8       # tiles around a horizontal ring, packed left→right
/// resolution = 256    # per-tile pixels (square)
/// pitch = 20.0        # camera pitch in degrees above the ring plane
/// ```
///
/// Optional throughout for the same reason [`LodGeneration`] is: the engine must
/// never reject a sidecar over a build-time key it does not use. `deny_unknown_fields`
/// still applies, so a misspelled key fails the build rather than being dropped.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct LodCapture {
    /// The `.glb` the atlas was rendered from — the ladder's near level.
    #[serde(default)]
    pub source: Option<String>,
    /// Number of yaw views packed into the atlas, left→right (a horizontal ring).
    #[serde(default)]
    pub yaw_views: Option<u32>,
    /// Per-tile resolution in pixels (square tiles).
    #[serde(default)]
    pub resolution: Option<u32>,
    /// Camera pitch in degrees above the ring plane the views were rendered at.
    #[serde(default)]
    pub pitch: Option<f32>,
}

/// Upper bound (exclusive) of level `i`'s distance band. A missing
/// `max_distance` means the band extends to infinity (the fallback level).
fn lod_upper_bound(levels: &[LodLevel], i: usize) -> f32 {
    levels[i].max_distance.unwrap_or(f32::INFINITY)
}

/// The naive (hysteresis-free) level for `distance`: the first level whose
/// upper bound exceeds `distance`, or the last level when `distance` is beyond
/// every bound. `levels` must be non-empty.
fn naive_lod_level(levels: &[LodLevel], distance: f32) -> usize {
    for i in 0..levels.len() {
        if distance < lod_upper_bound(levels, i) {
            return i;
        }
    }
    levels.len() - 1
}

/// Select which LOD level to display for a given camera `distance`, applying
/// hysteresis around band boundaries.
///
/// `levels` are ordered near→far; level `i` nominally covers
/// `[bound(i-1), bound(i))` where `bound(i)` is `levels[i].max_distance`
/// (missing = `f32::INFINITY`). `current` is the level shown last frame, or
/// `None` on the first evaluation.
///
/// Boundary behaviour: when already at `current`, the result only changes once
/// `distance` crosses the relevant boundary by more than
/// [`LOD_HYSTERESIS_MARGIN`]. Crossing outward (to a farther level) requires
/// `distance > upper_bound(current) + margin`; crossing inward (to a nearer
/// level) requires `distance < lower_bound(current) - margin`. Within the
/// margin the level holds. `current == None` uses the naive selection with no
/// hysteresis. Empty `levels` returns `0` (the caller handles the no-LOD case).
pub fn select_lod(levels: &[LodLevel], distance: f32, current: Option<usize>) -> usize {
    if levels.is_empty() {
        return 0;
    }
    let naive = naive_lod_level(levels, distance);
    let Some(cur) = current else {
        return naive;
    };
    // Clamp a possibly-stale index into range, then hold unless the distance has
    // cleared the boundary in the direction of travel by more than the margin.
    let cur = cur.min(levels.len() - 1);
    if naive == cur {
        return cur;
    }
    if naive > cur {
        // Moving outward: only switch once past this level's upper bound + margin.
        if distance > lod_upper_bound(levels, cur) + LOD_HYSTERESIS_MARGIN {
            naive
        } else {
            cur
        }
    } else {
        // Moving inward: only switch once below this level's lower bound - margin.
        let lower_bound = if cur == 0 {
            0.0
        } else {
            lod_upper_bound(levels, cur - 1)
        };
        if distance < lower_bound - LOD_HYSTERESIS_MARGIN {
            naive
        } else {
            cur
        }
    }
}

/// Reject an entity TOML that still authors `[[mesh.lod]]` (issue #914).
///
/// The ladder now lives in the model's rig sidecar. Silently ignoring the old
/// location would leave an author convinced they had a ladder while the entity
/// rendered a single level forever, and letting `deny_unknown_fields` handle it
/// yields "unknown field `lod`" — true, but it does not say where the field
/// went. So the check runs first and names the exact sidecar file whenever the
/// `[mesh]` section identifies a model, because "move it to the sidecar" is
/// only actionable if you know *which* sidecar.
fn reject_relocated_mesh_lod(value: &toml::Value) -> Result<(), toml::de::Error> {
    let Some(mesh) = value.get("mesh") else {
        return Ok(());
    };
    if mesh.get("lod").is_none() {
        return Ok(());
    }
    let sidecar = mesh
        .get("model")
        .and_then(|m| m.as_str())
        .map(|model| {
            crate::entities::model_rig::sidecar_path(
                model,
                mesh.get("variant").and_then(|v| v.as_str()),
            )
        })
        .unwrap_or_else(|| "assets/models/<model>.<variant>.toml".to_string());
    Err(SerdeError::custom(format!(
        "[[mesh.lod]] has moved to the model rig sidecar (issue #914): author the \
         chain as [[lod]] blocks in {sidecar} and delete it from the entity TOML. \
         The entity's [mesh] keeps the model reference and the flat fallback fields \
         that sidecar levels fall back to."
    )))
}

fn default_star_radius() -> f32 {
    40.0
}

fn default_star_longitude_segments() -> u32 {
    64
}

fn default_star_latitude_segments() -> u32 {
    32
}

fn default_star_surface_colour() -> [f32; 3] {
    [1.0, 0.72, 0.12]
}

fn default_star_hot_colour() -> [f32; 3] {
    [1.0, 0.96, 0.65]
}

fn default_star_cell_colour() -> [f32; 3] {
    [0.95, 0.32, 0.04]
}

fn default_star_halo_colour() -> [f32; 3] {
    [1.0, 0.78, 0.18]
}

fn default_star_halo_radius_multiplier() -> f32 {
    2.4
}

fn default_star_animation_speed() -> f32 {
    1.0
}

/// Animated procedural star/sun visual definition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct StarConfig {
    pub radius: f32,
    pub longitude_segments: u32,
    pub latitude_segments: u32,
    /// RGB colour `[r, g, b]` in linear 0-1 range.
    pub surface_colour: [f32; 3],
    /// RGB colour `[r, g, b]` in linear 0-1 range.
    pub hot_colour: [f32; 3],
    /// RGB colour `[r, g, b]` in linear 0-1 range.
    pub cell_colour: [f32; 3],
    /// RGB colour `[r, g, b]` in linear 0-1 range.
    pub halo_colour: [f32; 3],
    pub halo_radius_multiplier: f32,
    pub animation_speed: f32,
}

impl Default for StarConfig {
    fn default() -> Self {
        Self {
            radius: default_star_radius(),
            longitude_segments: default_star_longitude_segments(),
            latitude_segments: default_star_latitude_segments(),
            surface_colour: default_star_surface_colour(),
            hot_colour: default_star_hot_colour(),
            cell_colour: default_star_cell_colour(),
            halo_colour: default_star_halo_colour(),
            halo_radius_multiplier: default_star_halo_radius_multiplier(),
            animation_speed: default_star_animation_speed(),
        }
    }
}

fn default_planet_radius() -> f32 {
    20.0
}

fn default_planet_emissive_strength() -> f32 {
    1.0
}

fn default_planet_emissive_night_only() -> bool {
    true
}

fn default_planet_cloud_scale() -> f32 {
    1.03
}

fn default_planet_atmosphere_strength() -> f32 {
    1.0
}

fn default_planet_longitude_segments() -> u32 {
    128
}

fn default_planet_latitude_segments() -> u32 {
    64
}

/// Textured planet visual definition (`[planet]` section).
///
/// Renders as a UV sphere with a custom shader sampling equirectangular
/// texture maps: day/night lighting relative to the star, optional
/// nightside-gated emissive (city lights / nightglow), an optional
/// alpha-blended cloud/smog/ash shell on a slightly larger sphere, and an
/// optional fresnel atmosphere rim glow.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanetConfig {
    #[serde(default = "default_planet_radius")]
    pub radius: f32,
    #[serde(default = "default_planet_longitude_segments")]
    pub longitude_segments: u32,
    #[serde(default = "default_planet_latitude_segments")]
    pub latitude_segments: u32,
    /// Core surface texture set (`[planet.surface]`). Required.
    pub surface: PlanetSurfaceConfig,
    /// Optional cloud/smog/ash shell (`[planet.clouds]`).
    #[serde(default)]
    pub clouds: Option<PlanetCloudsConfig>,
    /// Optional atmosphere rim glow (`[planet.atmosphere]`).
    #[serde(default)]
    pub atmosphere: Option<PlanetAtmosphereConfig>,
}

/// Core surface texture maps for a `[planet]`. Paths are TOML-style
/// (`assets/...`-prefixed) like `MeshConfig.model`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanetSurfaceConfig {
    /// Base colour map (sRGB). Required.
    pub albedo: String,
    /// Tangent-space normal map (linear).
    #[serde(default)]
    pub normal: Option<String>,
    /// Grayscale roughness map (linear).
    #[serde(default)]
    pub roughness: Option<String>,
    /// Emissive colour map (sRGB): city lights, nightglow, lava glow.
    #[serde(default)]
    pub emissive_colour: Option<String>,
    /// Grayscale emissive mask (linear). When absent the emissive colour map
    /// is used unmasked (maps that are black where unlit need no mask).
    #[serde(default)]
    pub emissive_mask: Option<String>,
    /// Gate emission to the night side (city lights). `false` for emission
    /// that is visible on the day side too (lava).
    #[serde(default = "default_planet_emissive_night_only")]
    pub emissive_night_only: bool,
    #[serde(default = "default_planet_emissive_strength")]
    pub emissive_strength: f32,
}

/// Cloud/smog/ash shell rendered on a second, slightly larger sphere.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanetCloudsConfig {
    /// Cloud colour map (sRGB). Required.
    pub albedo: String,
    /// Grayscale opacity map (linear). When absent the albedo luminance is
    /// used as opacity.
    #[serde(default)]
    pub opacity: Option<String>,
    /// Cloud normal map — accepted for authoring completeness but unused by
    /// the core shader.
    #[serde(default)]
    pub normal: Option<String>,
    /// Shell radius as a multiple of the planet radius.
    #[serde(default = "default_planet_cloud_scale")]
    pub scale: f32,
    /// Longitudinal drift in UV wraps per second. 0 = static.
    #[serde(default)]
    pub drift_speed: f32,
}

/// Fresnel rim atmosphere glow parameters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanetAtmosphereConfig {
    /// RGB colour `[r, g, b]` in linear 0-1 range.
    pub colour: [f32; 3],
    #[serde(default = "default_planet_atmosphere_strength")]
    pub strength: f32,
}

/// Kind of a `[[light]]` entry: a point light or a directional light.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LightKind {
    Point,
    Directional,
}

/// One `[[light]]` entry from an entity template. Renderer-only data.
///
/// Replaces the per-section light fields that used to live on `[star]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LightConfig {
    pub kind: LightKind,
    /// RGB colour `[r, g, b]` in linear 0–1 range.
    pub colour: [f32; 3],
    /// Light intensity (candela for point lights, illuminance for directional).
    pub intensity: f32,
    /// Range in world units. Required for point lights; ignored for directional.
    #[serde(default)]
    pub range: Option<f32>,
    /// If true, the light is spawned as a child entity and continuously
    /// rotated to face the player's ship, regardless of how the parent
    /// entity itself is oriented.
    #[serde(default)]
    pub face_player: bool,
}

/// One entry in the `[[hull.system_hull]]` TOML array — the SystemId-keyed
/// hull config entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SystemHullEntry {
    /// Stable ship-wide system identifier (e.g. `"helm"`, `"phaser-fore"`).
    /// Deserialises from a bare TOML string via the `SystemId(String)`
    /// newtype.
    pub system_id: crate::core::messages::SystemId,
    /// Optional human-readable name. When omitted, downstream code falls
    /// back to the raw `system_id` string.
    #[serde(default)]
    pub display_name: Option<String>,
    /// Maximum (and starting) HP for this system.
    pub max_hp: f32,
    /// HP fraction below which the system enters the `Damaged` tier.
    /// Defaults to `0.75` (below 75 % → Damaged).
    #[serde(default = "default_damaged_threshold_pct")]
    pub damaged_threshold_pct: f32,
    /// HP fraction below which the system enters the `Disabled` tier.
    /// Defaults to `0.25` (below 25 % → Disabled).
    #[serde(default = "default_disabled_threshold_pct")]
    pub disabled_threshold_pct: f32,
    /// Performance reduction applied when the system is in the `Damaged` or
    /// `Disabled` tier (fraction, e.g. `0.15` = 15 % reduction).
    /// Defaults to `0.15`.
    #[serde(default = "default_debuff_magnitude")]
    pub debuff_magnitude: f32,
}

fn default_damaged_threshold_pct() -> f32 {
    0.75
}

fn default_disabled_threshold_pct() -> f32 {
    0.25
}

fn default_debuff_magnitude() -> f32 {
    0.15
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct HullConfig {
    /// HP for entities with a single hull slot (stations, asteroids, NPC ships).
    #[serde(default)]
    pub hull_integrity: f32,
    /// Per-system hull entries. When present, replaces `hull_integrity`.
    #[serde(default)]
    pub system_hull: Vec<SystemHullEntry>,
}

/// The body shapes a template may author. Each maps to exactly one
/// `bevy_rapier3d::Collider` constructor in
/// [`crate::entities::spawner::spawn_entity`], and that mapping is the whole
/// of the shape's meaning — nothing downstream re-derives geometry from the
/// variant.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ColliderShape {
    /// A sphere of [`ColliderConfig::radius`]. `Collider::ball`.
    Ball,
    /// A Y-axis capsule — a cylinder of [`ColliderConfig::radius`] with
    /// hemispherical caps, `length` tall through the straight section.
    /// `Collider::capsule_y`. Structurally taller than it is wide.
    Capsule,
    /// A Y-axis cylinder of [`ColliderConfig::radius`] and
    /// [`ColliderConfig::half_height`]. `Collider::cylinder`.
    ///
    /// The shape a DISC needs, and the reason the variant exists (the
    /// station-collider correction, and John's invariant that collision match
    /// visible size). A hub station is 34 across and 14 tall: a Ball at the max
    /// half-extent is right in the wide axis and over-covers the short one by
    /// ten units, and a Capsule cannot be authored wider than it is tall at
    /// all. A cylinder is the only one of the three that can be BOTH right —
    /// so a ship crossing directly over a hub now stops at the visible surface
    /// rather than well above it.
    ///
    /// Flat, not rounded: `Collider::cylinder`, not `round_cylinder`. The rim
    /// of a station deck is an edge, and a border radius would put the same
    /// vertical over-coverage back at the rim in miniature.
    Cylinder,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ColliderConfig {
    pub shape: ColliderShape,
    pub radius: f32,
    pub length: f32,
    /// Half the body's extent along Y, for [`ColliderShape::Cylinder`] only.
    ///
    /// A HALF-extent rather than a full height, because that is the number
    /// `Collider::cylinder` itself takes: authoring the half means the value in
    /// the TOML is the value handed to rapier, with no doubling or halving in
    /// between. (`length` is the other convention — a Capsule authors the full
    /// length of its straight section and the spawner halves it — and having
    /// the two spellings differ is precisely what keeps a cylinder from being
    /// silently authored at twice its intended height.)
    ///
    /// `Option` with a serde default so every Ball and Capsule template on disk
    /// parses unchanged; [`ColliderConfig::cylinder_half_height`] is the single
    /// reader, and it is where a `Cylinder` that forgot the field is caught.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub half_height: Option<f32>,
    /// Authored hazard fact (issue #958): whether this body moves under its own
    /// power. `true` is a mobile CONTACT (a ship, which can manoeuvre out of the
    /// way); `false` is static TERRAIN (an asteroid, a station, a planet, a
    /// moon, a star), which cannot.
    ///
    /// Read by the AI world-snapshot builders into
    /// [`crate::ai::AiWorldEntity::movable`], where it decides three things:
    /// whether the hazard may be dropped by the authored ignore-smaller rule
    /// (static terrain never is — issue #958), whether it contributes vertical
    /// repulsion (issue #780), and whether it counts toward the planner's
    /// moving-hazard urgency (issue #744).
    ///
    /// Defaults to [`default_collider_movable`] — static — so a template that
    /// forgets the field errs toward being avoided rather than ignored.
    #[serde(default = "default_collider_movable")]
    pub movable: bool,
}

/// Parse-time default for [`ColliderConfig::movable`]: `false`, i.e. static
/// terrain.
///
/// It is the safe direction for exactly ONE of the three things the field
/// gates, and it is NOT a blanket safe default. A body that forgets the field
/// is always avoided and never size-ignored (issue #958) — that is the safe
/// one. For the other two, `false` is the *unsafe* direction, and it fails
/// quietly rather than loudly:
///
///   * A real hull that omits the field stops contributing vertical repulsion
///     to everyone else's hazard field (issue #780), because
///     `assess_hazards` zeroes the vertical term for a static obstacle.
///   * The same hull stops counting toward the helm planner's moving-hazard
///     urgency (issue #744), which filters to `movable` contributions.
///
/// So a ship misfiled as terrain is over-avoided by others and under-reactive
/// itself, with nothing at parse time to say so. `false` is still the right
/// default, but only because it is not load-bearing: every shipped hull
/// authors `movable = true` and
/// `shipped_hulls_are_mobile_and_shipped_terrain_is_not` walks
/// `assets/entities/` to hold that line for new templates. The guard is what
/// makes the default safe, not the default itself.
fn default_collider_movable() -> bool {
    false
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppearanceConfig {
    pub colour: String,
    pub size_min: f32,
    pub size_max: f32,
}

/// Declares that an entity should appear on radar and how. There are no
/// defaults derived from tags or entity type anywhere downstream — this
/// table is the single source of truth. At least one of `icon` or
/// `region_colour` must be set; an entity with neither (or with no
/// `[radar_appearance]` table at all) never appears on radar.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RadarAppearanceConfig {
    /// Point-blip icon name. Free-form — resolved by naming convention to
    /// `assets/radar_icons/Icon-{Capitalized}.png` on both clients. No
    /// whitelist/enum; a missing PNG falls back to a coloured circle.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// Icon point colour (also the coloured-circle fallback when the icon
    /// PNG is missing). `None` renders the fallback in a single neutral
    /// constant colour.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub colour: Option<Vec<f32>>,
    /// World-space radius for the icon blip only. When `None`, the entity's
    /// physical collider radius is used. Does not affect region rendering.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<f32>,
    /// Area-fill colour for region/field entities. Geometry comes from the
    /// entity's existing `[shape]` or `[asteroid_field]` section; this only
    /// controls whether the region is drawn on radar and in what colour.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region_colour: Option<Vec<f32>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HelmConsoleConfig {
    #[serde(default)]
    pub max_speed: f32,
    #[serde(default)]
    pub max_reverse_speed: f32,
    #[serde(default)]
    pub acceleration: f32,
    #[serde(default)]
    pub deceleration: f32,
    #[serde(default)]
    pub max_yaw_rate: f32,
    /// Extra turn authority for flying slow, as a fraction added at a dead stop
    /// and lerped away to nothing at `max_speed`. `0.5` means a stationary hull
    /// turns 50% faster than it does at full throttle; `0.0` (the default) is
    /// the old speed-independent turn rate.
    ///
    /// This is the throttle-vs-turn trade that keeps evenly-matched hulls from
    /// deadlocking in a co-rotating circle. Authored per class: light hulls get
    /// the most, capital hulls none.
    #[serde(default)]
    pub low_speed_turn_boost: f32,
    /// Radar configuration for the Helm radar widget, from
    /// `[helm_console.radar]`.
    #[serde(default)]
    pub radar: Option<crate::radar_config::RadarConfig>,
    /// RGBA colour the helm radar uses for the red-alert hostile weapon-arc
    /// overlay (issue #874). Four floats in 0.0–1.0; the fourth is the fill
    /// opacity, and "faint" is the whole point of the overlay. When absent (or
    /// not exactly four entries) the `ShipClientConfig` default applies.
    #[serde(default)]
    pub hostile_arc_color: Vec<f32>,
    #[serde(default)]
    pub power_multipliers: Option<[f32; 4]>,
    /// Total time in seconds to fully charge the impulse drive.
    /// Defaults to `IMPULSE_CHARGE_DURATION` (3.0 s) when absent.
    #[serde(default = "default_impulse_charge_duration")]
    pub impulse_charge_duration: f32,
    /// Speed multiplier applied when impulse drive is active.
    /// Defaults to `IMPULSE_SPEED_MULTIPLIER` (10.0) when absent.
    #[serde(default = "default_impulse_speed_multiplier")]
    pub impulse_speed_multiplier: f32,
    /// Acceleration multiplier applied while impulse drive is active.
    /// Defaults to `IMPULSE_ACCELERATION_MULTIPLIER` (5.0) when absent.
    #[serde(default = "default_impulse_acceleration_multiplier")]
    pub impulse_acceleration_multiplier: f32,
    /// Minimum distance from target at which AI may engage impulse (world units).
    /// Defaults to 200.0 when absent.
    #[serde(default = "default_impulse_engage_distance")]
    pub impulse_engage_distance: f32,
    /// Distance from target at which AI cancels impulse (world units).
    /// Defaults to 40.0 when absent.
    #[serde(default = "default_impulse_cancel_distance")]
    pub impulse_cancel_distance: f32,
    /// Maximum visual banking (roll) angle in degrees when steering at full
    /// deflection. The ship leans into turns, lerped from 0 toward ±max_bank_deg
    /// based on steering input percentage. 0 = no banking.
    #[serde(default)]
    pub max_bank_deg: f32,
    /// How quickly the ship's visual roll lerps toward the target bank angle
    /// (units: per-second lerp rate). Defaults to
    /// [`crate::ship_plugin::BANK_LERP_RATE`] when absent.
    #[serde(default = "default_bank_lerp_rate")]
    pub bank_lerp_rate: f32,
    /// Optional boost drive config, from `[helm_console.boost]`. When absent the
    /// boost feature is disabled entirely (no button on the helm).
    #[serde(default)]
    pub boost: Option<BoostConfig>,
    /// Optional procedural engine PFX tuning, from `[helm_console.engine_pfx]`.
    /// Rendering code supplies defaults for omitted fields.
    #[serde(default)]
    pub engine_pfx: Option<EnginePfxConfig>,
    /// Optional lateral thrust tuning, from `[helm_console.lateral_thrust]`.
    /// When absent, ShipPhysicsConfig defaults are used.
    #[serde(default)]
    pub lateral_thrust: Option<LateralThrustConfig>,
    /// Inline stateless AI policy for the **Engines** (longitudinal thrust)
    /// fine system, from `[helm_console.engines_ai]` (issue #779). Absent ⇒ the
    /// canonical [`default_engines_ai_config`] (unconditional actuate) is
    /// synthesised at spawn. Drives the `longitudinal` channel with the
    /// `actuate_desired_travel` mode verb.
    #[serde(default)]
    pub engines_ai: Option<FineSystemAiConfigToml>,
    /// Inline stateless AI policy for the **Steering** (yaw) fine system, from
    /// `[helm_console.steering_ai]` (issue #779). Absent ⇒ the canonical
    /// [`default_steering_ai_config`] is synthesised at spawn. Drives the `yaw`
    /// channel with the `actuate_desired_facing` mode verb.
    #[serde(default)]
    pub steering_ai: Option<FineSystemAiConfigToml>,
    /// Inline stateless AI policy for the **Lateral Thrust** fine system, from
    /// `[helm_console.lateral_ai]` (issue #780). Absent ⇒ the canonical
    /// [`default_lateral_ai_config`] (unconditional actuate) is synthesised at
    /// spawn. Drives the `lateral` channel with the `actuate_lateral_thrust`
    /// mode verb.
    #[serde(default)]
    pub lateral_ai: Option<FineSystemAiConfigToml>,
    /// Inline stateless AI policy for the **Vertical Thrust** fine system, from
    /// `[helm_console.vertical_ai]` (issue #780). Absent ⇒ the canonical
    /// [`default_vertical_ai_config`] (unconditional actuate) is synthesised at
    /// spawn. Drives the `vertical` channel with the `actuate_vertical_thrust`
    /// mode verb.
    #[serde(default)]
    pub vertical_ai: Option<FineSystemAiConfigToml>,
    /// Inline stateless AI policy for the **Impulse** fine system, from
    /// `[helm_console.impulse_ai]` (issue #780). Absent ⇒ the canonical
    /// [`default_impulse_ai_config`] (unconditional permit) is synthesised at
    /// spawn. Drives the `impulse` channel with the `engage_impulse` mode verb.
    #[serde(default)]
    pub impulse_ai: Option<FineSystemAiConfigToml>,
    /// Inline stateless AI policy for the **Boost** fine system, from
    /// `[helm_console.boost_ai]` (issue #780). Absent ⇒ the canonical
    /// [`default_boost_ai_config`] (explicit idle — no AI boost) is synthesised
    /// at spawn. Drives the `boost` channel with the `engage_boost` mode verb.
    #[serde(default)]
    pub boost_ai: Option<FineSystemAiConfigToml>,
}

/// What vertical movement capability the ship has.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum VerticalMovementMode {
    /// No vertical movement — planar-only flight (current default).
    #[default]
    Planar,
    /// AI-only bounded vertical motion for collision avoidance.
    Bounded,
    /// Full 3D six-degree-of-freedom flight.
    Full3D,
}

/// Impulse capability tuning loaded from `[helm_capability.impulse]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImpulseCapabilityConfig {
    /// Steering multiplier applied while impulse is active.
    /// 0.0 = no steering, 0.1 = harsh but possible, 1.0 = full steering.
    #[serde(default = "default_impulse_steering_multiplier")]
    pub steering_multiplier: f32,
}

fn default_impulse_steering_multiplier() -> f32 {
    0.1
}

impl Default for ImpulseCapabilityConfig {
    fn default() -> Self {
        Self {
            steering_multiplier: default_impulse_steering_multiplier(),
        }
    }
}

/// Optional helm capability declaration for an entity (`[helm_capability]`).
///
/// When absent, the ship has no special helm capability and operates at the
/// default planar mode with full steering during impulse.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HelmCapabilityConfig {
    /// Vertical movement mode. Defaults to `Planar`.
    #[serde(default)]
    pub vertical_movement_mode: VerticalMovementMode,
    /// Maximum vertical offset (world units) a `Bounded` craft may climb away
    /// from its cruise plane while dodging moving hazards (issue #744). Ignored
    /// for `Planar` (no vertical motion) and `Full3D` (unbounded).
    /// Defaults to [`crate::ai::MAX_VERTICAL_OFFSET`] when absent.
    #[serde(default = "default_max_vertical_offset")]
    pub max_vertical_offset: f32,
    /// Gradual return-to-cruise gain for a `Bounded` craft once avoidance
    /// urgency falls (issue #744): the vertical actuator eases the ship back to
    /// its cruise plane at `-y * vertical_return_rate` rather than snapping.
    /// Defaults to [`crate::ai::VERTICAL_RETURN_RATE`] when absent.
    #[serde(default = "default_vertical_return_rate")]
    pub vertical_return_rate: f32,
    /// Impulse capability tuning.
    #[serde(default)]
    pub impulse: ImpulseCapabilityConfig,
}

fn default_max_vertical_offset() -> f32 {
    crate::ai::MAX_VERTICAL_OFFSET
}

fn default_vertical_return_rate() -> f32 {
    crate::ai::VERTICAL_RETURN_RATE
}

/// Hand-written so `HelmCapabilityConfig::default()` matches what serde produces
/// for a `[helm_capability]` block that omits every optional field — a derived
/// `Default` would zero the vertical tunables instead of reading their authored
/// constant defaults.
impl Default for HelmCapabilityConfig {
    fn default() -> Self {
        Self {
            vertical_movement_mode: VerticalMovementMode::default(),
            max_vertical_offset: default_max_vertical_offset(),
            vertical_return_rate: default_vertical_return_rate(),
            impulse: ImpulseCapabilityConfig::default(),
        }
    }
}

/// Procedural engine trail tuning, from `[helm_console.engine_pfx]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct EnginePfxConfig {
    /// RGBA trail colour in 0.0-1.0. When omitted, renderer defaults are used.
    #[serde(default)]
    pub color: Option<[f32; 4]>,
    /// Optional rig-marker names used as exhaust origins.
    #[serde(default)]
    pub markers: Vec<String>,
    /// Twist, in degrees, applied around the direction of marker-attached trail emitters.
    /// When omitted, attached trails retain their default orientation.
    #[serde(default)]
    pub roll_degrees: Option<f32>,
    /// Uniform width multiplier for marker-attached trail emitters.
    /// When omitted, attached trails retain their default size.
    #[serde(default)]
    pub scale: Option<f32>,
    /// Seconds each trail segment remains alive. When omitted, renderer defaults are used.
    #[serde(default)]
    pub trail_lifetime_secs: Option<f32>,
    /// Seconds between spawned trail segments. When omitted, renderer defaults are used.
    #[serde(default)]
    pub trail_spawn_interval_secs: Option<f32>,
}

/// Lateral thrust tuning, from `[helm_console.lateral_thrust]`.
/// When absent, the feature uses ShipPhysicsConfig defaults.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LateralThrustConfig {
    /// Maximum lateral speed in world units per second.
    #[serde(default = "default_lateral_thrust_max_speed")]
    pub max_lateral_speed: f32,
    /// Lateral acceleration in world units per second squared.
    #[serde(default = "default_lateral_thrust_acceleration")]
    pub lateral_acceleration: f32,
}

fn default_lateral_thrust_max_speed() -> f32 {
    15.0
}

fn default_lateral_thrust_acceleration() -> f32 {
    15.0
}

impl Default for LateralThrustConfig {
    fn default() -> Self {
        Self {
            max_lateral_speed: default_lateral_thrust_max_speed(),
            lateral_acceleration: default_lateral_thrust_acceleration(),
        }
    }
}

/// Boost drive tuning, from `[helm_console.boost]`. Presence of this table is
/// what enables the boost feature on a ship.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoostConfig {
    /// Multiplier applied to both max speed and acceleration while engaged.
    pub multiplier: f32,
    /// Multiplier applied to max yaw rate while engaged.
    #[serde(default = "default_boost_steering_multiplier")]
    pub steering_multiplier: f32,
    /// Seconds a full battery lasts while boost is engaged.
    pub active_duration: f32,
    /// Seconds for an empty battery to recharge to full.
    pub recharge_duration: f32,
}

impl HelmConsoleConfig {
    /// Radar range from `[helm_console.radar] range`. Returns `0.0` when the
    /// `[helm_console.radar]` table is absent.
    pub fn effective_radar_range(&self) -> f32 {
        self.radar.as_ref().map_or(0.0, |r| r.range)
    }
}

fn default_bank_lerp_rate() -> f32 {
    crate::ship_plugin::BANK_LERP_RATE
}

fn default_impulse_charge_duration() -> f32 {
    crate::ship::impulse::IMPULSE_CHARGE_DURATION
}

fn default_impulse_speed_multiplier() -> f32 {
    crate::ship::impulse::IMPULSE_SPEED_MULTIPLIER
}

fn default_impulse_engage_distance() -> f32 {
    200.0
}

fn default_impulse_cancel_distance() -> f32 {
    40.0
}

fn default_impulse_acceleration_multiplier() -> f32 {
    crate::ship::impulse::IMPULSE_ACCELERATION_MULTIPLIER
}

fn default_boost_steering_multiplier() -> f32 {
    crate::ship::boost::BOOST_STEERING_MULTIPLIER
}

/// Stable identifier for a phaser bank, parsed verbatim from the TOML
/// `id` field on `[[weapons_console.phaser_banks]]`. Used on the wire
/// to address a specific bank (e.g. `FirePhaser { bank: "port" }`).
pub type PhaserBankId = String;

/// One `[[weapons_console.phaser_banks]]` entry. Defines a single phaser
/// bank's orientation on the ship, its full fire arc (used for manual
/// fire validity and for the radar arc overlay), its narrower auto-fire
/// arc (used by `console_ai` for autonomous firing decisions), and its
/// effective beam range.
///
/// `facing_deg` is the bank's centre bearing in ship-local degrees:
/// `0` = forward (−Z), `90` = starboard (+X), `180` = aft (+Z),
/// `-90` / `270` = port. Wraps freely; only the wrapped direction
/// matters.
///
/// `fire_arc_deg` is the full arc width centred on `facing_deg`. A bank
/// with `facing_deg = -90`, `fire_arc_deg = 180` covers the port
/// hemisphere from forward to aft. Values must be in `(0, 360]`.
///
/// `auto_arc_deg` is the (narrower) auto-fire window, also centred on
/// `facing_deg`. Must satisfy `0 < auto_arc_deg <= fire_arc_deg`.
///
/// `beam_range` is in world units. When `0.0`, falls back to
/// `PhaserCombatConfig::DEFAULT_PHASER_RANGE`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct PhaserBankConfig {
    pub id: PhaserBankId,
    pub facing_deg: f32,
    pub fire_arc_deg: f32,
    pub auto_arc_deg: f32,
    #[serde(default)]
    pub beam_range: f32,
    /// Damage applied to the target per second of active beam.
    /// When `0.0`, falls back to `PhaserCombatConfig::DEFAULT_BEAM_DAMAGE_PER_SEC`.
    #[serde(default)]
    pub beam_damage_per_sec: f32,
    /// Active beam duration in seconds. When `0.0`, falls back to
    /// `PhaserCombatConfig::DEFAULT_BEAM_DURATION_SECS`.
    #[serde(default)]
    pub beam_duration_secs: f32,
    /// Post-beam cooldown in seconds. When `0.0`, falls back to
    /// `PhaserCombatConfig::DEFAULT_BEAM_COOLDOWN_SECS`.
    #[serde(default)]
    pub cooldown_secs: f32,
    /// RGBA beam colour as a 4-element float array `[r, g, b, a]` in 0.0–1.0.
    /// When absent (empty vec), the renderer falls back to `beam_render::DEFAULT_BEAM_COLOR`.
    #[serde(default)]
    pub beam_color: Vec<f32>,
    /// Fraction of beam damage that bypasses shields. When `None`, defaults to `0.0`.
    /// Clamped to `[0.0, 1.0]` at apply time.
    #[serde(default)]
    pub shield_pierce: Option<f32>,
    /// Optional rig-marker name linking this bank to a mount point in the
    /// model's rig sidecar (`[markers.<name>]`). When resolvable, downstream
    /// systems may use the marker's position/direction as the beam origin;
    /// when absent or unresolved they fall back to the hull-offset default.
    #[serde(default)]
    pub marker: Option<String>,
    /// Inline stateless AI policy for this bank's open-fire decision
    /// (issue #781). When authored it is validated at content load and drives
    /// `ai_phaser_auto_fire`'s per-bank fire gate; when absent the canonical
    /// [`default_phaser_bank_ai_config`] (unconditional fire) is synthesised at
    /// spawn so baseline auto-fire is preserved. An explicit `idle = true` is the
    /// per-bank opt-out (AC1).
    #[serde(default)]
    pub ai: Option<FineSystemAiConfigToml>,
}

/// Stable identifier for a blaster bank, parsed verbatim from the TOML
/// `id` field on `[[weapons_console.blaster_banks]]` (issue #631).
pub type BlasterBankId = String;

/// One `[[weapons_console.blaster_banks]]` entry (issue #631).
///
/// A blaster bank fires straight-flying projectiles in data-driven volleys
/// with linear motion prediction at fire time — no homing, no mid-flight
/// correction.
///
/// `facing_deg` and `fire_arc_deg` use the same convention as
/// [`PhaserBankConfig`] (ship-local degrees, 0 = forward). Note: do NOT
/// add `serde(deny_unknown_fields)` here — future issues will add more
/// fields (recoil, screenshake, visual variants).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct BlasterBankConfig {
    pub id: BlasterBankId,
    #[serde(default)]
    pub facing_deg: f32,
    #[serde(default = "default_blaster_fire_arc_deg")]
    pub fire_arc_deg: f32,
    #[serde(default = "default_blaster_volley_count")]
    pub volley_count: u32,
    #[serde(default = "default_blaster_volley_interval_secs")]
    pub volley_interval_secs: f32,
    /// Post-volley cooldown in seconds.
    #[serde(default = "default_blaster_cooldown_secs")]
    pub cooldown_secs: f32,
    /// Charge time before firing begins. `0` = instant (click-to-fire);
    /// `>0` = hold-to-fire (reserved for a later issue).
    #[serde(default)]
    pub charge_time_secs: f32,
    #[serde(default = "default_blaster_projectile_speed")]
    pub projectile_speed: f32,
    #[serde(default = "default_blaster_collision_radius")]
    pub collision_radius: f32,
    #[serde(default = "default_blaster_visual_scale")]
    pub visual_scale: f32,
    #[serde(default = "default_blaster_damage")]
    pub damage: i32,
    /// Fraction `[0.0, 1.0]` of damage that bypasses shields entirely.
    #[serde(default)]
    pub shield_pierce: f32,
    /// Recoil impulse magnitude (reserved for a later issue).
    #[serde(default)]
    pub recoil_impulse: f32,
    /// Screenshake magnitude (reserved for a later issue).
    #[serde(default)]
    pub screenshake_magnitude: f32,
    /// Optional rig-marker name linking this bank to a mount point. In the
    /// single-barrel (backward-compat) case this is the sole projectile origin.
    #[serde(default)]
    pub marker: Option<String>,
    /// Authored barrel-marker names (issue #765). Each entry is a rig-marker
    /// name; a barrel-index pattern step addresses these by position. When
    /// empty the bank has one implicit barrel = `marker` (unchanged behaviour).
    #[serde(default)]
    pub barrels: Vec<String>,
    /// Timed multi-barrel firing pattern (issue #765). A step fires its listed
    /// barrel indices simultaneously at `offset_secs`; successive steps at
    /// increasing offsets alternate. When empty the bank fires the uniform
    /// `volley_count` volley from the single implicit barrel (unchanged).
    #[serde(default)]
    pub pattern: crate::weapons::pattern::BarrelPattern,
    /// Maximum range in world units. Projectile lifespan is computed per-bank
    /// as `range / projectile_speed`. Use `default_blaster_range` (35.0) when
    /// absent from TOML.
    #[serde(default = "default_blaster_range")]
    pub range: f32,
    /// Inline stateless AI policy for this bank's open-fire decision
    /// (issue #781). When authored it is validated at content load and drives
    /// `tick_blaster_auto_fire`'s per-bank fire gate; when absent the canonical
    /// [`default_blaster_bank_ai_config`] (unconditional fire) is synthesised at
    /// spawn so baseline auto-fire is preserved. An explicit `idle = true` is the
    /// per-bank opt-out (AC1).
    #[serde(default)]
    pub ai: Option<FineSystemAiConfigToml>,
}

fn default_blaster_fire_arc_deg() -> f32 {
    90.0
}
fn default_blaster_volley_count() -> u32 {
    3
}
fn default_blaster_volley_interval_secs() -> f32 {
    0.15
}
fn default_blaster_cooldown_secs() -> f32 {
    3.0
}
fn default_blaster_projectile_speed() -> f32 {
    40.0
}
fn default_blaster_collision_radius() -> f32 {
    1.5
}
fn default_blaster_visual_scale() -> f32 {
    1.0
}
fn default_blaster_damage() -> i32 {
    20
}
fn default_blaster_range() -> f32 {
    35.0
}

impl BlasterBankConfig {
    /// Convert this TOML config into a runtime `crate::weapons::blaster::BlasterBankConfig`.
    pub fn to_runtime(&self) -> crate::weapons::blaster::BlasterBankConfig {
        crate::weapons::blaster::BlasterBankConfig {
            id: self.id.clone(),
            facing_deg: self.facing_deg,
            fire_arc_deg: self.fire_arc_deg,
            volley_count: self.volley_count,
            volley_interval_secs: self.volley_interval_secs,
            cooldown_secs: self.cooldown_secs,
            charge_time_secs: self.charge_time_secs,
            projectile_speed: self.projectile_speed,
            collision_radius: self.collision_radius,
            visual_scale: self.visual_scale,
            damage: self.damage,
            shield_pierce: self.shield_pierce,
            recoil_impulse: self.recoil_impulse,
            screenshake_magnitude: self.screenshake_magnitude,
            marker: self.marker.clone(),
            barrels: self.barrels.clone(),
            pattern: self.pattern.clone(),
            range: self.range,
        }
    }
}

/// Stable identifier for a torpedo tube, parsed verbatim from the TOML
/// `id` field on `[[torpedoes.tubes]]`. Used on the wire to address a
/// specific tube (e.g. `FireTorpedo { tube: "fore_port" }`).
pub type TorpedoTubeId = String;

/// One `[[torpedoes.tubes]]` entry. Defines a single torpedo tube's
/// orientation and launch arc. Ammo is **not** per-tube; the entire
/// ship draws from the shared `[torpedoes].count` pool.
///
/// `facing_deg` and `fire_arc_deg` use the same convention as
/// [`PhaserBankConfig`] (ship-local degrees, 0 = forward).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TorpedoTubeConfig {
    pub id: TorpedoTubeId,
    pub facing_deg: f32,
    pub fire_arc_deg: f32,
    /// Per-tube load/unload time override in seconds. Falls back to the
    /// global `[torpedoes] load_time` when absent.
    #[serde(default)]
    pub load_time: Option<f32>,
    /// Optional rig-marker name linking this tube to a mount point in the
    /// model's rig sidecar (`[markers.<name>]`). When absent or unresolved,
    /// callers fall back to the ship-centre launch origin. In the single-barrel
    /// (backward-compat) case this is the sole launch origin.
    #[serde(default)]
    pub marker: Option<String>,
    /// Authored barrel-marker names (issue #766). Each entry is a rig-marker
    /// name; a barrel-index pattern step addresses these by position. When
    /// empty the tube has one implicit barrel = `marker` (unchanged behaviour).
    /// Reuses the exact schema blasters wired in issue #765.
    #[serde(default)]
    pub barrels: Vec<String>,
    /// Timed multi-barrel firing pattern (issue #766). A step lists barrel
    /// indices; successive steps at increasing offsets order the barrels a
    /// volley's rounds leave from. The pattern governs only WHICH barrel each
    /// launched round leaves from and in what order — never how many rounds
    /// exist: the magazine, `loaded_count`, and the burst cadence remain the
    /// sole authority over the torpedo count. When empty the tube launches
    /// from the single implicit barrel exactly as before.
    #[serde(default)]
    pub pattern: crate::weapons::pattern::BarrelPattern,
    /// Maximum number of torpedoes that can be loaded into this tube at once
    /// (volley capacity). Default `1` preserves existing single-shot
    /// behaviour. Values greater than 1 allow the tube to queue multiple
    /// torpedoes and fire them as a rapid burst.
    #[serde(default = "default_tube_volley_max")]
    pub volley_max: u32,
    /// How many rounds an AI-operated crew keeps loaded in this tube.
    ///
    /// The AI has no console to poke, so it issues the same
    /// `SetTorpedoVolleyTarget` command a human operator's console sends
    /// (see `console_ai::server::ai_torpedo_load`) and this is the count it
    /// asks for. Falls back to `[torpedoes] ai_volley_target`, then to
    /// [`Self::volley_max`] — a designer who says nothing gets "the AI keeps
    /// the tube as full as it can", which is the sane default for a hull that
    /// authored tubes at all. Clamped to `volley_max` at runtime.
    /// `Some(0)` disables AI loading for this tube.
    #[serde(default)]
    pub ai_target_count: Option<u32>,
    /// Inline stateless AI policy for this tube's load + launch decisions
    /// (issue #782). When authored it is validated at content load and drives
    /// `ai_torpedo_load`'s per-tube load gate and `ai_torpedo_auto_fire`'s
    /// per-tube launch gate; when absent the canonical
    /// [`default_torpedo_tube_ai_config`] (unconditional load + launch) is
    /// synthesised at spawn so baseline behaviour is preserved. An explicit
    /// `idle = true` is the per-tube opt-out (AC1).
    #[serde(default)]
    pub ai: Option<FineSystemAiConfigToml>,
}

fn default_tube_volley_max() -> u32 {
    1
}

/// Validate a `[[weapons_console.phaser_banks]]` list parsed from TOML.
///
/// Rejects:
///   - empty list (caller may decide to fall back to a single hardcoded
///     bank — this validator returns `Err` so callers see the empty list)
///   - duplicate `id` values
///   - `fire_arc_deg` outside `(0, 360]`
///   - `auto_arc_deg` outside `(0, fire_arc_deg]`
pub fn validate_phaser_banks(banks: &[PhaserBankConfig]) -> Result<(), String> {
    if banks.is_empty() {
        return Err("phaser_banks list is empty".into());
    }
    let mut seen = std::collections::HashSet::new();
    for b in banks {
        if !seen.insert(b.id.as_str()) {
            return Err(format!("duplicate phaser bank id '{}'", b.id));
        }
        if !(b.fire_arc_deg > 0.0 && b.fire_arc_deg <= 360.0) {
            return Err(format!(
                "phaser bank '{}' has fire_arc_deg={} outside (0, 360]",
                b.id, b.fire_arc_deg
            ));
        }
        if !(b.auto_arc_deg > 0.0 && b.auto_arc_deg <= b.fire_arc_deg) {
            return Err(format!(
                "phaser bank '{}' has auto_arc_deg={} outside (0, fire_arc_deg={}]",
                b.id, b.auto_arc_deg, b.fire_arc_deg
            ));
        }
    }
    Ok(())
}

/// Validate a `[[weapons_console.blaster_banks]]` list parsed from TOML
/// (issue #765).
///
/// An empty list is accepted — most hulls carry no blasters. Rejects:
///   - duplicate `id` values,
///   - `fire_arc_deg` outside `(0, 360]`,
///   - a barrel pattern that fires no barrels in a step, references a barrel
///     index beyond the declared barrel count, uses a negative offset, or is
///     omitted while more than one barrel is declared (see
///     [`crate::weapons::pattern::validate_barrel_pattern`]).
///
/// The barrel count is the authored `barrels.len()`, or `1` for the implicit
/// single-barrel (backward-compat) bank.
pub fn validate_blaster_banks(banks: &[BlasterBankConfig]) -> Result<(), String> {
    let mut seen = std::collections::HashSet::new();
    for b in banks {
        if !seen.insert(b.id.as_str()) {
            return Err(format!("duplicate blaster bank id '{}'", b.id));
        }
        if !(b.fire_arc_deg > 0.0 && b.fire_arc_deg <= 360.0) {
            return Err(format!(
                "blaster bank '{}' has fire_arc_deg={} outside (0, 360]",
                b.id, b.fire_arc_deg
            ));
        }
        let barrel_count = if b.barrels.is_empty() {
            1
        } else {
            b.barrels.len()
        };
        crate::weapons::pattern::validate_barrel_pattern(
            &format!("blaster bank '{}'", b.id),
            barrel_count,
            &b.pattern,
        )?;
    }
    Ok(())
}

/// Validate a `[[torpedoes.tubes]]` list parsed from TOML.
///
/// Rejects: empty list, duplicate `id`, `fire_arc_deg` outside `(0, 360]`, and
/// (issue #766) a barrel pattern that fires no barrels in a step, references a
/// barrel index beyond the declared barrel count, uses a negative offset, or is
/// omitted while more than one barrel is declared (see
/// [`crate::weapons::pattern::validate_barrel_pattern`]).
///
/// The barrel count is the authored `barrels.len()`, or `1` for the implicit
/// single-barrel (backward-compat) tube.
pub fn validate_torpedo_tubes(tubes: &[TorpedoTubeConfig]) -> Result<(), String> {
    if tubes.is_empty() {
        return Err("torpedo tubes list is empty".into());
    }
    let mut seen = std::collections::HashSet::new();
    for t in tubes {
        if !seen.insert(t.id.as_str()) {
            return Err(format!("duplicate torpedo tube id '{}'", t.id));
        }
        if !(t.fire_arc_deg > 0.0 && t.fire_arc_deg <= 360.0) {
            return Err(format!(
                "torpedo tube '{}' has fire_arc_deg={} outside (0, 360]",
                t.id, t.fire_arc_deg
            ));
        }
        let barrel_count = if t.barrels.is_empty() {
            1
        } else {
            t.barrels.len()
        };
        crate::weapons::pattern::validate_barrel_pattern(
            &format!("torpedo tube '{}'", t.id),
            barrel_count,
            &t.pattern,
        )?;
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeaponsConsoleConfig {
    /// RGBA colour used by the client Tactical UI for torpedo fire-arc
    /// overlays. When absent, the `ShipClientConfig` default is used.
    #[serde(default)]
    pub torpedo_arc_color: Vec<f32>,
    #[serde(default)]
    pub power_multipliers: Option<[f32; 4]>,
    /// Per-bank phaser definitions parsed from
    /// `[[weapons_console.phaser_banks]]`. Each bank has its own facing,
    /// fire arc, auto-fire arc, range, damage, duration, cooldown, and colour.
    #[serde(default)]
    pub phaser_banks: Vec<PhaserBankConfig>,
    /// Per-bank blaster definitions parsed from
    /// `[[weapons_console.blaster_banks]]` (issue #631). Each bank has its own
    /// facing, fire arc, volley count, damage, shield pierce, and cooldown.
    #[serde(default)]
    pub blaster_banks: Vec<BlasterBankConfig>,
    /// Radar configuration for the Tactical console radar widget, from
    /// `[weapons_console.radar]`.
    #[serde(default)]
    pub radar: Option<crate::radar_config::RadarConfig>,
    /// Inline per-system target selector (issue #777). Loaded from
    /// `[weapons_console.selector]`; absent ⇒ the canonical
    /// [`default_tactical_target_selector_config`] is synthesised at spawn.
    /// Mirrors [`SensorsConsoleConfig::selector`] — the Tactical host ranks its
    /// own candidates independently and remains the sole writer of the
    /// authoritative `TacticalRadarSelection`.
    #[serde(default)]
    pub selector: Option<FineSystemAiSelectorToml>,
    /// Explicit Tactical-radar idle declaration (issue #781, AC6). When `true`
    /// the radar takes NO AI target selection — `ai_target_selection` clears any
    /// stale lock and skips the ship — even when a tactical fine system is
    /// AI-operated. This is the explicit AI-or-idle opt-out that distinguishes
    /// "the radar deliberately makes no AI selection" from "no selector authored
    /// → default selector". Defaults to `false` (radar runs its selector as
    /// before), so baseline behaviour is preserved.
    #[serde(default)]
    pub selector_idle: bool,
    /// Inline stateless AI policy for the ship's WEAPONS DOCTRINE fine system
    /// (`[weapons_console.ai]`, issue #956).
    ///
    /// The ship-level counterpart of the per-bank/per-tube `ai` blocks below:
    /// those say *when this emitter opens fire*, and this one says *which family
    /// the ship turns to bring to bear* when the target is in range but outside
    /// every arc of a family. It drives the channel-3 `ArcBearingRequest` Weapons
    /// sends Helm, over the `arc_bearing_first` / `arc_bearing_second` /
    /// `arc_bearing_third` channels — the rank ladder that replaced the Rust
    /// `[Phasers, Blasters, Torpedoes]` array in `tick_weapons_arc_request`.
    ///
    /// Authored in `fragments/ai/fleet_baseline.toml` for every hull that
    /// composes the ship-level spine, so a hull with no preference of its own
    /// resolves the FLEET BASELINE rather than an inline Rust order.
    #[serde(default)]
    pub ai: Option<FineSystemAiConfigToml>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EngineeringConsoleConfig {}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct CaptainConsoleConfig {
    /// Inline stateless AI policy for the Captain's Red Alert fine system
    /// (`[captain_console.ai]`, issue #775). When present it is validated at
    /// content load and drives `operate_captain_ai`; when absent the canonical
    /// [`default_captain_ai_config`] policy is synthesised at spawn.
    #[serde(default)]
    pub ai: Option<FineSystemAiConfigToml>,
}

/// Config block for the Comms CONSOLE's AI (issue #786), loaded from
/// `[comms_console]`.
///
/// Deliberately separate from the top-level `[comms]` section: that one is the
/// per-ENTITY comms RANGE (`CommsConfig`), present on stations and NPCs that are
/// merely hailable, and has nothing to do with who operates the console. The AI
/// policy belongs to the console, next to `[captain_console.ai]` and
/// `[sensors_console.selector]`.
///
/// Comms is the FIRST system to author BOTH fine-system AI machines: a #776
/// `selector` (WHO to hail — a variable candidate set keyed by real contact
/// UUIDs) and a #775 channel/verb `ai` policy (HOW to answer an open dialogue —
/// a fixed, index-addressed response list). See [`COMMS_RESPOND_CHANNEL`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct CommsConsoleConfig {
    /// Inline per-system target selector for hail target ranking (issue #786).
    /// Loaded from `[comms_console.selector]`; absent ⇒ the canonical
    /// [`default_comms_target_selector_config`] is synthesised at spawn.
    /// Validated in [`EntityConfig::from_toml`] against
    /// [`COMMS_SELECTOR_SOURCES`].
    #[serde(default)]
    pub selector: Option<FineSystemAiSelectorToml>,
    /// Inline stateless AI policy for the Comms dialogue-response fine system
    /// (issue #786). Loaded from `[comms_console.ai]`; absent ⇒ the canonical
    /// [`default_comms_response_ai_config`] is synthesised at spawn (baseline
    /// preservation). Validated against [`COMMS_RESPOND_CHANNELS`] /
    /// [`COMMS_RESPOND_VERBS`].
    #[serde(default)]
    pub ai: Option<FineSystemAiConfigToml>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CinematicCameraConfig {
    /// Offset from ship centre in ship-local space (Y-up, Z-behind).
    /// e.g. [0, 8, 15] means 8 units above and 15 units behind.
    pub position: [f32; 3],
    /// Downward pitch from horizontal, in degrees. e.g. 15.0
    #[serde(default = "default_cinematic_pitch")]
    pub default_pitch_deg: f32,
    /// Maximum distance (world units) to consider entities for tracking.
    #[serde(default = "default_cinematic_look_range")]
    pub entity_look_range: f32,
    /// Distance of the default look-ahead point when no entity is tracked.
    #[serde(default = "default_cinematic_look_ahead")]
    pub look_ahead_distance: f32,
    /// Minimum seconds between target re-evaluations (hysteresis).
    #[serde(default = "default_cinematic_hysteresis")]
    pub hysteresis_secs: f32,
    /// How fast (degrees/second) the chase camera's yaw catches up to the
    /// ship's actual heading. A rigid 1:1 lock makes the ship look frozen in
    /// frame during turns (camera and hull rotate identically), so the
    /// camera intentionally lags behind and lets the ship's turn be visible.
    #[serde(default = "default_cinematic_yaw_follow_rate")]
    pub yaw_follow_deg_per_sec: f32,
}

fn default_cinematic_pitch() -> f32 {
    15.0
}
fn default_cinematic_look_range() -> f32 {
    60.0
}
fn default_cinematic_look_ahead() -> f32 {
    100.0
}
fn default_cinematic_hysteresis() -> f32 {
    3.0
}
fn default_cinematic_yaw_follow_rate() -> f32 {
    45.0
}

// ── Inline stateless fine-system AI policy (issue #775) ───────────────────────

/// The `red_alert` output channel: the one channel the Captain policy drives.
pub const CAPTAIN_RED_ALERT_CHANNEL: &str = "red_alert";
/// The `set_red_alert` verb: the one typed verb the Captain policy emits.
pub const CAPTAIN_SET_RED_ALERT_VERB: &str = "set_red_alert";

// ── Captain first-contact facts (issue #912) ─────────────────────────────────
//
// Before #912 the Captain host seeded exactly ONE fact, `secs_since_combat`,
// and that timer starts only on damage taken, hostile fire taken, or own weapon
// fired. Since #872 a backfilled Alliance hull's own weapons hold fire until Red
// Alert, so the loop closed: such a hull could only ever RETURN fire, and the
// authoring surface could not express first contact at all. These two readings
// are what an authored guard needs to open an engagement, and they are seeded by
// `operate_captain_ai` — no Rust branch decides the alert.
//
// BOTH are seeded UNCONDITIONALLY, every evaluation. An absent fact makes every
// comparison against it read false, which is indistinguishable from "clear" and
// hides a guard that was never wired up (the #779 shape).

/// `1.0` when this ship has a faction-hostile contact in the shared
/// `WorldSnapshot`, `0.0` when it has none. Always seeded.
pub const CAPTAIN_HOSTILE_CONTACT_FACT: &str = "hostile_contact";
/// Planar distance to that nearest hostile contact, world units. Always seeded,
/// and reads `0.0` when there is no contact at all — which is precisely why an
/// authored guard must pair it with [`CAPTAIN_HOSTILE_CONTACT_FACT`] rather than
/// comparing it against a threshold on its own.
pub const CAPTAIN_HOSTILE_RANGE_FACT: &str = "hostile_range";

// ── Helm fine-system AI policy channels/verbs (issue #779) ────────────────────
//
// Engines and Steering are the first *continuous* fine actuators to move onto
// the data-authored #775 policy spine. Each drives a single output channel with
// a single value-less **mode** verb: the verb decides *whether* to actuate this
// tick, while the continuous thrust/yaw magnitude stays sourced from the shared
// `DesiredMotion` planner fact (AGENTS.md rule #11 — no geometry pinned here).

/// The `longitudinal` output channel: the Engines fine system's thrust axis.
pub const HELM_LONGITUDINAL_CHANNEL: &str = "longitudinal";
/// The `actuate_desired_travel` verb: the Engines mode verb. Its presence tells
/// the host to emit `SetThrust` with the scalar decoded from the planner's
/// `desired_velocity_local`; its absence ("hold") emits nothing.
pub const HELM_ACTUATE_DESIRED_TRAVEL_VERB: &str = "actuate_desired_travel";

/// The `yaw` output channel: the Steering fine system's turn axis.
pub const HELM_YAW_CHANNEL: &str = "yaw";
/// The `actuate_desired_facing` verb: the Steering mode verb. Its presence tells
/// the host to emit `SetSteering` with the scalar decoded from the planner's
/// `desired_facing_local`; its absence ("hold") emits nothing.
pub const HELM_ACTUATE_DESIRED_FACING_VERB: &str = "actuate_desired_facing";
/// The `hold_committed_heading` verb: the Steering fine system's SECOND mode
/// verb (issue #883). Its presence tells the host to fly the heading frozen in
/// this system's own `memory(escape_heading_rad)` at the last committed
/// transition, rather than re-solving the facing against a moving target.
/// Distinct from a "hold" (no rule fires), which holds the last steering
/// COMMAND and would keep a non-zero yaw turning for ever.
pub const HELM_HOLD_COMMITTED_HEADING_VERB: &str = "hold_committed_heading";
/// The `hold_recovery_orbit` verb: the Steering fine system's THIRD mode verb
/// (issue #788). Its presence tells the host to fly a tangent of the safe ring
/// around the current target — radius derived from that target's own direct-fire
/// reach plus this hull's authored `safe_range_margin`, circulation direction
/// taken from this system's host-written `memory(orbit_direction)`.
pub const HELM_HOLD_RECOVERY_ORBIT_VERB: &str = "hold_recovery_orbit";
/// The `pivot_to_reengage` verb: the Steering fine system's FOURTH mode verb
/// (issue #788). Tracks the target like `actuate_desired_facing`, but the host
/// pairs it with the authored `reengage_speed` throttle rather than the approach
/// throttle — the cut-thrust pivot that ends a recovery and starts the next run.
pub const HELM_PIVOT_TO_REENGAGE_VERB: &str = "pivot_to_reengage";
/// The `hold_combat_orbit` verb: the Steering fine system's FIFTH mode verb
/// (issue #790). Its presence tells the host to fly a tangent of a ring around
/// the current target whose radius is the hull's own authored
/// `combat_orbit_range` — a fighting range, not a standoff derived from the
/// target's reach — with the circulation direction taken from this system's
/// host-written `memory(orbit_direction)`.
pub const HELM_HOLD_COMBAT_ORBIT_VERB: &str = "hold_combat_orbit";
/// The `hold_torpedo_bearing` verb: the Steering fine system's SIXTH mode verb
/// (issue #791). Tracks the target's live position like `actuate_desired_facing`,
/// but the host pairs it with the authored `torpedo_bearing_speed` throttle
/// rather than with doctrine travel — the bow-on, thrust-cut hold a hull flies
/// while a fixed forward tube lines up on a shield facing that has gone down.
///
/// Deliberately NOT a reuse of `pivot_to_reengage`, whose geometry is the same
/// but whose host gate is the six-scalar shield-RECOVERY parameter set: a hull
/// with no standoff doctrine would have to invent all six to borrow it.
pub const HELM_HOLD_TORPEDO_BEARING_VERB: &str = "hold_torpedo_bearing";
/// The `hold_artillery_position` verb: the Steering fine system's SEVENTH mode
/// verb (issue #792). Tells the host to hold translational station on the
/// authored `artillery_hold_speed` while pivoting the bow onto a PREDICTIVE
/// intercept solution — where the target will be when this hull's own artillery
/// bolt arrives, not where it is now.
///
/// Deliberately NOT a reuse of `pivot_to_reengage` (whose host gate is the six
/// shield-RECOVERY scalars, all describing a standoff ring derived from the
/// TARGET's reach) nor of `hold_torpedo_bearing` (which tracks the target's live
/// position with no lead at all — the right answer for a fixed tube at knife
/// range and the wrong one for a slow bolt with seconds of flight time).
pub const HELM_HOLD_ARTILLERY_POSITION_VERB: &str = "hold_artillery_position";

/// The verbs a Steering (`yaw`) policy may emit
/// (issues #779, #883, #788, #790, #791, #792).
pub const HELM_STEERING_VERBS: &[&str] = &[
    HELM_ACTUATE_DESIRED_FACING_VERB,
    HELM_HOLD_COMMITTED_HEADING_VERB,
    HELM_HOLD_RECOVERY_ORBIT_VERB,
    HELM_PIVOT_TO_REENGAGE_VERB,
    HELM_HOLD_COMBAT_ORBIT_VERB,
    HELM_HOLD_TORPEDO_BEARING_VERB,
    HELM_HOLD_ARTILLERY_POSITION_VERB,
];

// ── Helm secondary fine-actuator AI policy channels/verbs (issue #780) ────────
//
// Lateral thrust, bounded vertical thrust, impulse, and boost move onto the same
// #775 policy spine. Each drives a single output channel with a single value-less
// MODE verb: the verb decides *whether* to actuate this tick, while the
// continuous magnitude / engage-vs-cancel decision stays sourced host-side from
// geometry, capability, and the shared hazard assessment (AGENTS.md rule #11 —
// no gameplay scalar pinned in the verb).

/// The `lateral` output channel: the Lateral Thrust fine system's dodge axis.
pub const HELM_LATERAL_CHANNEL: &str = "lateral";
/// The `actuate_lateral_thrust` verb: the Lateral Thrust mode verb. Its presence
/// lets the host emit `LateralThrustInput` with the magnitude from the shared
/// hazard surface (or docking translation); its absence ("hold") emits nothing.
pub const HELM_ACTUATE_LATERAL_THRUST_VERB: &str = "actuate_lateral_thrust";

/// The `vertical` output channel: the Vertical Thrust fine system's climb axis.
pub const HELM_VERTICAL_CHANNEL: &str = "vertical";
/// The `actuate_vertical_thrust` verb: the Vertical Thrust mode verb. Its
/// presence lets the host emit `VerticalThrustInput` with the climb/return
/// magnitude gated on the authored `VerticalMovementMode`; its absence emits
/// nothing.
pub const HELM_ACTUATE_VERTICAL_THRUST_VERB: &str = "actuate_vertical_thrust";

/// The `impulse` output channel: the Impulse fine system's engage/cancel axis.
pub const HELM_IMPULSE_CHANNEL: &str = "impulse";
/// The `engage_impulse` verb: the Impulse mode verb. Its presence permits the
/// impulse manoeuvre this tick; the host still applies the authored doctrine
/// `use_impulse` and the `decide_impulse` geometry. Its absence ("hold") emits
/// nothing.
pub const HELM_ENGAGE_IMPULSE_VERB: &str = "engage_impulse";

/// The `boost` output channel: the Boost fine system's engage axis.
pub const HELM_BOOST_CHANNEL: &str = "boost";
/// The `engage_boost` verb: the Boost mode verb. Its presence drives the ship's
/// boost active via the same admitted `SetBoost` a human uses; its absence
/// ("hold"/idle) leaves boost as it is.
pub const HELM_ENGAGE_BOOST_VERB: &str = "engage_boost";

// ── Weapon-bank fine-system AI policy channels/verbs (issue #781) ─────────────
//
// Each AI-capable phaser and blaster bank drives a single output channel with a
// single value-less ACTION verb: the verb decides *whether* to open fire this
// tick, while the target (the ship's authoritative combat lock), the firing
// bank, and the beam frequency all come from the host context — never the verb
// (AGENTS.md rule #11 — no fire thresholds/ranges/arcs/cooldowns pinned here;
// those stay TOML on the bank configs). Each bank enforces availability,
// cooldown, range, arc, and target validity host-side before resolving its
// policy, so the runtime only reports *whether the authored behaviour permits*
// firing an already-ready bank.

/// The `phaser_fire` output channel: a phaser bank's open-fire axis.
pub const PHASER_FIRE_CHANNEL: &str = "phaser_fire";
/// The `fire_phaser` verb: the phaser-bank fire verb. Its presence tells the
/// host to emit the same admitted `FirePhaser` a human does; its absence
/// ("hold"/idle) holds this bank's fire.
pub const PHASER_FIRE_VERB: &str = "fire_phaser";

/// The registered output channels a phaser bank policy may drive (issue #781).
pub const PHASER_BANK_CHANNELS: &[&str] = &[PHASER_FIRE_CHANNEL];
/// The registered verbs a phaser bank policy may emit (issue #781).
pub const PHASER_BANK_VERBS: &[&str] = &[PHASER_FIRE_VERB];

/// The `blaster_fire` output channel: a blaster bank's open-fire axis.
pub const BLASTER_FIRE_CHANNEL: &str = "blaster_fire";
/// The `fire_blaster` verb: the blaster-bank fire verb. Its presence tells the
/// host to emit the same admitted `ChargeBlasterStart` a human does; its absence
/// ("hold"/idle) holds this bank's volley.
pub const BLASTER_FIRE_VERB: &str = "fire_blaster";

/// The registered output channels a blaster bank policy may drive (issue #781).
pub const BLASTER_BANK_CHANNELS: &[&str] = &[BLASTER_FIRE_CHANNEL];
/// The registered verbs a blaster bank policy may emit (issue #781).
pub const BLASTER_BANK_VERBS: &[&str] = &[BLASTER_FIRE_VERB];

// ── Torpedo tube + magazine fine-system AI policy channels/verbs (issue #782) ─
//
// A torpedo tube is a two-stage pipeline owned by two fine systems: the TUBE
// decides whether to LOAD (reserve a round from the shared magazine) and whether
// to LAUNCH (fire an already-loaded round), while the shared MAGAZINE arbitrates
// whether to GRANT a pending reservation. Every verb is value-less: the tube, its
// authored volley target, the ship's authoritative combat lock, and all
// thresholds stay TOML/host-side, never in the verb (AGENTS.md rule #11). The
// host enforces loaded state, magazine availability, target validity, range, and
// arc before resolving these policies, so the runtime only reports *whether the
// authored behaviour permits* the load/launch/grant of an already-ready stage.

/// The `torpedo_load` output channel: a tube's load-a-round axis.
pub const TORPEDO_LOAD_CHANNEL: &str = "torpedo_load";
/// The `load_torpedo` verb. Its presence tells the host to emit the same admitted
/// `SetTorpedoVolleyTarget` a Tactical player does; its absence ("hold"/idle)
/// leaves the tube's volley target where it is.
pub const TORPEDO_LOAD_VERB: &str = "load_torpedo";

/// The `torpedo_launch` output channel: a tube's launch-a-loaded-round axis.
pub const TORPEDO_LAUNCH_CHANNEL: &str = "torpedo_launch";
/// The `launch_torpedo` verb. Its presence tells the host to emit the same
/// admitted `FireTorpedo` a human does; its absence ("hold"/idle) holds fire.
pub const TORPEDO_LAUNCH_VERB: &str = "launch_torpedo";

/// The registered output channels a torpedo tube policy may drive (issue #782).
pub const TORPEDO_TUBE_CHANNELS: &[&str] = &[TORPEDO_LOAD_CHANNEL, TORPEDO_LAUNCH_CHANNEL];
/// The registered verbs a torpedo tube policy may emit (issue #782).
pub const TORPEDO_TUBE_VERBS: &[&str] = &[TORPEDO_LOAD_VERB, TORPEDO_LAUNCH_VERB];

/// The `torpedo_magazine_grant` output channel: the shared magazine's
/// grant-a-claim axis, resolved inside the single magazine consumer.
pub const TORPEDO_MAGAZINE_CHANNEL: &str = "torpedo_magazine_grant";
/// The `grant_torpedo_round` verb. Its presence permits a pending
/// `ClaimTorpedoRound` reservation to proceed; its absence ("hold"/idle) refuses
/// the claim without touching the magazine counter.
pub const TORPEDO_MAGAZINE_GRANT_VERB: &str = "grant_torpedo_round";

// ── Torpedo conservation: magazine vs remaining mission (issue #943) ─────────
//
// The second channel the shared magazine drives, and the one place a torpedo
// launch is gated SYMMETRICALLY: it is resolved inside `handle_fire_torpedo`,
// the single consumer of `SystemControlPayload::FireTorpedo` for every ship,
// which admission reaches with the source identity already stripped. A human
// Tactical operator's launch and an AI backfill's launch pass through the same
// resolve, and nothing below admission may ask which one it was (AGENTS.md #6).
//
// The question it answers is not "may this weapon fire" — the tube's own
// `torpedo_launch` doctrine already answers that for the AI, and red alert
// answers it for the ship — but "can this ship AFFORD to spend a round here,
// given how much of the mission is still ahead". That measure is WORLD-scoped:
// the scenario publishes [`MISSION_THREAT_REMAINING_COUNTER`] and the host reads
// it off the ship's own layered flag chain, so the same hull paces differently
// in an eight-wave defence and in a single-target strike, with no per-hull
// constant anywhere.

/// The `torpedo_conservation` output channel: the shared magazine's
/// spend-a-round-here axis, resolved once per ship per tick, ahead of that
/// ship's admitted command loop.
pub const TORPEDO_CONSERVATION_CHANNEL: &str = "torpedo_conservation";
/// The `release_torpedo` verb. Its presence permits an already-authorised
/// launch to spend its round; its absence ("hold"/idle) holds the round for
/// later in the mission WITHOUT touching the magazine or the tube — the round
/// stays loaded and the same decision is offered again next tick.
///
/// A magazine policy that authors NO rule on
/// [`TORPEDO_CONSERVATION_CHANNEL`] is unconstrained: conservation is content,
/// so a hull (or a whole fleet) that never authors it fires exactly as it did
/// before this channel existed.
pub const TORPEDO_RELEASE_VERB: &str = "release_torpedo";

/// The WORLD counter a scenario publishes to say how much of the mission's
/// threat is still ahead of the ships flying it (issue #943).
///
/// Engine vocabulary, not a gameplay value: the NAME is fixed so a hull's
/// doctrine can be written once and paced by any scenario, while the NUMBER —
/// how much threat a mission poses, and when each unit of it is cleared — is
/// authored entirely in world TOML through the ordinary `set_flag_value` /
/// `increment_flag` trigger actions. `assets/worlds/combat_test.toml` sets it to
/// its eight-wave schedule and decrements it as each wave dies.
///
/// A world that publishes nothing leaves it at the unset default of `0`, which
/// the host reads as "no mission pressure" — see
/// [`TORPEDO_ROUNDS_PER_THREAT_FACT`] — so every existing scenario keeps
/// firing freely.
pub const MISSION_THREAT_REMAINING_COUNTER: &str = "mission_threat_remaining";

/// Host-seeded fact name: every round this ship still HAS —
/// `TorpedoSystem::rounds_aboard`, i.e. the magazine plus the rounds already
/// moved out of it into the tubes.
///
/// Deliberately NOT `torpedoes_remaining`. That counter is debited when a load
/// *starts*, so a hull whose tube doctrine keeps its tubes topped up reads
/// permanently short by its parked volley — three of the destroyer's twelve —
/// and a reserve measured against it would strand exactly those rounds: the
/// counter can reach 0 with a full salvo still sitting in the tubes, and every
/// further launch is refused for the rest of the mission. Conservation is about
/// what the ship can still put in the water, which is this.
pub const TORPEDO_ROUNDS_ABOARD_FACT: &str = "rounds_aboard";
/// Host-seeded fact name: [`MISSION_THREAT_REMAINING_COUNTER`] as this ship's
/// own layer chain reads it, so a ship spawned into a sub-world paces against
/// that layer's mission rather than the base world's.
pub const TORPEDO_MISSION_THREAT_FACT: &str = "mission_threat_remaining";
/// Host-seeded fact name: [`TORPEDO_ROUNDS_ABOARD_FACT`] PER remaining unit of
/// mission threat — the derived ratio a conservation guard compares against an
/// authored reserve, because the predicate grammar compares one atom to one
/// operand and has no arithmetic of its own.
///
/// With no remaining threat published (an unpaced world, or a mission whose
/// threat is spent) the ratio is `f64::INFINITY`: unbounded rounds per remaining
/// threat is the honest answer to "how many can I spend on each of the zero
/// things left", and it makes `>= param(...)` fire, so the unpaced case is the
/// permissive one.
pub const TORPEDO_ROUNDS_PER_THREAT_FACT: &str = "rounds_per_threat";
/// Host-seeded fact name: how many of this ship's own `[behaviour].doctrine`
/// entries are a Destroy directive that NAMES its target
/// (`directive_target`) — the "homing in on one target" reading of the issue's
/// carve-out.
///
/// Counting doctrine entries outright cannot express that carve-out, because a
/// world's `spawn_entity` override APPENDS to the template's doctrine rather
/// than replacing it (`behaviour.doctrine` reconciles by `id`, and an
/// `InstanceOverride` may not tombstone what the template authored). So
/// combat_test's raid cruiser — the shipped case the issue describes — carries
/// its template's untargeted `destroy-hostiles` standing order alongside the
/// `assault-starbase` brief the world gives it, and reads as two objectives
/// however sole its actual brief is. What is singular about it is the NAMED
/// target: a hull ordered to kill one specific thing has one engagement, and
/// the untargeted standing order underneath is what it does with whatever is in
/// front of it, not a second engagement to hoard rounds for.
///
/// A hull with no named target at all reads 0 and is NOT carved out — the
/// player destroyer's brief (`destroy-hostiles` + `hold-station`) is open-ended,
/// which is precisely the ship #943 was filed about.
pub const TORPEDO_TARGETED_OBJECTIVE_COUNT_FACT: &str = "targeted_objective_count";

/// The registered output channels a torpedo magazine policy may drive (#782,
/// widened by #943).
pub const TORPEDO_MAGAZINE_CHANNELS: &[&str] =
    &[TORPEDO_MAGAZINE_CHANNEL, TORPEDO_CONSERVATION_CHANNEL];
/// The registered verbs a torpedo magazine policy may emit (issues #782, #943).
pub const TORPEDO_MAGAZINE_VERBS: &[&str] = &[TORPEDO_MAGAZINE_GRANT_VERB, TORPEDO_RELEASE_VERB];

// ── Shields focus fine-system AI policy channel/verb (issue #783) ─────────────
//
// The Shields fine system focuses ONE of the ship's four arcs at a time. The
// #783 conversion keeps the retained arc-ranking kernel (`tick_shield_focus_ai`:
// damage-concentration primary, health-imbalance fallback) as the 4-way argmax
// and lifts only the AUTHORED windows/thresholds and the gate (whether to act)
// into an inline stateless policy. The `focus_shield_arc` verb is value-less —
// which arc wins is the kernel's call from the host context, never the verb
// (AGENTS.md rule #11: the concentration %, windows, and health ratio are policy
// `param`s, not literals). This is the channel/verb model of #775/#779–#782, not
// the #776 selector: shield arcs are a fixed 4-set of in-ship indices, not UUID
// entities, so there is nothing for a candidate-source selector to union.

/// The `shield_focus` output channel: the Shields fine system's focus-an-arc axis.
pub const SHIELD_FOCUS_CHANNEL: &str = "shield_focus";
/// The `focus_shield_arc` verb. Its presence tells the host to run the retained
/// arc-ranking kernel and emit the same admitted `SetShieldArcFocus` a human
/// Shields operator does; its absence ("hold"/idle) leaves the focus where it is.
pub const SHIELD_FOCUS_VERB: &str = "focus_shield_arc";

/// The registered output channels a Shields focus policy may drive (issue #783).
pub const SHIELD_FOCUS_CHANNELS: &[&str] = &[SHIELD_FOCUS_CHANNEL];
/// The registered verbs a Shields focus policy may emit (issue #783).
pub const SHIELD_FOCUS_VERBS: &[&str] = &[SHIELD_FOCUS_VERB];

/// Authored policy-parameter name: the maximum recent-damage window (seconds)
/// the kernel measures concentration over. Read host-side from the Shields focus
/// policy `param` map (issue #783); the kernel's arg equivalent of the retained
/// typed `ShieldsAiConfigToml::damage_window_secs` knob.
pub const SHIELD_FOCUS_DAMAGE_WINDOW_PARAM: &str = "damage_window_secs";
/// Authored policy-parameter name: the minimum window (seconds) floor.
pub const SHIELD_FOCUS_MIN_DAMAGE_WINDOW_PARAM: &str = "min_damage_window_secs";
/// Authored policy-parameter name: the concentration threshold (0–100).
pub const SHIELD_FOCUS_DAMAGE_PCT_PARAM: &str = "damage_pct_threshold";
/// Authored policy-parameter name: the health-imbalance fallback ratio (0–100).
pub const SHIELD_FOCUS_HEALTH_RATIO_PARAM: &str = "health_ratio_threshold";

// ── Power group allocation fine-system AI policy verb (issue #784) ────────────
//
// The Power reactor fine system allocates the ship's battery budget across the
// ship's AUTHORED power groups. The #784 conversion moves Power onto the same
// inline stateless `FineSystemAiConfigToml` spine as #779–#783, with two
// novelties: (1) the output CHANNELS are the ship's `[power_groups.*]` keys —
// dynamic per-ship data, not a fixed const slice — so the valid-channel set is
// built at load from ship data (AC1 "no fixed catalogue"); (2) the
// `set_power_group_allocation` verb is the FIRST verb to carry a MAGNITUDE — an
// absolute target level — in its payload (every prior verb was value-less or the
// boolean `set_red_alert`). The applier re-clamps to the per-group `[1, max]`
// range and the ship-wide `total <= 8` cap, so an absolute level is safe and
// idempotent; the host skips the emit when `level == current`.

/// The `set_power_group_allocation` verb: set the rule's power group to an
/// absolute target level. Its magnitude is the authored per-rule `level`
/// payload — never an inline Rust number (AGENTS.md rule #11).
pub const POWER_SET_ALLOCATION_VERB: &str = "set_power_group_allocation";

/// Authored policy-parameter name: the battery reserve (0–100) BELOW which the
/// default helm channel gives its elevated point back (AC2). The shed floor: the
/// hold rule reads it, and the channel falls to its baseline underneath it.
pub const POWER_HELM_RESERVE_PARAM: &str = "min_reserve_helm";
/// Authored policy-parameter name: the battery reserve (0–100) the default helm
/// channel must be back OVER before it may elevate again (issue #1003).
///
/// The upper half of the pair, and always above [`POWER_HELM_RESERVE_PARAM`].
/// One threshold would be both the shed floor and the re-elevate floor, and the
/// channel would then flip on every tick the charge rested on it — the lower
/// total recharges past a single threshold inside one tick. See
/// `fragments/ai/fleet_baseline.toml`.
pub const POWER_HELM_RESTORE_PARAM: &str = "min_restore_helm";
/// Authored policy-parameter name: the battery reserve (0–100) BELOW which the
/// default weapons channel gives its elevated point back (AC2).
pub const POWER_WEAPONS_RESERVE_PARAM: &str = "min_reserve_weapons";
/// Authored policy-parameter name: the battery reserve (0–100) the default
/// weapons channel must be back OVER before it may elevate again (issue #1003).
/// Sibling of [`POWER_HELM_RESTORE_PARAM`]; see there for what the gap buys.
pub const POWER_WEAPONS_RESTORE_PARAM: &str = "min_restore_weapons";
/// Host-seeded fact name: current battery charge as a percentage (0–100). The
/// reserve guard `fact(battery_pct) >= param(min_reserve_*)` reads this; it is
/// the stateless brownout-avoidance predicate (AC5).
pub const POWER_BATTERY_PCT_FACT: &str = "battery_pct";
/// Host-seeded fact name: latest forward thrust (0.0–1.0) from `LastHelmInput`.
pub const POWER_THRUST_FACT: &str = "thrust";
/// Host-seeded fact name: red-alert state as `1.0`/`0.0`.
pub const POWER_RED_ALERT_FACT: &str = "red_alert";

// ── Comms dialogue-response fine-system AI policy channel/verb (issue #786) ───
//
// Comms is the FIRST fine system to author BOTH machines at once, because it
// owns two different decisions:
//
//   * WHO to hail — a variable, per-tick candidate set of real contacts keyed by
//     genuine entity UUID. That is #776 selector vocabulary (see
//     [`COMMS_SELECTOR_SOURCES`] / [`default_comms_target_selector_config`]).
//   * HOW to answer an open dialogue — a fixed, small, INDEX-addressed set
//     (`ActiveDialogue.current_node.responses`, addressed by `usize`). That is
//     the #775 channel/verb model, for the same reason Shields (#783) stayed on
//     it: there is no entity set for a candidate-source selector to union.
//
// The `respond_to_message` verb is the SECOND value-carrying verb (after #784's
// `set_power_group_allocation`): only the response INDEX rides the verb — WHICH
// message is being answered comes from the host context, never the policy.

/// The `comms_respond` output channel: the Comms fine system's
/// answer-an-open-dialogue axis, resolved once per message awaiting a response.
pub const COMMS_RESPOND_CHANNEL: &str = "comms_respond";
/// The `respond_to_message` verb: answer the message being resolved with the
/// authored `response_index` payload. Its presence tells the host to emit the
/// same admitted `RespondToMessage` a human Comms officer sends — through the
/// SAME `handle_respond_to_message` router, so trigger actions and follow-ups
/// fire identically for AI and human (AGENTS.md rule #6). Its absence
/// ("hold"/idle) leaves the dialogue open this tick.
pub const COMMS_RESPOND_VERB: &str = "respond_to_message";

/// The registered output channels a Comms response policy may drive (#786).
pub const COMMS_RESPOND_CHANNELS: &[&str] = &[COMMS_RESPOND_CHANNEL];
/// The registered verbs a Comms response policy may emit (issue #786).
pub const COMMS_RESPOND_VERBS: &[&str] = &[COMMS_RESPOND_VERB];

// ── Weapons doctrine: which family the ship turns to present (issue #956) ─────
//
// `tick_weapons_arc_request` emits at most ONE channel-3 `ArcBearingRequest` per
// ship — "turn, so that this family can bear" — so it has to choose a family
// when several are equally unable to shoot. That choice used to be a Rust array,
// `[Phasers, Blasters, Torpedoes]`, documented as "structural, not a gameplay
// value"; which gun a ship manoeuvres to present is a tactical decision, so it
// is authored now, in `[weapons_console.ai]`.
//
// The channel is the RANK and the verb is the FAMILY. Three channels, one per
// place in the order, each a single ordinary channel decision with its own
// guards — so a doctrine can lead with its tubes while the target's striking arc
// is down and with its beams otherwise, which is the shape the issue's worked
// example asks for. The host resolves them in rank order, drops repeats, and
// walks the resulting list until a family actually qualifies; a rank nobody
// authors simply shortens the order.

/// The `arc_bearing_first` channel: the family this ship presents by preference.
pub const ARC_BEARING_FIRST_CHANNEL: &str = "arc_bearing_first";
/// The `arc_bearing_second` channel: the family it turns for when the first
/// cannot be the reason (no emitters, all offline, already bearing, out of
/// range).
pub const ARC_BEARING_SECOND_CHANNEL: &str = "arc_bearing_second";
/// The `arc_bearing_third` channel: the last family in the order.
pub const ARC_BEARING_THIRD_CHANNEL: &str = "arc_bearing_third";

/// The rank ladder in resolution order. The host reads exactly this slice, so
/// adding a rank is a one-line content-schema change rather than a host one.
pub const ARC_BEARING_CHANNELS: &[&str] = &[
    ARC_BEARING_FIRST_CHANNEL,
    ARC_BEARING_SECOND_CHANNEL,
    ARC_BEARING_THIRD_CHANNEL,
];

/// The `bring_phasers_to_bear` verb: name the phaser banks for this rank.
pub const BRING_PHASERS_TO_BEAR_VERB: &str = "bring_phasers_to_bear";
/// The `bring_blasters_to_bear` verb: name the blaster banks for this rank.
pub const BRING_BLASTERS_TO_BEAR_VERB: &str = "bring_blasters_to_bear";
/// The `bring_torpedoes_to_bear` verb: name the torpedo tubes for this rank.
pub const BRING_TORPEDOES_TO_BEAR_VERB: &str = "bring_torpedoes_to_bear";

/// The registered verbs a weapons-doctrine policy may emit (issue #956).
pub const WEAPONS_DOCTRINE_VERBS: &[&str] = &[
    BRING_PHASERS_TO_BEAR_VERB,
    BRING_BLASTERS_TO_BEAR_VERB,
    BRING_TORPEDOES_TO_BEAR_VERB,
];

/// Host-seeded fact name: HP of the target's shield arc a round from this ship
/// would strike, resolved through the target's own arc router. `<= 0` means the
/// arc is not blocking (down, offline, or absent entirely — an asteroid).
///
/// The reading the fleet's torpedo doctrine is authored against, seeded on the
/// weapons-doctrine snapshot (issue #956) as well as on the tube launch snapshot
/// (`seed_torpedo_tube_launch_facts`), so "lead with the tubes when the screen
/// is down" and "launch when the screen is down" ask the same question of the
/// same number.
pub const TARGET_FACING_SHIELDS_FACT: &str = "target_facing_shields";

// ── Per-system target selector sources (issue #776) ───────────────────────────

/// Candidate source: the ship's frozen combat lock (Tactical's designated
/// firing target), surfaced to the Sensors selector as the highest-priority
/// tier so Sensors mirrors what Tactical is engaging.
pub const SELECTOR_SOURCE_COMBAT_LOCK: &str = "combat-lock";
/// Candidate source: named `Destroy` objective targets resolved from the
/// scored objective pool.
pub const SELECTOR_SOURCE_OBJECTIVE_DESTROY: &str = "objective-destroy";
/// Candidate source: faction-hostile radar contacts inside the ship's horizon.
pub const SELECTOR_SOURCE_RADAR_CONTACTS: &str = "radar-contacts";

/// The registered candidate sources the Sensors target selector may union.
pub const SENSORS_SELECTOR_SOURCES: &[&str] = &[
    SELECTOR_SOURCE_COMBAT_LOCK,
    SELECTOR_SOURCE_OBJECTIVE_DESTROY,
    SELECTOR_SOURCE_RADAR_CONTACTS,
];

/// Candidate source: the ship's advisory **Science Target** — the Sensors
/// radar's selected target, surfaced from the frozen viewscreen blackboard
/// (issue #777). Tactical may strongly favour this pick through an authored
/// score bonus, but independently revalidates it before copying (AC2/AC3). It
/// is deliberately NOT the same as `combat-lock`: that is Tactical's OWN output
/// and is excluded to avoid circularity.
pub const SELECTOR_SOURCE_SENSORS_DESIGNATION: &str = "sensors-designation";
/// Candidate source: whoever last attacked this ship (`LastShipAttacker`).
pub const SELECTOR_SOURCE_LAST_ATTACKER: &str = "last-attacker";
/// Candidate source: the target named by an active per-verb operate directive
/// (`Tow`/`Stabilise`/`Escort`/`FieldRepair`), resolved from the scored
/// objective pool (issue #1162). Injected by `ai_target_selection` and tagged
/// `source_operate`; it is the DIRECTIVE-GATED way the AI reaches a non-hostile
/// lock. Inert when no operate directive is active — no candidate is added — so
/// combat auto-lock is unchanged and radar/scan auto-lock stays hostile-gated. A
/// hull opts into it by adding `... or candidate_fact(source_operate) > 0` to
/// its `[weapons_console.selector]` eligibility (fleet_baseline authors it).
pub const SELECTOR_SOURCE_OBJECTIVE_OPERATE: &str = "objective-operate";

/// The registered candidate sources the Tactical target selector may union
/// (issue #777). `combat-lock` is intentionally absent: it is Tactical's own
/// authoritative output, so unioning it would be circular. The ship's current
/// lock is instead surfaced by the host as an internal `source_retained`
/// retention candidate (not a cross-system source), so it too is absent here.
pub const TACTICAL_SELECTOR_SOURCES: &[&str] = &[
    SELECTOR_SOURCE_SENSORS_DESIGNATION,
    SELECTOR_SOURCE_OBJECTIVE_DESTROY,
    SELECTOR_SOURCE_LAST_ATTACKER,
    SELECTOR_SOURCE_RADAR_CONTACTS,
    SELECTOR_SOURCE_OBJECTIVE_OPERATE,
];

/// Candidate source: positive, Navigation-relevant (Helm-affinity) objective
/// destinations (issue #778). The Navigation host ranks the scored objective
/// pool, resolves the winner's directive to a destination — a fixed world
/// anchor (Reach / Retreat / Patrol) or a live entity anchor (Destroy) — and
/// surfaces it as the sole `reachable` candidate of this source.
pub const SELECTOR_SOURCE_NAV_OBJECTIVE: &str = "navigation-objectives";
/// Candidate source: live entities the Navigation chart shows, surfaced as
/// authorable entity-anchored destinations (issue #778). They do NOT carry the
/// `reachable` marker under the canonical policy, so by default they enrich a
/// coincident objective destination rather than independently steering the
/// ship; an author may re-tune the selector's eligibility to admit them.
pub const SELECTOR_SOURCE_CHART_CONTACTS: &str = "chart-contacts";

/// The registered candidate sources the Navigation target selector may union
/// (issue #778).
pub const NAVIGATION_SELECTOR_SOURCES: &[&str] = &[
    SELECTOR_SOURCE_NAV_OBJECTIVE,
    SELECTOR_SOURCE_CHART_CONTACTS,
];

/// Candidate source: stations the ship's coordination-delivered
/// `RepairRequestQueue` reports as damaged (issue #785). This is the AC1
/// "authoritative observable damage" surface: the Repair AI ranks only stations
/// a `RepairRequest` actually delivered — issue #830 deliberately removed the
/// raw hull poll, so a station nobody reported is not a candidate.
pub const SELECTOR_SOURCE_DAMAGED_STATIONS: &str = "damaged-stations";
/// Candidate source: the ownerless ship-wide `core` repair bucket (issue #785),
/// the second [`crate::core::messages::RepairTarget`] variant. Surfaced as a candidate
/// so an author can weight core repairs into the ranking; under the canonical
/// policy it only becomes eligible once a `RepairRequest` names it, mirroring
/// how `chart-contacts` enriches rather than independently steers Navigation.
pub const SELECTOR_SOURCE_CORE_BUCKET: &str = "core-bucket";

/// The registered candidate sources the Repair target selector may union
/// (issue #785).
pub const REPAIR_SELECTOR_SOURCES: &[&str] = &[
    SELECTOR_SOURCE_DAMAGED_STATIONS,
    SELECTOR_SOURCE_CORE_BUCKET,
];

/// Candidate source: positive, Comms-relevant `Hail` directives resolved from
/// the scored objective pool (issue #786). This is the AC1 surface: the Comms
/// AI ranks the hail orders it has actually been given, resolving each
/// directive's authored entity NAME to a real contact UUID before it can become
/// a candidate.
pub const SELECTOR_SOURCE_HAIL_OBJECTIVES: &str = "hail-objectives";
/// Candidate source: the authoritative comms contact list (issue #786) —
/// `CommsRuntime.contacts`, the same hailable roster a human Comms officer sees.
/// Under the canonical policy a contact is NOT independently eligible (the
/// default eligibility keys on `source_hail_objective`): it ENRICHES a
/// coincident hail directive with its live readings, exactly as
/// `chart-contacts` enriches a Navigation destination (#778). An author may
/// widen the eligibility to let the Comms AI hail on its own initiative.
pub const SELECTOR_SOURCE_COMMS_CONTACTS: &str = "comms-contacts";

/// The registered candidate sources the Comms hail selector may union
/// (issue #786).
pub const COMMS_SELECTOR_SOURCES: &[&str] = &[
    SELECTOR_SOURCE_HAIL_OBJECTIVES,
    SELECTOR_SOURCE_COMMS_CONTACTS,
];

/// One authored additive utility term (`[[sensors_console.selector.score]]`,
/// issue #776): a guard expression plus the weight it contributes to a
/// candidate's score when it fires.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScoreTermToml {
    /// Guard expression over self/candidate/target fact contexts; the term
    /// contributes `weight` when it evaluates `true`.
    pub when: String,
    /// Weight added to the candidate's score when `when` fires. An authored
    /// field (AGENTS.md rule #11 permits authored gameplay values in TOML).
    pub weight: f32,
}

/// Inline per-system target selector for an AI-capable fine system that owns a
/// target (`[sensors_console.selector]`, issue #776).
///
/// Sibling to [`FineSystemAiConfigToml`]: where the #775 policy resolves a
/// verb per output channel, the selector answers "which entity is my target?".
/// It unions authored candidate `sources`, filters inside `horizon`, keeps
/// candidates whose `eligibility` guard fires, sums the additive `score`,
/// retains the current target within `switch_margin`, and returns the winning
/// contact — all as a pure function of the immutable per-tick snapshot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FineSystemAiSelectorToml {
    /// Named numeric parameters referenced by the eligibility/score guards.
    #[serde(default)]
    pub param: std::collections::HashMap<String, f32>,
    /// Registered candidate-source ids this selector unions.
    #[serde(default)]
    pub sources: Vec<String>,
    /// Effective horizon (planar distance) beyond which candidates are dropped.
    pub horizon: f32,
    /// Hysteresis margin: the current target is retained while its score is
    /// within this margin of the best candidate's score.
    pub switch_margin: f32,
    /// Candidate eligibility guard over self/candidate/target fact contexts.
    pub eligibility: String,
    /// Additive utility terms summed per eligible candidate.
    #[serde(default)]
    pub score: Vec<ScoreTermToml>,
}

impl FineSystemAiSelectorToml {
    /// Resolve this authored block into the pure typed
    /// [`crate::ai::selector::TargetSelector`] the runtime consumes.
    ///
    /// Returns a diagnostic `Err` on an unparseable eligibility/score guard;
    /// call [`validate_fine_system_ai_selector`] first at content-load time so
    /// this never fails after world activation.
    pub fn to_selector(&self) -> Result<crate::ai::selector::TargetSelector, String> {
        let mut params = crate::world::flags::AiParams::new();
        for (k, v) in &self.param {
            params.set(k, *v as f64);
        }
        let eligibility = crate::world::flags::parse_predicate(&self.eligibility)?;
        let mut score = Vec::with_capacity(self.score.len());
        for term in &self.score {
            let when = crate::world::flags::parse_predicate(&term.when)?;
            score.push(crate::ai::selector::ScoreTerm {
                when,
                weight: term.weight as f64,
            });
        }
        Ok(crate::ai::selector::TargetSelector {
            params,
            sources: self.sources.clone(),
            horizon: self.horizon,
            switch_margin: self.switch_margin,
            eligibility,
            score,
        })
    }
}

/// Validate an inline per-system target selector before world activation
/// (issue #776), mirroring [`validate_fine_system_ai_policy`].
///
/// Rejects:
///   - an unknown candidate source id,
///   - an unparseable `eligibility` or score `when` expression,
///   - a `param(...)` reference to a parameter the author never declared.
///
/// HOST-AGNOSTIC, for the same reason [`validate_fine_system_ai_policy`] is:
/// the `flag(...)`/`counter(...)` check needs the host, so it lives in
/// [`validate_fine_system_ai_selector_for`].
pub fn validate_fine_system_ai_selector(
    cfg: &FineSystemAiSelectorToml,
    valid_sources: &[&str],
) -> Result<(), String> {
    validate_selector_inner(cfg, valid_sources, None)
}

/// [`validate_fine_system_ai_selector`] for a NAMED host, additionally
/// rejecting a `flag(...)`/`counter(...)` reference in the `eligibility`
/// expression or any score term's `when` that the host could never evaluate
/// (issue #891 stage 1). Four of the five selector hosts pass `&[]`, so this is
/// the same trap the policy hosts carry, on a second surface.
pub fn validate_fine_system_ai_selector_for(
    host: &crate::entities::ai_flag_hosts::AiHost,
    cfg: &FineSystemAiSelectorToml,
    valid_sources: &[&str],
) -> Result<(), String> {
    validate_selector_inner(cfg, valid_sources, Some(host))
}

fn validate_selector_inner(
    cfg: &FineSystemAiSelectorToml,
    valid_sources: &[&str],
    host: Option<&crate::entities::ai_flag_hosts::AiHost>,
) -> Result<(), String> {
    for src in &cfg.sources {
        if !valid_sources.contains(&src.as_str()) {
            return Err(format!(
                "target selector references unknown source '{src}' (valid: {valid_sources:?})"
            ));
        }
    }
    let check_params = |pred: &crate::world::flags::Predicate, what: &str| -> Result<(), String> {
        if let Some(host) = host {
            host.check_guard(&format!("target selector {what}"), pred)?;
            host.check_facts(&format!("target selector {what}"), pred)?;
        }
        // Unlike the flag chain, this one needs no host to answer (issue #890):
        // a selector evaluates through `Predicate::evaluate_selector`, which
        // hands in a DEFAULT private bag — there is no per-fine-system history
        // for a per-candidate scoring pass to fold into, on any host. So the
        // rejection fires on the host-less path too, and a `history(...)` in an
        // eligibility or score term can never become a permanently-false term.
        if let Some(atom) = pred.history_atom() {
            return Err(format!(
                "target selector {what} reads {}, but a target selector is evaluated \
                 per candidate against a snapshot with no history bag: no window is \
                 folded for it, so the comparison would read false for ever. A \
                 windowed question belongs in the owning system's policy, which has \
                 one",
                atom.render()
            ));
        }
        let mut refs = Vec::new();
        pred.referenced_params(&mut refs);
        for name in refs {
            if !cfg.param.contains_key(&name) {
                return Err(format!(
                    "target selector {what} references undeclared parameter '{name}'"
                ));
            }
        }
        Ok(())
    };
    let eligibility = crate::world::flags::parse_predicate(&cfg.eligibility)
        .map_err(|e| format!("target selector has invalid `eligibility` expression: {e}"))?;
    check_params(&eligibility, "eligibility")?;
    for (idx, term) in cfg.score.iter().enumerate() {
        let when = crate::world::flags::parse_predicate(&term.when)
            .map_err(|e| format!("target selector score term {idx} has invalid `when`: {e}"))?;
        check_params(&when, &format!("score term {idx}"))?;
    }
    Ok(())
}

/// One authored inline policy rule (`[[captain_console.ai.rule]]`, issue #775).
///
/// A rule binds a `priority` and an output `channel` to a `when` predicate
/// (the shared `world::flags` grammar, extended with typed `fact(...)` atoms
/// and `param(...)` references) and a typed `verb`. `value` is the boolean the
/// verb applies for boolean-channel verbs such as `set_red_alert`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FineSystemAiRuleToml {
    /// Higher wins within a channel; ties resolve to the earliest-authored rule.
    pub priority: i32,
    /// Output channel this rule contributes to (e.g. `"red_alert"`).
    pub channel: String,
    /// Guard expression; the rule fires when it evaluates `true`.
    pub when: String,
    /// Typed verb applied when this rule wins its channel (e.g. `"set_red_alert"`).
    pub verb: String,
    /// Boolean output value for boolean-channel verbs. Defaults to `false`.
    #[serde(default)]
    pub value: bool,
    /// Magnitude payload for value-carrying verbs (issue #784). Currently the
    /// absolute target level for `set_power_group_allocation`; ignored by
    /// value-less and boolean verbs. Defaults to `0`.
    #[serde(default)]
    pub level: u8,
    /// Index payload for the `respond_to_message` verb (issue #786): WHICH of
    /// the open dialogue node's responses this rule answers with. Ignored by
    /// every other verb. Deliberately a separate field from `level` — the two
    /// address different things (a power magnitude vs. a position in a fixed
    /// response list), and fusing them would make an authored rule's meaning
    /// depend on its verb. Defaults to `0` (the first response), reproducing the
    /// retired channel-2 auto-response stub. A rule that should NOT answer this
    /// tick simply does not fire; there is no "don't respond" index.
    #[serde(default)]
    pub response_index: u8,
}

/// One authored state of an inline STATEFUL policy
/// (`[[<system>.ai.state]]`, issue #882).
///
/// A state carries its own continuous `rule` list — the very same
/// [`FineSystemAiRuleToml`] the stateless path uses, so a rule's meaning does
/// not change with where it is authored — and its own outgoing `transition`
/// list. Note [`FineSystemAiRuleToml`] deliberately gained NO `state` field:
/// it is `deny_unknown_fields`, and nesting rules under the state that owns
/// them keeps a rule's owning state unambiguous.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FineSystemAiStateToml {
    /// Unique state id within this policy; referenced by `initial_state` and
    /// by every transition's `to`.
    pub id: String,
    /// Continuous per-channel rules that apply while this state is current.
    #[serde(default)]
    pub rule: Vec<FineSystemAiRuleToml>,
    /// Outgoing transitions, at most one of which fires per eligible tick.
    #[serde(default)]
    pub transition: Vec<FineSystemAiTransitionToml>,
    /// Whether this leg yields its solved facing to a channel-3
    /// `ArcBearingRequest` (issue #918).
    ///
    /// Defaults to `true`, which is exactly what every leg did before #918: a
    /// weapon family that cannot bear takes the facing and the ship turns to
    /// make it bear (#673-#684). A leg authors `false` when the heading it flies
    /// IS the manoeuvre — a broadside ring's tangent, a fly-through escape's
    /// frozen heading — and a request that arrives while it is flown is
    /// declined instead of overwriting it.
    ///
    /// Only a system with a `yaw` channel can consume this;
    /// [`validate_fine_system_ai_policy`] rejects a `false` authored on any
    /// other system, so a declaration that could never be read is a load error
    /// rather than a silent no-op.
    #[serde(default = "default_yields_to_arc_requests")]
    pub yields_to_arc_requests: bool,
}

/// The parse default for [`FineSystemAiStateToml::yields_to_arc_requests`]:
/// a leg that says nothing yields, as every leg did before issue #918.
pub(crate) fn default_yields_to_arc_requests() -> bool {
    true
}

impl Default for FineSystemAiStateToml {
    /// Hand-written rather than derived for the reason
    /// [`FineSystemAiConfigToml::default`] is: a derived `bool` default is
    /// `false`, and `yields_to_arc_requests: false` is not "unauthored", it is
    /// a leg that declines channel-3 requests. `..Default::default()` has to
    /// mean the same thing as an omitted field in TOML.
    fn default() -> Self {
        Self {
            id: String::new(),
            rule: Vec::new(),
            transition: Vec::new(),
            yields_to_arc_requests: default_yields_to_arc_requests(),
        }
    }
}

/// One authored transition out of the enclosing state
/// (`[[<system>.ai.state.transition]]`, issue #882).
///
/// There is no `from`: the source is the state this table is nested in.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct FineSystemAiTransitionToml {
    /// Higher wins; ties resolve to the earliest-authored transition.
    pub priority: i32,
    /// The state id entered when this transition fires.
    pub to: String,
    /// Guard expression; the transition becomes eligible when it evaluates
    /// `true`. May read `memory(...)` and `state_time` as well as the usual
    /// facts/flags/params.
    pub when: String,
}

/// Inline AI policy for an AI-capable fine system
/// (`[captain_console.ai]`, issues #775, #882).
///
/// A system declares EITHER a policy (`param` + `rule`, and optionally the
/// #882 state machine) OR an explicit `idle = true`. An empty declaration
/// (`ai = {}`) is neither and is rejected by
/// [`validate_fine_system_ai_policy`] — silence is not a valid declaration.
///
/// ## Back-compat guarantee (issue #882)
///
/// Every field added by the stateful path is `#[serde(default)]`, so all
/// twelve shipped stateless blocks parse byte-identically and decode to a
/// policy whose `machine` is `None`. A block that authors no `state` never
/// enters the transition code path at all.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FineSystemAiConfigToml {
    /// How many shared AI base ticks (`[global] ai_tick_hz`) pass between two
    /// evaluations of this policy (PRD #774 §9, issue #889).
    ///
    /// `1` — the parse default — means "every base tick", which is what every
    /// shipped policy authors today. A larger integer lets a host such as
    /// Sensors, Power, Repair or Comms decide less often as **authored data**,
    /// instead of the second hardcoded Rust `Timer` that #889 retired. The
    /// field is typed `u32`, so a non-integer multiple of the base cadence is a
    /// TOML type error at load; `0` is rejected by
    /// [`validate_fine_system_ai_policy`] (a policy that never evaluates is an
    /// `idle = true` declaration, not a cadence).
    #[serde(default = "default_evaluate_every_ticks")]
    pub evaluate_every_ticks: u32,
    /// Explicit idle marker. Mutually exclusive with `rule` and with `state`.
    #[serde(default)]
    pub idle: bool,
    /// Named numeric parameters referenced by rule guards.
    #[serde(default)]
    pub param: std::collections::HashMap<String, f32>,
    /// Prioritised per-channel reactive rules (the stateless path).
    #[serde(default)]
    pub rule: Vec<FineSystemAiRuleToml>,
    /// The state entered on reset. Required when — and rejected unless —
    /// `state` is non-empty (issue #882).
    #[serde(default)]
    pub initial_state: Option<String>,
    /// The declared states of the OPTIONAL state machine (issue #882).
    /// Absent/empty ⇒ this is a stateless policy.
    #[serde(default)]
    pub state: Vec<FineSystemAiStateToml>,
    /// Typed private memory declarations: name → initial value (issue #882).
    /// Readable through the `memory(name)` atom by THIS fine system only.
    #[serde(default)]
    pub memory: std::collections::HashMap<String, f32>,
}

/// The parse default for [`FineSystemAiConfigToml::evaluate_every_ticks`]:
/// evaluate on every shared AI base tick.
pub(crate) fn default_evaluate_every_ticks() -> u32 {
    1
}

impl Default for FineSystemAiConfigToml {
    /// Hand-written rather than derived so that `..Default::default()` yields
    /// the same `evaluate_every_ticks` the TOML parse default supplies. A
    /// derived `0` would be a policy that never evaluates.
    fn default() -> Self {
        Self {
            evaluate_every_ticks: default_evaluate_every_ticks(),
            idle: false,
            param: std::collections::HashMap::new(),
            rule: Vec::new(),
            initial_state: None,
            state: Vec::new(),
            memory: std::collections::HashMap::new(),
        }
    }
}

impl FineSystemAiConfigToml {
    /// Resolve this authored block into the pure typed [`crate::ai::policy::AiPolicy`]
    /// the runtime evaluator consumes.
    ///
    /// Returns a diagnostic `Err` on an unparseable guard or unknown verb; call
    /// [`validate_fine_system_ai_policy`] first at content-load time so this
    /// never fails after world activation.
    pub fn to_policy(&self) -> Result<crate::ai::policy::AiPolicy, String> {
        let mut params = crate::world::flags::AiParams::new();
        for (k, v) in &self.param {
            params.set(k, *v as f64);
        }
        let rules = decode_rules(&self.rule)?;
        // The OPTIONAL #882 state machine. No authored `state` tables ⇒ `None`,
        // which is what every shipped stateless block decodes to.
        let machine = if self.state.is_empty() {
            None
        } else {
            let mut states = Vec::with_capacity(self.state.len());
            for s in &self.state {
                let mut transitions = Vec::with_capacity(s.transition.len());
                for t in &s.transition {
                    transitions.push(crate::ai::policy::AiPolicyTransition {
                        priority: t.priority,
                        to: t.to.clone(),
                        when: crate::world::flags::parse_predicate(&t.when)?,
                    });
                }
                states.push(crate::ai::policy::AiPolicyState {
                    id: s.id.clone(),
                    rules: decode_rules(&s.rule)?,
                    transitions,
                    yields_to_arc_requests: s.yields_to_arc_requests,
                });
            }
            Some(crate::ai::policy::AiPolicyMachine {
                initial: self.initial_state.clone().ok_or_else(|| {
                    "ai policy declares states but no `initial_state`".to_string()
                })?,
                initial_memory: self.initial_memory(),
                states,
            })
        };
        Ok(crate::ai::policy::AiPolicy {
            params,
            rules,
            idle: self.idle,
            machine,
        })
    }

    /// The authored initial values of this policy's typed private memory
    /// (issue #882), as the runtime bag a fresh state component starts from.
    pub fn initial_memory(&self) -> crate::world::flags::AiPolicyMemory {
        let mut m = crate::world::flags::AiPolicyMemory::new();
        for (k, v) in &self.memory {
            m.set(k, *v as f64);
        }
        m
    }
}

/// Decode one authored rule list into typed policy rules (issue #882).
///
/// Shared by the top-level stateless `rule` list and by each state's own
/// `rule` list, so a rule decodes identically wherever it is authored.
fn decode_rules(
    src: &[FineSystemAiRuleToml],
) -> Result<Vec<crate::ai::policy::AiPolicyRule>, String> {
    let mut rules = Vec::with_capacity(src.len());
    for r in src {
        let when = crate::world::flags::parse_predicate(&r.when)?;
        rules.push(crate::ai::policy::AiPolicyRule {
            priority: r.priority,
            channel: r.channel.clone(),
            when,
            verb: decode_verb(r)?,
        });
    }
    Ok(rules)
}

/// Decode one authored rule's `verb` (plus its payload fields) into the typed
/// [`crate::ai::policy::AiPolicyVerb`] (issue #882 extraction; the match body
/// is unchanged from #775–#786).
fn decode_verb(r: &FineSystemAiRuleToml) -> Result<crate::ai::policy::AiPolicyVerb, String> {
    Ok(match r.verb.as_str() {
        CAPTAIN_SET_RED_ALERT_VERB => crate::ai::policy::AiPolicyVerb::SetRedAlert(r.value),
        // Helm continuous-actuator mode verbs (issue #779): value-less;
        // the `value` field is ignored — the magnitude lives in the
        // planner fact, not the policy.
        HELM_ACTUATE_DESIRED_TRAVEL_VERB => crate::ai::policy::AiPolicyVerb::ActuateDesiredTravel,
        HELM_ACTUATE_DESIRED_FACING_VERB => crate::ai::policy::AiPolicyVerb::ActuateDesiredFacing,
        // The frozen-heading Steering mode verb (issue #883): also value-less —
        // the heading is host-written private memory, not an authored constant.
        HELM_HOLD_COMMITTED_HEADING_VERB => crate::ai::policy::AiPolicyVerb::HoldCommittedHeading,
        // The recovery-orbit and re-engage Steering mode verbs (issue #788):
        // value-less too — the ring's radius is derived from the TARGET's
        // reach and the circulation direction is host-written private memory,
        // neither of which an authored constant could express.
        HELM_HOLD_RECOVERY_ORBIT_VERB => crate::ai::policy::AiPolicyVerb::HoldRecoveryOrbit,
        HELM_PIVOT_TO_REENGAGE_VERB => crate::ai::policy::AiPolicyVerb::PivotToReengage,
        // The combat broadside orbit (issue #790): value-less too — the ring's
        // radius, throttle and spiral gain are authored Steering `param`s and
        // the circulation direction is host-written private memory.
        HELM_HOLD_COMBAT_ORBIT_VERB => crate::ai::policy::AiPolicyVerb::HoldCombatOrbit,
        // The torpedo-opportunity bow hold (issue #791): value-less too — the
        // throttle is an authored Steering `param`, and which shield is down,
        // which arc the tubes cover and whether a salvo is still in flight are
        // all host readings.
        HELM_HOLD_TORPEDO_BEARING_VERB => crate::ai::policy::AiPolicyVerb::HoldTorpedoBearing,
        // The artillery firing position (issue #792): value-less too — the hold
        // throttle and the range band are authored Steering `param`s, and the
        // lead speed is a host reading of the hull's own artillery bolt.
        HELM_HOLD_ARTILLERY_POSITION_VERB => crate::ai::policy::AiPolicyVerb::HoldArtilleryPosition,
        // Helm secondary-actuator mode verbs (issue #780): value-less,
        // like the travel-axis verbs above.
        HELM_ACTUATE_LATERAL_THRUST_VERB => crate::ai::policy::AiPolicyVerb::ActuateLateralThrust,
        HELM_ACTUATE_VERTICAL_THRUST_VERB => crate::ai::policy::AiPolicyVerb::ActuateVerticalThrust,
        HELM_ENGAGE_IMPULSE_VERB => crate::ai::policy::AiPolicyVerb::EngageImpulse,
        HELM_ENGAGE_BOOST_VERB => crate::ai::policy::AiPolicyVerb::EngageBoost,
        // Weapon-bank action verbs (issue #781): value-less, like the
        // helm mode verbs. The `value` field is ignored — the target and
        // firing bank come from the host context, not the policy.
        PHASER_FIRE_VERB => crate::ai::policy::AiPolicyVerb::FirePhaser,
        BLASTER_FIRE_VERB => crate::ai::policy::AiPolicyVerb::FireBlaster,
        // Torpedo tube + magazine action verbs (issue #782): value-less,
        // like the weapon-bank verbs. The `value` field is ignored — the
        // tube, volley target, combat lock, and magazine come from the
        // host context, not the policy.
        TORPEDO_LOAD_VERB => crate::ai::policy::AiPolicyVerb::LoadTorpedo,
        TORPEDO_LAUNCH_VERB => crate::ai::policy::AiPolicyVerb::LaunchTorpedo,
        TORPEDO_MAGAZINE_GRANT_VERB => crate::ai::policy::AiPolicyVerb::GrantTorpedoRound,
        // Torpedo conservation verb (issue #943): value-less too — the
        // magazine level, the remaining mission threat and this ship's own
        // objective count are all host readings, and the reserve they are
        // compared against is an authored `param`.
        TORPEDO_RELEASE_VERB => crate::ai::policy::AiPolicyVerb::ReleaseTorpedo,
        // Shields focus action verb (issue #783): value-less, like the
        // weapon-bank verbs. The `value` field is ignored — which of the
        // four arcs is focused comes from the retained ranking kernel in
        // the host context, not the policy.
        SHIELD_FOCUS_VERB => crate::ai::policy::AiPolicyVerb::FocusShieldArc,
        // Power group allocation verb (issue #784): the FIRST verb to
        // carry a magnitude. The absolute target level is the authored
        // per-rule `level` payload, never an inline Rust number.
        POWER_SET_ALLOCATION_VERB => {
            crate::ai::policy::AiPolicyVerb::SetPowerGroupAllocation(r.level)
        }
        // Comms dialogue-response verb (issue #786): the SECOND
        // value-carrying verb. Only the response INDEX rides the verb —
        // WHICH message is being answered comes from the host context.
        COMMS_RESPOND_VERB => crate::ai::policy::AiPolicyVerb::RespondToMessage(r.response_index),
        // Weapons doctrine family verbs (issue #956): value-less, like the
        // fire verbs. The RANK is the channel and the FAMILY is the verb; the
        // arcs, ranges and geometry the resulting `ArcBearingRequest` carries
        // are host readings of this ship's own emitters, never authored here.
        BRING_PHASERS_TO_BEAR_VERB => crate::ai::policy::AiPolicyVerb::BringPhasersToBear,
        BRING_BLASTERS_TO_BEAR_VERB => crate::ai::policy::AiPolicyVerb::BringBlastersToBear,
        BRING_TORPEDOES_TO_BEAR_VERB => crate::ai::policy::AiPolicyVerb::BringTorpedoesToBear,
        other => return Err(format!("unknown ai policy verb '{other}'")),
    })
}

/// Validate an inline fine-system AI policy before world activation
/// (issues #775, #882), mirroring [`validate_phaser_banks`] et al.
///
/// Rejects (stateless path, issue #775):
///   - a silent declaration (neither `idle` nor any `rule` nor any `state`),
///   - a contradictory declaration (`idle = true` alongside rules/states),
///   - a MIXED declaration: top-level `rule`s alongside `state`s (issue #883) —
///     a machine resolves only in-state, so those rules are silently dead,
///   - an unparseable `when` guard (reusing `parse_predicate`'s diagnostic),
///   - an unknown output `channel` or `verb`,
///   - a `param(...)` reference to a parameter the author never declared.
///
/// Additionally rejects (stateful path, issue #882 AC6):
///   - an `initial_state` naming a state that was never declared (and a
///     `state` list with no `initial_state` at all),
///   - a transition whose `to` names a state that was never declared,
///   - duplicate state ids,
///   - an unreachable state (no inbound transition and not the initial state),
///   - a `memory(...)` or `state_time` reference in a STATELESS policy, and a
///     `memory(...)` reference to a slot the author never declared.
///
/// Additionally rejects (issue #794, PRD #774's deterministic-policy
/// categories):
///   - two or more transitions out of ONE state authored at the same
///     `priority`,
///   - two or more rules on ONE output channel authored at the same `priority`
///     — within a state for a machine, within the top-level list for a
///     stateless policy.
///
/// Both are the same defect wearing two hats. Resolution is "highest priority
/// wins, ties to the earliest-authored", so a tie IS resolved — silently, by
/// where the author happened to put the table. The file then reads as though
/// the two entries were interchangeable when the runtime has already picked
/// one, and re-ordering the file changes behaviour without changing a value.
/// Distinct priorities cost the author one character and make the decision
/// legible. Note the scope on both: repeated priorities across DIFFERENT
/// channels (a load rule and a launch rule both at 0) never compete, and are
/// left alone.
///
/// HOST-AGNOSTIC. `flag(...)`/`counter(...)` guards only read true where the
/// host passes a populated flag-store chain, and most hosts pass `&[]`
/// (issue #891); that check needs to know WHICH host is being validated, so it
/// lives in [`validate_fine_system_ai_policy_for`]. Every production call site
/// in [`EntityConfig::from_toml`] uses that one — a test asserts it — and this
/// entry point is for ad-hoc and unit-test validation of a policy with no host.
pub fn validate_fine_system_ai_policy(
    cfg: &FineSystemAiConfigToml,
    valid_channels: &[&str],
    valid_verbs: &[&str],
) -> Result<(), String> {
    validate_policy_inner(cfg, valid_channels, valid_verbs, None)
}

/// [`validate_fine_system_ai_policy`] for a NAMED host, additionally rejecting a
/// `flag(...)`/`counter(...)` guard the host could never evaluate (issue #891
/// stage 1 — see [`crate::entities::ai_flag_hosts`]).
pub fn validate_fine_system_ai_policy_for(
    host: &crate::entities::ai_flag_hosts::AiHost,
    cfg: &FineSystemAiConfigToml,
    valid_channels: &[&str],
    valid_verbs: &[&str],
) -> Result<(), String> {
    validate_policy_inner(cfg, valid_channels, valid_verbs, Some(host))
}

fn validate_policy_inner(
    cfg: &FineSystemAiConfigToml,
    valid_channels: &[&str],
    valid_verbs: &[&str],
    host: Option<&crate::entities::ai_flag_hosts::AiHost>,
) -> Result<(), String> {
    // Cadence first (issue #889): `evaluate_every_ticks` counts shared AI base
    // ticks, so it has to be a POSITIVE integer. `u32` already makes a
    // non-integer multiple of the base a TOML type error; zero would be a
    // policy that never evaluates, which is `idle = true`, not a cadence.
    if cfg.evaluate_every_ticks == 0 {
        return Err(
            "ai policy declares evaluate_every_ticks = 0: the value counts shared AI base \
             ticks between evaluations and must be a positive integer. A policy that should \
             never evaluate declares idle = true"
                .into(),
        );
    }
    if cfg.idle {
        if !cfg.rule.is_empty() {
            return Err("ai policy declares idle = true but also carries rules".into());
        }
        if !cfg.state.is_empty() {
            return Err("ai policy declares idle = true but also carries states".into());
        }
        return Ok(());
    }
    if cfg.rule.is_empty() && cfg.state.is_empty() {
        return Err(
            "ai policy is empty: declare at least one rule or state, or set idle = true".into(),
        );
    }
    // A policy is EITHER stateless (top-level `rule`) or a machine (`state`),
    // never both (issue #883, carried forward from the #882 review). A machine
    // resolves EXCLUSIVELY through `resolve_channel_in_state`, so top-level
    // rules on a stateful policy are silently dead code — and worse, the
    // `stateful` flag below makes a `memory(...)` reference inside one VALIDATE
    // while always evaluating false at runtime, because the stateless scan hands
    // `best_in` an empty memory bag. Both failures are silent, which is exactly
    // the class of defect #882's blocking bug belonged to, so the shape is
    // rejected at load rather than merely discouraged.
    if !cfg.rule.is_empty() && !cfg.state.is_empty() {
        return Err(format!(
            "ai policy declares both top-level rules ({}) and states ({}): a stateful \
             policy resolves only inside its current state, so the top-level rules \
             would never fire. Move them into the state(s) that should own them, or \
             delete the state machine",
            cfg.rule.len(),
            cfg.state.len()
        ));
    }
    let stateful = !cfg.state.is_empty();

    // ── Per-rule checks, run unchanged over the top-level rules and over each
    // state's own rules (issue #882 extends the loop's reach, not its body).
    let check_rule = |what: &str, r: &FineSystemAiRuleToml| -> Result<(), String> {
        if !valid_channels.contains(&r.channel.as_str()) {
            return Err(format!(
                "ai policy {what} has unknown channel '{}' (valid: {valid_channels:?})",
                r.channel
            ));
        }
        if !valid_verbs.contains(&r.verb.as_str()) {
            return Err(format!(
                "ai policy {what} has unknown verb '{}' (valid: {valid_verbs:?})",
                r.verb
            ));
        }
        let pred = crate::world::flags::parse_predicate(&r.when)
            .map_err(|e| format!("ai policy {what} has invalid `when` expression: {e}"))?;
        check_policy_predicate(cfg, stateful, host, &pred, what)
    };
    for (idx, r) in cfg.rule.iter().enumerate() {
        check_rule(&format!("rule {idx}"), r)?;
    }
    check_rule_priorities("", &cfg.rule)?;

    if !stateful {
        return Ok(());
    }

    // ── State-graph checks (issue #882 AC6) ─────────────────────────────────
    let mut seen: Vec<&str> = Vec::with_capacity(cfg.state.len());
    for s in &cfg.state {
        if seen.contains(&s.id.as_str()) {
            return Err(format!("ai policy declares duplicate state id '{}'", s.id));
        }
        seen.push(&s.id);
    }
    let Some(initial) = cfg.initial_state.as_deref() else {
        return Err("ai policy declares states but no `initial_state`".into());
    };
    if !seen.contains(&initial) {
        return Err(format!(
            "ai policy `initial_state` names undeclared state '{initial}' (declared: {seen:?})"
        ));
    }
    for s in &cfg.state {
        for (tidx, t) in s.transition.iter().enumerate() {
            let what = format!("state '{}' transition {tidx}", s.id);
            if !seen.contains(&t.to.as_str()) {
                return Err(format!(
                    "ai policy {what} targets undeclared state '{}' (declared: {seen:?})",
                    t.to
                ));
            }
            let pred = crate::world::flags::parse_predicate(&t.when)
                .map_err(|e| format!("ai policy {what} has invalid `when` expression: {e}"))?;
            check_policy_predicate(cfg, stateful, host, &pred, &what)?;
        }
        check_transition_priorities(&s.id, &s.transition)?;
        for (idx, r) in s.rule.iter().enumerate() {
            check_rule(&format!("state '{}' rule {idx}", s.id), r)?;
        }
        check_rule_priorities(&format!("state '{}' ", s.id), &s.rule)?;
        // A leg that declines channel-3 arc-bearing requests (issue #918) can
        // only be read by a system that steers, and the `yaw` channel is what
        // makes a system one. Authored anywhere else it is a declaration
        // nothing will ever consult — the silent-no-op class this validator
        // exists to turn into a load error.
        if !s.yields_to_arc_requests && !valid_channels.contains(&HELM_YAW_CHANNEL) {
            return Err(format!(
                "ai policy state '{}' declares yields_to_arc_requests = false, but this \
                 system drives {valid_channels:?} and an arc-bearing request is answered \
                 on the '{HELM_YAW_CHANNEL}' channel: nothing would ever read the \
                 declaration",
                s.id
            ));
        }
    }
    // Reachability is a FIXPOINT walk from `initial`, following transitions only
    // out of states already known reachable. A single pass over every state's
    // transitions would only catch zero-inbound orphans: a disconnected cluster
    // (`initial = a`; `b -> c`; `c -> b`) is targeted by transitions and would
    // pass, yet nothing can ever enter it. Every transition target is already
    // known to name a declared state by the loop above, so this walk cannot
    // wander off the graph.
    let mut reachable: Vec<&str> = vec![initial];
    let mut frontier: Vec<&str> = vec![initial];
    while let Some(current) = frontier.pop() {
        let Some(s) = cfg.state.iter().find(|s| s.id == current) else {
            continue;
        };
        for t in &s.transition {
            if !reachable.contains(&t.to.as_str()) {
                reachable.push(&t.to);
                frontier.push(&t.to);
            }
        }
    }
    for s in &cfg.state {
        if !reachable.contains(&s.id.as_str()) {
            return Err(format!(
                "ai policy declares unreachable state '{}': it is neither the \
                 initial state nor the target of any transition",
                s.id
            ));
        }
    }
    Ok(())
}

/// Reject two or more transitions out of one state sharing a `priority`
/// (issue #794, PRD #774's "competing equal-priority transitions").
///
/// The transition set of a state is a single winner-take-all race: the runtime
/// takes the highest-priority ELIGIBLE transition and breaks ties by authoring
/// order. So an authored tie is not an ambiguity the runtime chokes on — it is
/// a decision the runtime makes and the file does not record. Moving one of the
/// two tables past the other then changes which state the hull enters, with no
/// value anywhere in the file having changed.
///
/// The pair is reported rather than the count, and both `to` targets are named:
/// the author's next question is always "which two?", and two ties in a
/// six-transition state are otherwise indistinguishable from one.
fn check_transition_priorities(
    state_id: &str,
    transitions: &[FineSystemAiTransitionToml],
) -> Result<(), String> {
    for (i, a) in transitions.iter().enumerate() {
        for (j, b) in transitions.iter().enumerate().skip(i + 1) {
            if a.priority == b.priority {
                return Err(format!(
                    "ai policy state '{state_id}' declares transitions {i} (to '{}') and {j} \
                     (to '{}') at the same priority {}: equal-priority transitions out of one \
                     state are resolved by authoring order, so the file never says which wins. \
                     Give them distinct priorities",
                    a.to, b.to, a.priority
                ));
            }
        }
    }
    Ok(())
}

/// Reject two or more rules on one output channel sharing a `priority`
/// (issue #794, PRD #774's "competing equal-priority rules on one output
/// channel").
///
/// The sibling of [`check_transition_priorities`], and the same defect: channel
/// resolution is winner-take-all per channel with ties broken by authoring
/// order. The scope is deliberately (channel, priority) and NOT priority alone
/// — rules on different channels never compete, so a tube authoring a
/// `torpedo_load` rule and a `torpedo_launch` rule both at priority 0 is
/// ordinary content, not a tie.
///
/// `scope` is `""` for a stateless policy's top-level list, or
/// `"state '<id>' "` for a machine's per-state list, so the message reads as a
/// sentence either way.
fn check_rule_priorities(scope: &str, rules: &[FineSystemAiRuleToml]) -> Result<(), String> {
    for (i, a) in rules.iter().enumerate() {
        for (j, b) in rules.iter().enumerate().skip(i + 1) {
            if a.priority == b.priority && a.channel == b.channel {
                return Err(format!(
                    "ai policy {scope}declares rules {i} (verb '{}') and {j} (verb '{}') on \
                     channel '{}' at the same priority {}: equal-priority rules on one output \
                     channel are resolved by authoring order, so the file never says which \
                     wins. Give them distinct priorities",
                    a.verb, b.verb, a.channel, a.priority
                ));
            }
        }
    }
    Ok(())
}

/// Shared guard-expression checks for a policy predicate (issues #775, #882).
///
/// `param(...)` must be declared; `memory(...)` must be declared AND the policy
/// must be stateful; `state_time` requires a stateful policy. The stateless
/// rejections are AC6's "a memory or state-time reference in a stateless
/// policy" — private state is meaningless without a state machine to own it,
/// and silently reading `false` would be a trap.
///
/// `host` carries the same reasoning one step further (issue #891): a
/// `flag(...)`/`counter(...)` reference is meaningless on a host that evaluates
/// with an empty flag chain, and would likewise read `false` for ever. It is
/// `None` only for host-less validation (unit tests, ad-hoc checks), where the
/// question has no answer to give.
fn check_policy_predicate(
    cfg: &FineSystemAiConfigToml,
    stateful: bool,
    host: Option<&crate::entities::ai_flag_hosts::AiHost>,
    pred: &crate::world::flags::Predicate,
    what: &str,
) -> Result<(), String> {
    if let Some(host) = host {
        host.check_guard(&format!("ai policy {what}"), pred)?;
        host.check_facts(&format!("ai policy {what}"), pred)?;
    }
    let mut refs = Vec::new();
    pred.referenced_params(&mut refs);
    for name in refs {
        if !cfg.param.contains_key(&name) {
            return Err(format!(
                "ai policy {what} references undeclared parameter '{name}'"
            ));
        }
    }
    let mut mem_refs = Vec::new();
    pred.referenced_memory(&mut mem_refs);
    if !stateful && !mem_refs.is_empty() {
        return Err(format!(
            "ai policy {what} references memory('{}') but the policy declares no states: \
             private memory requires a stateful policy",
            mem_refs[0]
        ));
    }
    for name in mem_refs {
        if !cfg.memory.contains_key(&name) {
            return Err(format!(
                "ai policy {what} references undeclared memory '{name}'"
            ));
        }
    }
    if !stateful && pred.references_state_time() {
        return Err(format!(
            "ai policy {what} references state_time but the policy declares no states: \
             state time requires a stateful policy"
        ));
    }
    check_history_windows(cfg, stateful, pred, what)
}

/// Reject an authored `history(...)` atom the runtime could not honour
/// (issue #890).
///
/// Two rejections, and they close the two halves of the same trap:
///
/// * a history atom in a STATELESS policy. The window is per-fine-system
///   retained state carried on the same private bag as `memory(...)`, folded by
///   the host that ticks the state machine — a policy with no machine is never
///   ticked, so the window would never be advanced and the guard would read
///   false for ever. (`AiHost::check_guard` catches the sibling case: a stateful
///   policy on a host with no fold at all.)
/// * a window length that is not a positive whole number of shared AI ticks. A
///   literal is caught by the parser, which is the only place that sees it; a
///   `param(...)` can only be checked HERE, against its declared value, and a
///   fractional or zero one would silently disable the operator (a zero-capacity
///   window retains nothing and is never full — see [`crate::bounded_history`]).
fn check_history_windows(
    cfg: &FineSystemAiConfigToml,
    stateful: bool,
    pred: &crate::world::flags::Predicate,
    what: &str,
) -> Result<(), String> {
    let mut refs = Vec::new();
    pred.referenced_history(&mut refs);
    if refs.is_empty() {
        return Ok(());
    }
    if !stateful {
        return Err(format!(
            "ai policy {what} reads {} but the policy declares no states: a bounded \
             history window is per-fine-system retained state, advanced once per \
             shared AI tick by the host that ticks the state machine, so it requires \
             a stateful policy",
            refs[0].render()
        ));
    }
    for atom in &refs {
        let ticks = match &atom.window.ticks {
            crate::world::flags::Operand::Number(n) => *n,
            // An UNDECLARED parameter is already the caller's error above; this
            // arm only skips re-reporting it under a worse message.
            crate::world::flags::Operand::Param(name) => match cfg.param.get(name) {
                Some(value) => *value as f64,
                None => continue,
            },
        };
        if !ticks.is_finite() || ticks.fract() != 0.0 || ticks < 1.0 {
            return Err(format!(
                "ai policy {what} reads {} whose window length resolves to {ticks}: a \
                 history window counts shared AI ticks, so it must be a positive whole \
                 number",
                atom.render()
            ));
        }
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PowerConfigSection {
    pub capacity: f32,
    pub rates: [f32; 6],
    #[serde(default = "default_sustainable_power_total")]
    pub sustainable_total: u8,
    #[serde(default = "default_max_commanded_power_total")]
    pub max_commanded_total: u8,
    pub emergency_threshold: f32,
    /// Inline stateless AI policy for the Power reactor fine system (issue
    /// #784), loaded from `[power.ai_policy]`. Replaces the retired stateful
    /// `[power.ai]` engine (`PowerAiConfigToml` + `EngageState` hysteresis).
    /// Each authored `[[power.ai_policy.rule]]` binds a `priority` and a power
    /// GROUP `channel` to a `when` guard and the value-carrying
    /// `set_power_group_allocation` verb (its `level` payload the absolute
    /// target). Every allocation rule declares a minimum battery reserve as a
    /// `param(...)` referenced by its guard (AC2). Absent, the canonical
    /// [`default_power_ai_config`] is synthesised at spawn (baseline
    /// preservation). Validated in [`EntityConfig::from_toml`] against
    /// [`POWER_SET_ALLOCATION_VERB`] and a valid-channel set built dynamically
    /// from the ship's `[power_groups.*]` keys (AC1 — no fixed catalogue).
    #[serde(default)]
    pub ai_policy: Option<FineSystemAiConfigToml>,
}

const fn default_sustainable_power_total() -> u8 {
    6
}

const fn default_max_commanded_power_total() -> u8 {
    8
}

/// AI tuning parameters for the shields focus controller.
///
/// Loaded from `[shields_console.ai]` in the ship entity TOML.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShieldsAiConfigToml {
    /// Maximum time window (seconds) for tracking incoming damage per arc.
    /// Damage older than this is pruned.
    #[serde(default = "default_shields_ai_damage_window_secs")]
    pub damage_window_secs: f32,
    /// Minimum time window (seconds) before the AI acts on damage concentration.
    /// Damage newer than this is not yet considered.
    #[serde(default = "default_shields_ai_min_damage_window_secs")]
    pub min_damage_window_secs: f32,
    /// Percentage of total damage in the window that must hit the same arc
    /// before the AI focuses it (0–100).
    #[serde(default = "default_shields_ai_damage_pct_threshold")]
    pub damage_pct_threshold: f32,
    /// Percentage threshold: if the lowest-arc normalized health is below this
    /// fraction of the next-lowest arc, focus the weakest arc (0–100).
    #[serde(default = "default_shields_ai_health_ratio_threshold")]
    pub health_ratio_threshold: f32,
}

fn default_shields_ai_damage_window_secs() -> f32 {
    4.0
}
fn default_shields_ai_min_damage_window_secs() -> f32 {
    1.0
}
fn default_shields_ai_damage_pct_threshold() -> f32 {
    50.0
}
fn default_shields_ai_health_ratio_threshold() -> f32 {
    50.0
}

/// Config block for the Shields console focus bonuses/penalties.
///
/// Loaded from `[shields_console]` in the ship entity TOML. The nested
/// `[shields_console.base]` sub-block (modelled by [`ShieldsBaseConfig`])
/// supplies the underlying shield-system base values (number of facings,
/// max HP, regen, offline duration) that were previously hardcoded by
/// `ShieldConfig::default()` at `src/weapons/shield.rs:50-58`.
///
/// Damage multiplier fields `focus_focused_damage_multiplier` and
/// `focus_unfocused_damage_multiplier` default to 1.0 (no change).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShieldsConsoleConfig {
    /// Per-level bonus table for the `shields` power group (issue #952),
    /// indexed by level-1. Feeds `ModifierSlot::ShieldRegen`, so it decides
    /// what a reactor point spent here buys in regeneration. Absent ⇒ the
    /// fleet-wide `[-0.5, 0.0, 0.25, 0.5]` default.
    ///
    /// This field moved here from `[sensors_console]` when `shields` replaced
    /// `sensors` as a power group: nothing reads a `sensors` curve any more,
    /// because `ModifierSlot::RadarRange` no longer has a power producer.
    #[serde(default)]
    pub power_multipliers: Option<[f32; 4]>,
    /// Extra max HP applied to the focused facing.
    #[serde(default = "default_focus_bonus_max_hp")]
    pub focus_bonus_max_hp: i32,
    /// Extra regen per second applied to the focused facing.
    #[serde(default = "default_focus_bonus_regen")]
    pub focus_bonus_regen: f32,
    /// Max HP subtracted from each non-focused facing.
    #[serde(default = "default_focus_penalty_max_hp")]
    pub focus_penalty_max_hp: i32,
    /// Regen per second subtracted from each non-focused facing.
    #[serde(default = "default_focus_penalty_regen")]
    pub focus_penalty_regen: f32,
    /// HP per second decay applied to non-focused facings when above reduced max.
    #[serde(default = "default_focus_decay_rate")]
    pub focus_decay_rate: f32,
    /// Damage multiplier applied to incoming damage on the focused arc.
    /// 1.0 = no change, 0.7 = 30% reduction.
    #[serde(default = "default_focus_focused_damage_multiplier")]
    pub focus_focused_damage_multiplier: f32,
    /// Damage multiplier applied to incoming damage on non-focused arcs
    /// (when another arc is focused). 1.0 = no change, 1.25 = 25% increase.
    #[serde(default = "default_focus_unfocused_damage_multiplier")]
    pub focus_unfocused_damage_multiplier: f32,
    /// Base shield-system values (number of facings, max HP, regen,
    /// offline duration). When absent the historical hardcoded defaults
    /// from `ShieldConfig::default()` are used.
    #[serde(default)]
    pub base: Option<ShieldsBaseConfig>,
    /// Shield generator frequency (0.0–1.0). Default 0.5. When
    /// `[[shield_arc]]` blocks are present, the first arc's frequency
    /// takes precedence.
    #[serde(default = "default_shield_frequency")]
    pub frequency: f32,
    /// AI tuning parameters for the shields focus controller.
    #[serde(default)]
    pub ai: Option<ShieldsAiConfigToml>,
    /// Inline stateless AI policy for the Shields focus fine system (issue
    /// #783), loaded from `[shields_console.ai_policy]`. Sibling to `ai`: the
    /// typed `ai` knobs remain the fallback default source, while this block —
    /// when authored — carries the same four numbers as `param(...)` entries
    /// (`damage_window_secs`, `min_damage_window_secs`, `damage_pct_threshold`,
    /// `health_ratio_threshold`) plus the prioritised rules that gate whether the
    /// retained arc-ranking kernel acts. Absent, the canonical
    /// [`default_shields_focus_ai_config`] is synthesised at spawn (baseline
    /// preservation). Validated in [`crate::entities::config::EntityConfig::from_toml`]
    /// against [`SHIELD_FOCUS_CHANNELS`] / [`SHIELD_FOCUS_VERBS`].
    #[serde(default)]
    pub ai_policy: Option<FineSystemAiConfigToml>,
}

fn default_focus_bonus_max_hp() -> i32 {
    50
}
fn default_focus_bonus_regen() -> f32 {
    5.0
}
fn default_focus_penalty_max_hp() -> i32 {
    25
}
fn default_focus_penalty_regen() -> f32 {
    1.0
}
fn default_focus_decay_rate() -> f32 {
    10.0
}
fn default_focus_focused_damage_multiplier() -> f32 {
    1.0
}
fn default_focus_unfocused_damage_multiplier() -> f32 {
    1.0
}

impl Default for ShieldsConsoleConfig {
    fn default() -> Self {
        Self {
            power_multipliers: None,
            focus_bonus_max_hp: default_focus_bonus_max_hp(),
            focus_bonus_regen: default_focus_bonus_regen(),
            focus_penalty_max_hp: default_focus_penalty_max_hp(),
            focus_penalty_regen: default_focus_penalty_regen(),
            focus_decay_rate: default_focus_decay_rate(),
            focus_focused_damage_multiplier: default_focus_focused_damage_multiplier(),
            focus_unfocused_damage_multiplier: default_focus_unfocused_damage_multiplier(),
            base: None,
            frequency: default_shield_frequency(),
            ai: None,
            ai_policy: None,
        }
    }
}

/// Base shield-system values loaded from `[shields_console.base]`.
///
/// These map 1:1 onto `crate::weapons::shield::ShieldConfig` (the runtime struct
/// consumed by `ShieldSystem::new`). All fields default to the historical
/// hardcoded values from `ShieldConfig::default()` so omitting the block
/// changes nothing.
///
/// `num_facings` is exposed for symmetry but the client panel UI assumes
/// 4 quadrants — values other than 4 will break the Shields panel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShieldsBaseConfig {
    /// Number of equally-spaced shield facings. The client panel UI
    /// assumes 4 (fore/port/aft/starboard); other values will not render
    /// correctly.
    #[serde(default = "default_shields_num_facings")]
    pub num_facings: usize,
    /// Maximum HP per facing.
    #[serde(default = "default_shields_max_hp")]
    pub max_hp: i32,
    /// HP regenerated per second per online facing.
    #[serde(default = "default_shields_regen_per_sec")]
    pub regen_per_sec: f32,
    /// Seconds a facing stays offline after its HP is depleted.
    #[serde(default = "default_shields_offline_duration")]
    pub offline_duration: f32,
}

fn default_shields_num_facings() -> usize {
    4
}
fn default_shields_max_hp() -> i32 {
    100
}
fn default_shields_regen_per_sec() -> f32 {
    2.0
}
fn default_shields_offline_duration() -> f32 {
    10.0
}

impl Default for ShieldsBaseConfig {
    fn default() -> Self {
        Self {
            num_facings: default_shields_num_facings(),
            max_hp: default_shields_max_hp(),
            regen_per_sec: default_shields_regen_per_sec(),
            offline_duration: default_shields_offline_duration(),
        }
    }
}

impl ShieldsBaseConfig {
    /// Convert this TOML config into a runtime `ShieldConfig`.
    pub fn to_runtime(&self) -> crate::weapons::shield::ShieldConfig {
        crate::weapons::shield::ShieldConfig {
            num_facings: self.num_facings,
            max_hp: self.max_hp,
            regen_per_sec: self.regen_per_sec,
            offline_duration: self.offline_duration,
        }
    }
}

/// Designer-authored per-arc shield config (issue #514).
///
/// Loaded from top-level `[[shield_arc]]` blocks in the ship TOML. Every
/// arc auto-generates a matching `[[system]]` entry with
/// `kind = "shield_arc"` and `SystemId("shield-arc-<id>")` during
/// `EntityConfig::from_toml`. See [`ShieldArcConfig::to_runtime`] for the
/// runtime conversion consumed by [`crate::weapons::shield::ShieldSystem::from_arcs`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShieldArcConfig {
    /// Stable arc id (kebab-case fragment used to build the fine SystemId).
    pub id: String,
    /// Display label shown in the JS panel (e.g. `"Fore"`, `"All"`).
    pub label: String,
    /// Arc centre bearing in degrees (0 = fore, 90 = starboard).
    pub center_deg: f32,
    /// Arc angular width in degrees.
    pub width_deg: f32,
    /// Per-arc override for max HP; falls back to `[shields_console.base] max_hp`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_hp: Option<i32>,
    /// Per-arc override for regen/sec.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub regen_per_sec: Option<f32>,
    /// Per-arc override for offline duration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offline_duration: Option<f32>,
    /// Per-arc hull HP for damage-tier tracking (fed into `ShipArcHull`).
    #[serde(default)]
    pub hull_max_hp: f32,
    /// HP fraction below which the arc enters Damaged tier. Default 0.75.
    #[serde(default = "default_arc_hull_damaged_threshold")]
    pub hull_damaged_threshold_pct: f32,
    /// HP fraction below which the arc enters Disabled tier. Default 0.25.
    #[serde(default = "default_arc_hull_disabled_threshold")]
    pub hull_disabled_threshold_pct: f32,
    /// Debuff magnitude applied on Damaged/Disabled tier. Default 0.15.
    #[serde(default = "default_arc_hull_debuff_magnitude")]
    pub hull_debuff_magnitude: f32,
    /// Hit-routing priority. When multiple arcs cover the same bearing, the
    /// arc with the highest `priority` value absorbs the hit first. Falls
    /// through to the next tier only when all arcs at the higher priority
    /// covering that bearing are offline. Default 1.
    #[serde(default = "default_arc_priority")]
    pub priority: u32,
    /// Shield generator frequency (0.0–1.0) for this arc's shield system.
    /// When multiple arcs are declared, the first arc's frequency seeds the
    /// ship-wide shield frequency. Default 0.5.
    #[serde(default = "default_shield_frequency")]
    pub frequency: f32,
}

fn default_shield_frequency() -> f32 {
    0.5
}

fn default_arc_priority() -> u32 {
    1
}

fn default_arc_hull_damaged_threshold() -> f32 {
    0.75
}

fn default_arc_hull_disabled_threshold() -> f32 {
    0.25
}

fn default_arc_hull_debuff_magnitude() -> f32 {
    0.15
}

impl ShieldArcConfig {
    /// Convert this TOML block into the runtime shape consumed by
    /// [`crate::weapons::shield::ShieldSystem::from_arcs`].
    pub fn to_runtime(&self) -> crate::weapons::shield::ArcRuntimeConfig {
        crate::weapons::shield::ArcRuntimeConfig {
            id: self.id.clone(),
            label: self.label.clone(),
            center_deg: self.center_deg,
            width_deg: self.width_deg,
            max_hp: self.max_hp,
            regen_per_sec: self.regen_per_sec,
            offline_duration: self.offline_duration,
            priority: self.priority,
        }
    }
}

/// Player-ship phaser combat tuning. All per-bank values (`beam_range`,
/// `beam_damage_per_sec`, `beam_duration_secs`, `cooldown_secs`,
/// `beam_color`, `shield_pierce`) live on each [`PhaserBankConfig`] entry.
///
/// `PhaserCombatConfig` is the player-path source of truth, installed
/// as a Bevy resource by `WeaponsPlugin` and overridden in
/// `spawn_game_start_entities` from the player ship's `[weapons_console]`
/// block.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PhaserCombatConfig {
    /// Per-bank facing/arc/range/damage/duration/cooldown/colour list,
    /// parsed from `[[weapons_console.phaser_banks]]` in TOML order. Empty
    /// if the ship has no banks configured. The Tactical UI also receives a
    /// stripped subset of these via `PhaserBankClientConfig`.
    pub banks: Vec<PhaserBankConfig>,
}

impl PhaserCombatConfig {
    /// Canonical baseline phaser values used when a bank's field is `0.0`
    /// (the "zero means absent" convention). Other modules needing the
    /// baseline alias these constants rather than restating the numbers.
    pub const DEFAULT_PHASER_RANGE: f32 = 40.0;
    pub const DEFAULT_BEAM_DURATION_SECS: f32 = 6.0;
    pub const DEFAULT_BEAM_COOLDOWN_SECS: f32 = 6.0;
    pub const DEFAULT_BEAM_DAMAGE_PER_SEC: f32 = 5.0;
}

impl PhaserCombatConfig {
    /// Build a `PhaserCombatConfig` from a parsed `[weapons_console]` block.
    /// All combat tuning is now per-bank; this method just clones the banks list.
    pub fn from_weapons_console(wc: &WeaponsConsoleConfig) -> Self {
        Self {
            banks: wc.phaser_banks.clone(),
        }
    }

    /// Look up a bank by its id. Returns `None` if not found.
    pub fn bank_by_id(&self, id: &str) -> Option<&PhaserBankConfig> {
        self.banks.iter().find(|b| b.id == id)
    }
}

/// Config block for the repair-team state machine in a ship TOML.
///
/// Loaded from `[repair]` in the ship entity TOML (and any NPC ship TOML
/// that wishes to override repair pacing). All fields are optional; missing
/// fields fall back to the same defaults as `RepairTimings::default()` and
/// to the historical hardcoded constants (`TRAVEL_DURATION = 5.0`,
/// `REPAIR_RATE_HP_PER_SEC = 0.5`).
///
/// The same values are forwarded to the client via
/// `ShipClientConfig.repair_travel_secs` and
/// `ShipClientConfig.repair_rate_hp_per_sec` so that the Repair panel UI
/// can derive its progress-bar timings without redefining the constants.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepairConfig {
    /// Number of repair teams available to this ship. Absent ⇒ 0 ⇒ this ship
    /// has no repair teams — see [`Self::declares_teams`].
    #[serde(default)]
    pub repair_team_count: u32,
    /// Seconds a team spends travelling to a console (and the same again returning).
    #[serde(default = "default_repair_travel_duration_secs")]
    pub travel_duration_secs: f32,
    /// HP restored per second while a team is at a console.
    #[serde(default = "default_repair_rate_hp_per_sec")]
    pub repair_rate_hp_per_sec: f32,
    /// Inline per-system target selector (issue #785). Loaded from
    /// `[repair.selector]`; absent ⇒ the canonical
    /// [`default_repair_target_selector_config`] is synthesised at spawn.
    /// `operate_repair_ai` runs it once per free team to rank the ship's
    /// damaged stations into ordinary admitted `DispatchRepairTeam` inputs.
    ///
    /// This is the first selector block that is NOT inside a `*_console`
    /// section: repair teams are a ship-wide engineering capability whose
    /// tunables already live under `[repair]`, so the selector joins them there
    /// rather than inventing a `[repair_console]` table the wire never uses.
    #[serde(default)]
    pub selector: Option<FineSystemAiSelectorToml>,
    /// External repair-team dispatch (issue #1161). Loaded from
    /// `[repair.external_dispatch]`; present on a hull whose repair console can
    /// send a team to a nearby ally or structure, absent for everything else —
    /// which carries no `ExternalRepairDispatch` component and cannot dispatch a
    /// team abroad. It joins the other repair tunables under `[repair]` for
    /// `selector`'s reason: a team crossing over is a repair-console capability,
    /// not a `[[system]]` of its own. The reach and the repair rate are its two
    /// authored numbers (AGENTS.md rule 11).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_dispatch: Option<crate::console::repair::external::ExternalRepairConfig>,
}

fn default_repair_travel_duration_secs() -> f32 {
    5.0
}
fn default_repair_rate_hp_per_sec() -> f32 {
    0.5
}

impl Default for RepairConfig {
    fn default() -> Self {
        Self {
            repair_team_count: 0,
            travel_duration_secs: default_repair_travel_duration_secs(),
            repair_rate_hp_per_sec: default_repair_rate_hp_per_sec(),
            selector: None,
            external_dispatch: None,
        }
    }
}

impl RepairConfig {
    /// Whether this block gives the ship repair TEAMS, as opposed to existing
    /// only to carry `[repair.selector]`.
    ///
    /// Until #885b every NPC hull that authored no `[repair]` block had no
    /// teams, and the spawner used the block's mere PRESENCE as the gate. That
    /// stopped working the moment every hull had to author `[repair.selector]`
    /// to satisfy PRD #774 US7: a selector is a ranking policy, and TOML has no
    /// way to write `[repair.selector]` without also bringing `[repair]` into
    /// existence. Presence would then have handed two repair teams to six NPC
    /// hulls that never had any — a gameplay change smuggled in by a table
    /// header.
    ///
    /// So the gate is the count: **a ship has repair teams when its TOML says
    /// how many.** `repair_team_count = 0`, or omitted, means none. The
    /// `[repair.selector]` block is attached to every AI-bearing ship either
    /// way — the teams component is what gates dispatch, so a ship that gains
    /// teams later already has its ranking.
    ///
    /// The PLAYER ship does not come through here: `spawn_game_start_entities`
    /// gates its teams on `[hull]` and keeps its own `unwrap_or(2)` fallback,
    /// so a player hull that omits the count still crews two teams.
    pub fn declares_teams(&self) -> bool {
        self.repair_team_count > 0
    }

    /// Convert this TOML config into a runtime `RepairTimings`.
    pub fn to_runtime(&self) -> crate::modifiers::repair_teams::RepairTimings {
        crate::modifiers::repair_teams::RepairTimings {
            travel_duration: self.travel_duration_secs,
            repair_rate_hp_per_sec: self.repair_rate_hp_per_sec,
        }
    }
}

/// Config block for an entity's comms reachability.
///
/// Loaded from `[comms]` in entity TOMLs. When present, the entity is
/// reachable by the player's Comms console while inside `range` units of the
/// player ship. The player ship's own `[comms].range` defines how far it can
/// listen. Effective range between two entities is `min(a.range, b.range)`.
///
/// `range` alone says only "this endpoint is range-gated" — it does NOT put the
/// entity on the hail roster. `hailable = true` does (issue #985). The split is
/// deliberate: every shipped warship and station declares a range, so deriving
/// the roster from `range` alone would make every enemy wave ship in
/// `combat_test` hailable. See [`crate::comms::roster`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommsConfig {
    /// Comms range in world units.
    pub range: f32,
    /// Opt in to the hail roster (issue #985). `false` (the default) means the
    /// entity is a range-gated comms endpoint but not something the Comms
    /// officer can call up; the world's `[[comms]]` templates remain the only
    /// thing that puts it on the roster. Set `true` and the live entity is
    /// unioned into the roster for as long as it exists.
    #[serde(default)]
    pub hailable: bool,
    /// Optional player-facing label for the contact row, independent of the
    /// entity's `name` reference id (mirrors `[[comms]] display_name`, issue
    /// #751). `None` falls back to the entity's `EntityName`, then its UUID.
    /// Only meaningful alongside `hailable = true`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}

/// Config block for the torpedo system in a ship TOML.
///
/// Loaded from `[torpedoes]` in the ship entity TOML (and any NPC ship TOML
/// that wishes to override the torpedo loadout). All fields are optional;
/// missing fields fall back to the same defaults as `TorpedoConfig::default()`.
///
/// `turn_rate_deg_per_sec` is in **degrees per second** for designer
/// readability; it is converted to radians by `to_runtime()`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TorpedoesConfig {
    #[serde(default = "default_torpedo_count")]
    pub count: u32,
    #[serde(default = "default_torpedo_damage_hull")]
    pub damage_hull: i32,
    #[serde(default = "default_torpedo_damage_shields")]
    pub damage_shields: i32,
    #[serde(default = "default_torpedo_speed")]
    pub speed: f32,
    /// Maximum turn rate in **degrees per second** (homing).
    /// Converted to radians by `to_runtime()`.
    #[serde(default = "default_torpedo_turn_rate_deg_per_sec")]
    pub turn_rate_deg_per_sec: f32,
    #[serde(default = "default_torpedo_lifespan")]
    pub lifespan: f32,
    #[serde(default = "default_torpedo_load_time")]
    pub load_time: f32,
    /// Proximity-detonation radius in world units.
    #[serde(default = "default_torpedo_detonation_radius")]
    pub detonation_radius: f32,
    /// Fraction of `damage_shields` that bypasses shields and adds to
    /// hull damage at detonation. Default `0.0` — `damage_shields` is
    /// fully absorbed by the facing shield quadrant. `damage_hull` is
    /// unaffected (always hits hull). Clamped to `[0.0, 1.0]` at apply
    /// time.
    #[serde(default)]
    pub shield_pierce: f32,
    /// Per-tube torpedo definitions parsed from `[[torpedoes.tubes]]`.
    /// Each tube has its own facing and fire arc. Ammo is shared via
    /// the top-level `count` field. Empty when the ship has no explicit
    /// per-tube loadout.
    #[serde(default)]
    pub tubes: Vec<TorpedoTubeConfig>,
    /// Interval in seconds between successive torpedo launches in a burst
    /// volley. Applies to all tubes on the ship. Default `0.3s`.
    #[serde(default = "default_burst_interval_secs")]
    pub burst_interval_secs: f32,
    /// Ship-wide default for `[[torpedoes.tubes]] ai_target_count` — how many
    /// rounds an AI-operated crew keeps loaded in each tube. A per-tube
    /// `ai_target_count` overrides it; when both are absent each tube falls
    /// back to its own `volley_max`.
    #[serde(default)]
    pub ai_volley_target: Option<u32>,
    /// Inline stateless AI policy for the shared magazine's grant decision
    /// (issue #782, AC1). When authored it is validated at content load and
    /// resolved inside `handle_torpedo_magazine_inter_system` right before the
    /// authoritative `claim_magazine_round`; when absent the canonical
    /// [`default_torpedo_magazine_ai_config`] (unconditional grant) is
    /// synthesised at spawn so baseline claim behaviour is preserved. The offline
    /// gate stays the hard authority; this is a data-authored arbiter on top.
    #[serde(default)]
    pub ai: Option<FineSystemAiConfigToml>,
}

fn default_burst_interval_secs() -> f32 {
    0.3
}

fn default_torpedo_count() -> u32 {
    10
}
fn default_torpedo_damage_hull() -> i32 {
    50
}
fn default_torpedo_damage_shields() -> i32 {
    5
}
fn default_torpedo_speed() -> f32 {
    15.0
}
fn default_torpedo_turn_rate_deg_per_sec() -> f32 {
    45.0
}
fn default_torpedo_lifespan() -> f32 {
    20.0
}
fn default_torpedo_load_time() -> f32 {
    10.0
}
fn default_torpedo_detonation_radius() -> f32 {
    5.0
}

impl Default for TorpedoesConfig {
    fn default() -> Self {
        Self {
            count: default_torpedo_count(),
            damage_hull: default_torpedo_damage_hull(),
            damage_shields: default_torpedo_damage_shields(),
            speed: default_torpedo_speed(),
            turn_rate_deg_per_sec: default_torpedo_turn_rate_deg_per_sec(),
            lifespan: default_torpedo_lifespan(),
            load_time: default_torpedo_load_time(),
            detonation_radius: default_torpedo_detonation_radius(),
            shield_pierce: 0.0,
            tubes: Vec::new(),
            burst_interval_secs: default_burst_interval_secs(),
            ai_volley_target: None,
            ai: None,
        }
    }
}

impl TorpedoesConfig {
    /// Convert this TOML config into a runtime `TorpedoConfig`.
    /// Performs the degrees → radians conversion on `turn_rate_deg_per_sec`.
    pub fn to_runtime(&self) -> crate::weapons::torpedo::TorpedoConfig {
        crate::weapons::torpedo::TorpedoConfig {
            count: self.count,
            damage_hull: self.damage_hull,
            damage_shields: self.damage_shields,
            speed: self.speed,
            turn_rate: self.turn_rate_deg_per_sec.to_radians(),
            lifespan: self.lifespan,
            load_time: self.load_time,
            detonation_radius: self.detonation_radius,
            shield_pierce: self.shield_pierce,
            burst_interval_secs: self.burst_interval_secs,
            ai_volley_target: self.ai_volley_target,
        }
    }
}

/// Config block for the Navigation console in a ship TOML.
///
/// Loaded from `[navigation_console]` in the ship entity TOML.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NavigationConsoleConfig {
    /// System chart radar config for the Navigation console.
    #[serde(default)]
    pub system_chart: crate::radar_config::RadarConfig,
    /// Inline per-system target selector (issue #778). Loaded from
    /// `[navigation_console.selector]`; absent ⇒ the canonical
    /// [`default_navigation_target_selector_config`] is synthesised at spawn.
    /// `operate_navigation_ai` runs it to rank objective destinations and
    /// eligible chart contacts into the shared Waypoint.
    #[serde(default)]
    pub selector: Option<FineSystemAiSelectorToml>,
}

/// Config block for the Sensors console in a ship TOML.
///
/// Loaded from `[sensors_console]` in the ship entity TOML.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SensorsConsoleConfig {
    /// Long-range radar config for the Sensors console.
    #[serde(default)]
    pub long_range_radar: crate::radar_config::RadarConfig,
    /// AI tuning parameters for the Sensors frequency-hint controller.
    /// Loaded from `[sensors_console.ai]`.
    #[serde(default)]
    pub ai: Option<SensorsAiConfigToml>,
    /// Inline per-system target selector (issue #776). Loaded from
    /// `[sensors_console.selector]`; absent ⇒ the canonical
    /// [`default_sensors_target_selector_config`] is synthesised at spawn.
    #[serde(default)]
    pub selector: Option<FineSystemAiSelectorToml>,
}

/// AI tuning parameters for the Sensors frequency-hint controller
/// (`console_ai::tick_frequency_hint`, issue #692).
///
/// Loaded from `[sensors_console.ai]` in the ship entity TOML.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SensorsAiConfigToml {
    /// Delay (seconds) between a target lock and the AI-driven Sensors
    /// operator emitting a `FrequencyHint` coordination message to Tactical.
    #[serde(default = "default_sensors_ai_frequency_hint_delay_secs")]
    pub frequency_hint_delay_secs: f32,
}

fn default_sensors_ai_frequency_hint_delay_secs() -> f32 {
    3.0
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct EntityConfig {
    /// Display name (top-level scalar). Informational for most entities; used
    /// by triggers/comms to identify named instances.
    ///
    /// This doubles as a MACHINE identity: a world `[[entity]] name` overrides
    /// it with the instance name (e.g. `wave_1`) so triggers can target the
    /// spawned hull. That is why it cannot also be the crew-facing SHIP name —
    /// see [`Self::display_name`].
    pub name: Option<String>,
    /// The crew-facing PROPER NAME of a ship (e.g. "AEV Phoenix"), a
    /// `strings.csv` id resolved on the client. Distinct from [`Self::name`]:
    /// a world instance name overwrites `name` for trigger targeting, but a
    /// ship's proper name is a property of the HULL, not of the spawn, so it
    /// lives in its own field that the instance name never touches. When present
    /// it is what scans, comms, the radar and the ship picker SHOW; `name` (or
    /// the instance name) remains the identity underneath. Absent for anything
    /// that is not a named ship, which shows `name` exactly as before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub hull: Option<HullConfig>,
    pub collider: Option<ColliderConfig>,
    pub appearance: Option<AppearanceConfig>,
    pub helm_console: Option<HelmConsoleConfig>,
    /// Optional helm capability declaration (`[helm_capability]`).
    /// Describes vertical movement mode and impulse steering policy.
    #[serde(default)]
    pub helm_capability: Option<HelmCapabilityConfig>,
    pub weapons_console: Option<WeaponsConsoleConfig>,
    pub engineering_console: Option<EngineeringConsoleConfig>,
    pub captain_console: Option<CaptainConsoleConfig>,
    /// Comms CONSOLE config (issue #786): the hail `selector` and the
    /// dialogue-response `ai` policy. Distinct from the `comms` field below,
    /// which is the per-entity comms RANGE.
    #[serde(default)]
    pub comms_console: Option<CommsConsoleConfig>,
    pub power: Option<PowerConfigSection>,
    pub sensors_console: Option<SensorsConsoleConfig>,
    pub navigation_console: Option<NavigationConsoleConfig>,
    /// Unified shields config. Contains focus tuning at the top level and
    /// `num_facings` / `max_hp` / `regen_per_sec` / `offline_duration` in
    /// the nested `.base` sub-block. Every ship (player + NPC) reads this
    /// section — the legacy `[shields]` block was removed as part of the
    /// ship parity audit.
    pub shields_console: Option<ShieldsConsoleConfig>,
    /// Torpedo system config (player ship and any NPC ship with torpedoes).
    pub torpedoes: Option<TorpedoesConfig>,
    /// Repair team timings (travel duration, repair rate).
    pub repair: Option<RepairConfig>,
    /// Ship audio: filenames and tuning for the ambient bed, engine, blaster,
    /// phaser loop, and forcefield. Server-only playback — the host page's JS
    /// builds its audio graph from this. `None` ⇒ the ship is silent.
    #[serde(default)]
    pub audio: Option<crate::audio_config::ShipAudioConfig>,
    /// Comms range — when present, the entity can send/receive comms within
    /// this radius of the player ship.
    pub comms: Option<CommsConfig>,
    /// Asteroid field section from entity template (donut params, grid, etc.)
    pub asteroid_field: Option<AsteroidFieldConfig>,
    /// Region shape section — present for region entities.
    pub shape: Option<RegionShape>,
    /// Region effects section — present for region entities with effects.
    pub effects: Option<RegionEffectsConfig>,
    /// Infrastructure condition + capacity (issue #1025). Present for authored
    /// world furniture — skyhooks, fuel depots, transfer platforms — that
    /// degrades and is repaired over a mission and publishes named capacities.
    /// Absent for everything else, which behaves exactly as it did before this
    /// section existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub infrastructure: Option<crate::infrastructure::InfrastructureConfig>,
    /// The sensor suite's scan capability (issue #1032). Present on a hull whose
    /// science station can take a reading of an external structure; absent for
    /// everything else, which can scan nothing and is refused by name if asked.
    /// The mirror image of `infrastructure`: that table says what can be *done
    /// to* an entity, this one says what an entity can *read*.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scan: Option<crate::science::ScanConfig>,
    /// The tractor beam's coupling terms (issue #1156) — range, rig offset and
    /// minimum power level. Present on a hull whose engineering seat can take a
    /// derelict under tow; absent for everything else, which carries no
    /// `TractorBeam` component and is unchanged in every way. The `[[system]]
    /// kind = "tractor"` block declares the system's identity (power group,
    /// station, damage entry); this table carries what the coupling itself is,
    /// and a hull that authors one without the other is refused by name at load.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tractor: Option<crate::tractor::TractorConfig>,
    /// What being held by a tractor DOES to this entity as a TARGET (issue
    /// #1158) — follow, arrest-decline, station-keep or formation-keep. The
    /// mirror of `tractor`: that table says what a hull can do the holding
    /// with, this one says what happens to the thing held. Absent for every
    /// entity that authors nothing, which is merely held in place (station-keep)
    /// exactly as #1156 held it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub held_response: Option<crate::tractor::HeldResponseConfig>,
    /// The dock terms (issue #1159) — range, engage distance, approach speed,
    /// mate tolerance, undock clearance and minimum power level. Its presence
    /// opts a hull into docking: it can be docked WITH (its dock markers are read
    /// from the rig sidecar into a `DockMarkers` component), and, when paired with
    /// a `[[system]] kind = "dock"` block, it can actively dock. A hull that
    /// authors no `[dock]` table carries no dock markers and no `DockControl`, can
    /// neither dock nor be docked with, and is unchanged in every way — which is
    /// why the shipped destroyer is untouched by this slice and the probe fields
    /// dedicated hulls.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dock: Option<crate::dock::DockConfig>,
    /// The transfer-umbilical terms (issue #1160) — the capacity id it moves, the
    /// rate, the direction and the minimum power level. Present on a hull whose
    /// engineering seat can pass a capacity across a dock; absent for everything
    /// else, which carries no `TransferUmbilical` component and is unchanged in
    /// every way. The `[[system]] kind = "umbilical"` block declares the system's
    /// identity (power group, station, damage entry); this table carries what the
    /// flow itself is, and a hull that authors one without the other is refused by
    /// name at load. Both docked ends must carry an `[[infrastructure.capacity]]`
    /// under the umbilical's `capacity` id for anything to move.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub umbilical: Option<crate::umbilical::UmbilicalConfig>,
    /// The faint world-locked lattice drawn under this hull on the viewscreen.
    /// Present only on a hull meant to be FLOWN — the grid is a motion cue for
    /// the crew looking out of their own ship, and it is only ever read off the
    /// LOCAL ship's resolved config, so an NPC copy of an authored hull still
    /// draws nothing. Absent for everything else, which renders exactly as it
    /// did before this table existed.
    ///
    /// Inert outside a rendering build: nothing in the simulation reads it, no
    /// component carries it, and `server::reference_grid` — the only reader —
    /// is registered solely under `SimPluginOptions::render`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference_grid: Option<crate::reference_grid::ReferenceGridConfig>,
    /// Civilian traffic (issue #1028). Present for a hull that flies an authored
    /// `[[route]]` and can be given `hold` / `divert` / `dock` orders. Absent for
    /// everything else, which behaves exactly as it did before this section
    /// existed — a warship is not traffic.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub civilian: Option<crate::civilian::CivilianConfig>,
    /// Optional faction UUID this entity belongs to.
    #[serde(default)]
    pub faction: Option<Uuid>,
    /// Optional AI behaviour controller config.
    #[serde(default)]
    pub behaviour: Option<BehaviourConfig>,
    /// Optional AI profile (aggression, sensor range).
    #[serde(default)]
    pub ai_profile: Option<AiProfileConfig>,
    /// Optional high-fidelity bubble ([`crate::ai::server::LodBubble`]).
    #[serde(default)]
    pub lod_bubble: Option<LodBubbleConfig>,
    /// Radar appearance (colour, optional radius) for the helm radar blip.
    #[serde(default)]
    pub radar_appearance: Option<RadarAppearanceConfig>,
    /// Targetability section. When absent the entity is not targetable.
    #[serde(default)]
    pub target: Option<crate::entities::target::TargetSection>,
    /// 3-D mesh definition. When present the entity receives a visual on the viewscreen.
    #[serde(default)]
    pub mesh: Option<MeshConfig>,
    /// Procedural animated star/sun visual.
    #[serde(default)]
    pub star: Option<StarConfig>,
    /// Textured planet visual. Takes precedence over `[mesh]` when both are
    /// present (`[mesh]` stays as a fallback for headless/editor contexts).
    #[serde(default)]
    pub planet: Option<PlanetConfig>,
    /// Ship class identifier (e.g. "battleship", "cruiser"). Sourced from
    /// top-level TOML `class` field.
    #[serde(default)]
    pub class: Option<String>,
    /// Unique hull identifier/registry number (e.g. "NCC-1701"). Sourced from
    /// top-level TOML `hull_id` field.
    #[serde(default)]
    pub hull_id: Option<String>,
    /// Authored power rating for this ship. Sourced from top-level TOML
    /// `power_rating` field.
    #[serde(default)]
    pub power_rating: Option<i32>,
    /// Per-ship CSS theme URL or inline stylesheet. Sourced from top-level
    /// TOML `css` field.
    #[serde(default)]
    pub css: Option<String>,
    /// Authored mass, in the game's own mass unit (issue #1154). Every entity
    /// a crew could act on — a hull, a derelict, a structure — carries a real
    /// weight rather than a zero-weight tow: this is the property mass-driven
    /// mechanics (the tractor/tow helm penalty is the first of them) are a
    /// DETERMINISTIC FUNCTION of, so it has to be authored content rather than
    /// guessed from a hull's other stats, and it has to be the same number on
    /// every host. An entity that authors nothing takes [`default_mass`]
    /// rather than `0.0` — see that function for why, and
    /// [`validate_mass`] for what an authored value is refused for.
    #[serde(default = "default_mass")]
    pub mass: f32,
    /// Renderer light sources attached to this entity.
    #[serde(default)]
    pub light: Vec<LightConfig>,
    /// Ship stations/systems/power_groups block, populated by parsing the same
    /// `[[station]]` / `[[system]]` / `[power_groups.*]` TOML blocks that
    /// the ship entity TOML uses. Every ship-like entity (player + NPCs) reads
    /// its `ShipConfig` from this field via the same code path — no
    /// entity-type-specific branches.
    #[serde(skip)]
    pub ship_config: Option<crate::ship::config::ShipConfig>,
    /// Cinematic camera config. When present the ship supports the
    /// `ViewMode::Cinematic` viewscreen mode with dynamic entity tracking.
    #[serde(default)]
    pub cinematic_camera: Option<CinematicCameraConfig>,
    /// Designer-authored shield arcs (issue #514). Populated from
    /// top-level `[[shield_arc]]` TOML blocks. When non-empty, the parser
    /// auto-synthesises a matching `[[system]]` entry per arc with
    /// `kind = "shield_arc"` and `SystemId("shield-arc-<id>")`. Consumed
    /// by the runtime path (`ShieldSystem::from_arcs`) that spawns the
    /// ship's `ShipShields` component.
    #[serde(skip)]
    pub shield_arcs: Vec<ShieldArcConfig>,
}

impl EntityConfig {
    /// A stationary, ownerless phaser platform. Unlike a behaviour-driven NPC
    /// it has no helm or doctrine, but its AI-only Tactical systems need the
    /// shared combat substrate to acquire and fire.
    pub fn is_static_point_defence(&self) -> bool {
        self.behaviour.is_none()
            && self
                .weapons_console
                .as_ref()
                .is_some_and(|weapons| !weapons.phaser_banks.is_empty())
            && self.ship_config.as_ref().is_some_and(|ship| {
                ship.systems.iter().any(|system| {
                    system.ai_only
                        && system.kind == crate::ship::system_registry::TACTICAL_RADAR_KIND
                }) && ship.systems.iter().any(|system| {
                    system.ai_only && system.kind == crate::ship::system_registry::PHASER_BANK_KIND
                })
            })
    }

    /// Parse and validate an entity TOML in the default AI-declaration mode.
    ///
    /// That mode is [`AiDeclarationMode::DEFAULT`], and since #885b stage 5d it
    /// is `Strict`: an AI-capable fine system that declares neither a policy nor
    /// an explicit idle state is a LOAD ERROR here, on every path at once. The
    /// synthesisers that used to fill the gap are gone, so an undeclared system
    /// would simply never act — see [`crate::entities::ai_declaration_manifest`].
    pub fn from_toml(s: &str) -> Result<Self, toml::de::Error> {
        Self::from_toml_in_mode(s, AiDeclarationMode::DEFAULT)
    }

    /// [`Self::from_toml`] with the AI-declaration mode chosen explicitly.
    ///
    /// The only remaining caller of [`AiDeclarationMode::Lenient`] is a test
    /// fixture that is deliberately NOT a complete hull: a snippet exercising
    /// beam colours, torpedo fields or marker resolution declares no AI and has
    /// no business authoring twenty blocks to say so. Nothing in production
    /// passes it — `ai_declaration_manifest::tests` asserts the default is
    /// `Strict`, and the spawner attaches nothing for an undeclared system
    /// either way.
    pub fn from_toml_in_mode(
        s: &str,
        ai_declarations: AiDeclarationMode,
    ) -> Result<Self, toml::de::Error> {
        let mut value: toml::Value = toml::from_str(s)?;
        // The LOD ladder moved to the model rig sidecar (issue #914). Reject a
        // leftover `[[mesh.lod]]` here, BEFORE `deny_unknown_fields` turns it
        // into a generic "unknown field `lod`" — an author who reads that
        // learns only that the key is gone, not where it went. Checked on the
        // composed document, so a fragment that still carries one is caught
        // too.
        reject_relocated_mesh_lod(&value)?;
        // Extract [[shield_arc]] blocks BEFORE stripping so we can populate
        // `EntityConfig.shield_arcs` and synthesise matching `[[system]]`
        // entries during ship-config parsing.
        let shield_arcs_value = if let Some(table) = value.as_table_mut() {
            table.remove("shield_arc")
        } else {
            None
        };
        let shield_arcs: Vec<ShieldArcConfig> = match shield_arcs_value {
            Some(toml::Value::Array(arr)) => arr
                .into_iter()
                .map(|v| v.try_into::<ShieldArcConfig>())
                .collect::<Result<_, _>>()?,
            Some(other) => {
                return Err(SerdeError::custom(format!(
                    "[[shield_arc]] must be an array of tables, got {other:?}"
                )));
            }
            None => Vec::new(),
        };

        // Extract the ship-config sections BEFORE stripping so we can parse
        // them via ShipConfig::from_toml (the same path ship entity TOMLs use).
        let ship_config_toml = if let Some(table) = value.as_table_mut() {
            let has_station = table.contains_key("station");
            let has_system = table.contains_key("system");
            let has_power_groups = table.contains_key("power_groups");
            let out = if has_station || has_system || has_power_groups {
                let mut ship_table = toml::value::Table::new();
                if let Some(v) = table.get("station").cloned() {
                    ship_table.insert("station".to_string(), v);
                }
                if let Some(v) = table.get("system").cloned() {
                    ship_table.insert("system".to_string(), v);
                }
                if let Some(v) = table.get("power_groups").cloned() {
                    ship_table.insert("power_groups".to_string(), v);
                }
                Some(toml::Value::Table(ship_table))
            } else {
                None
            };
            // Now strip so deny_unknown_fields doesn't reject.
            table.remove("stations");
            table.remove("station");
            table.remove("system");
            table.remove("power_groups");
            out
        } else {
            None
        };
        let mut config: EntityConfig = value.try_into()?;
        config.shield_arcs = shield_arcs;
        if let Some(power) = config.power.as_ref() {
            validate_power_config(power).map_err(SerdeError::custom)?;
        }
        if let Some(collider) = config.collider.as_ref() {
            validate_collider_config(collider).map_err(SerdeError::custom)?;
        }
        validate_mass(config.mass).map_err(SerdeError::custom)?;

        // Parse the ship_config sub-block via the shared ShipConfig code path.
        // If the ship declares `[[shield_arc]]` blocks, they auto-generate
        // matching `[[system]]` entries with `kind = "shield_arc"` — appended
        // to whatever the ship's TOML already declared before we re-validate.
        if !config.shield_arcs.is_empty()
            || ship_config_toml.is_some()
            || config.behaviour.is_some()
        {
            let registry = crate::ship::system_registry::SystemKindRegistry::with_core_systems()
                .map_err(|e| {
                    serde::de::Error::custom(format!("system registry init failed: {e:?}"))
                })?;
            let kinds: Vec<&str> = registry.kinds().collect();

            let mut ship_config = if let Some(ship_toml_value) = ship_config_toml {
                let ship_toml_str = toml::to_string(&ship_toml_value).map_err(|e| {
                    serde::de::Error::custom(format!("ship_config re-serialise failed: {e}"))
                })?;
                // Parse only (no validation) so ships that declare stations
                // but no `[[system]]` blocks (relying on `[[shield_arc]]`
                // synthesis to populate systems) don't hit `EmptySystems`
                // during the initial pass. Final validation runs after
                // shield_arc synthesis below.
                toml::from_str::<crate::ship::config::ShipConfig>(&ship_toml_str).map_err(|e| {
                    serde::de::Error::custom(format!("ship_config parse failed: {e}"))
                })?
            } else {
                // No station/system/power_groups TOML at all but we do have
                // shield_arcs — synthesise a minimal ShipConfig so the
                // per-arc systems have a home. This path is used by NPC
                // ships that don't otherwise declare `[[system]]` blocks.
                crate::ship::config::ShipConfig {
                    stations: Vec::new(),
                    systems: Vec::new(),
                    power_groups: std::collections::HashMap::new(),
                    coordination_lag_secs: 2.0,
                }
            };

            // Synthesise `[[system]]` entries from `[[shield_arc]]` blocks.
            //
            // For player ships (has a `shields` station or a system with
            // kind="shields" in `systems`), each arc is owned by that
            // station with `ai_only = false`. For NPC ships (no shields
            // station and no shields system), arcs are ownerless AI-only
            // systems, matching how NPC phaser banks / power reactors work.
            // If a kind="shields" system exists, its station assignment
            // is used (allowing shields to live on e.g. Science).
            let shields_station_id = crate::core::messages::StationId("shields".into());
            let shields_system = ship_config
                .systems
                .iter()
                .find(|s| s.kind == crate::ship::system_registry::SHIELDS_KIND);
            let has_shields_station = shields_system.is_some()
                || ship_config
                    .stations
                    .iter()
                    .any(|s| s.id == shields_station_id);
            let effective_shields_station = shields_system
                .and_then(|s| s.station.clone())
                .unwrap_or(shields_station_id);
            for arc in &config.shield_arcs {
                let sid = crate::ship::system_registry::shield_arc_system_id(&arc.id).ok_or_else(
                    || SerdeError::custom(format!("shield_arc id {:?} is empty", arc.id)),
                )?;
                let mut synthesised_config = toml::value::Table::new();
                synthesised_config.insert(
                    "center_deg".into(),
                    toml::Value::Float(arc.center_deg as f64),
                );
                synthesised_config
                    .insert("width_deg".into(), toml::Value::Float(arc.width_deg as f64));
                if let Some(max_hp) = arc.max_hp {
                    synthesised_config.insert("max_hp".into(), toml::Value::Integer(max_hp as i64));
                }
                if let Some(regen) = arc.regen_per_sec {
                    synthesised_config
                        .insert("regen_per_sec".into(), toml::Value::Float(regen as f64));
                }
                if let Some(offline) = arc.offline_duration {
                    synthesised_config.insert(
                        "offline_duration".into(),
                        toml::Value::Float(offline as f64),
                    );
                }

                ship_config
                    .systems
                    .push(crate::ship::config::SystemInstanceConfig {
                        id: sid,
                        kind: crate::ship::system_registry::SHIELD_ARC_KIND.into(),
                        station: if has_shields_station {
                            Some(effective_shields_station.clone())
                        } else {
                            None
                        },
                        ai_only: !has_shields_station,
                        human_seeking: false,
                        seek_order: Vec::new(),
                        // Shield arcs are governed by the shields group as a
                        // whole through ShieldRegen; they are not an extra
                        // allocatable Operations channel.
                        power_group: None,
                        marker: None,
                        config: Some(toml::Value::Table(synthesised_config)),
                    });
            }

            // Provision a mandatory AI-only Red Alert capability for every
            // behaviour-driven NPC ship (issue #749). Behaviour-driven ships run
            // an `operate_captain_ai` loop that can raise its own Red Alert, but
            // only when the Red Alert system's control source resolves to `Ai` —
            // which requires the system to be *listed* on the ship. Authors who
            // omit an explicit `[[system]] kind = "red_alert"` block (every Harrow
            // and pirate NPC) would otherwise leave it defaulting to `Human`,
            // silently blocking the AI captain from ever going to Red Alert.
            //
            // Ownerless (`station = None`) + `ai_only = true`, mirroring how NPC
            // shield-arc / phaser / reactor systems are synthesised above and
            // satisfying `OwnerlessSystemWithoutAiOnly`. Idempotent: an explicit
            // authored red_alert system (the Alliance ships) is left untouched.
            if config.behaviour.is_some()
                && !ship_config
                    .systems
                    .iter()
                    .any(|s| s.kind == crate::ship::system_registry::RED_ALERT_KIND)
            {
                ship_config
                    .systems
                    .push(crate::ship::config::SystemInstanceConfig {
                        id: crate::core::messages::SystemId(
                            crate::ship::system_registry::RED_ALERT_SYSTEM_ID.into(),
                        ),
                        kind: crate::ship::system_registry::RED_ALERT_KIND.into(),
                        station: None,
                        ai_only: true,
                        human_seeking: false,
                        seek_order: Vec::new(),
                        power_group: None,
                        marker: None,
                        config: None,
                    });
            }

            // Re-run validation after synthesis (catches duplicate SystemIds,
            // dangling rating refs, etc.).
            if !ship_config.systems.is_empty() {
                crate::ship::config::validate(&ship_config, &kinds).map_err(|e| {
                    serde::de::Error::custom(format!(
                        "ship_config revalidate after shield_arc synthesis failed: {e:?}"
                    ))
                })?;
            }

            config.ship_config = Some(ship_config);
        }

        // Validation: region entity with effects but no shape is an error.
        if let Some(ref effects) = config.effects {
            if !effects.is_empty() && config.shape.is_none() {
                return Err(SerdeError::custom(
                    "region entity has effects but no [shape] section",
                ));
            }
        }

        // Validation: an [infrastructure] table has to describe a track that
        // can actually degrade (issue #1025). A ceiling of zero, a threshold
        // authored in points rather than fractions, an inverted hysteresis
        // band, or two capacities sharing an id are all author mistakes whose
        // only other symptom would be a structure that silently never crosses
        // anything.
        if let Some(ref infrastructure) = config.infrastructure {
            infrastructure.validate().map_err(SerdeError::custom)?;
        }

        // Validation: a [scan] table has to describe a fidelity ladder that can
        // actually answer (issue #1032). No bands at all, two bands claiming
        // the same id, ranges that do not strictly increase, an unlabelled band
        // or a reporting step outside (0, 1] are all author mistakes whose only
        // other symptom would be a science console that quietly returns nothing
        // for the rest of the mission.
        if let Some(ref scan) = config.scan {
            scan.validate().map_err(SerdeError::custom)?;
        }

        // Validation: a [tractor] table has to describe a beam that can hold
        // (issue #1156), and it has to be paired with the system that gives it
        // its identity. A zero range or zero minimum power is caught by
        // `TractorConfig::validate`; the pairing is checked here because the
        // coupling terms live in a table and the power group, station and damage
        // entry live on a `[[system]] kind = "tractor"` block — a hull that
        // authored one without the other would carry a control the crew can press
        // that grips nothing, or a system with terms nobody reads.
        if let Some(ref tractor) = config.tractor {
            tractor.validate().map_err(SerdeError::custom)?;
            let system = config
                .ship_config
                .as_ref()
                .and_then(|sc| {
                    sc.systems
                        .iter()
                        .find(|s| s.kind == crate::ship::system_registry::TRACTOR_KIND)
                })
                .ok_or_else(|| {
                    SerdeError::custom(
                        "a [tractor] table needs a matching [[system]] kind = \"tractor\" block \
                         to declare its power group, station and damage entry",
                    )
                })?;
            if system.power_group.is_none() {
                return Err(SerdeError::custom(
                    "the [[system]] kind = \"tractor\" block must declare a power_group — the \
                     tractor's power allocation is what an interruption checks",
                ));
            }
        }

        // Validation: a [held_response] table has to match its own kind (issue
        // #1158). A missing recover_per_sec on an arrest-decline, a zero-length
        // formation bearing, or a per-kind field on the wrong kind are all
        // author mistakes whose only other symptom would be a hold that arrests
        // nothing, or holds a target on top of the operator that grabbed it.
        if let Some(ref held_response) = config.held_response {
            held_response.validate().map_err(SerdeError::custom)?;
        }

        // Validation: a [dock] table has to describe a dock that can mate (issue
        // #1159), and a `kind = "dock"` SYSTEM has to be paired with a [dock]
        // table for its terms. Unlike the tractor, the pairing is one-directional:
        // a [dock] table alone makes a hull DOCKABLE (a passive berth carries its
        // dock markers but no active control), while the system is what makes it
        // an active docker — so a [dock] table without a dock system is allowed,
        // but a dock system without a [dock] table (or a power group) is a control
        // the crew can press that has no terms to run under.
        if let Some(ref dock) = config.dock {
            dock.validate().map_err(SerdeError::custom)?;
        }
        if let Some(system) = config.ship_config.as_ref().and_then(|sc| {
            sc.systems
                .iter()
                .find(|s| s.kind == crate::ship::system_registry::DOCK_KIND)
        }) {
            if config.dock.is_none() {
                return Err(SerdeError::custom(
                    "a [[system]] kind = \"dock\" block needs a matching [dock] table to declare \
                     its range and approach terms",
                ));
            }
            if system.power_group.is_none() {
                return Err(SerdeError::custom(
                    "the [[system]] kind = \"dock\" block must declare a power_group — the dock's \
                     power allocation is what an interruption checks",
                ));
            }
        }

        // Validation: a [repair.external_dispatch] table has to describe a
        // dispatch that can do something (issue #1161). A non-positive reach or
        // repair rate is an author mistake whose only other symptom would be a
        // repair-console control the crew can press that sends a team nowhere or
        // helps nobody.
        if let Some(external) = config
            .repair
            .as_ref()
            .and_then(|rc| rc.external_dispatch.as_ref())
        {
            external.validate().map_err(SerdeError::custom)?;
        }

        // Validation: an [umbilical] table has to describe a flow that can run
        // (issue #1160), and it has to be paired with the system that gives it
        // its identity. A blank capacity, a non-positive rate or a zero minimum
        // power is caught by `UmbilicalConfig::validate`; the pairing is checked
        // here for the tractor's reason — the flow terms live in a table and the
        // power group, station and damage entry live on a `[[system]] kind =
        // "umbilical"` block, so a hull that authored one without the other would
        // carry a control the crew can start that moves nothing, or a system with
        // terms nobody reads.
        if let Some(ref umbilical) = config.umbilical {
            umbilical.validate().map_err(SerdeError::custom)?;
            let system = config
                .ship_config
                .as_ref()
                .and_then(|sc| {
                    sc.systems
                        .iter()
                        .find(|s| s.kind == crate::ship::system_registry::UMBILICAL_KIND)
                })
                .ok_or_else(|| {
                    SerdeError::custom(
                        "an [umbilical] table needs a matching [[system]] kind = \"umbilical\" \
                         block to declare its power group, station and damage entry",
                    )
                })?;
            if system.power_group.is_none() {
                return Err(SerdeError::custom(
                    "the [[system]] kind = \"umbilical\" block must declare a power_group — the \
                     umbilical's power allocation is what an interruption checks",
                ));
            }
        }

        // Validation: a [reference_grid] table has to describe a lattice that
        // can actually be drawn and read. A spacing of zero, a major spacing
        // that is not a whole multiple of the minor one, a fade band wider than
        // the patch it fades, or an over-range colour that would bloom on an
        // HDR viewscreen are all author mistakes whose only other symptom would
        // be a grid that is invisible, doubled, or louder than the ships.
        if let Some(ref reference_grid) = config.reference_grid {
            reference_grid.validate().map_err(SerdeError::custom)?;
        }

        // Validation: a [civilian] table has to name a lane something can fly
        // (issue #1028). An empty route id, a negative priority or a disposition
        // authored with negative delays are all author mistakes whose only other
        // symptom would be traffic that sits still and never answers.
        if let Some(ref civilian) = config.civilian {
            civilian.validate().map_err(SerdeError::custom)?;
        }

        // Validation: a [radar_appearance] table must declare at least one
        // of icon/region_colour. An empty table is always an author mistake
        // (omit the whole section to mean "don't show on radar").
        if let Some(ref ra) = config.radar_appearance {
            if ra.icon.is_none() && ra.region_colour.is_none() {
                return Err(SerdeError::custom(
                    "[radar_appearance] must set icon and/or region_colour",
                ));
            }
        }

        // Validate an authored inline Captain AI policy before world
        // activation (issue #775). The Captain's single AI-capable fine system
        // (Red Alert) drives one output channel with one verb. Structural,
        // expression, channel, verb, and parameter errors are deterministic
        // content errors surfaced through serde so the entity fails to load.
        //
        // Every validator below is the `_for` variant, naming the host whose
        // runtime evaluation the block feeds (issue #891 stage 1): that is what
        // lets a `flag(...)`/`counter(...)` guard be rejected on the sixteen
        // hosts that evaluate with an empty flag chain, instead of parsing,
        // validating, and then reading false for ever. The bare
        // `validate_fine_system_ai_*` entry points are host-less and must NOT be
        // used here — `production_validation_names_its_host` asserts that.
        if let Some(ai) = config.captain_console.as_ref().and_then(|c| c.ai.as_ref()) {
            validate_fine_system_ai_policy_for(
                &ai_hosts::CAPTAIN_RED_ALERT,
                ai,
                &[CAPTAIN_RED_ALERT_CHANNEL],
                &[CAPTAIN_SET_RED_ALERT_VERB],
            )
            .map_err(SerdeError::custom)?;
        }

        // Validate authored inline Engines/Steering AI policies before world
        // activation (issue #779). These are the first continuous fine
        // actuators on the #775 spine: Engines drives the `longitudinal`
        // channel, Steering the `yaw` channel, each with its own single mode
        // verb. Unknown channels/verbs, unparseable guards, and undeclared
        // parameter references fail the entity load here, before any live tick.
        if let Some(hc) = config.helm_console.as_ref() {
            if let Some(ai) = hc.engines_ai.as_ref() {
                validate_fine_system_ai_policy_for(
                    &ai_hosts::HELM_ENGINES,
                    ai,
                    &[HELM_LONGITUDINAL_CHANNEL],
                    &[HELM_ACTUATE_DESIRED_TRAVEL_VERB],
                )
                .map_err(SerdeError::custom)?;
            }
            if let Some(ai) = hc.steering_ai.as_ref() {
                validate_fine_system_ai_policy_for(
                    &ai_hosts::HELM_STEERING,
                    ai,
                    &[HELM_YAW_CHANNEL],
                    HELM_STEERING_VERBS,
                )
                .map_err(SerdeError::custom)?;
            }
            // Secondary helm fine-actuator policies (issue #780): each drives its
            // own single channel with its own single mode verb. Wrong-axis verbs,
            // unknown channels, unparseable guards, and undeclared parameter
            // references fail the entity load here, before any live tick.
            if let Some(ai) = hc.lateral_ai.as_ref() {
                validate_fine_system_ai_policy_for(
                    &ai_hosts::HELM_LATERAL,
                    ai,
                    &[HELM_LATERAL_CHANNEL],
                    &[HELM_ACTUATE_LATERAL_THRUST_VERB],
                )
                .map_err(SerdeError::custom)?;
            }
            if let Some(ai) = hc.vertical_ai.as_ref() {
                validate_fine_system_ai_policy_for(
                    &ai_hosts::HELM_VERTICAL,
                    ai,
                    &[HELM_VERTICAL_CHANNEL],
                    &[HELM_ACTUATE_VERTICAL_THRUST_VERB],
                )
                .map_err(SerdeError::custom)?;
            }
            if let Some(ai) = hc.impulse_ai.as_ref() {
                validate_fine_system_ai_policy_for(
                    &ai_hosts::HELM_IMPULSE,
                    ai,
                    &[HELM_IMPULSE_CHANNEL],
                    &[HELM_ENGAGE_IMPULSE_VERB],
                )
                .map_err(SerdeError::custom)?;
            }
            if let Some(ai) = hc.boost_ai.as_ref() {
                validate_fine_system_ai_policy_for(
                    &ai_hosts::HELM_BOOST,
                    ai,
                    &[HELM_BOOST_CHANNEL],
                    &[HELM_ENGAGE_BOOST_VERB],
                )
                .map_err(SerdeError::custom)?;
            }
        }

        // Validate authored inline per-bank weapon AI policies before world
        // activation (issue #781). Each AI-capable phaser and blaster bank may
        // declare an inline `ai` block driving its single `phaser_fire` /
        // `blaster_fire` channel with its single fire verb. Unknown
        // channels/verbs, unparseable guards, and undeclared parameter
        // references fail the entity load here, before any live tick — mirroring
        // the helm validation block above.
        if let Some(wc) = config.weapons_console.as_ref() {
            for bank in &wc.phaser_banks {
                if let Some(ai) = bank.ai.as_ref() {
                    validate_fine_system_ai_policy_for(
                        &ai_hosts::PHASER_BANK,
                        ai,
                        PHASER_BANK_CHANNELS,
                        PHASER_BANK_VERBS,
                    )
                    .map_err(SerdeError::custom)?;
                }
            }
            for bank in &wc.blaster_banks {
                if let Some(ai) = bank.ai.as_ref() {
                    validate_fine_system_ai_policy_for(
                        &ai_hosts::BLASTER_BANK,
                        ai,
                        BLASTER_BANK_CHANNELS,
                        BLASTER_BANK_VERBS,
                    )
                    .map_err(SerdeError::custom)?;
                }
            }
            // The ship-level WEAPONS DOCTRINE (issue #956), validated here with
            // the per-bank policies rather than down among the target selectors:
            // it is a `[weapons_console.ai]` POLICY, and it belongs beside the
            // other weapons-console policies its host resolves alongside. Its
            // channels are the three arc-bearing ranks and its verbs the three
            // weapon families; a rule on an unknown rank, a misspelled family,
            // an unparseable guard or an undeclared `param(...)` fails the
            // entity load here rather than resolving to a silent "no family
            // qualifies" for the rest of the ship's life.
            if let Some(ai) = wc.ai.as_ref() {
                validate_fine_system_ai_policy_for(
                    &ai_hosts::WEAPONS_DOCTRINE,
                    ai,
                    ARC_BEARING_CHANNELS,
                    WEAPONS_DOCTRINE_VERBS,
                )
                .map_err(SerdeError::custom)?;
            }
        }

        // Validate authored inline torpedo tube + magazine AI policies before
        // world activation (issue #782). Each AI-capable tube may declare an
        // inline `ai` block driving its `torpedo_load` / `torpedo_launch`
        // channels; the shared magazine may declare its own `ai` block driving
        // the `torpedo_magazine_grant` channel. Unknown channels/verbs (a launch
        // verb on the magazine channel, a grant verb on a tube), unparseable
        // guards, and undeclared parameter references fail the entity load here,
        // before any live tick — mirroring the weapon-bank validation above.
        if let Some(tc) = config.torpedoes.as_ref() {
            for tube in &tc.tubes {
                if let Some(ai) = tube.ai.as_ref() {
                    validate_fine_system_ai_policy_for(
                        &ai_hosts::TORPEDO_TUBE,
                        ai,
                        TORPEDO_TUBE_CHANNELS,
                        TORPEDO_TUBE_VERBS,
                    )
                    .map_err(SerdeError::custom)?;
                }
            }
            if let Some(ai) = tc.ai.as_ref() {
                validate_fine_system_ai_policy_for(
                    &ai_hosts::TORPEDO_MAGAZINE,
                    ai,
                    TORPEDO_MAGAZINE_CHANNELS,
                    TORPEDO_MAGAZINE_VERBS,
                )
                .map_err(SerdeError::custom)?;
            }
        }

        // Validate an authored inline Shields focus AI policy before world
        // activation (issue #783). The Shields fine system drives its single
        // `shield_focus` channel with its single value-less `focus_shield_arc`
        // verb. A wrong-axis verb (e.g. a fire verb), an unknown channel, an
        // unparseable guard, and undeclared `param(...)` references fail the
        // entity load here, before any live tick — mirroring the helm/weapon
        // validation blocks above.
        if let Some(ai) = config
            .shields_console
            .as_ref()
            .and_then(|sc| sc.ai_policy.as_ref())
        {
            validate_fine_system_ai_policy_for(
                &ai_hosts::SHIELDS_FOCUS,
                ai,
                SHIELD_FOCUS_CHANNELS,
                SHIELD_FOCUS_VERBS,
            )
            .map_err(SerdeError::custom)?;
        }

        // Validate an authored inline Power allocation policy before world
        // activation (issue #784). Unlike every fine system above, the Power
        // reactor has NO fixed channel catalogue: its output channels are the
        // ship's AUTHORED `[power_groups.*]` keys, so the valid-channel set is
        // built dynamically from ship data here (AC1 "no fixed system
        // catalogue"). The single verb is the value-carrying
        // `set_power_group_allocation`. A rule targeting a non-authored group,
        // a wrong verb, an unparseable guard, or an undeclared `param(...)`
        // reserve fails the entity load here, before any live tick.
        if let Some(ai) = config.power.as_ref().and_then(|p| p.ai_policy.as_ref()) {
            // …and where the hull authors NO `[power_groups.*]` at all, the
            // channel set is the canonical trio the runtime seeds it with.
            // `PowerSystem::from_authored_groups` falls back to
            // `seeded_with_defaults` (helm / weapons / sensors at level 2) for
            // an empty authored map, and `ai_power_allocation` then resolves the
            // policy against exactly those groups — so validating against an
            // empty set would reject a policy the runtime is about to run.
            // Nothing shipped hit this until #885b made every hull author
            // `[power.ai_policy]`, including the six NPC hulls that declare no
            // power groups.
            let authored_channels: Vec<&str> = config
                .ship_config
                .as_ref()
                .map(|sc| sc.power_groups.keys().map(|g| g.0.as_str()).collect())
                .unwrap_or_default();
            let valid_channels: Vec<&str> = if authored_channels.is_empty() {
                crate::modifiers::power_system::POWER_GROUP_ORDER.to_vec()
            } else {
                authored_channels
            };
            validate_fine_system_ai_policy_for(
                &ai_hosts::POWER_ALLOCATION,
                ai,
                &valid_channels,
                &[POWER_SET_ALLOCATION_VERB],
            )
            .map_err(SerdeError::custom)?;
        }

        // Validate an authored inline Sensors target selector before world
        // activation (issue #776). Unknown sources, unparseable
        // eligibility/score expressions, and undeclared `param(...)` references
        // are deterministic content errors surfaced through serde so the entity
        // fails to load before any live tick evaluates it.
        if let Some(sel) = config
            .sensors_console
            .as_ref()
            .and_then(|c| c.selector.as_ref())
        {
            validate_fine_system_ai_selector_for(
                &ai_hosts::SENSORS_SELECTOR,
                sel,
                SENSORS_SELECTOR_SOURCES,
            )
            .map_err(SerdeError::custom)?;
        }

        // Validate an authored inline Tactical target selector before world
        // activation (issue #777). Same deterministic content-error surface as
        // the Sensors selector above — the Tactical host is the sole writer of
        // the authoritative weapons target, so a malformed ranking must fail
        // the entity load rather than reach a live tick.
        if let Some(sel) = config
            .weapons_console
            .as_ref()
            .and_then(|c| c.selector.as_ref())
        {
            validate_fine_system_ai_selector_for(
                &ai_hosts::TACTICAL_SELECTOR,
                sel,
                TACTICAL_SELECTOR_SOURCES,
            )
            .map_err(SerdeError::custom)?;
        }

        // Validate an authored inline Navigation target selector before world
        // activation (issue #778). Same deterministic content-error surface as
        // the Sensors/Tactical selectors above — the Navigation host emits the
        // authoritative waypoint through admission from this ranking, so a
        // malformed selector must fail the entity load rather than reach a live
        // tick.
        if let Some(sel) = config
            .navigation_console
            .as_ref()
            .and_then(|c| c.selector.as_ref())
        {
            validate_fine_system_ai_selector_for(
                &ai_hosts::NAVIGATION_SELECTOR,
                sel,
                NAVIGATION_SELECTOR_SOURCES,
            )
            .map_err(SerdeError::custom)?;
        }

        // Validate an authored inline Repair target selector before world
        // activation (issue #785). Same deterministic content-error surface as
        // the Sensors/Tactical/Navigation selectors above — `operate_repair_ai`
        // emits admitted `DispatchRepairTeam` inputs from this ranking, so a
        // malformed selector must fail the entity load rather than reach a live
        // tick. `[repair.selector]` is the first selector block outside a
        // `*_console` section.
        if let Some(sel) = config.repair.as_ref().and_then(|c| c.selector.as_ref()) {
            validate_fine_system_ai_selector_for(
                &ai_hosts::REPAIR_SELECTOR,
                sel,
                REPAIR_SELECTOR_SOURCES,
            )
            .map_err(SerdeError::custom)?;
        }

        // Validate the authored Comms console AI blocks before world activation
        // (issue #786). Comms is the first system to author BOTH machines, so
        // both validators run: the hail SELECTOR against its registered
        // candidate sources, and the dialogue-response POLICY against the single
        // `comms_respond` channel and its `respond_to_message` verb. Both emit
        // admitted commands into the shared comms router, so a malformed block
        // must fail the entity load rather than reach a live tick.
        if let Some(sel) = config
            .comms_console
            .as_ref()
            .and_then(|c| c.selector.as_ref())
        {
            validate_fine_system_ai_selector_for(
                &ai_hosts::COMMS_SELECTOR,
                sel,
                COMMS_SELECTOR_SOURCES,
            )
            .map_err(SerdeError::custom)?;
        }
        if let Some(ai) = config.comms_console.as_ref().and_then(|c| c.ai.as_ref()) {
            validate_fine_system_ai_policy_for(
                &ai_hosts::COMMS_RESPONSE,
                ai,
                COMMS_RESPOND_CHANNELS,
                COMMS_RESPOND_VERBS,
            )
            .map_err(SerdeError::custom)?;
        }

        // Reject a doctrine entry whose `directive_*` fields do not match its
        // own `directive_kind` — see `validate_doctrine_directives`. Runs on the
        // same surface as the selector/policy validators above, so a world
        // `spawn_entity` override that authors a mismatched directive fails at
        // load rather than resolving to a directive that can never fire.
        //
        // ── Overrides merge per-field, so flipping a kind can trip this ──
        //
        // An override reaches this check already merged: `merge_keyed_array`
        // (`src/entities/entity_override.rs`) deep-merges a doctrine override
        // into the template entry that shares its `id`, field by field. Change
        // an existing entry's `directive_kind` and the template's directive
        // fields come along with it — overriding `ship_harrow_patrol.toml`'s
        // Patrol entry with `{ id = "patrol-ironveil", directive_kind = "Reach",
        // directive_anchor = "x" }` yields a merged entry carrying BOTH
        // `directive_anchors` (from the template) and `directive_anchor`, which
        // this check rejects.
        //
        // The escape hatch is to clear the stale field inside the same override
        // entry — `directive_anchors = []`. That still works after issue #911:
        // `behaviour.doctrine.directive_anchors` is in neither identity table,
        // so a nested array inside a reconciled entry keeps replacing wholesale
        // at both merge layers.
        //
        // That matters most on the `spawn_entity` path, where the rejection is
        // NOT fatal: `world::dispatch::dispatch_spawn_entity` warns and keeps
        // the template, so an author who misses this gets the very doctrine they
        // were trying to replace — the silent-wrong-doctrine failure mode #838
        // set out to end. Nothing shipped is in this shape.
        if let Some(ref b) = config.behaviour {
            validate_doctrine_directives(&b.doctrine).map_err(SerdeError::custom)?;
        }

        // Clamp target_speed in every doctrine entry.
        if let Some(ref mut b) = config.behaviour {
            for d in &mut b.doctrine {
                d.target_speed = d.target_speed.clamp(0.0, 1.0);
            }
        }

        // Reject an AI-capable fine system that declares NEITHER a policy nor an
        // explicit idle state (PRD #774 US7), when strict mode is on.
        //
        // Runs last, after every `if let Some(ai) = ...` validator above, and it
        // is deliberately the mirror image of them: those check what an author
        // DID write, this one checks what they did not. Until #885b flips
        // `AiDeclarationMode::DEFAULT` the branch never runs on a shipped path,
        // and the nineteen synthesisers keep filling the gap exactly as before.
        if ai_declarations == AiDeclarationMode::Strict {
            if let Some(err) = crate::entities::ai_declaration_manifest::strict_error(&config) {
                return Err(SerdeError::custom(err));
            }
        }

        Ok(config)
    }

    /// Serialize this config to a `toml::Value` **losslessly** — re-emitting the
    /// `[[station]]` / `[[system]]` / `[power_groups]` and `[[shield_arc]]`
    /// blocks that [`from_toml`](Self::from_toml) assembles into the
    /// `#[serde(skip)]` [`ship_config`](Self::ship_config) /
    /// [`shield_arcs`](Self::shield_arcs) fields.
    ///
    /// # Why this exists (issue #838)
    ///
    /// The override-merge path (`entity_loader::resolve_entity_via` and
    /// `world::dispatch::dispatch_spawn_entity`) resolves a `spawn_entity` /
    /// `[[entity]]` override by round-tripping the template through TOML:
    /// `template → toml::Value → merge(overrides) → EntityConfig::from_toml`.
    /// A plain `toml::to_string(&config)` drops `ship_config` and `shield_arcs`
    /// because both are `#[serde(skip)]` (they have no serialized representation
    /// — `from_toml` reconstructs them from the raw blocks at parse time). The
    /// merged string therefore carried **no ship systems at all**, and the
    /// re-parsed config spawned a hull with zero stations, zero weapons, and
    /// nothing under AI control: a world-spawned "hostile" that could never lock
    /// a target or fire. Re-emitting the blocks here makes the round-trip
    /// faithful, so an override preserves the template's whole system suite.
    ///
    /// Synthesized `shield_arc` systems are filtered out of the emitted `system`
    /// array on purpose: `from_toml` re-synthesizes exactly one per
    /// `[[shield_arc]]` block, and emitting both would trip `DuplicateSystemId`.
    pub fn to_toml_value(&self) -> Result<toml::Value, toml::ser::Error> {
        let mut value = toml::Value::try_from(self)?;
        let table = value
            .as_table_mut()
            .expect("EntityConfig always serializes to a TOML table");

        if let Some(ship_config) = &self.ship_config {
            if !ship_config.stations.is_empty() {
                table.insert(
                    "station".to_string(),
                    toml::Value::try_from(&ship_config.stations)?,
                );
            }
            let declared_systems: Vec<&crate::ship::config::SystemInstanceConfig> = ship_config
                .systems
                .iter()
                .filter(|s| s.kind != crate::ship::system_registry::SHIELD_ARC_KIND)
                .collect();
            if !declared_systems.is_empty() {
                table.insert(
                    "system".to_string(),
                    toml::Value::try_from(&declared_systems)?,
                );
            }
            if !ship_config.power_groups.is_empty() {
                table.insert(
                    "power_groups".to_string(),
                    toml::Value::try_from(&ship_config.power_groups)?,
                );
            }
        }

        if !self.shield_arcs.is_empty() {
            table.insert(
                "shield_arc".to_string(),
                toml::Value::try_from(&self.shield_arcs)?,
            );
        }

        Ok(value)
    }
}

fn validate_power_config(power: &PowerConfigSection) -> Result<(), String> {
    let minimum = power
        .max_commanded_total
        .checked_sub(power.rates.len().saturating_sub(1) as u8)
        .ok_or_else(|| "power.max_commanded_total is too small for its rates ladder".to_string())?;
    if power.sustainable_total < minimum || power.sustainable_total > power.max_commanded_total {
        return Err(format!(
            "power.sustainable_total {} must fall within the rates ladder {}..={}",
            power.sustainable_total, minimum, power.max_commanded_total
        ));
    }
    for (offset, rate) in power.rates.iter().enumerate() {
        let total = minimum + offset as u8;
        if total <= power.sustainable_total && *rate < 0.0 {
            return Err(format!(
                "power rate at total {total} drains below sustainable_total {}",
                power.sustainable_total
            ));
        }
        if total > power.sustainable_total && *rate >= 0.0 {
            return Err(format!(
                "power rate at total {total} must drain above sustainable_total {}",
                power.sustainable_total
            ));
        }
    }
    Ok(())
}

/// Reject a `[collider]` whose numbers cannot describe the shape it names.
///
/// Only [`ColliderShape::Cylinder`] has anything to check, and the check is the
/// one that matters: a cylinder with no `half_height` is a zero-thickness disc
/// that nothing can ever be inside, which is exactly the pass-through bug the
/// station-collider correction was fixing. Serde cannot catch it — the field is
/// optional so that every Ball and Capsule already on disk parses unchanged —
/// so it is caught here, at load, in the same place and the same style as
/// [`validate_power_config`].
///
/// Ball and Capsule are deliberately left alone. Their fields were never
/// validated (a `radius = 0` Ball has always been authorable), and starting now
/// would reject templates that load today for reasons this change has nothing
/// to do with.
fn validate_collider_config(collider: &ColliderConfig) -> Result<(), String> {
    if collider.shape == ColliderShape::Cylinder {
        match collider.half_height {
            None => {
                return Err(
                    "collider.half_height is required for shape = \"Cylinder\" — a cylinder \
                     with no half-height is a zero-thickness disc nothing can collide with"
                        .to_string(),
                )
            }
            // NaN spelled out rather than left to `!(h > 0.0)`: it is the same
            // failure as zero — a body rapier cannot make sense of — and it
            // arrives from the same place, an author typing a number wrong.
            Some(h) if h.is_nan() || h <= 0.0 => {
                return Err(format!(
                    "collider.half_height must be positive for shape = \"Cylinder\", got {h}"
                ))
            }
            Some(_) => {}
        }
    }
    Ok(())
}

/// Documented parse-time default for [`EntityConfig::mass`] (issue #1154): the
/// weight an entity that authors no `mass` is given, rather than `0.0`.
///
/// A zero-mass tow is a physics-defeating exploit dressed as an unauthored
/// field, not an empty one — so the fallback has to be a real weight, and it
/// has to be a NUMBER, chosen once here, rather than a per-caller guess that
/// could disagree with itself between the spawner and a scan readout. `10_000`
/// sits mid-ladder between the lightest shipped hull (a courier) and the
/// heaviest (a battleship), so a template nobody has tuned yet behaves like an
/// ordinary mid-weight hull under a mass-driven mechanic rather than like
/// nothing (too light) or like a battleship (too heavy).
pub const DEFAULT_ENTITY_MASS: f32 = 10_000.0;

fn default_mass() -> f32 {
    DEFAULT_ENTITY_MASS
}

/// Reject an authored `mass` that could never be a real weight.
///
/// Non-positive and non-finite are the same author mistake wearing different
/// faces — a stray `0`, a typo'd negative, a `nan`/`inf` TOML can spell out
/// directly — and every one of them would hand a mass-driven mechanic (the
/// tow/tractor helm penalty divides by this number) either a divide-by-zero or
/// a constant that always wins or always loses, rather than an authored
/// weight. Caught here, at load, in the same style as
/// [`validate_collider_config`], so the failure names the file rather than
/// surfacing as a silent NaN three systems downstream.
fn validate_mass(mass: f32) -> Result<(), String> {
    if !mass.is_finite() || mass <= 0.0 {
        return Err(format!(
            "mass = {mass} is not a valid weight — mass must be a positive, finite number"
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;

// ── Leaf scene-shape types moved from former entities/map_config.rs (PRD #341) ──
// These describe entity-template physical/visual properties consumed by
// EntityConfig (one-per-template) and by steroids::spawner. They are not
// world-tree concerns and so live alongside the entity-template schema rather
// than in world::config.

/// Global configuration block (deterministic seed, lobby metadata, and the
/// shared AI-helm sim-tick rate, all surfaced through WorldConfig).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct GlobalConfig {
    /// Master seed for deterministic generation, feeding `SimRng` (issue
    /// #837). `None` — the key omitted — means "draw one from the OS"; it is
    /// not defaulted to a constant, because a constant here would make the
    /// random tier of the seed-precedence chain unreachable.
    #[serde(default)]
    pub seed: Option<u64>,
    /// Display name shown in the lobby title bar.
    #[serde(default)]
    pub title: Option<String>,
    /// Short description shown below the title in the lobby.
    #[serde(default)]
    pub description: Option<String>,
    /// Fixed rate (Hz) of the LOGICAL SIMULATION tick (issue #895, PRD #849).
    ///
    /// The rate `Time<Fixed>` steps at — every `SimSet` system, in every host
    /// (browser and headless alike), advances on this clock rather than on the
    /// rendered frame. The serde default of 60 Hz matches what the browser
    /// host effectively ran at while the sim was frame-driven, and headless'
    /// `DEFAULT_HZ`, so an unauthored world does not change pace.
    ///
    /// Must be a whole multiple of [`Self::ai_tick_hz`]: the AI decision
    /// cadence is derived from this tick by counting (see
    /// [`Self::sim_ticks_per_ai_tick`]), and `world::config::parse_world`
    /// rejects a non-commensurate pair the same way it rejects a bad
    /// `ai_tick_hz / ai_snapshot_hz` ratio.
    ///
    /// Must also be at least [`MIN_SIM_TICK_HZ`] (30 Hz), which `parse_world`
    /// enforces the same way. Below that floor the helm integrator's
    /// `HELM_AI_MAX_DT_SECS` cap would silently shorten every step: the ship
    /// under-integrates, and two hosts on different authored rates diverge.
    /// A slower rate is a content error at load, not a quiet loss of fidelity.
    ///
    /// Must be at most [`MAX_SIM_TICK_HZ`] (240 Hz), which `parse_world` also
    /// enforces. Above that ceiling the number of `FixedUpdate` steps a
    /// single lagged frame can unpack into (`Time<Virtual>::max_delta` /
    /// timestep) grows large enough to wedge the host — a faster rate is a
    /// content error at load, not a quiet performance cliff.
    #[serde(default = "default_sim_tick_hz")]
    pub sim_tick_hz: f32,
    /// Fixed rate (Hz) of the ONE shared AI decision tick (issue #889).
    ///
    /// Gates every AI policy host — the six per-axis helm systems, the seven
    /// weapons/shields/power deciders, and (through the derived slower cadence
    /// below) Captain and Sensors — decoupling AI decision cadence from the
    /// host's frame rate (issues #803, #889; PRD #620). The default matches the
    /// old `AiLateralThrustTimer` period.
    ///
    /// Authored as `ai_tick_hz`. The pre-#889 key `ai_helm_tick_hz` remains a
    /// serde alias — every shipped world TOML authors it — because the rate was
    /// never helm-specific in anything but name.
    #[serde(default = "default_ai_tick_hz", alias = "ai_helm_tick_hz")]
    pub ai_tick_hz: f32,
    /// Rate (Hz) of the DERIVED slower AI cadence: the `WorldSnapshot` /
    /// doctrine-blackboard rebuild and the two policy hosts that read them
    /// (Captain, Sensors).
    ///
    /// Before #889 this was a hardcoded 10 Hz `Timer` in `ai/server.rs` — a
    /// designer-tunable decision rate living as a Rust literal, and a second AI
    /// clock free to drift out of phase with the base one. It is now expressed
    /// as authored data and realised as an integer multiple of [`Self::ai_tick_hz`]
    /// (see [`Self::snapshot_every_ticks`]); a non-integer relationship between
    /// the two is rejected by `world::config::parse_world`.
    #[serde(default = "default_ai_snapshot_hz")]
    pub ai_snapshot_hz: f32,
    /// Hull fraction at or below which a seat's intent narration announces
    /// that the ship is breaking off (issue #879).
    ///
    /// Authored so the "we are pulling out" advisory fires where a designer
    /// says it should rather than at a Rust literal. This is the NARRATION
    /// threshold — what the crew is told — and is deliberately independent of
    /// the authored helm doctrine that decides whether the ship actually
    /// disengages; that lives in the movement fragments as a policy guard.
    #[serde(default = "default_intent_break_off_hull_fraction")]
    pub intent_break_off_hull_fraction: f32,
    /// How long (simulation seconds) a landed hit — shields or hull — keeps a
    /// ship's doctrine `attacked` condition true (issue #1010).
    ///
    /// `attacked` used to read the `LastShipAttacker` latch, which clears only
    /// on death or when the ship's red alert stands down. A hull's captain
    /// stand-down does release it (`combat_window_secs`, 10 s on every Harrow),
    /// but not while the fight continues — the captain's `secs_since_combat`
    /// fact counts the hull's own return fire, so a Harrow shooting back holds
    /// its own alert up and a `not_attacked`-gated raid stayed retired for as
    /// long as anything loitered nearby. This window governs the doctrine gate
    /// DIRECTLY: a raid yields to self-defence while the ship is being hit and
    /// resumes after a reprieve of this length, whatever the alert posture is
    /// doing. Authored so a designer can say how long a hull holds a grudge
    /// without a recompile — see `objectives::attacked_recently`.
    #[serde(default = "default_attacked_memory_secs")]
    pub attacked_memory_secs: f32,
    /// How long (simulation seconds) one station-activity debug bucket spans
    /// (issue #1145, PRD #1144).
    ///
    /// The always-on station-activity counters tally admitted commands per
    /// station per this time chunk, split by control source. Authored so a
    /// crew-control designer can widen or narrow the chart's resolution without a
    /// recompile — the serde default is the only sanctioned hardcoded copy of it
    /// (AGENTS.md #11), and it IS the shipped tuning: no world TOML authors the
    /// key. Converted to an integer tick count at `sim_tick_hz` by
    /// `debug::station_activity::StationActivityTracker::configure`.
    #[serde(default = "default_station_activity_bucket_secs")]
    pub station_activity_bucket_secs: f32,
    /// How many recent fires the trigger-fire-history debug recorder keeps per
    /// trigger (issue #1151, PRD #1144).
    ///
    /// When the scenario-state debug surface is on, a bounded ring records each
    /// trigger's recent fires with the predicate values observed at each, so an
    /// author can reconstruct why a beat fired early, late, or not at all. This
    /// is the ring depth — the bound that keeps a session that runs for hours
    /// from leaking. Read-only diagnostic capture into a `Presentation`-class
    /// resource, so it never moves the #894 digest whatever the depth. The serde
    /// default is the only sanctioned hardcoded copy (AGENTS.md #11); no world
    /// TOML authors the key, it IS the shipped tuning.
    #[serde(default = "default_trigger_fire_history_depth")]
    pub trigger_fire_history_depth: u32,
}

/// Serde default for [`GlobalConfig::intent_break_off_hull_fraction`]: half
/// hull. The only sanctioned hardcoded gameplay value is a TOML-parse fallback
/// (AGENTS.md #11).
fn default_intent_break_off_hull_fraction() -> f32 {
    0.5
}

/// Serde default for [`GlobalConfig::attacked_memory_secs`]: eight seconds.
/// The only sanctioned hardcoded gameplay value is a TOML-parse fallback
/// (AGENTS.md #11).
///
/// `[ai]` The eight-second figure is AI-origin tuning, chosen as long enough
/// that a hull under sustained fire never flickers back to its raid between
/// volleys and short enough that a single stray hit costs the raid seconds
/// rather than the run. The marker that RATIFICATION reads is the `[ai] `
/// rationale bullet on `objective-doctrine-score-policy` in
/// `pasm/spec/architecture/objectives.yaml` (AGENTS.md "AI-origin decisions");
/// this note is the pointer to it, not the record itself.
fn default_attacked_memory_secs() -> f32 {
    8.0
}

/// Serde default for [`GlobalConfig::station_activity_bucket_secs`]: fifteen
/// seconds (issue #1145). The only sanctioned hardcoded copy of the shipped
/// bucket length (AGENTS.md #11) — a TOML-parse fallback.
fn default_station_activity_bucket_secs() -> f32 {
    15.0
}

/// Serde default for [`GlobalConfig::trigger_fire_history_depth`]: sixteen fires
/// per trigger (issue #1151). The only sanctioned hardcoded copy of the shipped
/// ring depth (AGENTS.md #11) — a TOML-parse fallback. Sixteen is deep enough to
/// show a repeat trigger's recent rhythm and shallow enough to stay a "few
/// records per trigger" bound.
fn default_trigger_fire_history_depth() -> u32 {
    16
}

impl Default for GlobalConfig {
    fn default() -> Self {
        Self {
            seed: None,
            title: None,
            description: None,
            sim_tick_hz: default_sim_tick_hz(),
            ai_tick_hz: default_ai_tick_hz(),
            ai_snapshot_hz: default_ai_snapshot_hz(),
            intent_break_off_hull_fraction: default_intent_break_off_hull_fraction(),
            attacked_memory_secs: default_attacked_memory_secs(),
            station_activity_bucket_secs: default_station_activity_bucket_secs(),
            trigger_fire_history_depth: default_trigger_fire_history_depth(),
        }
    }
}

impl GlobalConfig {
    /// The number of base AI ticks per slower snapshot tick.
    ///
    /// `None` when the authored pair is not a positive integer relationship —
    /// e.g. `ai_tick_hz = 25` against `ai_snapshot_hz = 10` gives 2.5, which is
    /// a content error rather than something to silently round. Callers on the
    /// hot path use [`Self::snapshot_every_ticks`], which is only reachable
    /// after `parse_world` has rejected that case.
    pub fn checked_snapshot_every_ticks(&self) -> Option<u32> {
        if !(self.ai_tick_hz.is_finite() && self.ai_tick_hz > 0.0) {
            return None;
        }
        if !(self.ai_snapshot_hz.is_finite() && self.ai_snapshot_hz > 0.0) {
            return None;
        }
        let ratio = self.ai_tick_hz / self.ai_snapshot_hz;
        let rounded = ratio.round();
        if rounded < 1.0 || (ratio - rounded).abs() > SNAPSHOT_RATIO_EPSILON {
            return None;
        }
        Some(rounded as u32)
    }

    /// [`Self::checked_snapshot_every_ticks`] with the parse-time default
    /// applied, for the run-condition system that cannot return an error.
    pub fn snapshot_every_ticks(&self) -> u32 {
        self.checked_snapshot_every_ticks()
            .unwrap_or_else(|| (default_ai_tick_hz() / default_ai_snapshot_hz()).round() as u32)
    }

    /// The number of logical sim ticks per shared AI decision tick (issue
    /// #895): the AI cadence is derived from [`SimTick`](crate::sim_tick::SimTick)
    /// by counting, so `sim_tick_hz / ai_tick_hz` must be a positive integer.
    ///
    /// `None` when it is not — a content error `parse_world` rejects, exactly
    /// like [`Self::checked_snapshot_every_ticks`].
    pub fn checked_sim_ticks_per_ai_tick(&self) -> Option<u32> {
        if !(self.sim_tick_hz.is_finite() && self.sim_tick_hz > 0.0) {
            return None;
        }
        if !(self.ai_tick_hz.is_finite() && self.ai_tick_hz > 0.0) {
            return None;
        }
        let ratio = self.sim_tick_hz / self.ai_tick_hz;
        let rounded = ratio.round();
        if rounded < 1.0 || (ratio - rounded).abs() > SNAPSHOT_RATIO_EPSILON {
            return None;
        }
        Some(rounded as u32)
    }

    /// [`Self::checked_sim_ticks_per_ai_tick`] with the parse-time default
    /// applied, for the cadence system that cannot return an error.
    pub fn sim_ticks_per_ai_tick(&self) -> u32 {
        self.checked_sim_ticks_per_ai_tick()
            .unwrap_or_else(|| (default_sim_tick_hz() / default_ai_tick_hz()).round() as u32)
    }
}

/// Tolerance on the `ai_tick_hz / ai_snapshot_hz` ratio. Both are authored as
/// `f32`, so an exactly-integer relationship such as 30/10 can land a few ULPs
/// off; 2.5 is nowhere near this band.
const SNAPSHOT_RATIO_EPSILON: f32 = 1e-4;

/// Floor on the authored [`GlobalConfig::sim_tick_hz`], enforced by
/// `world::config::parse_world` (issue #895).
///
/// Derived from — not merely matching — the helm integrator's
/// `HELM_AI_MAX_DT_SECS` cap: a sim tick longer than that cap would be
/// silently shortened by it, so the sim would under-integrate and two hosts
/// on different rates would produce different trajectories from the same
/// commands. Keeping the floor tied to the constant means the two can never
/// drift apart.
pub const MIN_SIM_TICK_HZ: f32 = 1.0 / crate::ship::components::HELM_AI_MAX_DT_SECS;

/// Ceiling on the authored [`GlobalConfig::sim_tick_hz`], enforced by
/// `world::config::parse_world` (re-review of issue #895 — the floor above
/// had no matching upper bound).
///
/// `Time<Virtual>`'s `max_delta` (250 ms) bounds how much wall-clock lag a
/// single rendered frame can absorb, but the NUMBER of `FixedUpdate` steps
/// that lag unpacks into is `max_delta / timestep`: an unbounded rate lets a
/// fat-fingered or hostile TOML (`sim_tick_hz = 100000`) demand ~25 000
/// fixed steps back-to-back inside one frame, starving everything else on
/// the host thread and making the browser or headless runner appear to
/// hang. 240 Hz keeps that worst case to 60 steps — generous headroom above
/// the shipped 60 Hz default and well past the fastest cadence any current
/// design work asks for — while still catching authored rates nobody could
/// mean.
pub const MAX_SIM_TICK_HZ: f32 = 240.0;

fn default_sim_tick_hz() -> f32 {
    60.0
}

fn default_ai_tick_hz() -> f32 {
    30.0
}

fn default_ai_snapshot_hz() -> f32 {
    10.0
}

/// Configuration for the grid-based asteroid spawner.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct GridConfig {
    pub resolution: f32,
    #[serde(default = "default_fill_gameplay")]
    pub fill_gameplay: f32,
    #[serde(default = "default_fill_cosmetic")]
    pub fill_cosmetic: f32,
    #[serde(default)]
    pub uniformity: f32,
    #[serde(default = "default_noise_freq")]
    pub noise_freq: f32,
    #[serde(default = "default_noise_octaves")]
    pub noise_octaves: u32,
    #[serde(default = "default_density_noise_freq")]
    pub density_noise_freq: f32,
    #[serde(default = "default_density_noise_octaves")]
    pub density_noise_octaves: u32,
    #[serde(default)]
    pub jitter: f32,
    #[serde(default)]
    pub cosmetic_y_offset: f32,
    #[serde(default = "default_gameplay_y_variance")]
    pub gameplay_y_variance: f32,
    #[serde(default = "default_spawn_cells")]
    pub spawn_cells: u32,
    #[serde(default = "default_despawn_cells")]
    pub despawn_cells: u32,
}

fn default_fill_gameplay() -> f32 {
    0.4
}
fn default_fill_cosmetic() -> f32 {
    0.15
}
fn default_noise_freq() -> f32 {
    0.02
}
fn default_noise_octaves() -> u32 {
    3
}
fn default_density_noise_freq() -> f32 {
    0.01
}
fn default_density_noise_octaves() -> u32 {
    2
}
fn default_gameplay_y_variance() -> f32 {
    0.5
}
fn default_spawn_cells() -> u32 {
    10
}
fn default_despawn_cells() -> u32 {
    12
}

/// Shape variant for an asteroid field.
///
/// When the TOML schema omits `shape`, the field defaults to the historical
/// behaviour (cell-centre distance check against `inner_radius`/`outer_radius`,
/// which produces a disc/annulus depending on whether `inner_radius` is zero).
///
/// `Torus` selects the explicit annulus eligibility test: a cell is admitted
/// if its XZ bounding box overlaps the annulus `[inner_radius, outer_radius]`
/// around the world origin. Cells whose bounding box lies fully inside
/// `inner_radius` or whose nearest corner is beyond `outer_radius` are
/// rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AsteroidFieldShape {
    /// Annulus / ring-belt eligibility based on `inner_radius` and
    /// `outer_radius`. Cells whose bounding box overlaps the annulus
    /// are admitted.
    Torus,
}

/// One authored asteroid type in a field's type list, with its relative
/// rarity weight (issue #946).
///
/// Two spellings, both valid TOML in the same array:
///
/// ```toml
/// asteroid_type_paths = [
///     "assets/entities/asteroid_common_1_small.toml",
///     { path = "assets/entities/asteroid_rare_1_small.toml", weight = 0.01 },
/// ]
/// ```
///
/// The bare-string form is the pre-#946 schema and still parses — it means
/// exactly `weight = 1.0` — so every field TOML written before rarity
/// existed keeps working untouched.
///
/// Weights are **relative within one list**, not probabilities: an entry at
/// `0.1` is drawn a tenth as often as an entry at `1.0` beside it. Nothing
/// in Rust knows what "common", "uncommon" or "rare" mean; the rarity tiers
/// are entirely a property of the numbers a designer authors here, so a new
/// tier needs no code change. Non-positive weights clamp to zero, and a list
/// whose weights are *all* zero falls back to a uniform draw rather than
/// erasing the field (the same degenerate-authoring guard the field-level
/// `weight` uses).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum AsteroidTypeRef {
    /// `"assets/entities/foo.toml"` — an unweighted entry, i.e. weight 1.0.
    Path(String),
    /// `{ path = "assets/entities/foo.toml", weight = 0.1 }`.
    Weighted {
        path: String,
        #[serde(default = "default_asteroid_type_weight")]
        weight: f32,
    },
}

impl AsteroidTypeRef {
    /// The entity template this entry points at.
    pub fn path(&self) -> &str {
        match self {
            Self::Path(path) => path,
            Self::Weighted { path, .. } => path,
        }
    }

    /// The authored rarity weight; `1.0` for the bare-string form.
    pub fn weight(&self) -> f32 {
        match self {
            Self::Path(_) => default_asteroid_type_weight(),
            Self::Weighted { weight, .. } => *weight,
        }
    }
}

impl From<&str> for AsteroidTypeRef {
    fn from(path: &str) -> Self {
        Self::Path(path.to_string())
    }
}

impl From<String> for AsteroidTypeRef {
    fn from(path: String) -> Self {
        Self::Path(path)
    }
}

fn default_asteroid_type_weight() -> f32 {
    1.0
}

/// Configuration for an asteroid field.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct AsteroidFieldConfig {
    pub inner_radius: f32,
    pub outer_radius: f32,
    pub density: f32,
    /// Relative weight of this field's contribution to the world's composed
    /// density evaluator (#913). Every authored asteroid-field entity feeds
    /// one shared evaluator; where several fields cover the same lattice
    /// cell their densities and fill thresholds blend proportionally to
    /// weight, and the spawned rock's tuning (type lists, jitter, rotation,
    /// shield pierce) comes from one contributing field picked by the same
    /// weights. `1.0` (the default) makes all fields equal partners. `0.0`
    /// removes a field's influence wherever a positively-weighted field
    /// also covers the cell; if every covering field is zero-weighted the
    /// blend falls back to uniform so a field can never author itself into
    /// a divide-by-zero.
    #[serde(default = "default_field_weight")]
    pub weight: f32,
    #[serde(default = "default_spawn_distance")]
    pub spawn_distance: f32,
    #[serde(default = "default_despawn_distance")]
    pub despawn_distance: f32,
    /// Gameplay (targetable, hulled) asteroid types this field may spawn,
    /// each with its relative rarity weight. See [`AsteroidTypeRef`].
    #[serde(default)]
    pub asteroid_type_paths: Vec<AsteroidTypeRef>,
    /// Backdrop asteroid types for the two cosmetic layers, same shape.
    #[serde(default)]
    pub cosmetic_type_paths: Vec<AsteroidTypeRef>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub grid: Option<GridConfig>,
    /// Fraction of asteroid-collision damage that bypasses shields and
    /// applies directly to the hull. Default `0.0` — asteroid impacts are
    /// fully absorbed by the facing shield quadrant (matching pre-#414
    /// behaviour). Clamped to `[0.0, 1.0]` at apply time.
    #[serde(default)]
    pub shield_pierce: f32,
    /// Optional shape variant. When `None`, the historical cell-centre
    /// distance eligibility test is used. When `Some(Torus)`, cells are
    /// admitted iff their XZ bounding box overlaps the annulus
    /// `[inner_radius, outer_radius]`. See [`AsteroidFieldShape`].
    #[serde(default)]
    pub shape: Option<AsteroidFieldShape>,
    /// Optional world anchor name. When present, the field's eligibility
    /// region and per-asteroid spawn positions are translated so the
    /// `[inner_radius, outer_radius]` annulus is centred on the named
    /// anchor's world position instead of the world origin. The anchor
    /// is resolved against `WorldConfig.anchors` at spawn time; the
    /// resolved offset is written into `anchor_offset`. If the anchor
    /// name is not present in the world's anchor table, the field falls
    /// back to the world origin (`anchor_offset = [0, 0, 0]`) and a
    /// warning is logged.
    #[serde(default)]
    pub anchor: Option<String>,
    /// Resolved world-space offset for the anchor referenced by `anchor`.
    /// Defaults to `[0, 0, 0]` (world origin) when no anchor is set or
    /// the named anchor could not be resolved. Not serialised — derived
    /// at spawn time.
    #[serde(skip)]
    pub anchor_offset: [f32; 3],
    /// Maximum random rotation applied to each spawned asteroid, in degrees.
    /// `[x, y, z]` → ±x° pitch, ±y° roll, ±z° yaw. When `None` (default),
    /// asteroids spawn with no rotation. Set e.g. `[30, 30, 180]` for mild
    /// tilt with full spin freedom.
    #[serde(default)]
    pub random_rotation: Option<[f32; 3]>,
}

fn default_field_weight() -> f32 {
    1.0
}

fn default_spawn_distance() -> f32 {
    150.0
}
fn default_despawn_distance() -> f32 {
    250.0
}
