pub use phoenix_model::wire::{ShipStations, StationDef};
use std::collections::HashMap;

/// Look up a station by name. Returns `None` if not found.
pub fn get_station<'a>(stations: &'a ShipStations, name: &str) -> Option<&'a StationDef> {
    stations
        .stations
        .iter()
        .find(|d| d.name == name || d.id.0 == name)
}

/// Maps session token → station name.  A token absent from this map is a spectator.
pub type StationAssignments = HashMap<String, String>;
