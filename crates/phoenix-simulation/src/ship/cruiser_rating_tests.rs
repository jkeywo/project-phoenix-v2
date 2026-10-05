use super::components::{ActiveStationRatings, ShipConfigComponent, ShipSystemControlSources};
use super::rating::{resolve_system_depths, SystemDepth, BACKFILL_RATING};
use crate::core::messages::{ClientMessage, ServerMessage, SystemId};
use bevy::prelude::*;
use std::collections::HashMap;

fn cruiser() -> super::config::ShipConfig {
    crate::entities::include_resolve::load_entity_config("assets/entities/alliance_cruiser.toml")
        .unwrap()
        .ship_config
        .unwrap()
}

#[test]
fn cruiser_rungs_are_distinct_monotone_bundles_and_floor_only_raises_selected_systems() {
    let config = cruiser();
    for station in &config.stations {
        let mut previous = None;
        for rung in &station.ratings {
            let depths =
                resolve_system_depths(&config, &station.id, &rung.name, &HashMap::new()).unwrap();
            assert!(!depths.is_empty());
            if let Some(higher) = previous {
                assert_ne!(depths, higher, "duplicate rung on {:?}", station.id);
                assert!(depths.iter().all(|(id, depth)| *depth <= higher[id]));
            }
            for id in depths.keys() {
                let floor = HashMap::from([(id.clone(), SystemDepth::Detailed)]);
                let raised =
                    resolve_system_depths(&config, &station.id, &rung.name, &floor).unwrap();
                for (other, depth) in &depths {
                    assert_eq!(
                        raised[other],
                        if other == id {
                            SystemDepth::Detailed
                        } else {
                            *depth
                        }
                    );
                }
            }
            previous = Some(depths);
        }
        assert!(
            resolve_system_depths(&config, &station.id, BACKFILL_RATING, &HashMap::new())
                .unwrap()
                .values()
                .all(|depth| *depth == SystemDepth::Ai)
        );
    }
    let repair = SystemId("repair".into());
    let engineering = crate::core::messages::StationId("engineering".into());
    assert_eq!(
        resolve_system_depths(&config, &engineering, "Guided", &HashMap::new()).unwrap()[&repair],
        SystemDepth::Simplified
    );
}

#[test]
fn cruiser_lobby_roster_and_live_handler_support_every_authored_rung() {
    let config = cruiser();
    let roster = crate::lobby::stations_config::stations_from_ship_config(&config);
    let mut app = App::new();
    app.add_message::<crate::lobby::InboundMessage>()
        .init_resource::<crate::lobby::LobbyOutbox>()
        .init_resource::<crate::lobby::crew_replication::PendingCrewRatingChanges>()
        .add_systems(Update, super::rating_systems::handle_station_rating_change);
    let mut sessions = crate::lobby::session::SessionManager::new();
    sessions.register("crew".into(), "Crew".into()).unwrap();
    app.insert_resource(crate::lobby::Sessions(sessions));
    let ship = app
        .world_mut()
        .spawn((
            crate::server_app::LocalShip,
            ShipConfigComponent(config.clone()),
            ShipSystemControlSources::default(),
            ActiveStationRatings::default(),
        ))
        .id();
    for station in &config.stations {
        let offered = &roster
            .stations
            .iter()
            .find(|entry| entry.id == station.id)
            .unwrap()
            .ratings;
        assert_eq!(
            offered,
            &station
                .ratings
                .iter()
                .map(|rung| rung.name.clone())
                .collect::<Vec<_>>()
        );
        assert!(!offered.iter().any(|name| name == BACKFILL_RATING));
        app.world_mut()
            .resource_mut::<crate::lobby::Sessions>()
            .0
            .set_station("crew", Some(station.id.clone()));
        for name in offered
            .iter()
            .map(String::as_str)
            .chain(std::iter::once(BACKFILL_RATING))
        {
            app.world_mut()
                .resource_mut::<crate::lobby::LobbyOutbox>()
                .0
                .clear();
            app.world_mut().write_message(crate::lobby::InboundMessage {
                token: "crew".into(),
                msg: ClientMessage::SetStationRating {
                    rating_name: name.into(),
                },
            });
            app.update();
            assert_eq!(
                app.world().get::<ActiveStationRatings>(ship).unwrap().0[&station.id],
                name
            );
            let sources = &app.world().get::<ShipSystemControlSources>(ship).unwrap().0;
            for (id, depth) in
                resolve_system_depths(&config, &station.id, name, &HashMap::new()).unwrap()
            {
                assert_eq!(sources.source_for(&id), depth.control_source());
            }
            assert!(app.world().resource::<crate::lobby::LobbyOutbox>().0.iter().any(|(_, message)| matches!(message,
                ServerMessage::RatingChanged { station_id, rating_name } if station_id == &station.id && rating_name == name)));
        }
    }
}

use crate::ship::rating::SystemDepthRuntime as _;
