pub use phoenix_sim_session::lobby::stations_config::{get_station, StationAssignments};

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
            ratings: sc.ratings.iter().map(|r| r.name.clone()).collect(),
        })
        .collect();
    let consoles = config
        .consoles
        .iter()
        .map(|console| phoenix_model::wire::ConsoleDef {
            description: console.description.clone(),
            short_code: console.short_code.clone(),
            manual_overview: console.manual_overview.clone(),
            tutorials: console.tutorials.clone(),
            command_target: console.command_target.clone(),
            ratings: console.ratings.clone(),
            id: console.id.clone(),
            name: console.name.clone(),
            home_station: console.home_station.clone(),
            screen: console.screen.clone(),
            human_seeking: console.human_seeking,
            host_order: console.host_order.clone(),
            visiting_rating: console.visiting_rating.clone(),
            feeds: console.feeds.clone(),
            systems: config
                .systems
                .iter()
                .filter(|system| system.console.as_ref() == Some(&console.id))
                .map(|system| system.id.clone())
                .collect(),
        })
        .collect();
    ShipStations { stations, consoles }
}
