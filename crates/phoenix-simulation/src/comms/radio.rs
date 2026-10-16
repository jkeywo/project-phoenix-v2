//! Paid in-world text on authored equipment. Delivery never changes local send
//! feedback, and player text never executes as a machinery command or script.
use super::server::{CommsInboxRes, CommsRuntime};
use crate::{
    core::messages::{
        AdmittedCommands, CommsMessage, CommsPriority, SystemBlackboard, SystemControlPayload,
    },
    entities::spawner::{EntityName, EntitySystemHull, EntityUuid, FactionComponent},
    ship::{compartment_damage::ShipDamage, power::ShipPowerSystem, state::ShipPhysics},
    ship_plugin::{ShipConfigComponent, ShipSystemControlSources},
};
use bevy::prelude::*;
use phoenix_model::comms_channel::{CommsAudience, CommsRadioConfig, CommsTransmission};

/// Immutable accepted equipment, rebuilt through ordinary entity installation.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct RadioProfile(pub CommsRadioConfig);

pub(crate) fn install(
    config: &crate::entities::config::EntityConfig,
    commands: &mut EntityCommands,
) {
    if let Some(radio) = config.comms.as_ref().and_then(|comms| comms.radio.as_ref()) {
        commands.insert((
            RadioProfile(radio.clone()),
            super::media_control::ShipCommsMediaIntent::default(),
        ));
    }
    super::capture::install(config, commands);
}

#[derive(Clone)]
pub(crate) struct Endpoint {
    pub(crate) entity: Entity,
    pub(crate) uuid: String,
    pub(crate) name: String,
    pub(crate) point: Vec3,
    pub(crate) radio: CommsRadioConfig,
    pub(crate) listening: f32,
    pub(crate) fleet: bool,
    pub(crate) faction: Option<String>,
}

pub(crate) type Sources<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        &'static EntityUuid,
        &'static RadioProfile,
        Option<&'static ShipPhysics>,
        &'static Transform,
        &'static ShipConfigComponent,
        &'static ShipSystemControlSources,
        &'static AdmittedCommands,
        &'static mut ShipPowerSystem,
        &'static mut crate::server_app::ShipSystemBlackboards,
        (
            Option<&'static ShipDamage>,
            Option<&'static EntitySystemHull>,
            Option<&'static crate::modifiers::ShipModifiers>,
        ),
        (
            Option<&'static crate::lockstep::FleetSlotOf>,
            Option<&'static FactionComponent>,
            Option<&'static EntityName>,
        ),
    ),
>;

fn addressed(audience: &CommsAudience, source: &Endpoint, receiver: &Endpoint) -> bool {
    match audience {
        CommsAudience::Contact { target_uuid } => &receiver.uuid == target_uuid,
        // The existing frozen player fleet may contain opposing factions.
        // Its own-fleet audience excludes those opponents; faction traffic also
        // reaches ordinary non-roster actors of the selected faction.
        CommsAudience::Fleet => {
            source.fleet
                && receiver.fleet
                && source.faction.is_some()
                && source.faction == receiver.faction
        }
        CommsAudience::Faction { faction_uuid } => receiver.faction.as_ref() == Some(faction_uuid),
        CommsAudience::All => true,
    }
}

fn thread(channel: &str, audience: &CommsAudience, source: &str) -> String {
    match audience {
        CommsAudience::Contact { target_uuid } => {
            let (a, b) = if source < target_uuid.as_str() {
                (source, target_uuid.as_str())
            } else {
                (target_uuid.as_str(), source)
            };
            format!("radio/{channel}/contact/{a}/{b}")
        }
        CommsAudience::Fleet => format!("radio/{channel}/fleet"),
        CommsAudience::Faction { faction_uuid } => {
            format!("radio/{channel}/faction/{faction_uuid}")
        }
        CommsAudience::All => format!("radio/{channel}/all"),
    }
}

