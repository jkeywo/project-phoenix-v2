use bevy::prelude::*;

use crate::core::messages::ClientMessage;
use crate::lobby::{InboundMessage, Sessions};
use crate::server_app::LocalShip;
use crate::ship::components::{
    ActiveStationRatings, ShipConfigComponent, ShipSystemControlSources,
};
use crate::ship::rating;

// ── Station Rating Handler ──────────────────────────────────────────────────

/// System that processes `SetStationRating` messages from players mid-game.
/// Resolves the sender's station from their held consoles, looks up the
/// rating in the ship config, and updates `ShipSystemControlSources` and
/// `ActiveStationRatings` accordingly. An active fleet stages the choice for
/// admission next tick; only the shared command consumer changes live state.
pub fn handle_station_rating_change(
    fleet: Option<Res<crate::lockstep::FleetLockstep>>,
    mut intents: ResMut<crate::lobby::crew_replication::PendingCrewRatingChanges>,
    mut reader: MessageReader<InboundMessage>,
    sessions: Res<Sessions>,
    mut ship_components: Query<
        (
            &ShipConfigComponent,
            &mut ShipSystemControlSources,
            &mut ActiveStationRatings,
        ),
        With<LocalShip>,
    >,
    mut outbox: ResMut<crate::lobby::LobbyOutbox>,
) {
    let messages: Vec<_> = reader.read().collect();
    for (ship_config, mut control_sources, mut active_ratings) in ship_components.iter_mut() {
        for ev in messages.iter() {
            let ClientMessage::SetStationRating { rating_name } = &ev.msg else {
                continue;
            };

            let station_id = sessions.0.station_for_token(&ev.token).cloned();
            let Some(station_id) = station_id else {
                continue;
            };
            if sessions.0.is_afk(&ev.token) {
                continue;
            }

            // A holder may select only a rating on their own hull's station.
            // Refuse before changing either the replicated intent or UI state.
            if rating::resolve_automated_systems(&ship_config.0, &station_id, rating_name).is_none()
            {
                continue;
            }

            if fleet.is_some() {
                intents.push(station_id.clone(), rating_name.clone());
            } else {
                rating::apply_rating(
                    &ship_config.0,
                    &station_id,
                    rating_name,
                    &mut control_sources.0,
                );

                active_ratings
                    .0
                    .insert(station_id.clone(), rating_name.clone());
            }

            outbox.0.push((
                crate::lobby::handler::Target::All,
                crate::core::messages::ServerMessage::RatingChanged {
                    station_id,
                    rating_name: rating_name.clone(),
                },
            ));
        }
    }
}

#[cfg(test)]
#[path = "rating_systems_tests.rs"]
mod tests;
