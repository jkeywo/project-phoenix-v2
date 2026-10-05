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
    fleet: Option<Res<'w, crate::lockstep::FleetLockstep>>,
    pending: Option<Res<'w, crate::command_admission::PendingCommands>>,
    intents: ResMut<'w, crate::lobby::crew_replication::PendingCrewRatingChanges>,
    outbox: ResMut<'w, LobbyOutbox>,
    next_state: ResMut<'w, NextState<GamePhase>>,
    ship: Query<
        'w,
        's,
        (
            Entity,
            Option<&'static crate::entities::spawner::EntityUuid>,
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
            .map(|(entity, uuid, _, _, ratings)| self.project_ratings(entity, uuid, ratings))
            .unwrap_or_else(|_| sessions.pending_ratings().clone())
    }

    /// AFK snapshots a loaded Ship's latest crew choice, including a fleet
    /// choice waiting for its agreed tick. Before spawn it uses an empty map.
    pub fn afk_ratings(&self) -> HashMap<StationId, String> {
        self.ship
            .single()
            .map(|(entity, uuid, _, _, ratings)| self.project_ratings(entity, uuid, ratings))
            .unwrap_or_default()
    }

    // Lobby handlers run before AdmissionSet. Due commands are therefore still
    // pending here, or were applied last tick; there is no drained-but-unapplied
    // gap. Preserve a within-delay choice when AFK/disconnect saves its rating.
    fn project_ratings(
        &self,
        entity: Entity,
        uuid: Option<&crate::entities::spawner::EntityUuid>,
        ratings: &ActiveStationRatings,
    ) -> HashMap<StationId, String> {
        let mut projected = ratings.0.clone();
        if self.fleet.is_some() {
            self.intents
                .overlay(&mut projected, self.pending.as_deref(), entity, uuid);
        }
        projected
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
            if let Ok((_, _, config, mut sources, mut ratings)) = self.ship.single_mut() {
                if self.fleet.is_some() {
                    self.intents.push(station, name);
                } else {
                    rating::apply_rating(&config.0, &station, &name, &mut sources.0);
                    ratings.0.insert(station, name);
                }
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
        let projected = self.afk_ratings();
        let result = if let Ok((_, _, config, sources, _)) = self.ship.single_mut() {
            // The pure disconnect helper also writes its resolver. Apply its
            // returned intent below, so a fleet never mutates the live source
            // ahead of the agreed command tick.
            let mut departure_sources = sources.0.clone();
            handler::process_disconnect_with_stations(
                token,
                sessions,
                stations,
                &config.0,
                &mut departure_sources,
                &projected,
                phase,
                preload_complete,
            )
        } else {
            handler::process_disconnect(token, sessions, phase, preload_complete)
        };
        self.apply(result);
    }
}
