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
        commands.insert(RadioProfile(radio.clone()));
    }
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
        &'static crate::server_app::ShipSystemBlackboards,
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
                phoenix_model::comms_request::REPORT_RESPONSE,
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
    mut inbox: ResMut<CommsInboxRes>,
    mut runtime: ResMut<CommsRuntime>,
    paths: crate::regions::signal_path::RegionSignalQuery,
    tick: Res<crate::sim_tick::SimTick>,
    fixed: Res<Time<Fixed>>,
    mint: crate::world_id::LiveMint<'_, { crate::world_id::IdNamespace::Message as usize }>,
    mut outbox: Option<ResMut<crate::server_app::SimOutbox>>,
) {
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
    for source in &endpoints {
        let Ok((_, _, _, _, _, config, _, commands, mut power, boards, _, _)) =
            sources.get_mut(source.entity)
        else {
            continue;
        };
        for command in commands.for_target(source.radio.system.0.as_str()) {
            let (channel, audience, body, content) = match &command.payload {
                SystemControlPayload::SendCommsText {
                    channel,
                    audience,
                    body,
                } => (
                    channel,
                    audience,
                    body.as_str(),
                    phoenix_model::comms_request::CommsRadioContent::Text,
                ),
                SystemControlPayload::SendCommsRequest {
                    channel,
                    audience,
                    request,
                } => (
                    channel,
                    audience,
                    phoenix_model::comms_request::REPORT_STATUS_BODY,
                    phoenix_model::comms_request::CommsRadioContent::Request { request: *request },
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
            let applied = profile.is_some()
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
}

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
