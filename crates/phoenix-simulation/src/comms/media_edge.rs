//! Bounded ephemeral device ingress and selected-call routing. This module
//! reads current canonical authority/carriers and never writes simulation state.
use super::{
    media_control::PaidMediaCarriers,
    media_session::{EligibleMediaCall, MediaLeases, MediaOperator},
};
use bevy::prelude::*;
use phoenix_model::{
    comms_media::MediaSessionView,
    messages::{ClientMessage, ServerMessage, SystemControlPayload},
};
use std::collections::{BTreeMap, VecDeque};

const MAX_PENDING: usize = 64;
/// One edge owns actual physical connection handles. Callbacks cannot grant
/// Station authority; the processing pass independently resolves it below.
#[derive(Resource, Default)]
pub struct MediaEdge {
    connections: BTreeMap<String, u64>,
    pending: VecDeque<(String, u64, ClientMessage)>,
    outbound: Vec<(String, ServerMessage)>,
    last_views: BTreeMap<String, MediaSessionView>,
}
impl MediaEdge {
    pub fn invalidate(&mut self) {
        self.pending.clear();
        self.outbound.clear();
        self.last_views.clear();
    }
    pub fn connected(&mut self, token: String, connection: u64) {
        if self.current(&token) != Some(connection) {
            self.last_views.remove(&token);
            self.outbound.retain(|(target, _)| target != &token);
        }
        self.connections.insert(token, connection);
    }
    pub fn disconnected(&mut self, token: &str) {
        self.connections.remove(token);
        self.last_views.remove(token);
        self.outbound.retain(|(target, _)| target != token);
    }
    pub fn sync_connections(&mut self, current: BTreeMap<String, u64>) {
        let changed: Vec<_> = self
            .connections
            .iter()
            .filter(|(token, id)| current.get(*token) != Some(*id))
            .map(|(token, _)| token.clone())
            .collect();
        for token in changed {
            self.last_views.remove(&token);
            self.outbound.retain(|(target, _)| target != &token);
        }
        self.connections = current;
    }
    pub fn current(&self, token: &str) -> Option<u64> {
        self.connections.get(token).copied()
    }
    pub fn offer(&mut self, token: String, connection: u64, message: ClientMessage) -> bool {
        if self.current(&token) != Some(connection) || self.pending.len() >= MAX_PENDING {
            return false;
        }
        match &message {
            ClientMessage::CommsMediaFrame(packet) if packet.validate().is_ok() => {}
            ClientMessage::CommsMediaStart(start)
                if phoenix_model::comms_channel::identity(&start.call_origin_id) => {}
            ClientMessage::CommsMediaStop(_) => {}
            _ => return false,
        }
        self.pending.push_back((token, connection, message));
        true
    }
    fn publish_view(&mut self, token: &str, view: MediaSessionView) {
        if self.last_views.get(token) == Some(&view) || self.outbound.len() >= MAX_PENDING {
            return;
        }
        self.last_views.insert(token.into(), view.clone());
        self.outbound
            .push((token.into(), ServerMessage::CommsMediaState(view)));
    }
    fn retire_view(&mut self, token: &str) {
        if self.outbound.len() >= MAX_PENDING {
            return;
        }
        self.last_views.remove(token);
        self.outbound.push((
            token.into(),
            ServerMessage::CommsMediaState(MediaSessionView::default()),
        ));
    }
    pub fn take_outbound(&mut self) -> Vec<(String, ServerMessage)> {
        std::mem::take(&mut self.outbound)
    }
}

/// A complete current local authority projection. Co-hosted information alone
/// does not satisfy the existing System command policy.
#[allow(clippy::type_complexity)]
fn current_operators(
    edge: &MediaEdge,
    sessions: &crate::lobby::Sessions,
    ships: &Query<
        (
            &crate::entities::spawner::EntityUuid,
            &crate::ship_plugin::ShipConfigComponent,
            &crate::ship_plugin::ShipSystemControlSources,
            Option<&crate::ship_plugin::HumanSeekingHosts>,
            &super::radio::RadioProfile,
        ),
        With<crate::server_app::LocalShip>,
    >,
) -> Vec<MediaOperator> {
    let mut out = Vec::new();
    for (ship, config, sources, hosts, radio) in ships {
        let system = &radio.0.system;
        let Some(console) = config.0.console_for_system(system) else {
            continue;
        };
        let Some(station) = crate::command_admission::station_for_system(&config.0, hosts, system)
        else {
            continue;
        };
        for player in sessions
            .0
            .players()
            .iter()
            .filter(|player| player.connected && !player.spectator && !player.afk)
        {
            let Some(connection) = edge.current(&player.token) else {
                continue;
            };
            if crate::command_admission::is_command_authorized(
                &player.token,
                system,
                &SystemControlPayload::SetCommsMediaCall { origin_id: None },
                sources,
                sessions,
                &config.0,
                hosts,
            ) {
                out.push(MediaOperator {
                    token: player.token.clone(),
                    connection,
                    ship: ship.0.clone(),
                    system: system.clone(),
                    console: console.id.clone(),
                    station: station.clone(),
                });
            }
        }
    }
    out
}
fn call_for(
    paid: &PaidMediaCarriers,
    operator: &MediaOperator,
    tick: u64,
) -> Option<EligibleMediaCall> {
    paid.local_calls(&operator.ship, tick.checked_sub(1)?)
        .into_iter()
        .next()
}

