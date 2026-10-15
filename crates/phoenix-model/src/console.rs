//! Console definitions and read-only bindings. No simulation or transport ownership.
use crate::identity::{ConsoleId, StationId, SystemId};
use crate::messages::SystemBlackboard;
use serde::{Deserialize, Serialize};

/// Derived from direct crew occupancy and authored Console hosting each tick.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsoleHostSnapshot {
    pub console: ConsoleId,
    pub host: Option<StationId>,
    pub rating: String,
}

/// Shared work context, separate from machinery intent and device preferences.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConsoleWorkingView {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interior_deck: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub energy_depth: Option<crate::energy::EnergyControlDepth>,
    pub inspected_record: Option<uuid::Uuid>,
    pub comms_thread: Option<String>,
    pub overlay: Option<String>,
    pub navigation: Option<NavigationWorkingView>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NavigationWorkingView {
    pub zoom: f32,
    pub center_x: f32,
    pub center_y: f32,
    pub selected_uuid: Option<uuid::Uuid>,
}

impl ConsoleWorkingView {
    pub fn is_valid(&self) -> bool {
        [&self.comms_thread, &self.overlay, &self.interior_deck]
            .into_iter()
            .all(|value| {
                value.as_ref().is_none_or(|s| {
                    !s.is_empty() && s.len() <= 256 && !s.chars().any(char::is_control)
                })
            })
            && self.navigation.as_ref().is_none_or(|view| {
                (0.25..=8.0).contains(&view.zoom)
                    && view.center_x.is_finite()
                    && view.center_y.is_finite()
            })
    }
}

/// Supported observation shapes. A binding names both a publication and its type;
/// mismatched publications are unavailable rather than reinterpreted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ConsoleObservationKind {
    Passive,
    Recovery,
    Production,
    Stock,
    Logistics,
    Helm,
    Weapons,
    Power,
    Shields,
    Captain,
    Command,
    Repair,
    Comms,
    Sensors,
    Navigation,
    Viewscreen,
    HelmEngine,
    PhaserBank,
    TorpedoTube,
    TorpedoMagazine,
    PowerReactor,
    PowerBattery,
    ShieldArc,
    HelmLateralThrust,
    TacticalRadar,
    SensorRadar,
    Dossiers,
    Scan,
    Tractor,
    Dock,
    Umbilical,
    Security,
    Transporter,
    Demolition,
    Transit,
    Bay,
}

impl ConsoleObservationKind {
    pub const NAMES: &[&str] = &[
        "Passive",
        "Recovery",
        "Production",
        "Stock",
        "Logistics",
        "Helm",
        "Weapons",
        "Power",
        "Shields",
        "Captain",
        "Command",
        "Repair",
        "Comms",
        "Sensors",
        "Navigation",
        "Viewscreen",
        "HelmEngine",
        "PhaserBank",
        "TorpedoTube",
        "TorpedoMagazine",
        "PowerReactor",
        "PowerBattery",
        "ShieldArc",
        "HelmLateralThrust",
        "TacticalRadar",
        "SensorRadar",
        "Dossiers",
        "Scan",
        "Tractor",
        "Dock",
        "Umbilical",
        "Security",
        "Transporter",
        "Demolition",
        "Transit",
        "Bay",
    ];

