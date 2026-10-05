//! Entity schema: comms. Public paths remain in the parent module.
use super::*;

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
