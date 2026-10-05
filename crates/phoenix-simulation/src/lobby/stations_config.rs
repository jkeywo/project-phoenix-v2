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
