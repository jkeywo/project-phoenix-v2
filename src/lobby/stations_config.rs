use crate::messages::Console;
use bevy::prelude::Resource;
use std::collections::HashMap;

/// A single station in the fixed roster.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StationDef {
    pub name: String,
    pub description: String,
    pub consoles: Vec<Console>,
    pub rank: String,
    #[serde(default)]
    pub short_code: String,
}

use serde::{Deserialize, Serialize};

/// Return the default available complexity presets for every console.
/// Kept here until B4 removes SetComplexity entirely.
pub fn default_complexity_presets() -> HashMap<Console, Vec<String>> {
    let mut m = HashMap::new();
    for c in &[
        Console::CaptainChair,
        Console::Helm,
        Console::Tactical,
        Console::Repair,
        Console::Power,
        Console::Comms,
    ] {
        m.insert(c.clone(), vec!["Low".into(), "Std".into()]);
    }
    for c in &[Console::Sensors, Console::Shields, Console::Navigation] {
        m.insert(c.clone(), vec!["Std".into()]);
    }
    m
}

/// Fixed-roster station configuration. Populated from `ShipConfigResource`
/// at startup; per-player-count cascade machinery removed in B3 (issue #533).
#[derive(Resource, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ShipStations {
    pub stations: Vec<StationDef>,
    /// Per-console available complexity preset names (removed in B4 with SetComplexity).
    pub complexity_presets: HashMap<Console, Vec<String>>,
}

impl Default for ShipStations {
    fn default() -> Self {
        Self {
            stations: Vec::new(),
            complexity_presets: default_complexity_presets(),
        }
    }
}

/// Build a `ShipStations` from the new `ShipConfig` station list.
/// Each `StationConfig.console` string is resolved to a `Console` variant;
/// stations whose console string is unrecognised are silently skipped.
pub fn stations_from_ship_config(config: &crate::ship::config::ShipConfig) -> ShipStations {
    let stations = config
        .stations
        .iter()
        .filter_map(|sc| {
            let console = Console::from_console_id(&sc.console)?;
            Some(StationDef {
                name: sc.name.clone(),
                description: sc.description.clone(),
                consoles: vec![console],
                rank: sc.rank.clone(),
                short_code: sc.short_code.clone(),
            })
        })
        .collect();
    ShipStations {
        stations,
        complexity_presets: default_complexity_presets(),
    }
}

/// Look up a station by name. Returns `None` if not found.
pub fn get_station<'a>(stations: &'a ShipStations, name: &str) -> Option<&'a StationDef> {
    stations.stations.iter().find(|d| d.name == name)
}

/// Maps session token → station name.  A token absent from this map is a spectator.
pub type StationAssignments = HashMap<String, String>;
