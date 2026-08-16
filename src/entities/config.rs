use crate::entities::ai_declaration_manifest::AiDeclarationMode;
use crate::entities::ai_flag_hosts as ai_hosts;
use crate::region_effects::RegionEffectsConfig;
use crate::region_shape::RegionShape;
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
            Some(other) => {
                return Err(format!(
                    "doctrine '{}': unknown directive_kind '{other}'; \
                     valid: Patrol, Destroy, Reach, Retreat, Hail, Dock",
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
/// is itself always full-fidelity. See [`crate::ai_plugin::LodBubble`]. A player
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
/// [`crate::model_rig::ModelRig::lod`]; this section only names the model. The
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
/// ([`crate::model_rig::ModelRig::lod`]).
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
    /// How this level's `billboard` atlas was captured. Authored as a
    /// `[lod.capture]` sub-table. Build-time provenance only — see
    /// [`LodCapture`]; the renderer never reads it.
    #[serde(default)]
    pub capture: Option<LodCapture>,
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
            crate::model_rig::sidecar_path(model, mesh.get("variant").and_then(|v| v.as_str()))
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
    pub system_id: crate::messages::SystemId,
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
    crate::impulse::IMPULSE_CHARGE_DURATION
}

fn default_impulse_speed_multiplier() -> f32 {
    crate::impulse::IMPULSE_SPEED_MULTIPLIER
}

fn default_impulse_engage_distance() -> f32 {
    200.0
}

fn default_impulse_cancel_distance() -> f32 {
    40.0
}

fn default_impulse_acceleration_multiplier() -> f32 {
    crate::impulse::IMPULSE_ACCELERATION_MULTIPLIER
}

fn default_boost_steering_multiplier() -> f32 {
    crate::boost::BOOST_STEERING_MULTIPLIER
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
    /// Convert this TOML config into a runtime `crate::blaster::BlasterBankConfig`.
    pub fn to_runtime(&self) -> crate::blaster::BlasterBankConfig {
        crate::blaster::BlasterBankConfig {
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

/// Authored policy-parameter name: the forward-thrust level (0.0–1.0) above
/// which the default helm-elevation rule considers the ship actively driving.
pub const POWER_THRUST_THRESHOLD_PARAM: &str = "thrust_threshold";
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
/// Authored policy-parameter name: the (zero) reserve the default LOWERING
/// baseline rules reference so every rule declares a reserve (AC2) without ever
/// gating a de-allocation, which can never cause a brownout.
pub const POWER_BASELINE_RESERVE_PARAM: &str = "min_reserve_baseline";

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
/// the second [`crate::messages::RepairTarget`] variant. Surfaced as a candidate
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
/// These map 1:1 onto `crate::shield::ShieldConfig` (the runtime struct
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
    pub fn to_runtime(&self) -> crate::shield::ShieldConfig {
        crate::shield::ShieldConfig {
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
/// runtime conversion consumed by [`crate::shield::ShieldSystem::from_arcs`].
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
    /// [`crate::shield::ShieldSystem::from_arcs`].
    pub fn to_runtime(&self) -> crate::shield::ArcRuntimeConfig {
        crate::shield::ArcRuntimeConfig {
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
    pub fn to_runtime(&self) -> crate::repair_teams::RepairTimings {
        crate::repair_teams::RepairTimings {
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
    pub fn to_runtime(&self) -> crate::torpedo::TorpedoConfig {
        crate::torpedo::TorpedoConfig {
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
    pub name: Option<String>,
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
    /// External operations this hull can perform (issue #1026). Present on the
    /// hulls a scenario expects to stabilise, tow or escort with; absent for
    /// everything else, which can start no operation and is refused by name if
    /// asked to. The mirror image of `infrastructure`: that table says what can
    /// be done *to* an entity, this one says what an entity can do.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operations: Option<crate::operations::OperationsConfig>,
    /// The sensor suite's scan capability (issue #1032). Present on a hull whose
    /// science station can take a reading of an external structure; absent for
    /// everything else, which can scan nothing and is refused by name if asked.
    /// The mirror image of `infrastructure` in the other direction from
    /// `operations`: that table says what can be *done to* an entity, this one
    /// says what an entity can *read*.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scan: Option<crate::science::ScanConfig>,
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
    /// Optional high-fidelity bubble ([`crate::ai_plugin::LodBubble`]).
    #[serde(default)]
    pub lod_bubble: Option<LodBubbleConfig>,
    /// Radar appearance (colour, optional radius) for the helm radar blip.
    #[serde(default)]
    pub radar_appearance: Option<RadarAppearanceConfig>,
    /// Targetability section. When absent the entity is not targetable.
    #[serde(default)]
    pub target: Option<crate::entity_target::TargetSection>,
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
                    system.ai_only && system.kind == crate::system_registry::TACTICAL_RADAR_KIND
                }) && ship.systems.iter().any(|system| {
                    system.ai_only && system.kind == crate::system_registry::PHASER_BANK_KIND
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
            let shields_station_id = crate::messages::StationId("shields".into());
            let shields_system = ship_config
                .systems
                .iter()
                .find(|s| s.kind == crate::system_registry::SHIELDS_KIND);
            let has_shields_station = shields_system.is_some()
                || ship_config
                    .stations
                    .iter()
                    .any(|s| s.id == shields_station_id);
            let effective_shields_station = shields_system
                .and_then(|s| s.station.clone())
                .unwrap_or(shields_station_id);
            for arc in &config.shield_arcs {
                let sid =
                    crate::system_registry::shield_arc_system_id(&arc.id).ok_or_else(|| {
                        SerdeError::custom(format!("shield_arc id {:?} is empty", arc.id))
                    })?;
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
                        kind: crate::system_registry::SHIELD_ARC_KIND.into(),
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
                    .any(|s| s.kind == crate::system_registry::RED_ALERT_KIND)
            {
                ship_config
                    .systems
                    .push(crate::ship::config::SystemInstanceConfig {
                        id: crate::messages::SystemId(
                            crate::system_registry::RED_ALERT_SYSTEM_ID.into(),
                        ),
                        kind: crate::system_registry::RED_ALERT_KIND.into(),
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

        // Validation: an [operations] table has to describe an operation that
        // can actually run (issue #1026). A zero range, a zero duration, a
        // nameless power group or two blocks claiming the same verb are all
        // author mistakes whose only other symptom would be a capability the
        // crew can start and never finish.
        if let Some(ref operations) = config.operations {
            operations.validate().map_err(SerdeError::custom)?;
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
                .filter(|s| s.kind != crate::system_registry::SHIELD_ARC_KIND)
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

#[cfg(test)]
mod tests {
    #![allow(clippy::field_reassign_with_default)]

    use super::*;
    use crate::entity_tags::EntityTag;
    use crate::simmath;

    /// One shipped hull through the REAL load path — include resolution and all.
    ///
    /// Since issue #878 every Harrow hull is COMPOSED: its movement doctrine and
    /// its ship-level declarations arrive from `assets/entities/fragments/ai/`,
    /// so the authored file alone is not the document the game spawns. An
    /// `include_str!` here would assert on unresolved text and pass while the
    /// resolved hull said something else entirely;
    /// `include_resolve::tests::shipped_tree::include_str_baked_hulls_are_all_uncomposed`
    /// is the guard that names any site which forgets.
    /// The resolved document as TEXT: every assertion below parses it exactly as
    /// the loader does, and the tests that strike a line out of it to prove the
    /// load fails without that line find the line wherever it is now authored —
    /// hull or fragment.
    fn resolved_text(stem: &str) -> String {
        crate::entity_includes::resolve_from_disk(&format!("assets/entities/{stem}.toml"))
            .unwrap_or_else(|e| panic!("{stem} must resolve: {e}"))
            .toml
    }

    fn harrow_destroyer_toml() -> String {
        resolved_text("ship_harrow_destroyer")
    }

    fn harrow_cruiser_toml() -> String {
        resolved_text("ship_harrow_cruiser")
    }

    fn harrow_warhawk_toml() -> String {
        resolved_text("ship_harrow_warhawk")
    }

    #[test]
    fn all_sections_present_deserializes_to_some() {
        let toml_str = r##"
tags = ["gameplay", "combat", "primary"]

[hull]
hull_integrity = 100

[collider]
shape = "Ball"
radius = 2.0
length = 0.0

[appearance]
colour = "#ff0000"
size_min = 1.0
size_max = 3.0

[helm_console]
max_speed = 50.0
max_reverse_speed = 25.0
acceleration = 16.7
deceleration = 50.0
max_yaw_rate = 0.785

[helm_console.radar]
range = 50.0
shows = ["asteroid"]

[weapons_console]

[engineering_console]

[captain_console]
"##;
        // Lenient: this fixture is about which SECTIONS deserialize to `Some`,
        // and its bare `[weapons_console]` owes a `weapons_doctrine`
        // declaration under the default strict mode (issue #956 — the kind
        // gates on the console, not on `[behaviour]`).
        let config = EntityConfig::from_toml_in_mode(
            toml_str,
            crate::entities::ai_declaration_manifest::AiDeclarationMode::Lenient,
        )
        .expect("parse must succeed");

        assert_eq!(
            config.tags,
            vec![
                "gameplay".to_string(),
                "combat".to_string(),
                "primary".to_string()
            ]
        );

        assert!(config.hull.is_some());
        assert!((config.hull.as_ref().unwrap().hull_integrity - 100.0).abs() < 1e-6);

        assert!(config.collider.is_some());
        let c = config.collider.as_ref().unwrap();
        assert_eq!(c.shape, ColliderShape::Ball);
        assert_eq!(c.radius, 2.0);

        assert!(config.appearance.is_some());
        assert_eq!(config.appearance.as_ref().unwrap().colour, "#ff0000");

        assert!(config.helm_console.is_some());
        let h = config.helm_console.as_ref().unwrap();
        assert_eq!(h.max_speed, 50.0);
        assert_eq!(h.effective_radar_range(), 50.0);

        assert!(config.weapons_console.is_some());

        assert!(config.engineering_console.is_some());

        assert!(config.captain_console.is_some());
    }

    /// The red-alert hostile weapon-arc overlay colour (issue #874) is authored
    /// per hull, not inlined in the client — AGENTS.md #11.
    #[test]
    fn helm_console_parses_the_hostile_arc_color() {
        let config = EntityConfig::from_toml(
            r##"
[helm_console]
max_speed = 50.0
hostile_arc_color = [ 1, 0.3, 0.3, 0.07 ]
"##,
        )
        .expect("parse must succeed");
        assert_eq!(
            config.helm_console.as_ref().unwrap().hostile_arc_color,
            vec![1.0, 0.3, 0.3, 0.07]
        );
    }

    /// A hull that omits it keeps the wire default rather than failing to parse.
    #[test]
    fn helm_console_hostile_arc_color_is_optional() {
        let config = EntityConfig::from_toml("[helm_console]\nmax_speed = 50.0\n")
            .expect("parse must succeed");
        assert!(config
            .helm_console
            .as_ref()
            .unwrap()
            .hostile_arc_color
            .is_empty());
    }

    /// Every hull that renders `ph-helm-radar` must author the colour, or the
    /// overlay silently falls back to a value no designer chose.
    #[test]
    fn the_player_hulls_author_a_hostile_arc_color() {
        for path in [
            "assets/entities/alliance_battleship.toml",
            "assets/entities/alliance_cruiser.toml",
            "assets/entities/alliance_destroyer.toml",
        ] {
            // Through the include resolver (issue #906) so a composed hull is
            // judged on its resolved document — a raw read would assert on the
            // unresolved text and silently stop covering the hull.
            let config =
                crate::entity_includes::load_entity_config(path).expect("hull TOML must parse");
            let color = &config
                .helm_console
                .as_ref()
                .expect("hull declares [helm_console]")
                .hostile_arc_color;
            assert_eq!(color.len(), 4, "{path} must author an RGBA quad: {color:?}");
            assert!(
                color[3] < 0.25,
                "{path}: the overlay must stay FAINTER than the Tactical radar's \
                 own arc fills (0.30 / 0.25); got alpha {}",
                color[3]
            );
        }
    }

    #[test]
    fn helm_console_engine_pfx_deserializes_optional_block() {
        let toml_str = r##"
[helm_console]
max_speed = 50.0

[helm_console.engine_pfx]
color = [0.2, 0.7, 1.0, 0.8]
markers = ["engine_port", "engine_starboard"]
roll_degrees = 20.0
scale = 1.25
trail_lifetime_secs = 0.45
trail_spawn_interval_secs = 0.04
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let pfx = config
            .helm_console
            .as_ref()
            .and_then(|helm| helm.engine_pfx.as_ref())
            .expect("engine_pfx block must parse");

        assert_eq!(pfx.color, Some([0.2, 0.7, 1.0, 0.8]));
        assert_eq!(
            pfx.markers,
            vec!["engine_port".to_string(), "engine_starboard".to_string()]
        );
        assert_eq!(pfx.roll_degrees, Some(20.0));
        assert_eq!(pfx.scale, Some(1.25));
        assert_eq!(pfx.trail_lifetime_secs, Some(0.45));
        assert_eq!(pfx.trail_spawn_interval_secs, Some(0.04));
    }

    #[test]
    fn helm_console_engine_pfx_fields_default_when_block_is_sparse() {
        let toml_str = r##"
[helm_console]
max_speed = 50.0

[helm_console.engine_pfx]
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let pfx = config
            .helm_console
            .as_ref()
            .and_then(|helm| helm.engine_pfx.as_ref())
            .expect("engine_pfx block must parse");

        assert_eq!(pfx.color, None);
        assert!(pfx.markers.is_empty());
        assert_eq!(pfx.roll_degrees, None);
        assert_eq!(pfx.scale, None);
        assert_eq!(pfx.trail_lifetime_secs, None);
        assert_eq!(pfx.trail_spawn_interval_secs, None);
    }

    #[test]
    fn only_hull_and_tags_produces_none_for_console_fields() {
        let toml_str = r##"
tags = ["gameplay", "asteroid"]

[hull]
hull_integrity = 80
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");

        assert_eq!(
            config.tags,
            vec!["gameplay".to_string(), "asteroid".to_string()]
        );
        assert!(config.hull.is_some());
        assert!((config.hull.as_ref().unwrap().hull_integrity - 80.0).abs() < 1e-6);
        assert!(config.collider.is_none());
        assert!(config.appearance.is_none());
        assert!(config.helm_console.is_none());
        assert!(config.weapons_console.is_none());
        assert!(config.engineering_console.is_none());
        assert!(config.captain_console.is_none());
        assert!(
            config.radar_appearance.is_none(),
            "radar_appearance should default to None when not in TOML"
        );
    }

    #[test]
    fn malformed_field_returns_error() {
        let toml_str = r##"
[hull]
hull_integrity = "not_an_integer"
"##;
        let result = EntityConfig::from_toml(toml_str);
        assert!(result.is_err());
    }

    #[test]
    fn tags_field_deserializes_to_vec_string() {
        let toml_str = r##"
tags = ["foo", "bar", "baz", "quux"]
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        assert_eq!(
            config.tags,
            vec![
                "foo".to_string(),
                "bar".to_string(),
                "baz".to_string(),
                "quux".to_string()
            ]
        );
    }

    #[test]
    fn collider_capsule_shape_round_trips() {
        let toml_str = r##"
[collider]
shape = "Capsule"
radius = 1.5
length = 6.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        assert_eq!(
            config.collider.as_ref().unwrap().shape,
            ColliderShape::Capsule
        );
    }

    #[test]
    fn collider_cylinder_shape_round_trips() {
        let toml_str = r##"
[collider]
shape = "Cylinder"
radius = 17.04
half_height = 7.16
length = 0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let collider = config.collider.as_ref().unwrap();
        assert_eq!(collider.shape, ColliderShape::Cylinder);
        assert_eq!(collider.radius, 17.04);
        assert_eq!(collider.half_height, Some(7.16));
    }

    /// `half_height` is optional in the serde shape so that every Ball and
    /// Capsule already on disk parses untouched — which means serde cannot be
    /// the thing that catches a Cylinder without one.
    ///
    /// It has to be caught SOMEWHERE, because the failure is silent and it is
    /// the exact bug the station-collider work was fixing: a cylinder of zero
    /// half-height is a body with no interior, so ships fly through a structure
    /// they can see, and nothing anywhere says why.
    #[test]
    fn a_cylinder_without_a_half_height_is_a_load_error() {
        let err = EntityConfig::from_toml(
            r##"
[collider]
shape = "Cylinder"
radius = 17.04
length = 0
"##,
        )
        .expect_err("a Cylinder with no half_height must not load");
        assert!(
            err.to_string().contains("half_height"),
            "the error must name the missing field, got: {err}"
        );
    }

    /// Zero and negative are the same failure as absent — a disc with no
    /// thickness — and are rejected for the same reason.
    #[test]
    fn a_cylinder_with_a_non_positive_half_height_is_a_load_error() {
        for bad in ["0", "0.0", "-7.16"] {
            let toml_str = format!(
                r##"
[collider]
shape = "Cylinder"
radius = 17.04
half_height = {bad}
length = 0
"##
            );
            let err = EntityConfig::from_toml(&toml_str)
                .err()
                .unwrap_or_else(|| panic!("half_height = {bad} must not load"));
            assert!(
                err.to_string().contains("half_height"),
                "the error must name the offending field, got: {err}"
            );
        }
    }

    /// The other two shapes are untouched by the new field: neither reads it,
    /// and neither is required to author it. A Ball that omits `half_height`
    /// (which is every Ball and Capsule template in `assets/entities/`) must go
    /// on loading exactly as it did.
    #[test]
    fn ball_and_capsule_do_not_require_a_half_height() {
        for shape in ["Ball", "Capsule"] {
            let toml_str = format!(
                r##"
[collider]
shape = "{shape}"
radius = 1.5
length = 4.0
"##
            );
            let config = EntityConfig::from_toml(&toml_str)
                .unwrap_or_else(|e| panic!("a {shape} with no half_height must parse: {e}"));
            assert_eq!(config.collider.as_ref().unwrap().half_height, None);
        }
    }

    /// Issue #958: `[collider] movable` is the authored dynamic/static split the
    /// hazard rule reads. A template that omits it is TERRAIN — the safe
    /// direction, since terrain is never dropped by the ignore-smaller rule.
    #[test]
    fn collider_movable_defaults_to_static_terrain() {
        let unauthored = EntityConfig::from_toml(
            r##"
[collider]
shape = "Ball"
radius = 12.0
length = 0.0
"##,
        )
        .expect("parse must succeed");
        assert!(
            !unauthored.collider.as_ref().unwrap().movable,
            "an unauthored collider must default to static terrain"
        );

        let authored = EntityConfig::from_toml(
            r##"
[collider]
shape = "Capsule"
radius = 1.5
length = 4.0
movable = true
"##,
        )
        .expect("parse must succeed");
        assert!(
            authored.collider.as_ref().unwrap().movable,
            "`movable = true` must parse into a mobile contact"
        );
    }

    /// Issue #958: shipped authoring, not just the parser, and a walk rather
    /// than a list so a NEW template cannot quietly land on the wrong side.
    ///
    /// A template that declares a helm capability is a hull somebody flies, so
    /// it must author `movable = true` and take its chances with a bigger hull's
    /// `hazard_ignore_size_ratio`. Everything else with a collider is terrain —
    /// station, planet, moon, star, asteroid — and must stay static, so it is
    /// avoided at any relative size.
    ///
    /// The walk is RECURSIVE, mirroring `spawnable_templates_under` in
    /// `src/headless/app.rs`, which issue #954 made recursive for the same
    /// reason: that issue filed a spawned hull under
    /// `assets/entities/test/rng_coverage_lancer.toml`, and
    /// `assets/worlds/rng_coverage.toml` fields it twice. A top-level
    /// `read_dir` would leave that hull — and anything else a later issue files
    /// in a subdirectory — outside a guard whose whole purpose is to catch the
    /// template nobody remembered to author.
    ///
    /// `fragments/` is the one exclusion, and it is excluded for a property of
    /// its contents rather than of its name: nothing in it is spawnable. A
    /// fragment is a partial document that hulls compose FROM, so it is never
    /// itself a body publishing a hazard, and `composed_escort.toml` is a
    /// mechanism fixture rather than shipped content. That does leave the
    /// ship-shaped `npc_escort_core.toml` unguarded by construction: it authors
    /// `movable = true` because anything composing from it is by construction a
    /// hull, but that authoring is a convention this test cannot hold.
    #[test]
    fn shipped_hulls_are_mobile_and_shipped_terrain_is_not() {
        /// Every `.toml` under `dir` except the fragment tree, sorted so the
        /// failure a designer sees is the same on every filesystem.
        fn templates_under(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            let entries = std::fs::read_dir(dir)
                .unwrap_or_else(|e| panic!("{} must be readable: {e}", dir.display()));
            let mut paths: Vec<std::path::PathBuf> = entries.flatten().map(|e| e.path()).collect();
            paths.sort();
            for path in paths {
                if path.is_dir() {
                    if path.file_name().is_some_and(|n| n == "fragments") {
                        continue;
                    }
                    templates_under(&path, out);
                } else if path.extension().and_then(|e| e.to_str()) == Some("toml") {
                    out.push(path);
                }
            }
        }

        let dir = std::path::Path::new("assets/entities");
        let mut templates = Vec::new();
        templates_under(dir, &mut templates);
        assert!(
            !templates.is_empty(),
            "no templates found under {}",
            dir.display()
        );

        let (mut hulls, mut terrain) = (0, 0);
        for path in templates {
            let key = path.to_string_lossy().replace('\\', "/");
            let cfg = crate::entity_includes::load_entity_config(&key)
                .unwrap_or_else(|e| panic!("{key} must parse: {e}"));
            let Some(collider) = cfg.collider.as_ref() else {
                continue;
            };
            if cfg.helm_capability.is_some() || cfg.helm_console.is_some() {
                assert!(
                    collider.movable,
                    "{key} declares a helm capability, so it is a flyable hull \
                     and must author `[collider] movable = true`"
                );
                hulls += 1;
            } else {
                assert!(
                    !collider.movable,
                    "{key} has no helm capability, so it is static terrain and \
                     must never claim `[collider] movable = true`"
                );
                terrain += 1;
            }
        }
        assert!(hulls > 0, "no flyable hulls found in {}", dir.display());
        assert!(terrain > 0, "no static terrain found in {}", dir.display());
    }

    #[test]
    fn empty_toml_string_produces_all_none() {
        let config = EntityConfig::from_toml("").expect("parse must succeed");
        assert!(config.tags.is_empty());
        assert!(config.hull.is_none());
        assert!(config.collider.is_none());
        assert!(config.appearance.is_none());
        assert!(config.helm_console.is_none());
        assert!(config.weapons_console.is_none());
        assert!(config.engineering_console.is_none());
        assert!(config.captain_console.is_none());
        assert!(
            config.radar_appearance.is_none(),
            "radar_appearance should default to None"
        );
    }

    #[test]
    fn helm_console_partial_fields_work() {
        let toml_str = r##"
[helm_console]
max_speed = 30.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let h = config.helm_console.expect("helm_console must be Some");
        assert_eq!(h.max_speed, 30.0);
        assert_eq!(h.max_reverse_speed, 0.0);
    }

    #[test]
    fn helm_console_radar_table_parses_into_nested_field() {
        let toml_str = r##"
[helm_console]
max_speed = 30.0

[helm_console.radar]
range = 750.0
shows = ["asteroid", "ship"]
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let h = config.helm_console.expect("helm_console must be Some");
        let radar = h.radar.as_ref().expect("helm_console.radar must parse");
        assert_eq!(radar.range, 750.0);
        assert_eq!(h.effective_radar_range(), 750.0);
    }

    #[test]
    fn helm_console_effective_radar_range_zero_when_no_radar_table() {
        let toml_str = r##"
[helm_console]
max_speed = 30.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let h = config.helm_console.expect("helm_console must be Some");
        assert!(h.radar.is_none());
        assert_eq!(h.effective_radar_range(), 0.0);
    }

    #[test]
    fn helm_console_boost_table_parses_when_present() {
        let toml_str = r##"
[helm_console]
max_speed = 30.0

[helm_console.boost]
multiplier = 3.0
steering_multiplier = 2.0
active_duration = 4.0
recharge_duration = 20.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let h = config.helm_console.expect("helm_console must be Some");
        let boost = h.boost.as_ref().expect("helm_console.boost must parse");
        assert_eq!(boost.multiplier, 3.0);
        assert_eq!(boost.steering_multiplier, 2.0);
        assert_eq!(boost.active_duration, 4.0);
        assert_eq!(boost.recharge_duration, 20.0);
    }

    #[test]
    fn helm_console_boost_steering_multiplier_defaults_to_identity() {
        let toml_str = r##"
[helm_console]
max_speed = 30.0

[helm_console.boost]
multiplier = 3.0
active_duration = 4.0
recharge_duration = 20.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let h = config.helm_console.expect("helm_console must be Some");
        let boost = h.boost.as_ref().expect("helm_console.boost must parse");
        assert_eq!(
            boost.steering_multiplier,
            crate::boost::BOOST_STEERING_MULTIPLIER
        );
    }

    #[test]
    fn helm_console_boost_none_when_table_absent() {
        let toml_str = r##"
[helm_console]
max_speed = 30.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let h = config.helm_console.expect("helm_console must be Some");
        assert!(
            h.boost.is_none(),
            "missing boost table must disable the feature"
        );
    }

    #[test]
    fn weapons_console_beam_color_parses_rgba() {
        let toml_str = r##"
[weapons_console]

[[weapons_console.phaser_banks]]
id = "fore"
facing_deg = 0.0
fire_arc_deg = 180.0
auto_arc_deg = 180.0
beam_color = [1.0, 0.5, 0.2, 0.9]
"##;
        let config = EntityConfig::from_toml_in_mode(
            toml_str,
            crate::entities::ai_declaration_manifest::AiDeclarationMode::Lenient,
        )
        .expect("parse must succeed");
        let w = config
            .weapons_console
            .expect("weapons_console must be Some");
        assert_eq!(w.phaser_banks[0].beam_color, vec![1.0, 0.5, 0.2, 0.9]);
    }

    #[test]
    fn weapons_console_beam_color_defaults_to_empty_when_omitted() {
        let toml_str = r##"
[weapons_console]

[[weapons_console.phaser_banks]]
id = "fore"
facing_deg = 0.0
fire_arc_deg = 180.0
auto_arc_deg = 180.0
"##;
        let config = EntityConfig::from_toml_in_mode(
            toml_str,
            crate::entities::ai_declaration_manifest::AiDeclarationMode::Lenient,
        )
        .expect("parse must succeed");
        let w = config
            .weapons_console
            .expect("weapons_console must be Some");
        assert!(
            w.phaser_banks[0].beam_color.is_empty(),
            "beam_color should default to empty vec when omitted"
        );
    }

    // ── Power section tests ────────────────────────────────────────────────

    #[test]
    fn power_section_parses_capacity_rates_emergency_threshold() {
        let toml_str = r##"
[power]
capacity = 150.0
rates = [10.0, 8.0, 6.0, 4.0, -4.0, -10.0]
emergency_threshold = 30.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let p = config.power.expect("power must be Some");
        assert!((p.capacity - 150.0).abs() < 0.001);
        assert_eq!(p.rates, [10.0, 8.0, 6.0, 4.0, -4.0, -10.0]);
        assert!((p.emergency_threshold - 30.0).abs() < 0.001);
    }

    #[test]
    fn power_section_omitted_when_not_in_toml() {
        let config = EntityConfig::from_toml("").expect("parse must succeed");
        assert!(
            config.power.is_none(),
            "power should be None when not specified"
        );
    }

    #[test]
    fn alliance_hulls_author_the_six_plus_two_reactor_budget() {
        for path in [
            "assets/entities/alliance_courier.toml",
            "assets/entities/alliance_destroyer.toml",
            "assets/entities/alliance_cruiser.toml",
            "assets/entities/alliance_battleship.toml",
        ] {
            let config = crate::entity_includes::load_entity_config(path)
                .unwrap_or_else(|error| panic!("{path}: {error}"));
            let power = config.power.expect("Alliance hull authors [power]");
            assert_eq!(power.sustainable_total, 6, "{path}");
            assert_eq!(power.max_commanded_total, 8, "{path}");
            let minimum = power.max_commanded_total - (power.rates.len() as u8 - 1);
            for (offset, rate) in power.rates.into_iter().enumerate() {
                let total = minimum + offset as u8;
                assert_eq!(rate < 0.0, total > 6, "{path}: total {total}");
            }
        }
    }

    #[test]
    fn sensors_console_parses_with_long_range_radar() {
        let toml_str = r##"
tags = ["player", "ship"]

[sensors_console.long_range_radar]
range = 200.0
shows = ["region", "asteroid_field", "asteroid", "ship"]
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let sensors = config
            .sensors_console
            .expect("sensors_console must be Some");
        assert_eq!(sensors.long_range_radar.range, 200.0);
        assert!(sensors.long_range_radar.shows.contains(&EntityTag::Region));
        assert!(sensors
            .long_range_radar
            .shows
            .contains(&EntityTag::AsteroidField));
        assert!(sensors
            .long_range_radar
            .shows
            .contains(&EntityTag::Asteroid));
    }

    #[test]
    fn sensors_console_omitted_when_not_in_toml() {
        let config = EntityConfig::from_toml("").expect("parse must succeed");
        assert!(config.sensors_console.is_none());
    }

    /// `power_multipliers` lives on `[shields_console]` since issue #952 moved
    /// the third power group from `sensors` to `shields`. Replaces the
    /// `[sensors_console]` half of `sensors_console_parses_with_long_range_radar`,
    /// whose assertion was that a curve authored there was READ — which is no
    /// longer true of any curve on that console, because `RadarRange` has no
    /// power producer left to read it.
    #[test]
    fn shields_console_power_multipliers_parses() {
        let toml_str = r##"
[shields_console]
power_multipliers = [0.0, 0.25, 0.5, 1.0]
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let shields = config
            .shields_console
            .expect("shields_console must be Some");
        assert_eq!(shields.power_multipliers, Some([0.0, 0.25, 0.5, 1.0]));
    }

    /// A curve on `[sensors_console]` is now an unknown field, and the section
    /// is `deny_unknown_fields`, so an author who leaves one behind is told at
    /// load rather than watching it silently do nothing.
    #[test]
    fn sensors_console_power_multipliers_is_rejected() {
        let err = EntityConfig::from_toml(
            "[sensors_console]\npower_multipliers = [-0.5, 0.0, 0.25, 0.5]\n",
        )
        .expect_err("the field moved to [shields_console] in #952")
        .to_string();
        assert!(err.contains("power_multipliers"), "got: {err}");
    }

    #[test]
    fn helm_console_power_multipliers_parses() {
        let toml_str = r##"
[helm_console]
power_multipliers = [-0.8, 0.0, 0.4, 0.8]
max_speed = 50.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let h = config.helm_console.expect("helm_console must be Some");
        assert_eq!(h.power_multipliers, Some([-0.8, 0.0, 0.4, 0.8]));
    }

    /// Lenient, like the other `[weapons_console]` schema fixtures around it:
    /// since issue #956 a weapons console owes a `weapons_doctrine` declaration
    /// (the kind gates on the CONSOLE, not on `[behaviour]`), and this fixture
    /// is about one power curve rather than about AI authoring.
    #[test]
    fn weapons_console_power_multipliers_parses() {
        let toml_str = r##"
[weapons_console]
power_multipliers = [-0.3, 0.0, 0.15, 0.3]
"##;
        let config = EntityConfig::from_toml_in_mode(
            toml_str,
            crate::entities::ai_declaration_manifest::AiDeclarationMode::Lenient,
        )
        .expect("parse must succeed");
        let w = config
            .weapons_console
            .expect("weapons_console must be Some");
        assert_eq!(w.power_multipliers, Some([-0.3, 0.0, 0.15, 0.3]));
    }

    #[test]
    fn power_multipliers_defaults_to_none_when_omitted() {
        let toml_str = r##"
[helm_console]
max_speed = 30.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let h = config.helm_console.expect("helm_console must be Some");
        assert!(h.power_multipliers.is_none());
    }

    /// Every shipped entity template must parse under the strict
    /// (`deny_unknown_fields`) schema. Catches both schema drift in the code
    /// and typo'd keys in the TOMLs.
    #[test]
    fn all_shipped_entity_templates_parse_strictly() {
        let dir = std::path::Path::new("assets/entities");
        let mut checked = 0;
        for entry in std::fs::read_dir(dir).expect("assets/entities must exist") {
            let path = entry.expect("dir entry").path();
            if path.extension().and_then(|e| e.to_str()) != Some("toml") {
                continue;
            }
            // Resolved first (issue #906) — the strict schema applies to the
            // COMPOSED document, which is the only thing that ever reaches
            // `EntityConfig`.
            let key = path.to_string_lossy().replace('\\', "/");
            crate::entity_includes::load_entity_config(&key).unwrap_or_else(|e| {
                panic!(
                    "entity template {} failed strict parse: {e}",
                    path.display()
                )
            });
            checked += 1;
        }
        assert!(
            checked > 0,
            "no entity templates found in {}",
            dir.display()
        );
    }

    /// Unknown keys in an entity TOML must be rejected, not silently ignored.
    #[test]
    fn unknown_section_and_field_are_rejected() {
        assert!(EntityConfig::from_toml("[helm_consol]\nmax_speed = 1.0").is_err());
        assert!(EntityConfig::from_toml("[helm_console]\nmax_sped = 1.0").is_err());
    }

    // ── AsteroidField section tests ────────────────────────────────────────

    #[test]
    fn asteroid_field_section_parses_from_template() {
        let toml_str = r##"
tags = ["field", "main"]

[asteroid_field]
inner_radius = 100.0
outer_radius = 200.0
density = 0.005
spawn_distance = 150.0
despawn_distance = 250.0
asteroid_type_paths = ["assets/entities/asteroid_small.toml", "assets/entities/asteroid_large.toml"]
cosmetic_type_paths = ["assets/entities/asteroid_cosmetic.toml"]
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let field = config.asteroid_field.expect("asteroid_field must be Some");
        assert!((field.inner_radius - 100.0).abs() < 1e-6);
        assert_eq!(field.asteroid_type_paths.len(), 2);
        assert_eq!(field.cosmetic_type_paths.len(), 1);
    }

    #[test]
    fn asteroid_field_section_omitted_when_not_in_toml() {
        let config = EntityConfig::from_toml("").expect("parse must succeed");
        assert!(config.asteroid_field.is_none());
    }

    #[test]
    fn asteroid_field_shape_defaults_to_none_when_omitted() {
        // Back-compat: TOMLs that pre-date the `shape` field must continue
        // to deserialise unchanged, with `shape = None`.
        let toml_str = r##"
[asteroid_field]
inner_radius = 100.0
outer_radius = 200.0
density = 0.005
asteroid_type_paths = ["x.toml"]
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let field = config.asteroid_field.expect("asteroid_field must be Some");
        assert!(field.shape.is_none());
    }

    #[test]
    fn asteroid_field_anchor_parses_as_optional_string() {
        // PRD #397 fix 5: `[asteroid_field] anchor = "name"` carries the
        // reference verbatim. The serde-skipped `anchor_offset` defaults
        // to `[0,0,0]` and is filled in at spawn time against the world's
        // anchor table.
        let toml_str = r##"
[asteroid_field]
shape = "torus"
anchor = "belt_origin"
inner_radius = 300.0
outer_radius = 350.0
density = 0.005
asteroid_type_paths = ["x.toml"]
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let field = config.asteroid_field.expect("asteroid_field must be Some");
        assert_eq!(field.anchor.as_deref(), Some("belt_origin"));
        assert_eq!(
            field.anchor_offset,
            [0.0, 0.0, 0.0],
            "anchor_offset is serde-skipped and defaults to origin until spawn-time resolution"
        );
    }

    #[test]
    fn asteroid_field_anchor_omitted_defaults_to_none() {
        // Regression guard: existing TOML without an `anchor` key must keep
        // `anchor = None` and `anchor_offset = [0,0,0]` (legacy behaviour).
        let toml_str = r##"
[asteroid_field]
inner_radius = 100.0
outer_radius = 200.0
density = 0.005
asteroid_type_paths = ["x.toml"]
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let field = config.asteroid_field.expect("asteroid_field must be Some");
        assert!(field.anchor.is_none(), "missing anchor key → None");
        assert_eq!(field.anchor_offset, [0.0, 0.0, 0.0]);
    }

    #[test]
    fn asteroid_field_shape_torus_parses() {
        // Schema: `shape = "torus"` as a sibling of `inner_radius`/`outer_radius`.
        let toml_str = r##"
[asteroid_field]
shape = "torus"
inner_radius = 300.0
outer_radius = 350.0
density = 0.005
asteroid_type_paths = ["x.toml"]
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let field = config.asteroid_field.expect("asteroid_field must be Some");
        assert_eq!(
            field.shape,
            Some(crate::entity_config::AsteroidFieldShape::Torus)
        );
        assert!((field.inner_radius - 300.0).abs() < 1e-6);
        assert!((field.outer_radius - 350.0).abs() < 1e-6);
    }

    #[test]
    fn asteroid_field_shape_unknown_value_errors() {
        let toml_str = r##"
[asteroid_field]
shape = "donut"
inner_radius = 100.0
outer_radius = 200.0
density = 0.005
"##;
        let result = EntityConfig::from_toml(toml_str);
        assert!(
            result.is_err(),
            "unknown shape variant must be a parse error"
        );
    }

    // ── name / mesh.emissive / [[light]] tests (PRD: schema refactor slice 3) ──

    #[test]
    fn name_field_parses() {
        let toml_str = r#"name = "Sun""#;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        assert_eq!(config.name.as_deref(), Some("Sun"));
    }

    #[test]
    fn name_field_defaults_to_none() {
        let config = EntityConfig::from_toml("").expect("parse must succeed");
        assert!(config.name.is_none());
    }

    #[test]
    fn mesh_emissive_field_parses() {
        let toml_str = r##"
[mesh]
shape = "sphere"
colour = [1.0, 0.8, 0.0]
radius = 50.0
emissive = 2.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let mesh = config.mesh.expect("mesh must be Some");
        assert_eq!(mesh.emissive, Some(2.0));
    }

    #[test]
    fn mesh_emissive_defaults_to_none() {
        let toml_str = r##"
[mesh]
shape = "sphere"
colour = [1.0, 1.0, 1.0]
radius = 1.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let mesh = config.mesh.expect("mesh must be Some");
        assert!(mesh.emissive.is_none());
    }

    #[test]
    fn light_array_parses_multiple_entries() {
        let toml_str = r##"
[[light]]
kind = "point"
colour = [1.0, 0.95, 0.85]
intensity = 150000.0
range = 5000.0

[[light]]
kind = "point"
colour = [0.5, 0.5, 1.0]
intensity = 1000.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        assert_eq!(config.light.len(), 2);
        assert_eq!(config.light[0].kind, LightKind::Point);
        assert_eq!(config.light[0].colour, [1.0, 0.95, 0.85]);
        assert!((config.light[0].intensity - 150000.0).abs() < 1e-3);
        assert_eq!(config.light[0].range, Some(5000.0));
        assert_eq!(config.light[1].range, None);
    }

    #[test]
    fn light_defaults_to_empty_vec() {
        let config = EntityConfig::from_toml("").expect("parse must succeed");
        assert!(config.light.is_empty());
    }

    #[test]
    fn light_directional_kind_parses() {
        let toml_str = r##"
[[light]]
kind = "directional"
colour = [1.0, 1.0, 1.0]
intensity = 10000.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        assert_eq!(config.light.len(), 1);
        assert_eq!(config.light[0].kind, LightKind::Directional);
    }

    // ── Region shape tests ───────────────────────────────────────────────

    #[test]
    fn region_shape_sphere_parses_from_toml() {
        let toml_str = r##"
tags = ["region", "test"]

[shape]
type = "sphere"
radius = 100.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let shape = config.shape.expect("shape must be Some");
        assert_eq!(
            shape,
            crate::region_shape::RegionShape::Sphere { radius: 100.0 }
        );
    }

    #[test]
    fn region_shape_box_parses_from_toml() {
        let toml_str = r##"
tags = ["region", "test"]

[shape]
type = "box"
half_extents = [50.0, 30.0, 40.0]
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let shape = config.shape.expect("shape must be Some");
        assert_eq!(
            shape,
            crate::region_shape::RegionShape::Box {
                half_extents: [50.0, 30.0, 40.0],
                yaw: 0.0
            }
        );
    }

    #[test]
    fn region_shape_torus_parses_from_toml() {
        let toml_str = r##"
tags = ["region", "test"]

[shape]
type = "torus"
inner_radius = 50.0
outer_radius = 80.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let shape = config.shape.expect("shape must be Some");
        assert_eq!(
            shape,
            crate::region_shape::RegionShape::Torus {
                inner_radius: 50.0,
                outer_radius: 80.0
            }
        );
    }

    #[test]
    fn region_shape_parses_with_effects() {
        let toml_str = r##"
tags = ["region", "nebula"]

[shape]
type = "sphere"
radius = 150.0

[effects]
[effects.comms_jammed]
[effects.sensor_blind]
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        assert!(config.shape.is_some());
        let effects = config.effects.expect("effects must be Some");
        assert!(effects.comms_jammed.is_some());
        assert!(effects.sensor_blind.is_some());
    }

    #[test]
    fn region_effects_without_shape_returns_error() {
        let toml_str = r##"
tags = ["region"]

[effects]
[effects.comms_jammed]
"##;
        let result = EntityConfig::from_toml(toml_str);
        assert!(
            result.is_err(),
            "region entity with effects but no shape should error"
        );
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("shape"),
            "error should mention missing shape: {err}"
        );
    }

    #[test]
    fn shape_alone_without_effects_is_valid() {
        let toml_str = r##"
tags = ["region"]

[shape]
type = "sphere"
radius = 100.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        assert!(config.shape.is_some());
        assert!(config.effects.is_none());
    }

    #[test]
    fn empty_toml_produces_no_shape_or_effects() {
        let config = EntityConfig::from_toml("").expect("parse must succeed");
        assert!(config.shape.is_none());
        assert!(config.effects.is_none());
    }

    // ── Ship audio tests ─────────────────────────────────────────────────

    /// `EntityConfig` is `deny_unknown_fields`, so an `[audio]` block in a
    /// shipped template would break *every* load of that template — not just
    /// audio — if the field were ever removed. These parse the real files.
    /// One shipped hull, through the REAL load path (issue #875).
    ///
    /// `include_str!` bakes a template's bytes at compile time, so a baked site
    /// can never see include resolution — and since the player destroyer became
    /// a COMPOSED hull, its baked bytes are no longer the document the game
    /// loads. `include_str_baked_hulls_are_all_uncomposed` is the tripwire that
    /// names such sites; this helper is what they move to.
    fn shipped_hull(stem: &str) -> EntityConfig {
        let path = format!("assets/entities/{stem}.toml");
        crate::entity_includes::load_entity_config(&path)
            .unwrap_or_else(|e| panic!("{stem}.toml must compose and parse: {e}"))
    }

    #[test]
    fn player_ship_templates_parse_audio_block() {
        for name in [
            "alliance_cruiser",
            "alliance_destroyer",
            "alliance_battleship",
        ] {
            let config = shipped_hull(name);
            let audio = config
                .audio
                .as_ref()
                .unwrap_or_else(|| panic!("{name}.toml must have [audio]"));

            assert_eq!(
                audio.ambient.as_ref().expect("[audio.ambient]").file,
                "assets/sounds/Ambient.mp3",
                "{name}"
            );
            assert_eq!(
                audio.engine.as_ref().expect("[audio.engine]").file,
                "assets/sounds/Engine.mp3",
                "{name}"
            );
            assert_eq!(
                audio.blaster.as_ref().expect("[audio.blaster]").file,
                "assets/sounds/Blaster.mp3",
                "{name}"
            );
            assert_eq!(
                audio
                    .phaser_loop
                    .as_ref()
                    .expect("[audio.phaser_loop]")
                    .file,
                "assets/sounds/PhaserLoop.mp3",
                "{name}"
            );
            assert_eq!(
                audio.forcefield.as_ref().expect("[audio.forcefield]").file,
                "assets/sounds/ForcefieldHit.mp3",
                "{name}"
            );
        }
    }

    /// Preserves the volumes the JS previously hardcoded (`hum.volume = 0.25`,
    /// `engine.volume = thrust * 0.15`), so making them data-driven did not
    /// silently change how the game sounds.
    #[test]
    fn cruiser_audio_preserves_legacy_volumes() {
        let config = shipped_hull("alliance_cruiser");
        let audio = config.audio.as_ref().expect("[audio]");
        assert_eq!(audio.ambient.as_ref().unwrap().volume, 0.25);
        assert_eq!(audio.engine.as_ref().unwrap().volume_at_full_thrust, 0.15);
        assert_eq!(audio.engine.as_ref().unwrap().idle_volume, 0.0);
    }

    #[test]
    fn entity_without_audio_block_parses_to_none() {
        let config =
            EntityConfig::from_toml(include_str!("../../assets/entities/station_axiom.toml"))
                .expect("must parse");
        assert!(config.audio.is_none());
    }

    // ── Station hull tests (post-[station] removal; PRD slice 2) ──────────

    #[test]
    fn station_axiom_template_parses_hull_integrity() {
        let toml_str = include_str!("../../assets/entities/station_axiom.toml");
        let config = EntityConfig::from_toml(toml_str).expect("station_axiom.toml must parse");
        let hull = config.hull.as_ref().expect("must have [hull]");
        // (#474) Buffed 200 → 800 for the combat-test scenario, then 800 → 1600
        // in the stationary-station combat retune so the station survives the
        // eight-wave raid alongside its tripled point-defence damage.
        assert!((hull.hull_integrity - 1600.0).abs() < 1e-6);
    }

    #[test]
    fn station_outpost_template_parses_hull_integrity() {
        let toml_str = include_str!("../../assets/entities/station_outpost.toml");
        let config = EntityConfig::from_toml(toml_str).expect("station_outpost.toml must parse");
        let hull = config.hull.as_ref().expect("must have [hull]");
        assert!((hull.hull_integrity - 200.0).abs() < 1e-6);
    }

    #[test]
    fn station_research_outpost_template_parses_hull_integrity() {
        let toml_str = include_str!("../../assets/entities/station_research_outpost.toml");
        let config =
            EntityConfig::from_toml(toml_str).expect("station_research_outpost.toml must parse");
        let hull = config.hull.as_ref().expect("must have [hull]");
        assert!((hull.hull_integrity - 60.0).abs() < 1e-6);
    }

    #[test]
    fn all_sections_parsed_in_full_template() {
        let toml_str = r##"
tags = ["full"]

[asteroid_field]
inner_radius = 100.0
outer_radius = 200.0
density = 0.005

[hull]
hull_integrity = 100
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        assert!(
            config.asteroid_field.is_some(),
            "asteroid_field should be Some"
        );
        assert!(config.hull.is_some(), "hull should be Some");
        assert_eq!(config.tags, vec!["full"]);
    }

    // ── SystemId-keyed hull entries (parent issue #516 sub-issue #616) ────────

    #[test]
    fn hull_system_hull_parses_from_toml() {
        let toml_str = r##"
[hull]
hull_integrity = 100

[[hull.system_hull]]
system_id = "phaser-fore"
display_name = "Phaser Bank (Fore)"
max_hp = 25.0
damaged_threshold_pct = 0.6
disabled_threshold_pct = 0.2
debuff_magnitude = 0.25
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let hull = config.hull.as_ref().expect("hull must parse");
        assert_eq!(hull.system_hull.len(), 1);
        let entry = &hull.system_hull[0];
        assert_eq!(
            entry.system_id,
            crate::messages::SystemId("phaser-fore".into())
        );
        assert_eq!(entry.display_name.as_deref(), Some("Phaser Bank (Fore)"));
        assert!((entry.max_hp - 25.0).abs() < 1e-6);
        assert!((entry.damaged_threshold_pct - 0.6).abs() < 1e-6);
        assert!((entry.disabled_threshold_pct - 0.2).abs() < 1e-6);
        assert!((entry.debuff_magnitude - 0.25).abs() < 1e-6);
    }

    #[test]
    fn hull_system_hull_defaults_when_absent() {
        // Legacy TOML without [[hull.system_hull]] must still parse; the new
        // field defaults to an empty Vec.
        let toml_str = r##"
[hull]
hull_integrity = 100
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let hull = config.hull.as_ref().expect("hull must parse");
        assert!(hull.system_hull.is_empty());
    }

    #[test]
    fn hull_system_hull_entry_optional_fields_default() {
        // Only the required fields (system_id, max_hp) are provided; every
        // other field has a serde default.
        let toml_str = r##"
[hull]
[[hull.system_hull]]
system_id = "helm"
max_hp = 30.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let entry = &config.hull.as_ref().unwrap().system_hull[0];
        assert_eq!(entry.system_id, crate::messages::SystemId("helm".into()));
        assert!(entry.display_name.is_none());
        assert!((entry.damaged_threshold_pct - 0.75).abs() < 1e-6);
        assert!((entry.disabled_threshold_pct - 0.25).abs() < 1e-6);
        assert!((entry.debuff_magnitude - 0.15).abs() < 1e-6);
    }

    // ── Shipped template TOML files referenced by assets/worlds/default.toml ──
    //
    // These tests embed each template at compile time via include_str! so
    // the build fails if a referenced template is missing or malformed.

    #[test]
    fn empty_star_section_uses_defaults() {
        let config = EntityConfig::from_toml("[star]\n").expect("parse must succeed");
        let star = config.star.as_ref().expect("must parse [star]");
        assert!((star.radius - 40.0).abs() < 1e-6);
        assert_eq!(star.longitude_segments, 64);
        assert_eq!(star.latitude_segments, 32);
        assert_eq!(star.surface_colour, [1.0, 0.72, 0.12]);
        assert_eq!(star.hot_colour, [1.0, 0.96, 0.65]);
        assert_eq!(star.cell_colour, [0.95, 0.32, 0.04]);
        assert_eq!(star.halo_colour, [1.0, 0.78, 0.18]);
        assert!((star.halo_radius_multiplier - 2.4).abs() < 1e-6);
        assert!((star.animation_speed - 1.0).abs() < 1e-6);
    }

    #[test]
    fn star_section_overrides_defaults() {
        let toml_str = r#"
[star]
radius = 75.0
longitude_segments = 96
latitude_segments = 48
surface_colour = [0.9, 0.7, 0.2]
hot_colour = [1.0, 1.0, 0.8]
cell_colour = [0.8, 0.2, 0.1]
halo_colour = [1.0, 0.6, 0.1]
halo_radius_multiplier = 3.0
animation_speed = 0.5
"#;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let star = config.star.as_ref().expect("must parse [star]");
        assert!((star.radius - 75.0).abs() < 1e-6);
        assert_eq!(star.longitude_segments, 96);
        assert_eq!(star.latitude_segments, 48);
        assert_eq!(star.surface_colour, [0.9, 0.7, 0.2]);
        assert_eq!(star.hot_colour, [1.0, 1.0, 0.8]);
        assert_eq!(star.cell_colour, [0.8, 0.2, 0.1]);
        assert_eq!(star.halo_colour, [1.0, 0.6, 0.1]);
        assert!((star.halo_radius_multiplier - 3.0).abs() < 1e-6);
        assert!((star.animation_speed - 0.5).abs() < 1e-6);
    }

    #[test]
    fn star_section_rejects_unknown_fields() {
        let result = EntityConfig::from_toml(
            r#"
[star]
radius = 40.0
surfase_colour = [1.0, 0.7, 0.1]
"#,
        );
        assert!(result.is_err());
    }

    #[test]
    fn star_sun_template_parses_with_star_and_lights() {
        let toml_str = include_str!("../../assets/entities/star_sun.toml");
        let config = EntityConfig::from_toml(toml_str).expect("star_sun.toml must parse");
        // Display text lives in assets/strings/strings.csv; the TOML holds the
        // string id, which is what Rust passes through to the client.
        assert_eq!(config.name.as_deref(), Some("entity.star_sun.name"));
        assert!(config.star.is_some(), "star_sun.toml must have [star]");
        assert!(config.mesh.is_none(), "star_sun.toml must not keep [mesh]");
        assert!(
            !config.light.is_empty(),
            "star_sun.toml must have at least one [[light]]"
        );
        assert_eq!(config.light[0].kind, LightKind::Directional);
        let collider = config
            .collider
            .as_ref()
            .expect("star_sun.toml must have [collider]");
        assert_eq!(collider.shape, ColliderShape::Ball);
    }

    #[test]
    fn planet_earth_template_parses_with_mesh_and_collider() {
        let toml_str = include_str!("../../assets/entities/planet_earth.toml");
        let config = EntityConfig::from_toml(toml_str).expect("planet_earth.toml must parse");
        assert_eq!(config.name.as_deref(), Some("entity.planet_earth.name"));
        assert!(config.mesh.is_some(), "planet_earth.toml must have [mesh]");
        let collider = config
            .collider
            .as_ref()
            .expect("planet_earth.toml must have [collider]");
        assert_eq!(collider.shape, ColliderShape::Ball);

        // Textured-planet section: earth has clouds, atmosphere, and
        // night-gated city-light emission without a separate mask.
        let planet = config
            .planet
            .as_ref()
            .expect("planet_earth.toml must have [planet]");
        assert!(planet.surface.normal.is_some());
        assert!(planet.surface.emissive_colour.is_some());
        assert!(planet.surface.emissive_mask.is_none());
        assert!(planet.surface.emissive_night_only);
        assert!(planet.clouds.is_some());
        assert!(planet.atmosphere.is_some());
    }

    #[test]
    fn planet_lava_template_parses_with_dayside_emission() {
        let toml_str = include_str!("../../assets/entities/planet_lava.toml");
        let config = EntityConfig::from_toml(toml_str).expect("planet_lava.toml must parse");
        let planet = config
            .planet
            .as_ref()
            .expect("planet_lava.toml must have [planet]");
        // Lava glows on the day side too — the night gate must be off.
        assert!(!planet.surface.emissive_night_only);
        assert!(planet.surface.emissive_mask.is_some());
        let clouds = planet.clouds.as_ref().expect("ash shell expected");
        assert!((clouds.scale - 1.03).abs() < 1e-6);
        assert!(
            (clouds.drift_speed - 0.0).abs() < 1e-6,
            "no motion by default"
        );
    }

    #[test]
    fn moon_luna_template_parses_surface_only() {
        let toml_str = include_str!("../../assets/entities/moon_luna.toml");
        let config = EntityConfig::from_toml(toml_str).expect("moon_luna.toml must parse");
        let planet = config
            .planet
            .as_ref()
            .expect("moon_luna.toml must have [planet]");
        assert!(planet.surface.emissive_colour.is_none());
        assert!(planet.clouds.is_none());
        assert!(planet.atmosphere.is_none());
    }

    #[test]
    fn asteroid_field_main_template_parses_with_field_and_grid() {
        let toml_str = include_str!("../../assets/entities/asteroid_field_main.toml");
        let config =
            EntityConfig::from_toml(toml_str).expect("asteroid_field_main.toml must parse");
        let field = config
            .asteroid_field
            .as_ref()
            .expect("must have [asteroid_field]");
        field
            .grid
            .as_ref()
            .expect("must have [asteroid_field.grid]");
        // 4 common + 4 uncommon + 4 rare models, each in a small and a large
        // size (issue #946), plus the four commons again at the huge size
        // (issue #947). The cosmetic backdrop stays commons-only.
        assert_eq!(field.asteroid_type_paths.len(), 28);
        assert_eq!(field.cosmetic_type_paths.len(), 4);
    }

    /// The authored rarity groups, pinned to the currently-shipped weights.
    ///
    /// Two axes, deliberately kept apart. *Material* rarity is the class: an
    /// uncommon rock is drawn a tenth as often as a common and a rare a
    /// hundredth (issue #946). *Size* rarity multiplies it: the huge size
    /// (issue #947) is authored at a tenth of its class weight, because a rock
    /// that big is a landmark and at the class weight it would be a third of
    /// every gameplay rock in the field.
    ///
    /// The expected weights below are restated, not read off the TOML, so a
    /// deliberate retune of any group must update this test alongside the
    /// config; only group *membership* (which paths carry which class and
    /// size) is read from the file. If someone drops the weights and the
    /// entries fall back to bare paths, it fails too.
    #[test]
    fn asteroid_field_main_declares_three_rarity_tiers() {
        let toml_str = include_str!("../../assets/entities/asteroid_field_main.toml");
        let config =
            EntityConfig::from_toml(toml_str).expect("asteroid_field_main.toml must parse");
        let field = config
            .asteroid_field
            .as_ref()
            .expect("must have [asteroid_field]");

        let weights_of = |tier: &str, size: &str| -> Vec<f32> {
            field
                .asteroid_type_paths
                .iter()
                .filter(|t| {
                    t.path().contains(&format!("asteroid_{tier}_"))
                        && t.path().ends_with(&format!("_{size}.toml"))
                })
                .map(|t| t.weight())
                .collect()
        };
        let groups = [
            ("common", "small", 1.0f32),
            ("common", "large", 1.0),
            // The size-rarity multiplier, not a class of its own: 1.0 x 0.1.
            ("common", "huge", 0.1),
            ("uncommon", "small", 0.1),
            ("uncommon", "large", 0.1),
            ("rare", "small", 0.01),
            ("rare", "large", 0.01),
        ];
        let mut accounted = 0;
        for (tier, size, expected) in groups {
            let weights = weights_of(tier, size);
            assert_eq!(weights.len(), 4, "{tier} {size}: one entry per model");
            for w in &weights {
                assert!(
                    (w - expected).abs() < 1e-6,
                    "{tier} {size} entries must be authored at weight {expected}, found {w}"
                );
            }
            accounted += weights.len();
        }
        // Every entry belongs to a group named above, so a new class or size
        // cannot land unweighted and unnoticed.
        assert_eq!(
            accounted,
            field.asteroid_type_paths.len(),
            "an entry matches no (class, size) group this test knows about"
        );

        // Only the commons are scaled up. A landmark's job is to be recognised
        // at range, so it is the same four silhouettes every time; scaling the
        // uncommon and rare scans as well would make "that rock is enormous"
        // and "that rock is unusual" the same signal.
        for tier in ["uncommon", "rare"] {
            assert!(
                weights_of(tier, "huge").is_empty(),
                "the huge size is authored on the common class only"
            );
        }

        // The cosmetic layers carry no rarity tiers, so their entries keep the
        // bare-string spelling — which must still read as weight 1.0.
        for entry in &field.cosmetic_type_paths {
            assert!(matches!(entry, AsteroidTypeRef::Path(_)));
            assert!((entry.weight() - 1.0).abs() < 1e-6);
        }
    }

    /// The two shipped fields carry the same authored type lists, and are
    /// rewritten together by `scripts/import-asteroids.mjs` from one class
    /// table. Asserted rather than left to reviewer diligence: they were last
    /// edited by a script that touches both, and a hand-edit to one is exactly
    /// the change nobody would notice until a belt spawned a different mix of
    /// rocks from a field.
    #[test]
    fn both_shipped_asteroid_fields_carry_the_same_type_lists() {
        let field_of = |text: &str| {
            EntityConfig::from_toml(text)
                .expect("field template must parse")
                .asteroid_field
                .expect("must have [asteroid_field]")
        };
        let main = field_of(include_str!(
            "../../assets/entities/asteroid_field_main.toml"
        ));
        let belt = field_of(include_str!(
            "../../assets/entities/asteroid_belt_axiom.toml"
        ));

        let entries = |f: &AsteroidFieldConfig, gameplay: bool| -> Vec<(String, f32)> {
            let list = if gameplay {
                &f.asteroid_type_paths
            } else {
                &f.cosmetic_type_paths
            };
            list.iter()
                .map(|t| (t.path().to_string(), t.weight()))
                .collect()
        };
        assert_eq!(entries(&main, true), entries(&belt, true));
        assert_eq!(entries(&main, false), entries(&belt, false));
    }

    /// The huge size class (issue #947): a triple-size rock, authored as its
    /// own set of entity templates over the SAME four common models.
    ///
    /// `radius` is 12 against large's 4 — the "triple-size" the issue asks
    /// for. `hull_integrity` is 300, three times large's 100 and so linear in
    /// radius rather than in volume: the rule is that time-to-clear scales with
    /// how big the thing looks, and a cruiser's two phaser banks put out 8 hull
    /// a second, so a large rock is ~12 s of sustained fire and a huge one
    /// ~37 s. Cubing it to 2700 would be 5.6 minutes on one rock and would
    /// read as indestructible scenery that happens to have a health bar.
    ///
    /// It keeps `[target]` and `[hull]`: it spawns in the gameplay layer beside
    /// its small and large siblings, and a rock there that could not be
    /// targeted or destroyed would be the one exception the weapons and radar
    /// paths have to learn about. Hull-less, target-less rocks are the cosmetic
    /// backdrop, and the huge class is not that.
    #[test]
    fn the_huge_asteroid_size_is_a_targetable_triple_size_rock() {
        for n in 1..=4 {
            let path = format!("assets/entities/asteroid_common_{n}_huge.toml");
            let cfg = crate::entity_includes::load_entity_config(&path)
                .unwrap_or_else(|e| panic!("{path} must parse: {e}"));

            // Every asteroid variant shares one display id since the strings
            // consolidation (9b89a37b) — the per-variant names were folded into
            // `entity.asteroid.name`.
            assert_eq!(cfg.name.as_deref(), Some("entity.asteroid.name"));
            let collider = cfg
                .collider
                .as_ref()
                .unwrap_or_else(|| panic!("{path}: [collider]"));
            assert_eq!(collider.radius, 12.0, "{path}: three times large's 4");
            let hull = cfg
                .hull
                .as_ref()
                .unwrap_or_else(|| panic!("{path}: [hull]"));
            assert_eq!(
                hull.hull_integrity, 300.0,
                "{path}: three times large's 100"
            );
            assert!(
                cfg.target.is_some(),
                "{path}: a gameplay rock is targetable"
            );

            // Reuses the common model rather than shipping new geometry; the
            // size lives in the `huge` rig variant's scale.
            let mesh = cfg
                .mesh
                .as_ref()
                .unwrap_or_else(|| panic!("{path}: [mesh]"));
            assert_eq!(
                mesh.model.as_deref(),
                Some(&*format!("assets/models/asteroid_common_{n}.glb"))
            );
            assert_eq!(mesh.variant.as_deref(), Some("huge"));
            assert_eq!(mesh.radius, 12.0);
        }
    }

    /// Back-compat: the pre-#946 spelling (a flat list of path strings) still
    /// parses, and means weight 1.0. Every field TOML written before rarity
    /// existed depends on this.
    #[test]
    fn asteroid_type_paths_accept_bare_strings_and_weighted_tables() {
        let toml_str = r#"
tags = ["field"]

[asteroid_field]
inner_radius = 100.0
outer_radius = 200.0
density = 0.005
asteroid_type_paths = [
    "assets/entities/plain.toml",
    { path = "assets/entities/weighted.toml", weight = 0.25 },
    { path = "assets/entities/defaulted.toml" },
]
"#;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let field = config.asteroid_field.expect("must have [asteroid_field]");
        let types = &field.asteroid_type_paths;
        assert_eq!(types.len(), 3);
        assert_eq!(types[0].path(), "assets/entities/plain.toml");
        assert!((types[0].weight() - 1.0).abs() < 1e-6);
        assert_eq!(types[1].path(), "assets/entities/weighted.toml");
        assert!((types[1].weight() - 0.25).abs() < 1e-6);
        // A table that omits `weight` is the same as a bare string.
        assert_eq!(types[2].path(), "assets/entities/defaulted.toml");
        assert!((types[2].weight() - 1.0).abs() < 1e-6);
    }

    // ── Faction field tests ────────────────────────────────────────────────

    #[test]
    fn faction_field_parses_from_entity_toml() {
        let toml_str = r#"
tags = ["ship"]
faction = "aaaaaaaa-1111-4111-8111-aaaaaaaaaaaa"
"#;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let faction = config.faction.expect("faction must be Some");
        assert_eq!(faction.to_string(), "aaaaaaaa-1111-4111-8111-aaaaaaaaaaaa");
    }

    #[test]
    fn faction_field_defaults_to_none_when_absent() {
        let config = EntityConfig::from_toml("").expect("parse must succeed");
        assert!(config.faction.is_none());
    }

    #[test]
    fn battleship_toml_parses_with_federation_faction() {
        let config = shipped_hull("alliance_battleship");
        let faction = config
            .faction
            .expect("alliance_battleship must declare a faction");
        // Must match the Federation UUID in assets/factions/federation.toml
        let fed_toml = include_str!("../../assets/factions/federation.toml");
        let fed = crate::faction::parse_faction_config(fed_toml).unwrap();
        assert_eq!(faction, fed.uuid, "battleship faction must be Federation");
    }

    // ── Behaviour block tests ─────────────────────────────────────────────

    #[test]
    fn behaviour_block_absent_when_not_in_toml() {
        let config = EntityConfig::from_toml("").expect("parse must succeed");
        assert!(config.behaviour.is_none());
    }

    #[test]
    fn entity_with_hull_and_behaviour_has_both_sections() {
        let toml_str = r##"
tags = ["npc"]

[hull]
hull_integrity = 50.0

[behaviour]
"##;
        let config = EntityConfig::from_toml_in_mode(
            toml_str,
            crate::entities::ai_declaration_manifest::AiDeclarationMode::Lenient,
        )
        .expect("parse must succeed");
        assert!(config.hull.is_some());
        assert!(config.behaviour.is_some());
    }

    // ── DoctrineObjective tests ────────────────────────────────────────────

    #[test]
    fn behaviour_with_patrol_doctrine_parses() {
        let toml_str = r##"
[behaviour]

[[behaviour.doctrine]]
id = "patrol-sector"
text = "Patrol the sector"
directive_kind = "Patrol"
directive_anchors = ["alpha", "beta"]
directive_loop = true
base_priority = 20.0
target_speed = 0.5
"##;
        let config = EntityConfig::from_toml_in_mode(
            toml_str,
            crate::entities::ai_declaration_manifest::AiDeclarationMode::Lenient,
        )
        .expect("parse must succeed");
        let behaviour = config.behaviour.expect("behaviour must be Some");
        assert_eq!(behaviour.doctrine.len(), 1);
        let d = &behaviour.doctrine[0];
        assert_eq!(d.id, "patrol-sector");
        assert_eq!(d.directive_kind.as_deref(), Some("Patrol"));
        assert_eq!(d.directive_anchors, vec!["alpha", "beta"]);
        assert!(d.directive_loop);
        assert!((d.base_priority - 20.0).abs() < 1e-5);
        assert!((d.target_speed - 0.5).abs() < 1e-5);
    }

    #[test]
    fn behaviour_with_destroy_doctrine_parses() {
        let toml_str = r##"
[behaviour]

[[behaviour.doctrine]]
id = "destroy-hostiles"
text = "Engage and destroy hostile ships"
directive_kind = "Destroy"
base_priority = 35.0
target_speed = 0.8
maintain_range = 25.0
"##;
        let config = EntityConfig::from_toml_in_mode(
            toml_str,
            crate::entities::ai_declaration_manifest::AiDeclarationMode::Lenient,
        )
        .expect("parse must succeed");
        let d = &config.behaviour.unwrap().doctrine[0];
        assert_eq!(d.id, "destroy-hostiles");
        assert_eq!(d.directive_kind.as_deref(), Some("Destroy"));
        assert!((d.base_priority - 35.0).abs() < 1e-5);
        assert!((d.maintain_range - 25.0).abs() < 1e-5);
    }

    #[test]
    fn doctrine_target_speed_clamped_to_zero_when_negative() {
        let toml_str = r##"
[behaviour]

[[behaviour.doctrine]]
id = "patrol"
text = "Patrol"
base_priority = 10.0
target_speed = -0.5
"##;
        let config = EntityConfig::from_toml_in_mode(
            toml_str,
            crate::entities::ai_declaration_manifest::AiDeclarationMode::Lenient,
        )
        .expect("parse must succeed");
        let d = &config.behaviour.unwrap().doctrine[0];
        assert_eq!(d.target_speed, 0.0, "negative target_speed must clamp to 0");
    }

    #[test]
    fn doctrine_target_speed_clamped_to_one_when_above_one() {
        let toml_str = r##"
[behaviour]

[[behaviour.doctrine]]
id = "pursue"
text = "Pursue"
base_priority = 10.0
target_speed = 1.5
"##;
        let config = EntityConfig::from_toml_in_mode(
            toml_str,
            crate::entities::ai_declaration_manifest::AiDeclarationMode::Lenient,
        )
        .expect("parse must succeed");
        let d = &config.behaviour.unwrap().doctrine[0];
        assert_eq!(d.target_speed, 1.0, "target_speed > 1 must clamp to 1");
    }

    #[test]
    fn behaviour_doctrine_empty_by_default() {
        let toml_str = r##"
[behaviour]
"##;
        let config = EntityConfig::from_toml_in_mode(
            toml_str,
            crate::entities::ai_declaration_manifest::AiDeclarationMode::Lenient,
        )
        .expect("parse must succeed");
        let behaviour = config.behaviour.expect("behaviour must be Some");
        assert!(
            behaviour.doctrine.is_empty(),
            "doctrine array must default to empty"
        );
    }

    #[test]
    fn behaviour_multiple_doctrine_objectives_parse() {
        let toml_str = r##"
[behaviour]

[[behaviour.doctrine]]
id = "patrol"
text = "Patrol"
directive_kind = "Patrol"
directive_anchors = ["wp1", "wp2"]
base_priority = 20.0

[[behaviour.doctrine]]
id = "destroy"
text = "Destroy"
directive_kind = "Destroy"
base_priority = 35.0
"##;
        let config = EntityConfig::from_toml_in_mode(
            toml_str,
            crate::entities::ai_declaration_manifest::AiDeclarationMode::Lenient,
        )
        .expect("parse must succeed");
        let behaviour = config.behaviour.expect("behaviour must be Some");
        assert_eq!(behaviour.doctrine.len(), 2);
        assert_eq!(behaviour.doctrine[0].id, "patrol");
        assert_eq!(behaviour.doctrine[1].id, "destroy");
    }

    // ── ship_harrow_destroyer.toml compile-time template tests ─────────────
    //
    // (#892) These three used to load `pirate_raider.toml`, which was retired
    // as a duplicate: its display string was literally "Harrow Destroyer", the
    // same one `ship_harrow_destroyer.toml` publishes, on a 30-hull ship rather
    // than a 900-hull one. They are re-pointed at the surviving hull rather
    // than dropped — the claims (a Harrow NPC declares the Harrow faction, a
    // positive hull, and both consoles) are about the shipped enemy destroyer,
    // not about which file it lived in.

    #[test]
    fn harrow_destroyer_template_parses_with_harrow_faction() {
        // (#472) The enemy destroyer is Harrow-factioned so the player ship's
        // auto-fire (Federation faction) engages it.
        let toml_str = &resolved_text("ship_harrow_destroyer");
        let config =
            EntityConfig::from_toml(toml_str).expect("ship_harrow_destroyer.toml must parse");
        let faction = config
            .faction
            .expect("the Harrow Destroyer must declare a faction");
        assert_eq!(
            faction.to_string(),
            "cccccccc-3333-4333-8333-cccccccccccc",
            "the Harrow Destroyer's faction must be Harrow (#472)"
        );
    }

    #[test]
    fn harrow_destroyer_template_has_hull() {
        let toml_str = &resolved_text("ship_harrow_destroyer");
        let config =
            EntityConfig::from_toml(toml_str).expect("ship_harrow_destroyer.toml must parse");
        assert!(
            config.hull.is_some(),
            "the Harrow Destroyer must have a [hull] section"
        );
        let hull = config.hull.as_ref().unwrap();
        assert!(
            hull.hull_integrity > 0.0,
            "the Harrow Destroyer's [hull] must have a positive hull_integrity value"
        );
    }

    #[test]
    fn harrow_destroyer_template_has_helm_and_weapons_console() {
        let toml_str = &resolved_text("ship_harrow_destroyer");
        let config =
            EntityConfig::from_toml(toml_str).expect("ship_harrow_destroyer.toml must parse");
        assert!(
            config.helm_console.is_some(),
            "the Harrow Destroyer must have a [helm_console]"
        );
        assert!(
            config.weapons_console.is_some(),
            "the Harrow Destroyer must have a [weapons_console]"
        );
    }

    // ── Shield arc auto-synthesis tests (issue #514) ─────────────────────────

    #[test]
    fn shield_arc_toml_block_synthesises_system_instance() {
        // Minimal ship TOML with a single `[[shield_arc]]` block. The
        // parser must synthesise a matching `[[system]]` entry with
        // `kind = "shield_arc"` and `SystemId("shield-arc-<id>")`.
        let toml_str = r#"
tags = ["ship"]

[[shield_arc]]
id = "fore"
label = "Fore"
center_deg = 0
width_deg = 90
"#;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        assert_eq!(config.shield_arcs.len(), 1);
        assert_eq!(config.shield_arcs[0].id, "fore");

        let ship_config = config
            .ship_config
            .expect("shield_arc must synthesise a ship_config");
        assert_eq!(ship_config.systems.len(), 1);
        let sys = &ship_config.systems[0];
        assert_eq!(sys.id.0, "shield-arc-fore");
        assert_eq!(sys.kind, "shield_arc");
        // No `[shields]` station on this bare ship → arc is ai_only + ownerless.
        assert!(sys.ai_only, "ownerless arc must be ai_only");
        assert!(sys.station.is_none());
    }

    #[test]
    fn shield_arc_synthesises_with_shields_station_when_present() {
        // A ship that declares a `shields` station gets arcs owned by that
        // station with `ai_only = false`.
        let toml_str = r#"
tags = ["ship"]

[[shield_arc]]
id = "fore"
label = "Fore"
center_deg = 0
width_deg = 180

[[shield_arc]]
id = "aft"
label = "Aft"
center_deg = 180
width_deg = 180

[[station]]
id = "shields"
name = "Shields"
description = "Manage shield systems."
rank = "Ens."
short_code = "SHD"
console = "shields"

[[station.rating]]
name = "Std"
automated_systems = []
"#;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let ship_config = config.ship_config.expect("ship_config present");
        assert_eq!(ship_config.systems.len(), 2);
        for sys in &ship_config.systems {
            assert!(
                !sys.ai_only,
                "with a shields station, arcs are player-controlled"
            );
            assert_eq!(
                sys.station,
                Some(crate::messages::StationId("shields".into()))
            );
        }
    }

    /// The Courier is the two-station player hull. Its TOML-authored system
    /// ownership and support loops are individually easy to break, so pin them
    /// against the real asset.
    #[test]
    fn courier_toml_is_a_valid_two_station_hull() {
        use crate::messages::{StationId, SystemId};

        let toml_str = include_str!("../../assets/entities/alliance_courier.toml");
        let config = EntityConfig::from_toml(toml_str).expect("alliance_courier must parse");
        let ship_config = config.ship_config.clone().expect("ship_config present");

        // Two stations, one rating each.
        assert_eq!(ship_config.stations.len(), 2);
        let captain = StationId("captain".into());
        let tactical = StationId("tactical".into());
        assert_eq!(ship_config.stations[0].id, captain);
        assert_eq!(ship_config.stations[1].id, tactical);
        assert_eq!(ship_config.stations[0].ratings.len(), 1);
        assert_eq!(ship_config.stations[1].ratings.len(), 1);
        assert_eq!(
            ship_config.stations[0].console.as_deref(),
            Some("gui/courier/captain.html")
        );
        assert_eq!(
            ship_config.stations[1].console.as_deref(),
            Some("gui/courier/tactical.html")
        );

        // The guns live on Tactical, so every ship-level Tactical gate and the
        // WeaponsUpdate broadcast must resolve there.
        assert_eq!(ship_config.weapons_station(), Some(tactical.clone()));

        // Exactly one weapon: one blaster, no phasers, no torpedoes.
        let weapons = config.weapons_console.as_ref().expect("weapons_console");
        assert_eq!(weapons.blaster_banks.len(), 1);
        assert_eq!(weapons.blaster_banks[0].id, "fore");
        assert!(
            weapons.phaser_banks.is_empty(),
            "courier has no phasers — an absent list must not synthesise a default bank"
        );
        assert!(config.torpedoes.is_none(), "courier has no torpedoes");

        // Power is fully authored, with the three canonical groups.
        assert!(
            config.power.is_some(),
            "courier has an authored [power] block"
        );
        assert_eq!(ship_config.power_groups.len(), 3);

        // Every system is owned by Captain or Tactical. Ownerless + ai_only
        // would be inert on the player spawn path.
        for sys in &ship_config.systems {
            assert!(
                matches!(sys.station.as_ref(), Some(station) if station == &captain || station == &tactical),
                "system {:?} must be station-owned",
                sys.id
            );
            assert!(!sys.ai_only, "system {:?} must not rely on ai_only", sys.id);
        }

        // Two arcs, fore and aft, hang off Captain's shields system.
        assert_eq!(config.shield_arcs.len(), 2);
        let arc_ids: Vec<&str> = config.shield_arcs.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(arc_ids, vec!["fore", "aft"]);

        // Red Alert is human-controlled at the Captain station.
        let automated = &ship_config.stations[0].ratings[0].automated_systems;
        assert!(!automated.contains(&SystemId("red-alert".into())));
        assert!(automated.is_empty());

        // Cinematic button only resolves when this block exists.
        assert!(config.cinematic_camera.is_some());

        // One team serves both stations.
        let repair = config.repair.as_ref().expect("[repair] present");
        assert_eq!(repair.repair_team_count, 1);
        assert!(repair.repair_rate_hp_per_sec < 0.5);
    }

    #[test]
    fn battleship_toml_produces_five_shield_arcs() {
        let config = shipped_hull("alliance_battleship");
        assert_eq!(
            config.shield_arcs.len(),
            5,
            "battleship has 5 arcs (fore, starboard, aft, port, omni)"
        );
        let ids: Vec<&str> = config.shield_arcs.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, vec!["fore", "starboard", "aft", "port", "omni"]);

        // Synthesised systems have the expected shape.
        let ship_config = config.ship_config.expect("ship_config present");
        let arc_systems: Vec<_> = ship_config
            .systems
            .iter()
            .filter(|s| s.kind == "shield_arc")
            .collect();
        assert_eq!(arc_systems.len(), 5);
        let sys_ids: Vec<&str> = arc_systems.iter().map(|s| s.id.0.as_str()).collect();
        assert!(sys_ids.contains(&"shield-arc-fore"));
        assert!(sys_ids.contains(&"shield-arc-port"));
        assert!(sys_ids.contains(&"shield-arc-aft"));
        assert!(sys_ids.contains(&"shield-arc-starboard"));
        assert!(sys_ids.contains(&"shield-arc-omni"));
        // Player ship has a shields station → arcs are player-controlled.
        for sys in &arc_systems {
            assert!(!sys.ai_only);
            assert_eq!(
                sys.station,
                Some(crate::messages::StationId("shields".into()))
            );
            assert_eq!(sys.power_group, None);
        }
    }

    #[test]
    fn npc_ship_with_single_shield_arc_produces_one_arc_system() {
        // Verify each NPC TOML produces exactly one arc system, ai_only,
        // ownerless (no `shields` station declared on NPCs).
        for (path, expected_max_hp) in [
            // (#892) `pirate_raider.toml` + `pirate_raider_reinforcement.toml`
            // (15 each) were retired as duplicates; the Harrow Destroyer that
            // replaced them in the combat-test waves takes their place here.
            ("../../assets/entities/ship_harrow_destroyer.toml", 40),
            ("../../assets/entities/ship_harrow_patrol.toml", 60),
            ("../../assets/entities/ship_harrow_warhawk.toml", 120),
        ] {
            let toml_str = match path {
                "../../assets/entities/ship_harrow_destroyer.toml" => {
                    &resolved_text("ship_harrow_destroyer")
                }
                "../../assets/entities/ship_harrow_patrol.toml" => {
                    &resolved_text("ship_harrow_patrol")
                }
                "../../assets/entities/ship_harrow_warhawk.toml" => {
                    &resolved_text("ship_harrow_warhawk")
                }
                _ => unreachable!(),
            };
            let config = EntityConfig::from_toml(toml_str)
                .unwrap_or_else(|e| panic!("{path} must parse: {e}"));
            assert_eq!(
                config.shield_arcs.len(),
                1,
                "{path} must declare exactly one shield arc"
            );
            let arc = &config.shield_arcs[0];
            assert_eq!(arc.id, "all", "{path} NPC arc id must be 'all'");
            assert_eq!(arc.max_hp, Some(expected_max_hp), "{path} arc max_hp");

            let ship_config = config
                .ship_config
                .unwrap_or_else(|| panic!("{path} must have ship_config after arc synthesis"));
            let arc_systems: Vec<_> = ship_config
                .systems
                .iter()
                .filter(|s| s.kind == "shield_arc")
                .collect();
            assert_eq!(arc_systems.len(), 1, "{path} exactly one arc system");
            let sys = arc_systems[0];
            assert_eq!(sys.id.0, "shield-arc-all", "{path} SystemId shape");
            assert!(sys.ai_only, "{path} NPC arc must be ai_only");
            assert!(sys.station.is_none(), "{path} NPC arc must be ownerless");
        }
    }

    #[test]
    fn shield_arc_with_hull_max_hp_captures_tier_config() {
        let toml_str = r#"
tags = ["ship"]

[[shield_arc]]
id = "fore"
label = "Fore"
center_deg = 0
width_deg = 90
hull_max_hp = 7
hull_damaged_threshold_pct = 0.60
hull_disabled_threshold_pct = 0.20
hull_debuff_magnitude = 0.30
"#;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let arc = &config.shield_arcs[0];
        assert_eq!(arc.hull_max_hp, 7.0);
        assert!((arc.hull_damaged_threshold_pct - 0.60).abs() < 1e-6);
        assert!((arc.hull_disabled_threshold_pct - 0.20).abs() < 1e-6);
        assert!((arc.hull_debuff_magnitude - 0.30).abs() < 1e-6);
    }

    #[test]
    fn shield_arc_hull_thresholds_default_when_omitted() {
        let toml_str = r#"
tags = ["ship"]

[[shield_arc]]
id = "fore"
label = "Fore"
center_deg = 0
width_deg = 90
hull_max_hp = 6
"#;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let arc = &config.shield_arcs[0];
        assert!((arc.hull_damaged_threshold_pct - 0.75).abs() < 1e-6);
        assert!((arc.hull_disabled_threshold_pct - 0.25).abs() < 1e-6);
        assert!((arc.hull_debuff_magnitude - 0.15).abs() < 1e-6);
    }

    #[test]
    fn harrow_destroyer_template_has_shields_block() {
        // (#474) The Harrow Destroyer has a single omni shield (#471).
        // (#514) Migrated to `[[shield_arc]]` block; `[shields_console]`
        // block was retired for NPCs.
        // (#892) Re-pointed off the retired `pirate_raider.toml` duplicate. The
        // regen rate is load-bearing here, not incidental: the hull's #788
        // recovery doctrine sits out its standoff orbit at exactly this rate.
        let toml_str = &resolved_text("ship_harrow_destroyer");
        let config =
            EntityConfig::from_toml(toml_str).expect("ship_harrow_destroyer.toml must parse");
        assert_eq!(
            config.shield_arcs.len(),
            1,
            "the Harrow Destroyer must declare exactly one [[shield_arc]] block"
        );
        let arc = &config.shield_arcs[0];
        assert_eq!(arc.id, "all");
        assert_eq!(arc.max_hp, Some(40));
        assert!((arc.regen_per_sec.expect("regen") - 4.0).abs() < 1e-6);
    }

    #[test]
    fn ship_harrow_patrol_phaser_has_shield_pierce() {
        // (#474) Harrow weapons all have 0.1 pierce.
        // (#892) Re-pointed off the retired `pirate_raider.toml`. The Ironveil,
        // not the Harrow Destroyer, inherits this claim: the Destroyer carries
        // no phaser bank at all (blasters only — see
        // `harrow_destroyer_carries_forward_blasters_and_no_torpedoes`), so it
        // could not carry a phaser-pierce assertion.
        let toml_str = &resolved_text("ship_harrow_patrol");
        let config = EntityConfig::from_toml(toml_str).expect("ship_harrow_patrol.toml must parse");
        let wc = config.weapons_console.as_ref().unwrap();
        let bank = wc.phaser_banks.first().expect("must have a phaser bank");
        assert_eq!(bank.shield_pierce, Some(0.1));
    }

    #[test]
    fn ship_harrow_patrol_template_has_two_phaser_banks_and_shields() {
        // (#474) Cruiser gained weapons + shields.
        // (#514) Migrated to `[[shield_arc]]` block.
        let toml_str = &resolved_text("ship_harrow_patrol");
        let config = EntityConfig::from_toml(toml_str).expect("ship_harrow_patrol.toml must parse");
        let wc = config
            .weapons_console
            .as_ref()
            .expect("cruiser must have [weapons_console] (#474)");
        assert_eq!(
            wc.phaser_banks.len(),
            2,
            "cruiser must have port + starboard banks"
        );
        assert_eq!(
            config.shield_arcs.len(),
            1,
            "cruiser must declare one [[shield_arc]] block"
        );
        let arc = &config.shield_arcs[0];
        assert_eq!(arc.id, "all");
        assert_eq!(arc.max_hp, Some(60));
        assert!((arc.regen_per_sec.expect("regen") - 1.0).abs() < 1e-6);
    }

    #[test]
    fn ship_harrow_warhawk_template_has_full_behaviour_and_weapons() {
        // (#474) Battleship gained a full behaviour tree + weapons +
        // shields. Previously was a stub.
        // (#792) Gained a bow artillery blaster bank alongside the two beam
        // banks — asserted here as well, because "two banks" alone would go on
        // passing if the artillery piece were dropped.
        let toml_str = &resolved_text("ship_harrow_warhawk");
        let config =
            EntityConfig::from_toml(toml_str).expect("ship_harrow_warhawk.toml must parse");
        let wc = config
            .weapons_console
            .as_ref()
            .expect("battleship must have [weapons_console] (#474)");
        assert_eq!(
            wc.phaser_banks.len(),
            2,
            "battleship must have 2 beam banks"
        );
        let bank = &wc.phaser_banks[0];
        assert!((bank.beam_damage_per_sec - 12.0).abs() < 1e-6);
        assert!((bank.beam_range - 75.0).abs() < 1e-6);
        assert_eq!(
            wc.blaster_banks.len(),
            1,
            "battleship must carry exactly one artillery bank (#792) — the helm \
             doctrine reads its flight speed as the lead speed, and a second, \
             longer-reaching bank would silently become the one it leads by"
        );
        // (#514) Battleship migrated to `[[shield_arc]]` block.
        assert_eq!(
            config.shield_arcs.len(),
            1,
            "battleship must declare one [[shield_arc]] block"
        );
        let arc = &config.shield_arcs[0];
        assert_eq!(arc.id, "all");
        assert_eq!(arc.max_hp, Some(120));
        let behaviour = config.behaviour.as_ref().expect("must have [behaviour]");
        let directive_kinds: Vec<Option<&str>> = behaviour
            .doctrine
            .iter()
            .map(|d| d.directive_kind.as_deref())
            .collect();
        assert!(
            directive_kinds.contains(&Some("Patrol")),
            "battleship must have a Patrol doctrine (#572 doctrine-based AI)"
        );
        assert!(
            directive_kinds.contains(&Some("Destroy")),
            "battleship must have a Destroy doctrine (#572 doctrine-based AI)"
        );
    }

    // ── The Harrow Battleship artillery platform (issue #792) ────────────────

    /// AC1/AC2/AC3, as content: both travel axes author the three-state machine,
    /// the yaw channel resolves the SEVENTH mode verb in the hold and tracks on
    /// the way in, and every scalar the host reads by name is present on the
    /// Steering axis.
    ///
    /// The verb assertion carries the whole of the "why a new verb" argument, so
    /// it is spelled out rather than left to the constant: `pivot_to_reengage`
    /// has identical geometry to nothing here — its host gate is the six
    /// shield-RECOVERY scalars, all of them statements about a ring derived from
    /// the TARGET's reach, and an artillery platform authoring five unrelated
    /// standoff numbers in order to borrow one turn is exactly the invention
    /// AGENTS.md #11 forbids. `hold_torpedo_bearing` is closer and still wrong:
    /// it tracks the target's LIVE position with no lead at all, which at this
    /// hull's flight time is a different bearing from the one the gun fires on.
    #[test]
    fn harrow_warhawk_authors_the_artillery_machine_on_both_travel_axes() {
        let cfg = EntityConfig::from_toml(&harrow_warhawk_toml()).expect("hull must parse");
        let hc = cfg
            .helm_console
            .as_ref()
            .expect("the hull declares [helm_console]");

        for (name, ai) in [
            ("engines_ai", hc.engines_ai.as_ref()),
            ("steering_ai", hc.steering_ai.as_ref()),
        ] {
            let ai = ai.unwrap_or_else(|| panic!("{name} must be authored"));
            assert!(
                ai.rule.is_empty(),
                "{name} must be state-only (rule XOR state)"
            );
            let ids: Vec<&str> = ai.state.iter().map(|s| s.id.as_str()).collect();
            assert_eq!(
                ids,
                vec!["shadow", "acquire", "reposition", "hold"],
                "{name} resolves to the class artillery machine"
            );
            // `shadow` and `initial_state = "shadow"` arrive with the class
            // doctrine (issue #878): the shared fragment RESTS defensive on a
            // standoff ring and a hull unlocks the gun line by posture. This hull
            // authors `press_posture = 0.0`, the lowest rung, so the gate is open
            // on the first tick and the defensive leg is left immediately and
            // never re-entered.
            assert_eq!(ai.initial_state.as_deref(), Some("shadow"));
            assert!(
                ai.to_policy().expect("must decode").machine().is_some(),
                "{name} must decode to a machine"
            );
        }

        let steering = hc.steering_ai.as_ref().unwrap();
        let verb_of = |state_id: &str| -> String {
            let state = steering
                .state
                .iter()
                .find(|s| s.id == state_id)
                .unwrap_or_else(|| panic!("steering_ai must declare '{state_id}'"));
            assert_eq!(
                state.rule.len(),
                1,
                "'{state_id}' answers yaw with one rule"
            );
            state.rule[0].verb.clone()
        };
        assert_eq!(verb_of("acquire"), HELM_ACTUATE_DESIRED_FACING_VERB);
        assert_eq!(
            verb_of("reposition"),
            HELM_ACTUATE_DESIRED_FACING_VERB,
            "the run-in tracks the target itself: nothing is being fired at this \
             range, and a run-in aimed at an intercept would arrive beside it"
        );
        assert_eq!(
            verb_of("hold"),
            HELM_HOLD_ARTILLERY_POSITION_VERB,
            "the firing position is the SEVENTH yaw verb — NOT `pivot_to_reengage`, \
             whose host gate is the six shield-recovery scalars this hull would \
             have to invent, and NOT `hold_torpedo_bearing`, which points at where \
             the target IS rather than where the bolt and the target meet"
        );

        // Every scalar the host reads off this axis BY NAME. A rename in either
        // direction lights this up, and it must: the host's response to a missing
        // one is to decline the whole arm and fly ordinary doctrine travel.
        for required in crate::ship::helm_ai::ARTILLERY_PARAMS {
            assert!(
                steering.param.contains_key(*required),
                "steering_ai must author `{required}`: the host gates the whole \
                 artillery arm on all three together, and the throttle this hull \
                 wants (0.0) is indistinguishable from an omission unless the NAME \
                 is present"
            );
        }
        for required in [
            crate::ship::helm_ai::TRACKING_DEADBAND_PARAM,
            crate::ship::helm_ai::TRACKING_FULL_STEER_PARAM,
        ] {
            assert!(
                steering.param.contains_key(required),
                "steering_ai must author `{required}`"
            );
        }
        // ...and the absences that are still absences.
        //
        // Issue #878 composed this hull on `fragments/ai/movement_artillery.toml`,
        // and the class doctrine's DEFENSIVE leg — the standoff ring it rests on
        // while the alert is down — genuinely circles, so the six recovery
        // scalars and the circulation slot arrive with it and are no longer
        // absences to assert. What has NOT changed is that this hull's gun line
        // borrows nothing: it authors no combat-orbit and no bow-hold scalar, so
        // the artillery arm and the class standoff are the only leg sets the host
        // can publish for it. (`press_posture = 0.0` then makes the standoff
        // unreachable in practice — see the doctrine-tuning note on the hull —
        // but the fragment gates those six as one unit, so they stay declared.)
        assert!(
            steering
                .memory
                .contains_key(crate::ship::helm_ai::ORBIT_DIRECTION_MEMORY),
            "the class standoff ring needs its circulation slot declared, so its \
             pre-engagement value is authored rather than implicit"
        );
        for absent in crate::ship::helm_ai::COMBAT_ORBIT_PARAMS
            .iter()
            .chain(crate::ship::helm_ai::TORPEDO_BEARING_PARAMS)
        {
            assert!(
                !steering.param.contains_key(*absent),
                "steering_ai must NOT author `{absent}`: this hull flies one leg \
                 set and borrowing another's scalars is how a doctrine acquires \
                 behaviour nobody authored"
            );
        }
    }

    /// AC2, as content: the hold band is a PAIR of authored values, the inner one
    /// is ninety per cent of the outer, and the outer matches the bolt's own reach.
    ///
    /// The ratio is asserted here rather than computed in Rust deliberately — the
    /// point of AGENTS.md #11 is that a designer retunes the band by editing two
    /// numbers, and this test is what tells them if they broke the relationship
    /// the acceptance criterion names.
    #[test]
    fn harrow_warhawk_hold_range_is_ninety_percent_of_its_artillery_envelope() {
        let cfg = EntityConfig::from_toml(&harrow_warhawk_toml()).expect("hull must parse");
        let steering = cfg
            .helm_console
            .as_ref()
            .and_then(|hc| hc.steering_ai.as_ref())
            .expect("hull authors [helm_console.steering_ai]");
        let max = steering.param[crate::ship::helm_ai::MAX_ARTILLERY_RANGE_PARAM];
        let hold = steering.param[crate::ship::helm_ai::ARTILLERY_HOLD_RANGE_PARAM];

        assert!(
            (hold - max * 0.9).abs() < 1e-3,
            "repositioning must stop at ninety per cent of the envelope: \
             {hold} vs {max} * 0.9"
        );
        assert!(
            hold < max,
            "the band must have a gap — one threshold is not hysteresis, it is a \
             boundary the hull sits on and chatters across"
        );

        // The outer edge names the bolt's own reach, so the hull never holds a
        // gun line it cannot shoot down.
        let bank = &cfg
            .weapons_console
            .as_ref()
            .expect("hull declares [weapons_console]")
            .blaster_banks[0];
        assert!(
            (max - bank.range).abs() < 1e-3,
            "the artillery envelope ({max}) must be the bank's own range ({})",
            bank.range
        );

        // Engines runs its own copy of the machine and must reason about the SAME
        // band; a drift between the two axes is a ship whose thrust and yaw
        // disagree about which leg it is flying.
        let engines = cfg
            .helm_console
            .as_ref()
            .and_then(|hc| hc.engines_ai.as_ref())
            .expect("hull authors [helm_console.engines_ai]");
        for name in [
            crate::ship::helm_ai::MAX_ARTILLERY_RANGE_PARAM,
            crate::ship::helm_ai::ARTILLERY_HOLD_RANGE_PARAM,
        ] {
            assert_eq!(
                engines.param.get(name),
                steering.param.get(name),
                "both travel axes must author the same `{name}`"
            );
        }
    }

    /// AC4, as content: the bow bolt is POWERFUL and SLOW, and its slowness is
    /// what buys a manoeuvring target time to leave the predicted intercept.
    #[test]
    fn harrow_warhawk_bow_bolt_is_powerful_and_slow() {
        let cfg = EntityConfig::from_toml(&harrow_warhawk_toml()).expect("hull must parse");
        let wc = cfg.weapons_console.as_ref().unwrap();
        let bolt = &wc.blaster_banks[0];
        assert_eq!(bolt.facing_deg, 0.0, "the artillery piece is a BOW mount");

        // POWERFUL: one bolt lands more than either beam bank does in a second of
        // continuous fire, by a wide margin. Compared against the hull's own guns
        // rather than an absolute, so a future rebalance of the beams is what
        // this reads against.
        let beam_dps = wc
            .phaser_banks
            .iter()
            .map(|b| b.beam_damage_per_sec)
            .fold(0.0_f32, f32::max);
        assert!(
            bolt.damage as f32 > beam_dps * 4.0,
            "one artillery bolt ({}) must dwarf a second of beam fire ({beam_dps}): \
             the hull gets one shot every {} s and it has to be worth the wait",
            bolt.damage,
            bolt.cooldown_secs
        );

        // SLOW: slower than every other blaster the game ships, and slow enough
        // that crossing the hull's own envelope takes real seconds — which is the
        // window a course change after launch has to work in.
        //
        // Full paths rather than `shipped_hull` stems because the comparison set
        // deliberately reaches OUTSIDE the shipped fleet: issue #954 moved the
        // three-weapon RNG-coverage escort to `assets/entities/test/`, and its
        // `spike` bank is still a blaster this repo authors. Dropping it because
        // it stopped shipping would quietly shrink the set this claim is measured
        // against, which is the weaker test dressed up as the same one.
        for name in [
            "assets/entities/test/rng_coverage_lancer.toml",
            "assets/entities/ship_harrow_destroyer.toml",
            "assets/entities/alliance_destroyer.toml",
        ] {
            let other = crate::entity_includes::load_entity_config(name)
                .unwrap_or_else(|e| panic!("{name} must compose and parse: {e}"));
            for bank in &other.weapons_console.as_ref().unwrap().blaster_banks {
                assert!(
                    bolt.projectile_speed < bank.projectile_speed,
                    "the artillery bolt ({}) must be slower than {name}'s '{}' \
                     ({}) — the flight time IS the mechanic",
                    bolt.projectile_speed,
                    bank.id,
                    bank.projectile_speed
                );
            }
        }
        let flight_secs = bolt.range / bolt.projectile_speed;
        assert!(
            flight_secs > 4.0,
            "a bolt must take real seconds ({flight_secs}) to cross the envelope, \
             or 'rewards course changes after launch' is unobservable"
        );

        // How much crossing speed the bow cone admits, which is the failure mode
        // a tighter arc would hide: the fire gate reads the target's CURRENT
        // bearing while the hull is pointed at the intercept, so a cone sized for
        // a stationary target declines exactly the shots the prediction exists to
        // take.
        //
        // ## The lead angle is `asin(v/c)`, and it used to be `atan(v/c)`
        //
        // This derivation was re-authored when the intercept solver stopped being
        // a first-order estimate. For a target crossing square across the line of
        // sight at `v` against a bolt of speed `c`:
        //
        //   * the exact intercept solves `d² + (v·t)² = (c·t)²`, giving
        //     `t = d / sqrt(c² − v²)` and a lead angle of `asin(v/c)`;
        //   * the old estimate solved `t = d / c`, giving `atan(v/c)`.
        //
        // `asin` exceeds `atan` everywhere, and the gap widens fast as `v`
        // approaches `c`. So an EXACT solver asks the cone for MORE headroom than
        // the approximation ever did — the arc has not moved, the honest number
        // for what it must admit has.
        let hulls = [
            "ship_harrow_destroyer",
            "ship_harrow_cruiser",
            "ship_harrow_patrol",
            "alliance_courier",
            "alliance_destroyer",
        ]
        .into_iter()
        .map(shipped_hull)
        .filter_map(|c| c.helm_console)
        .collect::<Vec<_>>();
        let mut cruises = hulls.iter().map(|hc| hc.max_speed).collect::<Vec<_>>();
        cruises.sort_by(f32::total_cmp);
        let half_arc = bolt.fire_arc_deg * 0.5;
        let lead_angle = |v: f32| simmath::asin(v / bolt.projectile_speed).to_degrees();

        // Inverted, the cone's own admission limit: the fastest square-on crosser
        // whose lead still fits inside the half-arc.
        //
        // The inversion is `asin`'s, and `asin` only inverts `sin` on [0, 90].
        // Past a 90 deg half-arc `sin` turns back down, so `admits_crossing`
        // would start SHRINKING as the cone got wider and the pinned finding
        // below would pass trivially while the cone admitted every lead there
        // is — the exact silent pass this whole block exists to prevent. Assert
        // it rather than trusting the reader, because a designer widening
        // `fire_arc_deg` has no reason to come and read this.
        assert!(
            half_arc <= 90.0,
            "this derivation inverts `asin` and is only valid up to a 90 deg \
             half-arc; `fire_arc_deg` is now {} deg. A cone this wide admits \
             every lead, so re-derive the admission limit (or drop it) rather \
             than letting `sin` fold back and pass the finding below for free.",
            bolt.fire_arc_deg
        );
        let admits_crossing = bolt.projectile_speed * simmath::sin(half_arc.to_radians());

        // Every shipped hull is admitted at square-on cruise.
        // This is the property that actually has to hold, and a content change
        // that broke it — a slower bolt, a tighter cone, a general speed-up of the
        // fleet — fails here.
        for &v in &cruises {
            assert!(
                half_arc > lead_angle(v),
                "the bow cone ({} deg) must admit the {} deg lead a {v} u/s \
                 square-on crosser produces",
                bolt.fire_arc_deg,
                lead_angle(v)
            );
        }

        // ## FINDING, pinned rather than papered over
        //
        // The fastest shipped CRUISE no longer fits. The Harrow destroyer crosses
        // at 26 u/s against a 35 u/s bolt, which is `asin(26/35)` ≈ 48 deg of lead
        // — past the 45 deg half-arc. Under the old first-order estimate it read
        // `atan(26/35)` ≈ 37 deg and fitted, but that shot was never going to
        // connect: the estimate was under-leading by a ship's length and more.
        //
        // The consequence is the same bounded, benign one boost has always had:
        // the fire gate finds the target outside the arc and DECLINES, so the
        // battleship holds its round against a full-cruise square-on crosser
        // rather than loosing a mis-aimed bolt. It is only the SQUARE-ON case —
        // any closing or opening component shortens the lead and brings the shot
        // back inside the cone — and the destroyer is the only hull affected.
        //
        // The cone is deliberately NOT widened here. Admitting 26 u/s square-on
        // wants a 96 deg cone, and past that the boost case (2.4× = 62 u/s, which
        // is faster than the bolt and has no intercept at all) is unreachable at
        // any width. That is a tuning decision for a designer — either widen the
        // arc, or speed the bolt up — and it should be made on purpose, not
        // acquired by an assertion quietly relaxing.
        assert!(
            cruises.iter().all(|&v| v <= admits_crossing),
            "the bow cone ({} deg) admits square-on crossers up to \
             {admits_crossing} u/s, so every shipped cruise must fit inside it.",
            bolt.fire_arc_deg
        );
        let fastest_boosted = hulls
            .iter()
            .map(|hc| hc.max_speed * hc.boost.as_ref().map(|b| b.multiplier).unwrap_or(1.0))
            .fold(0.0_f32, f32::max);
        assert!(
            fastest_boosted > bolt.projectile_speed,
            "and a BOOSTED crosser ({fastest_boosted} u/s) outruns the bolt \
             ({} u/s) outright — no cone admits a shot that has no intercept",
            bolt.projectile_speed
        );
    }

    /// AC4's plumbing: the artillery bank is declared as an AI-operable system
    /// under the id the registry derives, or the battleship holds its gun line in
    /// silence and every helm assertion above still passes.
    #[test]
    fn harrow_warhawk_declares_its_artillery_bank_as_a_system() {
        let cfg = EntityConfig::from_toml(&harrow_warhawk_toml()).expect("hull must parse");
        let bank_id = cfg.weapons_console.as_ref().unwrap().blaster_banks[0]
            .id
            .clone();
        let expected = crate::system_registry::blaster_bank_system_id(&bank_id)
            .expect("a non-empty bank id resolves to a system id");
        let systems = &cfg
            .ship_config
            .as_ref()
            .expect("hull declares [[system]] blocks")
            .systems;
        let declared = systems
            .iter()
            .find(|s| s.id == expected)
            .unwrap_or_else(|| panic!("hull must declare `{}`", expected.0));
        // Since #871 the hull carries crew stations, so the bank is owned by
        // Tactical rather than being ownerless + `ai_only`. It is still
        // AI-operated on an unmanned hull — the Tactical seat boots on the
        // implicit `Backfill` rating, which automates every system it owns —
        // but the ownership is now what says so, not the `ai_only` flag.
        assert!(
            !declared.ai_only,
            "a station-owned system must not rely on `ai_only`"
        );
        assert_eq!(
            declared.station,
            Some(crate::messages::StationId("tactical".into())),
            "the artillery bank belongs to the Tactical seat"
        );
    }

    /// AC6, as content: nothing in this doctrine is guarded on a hazard.
    ///
    /// The three avoidance layers that DO apply — repulsion summed onto the
    /// solved facing inside the pure planner, the lateral-thrust axis nudging the
    /// hull off its held point, and the imminent-collision facing override — are
    /// all stateless and all outside the machine. A transition guarded on hazard
    /// urgency would turn a temporary bend into a state with an exit, which is
    /// how an artillery platform becomes an orbiting one.
    #[test]
    fn harrow_warhawk_authors_no_hazard_guarded_transition() {
        let cfg = EntityConfig::from_toml(&harrow_warhawk_toml()).expect("hull must parse");
        let hc = cfg.helm_console.as_ref().unwrap();
        for (name, ai) in [
            ("engines_ai", hc.engines_ai.as_ref().unwrap()),
            ("steering_ai", hc.steering_ai.as_ref().unwrap()),
        ] {
            for state in &ai.state {
                for transition in &state.transition {
                    assert!(
                        !transition
                            .when
                            .contains(crate::ship::helm_ai::HAZARD_URGENCY_FACT)
                            && !transition.when.contains("collision"),
                        "{name} state '{}' guards a transition on a hazard reading \
                         (`{}`): avoidance must stay a stateless bend, never a leg",
                        state.id,
                        transition.when
                    );
                }
            }
        }
    }

    /// The battleship switches its impulse drive off, and does it on the axis a
    /// scenario cannot reach.
    ///
    /// This is the content half of #792's blocking defect. `entities::spawner`
    /// gives an `ImpulseConfigResource` to every hull that declares a
    /// `[helm_console]` — parse defaults of engage 200 / cancel 40 — and the
    /// impulse autopilot replaces commanded throttle with full thrust while the
    /// drive runs. The authored hold band sits ENTIRELY inside that window, so an
    /// engaged drive discards the whole doctrine and flies the hull to the drive's
    /// release range instead. This hull is the first whose held radius lies there;
    /// the cruiser's ring is inside the cancel distance and the destroyer's legs
    /// are high-speed passes, so neither sibling ever noticed.
    ///
    /// The two halves asserted here are both load-bearing:
    ///
    /// * an explicit `idle` (not merely an absent block — absent means the
    ///   canonical UNCONDITIONAL PERMIT is synthesised at spawn, which is the
    ///   defect), and
    /// * the band still sitting inside the drive's default window, which is the
    ///   reason the idle is needed. If a future retune moved the band clear, this
    ///   assertion is what says so rather than leaving the `idle` looking like
    ///   superstition.
    ///
    /// Deliberately NOT expressed as `[[behaviour.doctrine]] use_impulse = false`:
    /// doctrine is the part of a hull a scenario replaces wholesale, and both
    /// `duel.toml` and `combat_test.toml`'s wave 8 do exactly that without
    /// authoring `use_impulse` — which `effective_use_impulse()` then resolves to
    /// TRUE. `harrow_warhawk_scenarios_cannot_re_enable_the_impulse_drive` pins
    /// that this is not hypothetical.
    #[test]
    fn harrow_warhawk_holds_its_impulse_drive_idle() {
        let cfg = EntityConfig::from_toml(&harrow_warhawk_toml()).expect("hull must parse");
        let hc = cfg.helm_console.as_ref().unwrap();
        let impulse_ai = hc.impulse_ai.as_ref().expect(
            "the battleship must author `[helm_console.impulse_ai]`: an ABSENT block \
             synthesises the canonical unconditional permit at spawn, which is the \
             defect, not the fix",
        );
        assert!(
            impulse_ai.idle,
            "the declaration must be an explicit idle — the impulse channel \
             resolving to nothing, whatever geometry or doctrine the host is handed"
        );
        assert!(
            impulse_ai.rule.is_empty() && impulse_ai.state.is_empty(),
            "an idle declaration carries no rules and no states (content validation \
             rejects the contradiction), so anything here is dead content"
        );

        // The reason it is needed: the authored band lies inside the drive's
        // default cruise window, so an engaged drive would cross the whole of it
        // at `thrust = 1.0`.
        let steering = hc.steering_ai.as_ref().unwrap();
        let hold = steering.param[crate::ship::helm_ai::ARTILLERY_HOLD_RANGE_PARAM];
        assert!(
            hc.impulse_cancel_distance < hold && hold < hc.impulse_engage_distance,
            "the hold range ({hold}) sits inside the impulse cruise window \
             (engage {}, cancel {}) — if a retune ever moves it clear, revisit \
             whether the idle above is still earning its place",
            hc.impulse_engage_distance,
            hc.impulse_cancel_distance
        );
    }

    /// The deliberate absences named in the hull header. All three are exactly the
    /// kind of content that gets helpfully filled in later, and each would quietly
    /// take the battleship off the firing position this issue exists to hold.
    #[test]
    fn harrow_warhawk_authors_no_boost_drive_and_no_helm_radar() {
        let cfg = EntityConfig::from_toml(&harrow_warhawk_toml()).expect("hull must parse");
        let hc = cfg.helm_console.as_ref().unwrap();
        assert!(
            hc.boost.is_none(),
            "the battleship mounts no boost drive: an artillery platform that lit \
             one would be leaving the firing position it just took up"
        );
        assert_idle_boost_declaration(
            hc,
            "the battleship: no boost doctrine to go with the drive it does not have",
        );
        assert!(
            hc.radar.is_none(),
            "and authors no `[helm_console.radar]`: an unauthored radar range means \
             UNLIMITED helm visibility, which is what lets a {}-unit envelope \
             resolve a target at all",
            hc.steering_ai.as_ref().unwrap().param[crate::ship::helm_ai::MAX_ARTILLERY_RANGE_PARAM]
        );
    }

    /// The scenarios that replace this hull's doctrine must not be able to switch
    /// the drive back on.
    ///
    /// A `use_impulse = false` on the hull's own `[[behaviour.doctrine]]` reads
    /// like the natural lever and would have been erased by every scenario that
    /// actually fields this hull. That is asserted against the shipped world files
    /// rather than described, because the claim is about THEM: each replaces the
    /// doctrine list wholesale and none authors `use_impulse`, so
    /// `effective_use_impulse()` resolves TRUE for their non-Patrol directives.
    /// The fix therefore has to live on the fine system's own policy, which is
    /// what the test above pins.
    #[test]
    fn harrow_warhawk_scenarios_cannot_re_enable_the_impulse_drive() {
        let doctrine = DoctrineObjective {
            directive_kind: Some("Destroy".into()),
            use_impulse: None,
            ..Default::default()
        };
        assert!(
            doctrine.effective_use_impulse(),
            "precondition: an unauthored `use_impulse` on a Destroy directive \
             defaults to permitting the drive — that default is what makes a \
             doctrine-level fix worthless here"
        );

        // The doctrine-replacement marker each world writes. BOTH are
        // [script]-authored since issue #984, so both spell it as a Rhai map and
        // the declarative `behaviour = { doctrine = [` form appears in no
        // shipped world at all. It means the same thing either way: "this
        // scenario replaces a spawned hull's doctrine list".
        for (name, world, doctrine_marker) in [
            (
                "combat_test.toml",
                include_str!("../../assets/worlds/combat_test.toml"),
                "behaviour: #{ doctrine: [",
            ),
            (
                "duel.toml",
                include_str!("../../assets/worlds/duel.toml"),
                "behaviour: #{ doctrine: [",
            ),
        ] {
            assert!(
                world.contains(doctrine_marker),
                "precondition: {name} must replace a spawned hull's doctrine list \
                 for this to be the scenario shape under test"
            );
            assert!(
                !world.contains("use_impulse"),
                "{name} authors `use_impulse` somewhere — if a scenario has started \
                 speaking about the drive, re-read whether the battleship's \
                 `[helm_console.impulse_ai]` idle is still the whole story"
            );
        }
    }

    /// The structural half of "decline rather than invent": the two range params
    /// cannot silently vanish from the FILE, because the doctrine's own
    /// transition guards name them and content validation rejects an undeclared
    /// `param(...)` at load.
    ///
    /// A second lock on the same door as [`crate::ship::helm_ai::ARTILLERY_PARAMS`]
    /// — the host-side gate — and worth having because the two fail at different
    /// moments: this one stops the hull existing at all, that one stops a hull
    /// that DOES load from flying a leg on a number nobody chose.
    ///
    /// The param lines are struck out of the TOML text rather than out of the
    /// parsed struct, because that is where the deletion would actually happen
    /// and because `to_policy()` alone does not re-validate references.
    #[test]
    fn harrow_warhawk_cannot_drop_a_guard_referenced_artillery_range() {
        for (omitted, line) in [
            (
                crate::ship::helm_ai::MAX_ARTILLERY_RANGE_PARAM,
                "max_artillery_range = 200.0",
            ),
            (
                crate::ship::helm_ai::ARTILLERY_HOLD_RANGE_PARAM,
                "artillery_hold_range = 180.0",
            ),
        ] {
            assert!(
                &harrow_warhawk_toml().contains(line),
                "precondition: the hull must author `{line}` for this to remove it"
            );
            let stripped = &harrow_warhawk_toml().replace(line, "");
            let err = EntityConfig::from_toml(stripped)
                .expect_err("a guard on an undeclared param must fail the entity load")
                .to_string();
            assert!(
                err.contains("undeclared parameter") && err.contains(omitted),
                "the hull without `{omitted}` must fail to load; got: {err}"
            );
        }
    }

    // ── The battleship's opportunistic close defence (issue #793) ────────────

    /// AC1, as content: the beam battery answers a player who has closed inside
    /// the artillery envelope with its WHOLE output, not half of it.
    ///
    /// #792 authored two 180-degree banks on ±90 facings. That covers the circle
    /// with no dead zone — but two half-planes only touch, they never overlap, so
    /// exactly one bank could bear at any real bearing and the hull's close-in
    /// output was 12 damage/s however it was engaged. The seam between them lies
    /// dead ahead, which is the one bearing an artillery platform holding a
    /// predictive lead solution keeps its target on.
    ///
    /// The ±30 assertions are the ones that discriminate, and they are why the
    /// test does not simply read the bow. A bearing of exactly 0 is admitted by
    /// BOTH banks under the old 180-degree authoring too — `in_arc` compares with
    /// `<=`, so the seam is a boundary tie and a fixture sitting on it proves
    /// nothing. Thirty degrees off the bow is inside the new overlap and outside
    /// the old one.
    #[test]
    fn harrow_warhawk_beams_double_up_across_the_bow_for_a_closing_player() {
        let cfg = EntityConfig::from_toml(&harrow_warhawk_toml()).expect("hull must parse");
        let wc = cfg
            .weapons_console
            .as_ref()
            .expect("hull declares [weapons_console]");
        let banks = &wc.phaser_banks;
        assert!(banks.len() > 1, "precondition: more than one beam bank");

        for bank in banks {
            assert!(
                (bank.auto_arc_deg - bank.fire_arc_deg).abs() < 1e-3,
                "bank '{}': the AUTO arc must reach as far as the fire arc ({} vs \
                 {}) — the overlap this hull's close defence lives in reaches to \
                 the edge of each bank's cone, and a narrower auto arc switches off \
                 exactly the cover the widening exists to create",
                bank.id,
                bank.auto_arc_deg,
                bank.fire_arc_deg
            );
        }

        // Beam damage available at one bearing, through the same `in_arc` the
        // auto-fire gate uses, summed over every bank that bears.
        let total_dps: f32 = banks.iter().map(|b| b.beam_damage_per_sec).sum();
        let dps_at = |deg: f32| -> f32 {
            let bearing = deg.to_radians();
            banks
                .iter()
                .filter(|b| {
                    crate::weapons::phaser::in_arc(
                        simmath::sin(bearing),
                        simmath::cos(bearing),
                        b.facing_deg,
                        b.auto_arc_deg,
                    )
                })
                .map(|b| b.beam_damage_per_sec)
                .sum()
        };

        // No dead zone anywhere — the property #792 already had, kept.
        for deg in -179..=180 {
            assert!(
                dps_at(deg as f32) > 0.0,
                "no bearing may be uncovered: {deg} degrees has no bank on it"
            );
        }

        // ...and the bow cone, which is where the hold puts a closing player,
        // gets the WHOLE battery rather than half of it.
        for deg in [-30.0_f32, 0.0, 30.0] {
            assert!(
                (dps_at(deg) - total_dps).abs() < 1e-3,
                "a target {deg} degrees off the bow must be engaged by every bank \
                 ({} of {total_dps} damage/s bears): the artillery hold holds a \
                 closing player on the centreline, so a battery split across it \
                 fights every engagement at half output",
                dps_at(deg)
            );
        }
        // The stern gets it too, which is the half the aft launcher works with.
        assert!(
            (dps_at(180.0) - total_dps).abs() < 1e-3,
            "the stern cone must be covered by every bank as well: a hull turning \
             at 0.20 rad/s cannot keep its nose on a close crosser"
        );

        // Close defence, and only close defence: the beams are for the player who
        // has come inside the gun line, and the gun line is authored much further
        // out. If a retune ever made them reach the holding radius, this hull is
        // no longer holding a standoff it cannot shoot into.
        let hold = cfg
            .helm_console
            .as_ref()
            .and_then(|hc| hc.steering_ai.as_ref())
            .expect("hull authors [helm_console.steering_ai]")
            .param[crate::ship::helm_ai::ARTILLERY_HOLD_RANGE_PARAM];
        for bank in banks {
            assert!(
                bank.beam_range < hold,
                "bank '{}' reaches {} units, at or beyond the {hold}-unit holding \
                 radius — 'close defence' means the player has to close for it",
                bank.id,
                bank.beam_range
            );
        }
    }

    /// AC2/AC3, as content: two opposed launchers, each gating on ITS OWN
    /// readiness, ITS OWN cone, and the arc ITS round would strike.
    ///
    /// The guard's choice of fact is the whole of AC2 and the one thing that
    /// cannot be read off behaviour alone, because the wrong fact fails silently:
    /// `fact(tubes_full)` is ship-wide (every tube at `volley_max`), which is
    /// right for the cruiser's committed salvo and wrong here — a loaded fore tube
    /// bearing on a collapsed arc would refuse the shot because the aft tube is
    /// eight seconds into a reload, and the two launchers would collapse into one.
    /// So the presence of `loaded` and the ABSENCE of `tubes_full` are both
    /// asserted.
    #[test]
    fn harrow_warhawk_carries_two_opposed_launchers_that_decide_independently() {
        let cfg = EntityConfig::from_toml(&harrow_warhawk_toml()).expect("hull must parse");
        let torpedoes = cfg
            .torpedoes
            .as_ref()
            .expect("the battleship carries close-defence launchers");

        let ids: Vec<&str> = torpedoes.tubes.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["fore", "aft"],
            "two launchers on OPPOSED facings: the fore tube answers the player \
             who closes down the gun line, the aft one the player who gets behind \
             a hull that cannot turn fast enough to stop them"
        );
        let fore = &torpedoes.tubes[0];
        let aft = &torpedoes.tubes[1];
        assert_eq!(fore.facing_deg, 0.0, "'fore' is a bow launcher");
        assert_eq!(aft.facing_deg, 180.0, "'aft' is a stern launcher");

        for tube in &torpedoes.tubes {
            assert_eq!(
                tube.volley_max, 1,
                "tube '{}' spends ONE round per opportunity — the opposite of the \
                 cruiser's committed salvo, and what makes the two launchers' \
                 reloads independent",
                tube.id
            );
            assert_eq!(
                tube.ai_target_count,
                Some(tube.volley_max),
                "an AI crew keeps tube '{}' loaded between opportunities: the \
                 reload ({} s) outlasts any window it could start inside",
                tube.id,
                torpedoes.load_time
            );
        }

        // The two cones must leave a REAL gap on each beam rather than meeting
        // there. A fore/aft pair whose cones touch has an arc boundary running
        // down each beam line, and `is_in_arc` admits a bearing sitting exactly on
        // it — so every "out of arc" fixture would pass vacuously, and the
        // armament would in truth cover every bearing, which is a turret and not
        // the opportunistic pair this doctrine authors.
        let covered = fore.fire_arc_deg * 0.5 + aft.fire_arc_deg * 0.5;
        assert!(
            covered < 180.0,
            "the fore ({}) and aft ({}) cones must leave the beams uncovered; \
             together they reach {covered} degrees off the centreline",
            fore.fire_arc_deg,
            aft.fire_arc_deg
        );

        // A round that arrives at a recovered arc must do NOTHING — which is why
        // the launch guard below gates on the arc being down instead of treating
        // the shield as something to shoot through.
        assert_eq!(
            torpedoes.damage_shields, 0,
            "these rounds go through a hole the beams made; they cannot make one"
        );
        assert!(
            torpedoes.damage_hull > 0,
            "and they hurt the hull once they are through"
        );

        // Reach. There is no range fact a launch guard can read — the host seeds
        // `in_range` as a constant `true` for every candidate — so a round's own
        // reach is the only thing deciding whether a shot taken at the far edge of
        // the gun line can arrive at all.
        let envelope = cfg
            .helm_console
            .as_ref()
            .and_then(|hc| hc.steering_ai.as_ref())
            .expect("hull authors [helm_console.steering_ai]")
            .param[crate::ship::helm_ai::MAX_ARTILLERY_RANGE_PARAM];
        let reach = torpedoes.speed * torpedoes.lifespan;
        assert!(
            reach >= envelope,
            "a round reaches {reach} units ({} x {} s) but the doctrine holds a \
             {envelope}-unit gun line: shots taken at the far edge would expire \
             short and drain the magazine for nothing",
            torpedoes.speed,
            torpedoes.lifespan
        );

        // The authored per-tube policy — all three of AC2/AC3's conditions, on the
        // launch channel, on EVERY tube, and none of them ship-wide.
        for tube in &torpedoes.tubes {
            let ai = tube
                .ai
                .as_ref()
                .unwrap_or_else(|| panic!("tube '{}' must author its own policy", tube.id));
            assert!(
                validate_fine_system_ai_policy(ai, TORPEDO_TUBE_CHANNELS, TORPEDO_TUBE_VERBS)
                    .is_ok(),
                "tube '{}' policy must pass content validation",
                tube.id
            );
            let load = ai
                .rule
                .iter()
                .find(|r| r.channel == TORPEDO_LOAD_CHANNEL)
                .unwrap_or_else(|| panic!("tube '{}' must author a load rule", tube.id));
            assert_eq!(load.verb, TORPEDO_LOAD_VERB);
            let launch = ai
                .rule
                .iter()
                .find(|r| r.channel == TORPEDO_LAUNCH_CHANNEL)
                .unwrap_or_else(|| panic!("tube '{}' must author a launch rule", tube.id));
            assert_eq!(launch.verb, TORPEDO_LAUNCH_VERB);
            for required in ["loaded", "target_facing_shields", "in_arc"] {
                assert!(
                    launch.when.contains(required),
                    "tube '{}': the launch guard must require `{required}` \
                     continuously, got `{}`",
                    tube.id,
                    launch.when
                );
            }
            assert!(
                !launch.when.contains("tubes_full"),
                "tube '{}': the launch guard must NOT gate on the SHIP-WIDE \
                 `tubes_full` — with it, a loaded launcher bearing on a downed arc \
                 holds fire because the OTHER launcher is reloading, which is the \
                 exact opposite of AC2's independence. Got `{}`",
                tube.id,
                launch.when
            );
        }

        // Fine systems: one per tube plus the shared magazine. Both the loader and
        // the launcher gate on the magazine before they look at a tube, so its
        // absence switches the whole armament off silently; a missing tube entry
        // leaves that one launcher unloadable, which is the half-battery
        // degradation the per-tube guard above exists to prevent.
        let ship_config = cfg.ship_config.as_ref().expect("hull declares systems");
        let declared =
            |id: &crate::messages::SystemId| ship_config.systems.iter().any(|s| &s.id == id);
        assert!(
            declared(&crate::system_registry::torpedo_magazine_system_id()),
            "the shared magazine needs a [[system]] entry or neither loading nor \
             launching runs at all"
        );
        for tube in &torpedoes.tubes {
            let expected = crate::system_registry::torpedo_tube_system_id(&tube.id)
                .expect("a non-empty tube id always resolves");
            assert!(
                declared(&expected),
                "tube '{}' must declare a [[system]] entry `{}`",
                tube.id,
                expected.0
            );
        }
    }

    /// AC4, as content: arming the hull changed nothing about how it points.
    ///
    /// The torpedo path is launcher-side from end to end — `ai_torpedo_auto_fire`
    /// only ever emits `FireTorpedo` at a tube's own system id, and nothing in it
    /// writes `ShipPhysics.yaw` or reaches the helm — so AC4 is satisfied by
    /// OMISSION, and an omission is exactly the kind of thing a later edit fills
    /// in helpfully. This is the lock: the travel axes may not acquire a torpedo
    /// leg, a torpedo param, or a torpedo-guarded transition, and the hold must
    /// still answer with the artillery verb.
    ///
    /// The cruiser is the counter-example that makes the assertion worth writing:
    /// it authors a whole `torpedo_run` state and a `torpedo_bearing_speed`, and
    /// copying that shape here would silently trade the predictive bow-artillery
    /// facing for one aimed at where the target IS.
    #[test]
    fn harrow_warhawk_close_defence_adds_no_steering_content() {
        let cfg = EntityConfig::from_toml(&harrow_warhawk_toml()).expect("hull must parse");
        assert!(
            cfg.torpedoes.as_ref().is_some_and(|t| !t.tubes.is_empty()),
            "precondition: the hull carries launchers, or this proves nothing"
        );
        let hc = cfg.helm_console.as_ref().unwrap();

        for (name, ai) in [
            ("engines_ai", hc.engines_ai.as_ref().unwrap()),
            ("steering_ai", hc.steering_ai.as_ref().unwrap()),
        ] {
            let ids: Vec<&str> = ai.state.iter().map(|s| s.id.as_str()).collect();
            assert_eq!(
                ids,
                vec!["shadow", "acquire", "reposition", "hold"],
                "{name} must still be the three-state artillery machine: the \
                 launchers take the bearing the gun line gives them and never ask \
                 for one"
            );
            for param in ai.param.keys() {
                assert!(
                    !param.contains("torpedo"),
                    "{name} authors a torpedo scalar `{param}`: the tubes are \
                     opportunistic and have no throttle or bearing of their own"
                );
            }
            for state in &ai.state {
                for rule in &state.rule {
                    assert!(
                        !rule.verb.contains("torpedo") && !rule.when.contains("torpedo"),
                        "{name} state '{}' answers a channel with torpedo content \
                         (`{}` / `{}`)",
                        state.id,
                        rule.when,
                        rule.verb
                    );
                }
                for transition in &state.transition {
                    assert!(
                        !transition.when.contains("torpedo"),
                        "{name} state '{}' guards a transition on a torpedo reading \
                         (`{}`): a launcher may never become a leg",
                        state.id,
                        transition.when
                    );
                }
            }
        }

        // The verb the whole doctrine turns on, unchanged — and the cruiser's
        // bow-hold scalars still absent, so the host cannot publish that leg for
        // this hull even if a state were added.
        let steering = hc.steering_ai.as_ref().unwrap();
        let hold = steering
            .state
            .iter()
            .find(|s| s.id == "hold")
            .expect("steering_ai declares 'hold'");
        assert_eq!(
            hold.rule[0].verb, HELM_HOLD_ARTILLERY_POSITION_VERB,
            "the firing position must still be aimed by the PREDICTIVE artillery \
             verb, not by anything the launchers wanted"
        );
        for absent in crate::ship::helm_ai::TORPEDO_BEARING_PARAMS {
            assert!(
                !steering.param.contains_key(*absent),
                "steering_ai must not author `{absent}`: it is the cruiser's \
                 bow-hold scalar, and the host gates that whole leg on the name \
                 being present"
            );
        }
    }

    /// AC5, as content: nothing in the close-defence armament can shove the hull
    /// off the position it is holding.
    ///
    /// Only one mechanism in the whole path could: `recoil_impulse`, which
    /// `handle_fire_blaster` adds straight onto `ShipPhysics.forward_speed` when
    /// it is positive. Phaser beams have no recoil mechanic and a torpedo launch
    /// never writes physics at all, so the blaster banks are the entire surface —
    /// and #792 authored the artillery piece without one only by leaving the field
    /// off, which is a default rather than a decision until something says so.
    #[test]
    fn harrow_warhawk_close_defence_cannot_shove_it_off_the_firing_position() {
        let cfg = EntityConfig::from_toml(&harrow_warhawk_toml()).expect("hull must parse");
        let banks = &cfg.weapons_console.as_ref().unwrap().blaster_banks;
        assert!(!banks.is_empty(), "precondition: the hull mounts a blaster");
        for bank in banks {
            assert_eq!(
                bank.recoil_impulse, 0.0,
                "bank '{}' authors a recoil impulse ({}): it is added straight onto \
                 `forward_speed` at fire time, so an artillery platform firing one \
                 would walk itself off the gun line it just spent a run-in taking up",
                bank.id, bank.recoil_impulse
            );
        }

        // The other half of "holds station": the hold's own throttle. Restated
        // here because AC5 is about the whole close-defence path, and a non-zero
        // throttle would give ground to a closing player for a different reason.
        let steering = cfg
            .helm_console
            .as_ref()
            .and_then(|hc| hc.steering_ai.as_ref())
            .unwrap();
        assert_eq!(
            steering.param[crate::ship::helm_ai::ARTILLERY_HOLD_SPEED_PARAM],
            0.0,
            "the held throttle must stay zero: a player who closes cannot make \
             this ship give ground"
        );
    }

    #[test]
    fn station_axiom_template_has_explicit_disc_collider() {
        // (#474) Explicit collider for robust hit detection.
        //
        // Both numbers come off the hull the station is DRAWN as, at John's
        // request that collision match visible size. `alliance_starbase.glb`
        // measures 1.8973 x 0.7958 x 1.8936 raw, and the [15, 18, 18] its
        // sidecar applies draws 28.46 x 14.33 x 34.08 — so the widest half-extent
        // is 17.04 and the drawn half-height is 7.16.
        //
        // The shape is what this test now exists to hold. A Ball at 17.04 was
        // right about the width and wrong about the height by a factor of two
        // and a bit; the 12.0 before it was wrong about both. Only a Cylinder
        // can carry the two independently, so a regression to EITHER of those is
        // a regression to a body the renderer does not draw.
        let toml_str = include_str!("../../assets/entities/station_axiom.toml");
        let config = EntityConfig::from_toml(toml_str).expect("station_axiom.toml must parse");
        let collider = config
            .collider
            .as_ref()
            .expect("station_axiom must have explicit [collider] (#474)");
        assert_eq!(collider.shape, ColliderShape::Cylinder);
        assert!(
            (collider.radius - 17.04).abs() < 1e-6,
            "expected the starbase hull's max half-extent, got {}",
            collider.radius
        );
        assert!(
            (collider
                .half_height
                .expect("a Cylinder must author a half-height")
                - 7.16)
                .abs()
                < 1e-6,
            "expected half the starbase hull's drawn height (14.325 / 2), got {:?}",
            collider.half_height
        );
    }

    /// A mesh's users must agree with each other: two bodies of different sizes
    /// cannot both be the thing one GLB draws. That is what let `skyhook` carry
    /// a 26 while `station_axiom` carried a 12 off the same starbase model.
    ///
    /// WALKED, not listed. `assets/entities/` is enumerated and every template
    /// whose `[mesh].model` is one of the two station GLBs is checked, so a
    /// SIXTH user arrives already covered rather than waiting for someone to
    /// remember this test. A hard-coded list would not have caught the fifth
    /// one that already exists — `station_research_outpost.toml` draws
    /// `alliance_research_outpost.glb` and authors no `[collider]` at all.
    ///
    /// Which is the one exemption, and it is deliberate rather than an
    /// oversight being papered over: a template with no `[collider]` collides
    /// with nothing, so it has no body to disagree about. It is a real gap in
    /// that template — a station ships fly straight through — but it is a
    /// DIFFERENT gap from this one, it predates the correction, and inventing a
    /// collider for a template no world spawns is not this change's business.
    /// The walk counts it separately so the exemption stays visible.
    #[test]
    fn every_station_mesh_user_authors_the_disc_its_mesh_draws() {
        fn templates_under(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            let entries = std::fs::read_dir(dir)
                .unwrap_or_else(|e| panic!("{} must be readable: {e}", dir.display()));
            let mut paths: Vec<std::path::PathBuf> = entries.flatten().map(|e| e.path()).collect();
            paths.sort();
            for path in paths {
                if path.is_dir() {
                    if path.file_name().is_some_and(|n| n == "fragments") {
                        continue;
                    }
                    templates_under(&path, out);
                } else if path.extension().and_then(|e| e.to_str()) == Some("toml") {
                    out.push(path);
                }
            }
        }

        // (model, radius, half_height) — radius is the widest half-extent of the
        // drawn hull and half_height is half its drawn height, both read off the
        // model's own rig sidecar `[extents].size`.
        let expected: [(&str, f32, f32); 2] = [
            ("assets/models/alliance_starbase.glb", 17.04, 7.16),
            ("assets/models/alliance_research_outpost.glb", 3.8, 1.68),
        ];

        let mut templates = Vec::new();
        templates_under(std::path::Path::new("assets/entities"), &mut templates);
        assert!(!templates.is_empty(), "no templates found");

        let (mut checked, mut colliderless) = (0, 0);
        for path in templates {
            let key = path.to_string_lossy().replace('\\', "/");
            let cfg = crate::entity_includes::load_entity_config(&key)
                .unwrap_or_else(|e| panic!("{key} must parse: {e}"));
            let Some(model) = cfg.mesh.as_ref().and_then(|m| m.model.as_deref()) else {
                continue;
            };
            let Some(&(_, radius, half_height)) = expected.iter().find(|(m, ..)| *m == model)
            else {
                continue;
            };
            let Some(collider) = cfg.collider.as_ref() else {
                colliderless += 1;
                continue;
            };
            checked += 1;
            assert_eq!(
                collider.shape,
                ColliderShape::Cylinder,
                "{key} draws {model}, so its collider must be the disc that mesh draws"
            );
            assert!(
                (collider.radius - radius).abs() < 1e-6,
                "{key}: expected radius {radius}, got {}",
                collider.radius
            );
            assert!(
                (collider
                    .half_height
                    .expect("a Cylinder authors a half-height")
                    - half_height)
                    .abs()
                    < 1e-6,
                "{key}: expected half_height {half_height}, got {:?}",
                collider.half_height
            );
        }
        // Four users with colliders (station_axiom, skyhook, depot_transfer,
        // station_outpost) and one without (station_research_outpost). Pinned so
        // a walk that silently stopped matching anything cannot pass vacuously,
        // and so a new colliderless station-mesh user is a visible edit here.
        assert_eq!(
            checked, 4,
            "expected four station-mesh users with colliders"
        );
        assert_eq!(
            colliderless, 1,
            "expected exactly one station-mesh user with no collider at all \
             (station_research_outpost); a second is a new gap, not this test's \
             exemption"
        );
    }

    #[test]
    fn ship_harrow_patrol_template_has_doctrine_objectives() {
        // (#572) FSM dissolved — NPC hulls use doctrine-based AI. Expects a
        // Patrol objective (sector sweep) and a higher-priority Destroy
        // objective (engage hostiles on sight).
        //
        // (#892) Re-pointed off the retired `pirate_raider.toml`. The Ironveil
        // rather than the Harrow Destroyer, because the Destroyer authors a
        // Destroy entry ONLY — it has no Patrol doctrine for the priority
        // ordering here to compare against, and after #892 the Ironveil is the
        // shipped hull that still carries both.
        let toml_str = &resolved_text("ship_harrow_patrol");
        let config = EntityConfig::from_toml(toml_str).expect("ship_harrow_patrol.toml must parse");
        let behaviour = config
            .behaviour
            .expect("the Ironveil must have a [behaviour] block");
        let ids: Vec<&str> = behaviour.doctrine.iter().map(|d| d.id.as_str()).collect();
        assert!(
            ids.contains(&"patrol-ironveil"),
            "must have patrol-ironveil doctrine"
        );
        assert!(
            ids.contains(&"destroy-hostiles"),
            "must have destroy-hostiles doctrine"
        );
        let destroy = behaviour
            .doctrine
            .iter()
            .find(|d| d.id == "destroy-hostiles")
            .unwrap();
        let patrol = behaviour
            .doctrine
            .iter()
            .find(|d| d.id == "patrol-ironveil")
            .unwrap();
        assert!(
            destroy.base_priority > patrol.base_priority,
            "destroy-hostiles must outscore patrol-ironveil"
        );
    }

    #[test]
    fn harrow_destroyer_doctrine_destroy_has_correct_directive_kind() {
        // (#572) FSM transitions dissolved — engagement logic now lives in the
        // utility scorer. Verify the destroy-hostiles objective carries the
        // Destroy directive kind so `ai_target_selection` picks it up.
        // (#892) Re-pointed off the retired `pirate_raider.toml`.
        let toml_str = &resolved_text("ship_harrow_destroyer");
        let config =
            EntityConfig::from_toml(toml_str).expect("ship_harrow_destroyer.toml must parse");
        let behaviour = config.behaviour.expect("behaviour must be Some");
        let destroy = behaviour
            .doctrine
            .iter()
            .find(|d| d.id == "destroy-hostiles")
            .expect("destroy-hostiles doctrine must be present");
        assert_eq!(
            destroy.directive_kind.as_deref(),
            Some("Destroy"),
            "destroy-hostiles must carry directive_kind = 'Destroy'"
        );
    }

    // ── validate_doctrine_directives ───────────────────────────────────────

    /// A doctrine-only fixture. Lenient: the subject is directive validation, and
    /// a bare `[behaviour]` snippet is not a hull — see
    /// [`EntityConfig::from_toml_in_mode`].
    fn doctrine_toml(body: &str) -> Result<EntityConfig, String> {
        EntityConfig::from_toml_in_mode(
            &format!("[behaviour]\n\n[[behaviour.doctrine]]\n{body}"),
            crate::entities::ai_declaration_manifest::AiDeclarationMode::Lenient,
        )
        .map_err(|e| e.to_string())
    }

    #[test]
    fn reach_directive_with_plural_patrol_anchors_is_rejected() {
        let err = doctrine_toml(
            r#"
id = "reach-destination"
directive_kind = "Reach"
directive_anchors = ["destination"]
"#,
        )
        .unwrap_err();
        assert!(
            err.contains("directive_anchors") && err.contains("Reach"),
            "the error must name the wrong field and the directive kind: {err}"
        );
    }

    #[test]
    fn reach_directive_without_an_anchor_is_rejected() {
        let err = doctrine_toml(
            r#"
id = "reach-destination"
directive_kind = "Reach"
"#,
        )
        .unwrap_err();
        assert!(
            err.contains("directive_anchor"),
            "the error must name the missing field: {err}"
        );
    }

    #[test]
    fn patrol_directive_with_singular_reach_anchor_is_rejected() {
        let err = doctrine_toml(
            r#"
id = "patrol-sector"
directive_kind = "Patrol"
directive_anchor = "alpha"
"#,
        )
        .unwrap_err();
        assert!(err.contains("directive_anchor"), "{err}");
    }

    #[test]
    fn destroy_directive_with_an_anchor_is_rejected() {
        let err = doctrine_toml(
            r#"
id = "destroy-hostiles"
directive_kind = "Destroy"
directive_anchor = "somewhere"
"#,
        )
        .unwrap_err();
        assert!(
            err.contains("directive_anchor") && err.contains("Destroy"),
            "{err}"
        );
    }

    #[test]
    fn directive_field_without_a_directive_kind_is_rejected() {
        let err = doctrine_toml(
            r#"
id = "hold-station"
directive_target = "Starbase Alpha"
"#,
        )
        .unwrap_err();
        assert!(err.contains("directive_target"), "{err}");
    }

    #[test]
    fn unknown_directive_kind_is_rejected() {
        let err = doctrine_toml(
            r#"
id = "wander"
directive_kind = "Wander"
"#,
        )
        .unwrap_err();
        assert!(err.contains("Wander"), "{err}");
    }

    /// The shapes every shipped hull and world override actually authors.
    #[test]
    fn well_formed_directives_of_every_kind_are_accepted() {
        for body in [
            "id = \"hold-station\"\nbase_priority = 20.0",
            "id = \"destroy-hostiles\"\ndirective_kind = \"Destroy\"",
            "id = \"assault\"\ndirective_kind = \"Destroy\"\ndirective_target = \"Starbase Alpha\"",
            "id = \"patrol\"\ndirective_kind = \"Patrol\"\ndirective_anchors = [\"a\", \"b\"]\ndirective_loop = true",
            "id = \"reach\"\ndirective_kind = \"Reach\"\ndirective_anchor = \"home\"",
            "id = \"retreat\"\ndirective_kind = \"Retreat\"\ndirective_anchor = \"haven\"",
            "id = \"hail\"\ndirective_kind = \"Hail\"\ndirective_hail_target = \"Axiom Station\"",
        ] {
            assert!(
                doctrine_toml(body).is_ok(),
                "well-formed doctrine must load: {body}"
            );
        }
    }

    /// Regression: the courier's only goal is a `Reach`, and it must name the
    /// singular anchor field or the directive resolves to `""` and never fires.
    #[test]
    fn requiem_courier_reach_directive_names_a_singular_anchor() {
        let toml_str = include_str!("../../assets/entities/ship_requiem_courier.toml");
        let config =
            EntityConfig::from_toml(toml_str).expect("ship_requiem_courier.toml must parse");
        let behaviour = config.behaviour.expect("behaviour must be Some");
        let reach = behaviour
            .doctrine
            .iter()
            .find(|d| d.id == "reach-destination")
            .expect("reach-destination doctrine must be present");
        assert_eq!(reach.directive_kind.as_deref(), Some("Reach"));
        assert_eq!(
            reach.directive_anchor.as_deref(),
            Some("requiem_courier_destination"),
            "Reach reads `directive_anchor`; the plural Patrol field is ignored"
        );
    }

    // ── [torpedoes] block tests ────────────────────────────────────────────

    #[test]
    fn torpedoes_block_full_round_trips() {
        let toml_str = r##"
[torpedoes]
count = 12
damage_hull = 60
damage_shields = 7
speed = 35.0
turn_rate_deg_per_sec = 90.0
lifespan = 25.0
load_time = 8.0
"##;
        let config = EntityConfig::from_toml_in_mode(
            toml_str,
            crate::entities::ai_declaration_manifest::AiDeclarationMode::Lenient,
        )
        .expect("parse must succeed");
        let t = config.torpedoes.expect("torpedoes must be Some");
        assert_eq!(t.count, 12);
        assert_eq!(t.damage_hull, 60);
        assert_eq!(t.damage_shields, 7);
        assert_eq!(t.speed, 35.0);
        assert_eq!(t.turn_rate_deg_per_sec, 90.0);
        assert_eq!(t.lifespan, 25.0);
        assert_eq!(t.load_time, 8.0);
    }

    #[test]
    fn torpedoes_block_absent_yields_none() {
        let config = EntityConfig::from_toml("").expect("parse must succeed");
        assert!(config.torpedoes.is_none());
    }

    #[test]
    fn torpedoes_block_partial_keeps_defaults_for_missing_fields() {
        let toml_str = r##"
[torpedoes]
count = 99
"##;
        let config = EntityConfig::from_toml_in_mode(
            toml_str,
            crate::entities::ai_declaration_manifest::AiDeclarationMode::Lenient,
        )
        .expect("parse must succeed");
        let t = config.torpedoes.expect("torpedoes must be Some");
        assert_eq!(t.count, 99, "override applied");
        assert_eq!(t.damage_hull, 50, "default preserved");
        assert_eq!(t.damage_shields, 5, "default preserved");
        assert_eq!(t.speed, 15.0, "default preserved");
        assert_eq!(t.turn_rate_deg_per_sec, 45.0, "default preserved");
        assert_eq!(t.lifespan, 20.0, "default preserved");
        assert_eq!(t.load_time, 10.0, "default preserved");
    }

    #[test]
    fn torpedoes_to_runtime_converts_degrees_to_radians() {
        let mut t = TorpedoesConfig::default();
        t.turn_rate_deg_per_sec = 45.0;
        let rt = t.to_runtime();
        assert!(
            (rt.turn_rate - std::f32::consts::FRAC_PI_4).abs() < 1e-5,
            "45 deg/s should convert to PI/4 rad/s, got {}",
            rt.turn_rate
        );
        assert_eq!(rt.count, 10);
        assert_eq!(rt.damage_hull, 50);
        assert_eq!(rt.load_time, 10.0);
    }

    #[test]
    fn torpedoes_defaults_match_runtime_torpedo_config_default() {
        let toml_default = TorpedoesConfig::default().to_runtime();
        let runtime_default = crate::torpedo::TorpedoConfig::default();
        assert_eq!(toml_default.count, runtime_default.count);
        assert_eq!(toml_default.damage_hull, runtime_default.damage_hull);
        assert_eq!(toml_default.damage_shields, runtime_default.damage_shields);
        assert_eq!(toml_default.speed, runtime_default.speed);
        assert!((toml_default.turn_rate - runtime_default.turn_rate).abs() < 1e-5);
        assert_eq!(toml_default.lifespan, runtime_default.lifespan);
        assert_eq!(toml_default.load_time, runtime_default.load_time);
    }

    #[test]
    fn battleship_toml_torpedoes_block_parses_correctly() {
        // Verify the [torpedoes] block in alliance_battleship.toml parses
        // and produces the expected runtime values.
        let config = shipped_hull("alliance_battleship");
        let t = config
            .torpedoes
            .expect("alliance_battleship must have [torpedoes]");
        let rt = t.to_runtime();
        // Values from alliance_battleship.toml [torpedoes] block
        assert_eq!(rt.count, 30, "battleship magazine size");
        assert_eq!(rt.damage_hull, 40);
        assert_eq!(rt.damage_shields, 4);
        assert_eq!(rt.speed, 15.0);
        assert!((rt.turn_rate - (45f32).to_radians()).abs() < 1e-5);
        assert_eq!(rt.lifespan, 20.0);
        assert_eq!(rt.load_time, 10.0);
    }

    /// Issue #942: the player destroyer's two launchers spend SMALL volleys.
    ///
    /// The tube COUNT was never the lever and is not what moved — this hull has
    /// always carried exactly one fore and one aft launcher, matched by its
    /// `torpedo-tube-fore` / `torpedo-tube-aft` hull entries. What moved is the
    /// volley: fore 4 -> 2, aft 2 -> 1. At the old sizes six rounds of a
    /// twelve-round magazine sat in the tubes and a single bearing could spend
    /// all six, so wave one met the whole payload and every wave after it met a
    /// hull with nothing to launch.
    ///
    /// This is authored content with no other guard on it: the sizes could drift
    /// back up, or the two tubes could even out, and the hull would still parse,
    /// still launch, and still pass every other test here. Hence the pin, and
    /// hence it pins the ORDERING too — the fore tube is the one whose cone the
    /// attack-pass doctrine actually brings to bear, so it is the tube that
    /// fires a pair.
    #[test]
    fn the_player_destroyer_launchers_fire_small_volleys() {
        let config = shipped_hull("alliance_destroyer");
        let t = config
            .torpedoes
            .as_ref()
            .expect("the player destroyer carries torpedo tubes");

        let ids: Vec<&str> = t.tubes.iter().map(|tube| tube.id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["fore", "aft"],
            "one launcher on each end and no more: a third tube would restore the \
             per-opportunity payload this hull just gave up"
        );
        let fore = &t.tubes[0];
        let aft = &t.tubes[1];
        assert_eq!(fore.facing_deg, 0.0, "'fore' is a bow launcher");
        assert_eq!(aft.facing_deg, 180.0, "'aft' is a stern launcher");

        assert_eq!(
            fore.volley_max, 2,
            "the bow launcher spends a PAIR per opportunity — the arc the attack \
             pass brings to bear is the one worth two rounds"
        );
        assert_eq!(
            aft.volley_max, 1,
            "the stern launcher spends ONE: a pair spent on whoever got behind is \
             a pair the bow tube does not have for the pass it is flying"
        );
        assert!(
            aft.volley_max < fore.volley_max,
            "the two launchers must stay asymmetric, or 'which tube is worth \
             loading' stops being a decision"
        );

        // Neither tube authors `ai_target_count` and the hull authors no
        // ship-wide `ai_volley_target`, so an AI backfill parks each tube at its
        // own `volley_max`: 3 rounds of the 12-round magazine, not 6. A future
        // `ai_target_count` above `volley_max` would clamp, but one BELOW it
        // would quietly disarm the backfilled hull relative to the human crew,
        // which #838's symmetry does not allow.
        let parked: u32 = t
            .tubes
            .iter()
            .map(|tube| {
                tube.ai_target_count
                    .or(t.ai_volley_target)
                    .unwrap_or(tube.volley_max)
                    .min(tube.volley_max)
            })
            .sum();
        assert_eq!(
            parked, 3,
            "an AI crew must keep both tubes at their authored volleys ({parked} \
             rounds parked); a human crew can ask for the same 3 and no more"
        );
        assert!(
            parked * 3 <= t.count,
            "a full load ({parked}) must stay a small fraction of the {}-round \
             magazine — the magazine is what makes a launch a decision",
            t.count
        );
    }

    // ── [repair] block tests ───────────────────────────────────────────────

    #[test]
    fn repair_block_full_round_trips() {
        let toml_str = r##"
[repair]
travel_duration_secs = 7.5
repair_rate_hp_per_sec = 1.25
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let r = config.repair.expect("repair must be Some");
        assert_eq!(r.travel_duration_secs, 7.5);
        assert_eq!(r.repair_rate_hp_per_sec, 1.25);
    }

    #[test]
    fn repair_block_absent_yields_none() {
        let config = EntityConfig::from_toml("").expect("parse must succeed");
        assert!(config.repair.is_none());
    }

    #[test]
    fn repair_block_partial_keeps_defaults_for_missing_fields() {
        let toml_str = r##"
[repair]
travel_duration_secs = 9.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let r = config.repair.expect("repair must be Some");
        assert_eq!(r.travel_duration_secs, 9.0, "override applied");
        assert_eq!(r.repair_rate_hp_per_sec, 0.5, "default preserved");
    }

    #[test]
    fn repair_to_runtime_preserves_values() {
        let r = RepairConfig {
            travel_duration_secs: 3.0,
            repair_rate_hp_per_sec: 2.0,
            ..Default::default()
        };
        let rt = r.to_runtime();
        assert_eq!(rt.travel_duration, 3.0);
        assert_eq!(rt.repair_rate_hp_per_sec, 2.0);
    }

    #[test]
    fn repair_defaults_match_runtime_repair_timings_default() {
        let toml_default = RepairConfig::default().to_runtime();
        let runtime_default = crate::repair_teams::RepairTimings::default();
        assert_eq!(
            toml_default.travel_duration,
            runtime_default.travel_duration
        );
        assert_eq!(
            toml_default.repair_rate_hp_per_sec,
            runtime_default.repair_rate_hp_per_sec
        );
    }

    #[test]
    fn battleship_toml_repair_block_matches_runtime_default_values() {
        // Drift guard: if the [repair] block in alliance_battleship.toml ever diverges
        // from RepairTimings::default(), this test fails so the owner can
        // confirm the change is intentional. (The defaults themselves match
        // the historical hardcoded constants in `repair_teams.rs`.)
        let config = shipped_hull("alliance_battleship");
        let r = config
            .repair
            .expect("alliance_battleship must have [repair]");
        let rt = r.to_runtime();
        let baseline = crate::repair_teams::RepairTimings::default();
        assert_eq!(
            rt.travel_duration, baseline.travel_duration,
            "travel duration drift"
        );
        assert_eq!(
            rt.repair_rate_hp_per_sec, baseline.repair_rate_hp_per_sec,
            "repair rate drift"
        );
    }

    // ── [shields_console.base] block tests ────────────────────────────────

    #[test]
    fn shields_console_base_block_full_round_trips() {
        let toml_str = r##"
[shields_console]

[shields_console.base]
num_facings = 6
max_hp = 200
regen_per_sec = 7.5
offline_duration = 12.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let sc = config
            .shields_console
            .expect("shields_console must be Some");
        let base = sc.base.expect("base sub-block must be Some");
        assert_eq!(base.num_facings, 6);
        assert_eq!(base.max_hp, 200);
        assert_eq!(base.regen_per_sec, 7.5);
        assert_eq!(base.offline_duration, 12.0);
    }

    #[test]
    fn shields_console_without_base_subblock_yields_none() {
        // The flat focus fields parse fine; absent `[shields_console.base]`
        // must produce `base: None` so the runtime falls back to
        // `ShieldConfig::default()`.
        let toml_str = r##"
[shields_console]
focus_bonus_max_hp = 99
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let sc = config
            .shields_console
            .expect("shields_console must be Some");
        assert!(
            sc.base.is_none(),
            "base sub-block must default to None when absent"
        );
        assert_eq!(sc.focus_bonus_max_hp, 99, "flat focus field still parses");
    }

    #[test]
    fn shields_base_block_partial_keeps_defaults_for_missing_fields() {
        let toml_str = r##"
[shields_console.base]
max_hp = 250
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let base = config
            .shields_console
            .expect("shields_console")
            .base
            .expect("base");
        assert_eq!(base.max_hp, 250, "override applied");
        assert_eq!(base.num_facings, 4, "default preserved");
        assert_eq!(base.regen_per_sec, 2.0, "default preserved");
        assert_eq!(base.offline_duration, 10.0, "default preserved");
    }

    #[test]
    fn shields_base_to_runtime_preserves_values() {
        let base = ShieldsBaseConfig {
            num_facings: 3,
            max_hp: 75,
            regen_per_sec: 2.5,
            offline_duration: 8.0,
        };
        let rt = base.to_runtime();
        assert_eq!(rt.num_facings, 3);
        assert_eq!(rt.max_hp, 75);
        assert_eq!(rt.regen_per_sec, 2.5);
        assert_eq!(rt.offline_duration, 8.0);
    }

    #[test]
    fn shields_base_defaults_match_runtime_shield_config_default() {
        let toml_default = ShieldsBaseConfig::default().to_runtime();
        let runtime_default = crate::shield::ShieldConfig::default();
        assert_eq!(toml_default.num_facings, runtime_default.num_facings);
        assert_eq!(toml_default.max_hp, runtime_default.max_hp);
        assert_eq!(toml_default.regen_per_sec, runtime_default.regen_per_sec);
        assert_eq!(
            toml_default.offline_duration,
            runtime_default.offline_duration
        );
    }

    #[test]
    fn battleship_toml_shields_base_block_parses_correctly() {
        // Verify the [shields_console.base] block in alliance_battleship.toml
        // parses and produces the expected runtime values.
        let config = shipped_hull("alliance_battleship");
        let base = config
            .shields_console
            .expect("alliance_battleship must have [shields_console]")
            .base
            .expect("alliance_battleship must have [shields_console.base]");
        let rt = base.to_runtime();
        // Values from alliance_battleship.toml [shields_console.base] block
        assert_eq!(rt.max_hp, 140, "battleship shield facing max_hp");
        assert_eq!(rt.regen_per_sec, 3.5, "battleship shield regen");
        assert_eq!(rt.offline_duration, 10.0, "offline duration");
    }

    // ── PhaserCombatConfig (player phaser tuning) tests ───────────────────
    //
    // PhaserCombatConfig is built from the per-bank fields on
    // [[weapons_console.phaser_banks]]. All combat tuning is per-bank.

    #[test]
    fn phaser_combat_config_from_weapons_console_clones_banks() {
        let toml_str = r##"
[weapons_console]

[[weapons_console.phaser_banks]]
id = "fore"
facing_deg = 0.0
fire_arc_deg = 180.0
auto_arc_deg = 180.0
beam_range = 99.0
beam_damage_per_sec = 12.0
beam_duration_secs = 4.0
cooldown_secs = 7.5
"##;
        let config = EntityConfig::from_toml_in_mode(
            toml_str,
            crate::entities::ai_declaration_manifest::AiDeclarationMode::Lenient,
        )
        .expect("parse must succeed");
        let wc = config.weapons_console.expect("weapons_console");
        let combat = PhaserCombatConfig::from_weapons_console(&wc);
        assert_eq!(combat.banks.len(), 1);
        assert_eq!(combat.banks[0].beam_range, 99.0);
        assert_eq!(combat.banks[0].beam_damage_per_sec, 12.0);
        assert_eq!(combat.banks[0].beam_duration_secs, 4.0);
        assert_eq!(combat.banks[0].cooldown_secs, 7.5);
    }

    #[test]
    fn phaser_combat_config_default_has_empty_banks() {
        let combat = PhaserCombatConfig::default();
        assert!(combat.banks.is_empty());
    }

    // ── PhaserBankConfig / TorpedoTubeConfig schema tests (Phase A) ───────

    #[test]
    fn phaser_banks_array_parses_full_entries() {
        let toml_str = r##"
[weapons_console]

[[weapons_console.phaser_banks]]
id = "port"
facing_deg = -90.0
fire_arc_deg = 180.0
auto_arc_deg = 120.0
beam_range = 35.0

[[weapons_console.phaser_banks]]
id = "starboard"
facing_deg = 90.0
fire_arc_deg = 180.0
auto_arc_deg = 120.0
"##;
        let config = EntityConfig::from_toml_in_mode(
            toml_str,
            crate::entities::ai_declaration_manifest::AiDeclarationMode::Lenient,
        )
        .expect("parse must succeed");
        let wc = config.weapons_console.expect("weapons_console");
        assert_eq!(wc.phaser_banks.len(), 2);
        assert_eq!(wc.phaser_banks[0].id, "port");
        assert_eq!(wc.phaser_banks[0].facing_deg, -90.0);
        assert_eq!(wc.phaser_banks[0].fire_arc_deg, 180.0);
        assert_eq!(wc.phaser_banks[0].auto_arc_deg, 120.0);
        assert_eq!(wc.phaser_banks[0].beam_range, 35.0);
        assert_eq!(wc.phaser_banks[1].id, "starboard");
        assert_eq!(
            wc.phaser_banks[1].beam_range, 0.0,
            "missing beam_range defaults to 0 (caller falls back to parent)"
        );
    }

    #[test]
    fn phaser_bank_shield_pierce_defaults_to_none_when_absent() {
        let toml_str = r##"
[weapons_console]

[[weapons_console.phaser_banks]]
id = "port"
facing_deg = -90.0
fire_arc_deg = 180.0
auto_arc_deg = 120.0
"##;
        let config = EntityConfig::from_toml_in_mode(
            toml_str,
            crate::entities::ai_declaration_manifest::AiDeclarationMode::Lenient,
        )
        .expect("parse must succeed");
        let wc = config.weapons_console.expect("weapons_console");
        assert_eq!(wc.phaser_banks[0].shield_pierce, None);
    }

    #[test]
    fn phaser_bank_shield_pierce_parses_when_present() {
        let toml_str = r##"
[weapons_console]

[[weapons_console.phaser_banks]]
id = "port"
facing_deg = -90.0
fire_arc_deg = 180.0
auto_arc_deg = 120.0
shield_pierce = 0.6
"##;
        let config = EntityConfig::from_toml_in_mode(
            toml_str,
            crate::entities::ai_declaration_manifest::AiDeclarationMode::Lenient,
        )
        .expect("parse must succeed");
        let wc = config.weapons_console.expect("weapons_console");
        assert_eq!(wc.phaser_banks[0].shield_pierce, Some(0.6));
    }

    #[test]
    fn torpedoes_shield_pierce_defaults_to_zero() {
        let toml_str = r##"
[torpedoes]
count = 5

[[torpedoes.tubes]]
id = "fore"
facing_deg = 0.0
fire_arc_deg = 90.0
"##;
        let config = EntityConfig::from_toml_in_mode(
            toml_str,
            crate::entities::ai_declaration_manifest::AiDeclarationMode::Lenient,
        )
        .expect("parse must succeed");
        let t = config.torpedoes.expect("torpedoes");
        assert_eq!(t.shield_pierce, 0.0);
        // Propagates into the runtime config that the in-flight torpedo
        // snapshots at launch.
        assert_eq!(t.to_runtime().shield_pierce, 0.0);
    }

    #[test]
    fn torpedoes_shield_pierce_parses_when_present() {
        let toml_str = r##"
[torpedoes]
count = 5
shield_pierce = 0.5

[[torpedoes.tubes]]
id = "fore"
facing_deg = 0.0
fire_arc_deg = 90.0
"##;
        let config = EntityConfig::from_toml_in_mode(
            toml_str,
            crate::entities::ai_declaration_manifest::AiDeclarationMode::Lenient,
        )
        .expect("parse must succeed");
        let t = config.torpedoes.expect("torpedoes");
        assert!((t.shield_pierce - 0.5).abs() < 1e-6);
        assert!((t.to_runtime().shield_pierce - 0.5).abs() < 1e-6);
    }

    #[test]
    fn torpedo_in_flight_snapshots_shield_pierce_at_launch() {
        // Wiring proof: changing the in-flight torpedo's snapshot mid-flight
        // doesn't affect future launches (it's a per-torpedo copy).
        use crate::torpedo::{TorpedoConfig, TorpedoSystem};
        use std::collections::HashMap;
        let mut cfg = TorpedoConfig::default();
        cfg.shield_pierce = 0.75;
        let tubes = vec![TorpedoTubeConfig {
            id: "fore".into(),
            facing_deg: 0.0,
            fire_arc_deg: 90.0,
            load_time: None,
            marker: None,
            barrels: Vec::new(),
            pattern: Vec::new(),
            volley_max: 1,
            ai_target_count: None,
            ai: None,
        }];
        let mut sys = TorpedoSystem::from_configs(&tubes, cfg);
        assert!(sys.start_load("fore"));
        let targets: HashMap<String, (f32, f32, f32)> = HashMap::new();
        sys.tick(sys.config.load_time, &targets, &mut || "test".into());
        sys.launch("fore", "t1".into(), 0.0, 0.0, 0.0, 0.0, None, None);
        assert!((sys.in_flight[0].shield_pierce - 0.75).abs() < 1e-6);

        let det = sys.handle_collision_full("t1").unwrap();
        assert!((det.shield_pierce - 0.75).abs() < 1e-6);
    }

    #[test]
    fn phaser_banks_defaults_to_empty_vec_when_absent() {
        let toml_str = r##"
[weapons_console]
"##;
        // Lenient: a bare `[weapons_console]` owes a `weapons_doctrine`
        // declaration since issue #956, and this fixture is about the serde
        // default for an absent bank list.
        let config = EntityConfig::from_toml_in_mode(
            toml_str,
            crate::entities::ai_declaration_manifest::AiDeclarationMode::Lenient,
        )
        .expect("parse must succeed");
        let wc = config.weapons_console.expect("weapons_console");
        assert!(
            wc.phaser_banks.is_empty(),
            "phaser_banks defaults to empty when [[phaser_banks]] absent"
        );
    }

    #[test]
    fn validate_phaser_banks_accepts_valid_list() {
        let banks = vec![
            PhaserBankConfig {
                id: "port".into(),
                facing_deg: -90.0,
                fire_arc_deg: 180.0,
                auto_arc_deg: 120.0,
                beam_range: 0.0,
                shield_pierce: None,
                marker: None,
                ..Default::default()
            },
            PhaserBankConfig {
                id: "starboard".into(),
                facing_deg: 90.0,
                fire_arc_deg: 180.0,
                auto_arc_deg: 120.0,
                beam_range: 0.0,
                shield_pierce: None,
                marker: None,
                ..Default::default()
            },
        ];
        assert!(validate_phaser_banks(&banks).is_ok());
    }

    #[test]
    fn validate_phaser_banks_rejects_empty_list() {
        let err = validate_phaser_banks(&[]).unwrap_err();
        assert!(err.contains("empty"), "error mentions empty: {err}");
    }

    #[test]
    fn validate_phaser_banks_rejects_duplicate_ids() {
        let banks = vec![
            PhaserBankConfig {
                id: "port".into(),
                facing_deg: -90.0,
                fire_arc_deg: 180.0,
                auto_arc_deg: 90.0,
                beam_range: 0.0,
                shield_pierce: None,
                marker: None,
                ..Default::default()
            },
            PhaserBankConfig {
                id: "port".into(),
                facing_deg: 90.0,
                fire_arc_deg: 180.0,
                auto_arc_deg: 90.0,
                beam_range: 0.0,
                shield_pierce: None,
                marker: None,
                ..Default::default()
            },
        ];
        let err = validate_phaser_banks(&banks).unwrap_err();
        assert!(err.contains("duplicate"), "error mentions duplicate: {err}");
        assert!(err.contains("port"));
    }

    #[test]
    fn validate_phaser_banks_rejects_auto_arc_greater_than_fire_arc() {
        let banks = vec![PhaserBankConfig {
            id: "port".into(),
            facing_deg: -90.0,
            fire_arc_deg: 90.0,
            auto_arc_deg: 180.0,
            beam_range: 0.0,
            shield_pierce: None,
            marker: None,
            ..Default::default()
        }];
        let err = validate_phaser_banks(&banks).unwrap_err();
        assert!(
            err.contains("auto_arc_deg"),
            "error mentions auto arc: {err}"
        );
    }

    #[test]
    fn validate_phaser_banks_rejects_fire_arc_out_of_range() {
        let banks = vec![PhaserBankConfig {
            id: "port".into(),
            facing_deg: 0.0,
            fire_arc_deg: 400.0,
            auto_arc_deg: 90.0,
            beam_range: 0.0,
            shield_pierce: None,
            marker: None,
            ..Default::default()
        }];
        let err = validate_phaser_banks(&banks).unwrap_err();
        assert!(
            err.contains("fire_arc_deg"),
            "error mentions fire arc: {err}"
        );

        let banks = vec![PhaserBankConfig {
            id: "port".into(),
            facing_deg: 0.0,
            fire_arc_deg: 0.0,
            auto_arc_deg: 0.0,
            beam_range: 0.0,
            shield_pierce: None,
            marker: None,
            ..Default::default()
        }];
        let err = validate_phaser_banks(&banks).unwrap_err();
        assert!(err.contains("fire_arc_deg"), "zero arc rejected: {err}");
    }

    #[test]
    fn torpedo_tubes_array_parses_full_entries() {
        let toml_str = r##"
[torpedoes]
count = 10

[[torpedoes.tubes]]
id = "fore_port"
facing_deg = -30.0
fire_arc_deg = 90.0

[[torpedoes.tubes]]
id = "fore_starboard"
facing_deg = 30.0
fire_arc_deg = 90.0

[[torpedoes.tubes]]
id = "aft"
facing_deg = 180.0
fire_arc_deg = 90.0
"##;
        let config = EntityConfig::from_toml_in_mode(
            toml_str,
            crate::entities::ai_declaration_manifest::AiDeclarationMode::Lenient,
        )
        .expect("parse must succeed");
        let t = config.torpedoes.expect("torpedoes");
        assert_eq!(t.tubes.len(), 3);
        assert_eq!(t.tubes[0].id, "fore_port");
        assert_eq!(t.tubes[0].facing_deg, -30.0);
        assert_eq!(t.tubes[0].fire_arc_deg, 90.0);
        assert_eq!(t.tubes[2].id, "aft");
        assert_eq!(t.tubes[2].facing_deg, 180.0);
    }

    #[test]
    fn torpedo_tubes_defaults_to_empty_vec_when_absent() {
        let toml_str = r##"
[torpedoes]
count = 10
"##;
        let config = EntityConfig::from_toml_in_mode(
            toml_str,
            crate::entities::ai_declaration_manifest::AiDeclarationMode::Lenient,
        )
        .expect("parse must succeed");
        let t = config.torpedoes.expect("torpedoes");
        assert!(
            t.tubes.is_empty(),
            "tubes defaults to empty when [[torpedoes.tubes]] absent"
        );
    }

    #[test]
    fn validate_torpedo_tubes_accepts_valid_list() {
        let tubes = vec![
            TorpedoTubeConfig {
                id: "fore_port".into(),
                facing_deg: -30.0,
                fire_arc_deg: 90.0,
                load_time: None,
                marker: None,
                barrels: Vec::new(),
                pattern: Vec::new(),
                volley_max: 1,
                ai_target_count: None,
                ai: None,
            },
            TorpedoTubeConfig {
                id: "aft".into(),
                facing_deg: 180.0,
                fire_arc_deg: 90.0,
                load_time: None,
                marker: None,
                barrels: Vec::new(),
                pattern: Vec::new(),
                volley_max: 1,
                ai_target_count: None,
                ai: None,
            },
        ];
        assert!(validate_torpedo_tubes(&tubes).is_ok());
    }

    #[test]
    fn validate_torpedo_tubes_rejects_empty_list() {
        let err = validate_torpedo_tubes(&[]).unwrap_err();
        assert!(err.contains("empty"));
    }

    #[test]
    fn validate_torpedo_tubes_rejects_duplicate_ids() {
        let tubes = vec![
            TorpedoTubeConfig {
                id: "aft".into(),
                facing_deg: 180.0,
                fire_arc_deg: 90.0,
                load_time: None,
                marker: None,
                barrels: Vec::new(),
                pattern: Vec::new(),
                volley_max: 1,
                ai_target_count: None,
                ai: None,
            },
            TorpedoTubeConfig {
                id: "aft".into(),
                facing_deg: 0.0,
                fire_arc_deg: 90.0,
                load_time: None,
                marker: None,
                barrels: Vec::new(),
                pattern: Vec::new(),
                volley_max: 1,
                ai_target_count: None,
                ai: None,
            },
        ];
        let err = validate_torpedo_tubes(&tubes).unwrap_err();
        assert!(err.contains("duplicate"));
        assert!(err.contains("aft"));
    }

    #[test]
    fn validate_torpedo_tubes_rejects_fire_arc_out_of_range() {
        let tubes = vec![TorpedoTubeConfig {
            id: "aft".into(),
            facing_deg: 180.0,
            fire_arc_deg: 0.0,
            load_time: None,
            marker: None,
            barrels: Vec::new(),
            pattern: Vec::new(),
            volley_max: 1,
            ai_target_count: None,
            ai: None,
        }];
        let err = validate_torpedo_tubes(&tubes).unwrap_err();
        assert!(err.contains("fire_arc_deg"));
    }

    // ── Torpedo tube barrel-pattern validation (issue #766) ──────────────────

    fn torpedo_tube(id: &str) -> TorpedoTubeConfig {
        TorpedoTubeConfig {
            id: id.into(),
            facing_deg: 0.0,
            fire_arc_deg: 90.0,
            load_time: None,
            marker: None,
            barrels: Vec::new(),
            pattern: Vec::new(),
            volley_max: 1,
            ai_target_count: None,
            ai: None,
        }
    }

    #[test]
    fn validate_torpedo_tubes_accepts_legacy_single_barrel() {
        // No barrels + no pattern is the backward-compat single-barrel tube.
        assert!(validate_torpedo_tubes(&[torpedo_tube("fore")]).is_ok());
    }

    #[test]
    fn validate_torpedo_tubes_accepts_valid_pattern() {
        let mut t = torpedo_tube("fore");
        t.barrels = vec!["b0".into(), "b1".into()];
        t.pattern = vec![
            crate::weapons::pattern::BarrelPatternStep {
                barrels: vec![0],
                offset_secs: 0.0,
            },
            crate::weapons::pattern::BarrelPatternStep {
                barrels: vec![0, 1],
                offset_secs: 0.3,
            },
        ];
        assert!(validate_torpedo_tubes(&[t]).is_ok());
    }

    #[test]
    fn validate_torpedo_tubes_rejects_barrel_index_out_of_range() {
        let mut t = torpedo_tube("fore");
        t.barrels = vec!["b0".into(), "b1".into()];
        t.pattern = vec![crate::weapons::pattern::BarrelPatternStep {
            barrels: vec![2], // only 0,1 exist
            offset_secs: 0.0,
        }];
        let err = validate_torpedo_tubes(&[t]).unwrap_err();
        assert!(err.contains("barrel index 2"), "{err}");
    }

    #[test]
    fn validate_torpedo_tubes_rejects_negative_offset() {
        let mut t = torpedo_tube("fore");
        t.barrels = vec!["b0".into()];
        t.pattern = vec![crate::weapons::pattern::BarrelPatternStep {
            barrels: vec![0],
            offset_secs: -0.5,
        }];
        let err = validate_torpedo_tubes(&[t]).unwrap_err();
        assert!(err.contains("offset_secs"), "{err}");
    }

    #[test]
    fn validate_torpedo_tubes_rejects_empty_step() {
        let mut t = torpedo_tube("fore");
        t.barrels = vec!["b0".into()];
        t.pattern = vec![crate::weapons::pattern::BarrelPatternStep {
            barrels: vec![],
            offset_secs: 0.0,
        }];
        assert!(validate_torpedo_tubes(&[t]).is_err());
    }

    #[test]
    fn validate_torpedo_tubes_rejects_multi_barrel_without_pattern() {
        let mut t = torpedo_tube("fore");
        t.barrels = vec!["b0".into(), "b1".into()];
        // No pattern: under-specified for >1 barrel.
        let err = validate_torpedo_tubes(&[t]).unwrap_err();
        assert!(err.contains("pattern"), "{err}");
    }

    #[test]
    fn every_shipped_torpedo_tube_config_validates() {
        // The authoring gate the editor enforces must hold for shipped hulls.
        let mut problems: Vec<String> = Vec::new();
        let entries = std::fs::read_dir("assets/entities").expect("assets/entities exists");
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("toml") {
                continue;
            }
            // Through the include resolver (issue #906) so a composed hull is
            // judged on its resolved document.
            let key = path.to_string_lossy().replace('\\', "/");
            let cfg: EntityConfig = match crate::entity_includes::load_entity_config(&key) {
                Ok(c) => c,
                Err(_) => continue,
            };
            let Some(torpedoes) = cfg.torpedoes.as_ref() else {
                continue;
            };
            if torpedoes.tubes.is_empty() {
                continue;
            }
            if let Err(e) = validate_torpedo_tubes(&torpedoes.tubes) {
                problems.push(format!("{}: {e}", path.display()));
            }
        }
        assert!(
            problems.is_empty(),
            "shipped torpedo tubes invalid: {problems:?}"
        );
    }

    // ── Blaster bank validation (issue #765) ─────────────────────────────────

    fn blaster_bank(id: &str) -> BlasterBankConfig {
        BlasterBankConfig {
            id: id.into(),
            fire_arc_deg: 90.0,
            ..BlasterBankConfig::default()
        }
    }

    #[test]
    fn validate_blaster_banks_accepts_empty_list() {
        // Most hulls carry no blasters; an empty list is fine.
        assert!(validate_blaster_banks(&[]).is_ok());
    }

    #[test]
    fn validate_blaster_banks_accepts_legacy_single_barrel() {
        // No barrels + no pattern is the backward-compat single-barrel bank.
        let banks = vec![blaster_bank("fore")];
        assert!(validate_blaster_banks(&banks).is_ok());
    }

    #[test]
    fn validate_blaster_banks_accepts_valid_pattern() {
        let mut b = blaster_bank("fore");
        b.barrels = vec!["b0".into(), "b1".into()];
        b.pattern = vec![
            crate::weapons::pattern::BarrelPatternStep {
                barrels: vec![0],
                offset_secs: 0.0,
            },
            crate::weapons::pattern::BarrelPatternStep {
                barrels: vec![0, 1],
                offset_secs: 0.3,
            },
        ];
        assert!(validate_blaster_banks(&[b]).is_ok());
    }

    #[test]
    fn validate_blaster_banks_rejects_barrel_index_out_of_range() {
        let mut b = blaster_bank("fore");
        b.barrels = vec!["b0".into(), "b1".into()];
        b.pattern = vec![crate::weapons::pattern::BarrelPatternStep {
            barrels: vec![2], // only 0,1 exist
            offset_secs: 0.0,
        }];
        let err = validate_blaster_banks(&[b]).unwrap_err();
        assert!(err.contains("barrel index 2"), "{err}");
    }

    #[test]
    fn validate_blaster_banks_rejects_negative_offset() {
        let mut b = blaster_bank("fore");
        b.barrels = vec!["b0".into()];
        b.pattern = vec![crate::weapons::pattern::BarrelPatternStep {
            barrels: vec![0],
            offset_secs: -0.5,
        }];
        let err = validate_blaster_banks(&[b]).unwrap_err();
        assert!(err.contains("offset_secs"), "{err}");
    }

    #[test]
    fn validate_blaster_banks_rejects_empty_step() {
        let mut b = blaster_bank("fore");
        b.barrels = vec!["b0".into()];
        b.pattern = vec![crate::weapons::pattern::BarrelPatternStep {
            barrels: vec![],
            offset_secs: 0.0,
        }];
        assert!(validate_blaster_banks(&[b]).is_err());
    }

    #[test]
    fn validate_blaster_banks_rejects_multi_barrel_without_pattern() {
        let mut b = blaster_bank("fore");
        b.barrels = vec!["b0".into(), "b1".into()];
        // No pattern: under-specified for >1 barrel.
        let err = validate_blaster_banks(&[b]).unwrap_err();
        assert!(err.contains("pattern"), "{err}");
    }

    #[test]
    fn validate_blaster_banks_rejects_duplicate_ids() {
        let banks = vec![blaster_bank("fore"), blaster_bank("fore")];
        let err = validate_blaster_banks(&banks).unwrap_err();
        assert!(err.contains("duplicate"));
    }

    #[test]
    fn every_shipped_blaster_bank_config_validates() {
        // The authoring gate the editor enforces must hold for shipped hulls.
        let mut problems: Vec<String> = Vec::new();
        let entries = std::fs::read_dir("assets/entities").expect("assets/entities exists");
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("toml") {
                continue;
            }
            // Through the include resolver (issue #906) so a composed hull is
            // judged on its resolved document.
            let file = path.to_string_lossy().replace('\\', "/");
            let cfg = match crate::entity_includes::load_entity_config(&file) {
                Ok(c) => c,
                Err(_) => continue,
            };
            if let Some(weapons) = cfg.weapons_console.as_ref() {
                if let Err(e) = validate_blaster_banks(&weapons.blaster_banks) {
                    problems.push(format!("{file}: {e}"));
                }
            }
        }
        assert!(
            problems.is_empty(),
            "shipped blaster banks must validate:\n{}",
            problems.join("\n")
        );
    }

    // ── Shared Harrow engine trail colour (issue #945) ───────────────────────

    #[test]
    fn every_shipped_harrow_dynasty_hull_shares_the_red_engine_trail_colour() {
        // The patrol, cruiser and warhawk author `[helm_console.engine_pfx]`
        // colour [1.0, 0.22, 0.08, 0.68]; the destroyer was missing it
        // entirely (#945), which meant it silently fell back to the
        // renderer's default trail colour (`ENGINE_DEFAULT_COLOR` in
        // `src/server/pfx.rs`, a blue) instead of matching the rest of the
        // faction. This sweep guards every shipped Harrow hull riding a
        // `dynasty_*.glb` model, not just the two the issue named, so a
        // future hull can't reintroduce the same silent-fallback gap.
        //
        // Deliberately no Rust-side colour literal here (see
        // `src/entities/authored_ai_pins.rs`'s rule against re-pinning
        // authored TOML content in Rust): the TOML is the specification, so
        // this compares every matched hull's colour against the *others*,
        // not against a baked-in constant. A designer retuning the Harrow
        // red across all four TOMLs should not have to also edit this file.
        const HARROW_FACTION: &str = "cccccccc-3333-4333-8333-cccccccccccc";

        let mut colours: Vec<(String, Option<[f32; 4]>)> = Vec::new();
        let entries = std::fs::read_dir("assets/entities").expect("assets/entities exists");
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("toml") {
                continue;
            }
            // Through the include resolver (issue #906) so a composed hull is
            // judged on its resolved document. Top-level only, matching every
            // other "shipped fleet" walk — `assets/entities/test/` fixtures
            // (e.g. the RNG-coverage escort, #954) are deliberately excluded.
            let file = path.to_string_lossy().replace('\\', "/");
            let Ok(cfg) = crate::entity_includes::load_entity_config(&file) else {
                continue;
            };
            let is_harrow = cfg
                .faction
                .map(|f| f.to_string() == HARROW_FACTION)
                .unwrap_or(false);
            let uses_dynasty_model = cfg
                .mesh
                .as_ref()
                .and_then(|m| m.model.as_ref())
                .map(|m| m.contains("assets/models/dynasty_"))
                .unwrap_or(false);
            if !is_harrow || !uses_dynasty_model {
                continue;
            }
            let color = cfg
                .helm_console
                .as_ref()
                .and_then(|h| h.engine_pfx.as_ref())
                .and_then(|pfx| pfx.color);
            colours.push((file, color));
        }

        // Non-vacuity: a load failure, a faction-authoring change, or a model
        // rename can each independently make the loop above match nothing —
        // and an empty `colours` would otherwise let every assertion below
        // pass while checking nothing. There are four shipped Harrow dynasty
        // hulls today (patrol, cruiser, warhawk, destroyer); guard that the
        // sweep still finds them.
        assert!(
            colours.len() >= 4,
            "expected at least 4 shipped Harrow dynasty hulls, found {}: {:?} — the sweep matched \
             too few hulls to guard anything (check HARROW_FACTION, the dynasty model path, and \
             that assets/entities TOMLs still load)",
            colours.len(),
            colours.iter().map(|(f, _)| f).collect::<Vec<_>>()
        );

        for (file, color) in &colours {
            assert!(
                color.is_some(),
                "{file}: [helm_console.engine_pfx] has no colour, so this hull silently falls \
                 back to the renderer's default trail colour instead of the shared Harrow red"
            );
        }

        let (baseline_file, baseline_color) = &colours[0];
        for (file, color) in &colours[1..] {
            assert_eq!(
                color, baseline_color,
                "{file}: engine_pfx.color = {color:?} does not match {baseline_file}'s \
                 {baseline_color:?}; every shipped Harrow dynasty hull must share one engine \
                 trail colour"
            );
        }
    }

    // ── Inline fine-system AI policy (issue #775) ────────────────────────────

    #[test]
    fn shipped_alliance_and_harrow_hulls_share_authored_visual_colours() {
        // Faction colours are designer-authored TOML, not Rust constants. Sweep
        // top-level hulls through the include resolver and compare each faction
        // against its own first authored value: this catches missing values,
        // fallback colours, and per-hull drift without re-pinning the palette
        // in code when a designer deliberately retunes it.
        const FACTIONS: [(&str, &str, usize); 2] = [
            ("Alliance", "aaaaaaaa-1111-4111-8111-aaaaaaaaaaaa", 4),
            ("Harrow", "cccccccc-3333-4333-8333-cccccccccccc", 4),
        ];

        for (name, faction_id, minimum_hulls) in FACTIONS {
            let mut trail_colours: Vec<(String, [f32; 3])> = Vec::new();
            let mut phaser_colours: Vec<(String, [f32; 4])> = Vec::new();
            let entries = std::fs::read_dir("assets/entities").expect("assets/entities exists");

            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|extension| extension.to_str()) != Some("toml") {
                    continue;
                }
                let file = path.to_string_lossy().replace('\\', "/");
                let Ok(cfg) = crate::entity_includes::load_entity_config(&file) else {
                    continue;
                };
                if cfg.faction.map(|faction| faction.to_string()) != Some(faction_id.to_string()) {
                    continue;
                }

                // Static faction-owned entities (such as Station Axiom) share
                // the faction palette but are not mobile hulls and therefore
                // have no Helm trail to compare.
                if cfg.helm_console.is_none() {
                    continue;
                }

                let trail_colour = cfg
                    .helm_console
                    .as_ref()
                    .and_then(|helm| helm.engine_pfx.as_ref())
                    .and_then(|pfx| pfx.color)
                    .unwrap_or_else(|| panic!("{file}: {name} hull has no engine_pfx.color"));
                trail_colours.push((
                    file.clone(),
                    trail_colour[..3].try_into().expect("RGBA has RGB"),
                ));

                if let Some(weapons) = cfg.weapons_console.as_ref() {
                    for bank in &weapons.phaser_banks {
                        let colour: [f32; 4] =
                            bank.beam_color.clone().try_into().unwrap_or_else(|_| {
                                panic!(
                                    "{file}: {name} phaser bank {} has no RGBA beam_color",
                                    bank.id
                                )
                            });
                        phaser_colours.push((format!("{file}:{}", bank.id), colour));
                    }
                }
            }

            assert!(
                trail_colours.len() >= minimum_hulls,
                "expected at least {minimum_hulls} shipped {name} hulls, found {}",
                trail_colours.len()
            );
            assert!(
                !phaser_colours.is_empty(),
                "expected at least one shipped {name} phaser bank"
            );

            let (trail_baseline_file, trail_baseline) = &trail_colours[0];
            for (file, colour) in &trail_colours[1..] {
                assert_eq!(
                    colour, trail_baseline,
                    "{file}: engine trail RGB {colour:?} does not match {trail_baseline_file}'s {trail_baseline:?}"
                );
            }

            let (phaser_baseline_file, phaser_baseline) = &phaser_colours[0];
            for (file, colour) in &phaser_colours[1..] {
                assert_eq!(
                    colour, phaser_baseline,
                    "{file}: phaser beam colour {colour:?} does not match {phaser_baseline_file}'s {phaser_baseline:?}"
                );
            }
        }
    }

    const CHANNELS: &[&str] = &[CAPTAIN_RED_ALERT_CHANNEL];
    const VERBS: &[&str] = &[CAPTAIN_SET_RED_ALERT_VERB];

    fn captain_ai_toml() -> &'static str {
        r#"
name = "Test Cruiser"

[captain_console.ai]
param = { combat_window_secs = 8.0 }

[[captain_console.ai.rule]]
priority = 10
channel = "red_alert"
when = "fact(secs_since_combat) < param(combat_window_secs)"
verb = "set_red_alert"
value = true

[[captain_console.ai.rule]]
priority = 0
channel = "red_alert"
when = "true"
verb = "set_red_alert"
value = false
"#
    }

    #[test]
    fn captain_ai_policy_parses_and_resolves_to_typed_policy() {
        let cfg = EntityConfig::from_toml(captain_ai_toml()).expect("parse must succeed");
        let ai = cfg
            .captain_console
            .as_ref()
            .and_then(|c| c.ai.as_ref())
            .expect("captain_console.ai present");
        assert_eq!(ai.param.get("combat_window_secs"), Some(&8.0));
        assert_eq!(ai.rule.len(), 2);
        let policy = ai.to_policy().expect("policy resolves");
        assert_eq!(policy.rules.len(), 2);
        assert!(!policy.idle);
    }

    #[test]
    fn default_captain_policy_validates_and_resolves() {
        let cfg = crate::entities::authored_ai_pins::shipped_policy_toml("captain");
        assert!(validate_fine_system_ai_policy(&cfg, CHANNELS, VERBS).is_ok());
        assert!(cfg.to_policy().is_ok());
    }

    // ── Optional stateful policy schema (issue #882) ─────────────────────────

    /// AC7 — THE back-compat guard. Every shipped stateless block still parses
    /// AND decodes to a policy with NO machine, no states and no memory: the
    /// #882 schema fields are all `#[serde(default)]`, so nothing an author
    /// wrote before this issue changed meaning. Enumerates all fourteen
    /// canonical defaults behind the twelve Group A hosts.
    #[test]
    fn every_shipped_stateless_default_still_parses_as_stateless() {
        let shipped: Vec<(&str, FineSystemAiConfigToml)> = vec![
            (
                "captain",
                crate::entities::authored_ai_pins::shipped_policy_toml("captain"),
            ),
            (
                "comms_response",
                crate::entities::authored_ai_pins::shipped_policy_toml("comms_response"),
            ),
            (
                "engines",
                crate::entities::authored_ai_pins::shipped_policy_toml("engines"),
            ),
            (
                "steering",
                crate::entities::authored_ai_pins::shipped_policy_toml("steering"),
            ),
            (
                "lateral",
                crate::entities::authored_ai_pins::shipped_policy_toml("lateral"),
            ),
            (
                "vertical",
                crate::entities::authored_ai_pins::shipped_policy_toml("vertical"),
            ),
            (
                "impulse",
                crate::entities::authored_ai_pins::shipped_policy_toml("impulse"),
            ),
            (
                "boost",
                crate::entities::authored_ai_pins::shipped_policy_toml("boost"),
            ),
            (
                "phaser_bank",
                crate::entities::authored_ai_pins::shipped_policy_toml("phaser_bank"),
            ),
            (
                "blaster_bank",
                crate::entities::authored_ai_pins::shipped_policy_toml("blaster_bank"),
            ),
            (
                "torpedo_tube",
                crate::entities::authored_ai_pins::shipped_policy_toml("torpedo_tube"),
            ),
            (
                "torpedo_magazine",
                crate::entities::authored_ai_pins::shipped_policy_toml("torpedo_magazine"),
            ),
            (
                "shields_focus",
                crate::entities::authored_ai_pins::shipped_policy_toml("shields_focus"),
            ),
            (
                "power",
                crate::entities::authored_ai_pins::shipped_policy_toml("power"),
            ),
        ];
        for (name, cfg) in shipped {
            assert!(
                cfg.initial_state.is_none() && cfg.state.is_empty() && cfg.memory.is_empty(),
                "{name}: a shipped default must declare no #882 state machine"
            );
            let policy = cfg
                .to_policy()
                .unwrap_or_else(|e| panic!("{name}: must decode: {e}"));
            assert!(
                policy.machine().is_none() && policy.initial_state().is_none(),
                "{name}: a stateless block must decode to `machine: None`"
            );
            assert_eq!(
                policy.rules.len(),
                cfg.rule.len(),
                "{name}: every authored rule still decodes to a top-level rule"
            );
        }
    }

    /// A minimal authored stateful block: `initial_state`, two states with
    /// their own continuous rules, explicitly prioritised transitions, and a
    /// typed private memory declaration (AC1).
    fn stateful_boost_toml() -> &'static str {
        r#"
name = "Stateful"
[helm_console.boost_ai]
initial_state = "cruise"

[helm_console.boost_ai.param]
surge_urgency = 0.5
surge_dwell_secs = 3.0
max_engagements = 3.0

[helm_console.boost_ai.memory]
engagements = 0.0

[[helm_console.boost_ai.state]]
id = "cruise"

[[helm_console.boost_ai.state.transition]]
priority = 10
to = "surge"
when = "fact(hazard_urgency) > param(surge_urgency) and memory(engagements) < param(max_engagements)"

[[helm_console.boost_ai.state]]
id = "surge"

[[helm_console.boost_ai.state.rule]]
priority = 0
channel = "boost"
when = "true"
verb = "engage_boost"

[[helm_console.boost_ai.state.transition]]
priority = 0
to = "cruise"
when = "state_time >= param(surge_dwell_secs)"
"#
    }

    /// AC1: an authored stateful block round-trips through the TOML schema into
    /// the typed machine, with per-state rules and prioritised transitions.
    #[test]
    fn stateful_policy_round_trips_from_toml_to_typed_machine() {
        let cfg = EntityConfig::from_toml(stateful_boost_toml()).expect("parse must succeed");
        let ai = cfg
            .helm_console
            .as_ref()
            .and_then(|h| h.boost_ai.as_ref())
            .expect("helm_console.boost_ai present");
        assert_eq!(ai.initial_state.as_deref(), Some("cruise"));
        assert_eq!(ai.state.len(), 2);
        assert_eq!(ai.memory.get("engagements"), Some(&0.0));

        let policy = ai.to_policy().expect("policy resolves");
        let machine = policy.machine().expect("machine decoded");
        assert_eq!(machine.initial, "cruise");
        assert_eq!(machine.states.len(), 2);
        assert!(
            policy.rules.is_empty(),
            "a purely stateful policy carries no top-level rules"
        );
        let cruise = machine.state("cruise").expect("cruise declared");
        assert!(cruise.rules.is_empty());
        assert_eq!(cruise.transitions.len(), 1);
        assert_eq!(cruise.transitions[0].to, "surge");
        assert_eq!(cruise.transitions[0].priority, 10);
        let surge = machine.state("surge").expect("surge declared");
        assert_eq!(surge.rules.len(), 1);
        assert_eq!(surge.rules[0].channel, HELM_BOOST_CHANNEL);
        assert_eq!(
            surge.rules[0].verb,
            crate::ai::policy::AiPolicyVerb::EngageBoost
        );
        assert_eq!(machine.initial_memory.get("engagements"), Some(0.0));
    }

    /// **Issue #918: whether a doctrine leg yields its solved facing to a
    /// channel-3 arc-bearing request is AUTHORED on the leg.**
    ///
    /// Three properties, and the first is the one that keeps #673-#684 working:
    /// a leg that says nothing yields, so every hull authored before this field
    /// existed — and every helm with no doctrine at all — behaves exactly as it
    /// did. The second is that `false` reaches the typed policy. The third is
    /// that the host's question is answered off the CURRENT leg and off nothing
    /// else: not off the verb, not off the state's name, and with no parameter
    /// through which the requester could be consulted.
    #[test]
    fn a_doctrine_leg_authors_whether_it_yields_to_arc_requests() {
        let hull = r#"
name = "Committed"
[helm_console.steering_ai]
initial_state = "travel"

[[helm_console.steering_ai.state]]
id = "travel"

  [[helm_console.steering_ai.state.rule]]
  priority = 0
  channel = "yaw"
  when = "true"
  verb = "actuate_desired_facing"

  [[helm_console.steering_ai.state.transition]]
  priority = 0
  to = "committed"
  when = "true"

[[helm_console.steering_ai.state]]
id = "committed"
yields_to_arc_requests = false

  [[helm_console.steering_ai.state.rule]]
  priority = 0
  channel = "yaw"
  when = "true"
  verb = "hold_committed_heading"
"#;
        let cfg = EntityConfig::from_toml(hull).expect("the authored hull must parse and validate");
        let steering = cfg
            .helm_console
            .as_ref()
            .and_then(|h| h.steering_ai.as_ref())
            .expect("hull declares [helm_console.steering_ai]");
        assert!(
            steering.state[0].yields_to_arc_requests,
            "an omitted declaration must parse as YIELDING — the pre-#918 behaviour \
             every authored hull and every doctrine-less helm depends on"
        );
        assert!(!steering.state[1].yields_to_arc_requests);

        let policy = steering.to_policy().expect("the authored policy decodes");
        let machine = policy.machine().expect("machine decoded");
        assert!(
            machine
                .state("travel")
                .expect("travel declared")
                .yields_to_arc_requests
        );
        assert!(
            !machine
                .state("committed")
                .expect("committed declared")
                .yields_to_arc_requests,
            "the declaration must survive into the typed policy the host reads"
        );

        // The host's question, asked of one leg at a time.
        assert!(policy.leg_yields_to_arc_requests(Some("travel")));
        assert!(!policy.leg_yields_to_arc_requests(Some("committed")));
        assert!(
            policy.leg_yields_to_arc_requests(None),
            "a machine that has entered nothing has committed to no heading"
        );
        assert!(
            policy.leg_yields_to_arc_requests(Some("no-such-leg")),
            "an unknown leg is not a licence to ignore Channel 3"
        );

        // ...and a STATELESS policy — the shape a helm with no authored
        // doctrine flies — has no legs to decline with, whatever it is asked.
        let stateless = crate::ai::policy::AiPolicy::default();
        assert!(stateless.leg_yields_to_arc_requests(None));
        assert!(stateless.leg_yields_to_arc_requests(Some("committed")));
    }

    /// Issue #918: the declaration is rejected on a system that could never read
    /// it. An arc-bearing request is answered on the `yaw` channel; authored on
    /// the boost machine, `yields_to_arc_requests = false` is a line a designer
    /// would reasonably expect to do something and that nothing would ever
    /// consult — so it fails the load rather than reading as a silent no-op.
    #[test]
    fn declining_arc_requests_is_rejected_on_a_system_that_does_not_steer() {
        let leg = |yields: bool| FineSystemAiStateToml {
            id: "cruise".to_string(),
            yields_to_arc_requests: yields,
            ..Default::default()
        };

        let err = validate_fine_system_ai_policy(
            &stateful_cfg(Some("cruise"), vec![leg(false)]),
            BOOST_CHANNELS,
            BOOST_VERBS,
        )
        .unwrap_err();
        assert!(err.contains("cruise"), "must name the state: {err}");
        assert!(
            err.contains("yields_to_arc_requests"),
            "must name the offending declaration: {err}"
        );

        // The same machine is fine on the axis that steers...
        assert!(validate_fine_system_ai_policy(
            &stateful_cfg(Some("cruise"), vec![leg(false)]),
            STEERING_CHANNELS,
            STEERING_VERBS,
        )
        .is_ok());
        // ...and leaving the default standing is fine anywhere, which is why
        // every already-authored hull keeps loading.
        assert!(validate_fine_system_ai_policy(
            &stateful_cfg(Some("cruise"), vec![leg(true)]),
            BOOST_CHANNELS,
            BOOST_VERBS,
        )
        .is_ok());
    }

    /// Build a stateful policy config for the AC6 rejection cases directly, so
    /// each rejection is isolated from TOML surface noise.
    fn stateful_cfg(
        initial: Option<&str>,
        states: Vec<FineSystemAiStateToml>,
    ) -> FineSystemAiConfigToml {
        FineSystemAiConfigToml {
            evaluate_every_ticks: default_evaluate_every_ticks(),
            idle: false,
            param: std::collections::HashMap::new(),
            rule: Vec::new(),
            initial_state: initial.map(str::to_string),
            state: states,
            memory: std::collections::HashMap::new(),
        }
    }

    fn boost_state(id: &str, to: &[&str]) -> FineSystemAiStateToml {
        FineSystemAiStateToml {
            id: id.to_string(),
            rule: Vec::new(),
            transition: to
                .iter()
                .map(|t| FineSystemAiTransitionToml {
                    priority: 0,
                    to: t.to_string(),
                    when: "true".to_string(),
                })
                .collect(),
            ..Default::default()
        }
    }

    /// AC6: an `initial_state` naming a state that was never declared.
    #[test]
    fn undeclared_initial_state_is_rejected() {
        let cfg = stateful_cfg(Some("nowhere"), vec![boost_state("cruise", &[])]);
        let err = validate_fine_system_ai_policy(&cfg, BOOST_CHANNELS, BOOST_VERBS).unwrap_err();
        assert!(
            err.contains("initial_state") && err.contains("nowhere"),
            "got: {err}"
        );
    }

    /// AC6: states declared with no `initial_state` at all is the same defect —
    /// there is no entry point.
    #[test]
    fn states_without_an_initial_state_are_rejected() {
        let cfg = stateful_cfg(None, vec![boost_state("cruise", &[])]);
        let err = validate_fine_system_ai_policy(&cfg, BOOST_CHANNELS, BOOST_VERBS).unwrap_err();
        assert!(err.contains("initial_state"), "got: {err}");
        // The decoder refuses it too, so a caller skipping validation cannot
        // build a half-machine.
        assert!(cfg.to_policy().is_err());
    }

    /// AC6: a transition targeting a state that was never declared.
    #[test]
    fn transition_to_undeclared_state_is_rejected() {
        let cfg = stateful_cfg(Some("cruise"), vec![boost_state("cruise", &["surge"])]);
        let err = validate_fine_system_ai_policy(&cfg, BOOST_CHANNELS, BOOST_VERBS).unwrap_err();
        assert!(
            err.contains("undeclared state") && err.contains("surge"),
            "got: {err}"
        );
    }

    /// AC6: duplicate state ids — "which `cruise` did you mean?" has no answer.
    #[test]
    fn duplicate_state_ids_are_rejected() {
        let cfg = stateful_cfg(
            Some("cruise"),
            vec![boost_state("cruise", &[]), boost_state("cruise", &[])],
        );
        let err = validate_fine_system_ai_policy(&cfg, BOOST_CHANNELS, BOOST_VERBS).unwrap_err();
        assert!(err.contains("duplicate state id"), "got: {err}");
    }

    /// AC6: an unreachable state — neither the initial state nor any
    /// transition's target. A self-loop does NOT make a state reachable.
    #[test]
    fn unreachable_state_is_rejected() {
        let cfg = stateful_cfg(
            Some("cruise"),
            vec![
                boost_state("cruise", &[]),
                boost_state("orphan", &["orphan"]),
            ],
        );
        let err = validate_fine_system_ai_policy(&cfg, BOOST_CHANNELS, BOOST_VERBS).unwrap_err();
        assert!(
            err.contains("unreachable state") && err.contains("orphan"),
            "got: {err}"
        );
        // ...but wiring it up from the initial state makes it legal.
        let ok = stateful_cfg(
            Some("cruise"),
            vec![
                boost_state("cruise", &["orphan"]),
                boost_state("orphan", &[]),
            ],
        );
        assert!(validate_fine_system_ai_policy(&ok, BOOST_CHANNELS, BOOST_VERBS).is_ok());
    }

    /// AC6, the transitive case: a DISCONNECTED CLUSTER. `cruise` is the
    /// initial state; `drift` and `wander` transition to each other but nothing
    /// reaches either of them. Both are "the target of a transition", so a
    /// single pass that credits every transition target regardless of whether
    /// its source is itself reachable accepts this graph — which is exactly the
    /// dead branch AC6 exists to reject. Reachability has to be a fixpoint walk
    /// from `initial`.
    #[test]
    fn disconnected_state_cluster_is_rejected() {
        let cfg = stateful_cfg(
            Some("cruise"),
            vec![
                boost_state("cruise", &[]),
                boost_state("drift", &["wander"]),
                boost_state("wander", &["drift"]),
            ],
        );
        let err = validate_fine_system_ai_policy(&cfg, BOOST_CHANNELS, BOOST_VERBS).unwrap_err();
        assert!(
            err.contains("unreachable state") && (err.contains("drift") || err.contains("wander")),
            "got: {err}"
        );
        // Wiring ONE edge from the initial state into the cluster makes the
        // whole cluster reachable — the walk is transitive, not one-hop.
        let ok = stateful_cfg(
            Some("cruise"),
            vec![
                boost_state("cruise", &["drift"]),
                boost_state("drift", &["wander"]),
                boost_state("wander", &["drift"]),
            ],
        );
        assert!(validate_fine_system_ai_policy(&ok, BOOST_CHANNELS, BOOST_VERBS).is_ok());
    }

    /// Build one transition, spelled out, for the tie cases below —
    /// `boost_state` hardcodes priority 0, which is the very thing under test.
    fn transition(priority: i32, to: &str) -> FineSystemAiTransitionToml {
        FineSystemAiTransitionToml {
            priority,
            to: to.to_string(),
            when: "true".to_string(),
        }
    }

    /// One unconditional tube rule at an explicit `(priority, channel)`.
    fn tube_rule(priority: i32, channel: &str, verb: &str) -> FineSystemAiRuleToml {
        FineSystemAiRuleToml {
            priority,
            channel: channel.to_string(),
            when: "true".to_string(),
            verb: verb.to_string(),
            value: false,
            level: 0,
            response_index: 0,
        }
    }

    /// Issue #794 / PRD #774: two transitions out of ONE state at the same
    /// priority.
    ///
    /// The runtime does not stall on this — it silently takes the
    /// earliest-authored of the two, so the file reads as if the pair were
    /// interchangeable while the outcome depends entirely on which table was
    /// typed first.
    #[test]
    fn equal_priority_transitions_out_of_one_state_are_rejected() {
        let tie = stateful_cfg(
            Some("cruise"),
            vec![
                FineSystemAiStateToml {
                    id: "cruise".to_string(),
                    rule: Vec::new(),
                    transition: vec![transition(3, "surge"), transition(3, "coast")],
                    ..Default::default()
                },
                boost_state("surge", &[]),
                boost_state("coast", &[]),
            ],
        );
        let err = validate_fine_system_ai_policy(&tie, BOOST_CHANNELS, BOOST_VERBS).unwrap_err();
        // The message has to be actionable without opening this file: WHICH
        // state, WHICH priority, and WHICH TWO targets are competing.
        assert!(err.contains("state 'cruise'"), "must name the state: {err}");
        assert!(
            err.contains("same priority 3"),
            "must name the duplicated priority: {err}"
        );
        assert!(
            err.contains("'surge'") && err.contains("'coast'"),
            "must name both competing targets: {err}"
        );

        // Separating them by one is the whole fix.
        let ok = stateful_cfg(
            Some("cruise"),
            vec![
                FineSystemAiStateToml {
                    id: "cruise".to_string(),
                    rule: Vec::new(),
                    transition: vec![transition(3, "surge"), transition(2, "coast")],
                    ..Default::default()
                },
                boost_state("surge", &[]),
                boost_state("coast", &[]),
            ],
        );
        assert!(validate_fine_system_ai_policy(&ok, BOOST_CHANNELS, BOOST_VERBS).is_ok());
    }

    /// The scope of the transition tie is ONE state's transition set. Two
    /// different states each authoring a priority-0 exit are not competing —
    /// only one of them is ever the current state — and rejecting that would
    /// make the common two-state machine unauthorable.
    #[test]
    fn equal_priorities_in_different_states_are_not_a_transition_tie() {
        let cfg = stateful_cfg(
            Some("cruise"),
            vec![
                FineSystemAiStateToml {
                    id: "cruise".to_string(),
                    rule: Vec::new(),
                    transition: vec![transition(0, "surge")],
                    ..Default::default()
                },
                FineSystemAiStateToml {
                    id: "surge".to_string(),
                    rule: Vec::new(),
                    transition: vec![transition(0, "cruise")],
                    ..Default::default()
                },
            ],
        );
        assert!(validate_fine_system_ai_policy(&cfg, BOOST_CHANNELS, BOOST_VERBS).is_ok());
    }

    /// Issue #794 / PRD #774: two rules on ONE output channel at the same
    /// priority, in a STATELESS policy's top-level list.
    #[test]
    fn equal_priority_rules_on_one_channel_are_rejected() {
        let stateless = |rules: Vec<FineSystemAiRuleToml>| FineSystemAiConfigToml {
            evaluate_every_ticks: default_evaluate_every_ticks(),
            idle: false,
            param: std::collections::HashMap::new(),
            rule: rules,
            initial_state: None,
            state: Vec::new(),
            memory: std::collections::HashMap::new(),
        };
        let tie = stateless(vec![
            tube_rule(0, TORPEDO_LAUNCH_CHANNEL, TORPEDO_LAUNCH_VERB),
            tube_rule(0, TORPEDO_LAUNCH_CHANNEL, TORPEDO_LAUNCH_VERB),
        ]);
        let err = validate_fine_system_ai_policy(&tie, TORPEDO_TUBE_CHANNELS, TORPEDO_TUBE_VERBS)
            .unwrap_err();
        assert!(
            err.contains(&format!("channel '{TORPEDO_LAUNCH_CHANNEL}'")),
            "must name the contested channel: {err}"
        );
        assert!(
            err.contains("same priority 0"),
            "must name the duplicated priority: {err}"
        );
        assert!(
            err.contains(&format!("verb '{TORPEDO_LAUNCH_VERB}'")),
            "must name the competing verbs: {err}"
        );

        // Distinct priorities on the same channel are the fix...
        let ok = stateless(vec![
            tube_rule(1, TORPEDO_LAUNCH_CHANNEL, TORPEDO_LAUNCH_VERB),
            tube_rule(0, TORPEDO_LAUNCH_CHANNEL, TORPEDO_LAUNCH_VERB),
        ]);
        assert!(
            validate_fine_system_ai_policy(&ok, TORPEDO_TUBE_CHANNELS, TORPEDO_TUBE_VERBS).is_ok()
        );

        // ...and the SAME priority on DIFFERENT channels was never a tie: those
        // rules do not compete. This is the shipped default tube policy
        // verbatim — a load rule and a launch rule, both at priority 0 — so a
        // check scoped to priority alone would have broken every tube on every
        // hull that authors no inline block.
        let default = crate::entities::authored_ai_pins::shipped_policy_toml("torpedo_tube");
        assert_eq!(default.rule.len(), 2);
        assert_eq!(default.rule[0].priority, default.rule[1].priority);
        assert_ne!(default.rule[0].channel, default.rule[1].channel);
        assert!(validate_fine_system_ai_policy(
            &default,
            TORPEDO_TUBE_CHANNELS,
            TORPEDO_TUBE_VERBS
        )
        .is_ok());
    }

    /// The same rule tie, inside a STATE. A machine resolves its channels
    /// per-state, so the competing set is the current state's rule list — and
    /// two states each answering the same channel at priority 0 is ordinary
    /// content, not a tie.
    #[test]
    fn equal_priority_rules_inside_one_state_are_rejected() {
        let boost_rule = |priority: i32| FineSystemAiRuleToml {
            priority,
            channel: HELM_BOOST_CHANNEL.to_string(),
            when: "true".to_string(),
            verb: HELM_ENGAGE_BOOST_VERB.to_string(),
            value: false,
            level: 0,
            response_index: 0,
        };
        let tie = stateful_cfg(
            Some("surge"),
            vec![FineSystemAiStateToml {
                id: "surge".to_string(),
                rule: vec![boost_rule(4), boost_rule(4)],
                transition: Vec::new(),
                ..Default::default()
            }],
        );
        let err = validate_fine_system_ai_policy(&tie, BOOST_CHANNELS, BOOST_VERBS).unwrap_err();
        assert!(err.contains("state 'surge'"), "must name the state: {err}");
        assert!(
            err.contains(&format!("channel '{HELM_BOOST_CHANNEL}'"))
                && err.contains("same priority 4"),
            "must name the channel and the priority: {err}"
        );

        // One rule per state at the same priority is not a tie.
        let ok = stateful_cfg(
            Some("cruise"),
            vec![
                FineSystemAiStateToml {
                    id: "cruise".to_string(),
                    rule: vec![boost_rule(0)],
                    transition: vec![transition(0, "surge")],
                    ..Default::default()
                },
                FineSystemAiStateToml {
                    id: "surge".to_string(),
                    rule: vec![boost_rule(0)],
                    transition: Vec::new(),
                    ..Default::default()
                },
            ],
        );
        assert!(validate_fine_system_ai_policy(&ok, BOOST_CHANNELS, BOOST_VERBS).is_ok());
    }

    /// Issue #794 AC8: every shipped entity template still loads.
    ///
    /// The tie rejections above are new refusals over content that has been
    /// shipping for a while, so the guard that matters is not "the fixture is
    /// rejected" but "nothing in `assets/entities/` is". Deliberately a
    /// DIRECTORY WALK rather than a hand-listed set of `include_str!`s: the
    /// failure mode being guarded against is a hull authored later that trips a
    /// rule nobody remembered, and a hand-listed set is exactly the thing that
    /// does not grow when a hull is added.
    #[test]
    fn every_shipped_entity_template_still_loads() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/entities");
        let mut checked = 0usize;
        for entry in std::fs::read_dir(&dir).expect("assets/entities must exist") {
            let path = entry.expect("readable dir entry").path();
            if path.extension().and_then(|e| e.to_str()) != Some("toml") {
                continue;
            }
            // Through the include resolver (issue #906), not `from_toml` on the
            // raw bytes: the day a shipped hull declares `includes`, a raw read
            // would assert on the UNRESOLVED text and this guard would quietly
            // stop covering it.
            let key = path.to_string_lossy().replace('\\', "/");
            if let Err(e) = crate::entity_includes::load_entity_config(&key) {
                panic!("shipped template {} no longer loads: {e}", path.display());
            }
            checked += 1;
        }
        assert!(
            checked > 20,
            "the walk found only {checked} templates — it is not reaching the \
             shipped content it is supposed to be guarding"
        );
    }

    /// AC6: a `memory(...)` reference in a STATELESS policy. Private memory has
    /// no owner without a state machine, and reading a silent `false` would be
    /// a trap rather than a diagnostic.
    #[test]
    fn memory_reference_in_a_stateless_policy_is_rejected() {
        let cfg = FineSystemAiConfigToml {
            evaluate_every_ticks: default_evaluate_every_ticks(),
            idle: false,
            param: std::collections::HashMap::new(),
            rule: vec![FineSystemAiRuleToml {
                priority: 0,
                channel: HELM_BOOST_CHANNEL.to_string(),
                when: "memory(engagements) > 0".to_string(),
                verb: HELM_ENGAGE_BOOST_VERB.to_string(),
                value: false,
                level: 0,
                response_index: 0,
            }],
            initial_state: None,
            state: Vec::new(),
            memory: std::collections::HashMap::new(),
        };
        let err = validate_fine_system_ai_policy(&cfg, BOOST_CHANNELS, BOOST_VERBS).unwrap_err();
        assert!(
            err.contains("memory") && err.contains("no states"),
            "got: {err}"
        );
    }

    /// AC6: a `state_time` reference in a STATELESS policy — the same defect on
    /// the other private atom.
    #[test]
    fn state_time_reference_in_a_stateless_policy_is_rejected() {
        let cfg = FineSystemAiConfigToml {
            evaluate_every_ticks: default_evaluate_every_ticks(),
            idle: false,
            param: std::collections::HashMap::new(),
            rule: vec![FineSystemAiRuleToml {
                priority: 0,
                channel: HELM_BOOST_CHANNEL.to_string(),
                when: "state_time > 5".to_string(),
                verb: HELM_ENGAGE_BOOST_VERB.to_string(),
                value: false,
                level: 0,
                response_index: 0,
            }],
            initial_state: None,
            state: Vec::new(),
            memory: std::collections::HashMap::new(),
        };
        let err = validate_fine_system_ai_policy(&cfg, BOOST_CHANNELS, BOOST_VERBS).unwrap_err();
        assert!(
            err.contains("state_time") && err.contains("no states"),
            "got: {err}"
        );
    }

    // ── Authored history operators (issue #890) ─────────────────────────────

    /// A `history(...)` guard in a STATELESS policy — the same defect on the
    /// third private atom. The window is per-fine-system retained state that
    /// the state-machine host advances; a policy with no machine is never
    /// ticked, so nothing would ever fill it.
    #[test]
    fn history_reference_in_a_stateless_policy_is_rejected() {
        let mut cfg = FineSystemAiConfigToml {
            rule: vec![FineSystemAiRuleToml {
                priority: 0,
                channel: HELM_BOOST_CHANNEL.to_string(),
                when: "history(min, hazard_urgency, param(window_ticks)) >= 1".to_string(),
                verb: HELM_ENGAGE_BOOST_VERB.to_string(),
                value: false,
                level: 0,
                response_index: 0,
            }],
            ..Default::default()
        };
        cfg.param.insert("window_ticks".to_string(), 8.0);
        let err = validate_fine_system_ai_policy(&cfg, BOOST_CHANNELS, BOOST_VERBS).unwrap_err();
        assert!(
            err.contains("history(min, hazard_urgency, param(window_ticks))")
                && err.contains("no states"),
            "got: {err}"
        );
    }

    /// The window length is a `param(...)` like any other reference, and an
    /// undeclared one is rejected — the author never has to guess whether a
    /// typo silently disabled the operator.
    #[test]
    fn an_undeclared_history_window_param_is_rejected() {
        let mut cfg = stateful_cfg(Some("cruise"), vec![boost_state("cruise", &[])]);
        cfg.state[0].transition = vec![FineSystemAiTransitionToml {
            priority: 0,
            to: "cruise".to_string(),
            when: "history(min, hazard_urgency, param(never_declared)) >= 1".to_string(),
        }];
        let err = validate_fine_system_ai_policy(&cfg, BOOST_CHANNELS, BOOST_VERBS).unwrap_err();
        assert!(
            err.contains("undeclared parameter") && err.contains("never_declared"),
            "got: {err}"
        );
    }

    /// The half of the malformed-window check the parser cannot make: only the
    /// hull knows what its parameter is worth. A zero-length window retains
    /// nothing and is never full, so it would disable the guard in silence.
    #[test]
    fn a_non_integral_or_zero_history_window_param_is_rejected() {
        for (value, needle) in [(8.5_f32, "8.5"), (0.0, "0"), (-3.0, "-3")] {
            let mut cfg = stateful_cfg(Some("cruise"), vec![boost_state("cruise", &[])]);
            cfg.param.insert("window_ticks".to_string(), value);
            cfg.state[0].transition = vec![FineSystemAiTransitionToml {
                priority: 0,
                to: "cruise".to_string(),
                when: "history(min, hazard_urgency, param(window_ticks)) >= 1".to_string(),
            }];
            let err =
                validate_fine_system_ai_policy(&cfg, BOOST_CHANNELS, BOOST_VERBS).unwrap_err();
            assert!(
                err.contains("positive whole number") && err.contains(needle),
                "window length {value} must be rejected naming the value; got: {err}"
            );
        }
    }

    /// An authored window of a positive whole number of ticks is accepted, in
    /// every position a stateful policy can carry a guard.
    #[test]
    fn an_authored_history_window_validates_in_rules_and_transitions() {
        let mut cfg = stateful_cfg(Some("cruise"), vec![boost_state("cruise", &[])]);
        cfg.param.insert("window_ticks".to_string(), 30.0);
        cfg.state[0].rule = vec![FineSystemAiRuleToml {
            priority: 0,
            channel: HELM_BOOST_CHANNEL.to_string(),
            when: "history(net_change, hazard_urgency, param(window_ticks)) > 0".to_string(),
            verb: HELM_ENGAGE_BOOST_VERB.to_string(),
            value: false,
            level: 0,
            response_index: 0,
        }];
        cfg.state[0].transition = vec![FineSystemAiTransitionToml {
            priority: 0,
            to: "cruise".to_string(),
            when: "history(min, hazard_urgency, param(window_ticks)) >= 1".to_string(),
        }];
        validate_fine_system_ai_policy(&cfg, BOOST_CHANNELS, BOOST_VERBS)
            .expect("an authored positive whole window is valid content");
    }

    /// A target selector has no history bag on ANY host — it is evaluated per
    /// candidate against a snapshot — so the rejection needs no host to answer
    /// and fires on the host-less path too.
    #[test]
    fn a_history_reference_in_a_target_selector_is_rejected() {
        let cfg: FineSystemAiSelectorToml = toml::from_str(
            r#"
            horizon = 100.0
            switch_margin = 0.0
            eligibility = "candidate_fact(detectable) > 0 and history(min, range_to_target, 30) > 0"
            "#,
        )
        .expect("fixture selector parses");
        let err = validate_fine_system_ai_selector(&cfg, SENSORS_SELECTOR_SOURCES).unwrap_err();
        assert!(
            err.contains("history(min, range_to_target, 30)") && err.contains("no history bag"),
            "got: {err}"
        );
    }

    /// An undeclared `memory(...)` slot is rejected in a STATEFUL policy too —
    /// the same contract `param(...)` has carried since #775.
    #[test]
    fn undeclared_memory_slot_is_rejected_in_a_stateful_policy() {
        let mut cfg = stateful_cfg(Some("cruise"), vec![boost_state("cruise", &[])]);
        cfg.state[0].transition = vec![FineSystemAiTransitionToml {
            priority: 0,
            to: "cruise".to_string(),
            when: "memory(never_declared) > 0".to_string(),
        }];
        let err = validate_fine_system_ai_policy(&cfg, BOOST_CHANNELS, BOOST_VERBS).unwrap_err();
        assert!(err.contains("undeclared memory"), "got: {err}");
    }

    /// The existing per-rule channel/verb/param checks run unchanged over each
    /// STATE's rules, not just the top-level list.
    #[test]
    fn per_state_rules_get_the_same_channel_and_verb_checks() {
        let mut cfg = stateful_cfg(Some("cruise"), vec![boost_state("cruise", &[])]);
        cfg.state[0].rule = vec![FineSystemAiRuleToml {
            priority: 0,
            channel: "not_a_channel".to_string(),
            when: "true".to_string(),
            verb: HELM_ENGAGE_BOOST_VERB.to_string(),
            value: false,
            level: 0,
            response_index: 0,
        }];
        let err = validate_fine_system_ai_policy(&cfg, BOOST_CHANNELS, BOOST_VERBS).unwrap_err();
        assert!(
            err.contains("unknown channel") && err.contains("state 'cruise' rule 0"),
            "got: {err}"
        );
    }

    /// `idle = true` alongside states is as contradictory as `idle` alongside
    /// rules.
    #[test]
    fn idle_alongside_states_is_rejected() {
        let mut cfg = stateful_cfg(Some("cruise"), vec![boost_state("cruise", &[])]);
        cfg.idle = true;
        let err = validate_fine_system_ai_policy(&cfg, BOOST_CHANNELS, BOOST_VERBS).unwrap_err();
        assert!(err.contains("idle") && err.contains("states"), "got: {err}");
    }

    /// Issue #883, carried forward from the #882 review: a policy declaring BOTH
    /// top-level rules and states is rejected outright.
    ///
    /// Before this, the shape validated and then quietly did nothing useful: a
    /// machine resolves exclusively through `resolve_channel_in_state`, so the
    /// top-level rules were dead code that looked live. Worse, `stateful` is
    /// computed from the presence of states, so a `memory(...)` reference in one
    /// of those dead top-level rules PASSED validation and then evaluated false
    /// for ever (the stateless scan hands `best_in` an empty memory bag). Two
    /// silent failures in one shape — the same class as #882's blocking bug.
    #[test]
    fn a_policy_with_both_top_level_rules_and_states_is_rejected() {
        let mut cfg = stateful_cfg(Some("cruise"), vec![boost_state("cruise", &[])]);
        cfg.rule = vec![FineSystemAiRuleToml {
            priority: 0,
            channel: HELM_BOOST_CHANNEL.to_string(),
            when: "true".to_string(),
            verb: HELM_ENGAGE_BOOST_VERB.to_string(),
            value: false,
            level: 0,
            response_index: 0,
        }];
        let err = validate_fine_system_ai_policy(&cfg, BOOST_CHANNELS, BOOST_VERBS).unwrap_err();
        assert!(
            err.contains("both top-level rules") && err.contains("states"),
            "got: {err}"
        );
    }

    /// The rejection above must not catch either honest shape: a purely
    /// stateless policy and a purely stateful one both still validate. (Every
    /// shipped default is the former; the destroyer's three policies are the
    /// latter.)
    #[test]
    fn rule_xor_state_leaves_both_honest_shapes_valid() {
        let stateless = crate::entities::authored_ai_pins::shipped_policy_toml("boost");
        assert!(validate_fine_system_ai_policy(&stateless, BOOST_CHANNELS, BOOST_VERBS).is_ok());
        let stateful = stateful_cfg(Some("cruise"), vec![boost_state("cruise", &[])]);
        assert!(stateful.rule.is_empty(), "the fixture must be state-only");
        assert!(validate_fine_system_ai_policy(&stateful, BOOST_CHANNELS, BOOST_VERBS).is_ok());
    }

    // ── The Harrow Destroyer hull (issue #883) ───────────────────────────────

    /// AC4, both halves. Forward blasters are PRESENT and correctly forward
    /// (narrow arc dead ahead); torpedoes are ABSENT — no magazine, no tubes,
    /// and no torpedo system entry.
    ///
    /// The absence is asserted explicitly because it is content that is very
    /// easy to "helpfully" restore: every other armed NPC hull in the set has a
    /// `[torpedoes]` block, so copying one as a starting point re-adds it
    /// silently, and nothing else in the suite would notice.
    #[test]
    fn harrow_destroyer_carries_forward_blasters_and_no_torpedoes() {
        let cfg = EntityConfig::from_toml(&harrow_destroyer_toml())
            .expect("the destroyer hull must pass content validation");

        let wc = cfg
            .weapons_console
            .as_ref()
            .expect("the hull declares [weapons_console]");
        assert!(
            !wc.blaster_banks.is_empty(),
            "the destroyer's whole armament is its forward blasters"
        );
        for bank in &wc.blaster_banks {
            assert_eq!(
                bank.facing_deg, 0.0,
                "bank '{}' must face dead ahead: a fly-through fires off the bow",
                bank.id
            );
            assert!(
                bank.fire_arc_deg > 0.0 && bank.fire_arc_deg <= 90.0,
                "bank '{}' must be a NARROW forward arc, got {}",
                bank.id,
                bank.fire_arc_deg
            );
        }
        assert!(
            wc.phaser_banks.is_empty(),
            "the destroyer is blaster-armed only"
        );

        // The absence assertion (AC4).
        assert!(
            cfg.torpedoes.is_none(),
            "the destroyer must carry NO torpedo magazine"
        );
        let ship_config = cfg
            .ship_config
            .as_ref()
            .expect("the hull declares [[system]] blocks");
        for system in &ship_config.systems {
            assert!(
                !system.kind.contains("torpedo"),
                "the destroyer must declare no torpedo system, found '{:?}' ({})",
                system.id,
                system.kind
            );
        }
    }

    /// The doctrine itself: all three travel axes author a STATEFUL policy, the
    /// two yaw mode verbs are both used, and boost is authored — the block
    /// without which `ai_helm_boost` returns before it does anything and the
    /// escape leg silently loses its back half.
    #[test]
    fn harrow_destroyer_authors_the_fly_through_machine_on_all_three_axes() {
        let cfg = EntityConfig::from_toml(&harrow_destroyer_toml()).expect("hull must parse");
        let hc = cfg
            .helm_console
            .as_ref()
            .expect("the hull declares [helm_console]");
        assert!(
            hc.boost.is_some(),
            "[helm_console.boost] is mandatory: without it the spawner inserts a \
             DISABLED BoostConfigResource and ai_helm_boost stands down"
        );

        for (name, ai) in [
            ("engines_ai", hc.engines_ai.as_ref()),
            ("steering_ai", hc.steering_ai.as_ref()),
            ("boost_ai", hc.boost_ai.as_ref()),
        ] {
            let ai = ai.unwrap_or_else(|| panic!("{name} must be authored"));
            assert!(
                ai.rule.is_empty(),
                "{name} must be state-only (rule XOR state)"
            );
            let ids: Vec<&str> = ai.state.iter().map(|s| s.id.as_str()).collect();
            assert_eq!(
                ids,
                vec![
                    "shadow",
                    "acquire",
                    "inbound",
                    "escape",
                    "recover",
                    "reenter",
                    "pressed_pivot",
                    "pressed_pass",
                ],
                "{name} resolves to the class pass + recovery + pressed machine \
                 (issues #789, #878)"
            );
            // `shadow` and `initial_state = "shadow"` arrive with the class
            // doctrine (issue #878): the shared fragment RESTS defensive and a
            // hull unlocks the aggressive half by posture. This hull authors
            // `press_posture = 0.0`, the lowest rung, so the gate is open on the
            // first tick and the defensive leg is left immediately and never
            // re-entered.
            assert_eq!(ai.initial_state.as_deref(), Some("shadow"));
            let policy = ai.to_policy().expect("must decode");
            assert!(
                policy.machine().is_some(),
                "{name} must decode to a machine"
            );
        }

        // The yaw channel carries ALL FOUR mode verbs, and which one wins is the
        // whole doctrine: tracking while inbound, frozen heading on the escape,
        // a ring while recovering, a cut-thrust pivot on the way back in.
        let steering = hc.steering_ai.as_ref().unwrap();
        let verbs: Vec<&str> = steering
            .state
            .iter()
            .flat_map(|s| s.rule.iter())
            .map(|r| r.verb.as_str())
            .collect();
        assert!(verbs.contains(&HELM_ACTUATE_DESIRED_FACING_VERB));
        assert!(verbs.contains(&HELM_HOLD_COMMITTED_HEADING_VERB));
        assert!(verbs.contains(&HELM_HOLD_RECOVERY_ORBIT_VERB));
        assert!(verbs.contains(&HELM_PIVOT_TO_REENGAGE_VERB));

        // Issue #788, AC7 / issue #789, AC4: none of the recovery states, and
        // not the pressed PASS, authors a boost rule. The absence is the
        // doctrine — a pass flown with the drive lit is not the "normal-speed
        // pass" the hull is supposed to be making — and an absence is exactly
        // the kind of content that gets helpfully filled in.
        let boost = hc.boost_ai.as_ref().unwrap();
        for id in ["recover", "reenter", "pressed_pass"] {
            let state = boost
                .state
                .iter()
                .find(|s| s.id == id)
                .unwrap_or_else(|| panic!("boost_ai must declare '{id}'"));
            assert!(
                state.rule.is_empty(),
                "boost_ai '{id}' must author NO rule: boost is cancelled before the pass"
            );
        }
    }

    /// Issue #789, AC4, as content: the pressed PIVOT is the one state outside
    /// the escape that lights the drive, and the hull's boost genuinely
    /// *increases* turn authority rather than trading it away.
    ///
    /// The second half is load-bearing and is not obvious from the doctrine
    /// alone: `apply_ship_physics` multiplies `max_yaw_rate` by
    /// `steering_multiplier`, so a hull authoring a value below 1.0 would boost
    /// its pivot into turning SLOWER. Nothing in the state machine can detect
    /// that; only this pin can.
    #[test]
    fn harrow_destroyer_boosts_the_pressed_pivot_with_a_drive_that_turns_harder() {
        let cfg = EntityConfig::from_toml(&harrow_destroyer_toml()).expect("hull must parse");
        let hc = cfg.helm_console.as_ref().unwrap();

        let pivot = hc
            .boost_ai
            .as_ref()
            .unwrap()
            .state
            .iter()
            .find(|s| s.id == "pressed_pivot")
            .expect("boost_ai must declare 'pressed_pivot'");
        assert_eq!(
            pivot.rule.len(),
            1,
            "the pressed pivot lights the drive with exactly one rule"
        );
        assert_eq!(pivot.rule[0].verb, HELM_ENGAGE_BOOST_VERB);
        assert!(
            !pivot.rule[0].when.contains("speed_fraction"),
            "the pivot is flown at ZERO throttle, so a minimum-speed guard would \
             refuse the one case this state exists for; got `{}`",
            pivot.rule[0].when
        );

        let boost = hc
            .boost
            .as_ref()
            .expect("[helm_console.boost] is mandatory");
        assert!(
            boost.steering_multiplier > 1.0,
            "boost must INCREASE the pivot's turn authority — physics multiplies \
             max_yaw_rate by this — got {}",
            boost.steering_multiplier
        );
    }

    /// Issue #789, AC1/AC3/AC5, as content, on all three machines.
    ///
    /// Each conjunct here is doing distinct work and each has a distinct failure
    /// mode if dropped, so they are asserted individually rather than as one
    /// string match:
    ///
    /// * the two pressed facts — drop either and the branch fires on a ship that
    ///   is escaping cleanly, or one that is nowhere near the target's guns;
    /// * the shield conjunct — drop it and a destroyer with its shields UP
    ///   abandons the ordinary pass cycle to jab at a range it never needed to
    ///   leave;
    /// * a higher priority than the recovery branch — equal or lower and the
    ///   pressed branch is unreachable, silently, because `recover`'s guard is a
    ///   strict subset of it.
    #[test]
    fn harrow_destroyer_presses_on_failed_progress_inside_the_targets_reach() {
        let cfg = EntityConfig::from_toml(&harrow_destroyer_toml()).expect("hull must parse");
        let hc = cfg.helm_console.as_ref().unwrap();
        for (name, ai) in [
            ("engines_ai", hc.engines_ai.as_ref().unwrap()),
            ("steering_ai", hc.steering_ai.as_ref().unwrap()),
            ("boost_ai", hc.boost_ai.as_ref().unwrap()),
        ] {
            let escape = ai.state.iter().find(|s| s.id == "escape").unwrap();
            let pressed = escape
                .transition
                .iter()
                .find(|t| t.to == "pressed_pivot")
                .unwrap_or_else(|| {
                    panic!("{name}: the escape must be able to reach the pressed arm")
                });
            for required in [
                crate::ship::helm_ai::SEPARATION_PROGRESS_FACT,
                crate::ship::helm_ai::INSIDE_THREAT_RANGE_FACT,
                crate::ship::helm_ai::PRESSED_MIN_PROGRESS_PARAM,
                crate::ship::helm_ai::SHIELD_FRACTION_FACT,
            ] {
                assert!(
                    pressed.when.contains(required),
                    "{name}: the pressed guard must reference `{required}`, got `{}`",
                    pressed.when
                );
            }
            let recover = escape
                .transition
                .iter()
                .find(|t| t.to == "recover")
                .unwrap_or_else(|| panic!("{name}: the escape must still reach recovery"));
            assert!(
                pressed.priority > recover.priority,
                "{name}: the pressed branch must outrank recovery ({} vs {}) or it can \
                 never fire — recovery's guard is a subset of it",
                pressed.priority,
                recover.priority
            );

            // AC3: the way OUT of the pressed loop is the ordinary escape, and
            // neither pressed state waits on a shield threshold or a held
            // distance the way recovery does.
            for id in ["pressed_pivot", "pressed_pass"] {
                let state = ai
                    .state
                    .iter()
                    .find(|s| s.id == id)
                    .unwrap_or_else(|| panic!("{name} must declare '{id}'"));
                for transition in &state.transition {
                    assert!(
                        !transition
                            .when
                            .contains(crate::ship::helm_ai::SAFE_DISTANCE_HELD_FACT)
                            && !transition.when.contains("reentry_shield_fraction"),
                        "{name} '{id}': pressed behaviour abandons the shield threshold and \
                         the standoff ring — it may not wait on either, got `{}`",
                        transition.when
                    );
                }
            }
            assert!(
                ai.state
                    .iter()
                    .find(|s| s.id == "pressed_pass")
                    .unwrap()
                    .transition
                    .iter()
                    .any(|t| t.to == "escape"),
                "{name}: every short pass must end in another real escape attempt"
            );
        }
    }

    /// Issue #789: the SHORT pass is short because of an authored scalar, and it
    /// is a different scalar from the ordinary pass's.
    ///
    /// Authoring the same number twice would make this arm indistinguishable
    /// from a re-run of the ordinary inbound leg while still passing every
    /// structural assertion above.
    #[test]
    fn harrow_destroyer_breaks_off_the_pressed_pass_sooner_than_an_ordinary_one() {
        let cfg = EntityConfig::from_toml(&harrow_destroyer_toml()).expect("hull must parse");
        let steering = cfg
            .helm_console
            .as_ref()
            .unwrap()
            .steering_ai
            .as_ref()
            .unwrap();
        for required in crate::ship::helm_ai::PRESSED_PARAMS {
            assert!(
                steering.param.contains_key(*required),
                "steering_ai must author `{required}`: the host gates the whole pressed \
                 arm on all four together"
            );
        }
        let pressed = steering
            .param
            .get(crate::ship::helm_ai::PRESSED_HYSTERESIS_PARAM)
            .copied()
            .unwrap();
        let ordinary = steering
            .param
            .get("closest_approach_hysteresis")
            .copied()
            .unwrap();
        assert!(
            pressed > 0.0 && pressed < ordinary,
            "the pressed pass must break off sooner than an ordinary one \
             ({pressed} vs {ordinary}) — equal values make it the same pass"
        );
        // ...and the two history windows are independently authored lengths.
        let pressed_window = steering
            .param
            .get(crate::ship::helm_ai::PRESSED_WINDOW_TICKS_PARAM)
            .copied()
            .unwrap();
        assert!(
            pressed_window > 1.0 && pressed_window.is_finite(),
            "the progress window must be a real, finite bound, got {pressed_window}"
        );
    }

    /// Issue #788, AC6: re-entry is gated on BOTH the shield fraction and the
    /// held distance, on every axis. Dropping either conjunct from any of the
    /// three machines would let one axis re-enter early and desynchronise the
    /// hull from itself — and would do it silently, because each machine runs
    /// its own copy.
    #[test]
    fn harrow_destroyer_reentry_requires_both_shields_and_held_distance() {
        let cfg = EntityConfig::from_toml(&harrow_destroyer_toml()).expect("hull must parse");
        let hc = cfg.helm_console.as_ref().unwrap();
        for (name, ai) in [
            ("engines_ai", hc.engines_ai.as_ref().unwrap()),
            ("steering_ai", hc.steering_ai.as_ref().unwrap()),
            ("boost_ai", hc.boost_ai.as_ref().unwrap()),
        ] {
            let recover = ai
                .state
                .iter()
                .find(|s| s.id == "recover")
                .unwrap_or_else(|| panic!("{name} must declare 'recover'"));
            assert_eq!(
                recover.transition.len(),
                2,
                "{name}: recovery has exactly two ways out — re-entry, and the \
                 class doctrine's posture break-off (issue #878), which \
                 `press_posture = 0.0` makes unreachable on this hull"
            );
            let reenter_exit = recover
                .transition
                .iter()
                .find(|t| t.to == "reenter")
                .unwrap_or_else(|| panic!("{name}: recovery must reach re-entry"));
            let guard = &reenter_exit.when;
            assert!(
                guard.contains(crate::ship::helm_ai::SHIELD_FRACTION_FACT)
                    && guard.contains("reentry_shield_fraction"),
                "{name}: re-entry must require the authored shield fraction, got `{guard}`"
            );
            assert!(
                guard.contains(crate::ship::helm_ai::SAFE_DISTANCE_HELD_FACT),
                "{name}: re-entry must require the HELD safe distance, got `{guard}`"
            );
            // ...and the escape must be able to reach recovery at all.
            let escape = ai.state.iter().find(|s| s.id == "escape").unwrap();
            assert!(
                escape.transition.iter().any(|t| t.to == "recover"),
                "{name}: the escape must hand off to recovery when the shields are gone"
            );
        }
    }

    /// Issue #788, AC2/AC3/AC5: every scalar the recovery manoeuvre needs is an
    /// authored `param` on the Steering axis, found by the host BY NAME. A
    /// rename in either direction lights this up — and it must, because the
    /// host's response to a missing one is to decline the recovery arm and
    /// quietly fly ordinary doctrine travel instead.
    #[test]
    fn harrow_destroyer_authors_every_recovery_scalar_as_a_steering_param() {
        let cfg = EntityConfig::from_toml(&harrow_destroyer_toml()).expect("hull must parse");
        let steering = cfg
            .helm_console
            .as_ref()
            .unwrap()
            .steering_ai
            .as_ref()
            .unwrap();
        for required in [
            crate::ship::helm_ai::SAFE_RANGE_MARGIN_PARAM,
            crate::ship::helm_ai::ORBIT_SPEED_PARAM,
            crate::ship::helm_ai::ORBIT_SPIRAL_GAIN_PARAM,
            crate::ship::helm_ai::SAFE_RING_TOLERANCE_PARAM,
            crate::ship::helm_ai::SAFE_DISTANCE_WINDOW_TICKS_PARAM,
            crate::ship::helm_ai::REENGAGE_SPEED_PARAM,
        ] {
            assert!(
                steering.param.contains_key(required),
                "steering_ai must author `{required}`"
            );
        }
        // AC7: the pivot is flown on CUT thrust, and that is authored, not
        // hardcoded anywhere in Rust.
        assert_eq!(
            steering
                .param
                .get(crate::ship::helm_ai::REENGAGE_SPEED_PARAM),
            Some(&0.0),
            "the re-entry pivot must cut thrust"
        );
        // AC6: the authored re-entry fraction is the issue's stated 75%.
        assert_eq!(
            steering.param.get("reentry_shield_fraction"),
            Some(&0.75),
            "the authored re-entry shield fraction is 75%"
        );
        // AC5: the distance history is BOUNDED, and its bound is authored.
        let window = steering
            .param
            .get(crate::ship::helm_ai::SAFE_DISTANCE_WINDOW_TICKS_PARAM)
            .copied()
            .unwrap();
        assert!(
            window > 1.0 && window.is_finite(),
            "the window must be a real, finite bound, got {window}"
        );
    }

    /// AC6: every manoeuvre threshold the doctrine flies by is an authored
    /// `param`, and the host-side pass surface can find the four it reads by
    /// name. A rename in either direction lights this up — which matters,
    /// because the host's response to a missing param is to decline the pass
    /// entirely and quietly fall back to ordinary doctrine travel.
    #[test]
    fn harrow_destroyer_authors_every_manoeuvre_threshold_as_a_param() {
        let cfg = EntityConfig::from_toml(&harrow_destroyer_toml()).expect("hull must parse");
        let hc = cfg.helm_console.as_ref().unwrap();
        let engines = hc.engines_ai.as_ref().unwrap();
        let steering = hc.steering_ai.as_ref().unwrap();
        let boost = hc.boost_ai.as_ref().unwrap();

        for required in [
            crate::ship::helm_ai::APPROACH_SPEED_PARAM,
            crate::ship::helm_ai::ESCAPE_SPEED_PARAM,
        ] {
            assert!(
                engines.param.contains_key(required),
                "engines_ai must author `{required}`"
            );
        }
        for required in [
            crate::ship::helm_ai::TRACKING_DEADBAND_PARAM,
            crate::ship::helm_ai::TRACKING_FULL_STEER_PARAM,
        ] {
            assert!(
                steering.param.contains_key(required),
                "steering_ai must author `{required}`"
            );
        }
        // Shared manoeuvre thresholds — every axis's guards reference them, so
        // every axis declares them (validation rejects an undeclared reference).
        for ai in [engines, steering, boost] {
            for required in [
                "commit_range",
                "closing_rate_epsilon",
                "closest_approach_hysteresis",
                "escape_duration_secs",
            ] {
                assert!(ai.param.contains_key(required), "must author `{required}`");
            }
            assert!(
                ai.memory
                    .contains_key(crate::ship::helm_ai::MIN_RANGE_SEEN_MEMORY),
                "the closest-approach detector's running minimum must be declared"
            );
        }
        assert!(boost.param.contains_key("escape_boost_secs"));
    }

    // ── The Harrow Cruiser hull (issue #790) ─────────────────────────────────

    /// AC4, as content: the two banks are on the CENTRELINE, one forward and one
    /// aft, and each sweeps 270 degrees.
    ///
    /// Every number here is load-bearing and each has its own silent failure
    /// mode. Facings of ±90 (port/starboard, the shape every other beam cruiser
    /// in the set uses) would give a hull whose banks cover one side each and
    /// never overlap. An arc of 180 would give centreline banks with no overlap
    /// either — they would meet exactly on the beam line and cover nothing
    /// twice. 270 is the smallest arc for which two opposed banks overlap on
    /// BOTH beams, which is the entire premise of the doctrine.
    ///
    /// The overlap is asserted through the shared `in_arc` predicate rather than
    /// by arithmetic on the authored numbers, so this pins the behaviour the
    /// firing paths actually see.
    #[test]
    fn harrow_cruiser_carries_overlapping_fore_and_aft_270_degree_phaser_banks() {
        let cfg = EntityConfig::from_toml(&harrow_cruiser_toml())
            .expect("the cruiser hull must pass content validation");
        let wc = cfg
            .weapons_console
            .as_ref()
            .expect("the hull declares [weapons_console]");

        let ids: Vec<&str> = wc.phaser_banks.iter().map(|b| b.id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["fore", "aft"],
            "the cruiser mounts exactly one forward and one aft beam bank"
        );
        for (id, facing) in [("fore", 0.0), ("aft", 180.0)] {
            let bank = wc.phaser_banks.iter().find(|b| b.id == id).unwrap();
            assert_eq!(
                bank.facing_deg, facing,
                "bank '{id}' must sit on the centreline facing {facing}"
            );
            assert_eq!(
                bank.fire_arc_deg, 270.0,
                "bank '{id}' must sweep 270 degrees — anything narrower removes the \
                 broadside overlap the whole doctrine is built on"
            );
            assert_eq!(
                bank.auto_arc_deg, 270.0,
                "bank '{id}': the AI fires on the same arc it may fire on. A narrower \
                 auto arc would switch off exactly the abeam overlap this hull exists for"
            );
        }

        // The overlap itself, through the shared predicate: a target directly
        // off either beam is inside BOTH banks' arcs, and a target dead astern
        // is outside the fore bank's (so the arcs are genuinely 270 and not 360).
        for (label, rx, ry) in [
            ("starboard beam", 10.0_f32, 0.0_f32),
            ("port beam", -10.0, 0.0),
        ] {
            for bank in &wc.phaser_banks {
                assert!(
                    crate::weapons::phaser::in_arc(rx, ry, bank.facing_deg, bank.fire_arc_deg),
                    "a target on the {label} must bear for bank '{}'",
                    bank.id
                );
            }
        }
        // ...and each bank still has a blind wedge opposite its own facing, so
        // the arcs are genuinely 270 and not 360. A bank that covers everything
        // leaves the orbit nothing to solve.
        // Ship-local bearing is `radar_x.atan2(radar_y)`, so `(0, +r)` is dead
        // ahead and `(0, -r)` dead astern.
        let fore = wc.phaser_banks.iter().find(|b| b.id == "fore").unwrap();
        let aft = wc.phaser_banks.iter().find(|b| b.id == "aft").unwrap();
        assert!(
            !crate::weapons::phaser::in_arc(0.0, -10.0, fore.facing_deg, fore.fire_arc_deg),
            "the fore bank must be blind dead astern"
        );
        assert!(
            !crate::weapons::phaser::in_arc(0.0, 10.0, aft.facing_deg, aft.fire_arc_deg),
            "the aft bank must be blind dead ahead"
        );

        // The deliberate absence that survives: no blasters. The beams are still
        // the only continuous weapon on the hull.
        assert!(
            wc.blaster_banks.is_empty(),
            "the cruiser is beam-armed only between torpedo opportunities"
        );
        let ship_config = cfg
            .ship_config
            .as_ref()
            .expect("the hull declares [[system]] blocks");
        // Every bank needs its own fine system or it is never AI-operable, and
        // the id follows the `phaser-<bank_id>` convention the resolver uses.
        for bank in &wc.phaser_banks {
            let expected = crate::system_registry::phaser_bank_system_id(&bank.id)
                .expect("a non-empty bank id always resolves");
            assert!(
                ship_config.systems.iter().any(|s| s.id == expected),
                "bank '{}' must declare a [[system]] entry `{}` — without it the bank \
                 is never registered as AI-operable and the hull never fires",
                bank.id,
                expected.0
            );
        }
    }

    /// AC2, as content — and the INVERSION of a #790 pin.
    ///
    /// #790 asserted this hull carried no `[torpedoes]` table and no torpedo
    /// `[[system]]` at all, because a ship that never presents its bow has
    /// nothing to launch a fixed forward tube at. #791 changes that premise: the
    /// cruiser now breaks its orbit to point at a shield gap, so the pin is
    /// replaced rather than deleted, and the replacement is at least as specific.
    /// Every number below has its own silent failure mode:
    ///
    /// * fewer than two tubes and `fact(tubes_full)` degenerates to "this tube is
    ///   full" — the salvo doctrine would still parse and would pin nothing;
    /// * a tube facing anywhere but dead ahead, or a wide arc, and the ORBIT
    ///   already satisfies `in_arc` — the whole bow-on phase becomes decoration
    ///   the hull never needs;
    /// * a missing `[[system]]` entry and the tube is never AI-operable, so the
    ///   salvo can never be full and the phase never launches.
    #[test]
    fn harrow_cruiser_carries_two_narrow_bow_tubes_for_the_shield_opportunity() {
        let cfg = EntityConfig::from_toml(&harrow_cruiser_toml()).expect("hull must parse");
        let torpedoes = cfg
            .torpedoes
            .as_ref()
            .expect("the cruiser carries a torpedo magazine for the shield opportunity");

        let ids: Vec<&str> = torpedoes.tubes.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["bow_port", "bow_starboard"],
            "two forward tubes: with one, `tubes_full` says nothing a per-tube \
             `loaded` reading does not already say"
        );
        for tube in &torpedoes.tubes {
            assert_eq!(
                tube.facing_deg, 0.0,
                "tube '{}' must be a FIXED forward tube — the phase exists because \
                 the guns cannot be pointed without pointing the ship",
                tube.id
            );
            assert!(
                tube.fire_arc_deg > 0.0 && tube.fire_arc_deg <= 30.0,
                "tube '{}' must have a NARROW bow arc ({} deg): the orbit holds the \
                 target abeam, so an arc wide enough to cover the beam would let the \
                 cruiser launch without ever breaking off",
                tube.id,
                tube.fire_arc_deg
            );
            assert!(
                tube.volley_max > 1,
                "tube '{}' fires a salvo, not a round",
                tube.id
            );
            assert_eq!(
                tube.ai_target_count,
                Some(tube.volley_max),
                "an AI crew keeps tube '{}' at its full volley between \
                 opportunities — the load time is longer than the window",
                tube.id
            );
        }

        // The tubes are barely-homing hull-killers aimed by the bow, not a way
        // through a shield: a round that arrives after the arc recovers must do
        // nothing, which is what makes the abort transition matter.
        assert_eq!(
            torpedoes.damage_shields, 0,
            "these rounds go through the hole the beams made; they do not make one"
        );
        assert!(
            torpedoes.damage_hull > 0,
            "and they hurt the hull once they are through"
        );
        assert!(
            torpedoes.load_time > torpedoes.lifespan,
            "reloading ({}) must outlast a round's whole flight ({}), or the cruiser \
             could refill inside one opportunity and the doctrine would collapse into \
             holding the bow on and emptying the magazine",
            torpedoes.load_time,
            torpedoes.lifespan
        );

        // The authored per-tube policy — the first in the set. All three of AC2's
        // conditions, on the launch channel, on EVERY tube.
        for tube in &torpedoes.tubes {
            let ai = tube
                .ai
                .as_ref()
                .unwrap_or_else(|| panic!("tube '{}' must author its own policy", tube.id));
            assert!(
                validate_fine_system_ai_policy(ai, TORPEDO_TUBE_CHANNELS, TORPEDO_TUBE_VERBS)
                    .is_ok(),
                "tube '{}' policy must pass content validation",
                tube.id
            );
            let load = ai
                .rule
                .iter()
                .find(|r| r.channel == TORPEDO_LOAD_CHANNEL)
                .unwrap_or_else(|| panic!("tube '{}' must author a load rule", tube.id));
            assert_eq!(load.verb, TORPEDO_LOAD_VERB);
            let launch = ai
                .rule
                .iter()
                .find(|r| r.channel == TORPEDO_LAUNCH_CHANNEL)
                .unwrap_or_else(|| panic!("tube '{}' must author a launch rule", tube.id));
            assert_eq!(launch.verb, TORPEDO_LAUNCH_VERB);
            for required in ["tubes_full", "target_facing_shields", "in_arc"] {
                assert!(
                    launch.when.contains(required),
                    "tube '{}': the launch guard must require `{required}` continuously, \
                     got `{}`",
                    tube.id,
                    launch.when
                );
            }
        }

        // Fine systems: one per tube plus the shared magazine. Both loaders gate
        // on the magazine before they look at a tube, so its absence would
        // silently switch the whole armament off.
        let ship_config = cfg.ship_config.as_ref().expect("hull declares systems");
        let declared =
            |id: &crate::messages::SystemId| ship_config.systems.iter().any(|s| &s.id == id);
        assert!(
            declared(&crate::system_registry::torpedo_magazine_system_id()),
            "the shared magazine needs a [[system]] entry or neither loading nor \
             launching runs at all"
        );
        for tube in &torpedoes.tubes {
            let expected = crate::system_registry::torpedo_tube_system_id(&tube.id)
                .expect("a non-empty tube id always resolves");
            assert!(
                declared(&expected),
                "tube '{}' must declare a [[system]] entry `{}`",
                tube.id,
                expected.0
            );
        }
    }

    /// AC5, as content: the fore phaser bank still bears on a target held dead
    /// ahead, so ordinary beam pressure continues through the whole torpedo
    /// phase rather than pausing for it.
    ///
    /// This is a geometry claim, not a plumbing one — `ai_phaser_auto_fire` never
    /// reads the Steering verb or the pass surface (pinned in the weapons tests)
    /// — but the geometry is the half that could silently stop being true: narrow
    /// the fore bank's arc and the cruiser would go quiet exactly while it was
    /// most exposed.
    #[test]
    fn harrow_cruiser_fore_bank_still_bears_while_the_bow_is_held_on_the_target() {
        let cfg = EntityConfig::from_toml(&harrow_cruiser_toml()).expect("hull must parse");
        let wc = cfg.weapons_console.as_ref().unwrap();
        let fore = wc.phaser_banks.iter().find(|b| b.id == "fore").unwrap();
        // Ship-local bearing is `radar_x.atan2(radar_y)`, so `(0, +r)` is dead
        // ahead — where the bow hold puts the target.
        assert!(
            crate::weapons::phaser::in_arc(0.0, 10.0, fore.facing_deg, fore.auto_arc_deg),
            "a target dead ahead must be inside the fore bank's AUTO arc: the beams \
             keep working while the tubes line up"
        );
        // ...and the tubes' own cone sits inside that arc, so there is no bearing
        // at which the torpedoes may launch but the beams may not fire.
        let torpedoes = cfg.torpedoes.as_ref().unwrap();
        for tube in &torpedoes.tubes {
            let half = tube.fire_arc_deg * 0.5;
            for edge in [-half, half] {
                let (x, y) = (
                    simmath::sin(edge.to_radians()) * 10.0,
                    simmath::cos(edge.to_radians()) * 10.0,
                );
                assert!(
                    crate::weapons::phaser::in_arc(x, y, fore.facing_deg, fore.auto_arc_deg),
                    "the edge of tube '{}' arc ({edge} deg) must still be inside the \
                     fore bank's auto arc",
                    tube.id
                );
            }
        }
    }

    /// AC1/AC2, as content: both travel axes author the three-state machine
    /// (issue #791 adds `torpedo_run` to #790's pair), the yaw channel resolves
    /// the combat-orbit verb in the ring and the bow-hold verb in the phase, and
    /// every scalar the host reads by name is present on the Steering axis.
    #[test]
    fn harrow_cruiser_authors_the_broadside_orbit_machine_on_both_travel_axes() {
        let cfg = EntityConfig::from_toml(&harrow_cruiser_toml()).expect("hull must parse");
        let hc = cfg
            .helm_console
            .as_ref()
            .expect("the hull declares [helm_console]");

        for (name, ai) in [
            ("engines_ai", hc.engines_ai.as_ref()),
            ("steering_ai", hc.steering_ai.as_ref()),
        ] {
            let ai = ai.unwrap_or_else(|| panic!("{name} must be authored"));
            assert!(
                ai.rule.is_empty(),
                "{name} must be state-only (rule XOR state)"
            );
            let ids: Vec<&str> = ai.state.iter().map(|s| s.id.as_str()).collect();
            assert_eq!(
                ids,
                vec!["shadow", "acquire", "orbit", "torpedo_run"],
                "{name} resolves to the class orbit + shield-opportunity machine"
            );
            // `shadow` and `initial_state = "shadow"` arrive with the class
            // doctrine (issue #878): the shared fragment RESTS defensive and a
            // hull unlocks the aggressive half by posture. This hull authors
            // `press_posture = 0.0`, the lowest rung, so the gate is open on the
            // first tick and the defensive leg is left immediately and never
            // re-entered —
            // `the_harrow_hulls_unlock_their_class_doctrine_by_posture_alone`
            // (`authored_ai_pins.rs`) is what proves that rather than assuming it.
            assert_eq!(ai.initial_state.as_deref(), Some("shadow"));
            assert!(
                ai.to_policy().expect("must decode").machine().is_some(),
                "{name} must decode to a machine"
            );
        }

        // The yaw channel resolves the FIFTH mode verb in the orbit state, and
        // tracks in the approach.
        let steering = hc.steering_ai.as_ref().unwrap();
        let verb_of = |state_id: &str| -> String {
            let state = steering
                .state
                .iter()
                .find(|s| s.id == state_id)
                .unwrap_or_else(|| panic!("steering_ai must declare '{state_id}'"));
            assert_eq!(
                state.rule.len(),
                1,
                "'{state_id}' answers yaw with one rule"
            );
            state.rule[0].verb.clone()
        };
        assert_eq!(verb_of("acquire"), HELM_ACTUATE_DESIRED_FACING_VERB);
        assert_eq!(
            verb_of("orbit"),
            HELM_HOLD_COMBAT_ORBIT_VERB,
            "the orbit leg is the combat-orbit verb — NOT `hold_recovery_orbit`, \
             whose ring is derived from the target's reach and gated on a shield \
             doctrine this hull does not have"
        );
        assert_eq!(
            verb_of("torpedo_run"),
            HELM_HOLD_TORPEDO_BEARING_VERB,
            "the shield-opportunity leg is the SIXTH yaw verb — NOT \
             `pivot_to_reengage`, whose geometry is the same but whose host gate \
             is the six shield-recovery scalars this hull would have to invent"
        );

        // Every scalar the host reads off this axis BY NAME. A rename in either
        // direction lights this up, and it must: the host's response to a
        // missing one is to decline the whole arm and fly ordinary doctrine
        // travel instead.
        for required in crate::ship::helm_ai::COMBAT_ORBIT_PARAMS {
            assert!(
                steering.param.contains_key(*required),
                "steering_ai must author `{required}`: the host gates the whole \
                 combat-orbit arm on all three together"
            );
        }
        for required in crate::ship::helm_ai::TORPEDO_BEARING_PARAMS {
            assert!(
                steering.param.contains_key(*required),
                "steering_ai must author `{required}`: the host gates the whole \
                 bow-hold arm on it, and the value this hull wants (0.0) is \
                 indistinguishable from an omission unless the NAME is present"
            );
        }
        for required in [
            crate::ship::helm_ai::TRACKING_DEADBAND_PARAM,
            crate::ship::helm_ai::TRACKING_FULL_STEER_PARAM,
        ] {
            assert!(
                steering.param.contains_key(required),
                "steering_ai must author `{required}`"
            );
        }
        assert!(
            steering
                .memory
                .contains_key(crate::ship::helm_ai::ORBIT_DIRECTION_MEMORY),
            "the circulation direction slot must be declared so its pre-engagement \
             value is authored rather than implicit"
        );
    }

    /// Issue #794, as content: the cruiser's `torpedo_run` exits carry
    /// DISTINCT priorities on both travel axes.
    ///
    /// The state has four ways out and three of them land in `orbit`, so the
    /// two that used to share priority 0 — "the salvo is spent" and "the
    /// battery is gone" — were behaviourally interchangeable and read as if the
    /// order were arbitrary. It was not arbitrary; it was the file order. The
    /// pin is here rather than left to the generic validator test because a
    /// re-author that collapsed them back would fail the load with a message
    /// about a hull, not about a fixture, and the cruiser is the hull that
    /// motivated the rule.
    #[test]
    fn harrow_cruiser_torpedo_run_exits_carry_distinct_priorities() {
        let cfg = EntityConfig::from_toml(&harrow_cruiser_toml()).expect("hull must parse");
        let hc = cfg.helm_console.as_ref().unwrap();
        for (name, ai) in [
            ("engines_ai", hc.engines_ai.as_ref().unwrap()),
            ("steering_ai", hc.steering_ai.as_ref().unwrap()),
        ] {
            let run = ai
                .state
                .iter()
                .find(|s| s.id == "torpedo_run")
                .unwrap_or_else(|| panic!("{name} must declare 'torpedo_run'"));
            let mut priorities: Vec<i32> = run.transition.iter().map(|t| t.priority).collect();
            let authored = priorities.len();
            // FIVE since issue #878 composed this hull on the class fragment:
            // the four documented here plus the class doctrine's posture
            // break-off to `shadow`, which `press_posture = 0.0` makes
            // unreachable on this hull.
            assert_eq!(authored, 5, "{name} authors the documented exits");
            priorities.sort_unstable();
            priorities.dedup();
            assert_eq!(
                priorities.len(),
                authored,
                "{name} `torpedo_run` must give every exit its own priority — a tie \
                 resolves by file order, which the file does not say out loud"
            );
            // The re-author is an ORDERING and not a re-aim: all three
            // window-closed exits still land back on the ring.
            let to_orbit = run.transition.iter().filter(|t| t.to == "orbit").count();
            assert_eq!(
                to_orbit, 3,
                "{name} keeps all three window-closed exits pointed at 'orbit'"
            );
        }
    }

    /// AC2, as content: the authored fighting ring sits INSIDE the banks' own
    /// beam range, and the orbit is flown under power.
    ///
    /// This is the assertion that makes the ring a fighting range rather than a
    /// standoff. A ring authored at or beyond `beam_range` would produce a
    /// cruiser that circles a target it cannot hit — every structural test above
    /// would still pass, and the hull would look correct and do nothing.
    #[test]
    fn harrow_cruiser_orbits_inside_its_own_beam_envelope_and_under_power() {
        let cfg = EntityConfig::from_toml(&harrow_cruiser_toml()).expect("hull must parse");
        let steering = cfg
            .helm_console
            .as_ref()
            .unwrap()
            .steering_ai
            .as_ref()
            .unwrap();
        let ring = steering
            .param
            .get(crate::ship::helm_ai::COMBAT_ORBIT_RANGE_PARAM)
            .copied()
            .unwrap();
        let shortest_beam = cfg
            .weapons_console
            .as_ref()
            .unwrap()
            .phaser_banks
            .iter()
            .map(|b| b.beam_range)
            .fold(f32::INFINITY, f32::min);
        assert!(
            ring > 0.0 && ring < shortest_beam,
            "the fighting ring ({ring}) must sit inside every bank's beam range \
             ({shortest_beam}) — a ring outside it circles a target it cannot hit"
        );

        let speed = steering
            .param
            .get(crate::ship::helm_ai::COMBAT_ORBIT_SPEED_PARAM)
            .copied()
            .unwrap();
        assert!(
            speed > 0.0 && speed <= 1.0,
            "the ring is flown UNDER POWER: an orbit at zero throttle is a parked \
             ship inside a hostile's guns, got {speed}"
        );
        let gain = steering
            .param
            .get(crate::ship::helm_ai::COMBAT_ORBIT_SPIRAL_GAIN_PARAM)
            .copied()
            .unwrap();
        assert!(
            gain > 0.0 && gain.is_finite(),
            "a zero spiral gain flies the bare tangent and never corrects the \
             radius, got {gain}"
        );
    }

    /// AC3, as content, and the reason it is asserted at all: NO transition
    /// anywhere in this hull's doctrine is guarded on a hazard reading.
    ///
    /// Avoidance composes onto the orbit additively inside the pure planner and
    /// through the stateless imminent-collision facing override — both temporary
    /// and both outside the state machine. A `fact(hazard_urgency)` transition
    /// here would replace that with a manoeuvre the hull has to be talked out
    /// of, and re-entering the orbit afterwards would RE-DRAW the circulation
    /// direction, so flying past an asteroid would randomise which way the
    /// cruiser circles. An absence is exactly the kind of content that gets
    /// helpfully filled in.
    #[test]
    fn harrow_cruiser_never_leaves_the_orbit_for_a_hazard() {
        let cfg = EntityConfig::from_toml(&harrow_cruiser_toml()).expect("hull must parse");
        let hc = cfg.helm_console.as_ref().unwrap();
        for (name, ai) in [
            ("engines_ai", hc.engines_ai.as_ref().unwrap()),
            ("steering_ai", hc.steering_ai.as_ref().unwrap()),
        ] {
            let orbit = ai.state.iter().find(|s| s.id == "orbit").unwrap();
            // Exactly TWO ways out, both of them named here. #790 pinned one;
            // #791 adds the shield opportunity and re-pins the whole set rather
            // than loosening the count, because "some number of exits" would let
            // a third grow in unnoticed.
            let exits: Vec<&str> = orbit.transition.iter().map(|t| t.to.as_str()).collect();
            assert_eq!(
                exits,
                vec!["shadow", "torpedo_run", "acquire"],
                "{name}: the orbit has exactly three ways out, in this priority order"
            );
            // The `shadow` exit is the class doctrine's posture break-off and is
            // UNREACHABLE on this hull (`press_posture = 0.0`), which is why the
            // hazard claim below is unaffected: it is still true that no exit
            // anywhere is guarded on hazard urgency.
            assert!(
                orbit.transition[1]
                    .when
                    .contains(crate::ship::helm_ai::TARGET_FACING_SHIELD_DOWN_FACT),
                "{name}: the shield opportunity is what interrupts the orbit, got `{}`",
                orbit.transition[1].when
            );
            // ...and the interruption stays an interruption, which takes BOTH
            // armament readings and not either alone.
            //
            // `tubes_full` is the load-bearing one and it is the LAUNCHER's
            // question: entry is what spends the broadside geometry, so it must
            // ask exactly what the `torpedo_launch` policy asks. Guarding on
            // `tubes_fillable` alone was measured at 506 bow-on ticks against
            // 431 orbiting over a 400 s run, only 29 of them with the tubes
            // actually full — reachability stays true through the whole 18 s
            // reload, so the ring broke on collapses with nothing loadable
            // inside the window.
            //
            // `tubes_fillable` stays beside it because it catches what
            // `tubes_full` cannot: a tube that is loaded but has been shot out,
            // and a magazine that can no longer top the battery up.
            //
            // Both pinned as content because the failure is invisible in any
            // test that fights a single engagement.
            for required in [
                crate::ship::helm_ai::TUBES_FULL_FACT,
                crate::ship::helm_ai::TUBES_FILLABLE_FACT,
            ] {
                assert!(
                    orbit.transition[1].when.contains(required),
                    "{name}: the orbit may only be given up with `{required}` \
                     satisfied — a salvo loaded, in a battery that can still fire \
                     it, got `{}`",
                    orbit.transition[1].when
                );
            }
            assert!(
                orbit.transition[2]
                    .when
                    .contains(crate::ship::helm_ai::TARGET_VALID_FACT),
                "{name}: losing the target is the other thing that ends the orbit, got `{}`",
                orbit.transition[2].when
            );
            // And the phase resumes the ring THREE ways, which is the whole of
            // the trap fix. Pinned as a set rather than as "at least one exit
            // mentioning the right facts", because it is precisely the ones
            // after the first that are easy to lose and impossible to miss the
            // absence of in a fixture whose target has shields and whose tubes
            // survive the engagement.
            let phase = ai.state.iter().find(|s| s.id == "torpedo_run").unwrap();
            let resumes: Vec<&str> = phase
                .transition
                .iter()
                .filter(|t| t.to == "orbit")
                .map(|t| t.when.as_str())
                .collect();
            assert_eq!(
                resumes.len(),
                3,
                "{name}: the phase must have exactly three ways back to the ring — \
                 the window closing, the salvo being spent and the battery becoming \
                 unusable, got {resumes:?}"
            );

            // THE WINDOW CLOSED. Both conjuncts: the shield being back is not
            // enough while a salvo is still in the air, or the cruiser turns
            // away mid-flight the instant the arc regenerates — which, since it
            // regenerates the whole time the rounds are flying, is nearly always.
            for required in [
                crate::ship::helm_ai::TARGET_FACING_SHIELD_DOWN_FACT,
                crate::ship::helm_ai::TORPEDOES_IN_FLIGHT_FACT,
            ] {
                assert!(
                    resumes[0].contains(required),
                    "{name}: the window-closed resume must require `{required}`, \
                     got `{}`",
                    resumes[0]
                );
            }

            // THE SALVO IS SPENT, and this one may not mention the target's
            // shields AT ALL. `target_facing_shield_down` reads a permanent 1.0
            // against any resolvable target with no `[shields]` block — a
            // station, a probe — so an exit that consulted it would be no exit
            // at all for those targets, and the cruiser would hold its nose on
            // one until something died. The bound has to be the hull's own
            // armament.
            for required in [
                crate::ship::helm_ai::TUBES_FULL_FACT,
                crate::ship::helm_ai::TORPEDOES_IN_FLIGHT_FACT,
            ] {
                assert!(
                    resumes[1].contains(required),
                    "{name}: the salvo-spent resume must require `{required}`, \
                     got `{}`",
                    resumes[1]
                );
            }
            assert!(
                !resumes[1].contains(crate::ship::helm_ai::TARGET_FACING_SHIELD_DOWN_FACT),
                "{name}: the salvo-spent resume must not depend on the target ever \
                 raising a shield — that is the one thing a shieldless target never \
                 does, got `{}`",
                resumes[1]
            );

            // THE BATTERY IS GONE, and this one exists because the guard above
            // cannot see it. `tubes_full` reads the ROUNDS, and a tube that is
            // shot out mid-phase keeps the rounds already in it — so the
            // salvo-spent resume stays shut, `torpedoes_in_flight` is zero, and
            // against a target with no arc to raise the hull is trapped bow-on
            // for a salvo `handle_fire_torpedo` will decline. Reachability is
            // the reading that notices, and it must be on the EXIT and not only
            // on the entry guard.
            for required in [
                crate::ship::helm_ai::TUBES_FILLABLE_FACT,
                crate::ship::helm_ai::TORPEDOES_IN_FLIGHT_FACT,
            ] {
                assert!(
                    resumes[2].contains(required),
                    "{name}: the battery-lost resume must require `{required}`, \
                     got `{}`",
                    resumes[2]
                );
            }
            assert!(
                !resumes[2].contains(crate::ship::helm_ai::TARGET_FACING_SHIELD_DOWN_FACT),
                "{name}: the battery-lost resume must not depend on the target \
                 either, got `{}`",
                resumes[2]
            );
            for state in &ai.state {
                for transition in &state.transition {
                    for forbidden in [
                        crate::ship::helm_ai::HAZARD_URGENCY_FACT,
                        "hazard_present",
                        "moving_hazard_threat",
                    ] {
                        assert!(
                            !transition.when.contains(forbidden),
                            "{name} '{}': no transition may be guarded on `{forbidden}` — \
                             a detour must bend the orbit, never exit it, got `{}`",
                            state.id,
                            transition.when
                        );
                    }
                }
            }
        }
    }

    /// The deliberate absence of a boost drive (see the hull header). A cruiser
    /// that lights the drive on the ring widens it; nothing in the doctrine asks
    /// for that, and there is no `[helm_console.boost]` block for it to use.
    #[test]
    fn harrow_cruiser_authors_no_boost_drive_and_no_boost_doctrine() {
        let cfg = EntityConfig::from_toml(&harrow_cruiser_toml()).expect("hull must parse");
        let hc = cfg.helm_console.as_ref().unwrap();
        assert!(
            hc.boost.is_none(),
            "the cruiser mounts no boost drive: a broadside orbit is flown at a \
             steady authored throttle"
        );
        assert_idle_boost_declaration(
            hc,
            "the cruiser: no boost doctrine to go with the drive it does not have",
        );
    }

    /// A hull says "this axis engages no boost" by AUTHORING an idle
    /// declaration, not by leaving the block out.
    ///
    /// These assertions read `boost_ai.is_none()` until #885b stage 5c, when
    /// every hull authored `[helm_console.boost_ai]`. Absence stopped meaning
    /// "no boost doctrine" the moment a synthesised `idle = true` stopped
    /// standing in for one — so the check moves onto the declaration rather than
    /// off the property. It is strictly stronger than the old form: an empty
    /// block, a rule on the `boost` channel, or a state machine all fail here,
    /// where `is_none()` only ever caught the last two.
    fn assert_idle_boost_declaration(hc: &HelmConsoleConfig, what: &str) {
        let boost_ai = hc.boost_ai.as_ref().unwrap_or_else(|| {
            panic!(
                "{what} — but the axis must still DECLARE that (PRD #774 US7): an \
                 omitted `[helm_console.boost_ai]` is silence, and silence is what \
                 gets a Rust-side policy synthesised for it"
            )
        });
        assert!(
            boost_ai.idle && boost_ai.rule.is_empty() && boost_ai.state.is_empty(),
            "{what} — the declaration must be an explicit `idle = true` and nothing \
             else, got {boost_ai:?}"
        );
    }

    /// The authored stateful block validates end to end through the real
    /// content-load path (`EntityConfig::from_toml` runs the validator).
    #[test]
    fn authored_stateful_block_passes_content_validation() {
        let cfg = EntityConfig::from_toml(stateful_boost_toml()).expect("parse must succeed");
        let ai = cfg
            .helm_console
            .as_ref()
            .and_then(|h| h.boost_ai.as_ref())
            .expect("boost_ai present");
        assert!(validate_fine_system_ai_policy(ai, BOOST_CHANNELS, BOOST_VERBS).is_ok());
    }

    #[test]
    fn empty_ai_declaration_is_rejected_as_silence() {
        // `[captain_console.ai]` with neither `idle` nor a rule is silence.
        let toml = r#"
name = "Silent"
[captain_console.ai]
"#;
        let err = EntityConfig::from_toml(toml).unwrap_err().to_string();
        assert!(err.contains("empty") || err.contains("idle"), "got: {err}");
    }

    #[test]
    fn explicit_idle_declaration_is_accepted() {
        let toml = r#"
name = "Idle"
[captain_console.ai]
idle = true
"#;
        let cfg = EntityConfig::from_toml(toml).expect("idle is a valid declaration");
        let ai = cfg.captain_console.unwrap().ai.unwrap();
        assert!(ai.idle);
        assert!(ai.to_policy().unwrap().idle);
    }

    // ── Per-bank weapon AI policy (issue #781) ───────────────────────────────

    #[test]
    fn default_phaser_and_blaster_bank_policies_validate_and_resolve() {
        let p = crate::entities::authored_ai_pins::shipped_policy_toml("phaser_bank");
        assert!(
            validate_fine_system_ai_policy(&p, PHASER_BANK_CHANNELS, PHASER_BANK_VERBS).is_ok()
        );
        let pp = p.to_policy().expect("phaser default resolves");
        // Baseline: unconditional fire (not idle, one rule).
        assert!(!pp.idle);
        assert_eq!(pp.rules.len(), 1);

        let b = crate::entities::authored_ai_pins::shipped_policy_toml("blaster_bank");
        assert!(
            validate_fine_system_ai_policy(&b, BLASTER_BANK_CHANNELS, BLASTER_BANK_VERBS).is_ok()
        );
        assert!(!b.to_policy().expect("blaster default resolves").idle);
    }

    #[test]
    fn phaser_bank_inline_ai_policy_parses_from_toml() {
        let toml = r#"
name = "Gunboat"

# An armed hull owes its ship-level doctrine too since issue #956, whether or
# not it authors a `[behaviour]`. `idle = true` is the in-band way to say "this
# hull turns for nothing", which keeps the fixture about the BANK policy below.
[weapons_console.ai]
idle = true

[[weapons_console.phaser_banks]]
id = "fore"
facing_deg = 0.0
fire_arc_deg = 90.0
auto_arc_deg = 60.0

[[weapons_console.phaser_banks.ai.rule]]
priority = 0
channel = "phaser_fire"
when = "fact(in_range) > 0 and fact(in_arc) > 0"
verb = "fire_phaser"
value = false
"#;
        let cfg = EntityConfig::from_toml(toml).expect("phaser bank ai must parse + validate");
        let bank = &cfg.weapons_console.unwrap().phaser_banks[0];
        let policy = bank.ai.as_ref().unwrap().to_policy().unwrap();
        assert_eq!(policy.rules.len(), 1);
        assert_eq!(
            policy.rules[0].verb,
            crate::ai::policy::AiPolicyVerb::FirePhaser
        );
    }

    #[test]
    fn blaster_bank_inline_idle_ai_policy_parses_from_toml() {
        let toml = r#"
name = "Escort"

# The SHIP-LEVEL doctrine (issue #956), distinct from the bank's own idle
# declaration below — both are owed on an armed hull with no `[behaviour]`.
[weapons_console.ai]
idle = true

[[weapons_console.blaster_banks]]
id = "fore"

[weapons_console.blaster_banks.ai]
idle = true
"#;
        let cfg = EntityConfig::from_toml(toml).expect("blaster bank idle ai must parse");
        let bank = &cfg.weapons_console.unwrap().blaster_banks[0];
        assert!(bank.ai.as_ref().unwrap().to_policy().unwrap().idle);
    }

    #[test]
    fn phaser_bank_ai_rejects_unknown_verb_at_load() {
        // The blaster verb on a phaser bank channel is an authoring error caught
        // by the from_toml validation loop, before any live tick.
        let toml = r#"
name = "Bad"

[[weapons_console.phaser_banks]]
id = "fore"
facing_deg = 0.0
fire_arc_deg = 90.0
auto_arc_deg = 60.0

[[weapons_console.phaser_banks.ai.rule]]
priority = 0
channel = "phaser_fire"
when = "true"
verb = "fire_blaster"
value = false
"#;
        let err = EntityConfig::from_toml(toml).unwrap_err().to_string();
        assert!(err.contains("unknown verb"), "got: {err}");
    }

    #[test]
    fn blaster_bank_ai_rejects_unknown_channel_at_load() {
        let toml = r#"
name = "Bad2"

[[weapons_console.blaster_banks]]
id = "fore"

[[weapons_console.blaster_banks.ai.rule]]
priority = 0
channel = "phaser_fire"
when = "true"
verb = "fire_blaster"
value = false
"#;
        let err = EntityConfig::from_toml(toml).unwrap_err().to_string();
        assert!(err.contains("unknown channel"), "got: {err}");
    }

    // ── Shields focus AI policy (issue #783) ─────────────────────────────────

    #[test]
    fn default_shields_focus_policy_validates_and_resolves() {
        let s = crate::entities::authored_ai_pins::shipped_policy_toml("shields_focus");
        assert!(
            validate_fine_system_ai_policy(&s, SHIELD_FOCUS_CHANNELS, SHIELD_FOCUS_VERBS).is_ok()
        );
        let sp = s.to_policy().expect("shields focus default resolves");
        // Baseline: a priority-10 damage rule + a priority-0 imbalance fallback.
        assert!(!sp.idle);
        assert_eq!(sp.rules.len(), 2);
        // All four authored numbers seeded as params.
        assert!(sp.params.get(SHIELD_FOCUS_DAMAGE_WINDOW_PARAM).is_some());
        assert!(sp
            .params
            .get(SHIELD_FOCUS_MIN_DAMAGE_WINDOW_PARAM)
            .is_some());
        assert!(sp.params.get(SHIELD_FOCUS_DAMAGE_PCT_PARAM).is_some());
        assert!(sp.params.get(SHIELD_FOCUS_HEALTH_RATIO_PARAM).is_some());
    }

    #[test]
    fn shields_ai_policy_parses_from_toml() {
        let toml = r#"
name = "Warden"

[shields_console.ai_policy]
param = { damage_window_secs = 6.0, min_damage_window_secs = 2.0, damage_pct_threshold = 60.0, health_ratio_threshold = 40.0 }

[[shields_console.ai_policy.rule]]
priority = 10
channel = "shield_focus"
when = "fact(recent_damage_pct_max) >= param(damage_pct_threshold)"
verb = "focus_shield_arc"

[[shields_console.ai_policy.rule]]
priority = 0
channel = "shield_focus"
when = "true"
verb = "focus_shield_arc"
"#;
        let cfg = EntityConfig::from_toml(toml).expect("shields ai_policy must parse + validate");
        let ai = cfg
            .shields_console
            .as_ref()
            .and_then(|sc| sc.ai_policy.as_ref())
            .expect("shields_console.ai_policy present");
        assert_eq!(ai.param.get("damage_window_secs"), Some(&6.0));
        assert_eq!(ai.param.get("health_ratio_threshold"), Some(&40.0));
        let policy = ai.to_policy().expect("policy resolves");
        assert_eq!(policy.rules.len(), 2);
        assert_eq!(
            policy.rules[0].verb,
            crate::ai::policy::AiPolicyVerb::FocusShieldArc
        );
    }

    // ── Power allocation AI policy (issue #784) ──────────────────────────────

    /// Minimal ship TOML carrying authored `[power_groups.*]` plus a
    /// `[power.ai_policy]` block, used by the power-policy load tests.
    fn power_policy_toml(rules: &str) -> String {
        format!(
            r#"
name = "Reactorer"

[power]
capacity = 90
rates = [ 5, 4, 3, 2, -2, -5 ]
emergency_threshold = 22

[power.ai_policy.param]
thrust_threshold = 0.7
min_reserve_helm = 50.0

{rules}

[power_groups.helm]
label = "HELM"
default_level = 2

[power_groups.weapons]
label = "WEAPONS"
default_level = 2

[power_groups.sensors]
label = "SENSORS"
default_level = 1

[power_groups.ops]
label = "OPS"
default_level = 1
"#
        )
    }

    #[test]
    fn default_power_policy_validates_and_resolves() {
        // The shipped authored block is six rules since issue #1003 — hold,
        // elevate and baseline on each of helm and weapons — all emitting the
        // value-carrying allocation verb, and every rule declares a reserve
        // param.
        let cfg = crate::entities::authored_ai_pins::shipped_policy_toml("power");
        // Validated against the canonical group channels.
        assert!(validate_fine_system_ai_policy(
            &cfg,
            &["helm", "weapons", "sensors"],
            &[POWER_SET_ALLOCATION_VERB]
        )
        .is_ok());
        let p = cfg.to_policy().expect("default power policy resolves");
        assert!(!p.idle);
        assert_eq!(p.rules.len(), 6);
        // The elevate and hold rules carry the absolute magnitude in the verb
        // payload, and the elevated level is strictly above the baseline one —
        // the authored numbers themselves are the designer's business (#885b
        // stage 5d deleted the Rust constants they used to have to match).
        let levels: Vec<u8> = p
            .rules
            .iter()
            .filter_map(|r| match r.verb {
                crate::ai::policy::AiPolicyVerb::SetPowerGroupAllocation(level) => Some(level),
                _ => None,
            })
            .collect();
        assert_eq!(levels.len(), 6, "every rule carries an allocation payload");
        assert!(
            levels.iter().max() > levels.iter().min(),
            "the elevate rules must raise their group above the baseline rules, or              the whole policy is a no-op: {levels:?}"
        );
        assert!(cfg.param.contains_key(POWER_HELM_RESERVE_PARAM));
        assert!(cfg.param.contains_key(POWER_WEAPONS_RESERVE_PARAM));
        // Each channel's SHED floor has a matching RESTORE floor above it: the
        // pair is the hysteresis, and one without the other is a ladder that
        // flips its channel every tick the charge rests on the floor.
        for (shed, restore) in [
            (POWER_HELM_RESERVE_PARAM, POWER_HELM_RESTORE_PARAM),
            (POWER_WEAPONS_RESERVE_PARAM, POWER_WEAPONS_RESTORE_PARAM),
        ] {
            let (lo, hi) = (
                cfg.param
                    .get(shed)
                    .unwrap_or_else(|| panic!("the shipped policy authors `{shed}`")),
                cfg.param
                    .get(restore)
                    .unwrap_or_else(|| panic!("the shipped policy authors `{restore}`")),
            );
            assert!(hi > lo, "`{restore}` ({hi}) must sit above `{shed}` ({lo})");
        }
    }

    #[test]
    fn power_ai_policy_parses_and_decodes_magnitude_verb_from_toml() {
        let toml = power_policy_toml(
            r#"[[power.ai_policy.rule]]
priority = 10
channel = "helm"
when = "fact(thrust) >= param(thrust_threshold) and fact(battery_pct) >= param(min_reserve_helm)"
verb = "set_power_group_allocation"
level = 3

[[power.ai_policy.rule]]
priority = 0
channel = "ops"
when = "true"
verb = "set_power_group_allocation"
level = 1"#,
        );
        let cfg = EntityConfig::from_toml(&toml).expect("power ai_policy must parse + validate");
        let ai = cfg
            .power
            .as_ref()
            .and_then(|p| p.ai_policy.as_ref())
            .expect("power.ai_policy present");
        let policy = ai.to_policy().expect("policy resolves");
        assert_eq!(policy.rules.len(), 2);
        // The magnitude decodes into the verb payload (AC: absolute level).
        assert_eq!(
            policy.rules[0].verb,
            crate::ai::policy::AiPolicyVerb::SetPowerGroupAllocation(3)
        );
    }

    #[test]
    fn power_ai_policy_rejects_non_authored_group_channel() {
        // AC1: channels are validated against the ship's AUTHORED power groups.
        // A rule targeting a group the ship does not author fails the load.
        let toml = power_policy_toml(
            r#"[[power.ai_policy.rule]]
priority = 0
channel = "shields"
when = "true"
verb = "set_power_group_allocation"
level = 2"#,
        );
        let err = EntityConfig::from_toml(&toml).unwrap_err().to_string();
        assert!(err.contains("unknown channel"), "got: {err}");
    }

    /// A hull that authors NO `[power_groups.*]` validates against the trio the
    /// runtime seeds it with, not against an empty set.
    ///
    /// `PowerSystem::from_authored_groups` falls back to
    /// `seeded_with_defaults` — helm / weapons / shields at level 2 — for an
    /// empty authored map, and `ai_power_allocation` then resolves the policy
    /// against exactly those groups. Validating against nothing would have
    /// rejected a policy the runtime was about to run, and that is not
    /// hypothetical: the six NPC hulls that declare no power groups had to
    /// author `[power.ai_policy]` in #885b stage 5c, so they are all in this
    /// state today.
    ///
    /// The negative case is `sensors`, which is the group that no longer
    /// exists. It used to be `shields` — issue #952 swapped the two over in
    /// `POWER_GROUP_ORDER`, and left as it was this test would have asserted
    /// that a channel the runtime now seeds is rejected, i.e. the exact
    /// opposite of the rule it is guarding.
    #[test]
    fn power_ai_policy_on_a_hull_with_no_authored_groups_validates_against_the_seeded_trio() {
        let toml = |channel: &str| {
            format!(
                r#"
name = "Grouper"

[power]
capacity = 100.0
rates = [ 6, 5, 4, 2, -2, -6 ]
emergency_threshold = 25.0

[[power.ai_policy.rule]]
priority = 0
channel = "{channel}"
when = "true"
verb = "set_power_group_allocation"
level = 2
"#
            )
        };
        for channel in crate::modifiers::power_system::POWER_GROUP_ORDER {
            let cfg = EntityConfig::from_toml(&toml(channel)).unwrap_or_else(|e| {
                panic!("`{channel}` is a group the runtime seeds, so it must validate: {e}")
            });
            assert!(
                cfg.ship_config
                    .as_ref()
                    .is_none_or(|sc| sc.power_groups.is_empty()),
                "precondition: this hull authors no `[power_groups.*]`"
            );
        }
        // …and the check has not simply been switched off: a group neither
        // authored nor seeded is still rejected.
        let err = EntityConfig::from_toml(&toml("sensors"))
            .expect_err("`sensors` is neither authored nor seeded")
            .to_string();
        assert!(err.contains("unknown channel"), "got: {err}");
    }

    #[test]
    fn power_ai_policy_rejects_wrong_verb_at_load() {
        let toml = power_policy_toml(
            r#"[[power.ai_policy.rule]]
priority = 0
channel = "helm"
when = "true"
verb = "focus_shield_arc"
level = 2"#,
        );
        let err = EntityConfig::from_toml(&toml).unwrap_err().to_string();
        assert!(err.contains("unknown verb"), "got: {err}");
    }

    #[test]
    fn power_ai_policy_rejects_undeclared_reserve_param() {
        // AC2 / AC6: a guard referencing an undeclared min-reserve param fails.
        let toml = power_policy_toml(
            r#"[[power.ai_policy.rule]]
priority = 10
channel = "weapons"
when = "fact(battery_pct) >= param(min_reserve_weapons)"
verb = "set_power_group_allocation"
level = 3"#,
        );
        let err = EntityConfig::from_toml(&toml).unwrap_err().to_string();
        assert!(err.contains("undeclared parameter"), "got: {err}");
    }

    #[test]
    fn power_ai_policy_rejects_unparseable_guard() {
        let toml = power_policy_toml(
            r#"[[power.ai_policy.rule]]
priority = 0
channel = "helm"
when = "fact(thrust) >>> broken"
verb = "set_power_group_allocation"
level = 2"#,
        );
        let err = EntityConfig::from_toml(&toml).unwrap_err().to_string();
        assert!(err.contains("invalid `when`"), "got: {err}");
    }

    #[test]
    fn power_ai_policy_rejects_empty_declaration() {
        let toml = power_policy_toml("");
        // `[power.ai_policy.param]` present but no rule and no idle → silence.
        let err = EntityConfig::from_toml(&toml).unwrap_err().to_string();
        assert!(err.contains("ai policy is empty"), "got: {err}");
    }

    #[test]
    fn shields_idle_ai_policy_parses_from_toml() {
        let toml = r#"
name = "Passive"

[shields_console.ai_policy]
idle = true
"#;
        let cfg = EntityConfig::from_toml(toml).expect("shields idle ai_policy must parse");
        let ai = cfg.shields_console.unwrap().ai_policy.unwrap();
        assert!(ai.to_policy().unwrap().idle);
    }

    #[test]
    fn shields_ai_policy_rejects_wrong_verb_at_load() {
        // A fire verb on the shield_focus channel is an authoring error caught by
        // the from_toml validation loop, before any live tick.
        let toml = r#"
name = "Bad"

[[shields_console.ai_policy.rule]]
priority = 0
channel = "shield_focus"
when = "true"
verb = "fire_phaser"
"#;
        let err = EntityConfig::from_toml(toml).unwrap_err().to_string();
        assert!(err.contains("unknown verb"), "got: {err}");
    }

    #[test]
    fn shields_ai_policy_rejects_unknown_channel_at_load() {
        let toml = r#"
name = "Bad2"

[[shields_console.ai_policy.rule]]
priority = 0
channel = "phaser_fire"
when = "true"
verb = "focus_shield_arc"
"#;
        let err = EntityConfig::from_toml(toml).unwrap_err().to_string();
        assert!(err.contains("unknown channel"), "got: {err}");
    }

    #[test]
    fn shields_ai_policy_rejects_undeclared_param_at_load() {
        let toml = r#"
name = "Bad3"

[[shields_console.ai_policy.rule]]
priority = 0
channel = "shield_focus"
when = "fact(recent_damage_pct_max) >= param(nonexistent)"
verb = "focus_shield_arc"
"#;
        let err = EntityConfig::from_toml(toml).unwrap_err().to_string();
        assert!(err.contains("undeclared parameter"), "got: {err}");
    }

    // ── Torpedo tube + magazine AI policy (issue #782) ───────────────────────

    #[test]
    fn default_torpedo_tube_and_magazine_policies_validate_and_resolve() {
        let t = crate::entities::authored_ai_pins::shipped_policy_toml("torpedo_tube");
        assert!(
            validate_fine_system_ai_policy(&t, TORPEDO_TUBE_CHANNELS, TORPEDO_TUBE_VERBS).is_ok()
        );
        let tp = t.to_policy().expect("tube default resolves");
        // Baseline: unconditional load + launch (not idle, two rules).
        assert!(!tp.idle);
        assert_eq!(tp.rules.len(), 2);

        let m = crate::entities::authored_ai_pins::shipped_policy_toml("torpedo_magazine");
        assert!(validate_fine_system_ai_policy(
            &m,
            TORPEDO_MAGAZINE_CHANNELS,
            TORPEDO_MAGAZINE_VERBS
        )
        .is_ok());
        assert!(!m.to_policy().expect("magazine default resolves").idle);
    }

    #[test]
    fn torpedo_tube_inline_ai_policy_parses_from_toml() {
        let toml = r#"
name = "Bomber"

[torpedoes]
count = 8

[[torpedoes.tubes]]
id = "fore_port"
facing_deg = 0.0
fire_arc_deg = 90.0

[[torpedoes.tubes.ai.rule]]
priority = 0
channel = "torpedo_load"
when = "fact(magazine) > 0"
verb = "load_torpedo"
value = false

[[torpedoes.tubes.ai.rule]]
priority = 0
channel = "torpedo_launch"
when = "fact(target_facing_shields) <= 0"
verb = "launch_torpedo"
value = false
"#;
        let cfg = EntityConfig::from_toml_in_mode(
            toml,
            crate::entities::ai_declaration_manifest::AiDeclarationMode::Lenient,
        )
        .expect("tube ai must parse + validate");
        let tube = &cfg.torpedoes.unwrap().tubes[0];
        let policy = tube.ai.as_ref().unwrap().to_policy().unwrap();
        assert_eq!(policy.rules.len(), 2);
        assert_eq!(
            policy.rules[0].verb,
            crate::ai::policy::AiPolicyVerb::LoadTorpedo
        );
        assert_eq!(
            policy.rules[1].verb,
            crate::ai::policy::AiPolicyVerb::LaunchTorpedo
        );
    }

    #[test]
    fn torpedo_magazine_inline_ai_policy_parses_from_toml() {
        let toml = r#"
name = "Bomber"

[torpedoes]
count = 8

[[torpedoes.ai.rule]]
priority = 0
channel = "torpedo_magazine_grant"
when = "fact(in_flight) < 3"
verb = "grant_torpedo_round"
value = false
"#;
        let cfg = EntityConfig::from_toml(toml).expect("magazine ai must parse + validate");
        let policy = cfg
            .torpedoes
            .unwrap()
            .ai
            .as_ref()
            .unwrap()
            .to_policy()
            .unwrap();
        assert_eq!(
            policy.rules[0].verb,
            crate::ai::policy::AiPolicyVerb::GrantTorpedoRound
        );
    }

    #[test]
    fn torpedo_tube_inline_idle_ai_policy_parses_from_toml() {
        let toml = r#"
name = "Bomber"

[torpedoes]
count = 8

[[torpedoes.tubes]]
id = "fore_port"
facing_deg = 0.0
fire_arc_deg = 90.0

[torpedoes.tubes.ai]
idle = true
"#;
        let cfg = EntityConfig::from_toml_in_mode(
            toml,
            crate::entities::ai_declaration_manifest::AiDeclarationMode::Lenient,
        )
        .expect("tube idle ai must parse");
        let tube = &cfg.torpedoes.unwrap().tubes[0];
        assert!(tube.ai.as_ref().unwrap().to_policy().unwrap().idle);
    }

    #[test]
    fn torpedo_tube_ai_rejects_unknown_verb_at_load() {
        // The magazine grant verb on a tube channel is an authoring error caught
        // by the from_toml validation loop, before any live tick.
        let toml = r#"
name = "Bad"

[torpedoes]
count = 8

[[torpedoes.tubes]]
id = "fore_port"
facing_deg = 0.0
fire_arc_deg = 90.0

[[torpedoes.tubes.ai.rule]]
priority = 0
channel = "torpedo_load"
when = "true"
verb = "grant_torpedo_round"
value = false
"#;
        let err = EntityConfig::from_toml(toml).unwrap_err().to_string();
        assert!(err.contains("unknown verb"), "got: {err}");
    }

    #[test]
    fn torpedo_magazine_ai_rejects_unknown_channel_at_load() {
        // A tube channel on the magazine block is rejected.
        let toml = r#"
name = "Bad"

[torpedoes]
count = 8

[[torpedoes.ai.rule]]
priority = 0
channel = "torpedo_launch"
when = "true"
verb = "grant_torpedo_round"
value = false
"#;
        let err = EntityConfig::from_toml(toml).unwrap_err().to_string();
        assert!(err.contains("unknown channel"), "got: {err}");
    }

    #[test]
    fn idle_with_rules_is_contradictory_and_rejected() {
        let cfg = FineSystemAiConfigToml {
            evaluate_every_ticks: default_evaluate_every_ticks(),
            idle: true,
            param: Default::default(),
            rule: vec![FineSystemAiRuleToml {
                priority: 1,
                channel: CAPTAIN_RED_ALERT_CHANNEL.into(),
                when: "true".into(),
                verb: CAPTAIN_SET_RED_ALERT_VERB.into(),
                value: true,
                level: 0,
                response_index: 0,
            }],
            initial_state: None,
            state: Vec::new(),
            memory: std::collections::HashMap::new(),
        };
        let err = validate_fine_system_ai_policy(&cfg, CHANNELS, VERBS).unwrap_err();
        assert!(err.contains("idle"), "got: {err}");
    }

    #[test]
    fn invalid_when_expression_is_rejected() {
        let toml = r#"
name = "BadExpr"
[captain_console.ai]
[[captain_console.ai.rule]]
priority = 1
channel = "red_alert"
when = "fact(x) &"
verb = "set_red_alert"
value = true
"#;
        let err = EntityConfig::from_toml(toml).unwrap_err().to_string();
        assert!(
            err.contains("invalid `when`") || err.contains("position"),
            "got: {err}"
        );
    }

    #[test]
    fn unknown_channel_is_rejected() {
        let cfg = FineSystemAiConfigToml {
            evaluate_every_ticks: default_evaluate_every_ticks(),
            idle: false,
            param: Default::default(),
            rule: vec![FineSystemAiRuleToml {
                priority: 1,
                channel: "shields".into(),
                when: "true".into(),
                verb: CAPTAIN_SET_RED_ALERT_VERB.into(),
                value: true,
                level: 0,
                response_index: 0,
            }],
            initial_state: None,
            state: Vec::new(),
            memory: std::collections::HashMap::new(),
        };
        let err = validate_fine_system_ai_policy(&cfg, CHANNELS, VERBS).unwrap_err();
        assert!(err.contains("unknown channel"), "got: {err}");
    }

    #[test]
    fn unknown_verb_is_rejected() {
        let cfg = FineSystemAiConfigToml {
            evaluate_every_ticks: default_evaluate_every_ticks(),
            idle: false,
            param: Default::default(),
            rule: vec![FineSystemAiRuleToml {
                priority: 1,
                channel: CAPTAIN_RED_ALERT_CHANNEL.into(),
                when: "true".into(),
                verb: "launch_torpedoes".into(),
                value: true,
                level: 0,
                response_index: 0,
            }],
            initial_state: None,
            state: Vec::new(),
            memory: std::collections::HashMap::new(),
        };
        let err = validate_fine_system_ai_policy(&cfg, CHANNELS, VERBS).unwrap_err();
        assert!(err.contains("unknown verb"), "got: {err}");
    }

    #[test]
    fn undeclared_parameter_reference_is_rejected() {
        let cfg = FineSystemAiConfigToml {
            evaluate_every_ticks: default_evaluate_every_ticks(),
            idle: false,
            param: Default::default(), // no params declared
            rule: vec![FineSystemAiRuleToml {
                priority: 1,
                channel: CAPTAIN_RED_ALERT_CHANNEL.into(),
                when: "fact(secs_since_combat) < param(combat_window_secs)".into(),
                verb: CAPTAIN_SET_RED_ALERT_VERB.into(),
                value: true,
                level: 0,
                response_index: 0,
            }],
            initial_state: None,
            state: Vec::new(),
            memory: std::collections::HashMap::new(),
        };
        let err = validate_fine_system_ai_policy(&cfg, CHANNELS, VERBS).unwrap_err();
        assert!(err.contains("undeclared parameter"), "got: {err}");
    }

    #[test]
    fn unknown_verb_surfaces_through_to_policy() {
        let cfg = FineSystemAiConfigToml {
            evaluate_every_ticks: default_evaluate_every_ticks(),
            idle: false,
            param: Default::default(),
            rule: vec![FineSystemAiRuleToml {
                priority: 1,
                channel: CAPTAIN_RED_ALERT_CHANNEL.into(),
                when: "true".into(),
                verb: "nope".into(),
                value: true,
                level: 0,
                response_index: 0,
            }],
            initial_state: None,
            state: Vec::new(),
            memory: std::collections::HashMap::new(),
        };
        assert!(cfg.to_policy().is_err());
    }

    // ── Helm Engines/Steering AI policy (issue #779) ─────────────────────────

    const ENGINES_CHANNELS: &[&str] = &[HELM_LONGITUDINAL_CHANNEL];
    const ENGINES_VERBS: &[&str] = &[HELM_ACTUATE_DESIRED_TRAVEL_VERB];
    const STEERING_CHANNELS: &[&str] = &[HELM_YAW_CHANNEL];
    const STEERING_VERBS: &[&str] = &[HELM_ACTUATE_DESIRED_FACING_VERB];

    #[test]
    fn default_helm_policies_validate_and_resolve() {
        let eng = crate::entities::authored_ai_pins::shipped_policy_toml("engines");
        assert!(validate_fine_system_ai_policy(&eng, ENGINES_CHANNELS, ENGINES_VERBS).is_ok());
        let eng_policy = eng.to_policy().expect("engines policy resolves");
        assert_eq!(
            eng_policy.resolve_channel(
                HELM_LONGITUDINAL_CHANNEL,
                &crate::world::flags::AiFacts::new(),
                &[]
            ),
            Some(&crate::ai::policy::AiPolicyVerb::ActuateDesiredTravel),
            "the default Engines policy actuates desired travel unconditionally"
        );

        let steer = crate::entities::authored_ai_pins::shipped_policy_toml("steering");
        assert!(validate_fine_system_ai_policy(&steer, STEERING_CHANNELS, STEERING_VERBS).is_ok());
        let steer_policy = steer.to_policy().expect("steering policy resolves");
        assert_eq!(
            steer_policy.resolve_channel(
                HELM_YAW_CHANNEL,
                &crate::world::flags::AiFacts::new(),
                &[]
            ),
            Some(&crate::ai::policy::AiPolicyVerb::ActuateDesiredFacing),
        );
    }

    #[test]
    fn authored_helm_policies_parse_and_resolve_to_typed_policy() {
        let toml = r#"
name = "Test Cruiser"

[helm_console]
max_speed = 30.0

[helm_console.engines_ai]
param = { arrival_radius = 5.0 }

[[helm_console.engines_ai.rule]]
priority = 10
channel = "longitudinal"
when = "fact(distance_to_dest) > param(arrival_radius)"
verb = "actuate_desired_travel"

[helm_console.steering_ai]
idle = true
"#;
        let cfg = EntityConfig::from_toml(toml).expect("parse must succeed");
        let hc = cfg.helm_console.as_ref().expect("helm_console present");
        let engines = hc.engines_ai.as_ref().expect("engines_ai present");
        assert_eq!(engines.param.get("arrival_radius"), Some(&5.0));
        let engines_policy = engines.to_policy().expect("engines policy resolves");
        assert_eq!(engines_policy.rules.len(), 1);
        // An explicit idle Steering policy is a legal declaration (a ship whose
        // Steering never AI-actuates), distinct from silence.
        let steering = hc.steering_ai.as_ref().expect("steering_ai present");
        assert!(steering.to_policy().expect("steering resolves").idle);
    }

    #[test]
    fn unknown_helm_engines_verb_is_rejected() {
        let cfg = FineSystemAiConfigToml {
            evaluate_every_ticks: default_evaluate_every_ticks(),
            idle: false,
            param: Default::default(),
            rule: vec![FineSystemAiRuleToml {
                priority: 1,
                channel: HELM_LONGITUDINAL_CHANNEL.into(),
                // The Steering verb on the Engines channel is unknown here.
                verb: HELM_ACTUATE_DESIRED_FACING_VERB.into(),
                when: "true".into(),
                value: false,
                level: 0,
                response_index: 0,
            }],
            initial_state: None,
            state: Vec::new(),
            memory: std::collections::HashMap::new(),
        };
        let err =
            validate_fine_system_ai_policy(&cfg, ENGINES_CHANNELS, ENGINES_VERBS).unwrap_err();
        assert!(err.contains("unknown verb"), "got: {err}");
    }

    #[test]
    fn helm_wrong_channel_is_rejected() {
        // The Captain's `red_alert` channel is not a valid Steering channel.
        let cfg = FineSystemAiConfigToml {
            evaluate_every_ticks: default_evaluate_every_ticks(),
            idle: false,
            param: Default::default(),
            rule: vec![FineSystemAiRuleToml {
                priority: 1,
                channel: CAPTAIN_RED_ALERT_CHANNEL.into(),
                verb: HELM_ACTUATE_DESIRED_FACING_VERB.into(),
                when: "true".into(),
                value: false,
                level: 0,
                response_index: 0,
            }],
            initial_state: None,
            state: Vec::new(),
            memory: std::collections::HashMap::new(),
        };
        let err =
            validate_fine_system_ai_policy(&cfg, STEERING_CHANNELS, STEERING_VERBS).unwrap_err();
        assert!(err.contains("unknown channel"), "got: {err}");
    }

    #[test]
    fn unknown_helm_verb_rejected_at_entity_load() {
        let toml = r#"
name = "BadHelm"
[helm_console]
max_speed = 30.0
[helm_console.engines_ai]
[[helm_console.engines_ai.rule]]
priority = 1
channel = "longitudinal"
when = "true"
verb = "warp_speed"
"#;
        let err = EntityConfig::from_toml(toml).unwrap_err().to_string();
        assert!(err.contains("unknown verb"), "got: {err}");
    }

    #[test]
    fn empty_helm_engines_declaration_is_rejected_as_silence() {
        let toml = r#"
name = "SilentHelm"
[helm_console]
max_speed = 30.0
[helm_console.engines_ai]
"#;
        let err = EntityConfig::from_toml(toml).unwrap_err().to_string();
        assert!(err.contains("empty") || err.contains("idle"), "got: {err}");
    }

    // ── Helm secondary-actuator AI policy (issue #780) ───────────────────────

    const LATERAL_CHANNELS: &[&str] = &[HELM_LATERAL_CHANNEL];
    const LATERAL_VERBS: &[&str] = &[HELM_ACTUATE_LATERAL_THRUST_VERB];
    const VERTICAL_CHANNELS: &[&str] = &[HELM_VERTICAL_CHANNEL];
    const VERTICAL_VERBS: &[&str] = &[HELM_ACTUATE_VERTICAL_THRUST_VERB];
    const IMPULSE_CHANNELS: &[&str] = &[HELM_IMPULSE_CHANNEL];
    const IMPULSE_VERBS: &[&str] = &[HELM_ENGAGE_IMPULSE_VERB];
    const BOOST_CHANNELS: &[&str] = &[HELM_BOOST_CHANNEL];
    const BOOST_VERBS: &[&str] = &[HELM_ENGAGE_BOOST_VERB];

    #[test]
    fn default_secondary_helm_policies_validate_and_resolve() {
        // Lateral / vertical / impulse default to unconditional actuate/permit;
        // boost defaults to explicit idle (no AI boost).
        let lat = crate::entities::authored_ai_pins::shipped_policy_toml("lateral");
        assert!(validate_fine_system_ai_policy(&lat, LATERAL_CHANNELS, LATERAL_VERBS).is_ok());
        assert_eq!(
            lat.to_policy().unwrap().resolve_channel(
                HELM_LATERAL_CHANNEL,
                &crate::world::flags::AiFacts::new(),
                &[]
            ),
            Some(&crate::ai::policy::AiPolicyVerb::ActuateLateralThrust),
        );

        let vert = crate::entities::authored_ai_pins::shipped_policy_toml("vertical");
        assert!(validate_fine_system_ai_policy(&vert, VERTICAL_CHANNELS, VERTICAL_VERBS).is_ok());
        assert_eq!(
            vert.to_policy().unwrap().resolve_channel(
                HELM_VERTICAL_CHANNEL,
                &crate::world::flags::AiFacts::new(),
                &[]
            ),
            Some(&crate::ai::policy::AiPolicyVerb::ActuateVerticalThrust),
        );

        let imp = crate::entities::authored_ai_pins::shipped_policy_toml("impulse");
        assert!(validate_fine_system_ai_policy(&imp, IMPULSE_CHANNELS, IMPULSE_VERBS).is_ok());
        assert_eq!(
            imp.to_policy().unwrap().resolve_channel(
                HELM_IMPULSE_CHANNEL,
                &crate::world::flags::AiFacts::new(),
                &[]
            ),
            Some(&crate::ai::policy::AiPolicyVerb::EngageImpulse),
        );

        let boost = crate::entities::authored_ai_pins::shipped_policy_toml("boost");
        assert!(validate_fine_system_ai_policy(&boost, BOOST_CHANNELS, BOOST_VERBS).is_ok());
        let boost_policy = boost.to_policy().unwrap();
        assert!(
            boost_policy.idle,
            "default boost policy is an explicit idle"
        );
        assert_eq!(
            boost_policy.resolve_channel(
                HELM_BOOST_CHANNEL,
                &crate::world::flags::AiFacts::new(),
                &[]
            ),
            None,
            "an idle boost policy never engages"
        );
    }

    #[test]
    fn authored_secondary_helm_policies_parse_at_entity_load() {
        let toml = r#"
name = "Test Cruiser"

[helm_console]
max_speed = 30.0

[helm_console.boost_ai.param]
boost_urgency = 0.5

[[helm_console.boost_ai.rule]]
priority = 10
channel = "boost"
when = "fact(hazard_urgency) > param(boost_urgency) and fact(boost_available) > 0"
verb = "engage_boost"

[helm_console.impulse_ai]
[[helm_console.impulse_ai.rule]]
priority = 10
channel = "impulse"
when = "fact(impulse_available) > 0"
verb = "engage_impulse"
"#;
        let cfg = EntityConfig::from_toml(toml).expect("parse must succeed");
        let hc = cfg.helm_console.as_ref().expect("helm_console present");
        let boost = hc.boost_ai.as_ref().expect("boost_ai present");
        assert_eq!(boost.to_policy().unwrap().rules.len(), 1);
        let impulse = hc.impulse_ai.as_ref().expect("impulse_ai present");
        assert_eq!(impulse.to_policy().unwrap().rules.len(), 1);
    }

    #[test]
    fn wrong_verb_on_secondary_helm_channel_is_rejected() {
        // The impulse verb on the boost channel is unknown to the boost host.
        let cfg = FineSystemAiConfigToml {
            evaluate_every_ticks: default_evaluate_every_ticks(),
            idle: false,
            param: Default::default(),
            rule: vec![FineSystemAiRuleToml {
                priority: 1,
                channel: HELM_BOOST_CHANNEL.into(),
                verb: HELM_ENGAGE_IMPULSE_VERB.into(),
                when: "true".into(),
                value: false,
                level: 0,
                response_index: 0,
            }],
            initial_state: None,
            state: Vec::new(),
            memory: std::collections::HashMap::new(),
        };
        let err = validate_fine_system_ai_policy(&cfg, BOOST_CHANNELS, BOOST_VERBS).unwrap_err();
        assert!(err.contains("unknown verb"), "got: {err}");
    }

    #[test]
    fn wrong_secondary_helm_verb_rejected_at_entity_load() {
        // Authoring the lateral verb on the vertical channel fails the load.
        let toml = r#"
name = "BadHelm"
[helm_console]
max_speed = 30.0
[helm_console.vertical_ai]
[[helm_console.vertical_ai.rule]]
priority = 1
channel = "vertical"
when = "true"
verb = "actuate_lateral_thrust"
"#;
        let err = EntityConfig::from_toml(toml).unwrap_err().to_string();
        assert!(err.contains("unknown verb"), "got: {err}");
    }

    // ── Sensors target selector schema + validation (issue #776) ─────────────

    fn sensors_selector_toml() -> &'static str {
        r##"
[sensors_console.selector]
horizon = 4000.0
switch_margin = 25.0
sources = ["combat-lock", "objective-destroy", "radar-contacts"]
eligibility = "candidate_fact(detectable) > 0 and candidate_fact(hostile) > 0"

[sensors_console.selector.param]
lock_weight = 900.0

[[sensors_console.selector.score]]
when = "candidate_fact(source_combat_lock) > 0"
weight = 900.0

[[sensors_console.selector.score]]
when = "candidate_fact(source_radar) > 0"
weight = 1.0
"##
    }

    #[test]
    fn sensors_selector_parses_and_resolves_to_typed_selector() {
        let config = EntityConfig::from_toml(sensors_selector_toml()).expect("parse must succeed");
        let sel = config
            .sensors_console
            .as_ref()
            .and_then(|c| c.selector.as_ref())
            .expect("selector section present");
        let resolved = sel.to_selector().expect("selector resolves");
        assert_eq!(resolved.horizon, 4000.0);
        assert_eq!(resolved.switch_margin, 25.0);
        assert_eq!(resolved.score.len(), 2);
        assert!(validate_fine_system_ai_selector(sel, SENSORS_SELECTOR_SOURCES).is_ok());
    }

    #[test]
    fn default_sensors_selector_is_valid_and_resolves() {
        let cfg = crate::entities::authored_ai_pins::shipped_selector_toml("sensors");
        assert!(validate_fine_system_ai_selector(&cfg, SENSORS_SELECTOR_SOURCES).is_ok());
        let resolved = cfg.to_selector().expect("default selector resolves");
        assert_eq!(resolved.score.len(), 3);
    }

    #[test]
    fn selector_unknown_source_is_rejected() {
        let mut cfg = crate::entities::authored_ai_pins::shipped_selector_toml("sensors");
        cfg.sources.push("mystery-source".into());
        let err = validate_fine_system_ai_selector(&cfg, SENSORS_SELECTOR_SOURCES).unwrap_err();
        assert!(err.contains("mystery-source"), "got: {err}");
    }

    #[test]
    fn selector_unparseable_eligibility_is_rejected() {
        let mut cfg = crate::entities::authored_ai_pins::shipped_selector_toml("sensors");
        cfg.eligibility = "candidate_fact(hostile) >".into();
        let err = validate_fine_system_ai_selector(&cfg, SENSORS_SELECTOR_SOURCES).unwrap_err();
        assert!(err.contains("eligibility"), "got: {err}");
    }

    #[test]
    fn selector_undeclared_param_reference_is_rejected() {
        let mut cfg = crate::entities::authored_ai_pins::shipped_selector_toml("sensors");
        cfg.eligibility = "self_fact(power_rating) >= param(never_declared)".into();
        let err = validate_fine_system_ai_selector(&cfg, SENSORS_SELECTOR_SOURCES).unwrap_err();
        assert!(err.contains("never_declared"), "got: {err}");
    }

    #[test]
    fn selector_bad_content_fails_entity_load() {
        let bad = r##"
[sensors_console.selector]
horizon = 100.0
switch_margin = 0.0
sources = ["not-a-real-source"]
eligibility = "candidate_fact(detectable) > 0"
"##;
        assert!(
            EntityConfig::from_toml(bad).is_err(),
            "unknown selector source must fail from_toml before world activation"
        );
    }

    // ── Navigation target selector schema + validation (issue #778) ──────────

    fn navigation_selector_toml() -> &'static str {
        r##"
[navigation_console.selector]
horizon = 5000.0
switch_margin = 30.0
sources = ["navigation-objectives", "chart-contacts"]
eligibility = "candidate_fact(reachable) > 0"

[navigation_console.selector.param]
objective_weight = 200.0

[[navigation_console.selector.score]]
when = "candidate_fact(source_nav_objective) > 0"
weight = 200.0

[[navigation_console.selector.score]]
when = "candidate_fact(source_chart_contact) > 0"
weight = 1.0
"##
    }

    #[test]
    fn navigation_selector_parses_and_resolves_to_typed_selector() {
        let config =
            EntityConfig::from_toml(navigation_selector_toml()).expect("parse must succeed");
        let sel = config
            .navigation_console
            .as_ref()
            .and_then(|c| c.selector.as_ref())
            .expect("selector section present");
        let resolved = sel.to_selector().expect("selector resolves");
        assert_eq!(resolved.horizon, 5000.0);
        assert_eq!(resolved.switch_margin, 30.0);
        assert_eq!(resolved.score.len(), 2);
        assert!(validate_fine_system_ai_selector(sel, NAVIGATION_SELECTOR_SOURCES).is_ok());
    }

    #[test]
    fn default_navigation_selector_is_valid_and_resolves() {
        let cfg = crate::entities::authored_ai_pins::shipped_selector_toml("navigation");
        assert!(validate_fine_system_ai_selector(&cfg, NAVIGATION_SELECTOR_SOURCES).is_ok());
        let resolved = cfg.to_selector().expect("default selector resolves");
        // objective + chart-contact tiers.
        assert_eq!(resolved.score.len(), 2);
    }

    #[test]
    fn navigation_selector_unknown_source_is_rejected() {
        let mut cfg = crate::entities::authored_ai_pins::shipped_selector_toml("navigation");
        cfg.sources.push("radar-contacts".into());
        let err = validate_fine_system_ai_selector(&cfg, NAVIGATION_SELECTOR_SOURCES).unwrap_err();
        assert!(err.contains("radar-contacts"), "got: {err}");
    }

    #[test]
    fn navigation_selector_bad_content_fails_entity_load() {
        let bad = r##"
[navigation_console.selector]
horizon = 100.0
switch_margin = 0.0
sources = ["not-a-real-source"]
eligibility = "candidate_fact(reachable) > 0"
"##;
        assert!(
            EntityConfig::from_toml(bad).is_err(),
            "unknown selector source must fail from_toml before world activation"
        );
    }

    // ── Tactical target selector schema + validation (issue #777) ────────────

    fn tactical_selector_toml() -> &'static str {
        r##"
[weapons_console.selector]
horizon = 3000.0
switch_margin = 40.0
sources = ["sensors-designation", "objective-destroy", "last-attacker", "radar-contacts"]
eligibility = "candidate_fact(detectable) > 0 and (candidate_fact(source_objective) > 0 or candidate_fact(hostile) > 0)"

[weapons_console.selector.param]
sensors_designation_weight = 800.0

[[weapons_console.selector.score]]
when = "candidate_fact(source_sensors_designation) > 0"
weight = 800.0

[[weapons_console.selector.score]]
when = "candidate_fact(source_radar) > 0"
weight = 1.0
"##
    }

    #[test]
    fn tactical_selector_parses_and_resolves_to_typed_selector() {
        // Lenient: the fixture's `[weapons_console]` owes a `weapons_doctrine`
        // declaration since issue #956, and this test is about the SELECTOR
        // schema beside it.
        let config = EntityConfig::from_toml_in_mode(
            tactical_selector_toml(),
            crate::entities::ai_declaration_manifest::AiDeclarationMode::Lenient,
        )
        .expect("parse must succeed");
        let sel = config
            .weapons_console
            .as_ref()
            .and_then(|c| c.selector.as_ref())
            .expect("selector section present");
        let resolved = sel.to_selector().expect("selector resolves");
        assert_eq!(resolved.horizon, 3000.0);
        assert_eq!(resolved.switch_margin, 40.0);
        assert_eq!(resolved.score.len(), 2);
        assert!(validate_fine_system_ai_selector(sel, TACTICAL_SELECTOR_SOURCES).is_ok());
    }

    #[test]
    fn default_tactical_selector_is_valid_and_resolves() {
        let cfg = crate::entities::authored_ai_pins::shipped_selector_toml("tactical");
        assert!(validate_fine_system_ai_selector(&cfg, TACTICAL_SELECTOR_SOURCES).is_ok());
        let resolved = cfg.to_selector().expect("default selector resolves");
        // objective, sensors-designation, retained, last-attacker, radar.
        assert_eq!(resolved.score.len(), 5);
    }

    /// The precedence invariant that prevents the #777 additive-stacking bug:
    /// the objective weight must strictly dominate the maximum non-objective
    /// stack (`sensors_designation + retained + last_attacker + radar`) by more
    /// than `switch_margin`, so an in-range named Destroy objective always wins
    /// the ranking AND survives hysteresis retention — even against the ship's
    /// own current lock coinciding with its Sensors designation.
    ///
    /// This was a `const {}` block over the synthesiser's Rust constants until
    /// #885b stage 5d deleted them. It is now the arithmetic form of the same
    /// invariant read off the SHIPPED authored weights, so a designer retuning
    /// the block in TOML is held to it — which is what the constants were really
    /// standing in for. The behavioural form lives in
    /// `entities::authored_ai_pins::tactical_objective_beats_the_maximum_non_objective_stack`.
    #[test]
    fn shipped_tactical_selector_objective_dominates_max_non_objective_stack() {
        let cfg = crate::entities::authored_ai_pins::shipped_selector_toml("tactical");
        let weight = |fact: &str| {
            cfg.score
                .iter()
                .find(|t| t.when.contains(fact))
                .unwrap_or_else(|| panic!("the authored Tactical selector scores `{fact}`"))
                .weight
        };
        let max_non_objective = weight("source_sensors_designation")
            + weight("source_retained")
            + weight("source_last_attacker")
            + weight("source_radar");
        assert!(
            max_non_objective < weight("source_objective") - cfg.switch_margin,
            "objective must dominate the max non-objective stack by more than the \
             switch margin, or a stacked non-objective candidate can beat — or be \
             retained over — an explicit Destroy objective (#777)."
        );
        assert!(
            weight("source_retained") > weight("source_last_attacker"),
            "retention must still outrank a fresh last attacker so an established \
             engagement is not broken off (the retired tier-2 > tier-3 ordering)."
        );
    }

    #[test]
    fn tactical_selector_rejects_combat_lock_source() {
        // `combat-lock` is Tactical's OWN output — unioning it would be
        // circular, so it is not a registered Tactical source.
        let mut cfg = crate::entities::authored_ai_pins::shipped_selector_toml("tactical");
        cfg.sources.push(SELECTOR_SOURCE_COMBAT_LOCK.into());
        let err = validate_fine_system_ai_selector(&cfg, TACTICAL_SELECTOR_SOURCES).unwrap_err();
        assert!(err.contains(SELECTOR_SOURCE_COMBAT_LOCK), "got: {err}");
    }

    #[test]
    fn tactical_selector_bad_content_fails_entity_load() {
        let bad = r##"
[weapons_console.selector]
horizon = 100.0
switch_margin = 0.0
sources = ["not-a-real-source"]
eligibility = "candidate_fact(detectable) > 0"
"##;
        // Assert on the error TEXT, not just `is_err()`: since issue #956 a
        // bare `[weapons_console.selector]` also owes a `weapons_doctrine`
        // declaration under strict mode, so a bare `is_err()` here would keep
        // passing — for the wrong reason — if the unknown-source validation
        // were ever weakened, because the doctrine check runs later in
        // `from_toml` and would still reject the fixture. Pinning the message
        // to the source name keeps this test load-bearing on the Tactical
        // selector-source validation it names.
        let err = EntityConfig::from_toml(bad).unwrap_err().to_string();
        assert!(
            err.contains("not-a-real-source"),
            "unknown Tactical selector source must fail from_toml before world \
             activation, got: {err}"
        );
    }

    // ── Repair selector (issue #785) ────────────────────────────────────────

    /// BASELINE PRESERVATION: the shipped Repair selector reproduces the retired
    /// `(tier desc, deficit desc)` comparator, so a single damage-tier step must
    /// strictly dominate the entire deficit ladder.
    ///
    /// Read off the AUTHORED block since #885b stage 5d deleted the constants
    /// this used to be a `const {}` block over. The behavioural form lives in
    /// `entities::authored_ai_pins::repair_one_tier_step_beats_the_whole_deficit_ladder`.
    #[test]
    fn shipped_repair_selector_tier_dominates_max_deficit_stack() {
        let cfg = crate::entities::authored_ai_pins::shipped_selector_toml("repair");
        let tier: Vec<f32> = cfg
            .score
            .iter()
            .filter(|t| t.when.contains("tier_ordinal"))
            .map(|t| t.weight)
            .collect();
        let deficit: Vec<f32> = cfg
            .score
            .iter()
            .filter(|t| t.when.contains("damage_fraction"))
            .map(|t| t.weight)
            .collect();
        assert_eq!(tier.len(), 3, "three tier rungs");
        assert_eq!(deficit.len(), 3, "three deficit bands");
        let max_deficit_stack: f32 = deficit.iter().sum();
        let one_tier_step = tier[0];
        assert!(
            max_deficit_stack < one_tier_step - cfg.switch_margin,
            "the whole deficit ladder must lose to ONE tier step, hysteresis included, \
             or the AI starts sending teams to nearly-dead minor stations ahead of \
             disabled critical ones."
        );

        let band = |key: &str| {
            *cfg.param
                .get(key)
                .unwrap_or_else(|| panic!("the authored Repair selector declares `{key}`"))
        };
        let (low, mid, high) = (
            band("deficit_band_low"),
            band("deficit_band_mid"),
            band("deficit_band_high"),
        );
        assert!(low < mid && mid < high && high < 1.0, "a monotone ladder");
        // ...and they sit INSIDE the urgent range, strictly above the
        // Damaged→Disabled damage-fraction boundary (1 − 0.25 HP). Bands placed
        // AT the tier thresholds all fire together for every Disabled station
        // and discriminate nothing.
        assert!(low > 1.0 - 0.25);
    }

    #[test]
    fn default_repair_selector_config_validates() {
        let cfg = crate::entities::authored_ai_pins::shipped_selector_toml("repair");
        assert!(
            validate_fine_system_ai_selector(&cfg, REPAIR_SELECTOR_SOURCES).is_ok(),
            "the canonical Repair selector must validate against its own sources"
        );
        assert!(
            cfg.to_selector().is_ok(),
            "the canonical Repair selector must resolve to a typed selector"
        );
    }

    #[test]
    fn repair_selector_rejects_unregistered_source() {
        let mut cfg = crate::entities::authored_ai_pins::shipped_selector_toml("repair");
        cfg.sources.push(SELECTOR_SOURCE_RADAR_CONTACTS.into());
        let err = validate_fine_system_ai_selector(&cfg, REPAIR_SELECTOR_SOURCES).unwrap_err();
        assert!(err.contains(SELECTOR_SOURCE_RADAR_CONTACTS), "got: {err}");
    }

    #[test]
    fn repair_selector_undeclared_param_is_rejected() {
        let mut cfg = crate::entities::authored_ai_pins::shipped_selector_toml("repair");
        cfg.eligibility = "candidate_fact(damage_fraction) >= param(nope)".to_string();
        let err = validate_fine_system_ai_selector(&cfg, REPAIR_SELECTOR_SOURCES).unwrap_err();
        assert!(err.contains("nope"), "got: {err}");
    }

    /// `[repair.selector]` is the first selector block outside a `*_console`
    /// section; it parses, and bad content fails the entity load before any
    /// live tick.
    #[test]
    fn repair_selector_parses_from_toml_and_bad_content_fails_entity_load() {
        let good = r##"
[repair]
repair_team_count = 2

[repair.selector]
horizon = 1000.0
switch_margin = 0.0
sources = ["damaged-stations", "core-bucket"]
eligibility = "candidate_fact(source_repair_request) > 0"

[[repair.selector.score]]
when = "candidate_fact(tier_ordinal) >= 2"
weight = 100.0
"##;
        let cfg = EntityConfig::from_toml(good).expect("valid [repair.selector] must parse");
        let sel = cfg
            .repair
            .expect("repair section present")
            .selector
            .expect("selector present");
        assert_eq!(sel.sources.len(), 2);
        assert_eq!(sel.score.len(), 1);

        let bad = r##"
[repair]
repair_team_count = 2

[repair.selector]
horizon = 1000.0
switch_margin = 0.0
sources = ["not-a-real-source"]
eligibility = "candidate_fact(source_repair_request) > 0"
"##;
        assert!(
            EntityConfig::from_toml(bad).is_err(),
            "unknown Repair selector source must fail from_toml before world activation"
        );
    }

    #[test]
    fn repair_config_without_selector_defaults_to_none() {
        let cfg = EntityConfig::from_toml("[repair]\nrepair_team_count = 2\n")
            .expect("parse must succeed");
        assert!(cfg.repair.expect("repair present").selector.is_none());
    }

    // ── Comms console AI (issue #786) ───────────────────────────────────────

    /// BAND PLACEMENT (the #785 lesson): the objective-score ladder must be a
    /// strictly increasing set of thresholds that actually straddles the
    /// population of authored `base_priority` values (20 … 100), or every hail
    /// scores identically and the "ranking" collapses onto the selector's
    /// smallest-UUID tie-break.
    ///
    /// Read off the AUTHORED block since #885b stage 5d deleted the constants
    /// this used to be a `const {}` block over. The behavioural form lives in
    /// `entities::authored_ai_pins::comms_band_ladder_ranks_hails_by_objective_utility`.
    #[test]
    fn shipped_comms_selector_bands_are_a_monotone_ladder_over_real_scores() {
        let cfg = crate::entities::authored_ai_pins::shipped_selector_toml("comms_hail");
        let band = |key: &str| {
            *cfg.param
                .get(key)
                .unwrap_or_else(|| panic!("the authored Comms selector declares `{key}`"))
        };
        let (low, mid, high) = (
            band("score_band_low"),
            band("score_band_mid"),
            band("score_band_high"),
        );
        assert!(low < mid && mid < high, "a monotone ladder");
        // Straddles the shipped authoring range: the lowest band sits above the
        // cheapest authored priority (20) and the highest below the dearest
        // (100), so all four buckets are reachable.
        assert!(low > 20.0);
        assert!(high < 100.0);
        // A hail is a one-shot event: nothing to retain, so no hysteresis.
        assert_eq!(cfg.switch_margin, 0.0);
    }

    #[test]
    fn default_comms_selector_config_validates() {
        let cfg = crate::entities::authored_ai_pins::shipped_selector_toml("comms_hail");
        assert!(
            validate_fine_system_ai_selector(&cfg, COMMS_SELECTOR_SOURCES).is_ok(),
            "the canonical Comms selector must validate against its own sources"
        );
        assert!(
            cfg.to_selector().is_ok(),
            "the canonical Comms selector must resolve to a typed selector"
        );
    }

    /// The two eligibility terms the #786 review added, pinned by name so a
    /// future edit cannot quietly drop them:
    ///   - `has_open_hail_thread` (NOT `has_unread_from_sender`) is the
    ///     anti-respam gate — it must key on hails WE issued, or a
    ///     scenario-pushed greeting permanently suppresses a legitimate hail;
    ///   - `self_fact(comms_available)` is the AC2 system-availability gate,
    ///     which the AC names explicitly and which nothing else in the hail path
    ///     enforces.
    #[test]
    fn default_comms_selector_eligibility_names_the_anti_respam_and_availability_gates() {
        let cfg = crate::entities::authored_ai_pins::shipped_selector_toml("comms_hail");
        assert!(
            cfg.eligibility
                .contains("candidate_fact(has_open_hail_thread) < 1"),
            "got: {}",
            cfg.eligibility
        );
        assert!(
            !cfg.eligibility.contains("has_unread_from_sender"),
            "inbound traffic of unknown provenance must NOT gate hailing; got: {}",
            cfg.eligibility
        );
        assert!(
            cfg.eligibility.contains("self_fact(comms_available) > 0"),
            "got: {}",
            cfg.eligibility
        );
    }

    #[test]
    fn comms_selector_rejects_unregistered_source() {
        let mut cfg = crate::entities::authored_ai_pins::shipped_selector_toml("comms_hail");
        cfg.sources.push(SELECTOR_SOURCE_RADAR_CONTACTS.into());
        let err = validate_fine_system_ai_selector(&cfg, COMMS_SELECTOR_SOURCES).unwrap_err();
        assert!(err.contains(SELECTOR_SOURCE_RADAR_CONTACTS), "got: {err}");
    }

    #[test]
    fn comms_selector_undeclared_param_is_rejected() {
        let mut cfg = crate::entities::authored_ai_pins::shipped_selector_toml("comms_hail");
        cfg.eligibility = "candidate_fact(objective_score) >= param(nope)".to_string();
        let err = validate_fine_system_ai_selector(&cfg, COMMS_SELECTOR_SOURCES).unwrap_err();
        assert!(err.contains("nope"), "got: {err}");
    }

    #[test]
    fn comms_selector_bad_guard_is_rejected() {
        let mut cfg = crate::entities::authored_ai_pins::shipped_selector_toml("comms_hail");
        cfg.eligibility = "candidate_fact(in_range) >>> 0".to_string();
        assert!(validate_fine_system_ai_selector(&cfg, COMMS_SELECTOR_SOURCES).is_err());
    }

    /// BASELINE PRESERVATION: the canonical response policy reproduces the
    /// retired `handle_comms_channel2` stub's decision — a single rule answering
    /// with index 0 — while routing it through admission.
    ///
    /// The rule is not `when = "true"`. The stub ran ONLY on channel-2 arrival,
    /// so it could not repeat and its sender was in range by construction; this
    /// policy is re-resolved every tick against every open dialogue. The two
    /// guard terms restore exactly those two implicit preconditions:
    /// `sender_in_range` (or the router rejects the response, forever) and
    /// `comms_available` (AC2 — a Destroyed Comms system answers nothing).
    #[test]
    fn default_comms_response_ai_config_reproduces_the_retired_stub_decision() {
        let cfg = crate::entities::authored_ai_pins::shipped_policy_toml("comms_response");
        assert!(
            validate_fine_system_ai_policy(&cfg, COMMS_RESPOND_CHANNELS, COMMS_RESPOND_VERBS)
                .is_ok(),
            "the canonical Comms response policy must validate"
        );
        assert_eq!(cfg.rule.len(), 1);
        assert_eq!(cfg.rule[0].channel, COMMS_RESPOND_CHANNEL);
        assert_eq!(
            cfg.rule[0].when, "fact(comms_available) > 0 and fact(sender_in_range) > 0",
            "AC2 system availability and the router's range precondition are both \
             named — an unguarded rule re-emits rejected responses every tick"
        );
        assert_eq!(
            cfg.rule[0].response_index, 0,
            "the shipped policy answers with the FIRST response, reproducing the              retired `record_response(id, 0)` stub's decision"
        );
        let policy = cfg.to_policy().expect("must resolve to a typed policy");
        assert_eq!(
            policy.rules[0].verb,
            crate::ai::policy::AiPolicyVerb::RespondToMessage(0),
            "the authored response_index must ride the verb"
        );
    }

    /// The `response_index` payload decodes onto the verb (the SECOND
    /// value-carrying verb, after `set_power_group_allocation`), and is a
    /// SEPARATE field from `level` so a rule's meaning never depends on its verb.
    #[test]
    fn comms_respond_verb_decodes_its_own_response_index_field() {
        let cfg = FineSystemAiConfigToml {
            evaluate_every_ticks: default_evaluate_every_ticks(),
            idle: false,
            param: std::collections::HashMap::new(),
            rule: vec![FineSystemAiRuleToml {
                priority: 5,
                channel: COMMS_RESPOND_CHANNEL.to_string(),
                when: "true".to_string(),
                verb: COMMS_RESPOND_VERB.to_string(),
                value: true,
                // A non-zero `level` must be ignored by this verb.
                level: 3,
                response_index: 2,
            }],
            initial_state: None,
            state: Vec::new(),
            memory: std::collections::HashMap::new(),
        };
        let policy = cfg.to_policy().expect("must resolve");
        assert_eq!(
            policy.rules[0].verb,
            crate::ai::policy::AiPolicyVerb::RespondToMessage(2)
        );
    }

    #[test]
    fn comms_response_policy_rejects_wrong_verb_and_unknown_channel() {
        let mut wrong_verb =
            crate::entities::authored_ai_pins::shipped_policy_toml("comms_response");
        wrong_verb.rule[0].verb = POWER_SET_ALLOCATION_VERB.to_string();
        let err = validate_fine_system_ai_policy(
            &wrong_verb,
            COMMS_RESPOND_CHANNELS,
            COMMS_RESPOND_VERBS,
        )
        .unwrap_err();
        assert!(err.contains(POWER_SET_ALLOCATION_VERB), "got: {err}");

        let mut wrong_channel =
            crate::entities::authored_ai_pins::shipped_policy_toml("comms_response");
        wrong_channel.rule[0].channel = "shield_focus".to_string();
        let err = validate_fine_system_ai_policy(
            &wrong_channel,
            COMMS_RESPOND_CHANNELS,
            COMMS_RESPOND_VERBS,
        )
        .unwrap_err();
        assert!(err.contains("shield_focus"), "got: {err}");
    }

    #[test]
    fn comms_response_policy_rejects_undeclared_param() {
        let mut cfg = crate::entities::authored_ai_pins::shipped_policy_toml("comms_response");
        cfg.rule[0].when = "fact(response_count) > param(nope)".to_string();
        let err = validate_fine_system_ai_policy(&cfg, COMMS_RESPOND_CHANNELS, COMMS_RESPOND_VERBS)
            .unwrap_err();
        assert!(err.contains("nope"), "got: {err}");
    }

    /// `[comms_console]` carries BOTH machines, parses, and bad content in
    /// either fails the entity load before any live tick.
    #[test]
    fn comms_console_parses_both_blocks_and_bad_content_fails_entity_load() {
        let good = r##"
[comms_console.selector]
horizon = 1000.0
switch_margin = 0.0
sources = ["hail-objectives", "comms-contacts"]
eligibility = "candidate_fact(source_hail_objective) > 0 and candidate_fact(in_range) > 0"

[[comms_console.selector.score]]
when = "candidate_fact(objective_score) > 0"
weight = 100.0

[[comms_console.ai.rule]]
priority = 10
channel = "comms_respond"
when = "fact(is_urgent) > 0"
verb = "respond_to_message"
response_index = 1
"##;
        let cfg = EntityConfig::from_toml(good).expect("valid [comms_console] must parse");
        let console = cfg.comms_console.expect("comms_console present");
        let sel = console.selector.expect("selector present");
        assert_eq!(sel.sources.len(), 2);
        assert_eq!(sel.score.len(), 1);
        let ai = console.ai.expect("ai present");
        assert_eq!(ai.rule.len(), 1);
        assert_eq!(ai.rule[0].response_index, 1);

        let bad_source = r##"
[comms_console.selector]
horizon = 1000.0
switch_margin = 0.0
sources = ["radar-contacts"]
eligibility = "candidate_fact(source_hail_objective) > 0"
"##;
        assert!(
            EntityConfig::from_toml(bad_source).is_err(),
            "unknown Comms selector source must fail from_toml"
        );

        let bad_guard = r##"
[comms_console.selector]
horizon = 1000.0
switch_margin = 0.0
sources = ["hail-objectives"]
eligibility = "candidate_fact(in_range) >>> 0"
"##;
        assert!(
            EntityConfig::from_toml(bad_guard).is_err(),
            "an unparseable Comms selector guard must fail from_toml"
        );

        let undeclared_param = r##"
[comms_console.selector]
horizon = 1000.0
switch_margin = 0.0
sources = ["hail-objectives"]
eligibility = "candidate_fact(objective_score) > param(nope)"
"##;
        assert!(
            EntityConfig::from_toml(undeclared_param).is_err(),
            "an undeclared selector param must fail from_toml"
        );

        let bad_verb = r##"
[[comms_console.ai.rule]]
priority = 0
channel = "comms_respond"
when = "true"
verb = "set_red_alert"
"##;
        assert!(
            EntityConfig::from_toml(bad_verb).is_err(),
            "a non-Comms verb on the comms_respond channel must fail from_toml"
        );

        let bad_channel = r##"
[[comms_console.ai.rule]]
priority = 0
channel = "not_a_channel"
when = "true"
verb = "respond_to_message"
"##;
        assert!(
            EntityConfig::from_toml(bad_channel).is_err(),
            "an unknown Comms channel must fail from_toml"
        );
    }

    /// `[comms]` (per-entity comms RANGE) and `[comms_console]` (the console's
    /// AI) are deliberately different sections; authoring one must not imply the
    /// other.
    #[test]
    fn comms_range_section_does_not_carry_the_console_ai() {
        let cfg = EntityConfig::from_toml("[comms]\nrange = 8000.0\n").expect("parse must succeed");
        assert!(cfg.comms.is_some());
        assert!(
            cfg.comms_console.is_none(),
            "[comms] is the entity's comms RANGE; the console AI lives in [comms_console]"
        );
    }

    #[test]
    fn comms_config_parses_range_from_toml() {
        let toml_str = r##"
[comms]
range = 8000.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let comms = config.comms.expect("comms section present");
        assert_eq!(comms.range, 8000.0);
    }

    #[test]
    fn comms_config_is_none_when_section_absent() {
        let config = EntityConfig::from_toml("").expect("parse must succeed");
        assert!(config.comms.is_none(), "no [comms] → field is None");
    }

    // â"€â"€ LOD selection â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€

    /// Two levels: GLB near (< 50), sphere fallback beyond.
    fn two_level_lods() -> Vec<LodLevel> {
        vec![
            LodLevel {
                max_distance: Some(50.0),
                model: Some("assets/models/rock.glb".into()),
                ..Default::default()
            },
            LodLevel {
                max_distance: None,
                shape: Some(MeshShape::Sphere),
                ..Default::default()
            },
        ]
    }

    #[test]
    fn select_lod_empty_returns_zero() {
        assert_eq!(select_lod(&[], 123.0, None), 0);
        assert_eq!(select_lod(&[], 0.0, Some(3)), 0);
    }

    #[test]
    fn select_lod_single_level_always_zero() {
        let levels = vec![LodLevel {
            max_distance: None,
            shape: Some(MeshShape::Sphere),
            ..Default::default()
        }];
        assert_eq!(select_lod(&levels, 0.0, None), 0);
        assert_eq!(select_lod(&levels, 9999.0, None), 0);
        assert_eq!(select_lod(&levels, 9999.0, Some(0)), 0);
    }

    #[test]
    fn select_lod_basic_band_selection() {
        let levels = two_level_lods();
        // Near band → level 0.
        assert_eq!(select_lod(&levels, 0.0, None), 0);
        assert_eq!(select_lod(&levels, 49.0, None), 0);
        // Far band → level 1.
        assert_eq!(select_lod(&levels, 60.0, None), 1);
        assert_eq!(select_lod(&levels, 100_000.0, None), 1);
    }

    #[test]
    fn select_lod_boundary_is_exclusive_upper() {
        let levels = two_level_lods();
        // Exactly at the boundary belongs to the far band (upper bound exclusive).
        assert_eq!(select_lod(&levels, 50.0, None), 1);
        // Just below stays near.
        assert_eq!(select_lod(&levels, 49.999, None), 0);
    }

    #[test]
    fn select_lod_hysteresis_holds_when_moving_outward() {
        let levels = two_level_lods();
        // Currently at near level 0; distance crept just past the boundary but
        // within the margin → hold at 0.
        assert_eq!(select_lod(&levels, 52.0, Some(0)), 0);
        assert_eq!(
            select_lod(&levels, 50.0 + LOD_HYSTERESIS_MARGIN, Some(0)),
            0,
            "exactly boundary + margin still holds (strict >)"
        );
        // Clear of the margin → switch outward to level 1.
        assert_eq!(
            select_lod(&levels, 50.0 + LOD_HYSTERESIS_MARGIN + 0.1, Some(0)),
            1
        );
    }

    #[test]
    fn select_lod_hysteresis_holds_when_moving_inward() {
        let levels = two_level_lods();
        // Currently at far level 1; distance dropped just below the boundary but
        // within the margin → hold at 1.
        assert_eq!(select_lod(&levels, 48.0, Some(1)), 1);
        assert_eq!(
            select_lod(&levels, 50.0 - LOD_HYSTERESIS_MARGIN, Some(1)),
            1,
            "exactly boundary - margin still holds (strict <)"
        );
        // Clear of the margin → switch inward to level 0.
        assert_eq!(
            select_lod(&levels, 50.0 - LOD_HYSTERESIS_MARGIN - 0.1, Some(1)),
            0
        );
    }

    #[test]
    fn select_lod_no_thrash_across_repeated_calls_at_boundary() {
        let levels = two_level_lods();
        // Sit right on the boundary and re-evaluate repeatedly: whatever level we
        // start at, we should stay there (no oscillation).
        let mut level = 0usize;
        for _ in 0..10 {
            level = select_lod(&levels, 50.0, Some(level));
        }
        assert_eq!(level, 0, "started near, stays near at the boundary");

        let mut level = 1usize;
        for _ in 0..10 {
            level = select_lod(&levels, 50.0, Some(level));
        }
        assert_eq!(level, 1, "started far, stays far at the boundary");
    }

    #[test]
    fn select_lod_three_levels_and_out_of_range() {
        let levels = vec![
            LodLevel {
                max_distance: Some(30.0),
                model: Some("a.glb".into()),
                ..Default::default()
            },
            LodLevel {
                max_distance: Some(80.0),
                shape: Some(MeshShape::Sphere),
                ..Default::default()
            },
            LodLevel {
                max_distance: None,
                shape: Some(MeshShape::Sphere),
                ..Default::default()
            },
        ];
        assert_eq!(select_lod(&levels, 10.0, None), 0);
        assert_eq!(select_lod(&levels, 50.0, None), 1);
        assert_eq!(select_lod(&levels, 500.0, None), 2);
        // Negative / zero distances clamp to the nearest band.
        assert_eq!(select_lod(&levels, -5.0, None), 0);
        // A stale current index beyond the list is clamped and re-resolved.
        assert_eq!(select_lod(&levels, 500.0, Some(99)), 2);
    }

    // ── `[[mesh.lod]]` has moved to the model sidecar (issue #914) ─────────

    /// The old location cannot come back silently: rejected at parse, with a
    /// message that names the sidecar the chain belongs in — not the generic
    /// "unknown field `lod`" that `deny_unknown_fields` would emit.
    #[test]
    fn mesh_lod_in_entity_toml_is_rejected_with_a_targeted_message() {
        let toml_str = r##"
[mesh]
model = "assets/models/rock.glb"
variant = "small"
shape = "sphere"
colour = [0.5, 0.5, 0.5]
radius = 2.0

[[mesh.lod]]
max_distance = 50.0
model = "assets/models/rock.glb"
"##;
        let err = EntityConfig::from_toml(toml_str)
            .expect_err("[[mesh.lod]] must not parse from an entity TOML");
        let msg = err.to_string();
        assert!(
            msg.contains("assets/models/rock.small.toml"),
            "the error must name the sidecar the chain moved to; got: {msg}"
        );
        assert!(
            msg.contains("[[lod]]"),
            "the error must name the new block; got: {msg}"
        );
        assert!(
            !msg.contains("unknown field"),
            "the targeted check must run before deny_unknown_fields; got: {msg}"
        );
    }

    /// A template with no `model` still gets a pointer, just a generic one —
    /// the check must not depend on the mesh naming a GLB.
    #[test]
    fn mesh_lod_is_rejected_even_without_a_model_reference() {
        let toml_str = "[mesh]\nshape = \"sphere\"\ncolour = [0.5, 0.5, 0.5]\n\n[[mesh.lod]]\nshape = \"sphere\"\n";
        let err = EntityConfig::from_toml(toml_str).expect_err("must not parse");
        assert!(err.to_string().contains("model rig sidecar"));
    }

    /// The guard is scoped: a mesh without a ladder is untouched, and `lod`
    /// elsewhere in the document is not this field.
    #[test]
    fn a_mesh_without_a_ladder_still_parses() {
        let config = EntityConfig::from_toml(
            "[mesh]\nmodel = \"assets/models/rock.glb\"\nshape = \"sphere\"\ncolour = [0.5, 0.5, 0.5]\n",
        )
        .expect("a plain [mesh] must still parse");
        let mesh = config.mesh.expect("mesh section present");
        assert_eq!(mesh.model.as_deref(), Some("assets/models/rock.glb"));
    }

    // ── NPC red-alert provisioning (issue #749) ─────────────────────────────────

    fn red_alert_systems(config: &EntityConfig) -> Vec<&crate::ship::config::SystemInstanceConfig> {
        config
            .ship_config
            .as_ref()
            .map(|sc| {
                sc.systems
                    .iter()
                    .filter(|s| s.kind == crate::system_registry::RED_ALERT_KIND)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// A minimal `[behaviour]` hull that authors no `red_alert` system, so the
    /// #749 provision is exercised on a fixture rather than on a shipped hull.
    ///
    /// Every shipped hull authors its own `red_alert` since #871 gave the NPC
    /// hulls a Captain seat to own it, so the provision has no shipped hull left
    /// to fire on — but it is still live code for any hull authored without one,
    /// which is what these fixtures keep covered.
    const BARE_BEHAVIOUR_HULL: &str = r#"
tags = ["ship"]

[behaviour]

[[behaviour.doctrine]]
id = "destroy-hostiles"
text = "Destroy hostiles"
directive_kind = "Destroy"
base_priority = 35.0

[[system]]
id = "helm-thrust"
kind = "helm_thrust"
ai_only = true
"#;

    #[test]
    fn behaviour_npc_without_red_alert_gets_ai_only_ownerless_provision() {
        // A hull that authors [behaviour] but no red_alert system. Spawn
        // provisioning must add exactly one AI-only, ownerless red_alert
        // capability so the AI captain can raise it.
        let config = EntityConfig::from_toml_in_mode(
            BARE_BEHAVIOUR_HULL,
            crate::entities::ai_declaration_manifest::AiDeclarationMode::Lenient,
        )
        .expect("fixture must parse");
        let reds = red_alert_systems(&config);
        assert_eq!(
            reds.len(),
            1,
            "behaviour NPC must be provisioned exactly one red_alert system"
        );
        let sys = reds[0];
        assert_eq!(sys.id.0, crate::system_registry::RED_ALERT_SYSTEM_ID);
        assert!(sys.ai_only, "provisioned red_alert must be ai_only");
        assert!(
            sys.station.is_none(),
            "provisioned red_alert must be ownerless"
        );
    }

    #[test]
    fn behaviour_npc_red_alert_provision_is_not_hull_specific() {
        // A second, differently-shaped behaviour hull — shield arcs and a weapon
        // rather than a bare helm axis — to confirm the provisioning keys off
        // `[behaviour]` alone.
        let toml_str = r#"
tags = ["ship"]

[[shield_arc]]
id = "all"
label = "All"
center_deg = 0
width_deg = 360
max_hp = 15

[weapons_console]

[[weapons_console.phaser_banks]]
id = "fore"
facing_deg = 0.0
fire_arc_deg = 180.0
auto_arc_deg = 180.0

[behaviour]

[[behaviour.doctrine]]
id = "destroy-hostiles"
text = "Destroy hostiles"
directive_kind = "Destroy"
base_priority = 35.0

[[system]]
id = "phaser-fore"
kind = "phaser_bank"
ai_only = true
"#;
        let config = EntityConfig::from_toml_in_mode(
            toml_str,
            crate::entities::ai_declaration_manifest::AiDeclarationMode::Lenient,
        )
        .expect("fixture must parse");
        let reds = red_alert_systems(&config);
        assert_eq!(reds.len(), 1, "behaviour NPC provisioned one red_alert");
        assert!(reds[0].ai_only && reds[0].station.is_none());
    }

    #[test]
    fn harrow_destroyer_authors_its_own_captain_owned_red_alert() {
        // Since #871 this NPC hull carries a Captain seat and authors its own
        // red_alert on it. Provisioning must be idempotent — no second system —
        // and the authored ownership must survive. The control source is
        // unchanged from the provisioned era: an unmanned Captain seat boots on
        // `Backfill`, which automates every system it owns, so
        // `operate_captain_ai` still raises Red Alert.
        // (#892) Re-pointed off the retired `pirate_raider.toml`.
        let toml_str = &resolved_text("ship_harrow_destroyer");
        let config = EntityConfig::from_toml(toml_str).expect("the Harrow Destroyer must parse");
        let reds = red_alert_systems(&config);
        assert_eq!(
            reds.len(),
            1,
            "authored red_alert must not be double-provisioned"
        );
        assert_eq!(
            reds[0].station,
            Some(crate::messages::StationId("captain".into())),
            "the Captain seat owns Red Alert"
        );
        assert!(
            !reds[0].ai_only,
            "a station-owned system must not rely on `ai_only`"
        );
    }

    #[test]
    fn explicit_red_alert_is_left_untouched() {
        // The Alliance Destroyer authors an explicit red_alert system owned by
        // the captain station. Provisioning must be idempotent: no second
        // system, and the authored ownership survives (AC4).
        // Through the resolver, not `include_str!`: this hull is COMPOSED since
        // #875, so its baked bytes are no longer the document that spawns. The
        // assertion below is unchanged — provisioning idempotence is the claim,
        // and it is now made against the real resolved hull.
        let config = shipped_hull("alliance_destroyer");
        let reds = red_alert_systems(&config);
        assert_eq!(
            reds.len(),
            1,
            "authored red_alert must not be double-provisioned"
        );
        assert_eq!(
            reds[0].station,
            Some(crate::messages::StationId("captain".into())),
            "authored captain ownership must survive provisioning"
        );
        assert!(
            !reds[0].ai_only,
            "authored player red_alert must remain non-ai_only"
        );
    }

    #[test]
    fn non_behaviour_entity_gets_no_red_alert_provision() {
        // An asteroid has no [behaviour] block → no ship capabilities at all.
        let toml_str = r#"
tags = ["asteroid"]
"#;
        let config = EntityConfig::from_toml(toml_str).expect("asteroid must parse");
        assert!(
            config.ship_config.is_none(),
            "non-behaviour entity must not synthesise a ship_config"
        );
        assert!(
            red_alert_systems(&config).is_empty(),
            "non-behaviour entity must get no red_alert system"
        );
    }
}

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
