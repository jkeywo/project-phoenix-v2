//! Entity schema: hull. Public paths remain in the parent module.
use super::*;

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
