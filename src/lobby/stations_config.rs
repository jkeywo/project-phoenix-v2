use crate::messages::StationId;
use bevy::prelude::Resource;
use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// A single station in the fixed roster.
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
}

/// Fixed-roster station configuration. Populated from `ShipConfigResource`
/// at startup; per-player-count cascade machinery removed in B3 (issue #533).
#[derive(Resource, Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct ShipStations {
    pub stations: Vec<StationDef>,
}

/// Build a `ShipStations` from the new `ShipConfig` station list.
///
/// Core stations (id == "core") are forbidden by `ShipConfig::validate` via
/// `ReservedCoreStationId`, so we never see them here.
pub fn stations_from_ship_config(config: &crate::ship::config::ShipConfig) -> ShipStations {
    let stations = config
        .stations
        .iter()
        .map(|sc| StationDef {
            id: sc.id.clone(),
            name: sc.name.clone(),
            description: sc.description.clone(),
            rank: sc.rank.clone(),
            short_code: sc.short_code.clone(),
        })
        .collect();
    ShipStations { stations }
}

/// Look up a station by name. Returns `None` if not found.
pub fn get_station<'a>(stations: &'a ShipStations, name: &str) -> Option<&'a StationDef> {
    stations
        .stations
        .iter()
        .find(|d| d.name == name || d.id.0 == name)
}

/// Maps session token → station name.  A token absent from this map is a spectator.
pub type StationAssignments = HashMap<String, String>;
