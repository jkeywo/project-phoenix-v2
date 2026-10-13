//! Presentation-only acknowledgement, authorised through ordinary Console hosting.
use super::StationImportanceRes;
use crate::core::messages::{ClientMessage, ConsoleId, StationId};
use crate::lobby::{InboundMessage, Sessions};
use crate::ship::components::{ConsoleHosts, ShipConfigComponent};
use crate::station_importance::AttentionOwner;
use bevy::prelude::*;

fn may_visit(
    sessions: &Sessions,
    token: &str,
    config: &crate::ship::config::ShipConfig,
    id: &ConsoleId,
    hosts: &ConsoleHosts,
) -> bool {
    if sessions.0.is_afk(token)
        || sessions.0.is_spectator(token)
        || !sessions
            .0
            .players()
            .iter()
            .any(|player| player.token == token && player.connected)
    {
        return false;
    }
    if config.console(id).is_none() {
        return false;
    }
    hosts
        .0
        .iter()
        .find(|assignment| &assignment.console == id)
        .and_then(|assignment| assignment.host.as_ref())
        .is_some_and(|host| sessions.0.holder_for_station(host) == Some(token))
}

pub(crate) fn drain_station_visited(
    mut reader: MessageReader<InboundMessage>,
    sessions: Res<Sessions>,
    ships: Query<(&ShipConfigComponent, &ConsoleHosts), With<crate::server_app::LocalShip>>,
    importance: Option<ResMut<StationImportanceRes>>,
) {
    let Some(mut importance) = importance else {
        return;
    };
    let Ok((config, hosts)) = ships.single() else {
        return;
    };
    for event in reader.read() {
        match &event.msg {
            ClientMessage::ConsoleVisited { console }
                if may_visit(&sessions, &event.token, &config.0, console, hosts) =>
            {
                importance
                    .0
                    .visit(&AttentionOwner::Console(console.clone()));
            }
            // Existing Station-addressed callers acknowledge only their authorised
            // home Consoles. Visiting a Console never acknowledges its neighbours.
            ClientMessage::StationVisited { station } => {
                for console in config
                    .0
                    .consoles
                    .iter()
                    .filter(|console| console.home_station.as_ref() == Some(station))
                {
                    if may_visit(&sessions, &event.token, &config.0, &console.id, hosts) {
                        importance
                            .0
                            .visit(&AttentionOwner::Console(console.id.clone()));
                    }
                }
                if station == &StationId("core".into())
                    && sessions.0.players().iter().any(|player| {
                        player.token == event.token
                            && player.connected
                            && player.station.is_some()
                            && !sessions.0.is_afk(&event.token)
                            && !sessions.0.is_spectator(&event.token)
                    })
                {
                    importance.0.visit(&AttentionOwner::Core);
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
#[path = "console_visits_tests.rs"]
mod tests;