pub(crate) fn copy_message(
    id: String,
    source: &Endpoint,
    recipient: &str,
    body: &str,
    metadata: CommsTransmission,
) -> CommsMessage {
    let mut message = CommsMessage::injected(
        id,
        source.uuid.clone(),
        source.name.clone(),
        body.into(),
        Default::default(),
        vec![],
        thread(&metadata.channel, &metadata.audience, &source.uuid),
        true,
        CommsPriority::Routine,
    );
    use phoenix_model::comms_request::{CommsRadioContent, CommsRequestProgress};
    message.literal_body = metadata.content.is_text();
    if !message.literal_body {
        message.request_progress = Some(CommsRequestProgress::default());
        if matches!(metadata.content, CommsRadioContent::Request { .. })
            && metadata.received_tick.is_some()
        {
            message.responses = [
                match &metadata.content {
                    CommsRadioContent::Request { request } => request.positive_response(),
                    _ => unreachable!("received request"),
                },
                phoenix_model::comms_request::DECLINE_RESPONSE,
            ]
            .into_iter()
            .map(|text| crate::core::messages::CommsResponseView {
                text: text.into(),
                important: false,
                available: true,
            })
            .collect();
        }
    }
    message.is_read = metadata.received_tick.is_none();
    message.recipient_ship = Some(phoenix_model::wire::ShipKey(recipient.into()));
    message.transmission = Some(metadata);
    message
}

