//! Frozen, identity-free Station ratings shared by every fleet peer.

use crate::core::messages::StationId;
use crate::ship::config::ShipConfig;

// Kept in step with gui/fleet-crew.js at the two typed ingress boundaries.

pub fn crew_matches_hull(crew: &[(StationId, String)], hull: &ShipConfig) -> bool {
    crew.iter().all(|(id, rating)| {
        hull.stations.iter().any(|station| {
            station.id == *id
                && !station.auxiliary
                && (station.ratings.iter().any(|choice| choice.name == *rating)
                    || (station.ratings.is_empty() && rating == "Std"))
        })
    })
}

/// Validate before any clock or roster state changes. Each peer checks the
/// frozen choices against its own already-preloaded copy of the selected hull.
pub fn roster_crew_matches_hulls(
    world: &bevy::prelude::World,
    roster: &super::FleetRoster,
) -> bool {
    roster.ships().iter().all(|ship| {
        if canonical_station_ratings(ship.crew.clone()).is_none() {
            return false;
        }
        if ship.crew.is_empty() {
            return true;
        }
        match ship.ship_path.as_deref() {
            Some(path) => crate::entities::config_cache::get_cached_entity_config(path)
                .and_then(|config| config.ship_config)
                .is_some_and(|hull| crew_matches_hull(&ship.crew, &hull)),
            None => world
                .get_resource::<crate::ship_plugin::PendingShipConfig>()
                .is_some_and(|hull| crew_matches_hull(&ship.crew, &hull.0)),
        }
    })
}

#[cfg(test)]
#[path = "crew_tests.rs"]
mod tests;

pub use phoenix_sim_session::lockstep::crew::*;
