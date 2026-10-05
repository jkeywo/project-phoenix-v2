//! Shared Phoenix wire value records. No simulation or host I/O.
use crate::identity::{StationId, SystemId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GmOperator {
    pub id: String,
    pub name: String,
    pub connected: bool,
    pub ready: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StationDef {
    /// Stable designer-authored id (mirrors `StationConfig.id`).
    #[serde(default)]
    pub id: StationId,
    pub name: String,
    pub description: String,
    pub rank: String,
    #[serde(default)]
    pub short_code: String,
    /// Console root URL (e.g. "gui/captain-console.html"). Mirrors
    /// `StationConfig.console`. When absent, the client launches a
    /// generic fallback panel.
    #[serde(default)]
    pub console: Option<String>,
    /// Selectable rating names for this station, excluding the implicit
    /// `Backfill` rating (a runtime-only disconnect/unmanned state, never a
    /// lobby-selectable choice). Mirrors `StationConfig.ratings`' names, in
    /// TOML declaration order. Every station has at least `"Std"`; a length
    /// greater than 1 is the client's sole signal to render the lobby
    /// complexity toggle — never hardcode station ids for this.
    #[serde(default)]
    pub ratings: Vec<String>,
    #[serde(default)]
    pub human_seeking: bool,
    #[serde(default)]
    pub host_order: Vec<StationId>,
    #[serde(default)]
    pub visiting_rating: Option<String>,
    #[serde(default)]
    pub auxiliary: bool,
    /// The Station this one directs when it is AI-controlled (mirrors
    /// `StationConfig.command_target`). Only the auxiliary Command station
    /// authors this today; absent for every other Station. The client uses it
    /// to hide the directing Station's hero-bar tab when its target is
    /// human-held (issue tracked alongside #1107/#1387's Command work).
    #[serde(default)]
    pub command_target: Option<StationId>,
}

#[cfg_attr(feature = "ecs", derive(bevy_ecs::prelude::Resource))]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct ShipStations {
    pub stations: Vec<StationDef>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum DamageTier {
    Operational,
    Damaged,
    Disabled,
    /// HP reached exactly 0. Unrepairable until `restore()` is called.
    Destroyed,
}

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub enum SystemDepth {
    Ai,
    Simplified,
    Detailed,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SystemManualSection {
    /// Stable section-kind code — the system kind (e.g. `"shields"`). NOT a
    /// strings id: the client maps it to a heading id (`manual.section.<kind>`).
    pub kind: String,
    /// Numeric metrics, each a machine `code` + value. The client maps
    /// `(kind, code)` to a label id (`manual.<kind>.<code>`) and interpolates
    /// the value via `t()`.
    #[serde(default)]
    pub metrics: Vec<SystemManualMetric>,
    /// Non-numeric capabilities that don't fit an `f64` metric (issue #773):
    /// each carries a machine `code` plus a machine `value_code`. The client
    /// maps the label id (`manual.<kind>.<code>`) and the value id
    /// (`manual.<kind>.<code>.<value_code>`) through `t()` — e.g. the Helm
    /// movement mode renders as a readable capability rather than a bare
    /// number. Empty for every provider that needs only numeric metrics.
    /// `#[serde(default)]` keeps pre-#773 manual round-trips (which never
    /// carried this field) intact.
    #[serde(default)]
    pub capabilities: Vec<SystemManualCapability>,
    /// Rating→AI automation for the owning station: for each authored rating
    /// (plus the implicit `Backfill`), which owned systems become AI-operated.
    /// Derived from [`resolve_automated_systems`].
    #[serde(default)]
    pub automation: Vec<StationRatingAutomation>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SystemManualMetric {
    /// Machine code (e.g. `"max_hp"`); the client maps it to a strings label id.
    pub code: String,
    /// The configured value, interpolated into the label by the client.
    pub value: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SystemManualCapability {
    /// Machine code for the capability (e.g. `"movement_mode"`).
    pub code: String,
    /// Machine code for the current value (e.g. `"planar"`).
    pub value_code: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StationRatingAutomation {
    /// Authored rating name (TOML data — an identifier the client maps to a
    /// caption, like the settings-panel rating toggle already does).
    pub rating: String,
    /// System ids automated under this rating (machine ids, not English).
    #[serde(default)]
    pub automated_systems: Vec<SystemId>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StationManualWire {
    pub station_id: StationId,
    /// Authored String Id or legacy prose from `[[station]] manual_overview`.
    /// Resolved by the client; `None` when the station authored no overview.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overview: Option<String>,
    /// Generated sections, one per owned system that has a registered provider.
    #[serde(default)]
    pub sections: Vec<SystemManualSection>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ShipManualWire {
    #[serde(default)]
    pub stations: Vec<StationManualWire>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ShipKey(pub String);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "verb", rename_all = "snake_case")]
pub enum CivilianOrder {
    /// Stop where you are and hold station.
    Hold,
    /// Take this alternate route, or make for this single anchor.
    Divert {
        /// A `[[route]]` id.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        route: Option<String>,
        /// An `[anchors]` name.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        anchor: Option<String>,
    },
    /// Proceed to and dock at the named structure.
    Dock {
        /// The structure's authored world entity name.
        structure: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderKind {
    /// `hold`.
    Hold,
    /// `divert`, either flavour.
    Divert,
    /// `dock`.
    Dock,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnclaimedSlotPolicy {
    #[default]
    Backfill,
    Absent,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresentationCardWire {
    pub kind: String,
    pub title: String,
    pub body: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub body_params: BTreeMap<String, String>,
    #[serde(default)]
    pub literal_body: bool,
    #[serde(default)]
    pub literal_title: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SensorReport {
    pub observed_tick: u64,
    pub age_ticks: u64,
    pub source: String,
    pub name: String,
    pub position_mm: [i64; 3],
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContactMode {
    Reveal,
    Conceal,
    #[default]
    Normal,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DebrisAssessment {
    /// `strings.csv` id for the protected asset's crew-facing name — WHAT IS
    /// UNDER THIS ROCK, which is the one thing a threat readout has to say.
    #[serde(default)]
    pub protected_name: String,
    /// The debris's course relative to the protected asset, planar `[x, z]` in
    /// world units per second — the same shape and units the sensors radar's
    /// `selected_target_relative_velocity` (issue #1339) already publishes, so
    /// a console draws this projection with the code it already has.
    pub course: [f32; 2],
    /// How close the debris gets to the protected asset on this course, world
    /// units. Equal to the present separation when the contact is opening.
    pub closest_approach: f32,
    /// Seconds until that closest approach. `0.0` when it is already past —
    /// the rock's nearest moment was behind it and it is drawing away.
    pub seconds_to_closest_approach: f32,
    /// Whether [`closest_approach`](Self::closest_approach) falls inside the
    /// authored impact radius. **The threat verdict**, and a comparison of two
    /// numbers above rather than a judgement of its own.
    pub on_collision_course: bool,
    /// Seconds until the debris crosses into the impact radius. `None` when it
    /// is not on a collision course at all — never `0.0` standing in for "no
    /// answer", because a crew reading zero seconds must be reading an impact
    /// that is happening, not one that will never happen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seconds_to_impact: Option<f32>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReadinessTally {
    pub connected: u32,
    pub ready: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadinessTallyError {
    ReadyExceedsConnected,
    TotalOverflow,
}

impl<'de> Deserialize<'de> for ReadinessTally {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Counts {
            connected: u32,
            ready: u32,
        }
        let counts = Counts::deserialize(deserializer)?;
        Self::try_new(counts.connected, counts.ready)
            .map_err(|_| serde::de::Error::custom("ready exceeds connected"))
    }
}

impl CivilianOrder {
    /// Divert onto a named route.
    pub fn divert_to_route(route: impl Into<String>) -> Self {
        Self::Divert {
            route: Some(route.into()),
            anchor: None,
        }
    }

    /// Divert to a single named anchor.
    pub fn divert_to_anchor(anchor: impl Into<String>) -> Self {
        Self::Divert {
            route: None,
            anchor: Some(anchor.into()),
        }
    }

    /// Dock at a named structure.
    pub fn dock_at(structure: impl Into<String>) -> Self {
        Self::Dock {
            structure: structure.into(),
        }
    }

    /// Which verb this is.
    pub fn kind(&self) -> OrderKind {
        match self {
            Self::Hold => OrderKind::Hold,
            Self::Divert { .. } => OrderKind::Divert,
            Self::Dock { .. } => OrderKind::Dock,
        }
    }

    /// Authored route destination, when this is a route divert. World
    /// validation uses this to reject a button that can only submit a dangling
    /// lane reference before the scenario activates.
    pub fn route_destination(&self) -> Option<&str> {
        match self {
            Self::Divert {
                route: Some(route), ..
            } => Some(route),
            _ => None,
        }
    }

    /// Reject an order that cannot be carried out by construction.
    ///
    /// Checked at the admission and script boundaries both, so a malformed
    /// order is refused *at the console* with a reason rather than becoming a
    /// civilian stuck in `non_compliant` for a reason nobody can act on.
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::Hold => Ok(()),
            Self::Divert { route, anchor } => match (route, anchor) {
                (Some(r), None) if !r.trim().is_empty() => Ok(()),
                (None, Some(a)) if !a.trim().is_empty() => Ok(()),
                (Some(_), Some(_)) => Err("a divert order names both a route and an \
                                           anchor; it takes exactly one"
                    .to_string()),
                _ => Err("a divert order names neither a route nor an anchor; it \
                          takes exactly one"
                    .to_string()),
            },
            Self::Dock { structure } if structure.trim().is_empty() => {
                Err("a dock order names no structure".to_string())
            }
            Self::Dock { .. } => Ok(()),
        }
    }
}

impl ReadinessTally {
    pub fn try_new(connected: u32, ready: u32) -> Result<Self, ReadinessTallyError> {
        if ready > connected {
            return Err(ReadinessTallyError::ReadyExceedsConnected);
        }
        Ok(Self { connected, ready })
    }

    pub fn all_ready(self) -> bool {
        self.connected > 0 && self.ready == self.connected
    }

    pub fn checked_add(self, other: Self) -> Result<Self, ReadinessTallyError> {
        Self::try_new(self.connected, self.ready)?;
        Self::try_new(other.connected, other.ready)?;
        let connected = self
            .connected
            .checked_add(other.connected)
            .ok_or(ReadinessTallyError::TotalOverflow)?;
        let ready = self
            .ready
            .checked_add(other.ready)
            .ok_or(ReadinessTallyError::TotalOverflow)?;
        Self::try_new(connected, ready)
    }
}

impl ShipKey {
    /// Whether this key names a ship a replay could resolve.
    pub fn is_named(&self) -> bool {
        !self.0.is_empty()
    }
}
pub fn default_sensors_projection_horizon_secs() -> f32 {
    60.0
}
pub fn default_sensors_projection_marker_interval_secs() -> f32 {
    10.0
}
pub const fn default_min_power_level() -> u8 {
    1
}

pub const RED_ALERT_SYSTEM_ID: &str = "red-alert";
pub const VIEWSCREEN_SYSTEM_ID: &str = "viewscreen";
pub const COMMAND_SYSTEM_ID: &str = "command";

impl GmOperator {
    /// Create a newly admitted operator. Readiness is deliberately never
    /// inherited from admission or reconnect state.
    pub fn new(id: String, name: String, connected: bool) -> Self {
        Self {
            id,
            name,
            connected,
            ready: false,
        }
    }
}

impl OrderKind {
    /// The wire/script label.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Hold => "hold",
            Self::Divert => "divert",
            Self::Dock => "dock",
        }
    }
}

#[cfg(test)]
#[path = "wire_tests.rs"]
mod tests;
