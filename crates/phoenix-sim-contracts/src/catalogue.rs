//! Pre-load scenario and hull selection records shared by World and Session.
use phoenix_model::wire::UnclaimedSlotPolicy;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AvailableShipEntry {
    pub template_path: String,
    #[serde(default)]
    pub label: Option<String>,
}

/// One mission-authored player-ship position (issue #1518).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShipSlotConfig {
    pub id: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub ships: Vec<AvailableShipEntry>,
    pub default_ship: String,
    /// What launch does when no host claimed the slot.
    #[serde(default)]
    pub unclaimed: UnclaimedSlotPolicy,
}

/// One player-ship option offered by a scenario (reuses the world's
/// [`AvailableShipEntry`]).
pub type CatalogShip = AvailableShipEntry;

/// One selectable scenario in the pre-load catalog, with its display metadata
/// and the ships it offers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScenarioCatalogEntry {
    /// Stable scenario id from the manifest.
    pub id: String,
    /// World TOML path.
    pub world: String,
    /// Display label: the manifest entry's `label`, else the world's
    /// `[global] title`, else `None`.
    pub label: Option<String>,
    /// The world's `[global] description`, when present.
    pub description: Option<String>,
    /// The ships this scenario offers — `[[available_ships]]` for a legacy
    /// world, or the deduplicated hull options of its explicit ship slots.
    pub ships: Vec<CatalogShip>,
    /// Authored mission slots after legacy one-slot compatibility synthesis.
    pub slots: Vec<ShipSlotConfig>,
    /// Provenance: the pack id this scenario came from, or `None` for a
    /// base-manifest scenario (issue #987). `build_catalog` always sets `None`;
    /// [`build_merged_catalog`] stamps each mod scenario with its pack id.
    pub origin: Option<String>,
}

/// The authoritative pre-load catalog: the selectable scenarios and their
/// per-scenario ship lists, built from the manifest before any world is active.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScenarioCatalog {
    pub scenarios: Vec<ScenarioCatalogEntry>,
}
