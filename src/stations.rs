// Re-export shim — will be removed when stations modules migrate to src/lobby/.
// See split: stations_config.rs (parse + lookup), stations_policy.rs (assignment policy).
pub use crate::stations_config::{
    StationDef, ShipStations, StationConfigError, StationAssignments,
    parse_and_validate, get_station, all_stations_filled,
};
pub use crate::stations_policy::{
    reassign_on_join, advance_on_join, reassign_on_leave,
};
