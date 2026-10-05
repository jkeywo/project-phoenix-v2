//! A destroyed crew keeps its ship identity; only its camera follows another hull.
use crate::core::messages::{
    ClientMessage, CrewSpectatorPayload, CrewSpectatorShip, DeliveryClass, GamePhase, ServerMessage,
};
use crate::entities::spawner::{EntityName, EntitySystemHull, EntityUuid};
use crate::lobby::{InboundMessage, OutboundMessage, Sessions, Target};
use crate::server_app::{LocalShip, Ship};
use bevy::prelude::*;
use std::collections::{BTreeMap, BTreeSet};

/// Host-local presentation state. The original LocalShip and its private
/// projections never change when the shared camera follows another ship.
#[derive(Resource, Default)]
pub struct CrewSpectator {
    pub active: bool,
    pub target: Option<String>,
    crew_ship: Option<String>,
    original_crew: BTreeSet<String>,
    last_sent: BTreeMap<String, CrewSpectatorPayload>,
}

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CrewSpectatorPresentation;

pub struct CrewSpectatorPlugin;
impl Plugin for CrewSpectatorPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CrewSpectator>().add_systems(
            Update,
            update_crew_spectator
                .in_set(CrewSpectatorPresentation)
                // GM view expiry first clears its own force. A destroyed crew
                // then installs the cinematic force for this frame; without
                // this order the GM sync can clear it back to Camera.
                .after(crate::gm_presentation::sync_views)
                .run_if(resource_exists::<Sessions>),
        );
    }
}

/// Runs outside the authoritative tick: this is the local crew's camera and
/// reliable Console projection, shared by native and browser hosts.
fn update_crew_spectator(
    policy: CrewMissionPolicy,
    phase: Res<State<GamePhase>>,
    sessions: Res<Sessions>,
    local: Query<(&EntityUuid, &EntitySystemHull), With<LocalShip>>,
    ships: Query<(&EntityUuid, Option<&EntityName>, &EntitySystemHull), With<Ship>>,
    mut modes: Query<&mut crate::ship::state::ShipViewMode, With<LocalShip>>,
    mut state: ResMut<CrewSpectator>,
    mut inbound: MessageReader<InboundMessage>,
    mut outbound: MessageWriter<OutboundMessage>,
) {
    let messages: Vec<_> = inbound.read().collect();
    let dead = local
        .single()
        .ok()
        .filter(|(_, hull)| hull.0.is_destroyed());
    let active = *phase.get() == GamePhase::InProgress
        && policy.continues_after_ship_loss()
        && dead.is_some();
    if *phase.get() == GamePhase::Lobby {
        state.crew_ship = None;
        state.original_crew.clear();
    }
    if active {
        let uuid = &dead.unwrap().0 .0;
        if state.crew_ship.as_ref() != Some(uuid) {
            state.crew_ship = Some(uuid.clone());
            state.original_crew = sessions
                .0
                .players()
                .iter()
                .filter(|player| !player.spectator)
                .map(|player| player.token.clone())
                .collect();
            state.target = None;
        }
    }
    let was_active = state.active;
    state.active = active;
    let mut candidates: Vec<_> = if active {
        ships
            .iter()
            .filter(|(_, _, hull)| !hull.0.is_destroyed())
            .map(|(uuid, name, _)| CrewSpectatorShip {
                uuid: uuid.0.clone(),
                name: name.map(|n| n.0.clone()).unwrap_or_else(|| uuid.0.clone()),
            })
            .collect()
    } else {
        Vec::new()
    };
    candidates.sort_by(|a, b| a.uuid.cmp(&b.uuid));
    if !state
        .target
        .as_ref()
        .is_some_and(|uuid| candidates.iter().any(|ship| &ship.uuid == uuid))
    {
        state.target = candidates.first().map(|ship| ship.uuid.clone());
    }
    for ev in &messages {
        let ClientMessage::SelectCrewSpectatorTarget { uuid } = &ev.msg else {
            continue;
        };
        if active
            && state.original_crew.contains(&ev.token)
            && sessions
                .0
                .players()
                .iter()
                .any(|p| p.token == ev.token && p.connected)
            && candidates.iter().any(|ship| ship.uuid == *uuid)
        {
            state.target = Some(uuid.clone());
        }
    }
    if active || was_active {
        if let Ok(mut mode) = modes.single_mut() {
            mode.force_view_mode(active.then_some(crate::core::messages::ViewMode::Cinematic));
        }
    }
    for player in sessions.0.players().iter().filter(|p| p.connected) {
        let crew_active = active && state.original_crew.contains(&player.token);
        let payload = CrewSpectatorPayload {
            active: crew_active,
            can_select: crew_active,
            target: crew_active.then(|| state.target.clone()).flatten(),
            ships: if crew_active {
                candidates.clone()
            } else {
                Vec::new()
            },
        };
        let reconnect = messages
            .iter()
            .any(|ev| ev.token == player.token && matches!(ev.msg, ClientMessage::Identify { .. }));
        if reconnect || state.last_sent.get(&player.token) != Some(&payload) {
            state
                .last_sent
                .insert(player.token.clone(), payload.clone());
            outbound.write(OutboundMessage {
                target: Target::Token(player.token.clone()),
                msg: ServerMessage::CrewSpectatorState(payload),
                delivery: DeliveryClass::Reliable,
            });
        }
    }
}

/// Mission topology, independent of which crew this host projects.
#[derive(bevy::ecs::system::SystemParam)]
pub(crate) struct CrewMissionPolicy<'w> {
    world: Option<Res<'w, crate::world::config::WorldConfig>>,
    fleet: Option<Res<'w, crate::lockstep::FleetRoster>>,
}

impl CrewMissionPolicy<'_> {
    pub(crate) fn continues_after_ship_loss(&self) -> bool {
        self.world
            .as_ref()
            .is_some_and(|world| world.ship_slots.len() > 1)
            || self
                .fleet
                .as_ref()
                .is_some_and(|fleet| fleet.ships().len() > 1)
    }
}

#[cfg(test)]
#[path = "crew_spectator_tests.rs"]
mod tests;
