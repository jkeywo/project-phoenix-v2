//! Frozen, identity-free Station ratings shared by every fleet peer.

use crate::core::messages::StationId;
use crate::ship::config::ShipConfig;

// Kept in step with gui/fleet-crew.js at the two typed ingress boundaries.
pub const MAX_FLEET_CREW_SEATS: usize = 128;
pub const MAX_FLEET_CREW_FIELD_BYTES: usize = 128;

pub fn canonical_station_ratings(
    mut crew: Vec<(StationId, String)>,
) -> Option<Vec<(StationId, String)>> {
    let valid = |text: &str| {
        !text.is_empty()
            && text.len() <= MAX_FLEET_CREW_FIELD_BYTES
            && !text.chars().any(char::is_control)
    };
    if crew.len() > MAX_FLEET_CREW_SEATS
        || crew
            .iter()
            .any(|(station, rating)| !valid(&station.0) || !valid(rating))
    {
        return None;
    }
    crew.sort_by(|a, b| a.0 .0.cmp(&b.0 .0));
    if crew.windows(2).any(|pair| pair[0].0 == pair[1].0) {
        return None;
    }
    Some(crew)
}

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
mod tests {
    use super::*;

    fn decode(crew: serde_json::Value) -> Option<super::super::FleetRoster> {
        let raw = serde_json::json!({
            "local": 1, "owner": 1, "participants": [1],
            "ships": [{"host": 1, "crew": crew}]
        })
        .to_string();
        crate::core::codec::decode_fleet_roster(&raw).map(|(roster, _)| roster)
    }

    #[test]
    fn fleet_crew_ingress_refuses_malformed_or_duplicate_seats_and_orders_exact_pairs() {
        use serde_json::json;
        for invalid in [
            json!(null),
            json!({}),
            json!([["helm"]]),
            json!([["helm", "Std", "private-token"]]),
            json!([["", "Std"]]),
            json!([["helm", ""]]),
            json!([["helm", "Std"], ["helm", "Manual"]]),
            json!([["helm", "Bad\nRating"]]),
            json!([["helm", "é".repeat(MAX_FLEET_CREW_FIELD_BYTES)]]),
            json!((0..=MAX_FLEET_CREW_SEATS)
                .map(|i| (format!("s{i}"), "Std"))
                .collect::<Vec<_>>()),
        ] {
            assert!(decode(invalid.clone()).is_none(), "accepted {invalid}");
        }
        let roster = decode(json!([
            ["\u{10000}", "Manual"],
            ["helm", "Assisted"],
            ["\u{e000}", "Std"]
        ]))
        .unwrap();
        assert_eq!(
            roster.ships()[0]
                .crew
                .iter()
                .map(|(id, _)| id.0.as_str())
                .collect::<Vec<_>>(),
            ["helm", "\u{e000}", "\u{10000}"]
        );
    }
}