    pub fn of(value: &SystemBlackboard) -> Self {
        match value {
            SystemBlackboard::Passive(..) => Self::Passive,
            SystemBlackboard::Recovery(..) => Self::Recovery,
            SystemBlackboard::Production(..) => Self::Production,
            SystemBlackboard::Stock(..) => Self::Stock,
            SystemBlackboard::Logistics(..) => Self::Logistics,
            SystemBlackboard::Helm(..) => Self::Helm,
            SystemBlackboard::Weapons(..) => Self::Weapons,
            SystemBlackboard::Power(..) => Self::Power,
            SystemBlackboard::Shields(..) => Self::Shields,
            SystemBlackboard::Captain(..) => Self::Captain,
            SystemBlackboard::Command(..) => Self::Command,
            SystemBlackboard::Repair(..) => Self::Repair,
            SystemBlackboard::Comms(..) => Self::Comms,
            SystemBlackboard::Sensors(..) => Self::Sensors,
            SystemBlackboard::Navigation(..) => Self::Navigation,
            SystemBlackboard::Viewscreen(..) => Self::Viewscreen,
            SystemBlackboard::HelmEngine(..) => Self::HelmEngine,
            SystemBlackboard::PhaserBank(..) => Self::PhaserBank,
            SystemBlackboard::TorpedoTube(..) => Self::TorpedoTube,
            SystemBlackboard::TorpedoMagazine(..) => Self::TorpedoMagazine,
            SystemBlackboard::PowerReactor(..) => Self::PowerReactor,
            SystemBlackboard::PowerBattery(..) => Self::PowerBattery,
            SystemBlackboard::ShieldArc(..) => Self::ShieldArc,
            SystemBlackboard::HelmLateralThrust(..) => Self::HelmLateralThrust,
            SystemBlackboard::TacticalRadar(..) => Self::TacticalRadar,
            SystemBlackboard::SensorRadar(..) => Self::SensorRadar,
            SystemBlackboard::Dossiers(..) => Self::Dossiers,
            SystemBlackboard::Scan(..) => Self::Scan,
            SystemBlackboard::Tractor(..) => Self::Tractor,
            SystemBlackboard::Dock(..) => Self::Dock,
            SystemBlackboard::Umbilical(..) => Self::Umbilical,
            SystemBlackboard::Security(..) => Self::Security,
            SystemBlackboard::Transporter(..) => Self::Transporter,
            SystemBlackboard::Demolition(..) => Self::Demolition,
            SystemBlackboard::Transit(..) => Self::Transit,
            SystemBlackboard::Bay(..) => Self::Bay,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConsoleFeed {
    /// Stable publication key; some existing observations are channels rather
    /// than controllable equipment. This does not grant command authority.
    pub source: SystemId,
    pub kind: ConsoleObservationKind,
}

impl ConsoleFeed {
    pub fn accepts(&self, source: &SystemId, observation: &SystemBlackboard) -> bool {
        &self.source == source
            && self.kind == ConsoleObservationKind::of(observation)
            && match observation {
                SystemBlackboard::Scan(scan) => {
                    crate::sensing::scan_owner_from_blackboard_key(source).as_ref()
                        == scan.owner.as_ref()
                        && scan.owner.is_some()
                }
                _ => true,
            }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConsoleRatingConfig {
    pub name: String,
    pub automated_systems: Vec<SystemId>,
    /// Explicit direct-control systems in this authored rung. Remaining owned
    /// systems are simplified. Absent on legacy hulls: their non-automated
    /// systems retain direct control, including their existing default rating.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detailed_systems: Option<Vec<SystemId>>,
    /// Per-rating AI tuning parameters (replaces assets/complexity/*.toml).
    /// Keys are AI rule names (e.g. "torpedo_auto_fire", "frequency_match");
    /// values are TOML tables with rule-specific parameters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ai_tuning: Option<toml::Value>,
}

/// Authored Console identity is independent of its screen implementation and
/// hosting Station. Multiple Consoles may use one screen and one Station.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConsoleConfig {
    pub id: ConsoleId,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub short_code: String,
    #[serde(default)]
    pub manual_overview: Option<String>,
    #[serde(default, rename = "tutorial")]
    pub tutorials: Vec<crate::messages::TutorialOverlayWire>,
    #[serde(default)]
    pub command_target: Option<StationId>,
    /// Detached auxiliary screens retain their own complexity choices.
    #[serde(default, rename = "rating")]
    pub ratings: Vec<ConsoleRatingConfig>,
    #[serde(default)]
    pub home_station: Option<StationId>,
    /// None is an AI-only knowledge scope with no mounted human screen.
    #[serde(default)]
    pub screen: Option<String>,
    #[serde(default)]
    pub human_seeking: bool,
    #[serde(default)]
    pub host_order: Vec<StationId>,
    #[serde(default)]
    pub visiting_rating: Option<String>,
    #[serde(default, rename = "feed")]
    pub feeds: Vec<ConsoleFeed>,
}

impl ConsoleConfig {
    /// Deterministic placement from direct, eligible human occupancy only.
    /// Callers must not pass hosted Console presence as Station occupancy.
    pub fn resolve_host(&self, available: impl Fn(&StationId) -> bool) -> Option<StationId> {
        self.screen.as_ref()?;
        if let Some(home) = self.home_station.as_ref().filter(|home| available(home)) {
            return Some(home.clone());
        }
        if self.human_seeking {
            self.host_order.iter().find(|host| available(host)).cloned()
        } else {
            None
        }
    }

    /// View over already filtered publications, not another mutable fact store.
    /// Ordering follows authored bindings; an absent or mistyped feed is absent.
    pub fn observations<'a>(
        &'a self,
        publications: &'a std::collections::HashMap<SystemId, SystemBlackboard>,
    ) -> impl Iterator<Item = (&'a SystemId, &'a SystemBlackboard)> + 'a {
        self.feeds.iter().filter_map(|feed| {
            let value = publications.get(&feed.source)?;
            feed.accepts(&feed.source, value)
                .then_some((&feed.source, value))
        })
    }
}

#[cfg(test)]
#[path = "console_tests.rs"]
mod tests;

/// Console attention projected from the existing importance owner.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ConsoleImportanceSnapshot {
    pub console: crate::identity::ConsoleId,
    pub unread: bool,
    pub critical: bool,
}
