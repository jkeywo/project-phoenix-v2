//! Complete application of a pure Lobby handler result in the Bevy host.
//!
//! The adapter resolves whether the LocalShip exists yet. Message systems keep
//! their own readers, phase gates and pure handler; they do not reproduce the
//! loaded-Ship/pre-spawn projection branches or invent temporary rating sinks.

use bevy::{ecs::system::SystemParam, prelude::*};
use std::collections::HashMap;

use super::{CountdownTimer, LobbyOutbox};
use crate::lobby::{
    handler::{self, CountdownAction, LobbyHandlerResult, Target},
    session::SessionManager,
    stations_config::ShipStations,
};
use crate::{
    core::messages::{GamePhase, ServerMessage, StationId},
    ship::rating,
    ship_plugin::{ActiveStationRatings, ShipConfigComponent, ShipSystemControlSources},
};

/// The Bevy-owned output half of each per-variant Lobby message system.
#[derive(SystemParam)]
pub struct LobbyResultApplier<'w, 's> {
    outbox: ResMut<'w, LobbyOutbox>,
    next_state: ResMut<'w, NextState<GamePhase>>,
    ship: Query<
        'w,
        's,
        (
            &'static ShipConfigComponent,
            &'static mut ShipSystemControlSources,
            &'static mut ActiveStationRatings,
        ),
        With<crate::server_app::LocalShip>,
    >,
    countdown: Option<ResMut<'w, CountdownTimer>>,
}

impl LobbyResultApplier<'_, '_> {
    /// Welcome shows the live Ship's ratings, or the crew's pending Lobby
    /// choices before that Ship exists. The handler receives a stable snapshot.
    pub fn identify_ratings(&self, sessions: &SessionManager) -> HashMap<StationId, String> {
        self.ship
            .single()
            .map(|(_, _, ratings)| ratings.0.clone())
            .unwrap_or_else(|_| sessions.pending_ratings().clone())
    }

    /// AFK snapshots only a rating already applied to a Ship. Its existing
    /// pre-spawn policy uses an empty map, rather than a pending Lobby choice.
    pub fn afk_ratings(&self) -> HashMap<StationId, String> {
        self.ship
            .single()
            .map(|(_, _, ratings)| ratings.0.clone())
            .unwrap_or_default()
    }

    pub fn send(&mut self, target: Target, message: ServerMessage) {
        self.outbox.0.push((target, message));
    }

    /// Apply countdown, phase, Station Rating / Control Source and outbound
    /// messages in their existing order. Before spawn, the pure handlers own
    /// pending choices in SessionManager; no temporary ActiveStationRatings
    /// component pretends to retain a projection which has no Ship yet.
    pub fn apply(&mut self, result: LobbyHandlerResult) {
        if let Some(action) = result.countdown_action {
            if let Some(timer) = self.countdown.as_deref_mut() {
                match action {
                    CountdownAction::Start {
                        secs,
                        pending_phase,
                    } if timer.local_start_allowed && timer.remaining_secs <= 0.0 => {
                        timer.remaining_secs = secs as f32;
                        timer.pending_phase = Some(pending_phase);
                        self.outbox.0.push((
                            Target::All,
                            ServerMessage::GameStartCountdown {
                                remaining_secs: secs,
                            },
                        ));
                    }
                    CountdownAction::Cancel if timer.remaining_secs > 0.0 => {
                        timer.remaining_secs = 0.0;
                        timer.pending_phase = None;
                        self.outbox.0.push((
                            Target::All,
                            ServerMessage::GameStartCountdown { remaining_secs: 0 },
                        ));
                    }
                    _ => {}
                }
            }
        }
        if let Some(phase) = result.new_phase {
            self.next_state.set(phase);
        }
        if let Some((station, name)) = result.station_rating_update {
            if let Ok((config, mut sources, mut ratings)) = self.ship.single_mut() {
                rating::apply_rating(&config.0, &station, &name, &mut sources.0);
                ratings.0.insert(station, name);
            }
        }
        self.outbox.0.extend(result.outbound);
    }

    /// Departure needs the pre-disconnect rating and the loaded Ship resolver
    /// when present. Resolve that context here, evaluate one pure path, then
    /// finish its result through the same projection used by every message.
    pub fn disconnect(
        &mut self,
        token: &str,
        sessions: &mut SessionManager,
        stations: &ShipStations,
        phase: GamePhase,
        preload_complete: bool,
    ) {
        let result = if let Ok((config, mut sources, ratings)) = self.ship.single_mut() {
            handler::process_disconnect_with_stations(
                token,
                sessions,
                stations,
                &config.0,
                &mut sources.0,
                &ratings.0,
                phase,
                preload_complete,
            )
        } else {
            handler::process_disconnect(token, sessions, phase, preload_complete)
        };
        self.apply(result);
    }
}