/// Runs after actual energy allocation. Listening consumes ordinary continuous
/// work once per radio, irrespective of message count or chosen audience.
pub(crate) fn tick_radio(
    mut sources: Sources,
    mut captured: Query<&mut super::capture::ShipCommsCapture>,
    capture_commands: Query<(
        &crate::core::messages::AdmittedCommands,
        Option<&RadioProfile>,
        Has<super::capture::ShipCommsCapture>,
    )>,
    mut inbox: ResMut<CommsInboxRes>,
    mut runtime: ResMut<CommsRuntime>,
    paths: crate::regions::signal_path::RegionSignalQuery,
    tick: Res<crate::sim_tick::SimTick>,
    fixed: Res<Time<Fixed>>,
    mint: crate::world_id::LiveMint<'_, { crate::world_id::IdNamespace::Message as usize }>,
    mut outbox: Option<ResMut<crate::server_app::SimOutbox>>,
    mut world_runtime: Option<ResMut<crate::world::server::WorldContentRuntime>>,
    base: Option<Res<crate::world::config::WorldConfig>>,
    intents: Query<&super::media_control::ShipCommsMediaIntent>,
    mut media: Option<ResMut<super::media_control::PaidMediaCarriers>>,
    hailable: Query<&crate::comms::CommsHailable>,
) {
    super::capture::refuse_unsupported(&capture_commands, outbox.as_deref_mut());
    let mut ordered: Vec<_> = sources
        .iter()
        .map(|row| (row.1 .0.clone(), row.0))
        .collect();
    ordered.sort_by(|a, b| a.0.cmp(&b.0));
    let mut endpoints = Vec::new();
    for (_, entity) in &ordered {
        let Ok((
            entity,
            uuid,
            profile,
            physics,
            transform,
            config,
            controls,
            _,
            mut power,
            _,
            (damage, hull, modifiers),
            (slot, faction, name),
        )) = sources.get_mut(*entity)
        else {
            continue;
        };
        let available = super::server::information_endpoint_available(config, controls, modifiers)
            && !crate::ship::compartment_damage::operation_disabled(
                damage,
                hull,
                &profile.0.system,
            );
        let operation = damage.map_or(1.0, |damage| {
            damage.factor(
                &profile.0.system,
                phoenix_model::damage::DamageBinding::Operation,
            )
        });
        let listening = if available {
            power
                .0
                .work_continuous(&profile.0.system, fixed.timestep().as_secs_f32(), 1.0)
                .map_or(0.0, |grant| grant.fraction)
        } else {
            0.0
        } * operation.clamp(0.0, 1.0);
        endpoints.push(Endpoint {
            entity,
            uuid: uuid.0.clone(),
            name: name.map_or_else(|| uuid.0.clone(), |name| name.0.clone()),
            point: physics.map_or(transform.translation, |pose| {
                Vec3::new(pose.x, pose.y, pose.z)
            }),
            radio: profile.0.clone(),
            listening,
            fleet: slot.is_some(),
            faction: faction.map(|value| value.0.to_string()),
        });
    }
    own_contacts::publish(&endpoints, &mut sources, &hailable, &paths);
    for source in &endpoints {
        let Ok((_, _, _, _, _, config, _, commands, mut power, boards, _, _)) =
            sources.get_mut(source.entity)
        else {
            continue;
        };
        for command in commands.for_target(source.radio.system.0.as_str()) {
            let (channel, audience, body, content, planned_context) = match &command.payload {
                SystemControlPayload::SendCommsText {
                    channel,
                    audience,
                    body,
                } => (
                    channel,
                    audience,
                    body.as_str(),
                    phoenix_model::comms_request::CommsRadioContent::Text,
                    None,
                ),
                SystemControlPayload::SendCommsRequest {
                    channel,
                    audience,
                    request,
                    planned_request,
                } => (
                    channel,
                    audience,
                    request.body(),
                    phoenix_model::comms_request::CommsRadioContent::Request {
                        request: request.clone(),
                    },
                    planned_request.as_deref(),
                ),
                _ => continue,
            };
            let profile = source
                .radio
                .channels
                .iter()
                .find(|profile| &profile.id == channel);
            let permitted = config
                .0
                .observations_for_system(&source.radio.system, &boards.0);
            let address_known = match audience {
                CommsAudience::Contact { target_uuid } => {
                    matches!(permitted.get(&source.radio.system), Some(SystemBlackboard::Comms(board)) if board.contacts.iter().any(|contact| &contact.uuid == target_uuid))
                        || inbox.0.iter().any(|message| {
                            message.is_for_ship(Some(&source.uuid))
                                && &message.sender_uuid == target_uuid
                        })
                }
                CommsAudience::Fleet => source.fleet && source.faction.is_some(),
                CommsAudience::Faction { faction_uuid } => {
                    uuid::Uuid::parse_str(faction_uuid).is_ok()
                }
                CommsAudience::All => true,
            };
            let context_known = planned_context.is_none_or(|instance| match &content {
                phoenix_model::comms_request::CommsRadioContent::Request { request } => {
                    super::planned::matches(
                        config,
                        &boards,
                        &source.radio.system,
                        instance,
                        audience,
                        channel,
                        request,
                        tick.0,
                    )
                }
                _ => false,
            });
            let prepared=match &content {
                phoenix_model::comms_request::CommsRadioContent::Request{request} if context_known=>
                    crate::world::activities::prepare_communication_request(world_runtime.as_deref_mut(),base.as_deref(),
                        &source.uuid,&source.radio.system,planned_context,audience,channel,request,tick.0),
                _ if planned_context.is_none()=>Ok(None),
                _=>Err("planned request is not present in this Console's current permitted publication".into()),
            };
            let formation = matches!(&content, phoenix_model::comms_request::CommsRadioContent::Request { request } if request.is_formation());
            let formation_sequence = if formation {
                inbox
                    .0
                    .iter()
                    .filter(|message| message.sender_uuid == source.uuid)
                    .filter_map(|message| {
                        message
                            .transmission
                            .as_ref()
                            .filter(|wire| wire.received_tick.is_none())?
                            .formation_sequence
                    })
                    .max()
                    .unwrap_or(0)
                    .checked_add(1)
            } else {
                None
            };
            let request_valid = match &content {
                phoenix_model::comms_request::CommsRadioContent::Request { request } => {
                    request.validate().is_ok()
                        && match request {
                            phoenix_model::comms_request::GenericCommsRequest::MediaCall {
                                mode,
                            } => {
                                matches!(audience, CommsAudience::Contact { .. })
                                    && source
                                        .radio
                                        .media
                                        .is_some_and(|capability| capability.supports(*mode))
                            }
                            phoenix_model::comms_request::GenericCommsRequest::ReceiveCraft { craft } => {
                                matches!(audience, CommsAudience::Contact { .. })
                                    && boards.0.keys().filter_map(|source|permitted.get(source)).any(|board| matches!(board,
                                        SystemBlackboard::Bay(bay) if bay.stored.iter().any(|row| &row.id == craft)))
                            }
                            _ => true,
                        }
                        && (!formation
                            || formation_sequence.is_some()
                                && matches!(audience, CommsAudience::Contact { .. }))
                }
                _ => true,
            };
            let context_valid = prepared.is_ok();
            let prepared = prepared.ok().flatten();
            let sustained = prepared.is_some()
                || formation && matches!(audience, CommsAudience::Contact { .. });
            let applied = request_valid
                && context_known
                && context_valid
                && profile.is_some()
                && source.listening > 0.0
                && address_known
                && audience.is_valid()
                && (!content.is_text() || phoenix_model::comms_channel::valid_text(body))
                && power
                    .0
                    .try_work_discrete_count(
                        &source.radio.system,
                        profile.unwrap().work_units as usize,
                    )
                    .unwrap_or(false);
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
            if !applied {
                continue;
            }
            let new_id = || {
                crate::world_id::mint_live_id_with(
                    mint.as_deref(),
                    crate::world_id::IdNamespace::Message,
                )
            };
            let metadata = CommsTransmission {
                formation_sequence,
                sustained,
                origin_id: new_id(),
                channel: channel.clone(),
                audience: audience.clone(),
                emitted_tick: tick.0,
                received_tick: None,
                content,
            };
            inbox.0.inject(copy_message(
                new_id(),
                source,
                &source.uuid,
                body,
                metadata.clone(),
            ));
            if let Some(prepared) = prepared {
                crate::world::activities::bind_communication_request(
                    prepared,
                    &metadata.origin_id,
                    tick.0,
                );
                runtime
                    .pending_ai_responses
                    .remove(&super::server::PendingAiResponseKey::planned(
                        &source.uuid,
                        planned_context.expect("validated prepared context"),
                    ));
            }
            for receiver in &endpoints {
                if receiver.uuid == source.uuid
                    || receiver.listening <= 0.0
                    || !addressed(audience, source, receiver)
                {
                    continue;
                }
                if !route_available(&endpoints, source, receiver, channel, &paths) {
                    continue;
                }
                let mut heard = metadata.clone();
                heard.received_tick = Some(tick.0);
                // Conditional reception cannot advance a globally visible mint:
                // the next source emission must not encode hidden receiver count.
                let copy_id = format!("radio-copy/{}/{}", metadata.origin_id, receiver.uuid);
                inbox
                    .0
                    .inject(copy_message(copy_id, source, &receiver.uuid, body, heard));
            }
            runtime.needs_broadcast = true;
        }
    }
    request_delivery::settle(&endpoints, &mut inbox, &mut runtime, &paths, tick.0);
    // Same radio work/frontier, retained canonical inbox and endpoint pass.
    // Structured outcomes are a sustained service; text above remains one-shot.
    super::request_returns::settle(
        &endpoints,
        &mut sources,
        &mut inbox,
        &mut runtime,
        &paths,
        tick.0,
        || {
            crate::world_id::mint_live_id_with(
                mint.as_deref(),
                crate::world_id::IdNamespace::Message,
            )
        },
    );
    super::capture::settle(
        &endpoints,
        &mut sources,
        &mut captured,
        &inbox,
        &mut runtime,
        &paths,
        tick.0,
        fixed.timestep().as_secs_f32(),
        outbox.as_deref_mut(),
    );
    if let Some(cache) = media.as_deref_mut() {
        // Read the SAME endpoints whose ordinary carrier work was paid above.
        // No frame callback, extra payment, route publication or saved cache.
        super::media_control::derive_paid_carriers(
            cache, &endpoints, &intents, &inbox, &paths, tick.0,
        );
    }
}

#[path = "request_delivery.rs"]
mod request_delivery;

#[path = "radio_contacts.rs"]
mod own_contacts;

#[path = "radio_route.rs"]
mod route;
pub(super) use route::available as route_available;

pub(crate) fn register(app: &mut App) {
    use crate::authoritative::{DeclareState, StateClass};
    use crate::sim_sets::{FixedStep, SimSet};
    app.declare_state::<RadioProfile>(StateClass::Derived, "comms-range-state")
        .add_systems(
            FixedUpdate,
            tick_radio
                .in_set(SimSet::Physics)
                .in_set(FixedStep::TickCommsRadio)
                .after(FixedStep::TickPowerSystem),
        );
}

#[cfg(test)]
#[path = "radio_tests.rs"]
mod tests;
