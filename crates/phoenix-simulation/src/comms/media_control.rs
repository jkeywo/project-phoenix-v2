//! One canonical selected call per actual Comms System; derived paid routes are
//! separate and are rebuilt inside the existing fixed radio pass.
use super::{radio::Endpoint, server::CommsInboxRes};
use crate::core::messages::{AdmittedCommands, SystemControlPayload};
use bevy::prelude::*;
use phoenix_model::{
    comms_channel::CommsAudience,
    comms_media::MediaMode,
    comms_request::{CommsRadioContent, CommsRequestDecision, GenericCommsRequest},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Canonical intent only. It neither opens a device nor retains a Session lease.
#[derive(Component, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShipCommsMediaIntent {
    pub origin_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MediaCallBinding {
    pub origin: String,
    pub source_ship: String,
    pub recipient_ship: String,
    pub channel: String,
    pub mode: MediaMode,
}
impl MediaCallBinding {
    pub fn for_local(&self, ship: &str) -> Option<super::media_session::EligibleMediaCall> {
        let peer = if self.source_ship == ship {
            &self.recipient_ship
        } else if self.recipient_ship == ship {
            &self.source_ship
        } else {
            return None;
        };
        Some(super::media_session::EligibleMediaCall {
            origin: self.origin.clone(),
            local_ship: ship.into(),
            peer_ship: peer.clone(),
            mode: self.mode,
        })
    }
}

/// Own inbox evidence only: recipient accepted its actual heard Request; source
/// actually heard the matching accepted Outcome and retains the original issue.
pub fn accepted_call(
    inbox: &CommsInboxRes,
    ship: &str,
    origin: &str,
    now: u64,
) -> Option<MediaCallBinding> {
    for message in inbox.0.iter().filter(|message| {
        message
            .recipient_ship
            .as_ref()
            .is_some_and(|recipient| recipient.0 == ship)
    }) {
        let Some(wire) = message.transmission.as_ref() else {
            continue;
        };
        if wire.validate(now).is_err() || wire.received_tick.is_none() {
            continue;
        }
        match &wire.content {
            CommsRadioContent::Request {
                request: GenericCommsRequest::MediaCall { mode },
            } if wire.origin_id == origin
                && message.sender_uuid != ship
                && matches!(&wire.audience, CommsAudience::Contact {target_uuid} if target_uuid == ship)
                && message.request_progress.as_ref().is_some_and(|progress| {
                    progress.validate(wire.received_tick, now).is_ok()
                        && progress
                            .facts
                            .decision
                            .as_ref()
                            .is_some_and(|decision| decision.choice == CommsRequestDecision::Accept)
                }) =>
            {
                return Some(MediaCallBinding {
                    origin: origin.into(),
                    source_ship: message.sender_uuid.clone(),
                    recipient_ship: ship.into(),
                    channel: wire.channel.clone(),
                    mode: *mode,
                });
            }
            CommsRadioContent::Outcome { outcome }
                if outcome.request.origin_id == origin
                    && outcome.request.source_ship == ship
                    && message.sender_uuid == outcome.request.recipient_ship
                    && matches!(&wire.audience, CommsAudience::Contact {target_uuid} if target_uuid == ship)
                    && outcome.facts.decision.as_ref().is_some_and(|decision| {
                        decision.choice == CommsRequestDecision::Accept
                    }) =>
            {
                let GenericCommsRequest::MediaCall { mode } = &outcome.request.request else {
                    continue;
                };
                if !inbox.0.iter().any(|original| original.sender_uuid == ship && original.is_for_ship(Some(ship))
                        && original.transmission.as_ref().is_some_and(|issued| issued.origin_id == origin
                            && issued.received_tick.is_none() && issued.emitted_tick == outcome.request.emitted_tick
                            && issued.channel == wire.channel
                            && matches!(&issued.audience, CommsAudience::Contact {target_uuid} if *target_uuid == outcome.request.recipient_ship)
                            && issued.content == CommsRadioContent::Request {request: outcome.request.request.clone()})) {continue;}
                return Some(MediaCallBinding {
                    origin: origin.into(),
                    source_ship: ship.into(),
                    recipient_ship: outcome.request.recipient_ship.clone(),
                    channel: wire.channel.clone(),
                    mode: *mode,
                });
            }
            _ => {}
        }
    }
    None
}

#[allow(clippy::type_complexity)]
pub(crate) fn apply_media_call_commands(
    mut ships: Query<(
        Entity,
        &crate::entities::spawner::EntityUuid,
        &super::radio::RadioProfile,
        &AdmittedCommands,
        Option<&mut ShipCommsMediaIntent>,
    )>,
    inbox: Res<CommsInboxRes>,
    tick: Res<crate::sim_tick::SimTick>,
    mut commands: Commands,
    mut outbox: Option<ResMut<crate::server_app::SimOutbox>>,
) {
    for (entity, ship, profile, admitted, mut intent) in &mut ships {
        for command in admitted.for_target(&profile.0.system.0) {
            let SystemControlPayload::SetCommsMediaCall { origin_id } = &command.payload else {
                continue;
            };
            let selected = origin_id
                .as_ref()
                .and_then(|origin| accepted_call(&inbox, &ship.0, origin, tick.0));
            let applied = origin_id.is_none()
                || selected.as_ref().is_some_and(|call| {
                    profile
                        .0
                        .media
                        .is_some_and(|capability| capability.supports(call.mode))
                });
            if applied {
                if let Some(current) = intent.as_deref_mut() {
                    current.origin_id = origin_id.clone();
                } else {
                    commands.entity(entity).insert(ShipCommsMediaIntent {
                        origin_id: origin_id.clone(),
                    });
                }
            }
            if let Some(outbox) = outbox.as_deref_mut() {
                crate::command_admission::feedback::finish_outbox_action_feedback(
                    command,
                    outbox,
                    if applied {
                        crate::core::messages::ActionFeedbackOutcome::Applied
                    } else {
                        crate::core::messages::ActionFeedbackOutcome::Refused
                    },
                );
            }
        }
    }
}

/// Current derived eligibility only. Never serialize or carry across restore.
#[derive(Resource, Default, Debug)]
pub struct PaidMediaCarriers {
    sample_tick: Option<u64>,
    routes: BTreeMap<(String, String), MediaCallBinding>,
}
impl PaidMediaCarriers {
    pub fn clear(&mut self) {
        self.sample_tick = None;
        self.routes.clear();
    }
    pub fn call(
        &self,
        ship: &str,
        peer: &str,
        now: u64,
    ) -> Option<super::media_session::EligibleMediaCall> {
        if self.sample_tick != Some(now) {
            return None;
        }
        self.routes
            .get(&(ship.into(), peer.into()))?
            .for_local(ship)
    }
    pub fn local_calls(
        &self,
        ship: &str,
        now: u64,
    ) -> Vec<super::media_session::EligibleMediaCall> {
        if self.sample_tick != Some(now) {
            return Vec::new();
        }
        self.routes
            .iter()
            .filter(|((source, _), _)| source == ship)
            .filter_map(|(_, call)| call.for_local(ship))
            .collect()
    }
}

pub(crate) fn derive_paid_carriers(
    cache: &mut PaidMediaCarriers,
    endpoints: &[Endpoint],
    intents: &Query<&ShipCommsMediaIntent>,
    inbox: &CommsInboxRes,
    paths: &crate::regions::signal_path::RegionSignalQuery<'_, '_>,
    now: u64,
) {
    cache.clear();
    cache.sample_tick = Some(now);
    for source in endpoints.iter().filter(|endpoint| endpoint.listening > 0.0) {
        let Some(origin) = intents
            .get(source.entity)
            .ok()
            .and_then(|intent| intent.origin_id.as_deref())
        else {
            continue;
        };
        let Some(call) = accepted_call(inbox, &source.uuid, origin, now) else {
            continue;
        };
        if !source
            .radio
            .media
            .is_some_and(|capability| capability.supports(call.mode))
        {
            continue;
        }
        let Some(local) = call.for_local(&source.uuid) else {
            continue;
        };
        let Some(receiver) = endpoints
            .iter()
            .find(|endpoint| endpoint.uuid == local.peer_ship && endpoint.listening > 0.0)
        else {
            continue;
        };
        if !receiver
            .radio
            .media
            .is_some_and(|capability| capability.supports(call.mode))
            || !intents
                .get(receiver.entity)
                .is_ok_and(|intent| intent.origin_id.as_deref() == Some(origin))
            || accepted_call(inbox, &receiver.uuid, origin, now).as_ref() != Some(&call)
            || !super::radio::route_available(endpoints, source, receiver, &call.channel, paths)
        {
            continue;
        }
        cache
            .routes
            .insert((source.uuid.clone(), receiver.uuid.clone()), call);
    }
}

/// Selection uses the ordinary admitted System transaction; radio derives routes
/// later in Physics from the once-paid endpoints. Session leases are wall-clock
/// transport bookkeeping and are deliberately absent from fixed state.
pub(crate) fn register(app: &mut App) {
    use crate::authoritative::{DeclareState, StateClass};
    use crate::sim_sets::{FixedStep, SimSet};
    app.declare_state::<ShipCommsMediaIntent>(StateClass::Folded, "ship-comms-media-intent")
        .declare_state::<PaidMediaCarriers>(StateClass::Derived, "comms-media-current-carriers")
        .declare_state::<super::media_session::MediaLeases>(
            StateClass::Timer,
            "comms-media-session-leases",
        )
        .declare_state::<super::media_edge::MediaEdge>(
            StateClass::Timer,
            "comms-media-session-leases",
        )
        .declare_state::<super::media_mesh::MediaMesh>(
            StateClass::Timer,
            "comms-media-session-leases",
        )
        .init_resource::<super::media_edge::MediaEdge>()
        .init_resource::<super::media_mesh::MediaMesh>()
        .add_systems(PostUpdate, super::media_edge::process)
        .init_resource::<PaidMediaCarriers>()
        .init_resource::<super::media_session::MediaLeases>()
        .add_systems(
            FixedUpdate,
            apply_media_call_commands
                .in_set(SimSet::Input)
                .in_set(FixedStep::HandleCommsMediaCall),
        );
}