#[allow(clippy::type_complexity)]
pub fn process(
    mut edge: ResMut<MediaEdge>,
    mut leases: ResMut<MediaLeases>,
    paid: Res<PaidMediaCarriers>,
    mut mesh: ResMut<super::media_mesh::MediaMesh>,
    roster: Option<Res<crate::lockstep::FleetRoster>>,
    fleet_ships: Query<(
        &crate::entities::spawner::EntityUuid,
        &crate::lockstep::FleetSlotOf,
    )>,
    sessions: Option<Res<crate::lobby::Sessions>>,
    tick: Option<Res<crate::sim_tick::SimTick>>,
    clock: Option<Res<Time<Real>>>,
    ships: Query<
        (
            &crate::entities::spawner::EntityUuid,
            &crate::ship_plugin::ShipConfigComponent,
            &crate::ship_plugin::ShipSystemControlSources,
            Option<&crate::ship_plugin::HumanSeekingHosts>,
            &super::radio::RadioProfile,
        ),
        With<crate::server_app::LocalShip>,
    >,
) {
    let (Some(sessions), Some(tick), Some(clock)) = (sessions, tick, clock) else {
        return;
    };
    let operators = current_operators(&edge, &sessions, &ships);
    let raw_calls: Vec<_> = operators
        .iter()
        .filter_map(|operator| {
            call_for(&paid, operator, tick.0).map(|call| (operator.clone(), call))
        })
        .collect();

    let now = (clock.elapsed_secs_f64() * 1000.0).max(0.0) as u64;
    mesh.reconcile(now, |call| {
        raw_calls.iter().any(|(_, current)| current == call)
    });
    if let Some(roster) = roster.as_ref() {
        let local = roster.local().0;
        for (authenticated, frame) in mesh.drain() {
            let body = frame.d;
            if body.to != local
                || !roster.is_member(crate::command_admission::HostSlot(body.from))
                || (authenticated != body.from && authenticated != roster.owner().0)
            {
                continue;
            }
            let Some(source) = fleet_ships
                .iter()
                .find_map(|(uuid, slot)| (slot.0 .0 == body.from).then_some(uuid.0.as_str()))
            else {
                continue;
            };
            for (operator, call) in &raw_calls {
                if call.peer_ship != source || call.origin != body.call_origin_id {
                    continue;
                }
                if let Some(packet) = body.packet.as_ref() {
                    if leases.generation(operator, call).is_some()
                        && mesh.admit(call, packet, now)
                        && edge.outbound.len() < MAX_PENDING
                    {
                        edge.outbound.push((
                            operator.token.clone(),
                            ServerMessage::CommsMediaFrame(packet.clone()),
                        ));
                    }
                } else {
                    leases.raise_floor(operator, call, body.generation_floor);
                    mesh.announce(call, body.generation, now);
                }
            }
        }
        if mesh.heartbeat(now) {
            for (operator, call) in &raw_calls {
                if let Some(to) = fleet_ships
                    .iter()
                    .find_map(|(uuid, slot)| (uuid.0 == call.peer_ship).then_some(slot.0 .0))
                {
                    mesh.push(super::media_mesh::MediaHostBody {
                        from: local,
                        to,
                        call_origin_id: call.origin.clone(),
                        generation: leases.generation(operator, call),
                        generation_floor: mesh.required_floor(call),
                        packet: None,
                    });
                }
            }
        }
    }
    let calls: Vec<_> = raw_calls
        .iter()
        .filter(|(operator, call)| {
            mesh.route_available(call, now)
                || raw_calls.iter().any(|(peer, other)| {
                    peer.ship == call.peer_ship
                        && other.peer_ship == operator.ship
                        && other.origin == call.origin
                })
        })
        .cloned()
        .collect();
    leases.reconcile(|operator, call| {
        calls
            .iter()
            .any(|(current, current_call)| current == operator && current_call == call)
    });
    while let Some((token, connection, message)) = edge.pending.pop_front() {
        let Some((operator, call)) = calls
            .iter()
            .find(|(operator, _)| operator.token == token && operator.connection == connection)
        else {
            continue;
        };
        match message {
            ClientMessage::CommsMediaStart(start) if start.call_origin_id == call.origin => {
                let _ = leases.start(operator.clone(), call.clone(), now);
            }
            ClientMessage::CommsMediaStop(stop) => {
                leases.stop(operator, stop.lease_generation);
            }
            ClientMessage::CommsMediaFrame(packet) => {
                if leases.admit(operator, Some(call), &packet, now).is_ok() {
                    if let Some(roster) = roster.as_ref() {
                        if mesh.generation(call).is_some() {
                            if let Some(to) = fleet_ships.iter().find_map(|(uuid, slot)| {
                                (uuid.0 == call.peer_ship).then_some(slot.0 .0)
                            }) {
                                mesh.push(super::media_mesh::MediaHostBody {
                                    from: roster.local().0,
                                    to,
                                    call_origin_id: call.origin.clone(),
                                    generation: Some(packet.lease_generation),
                                    generation_floor: 0,
                                    packet: Some(packet.clone()),
                                });
                            }
                        }
                    }
                    // Actual local peer only. Cross-host admission uses the
                    // authenticated frozen-slot adapter, never a wire token.
                    for (peer, peer_call) in &calls {
                        if peer.ship == call.peer_ship
                            && peer_call.peer_ship == operator.ship
                            && peer_call.origin == call.origin
                            && leases.generation(peer, peer_call).is_some()
                        {
                            if edge.outbound.len() < MAX_PENDING {
                                edge.outbound.push((
                                    peer.token.clone(),
                                    ServerMessage::CommsMediaFrame(packet.clone()),
                                ));
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
    for operator in &operators {
        let call = calls
            .iter()
            .find(|(current, _)| current == operator)
            .map(|(_, call)| call);
        let view = call.map_or_else(MediaSessionView::default, |call| MediaSessionView {
            call_origin_id: Some(call.origin.clone()),
            mode: Some(call.mode),
            eligible: true,
            lease_generation: leases.generation(operator, call),
            peer_lease_generation: mesh.generation(call).or_else(|| {
                calls.iter().find_map(|(peer, peer_call)| {
                    (peer.ship == call.peer_ship
                        && peer_call.peer_ship == operator.ship
                        && peer_call.origin == call.origin)
                        .then(|| leases.generation(peer, peer_call))
                        .flatten()
                })
            }),
        });
        edge.publish_view(&operator.token, view);
    }
    // A former holder also needs an explicit stop view, even when silent.
    let retired: Vec<_> = edge
        .last_views
        .keys()
        .filter(|token| !operators.iter().any(|operator| &operator.token == *token))
        .cloned()
        .collect();
    for token in retired {
        edge.retire_view(&token);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn full_frame_queue_does_not_cache_unsent_lease_or_authority_loss_views() {
        let mut edge = MediaEdge::default();
        let mut initial = MediaSessionView::default();
        initial.lease_generation = Some(1);
        edge.publish_view("operator", initial.clone());
        edge.take_outbound();
        for _ in 0..MAX_PENDING {
            edge.outbound.push((
                "other".into(),
                ServerMessage::CommsMediaState(MediaSessionView::default()),
            ));
        }
        let mut next = initial.clone();
        next.lease_generation = Some(2);
        edge.publish_view("operator", next.clone());
        assert_eq!(edge.last_views["operator"], initial);
        edge.take_outbound();
        edge.publish_view("operator", next.clone());
        assert!(
            matches!(edge.take_outbound().as_slice(),[(token,ServerMessage::CommsMediaState(view))] if token=="operator"&&view==&next)
        );
        for _ in 0..MAX_PENDING {
            edge.outbound.push((
                "other".into(),
                ServerMessage::CommsMediaState(MediaSessionView::default()),
            ));
        }
        edge.retire_view("operator");
        assert!(edge.last_views.contains_key("operator"));
        edge.take_outbound();
        edge.retire_view("operator");
        assert!(!edge.last_views.contains_key("operator"));
        assert!(
            matches!(edge.take_outbound().as_slice(),[(token,ServerMessage::CommsMediaState(view))] if token=="operator"&&view==&MediaSessionView::default())
        );
    }
    #[test]
    fn a_new_physical_incarnation_receives_even_an_identical_eligible_view() {
        let mut edge = MediaEdge::default();
        edge.connected("operator".into(), 1);
        let view = MediaSessionView {
            eligible: true,
            ..Default::default()
        };
        edge.publish_view("operator", view.clone());
        edge.take_outbound();
        edge.sync_connections(BTreeMap::from([("operator".into(), 2)]));
        edge.publish_view("operator", view.clone());
        assert!(
            matches!(edge.take_outbound().as_slice(),[(token,ServerMessage::CommsMediaState(actual))] if token=="operator"&&actual==&view)
        );
    }
    #[test]
    fn restore_clears_queued_starts_and_frames_but_keeps_actual_connection_scope() {
        let mut edge = MediaEdge::default();
        edge.connected("operator".into(), 7);
        assert!(edge.offer(
            "operator".into(),
            7,
            ClientMessage::CommsMediaStart(phoenix_model::comms_media::MediaStart {
                call_origin_id: "accepted-call".into()
            })
        ));
        edge.invalidate();
        assert!(edge.pending.is_empty());
        assert_eq!(edge.current("operator"), Some(7));
        edge.connected("operator".into(), 8);
        assert!(!edge.offer(
            "operator".into(),
            7,
            ClientMessage::CommsMediaStart(phoenix_model::comms_media::MediaStart {
                call_origin_id: "accepted-call".into()
            })
        ));
    }
}
