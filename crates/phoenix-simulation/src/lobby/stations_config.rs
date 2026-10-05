use std::collections::HashMap;

/// A single station in the fixed roster.
pub use phoenix_model::wire::StationDef;

/// Fixed-roster station configuration. Populated from `ShipConfigResource`
/// at startup; per-player-count cascade machinery removed in B3 (issue #533).
pub use phoenix_model::wire::ShipStations;

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
            console: sc.console.clone(),
            ratings: sc.ratings.iter().map(|r| r.name.clone()).collect(),
            human_seeking: sc.human_seeking,
            host_order: sc.host_order.clone(),
            visiting_rating: sc.visiting_rating.clone(),
            auxiliary: sc.auxiliary,
            command_target: sc.command_target.clone(),
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
