//! Structured requests reuse the Comms inbox, response AI and actual radio.
//! This module owns local interpretation seams and mechanical reading only;
//! it never interprets text or grants another System machinery authority.
use super::server::{CommsInboxRes, CommsRuntime};
use crate::{
    core::messages::{AdmittedCommands, CommsMessage, SystemBlackboard, SystemControlPayload},
    entities::spawner::{EntitySystemHull, EntityUuid},
    server_app::ShipSystemBlackboards,
    ship::{compartment_damage::ShipDamage, power::ShipPowerSystem},
    ship_plugin::{ShipConfigComponent, ShipSystemControlSources},
};
use bevy::prelude::*;
use phoenix_model::comms_request::{
    CommsRadioContent, CommsRequestDecision, CommsStatusReport, GenericCommsRequest,
};

pub(crate) fn is_request(message: &CommsMessage) -> bool {
    message.transmission.as_ref().is_some_and(|transmission| {
        matches!(transmission.content, CommsRadioContent::Request { .. })
    })
}

/// Same actual Console read view for a human operator and its System AI. The
/// inbox supplies local progress only after the original content is permitted.
pub(crate) fn permitted(
    config: &ShipConfigComponent,
    boards: &ShipSystemBlackboards,
    message: &CommsMessage,
) -> bool {
    let owner = crate::ship::system_registry::comms_system_id();
    let view = config.0.observations_for_system(&owner, &boards.0);
    matches!(view.get(&owner), Some(SystemBlackboard::Comms(board)) if board.messages.iter().any(|held|
        held.id == message.id && held.sender_uuid == message.sender_uuid
            && held.recipient_ship == message.recipient_ship && held.transmission == message.transmission))
}

/// Internal reliable-delivery bookkeeping is never an operator observation.
pub(crate) fn redact_private_progress(message: &mut CommsMessage) {
    if let Some(progress) = &mut message.request_progress {
        progress.last_returned = None;
    }
}

/// Facts from the heard request and own Console only. Remote reachability,
/// actor survival and unbound alert state remain absent (unknown), not zero.
pub(crate) fn response_facts(
    message: &CommsMessage,
    available: bool,
    power_rating: Option<f32>,
) -> crate::world::flags::AiFacts {
    use crate::entities::ai_flag_hosts as fid;
    let mut facts = crate::world::flags::AiFacts::new();
    if let Some(CommsRadioContent::Request { request }) =
        message.transmission.as_ref().map(|wire| &wire.content)
    {
        facts.set_fact(
            match request {
                GenericCommsRequest::FormUp { .. } => fid::REQUEST_FORM_UP,
                GenericCommsRequest::MediaCall { .. } => fid::REQUEST_MEDIA_CALL,
                GenericCommsRequest::ReportStatus => fid::REQUEST_REPORT_STATUS,
                GenericCommsRequest::ReceiveCraft { .. } => fid::REQUEST_RECEIVE_CRAFT,
            },
            1.0,
        );
    }
    facts.set_fact(fid::RESPONSE_COUNT, 2.0);
    facts.set_fact(fid::AVAILABLE_RESPONSE_COUNT, 2.0);
    facts.set_fact(fid::IMPORTANT_RESPONSE_COUNT, 0.0);
    facts.set_fact(
        fid::IS_URGENT,
        if message.effective_priority().is_urgent() {
            1.0
        } else {
            0.0
        },
    );
    facts.set_fact(fid::IS_READ, if message.is_read { 1.0 } else { 0.0 });
    facts.set_fact(fid::COMMS_AVAILABLE, if available { 1.0 } else { 0.0 });
    if let Some(rating) = power_rating {
        facts.set_fact(fid::POWER_RATING, rating as f64);
    }
    facts
}

/// Explicit local read, never a delivery receipt. Outcome reading changes only
/// that outcome's own inbox flag, not the request events it reports.
pub(crate) fn mark_read(inbox: &mut CommsInboxRes, ship: &str, message_id: &str, now: u64) -> bool {
    let Some(message) = inbox
        .0
        .iter()
        .find(|message| {
            message.id == message_id
                && message
                    .recipient_ship
                    .as_ref()
                    .is_some_and(|recipient| recipient.0 == ship)
        })
        .cloned()
    else {
        return false;
    };
    let Some(transmission) = &message.transmission else {
        return false;
    };
    let Some(heard) = transmission.received_tick else {
        return false;
    };
    let mut next = message.clone();
    let Some(progress) = &mut next.request_progress else {
        return false;
    };
    if is_request(&message) && progress.mark_read(heard, now).is_err() {
        return false;
    }
    if !is_request(&message) && !matches!(transmission.content, CommsRadioContent::Outcome { .. }) {
        return false;
    }
    next.is_read = true;
    if next != message {
        *inbox
            .0
            .request_copy_mut(message_id, ship)
            .expect("same checked recipient copy") = next;
    }
    true
}

/// The existing admitted RespondToMessage handler calls this instead of its
/// plot callback path. Decisions are local even if no reverse link exists.
#[allow(clippy::too_many_arguments)]
pub(crate) fn respond(
    inbox: &mut CommsInboxRes,
    ship: &str,
    message_id: &str,
    index: usize,
    now: u64,
    config: &ShipConfigComponent,
    boards: &ShipSystemBlackboards,
    sources: &ShipSystemControlSources,
    damage: Option<&ShipDamage>,
    power: Option<&ShipPowerSystem>,
    radio: Option<&super::radio::RadioProfile>,
) -> bool {
    let Some(message) = inbox
        .0
        .iter()
        .find(|message| {
            message.id == message_id
                && message
                    .recipient_ship
                    .as_ref()
                    .is_some_and(|recipient| recipient.0 == ship)
        })
        .cloned()
    else {
        return false;
    };
    if !is_request(&message) || !permitted(config, boards, &message) {
        return false;
    }
    let Some(heard) = message
        .transmission
        .as_ref()
        .and_then(|transmission| transmission.received_tick)
    else {
        return false;
    };
    let wire = message.transmission.as_ref().expect("actual heard request");
    let CommsRadioContent::Request { request } = &wire.content else {
        return false;
    };
    if index == 0
        && request.is_formation()
        && inbox.0.iter().any(|newer| {
            newer.sender_uuid == message.sender_uuid
                && newer.recipient_ship == message.recipient_ship
                && newer
                    .request_progress
                    .as_ref()
                    .and_then(|progress| progress.facts.decision.as_ref())
                    .is_some_and(|decision| decision.choice == CommsRequestDecision::Accept)
                && newer.transmission.as_ref().is_some_and(|newer| {
                    newer.received_tick.is_some()
                        && newer.formation_sequence > wire.formation_sequence
                })
        })
    {
        return false;
    }
    let choice = match index {
        0 if request.is_formation() || request.is_media() || request.is_reception() => {
            CommsRequestDecision::Accept
        }
        0 => CommsRequestDecision::Report,
        1 => CommsRequestDecision::Decline,
        _ => return false,
    };
    let owner = crate::ship::system_registry::comms_system_id();
    let report = (choice == CommsRequestDecision::Report).then(|| CommsStatusReport {
        system: owner.clone(),
        available: radio.is_some_and(|profile| profile.0.system == owner)
            && sources.0.entries().any(|(id, _)| id == &owner)
            && !sources.0.is_offline(&owner)
            && sources.0.source_for(&owner) != crate::ship::control_source::ControlSource::Offline
            && damage.is_none_or(|damage| {
                damage.factor(&owner, phoenix_model::damage::DamageBinding::Operation) > 0.0
            })
            && power.is_some_and(|power| {
                power
                    .0
                    .network()
                    .consumers
                    .get(&owner)
                    .is_some_and(|state| state.energy > 0.0 && !state.thermal_stopped)
            }),
        observed_tick: now,
    });
    let mut next = message;
    let Some(progress) = &mut next.request_progress else {
        return false;
    };
    if progress.decide(heard, now, choice, report).is_err() {
        return false;
    }
    next.is_read = true;
    next.selected_response = Some(index);
    *inbox
        .0
        .request_copy_mut(message_id, ship)
        .expect("same checked recipient copy") = next;
    true
}

#[allow(clippy::type_complexity)]
pub(crate) fn handle_read_commands(
    ships: Query<(
        &EntityUuid,
        &AdmittedCommands,
        &ShipConfigComponent,
        &ShipSystemBlackboards,
    )>,
    mut inbox: ResMut<CommsInboxRes>,
    tick: Res<crate::sim_tick::SimTick>,
    mut runtime: ResMut<CommsRuntime>,
    mut outbox: Option<ResMut<crate::server_app::SimOutbox>>,
) {
    for (uuid, commands, config, boards) in &ships {
        for command in commands.for_target(crate::ship::system_registry::COMMS_SYSTEM_ID) {
            let SystemControlPayload::MarkCommsRead { message_id } = &command.payload else {
                continue;
            };
            let known = inbox.0.iter().any(|message| {
                message.id == *message_id
                    && message.is_for_ship(Some(&uuid.0))
                    && permitted(config, boards, message)
            });
            let applied = known && mark_read(&mut inbox, &uuid.0, message_id, tick.0);
            runtime.needs_broadcast |= applied;
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

/// Mechanical progress on the SAME existing pending response entry. It makes
/// no choice, emits no response and never counts a Human/Offline interval.
#[allow(clippy::type_complexity)]
pub(crate) fn advance_reading(
    ships: Query<(
        &EntityUuid,
        &super::radio::RadioProfile,
        &ShipConfigComponent,
        &ShipSystemControlSources,
        &ShipSystemBlackboards,
        Option<&EntitySystemHull>,
        Option<&ShipDamage>,
    )>,
    mut inbox: ResMut<CommsInboxRes>,
    mut runtime: ResMut<CommsRuntime>,
    tick: Res<crate::sim_tick::SimTick>,
    content: Option<Res<crate::world::server::WorldContentRuntime>>,
) {
    let operators: std::collections::BTreeMap<_, _> =
        ships.iter().map(|row| (row.0 .0.as_str(), row)).collect();
    let keys: Vec<_> = runtime
        .pending_ai_responses
        .iter()
        .filter(|(_, record)| record.request_reading.is_some())
        .map(|(key, _)| key.clone())
        .collect();
    for key in keys {
        let Some((_, profile, config, sources, boards, hull, damage)) =
            operators.get(key.ship_uuid.as_str()).copied()
        else {
            runtime.pending_ai_responses.remove(&key);
            continue;
        };
        if key.planned_request {
            let active = content.as_deref().and_then(|runtime| {
                crate::world::activities::communication_request_release(
                    runtime,
                    &key.ship_uuid,
                    &profile.0.system,
                    &key.message_id,
                    tick.0,
                )
            });
            if active.is_none() {
                runtime.pending_ai_responses.remove(&key);
                continue;
            }
            let known = super::planned::rows(config, boards, &profile.0.system, tick.0)
                .iter()
                .any(|row| row.id == key.message_id);
            let eligible = known
                && sources.0.entries().any(|(id, _)| id == &profile.0.system)
                && crate::ai::host::ai_operates(&sources.0, &profile.0.system)
                && !crate::ship::compartment_damage::operation_disabled(
                    damage,
                    hull,
                    &profile.0.system,
                );
            let record = runtime
                .pending_ai_responses
                .get_mut(&key)
                .expect("same planned reading");
            if let Some(reading) = record.request_reading.as_mut() {
                if reading
                    .validate(profile.0.request_reading_ticks, tick.0)
                    .is_ok()
                    && reading.advance(tick.0, eligible).is_ok()
                {
                    record.due_tick = reading
                        .due_tick()
                        .expect("checked planned reading frontier");
                }
            }
            continue;
        }
        let Some(message) = inbox
            .0
            .iter()
            .find(|message| {
                message.id == key.message_id
                    && message
                        .recipient_ship
                        .as_ref()
                        .is_some_and(|recipient| recipient.0 == key.ship_uuid)
            })
            .cloned()
        else {
            runtime.pending_ai_responses.remove(&key);
            continue;
        };
        if !is_request(&message) || message.is_read || message.selected_response.is_some() {
            runtime.pending_ai_responses.remove(&key);
            continue;
        }
        let eligible = permitted(config, boards, &message)
            && sources.0.entries().any(|(id, _)| id == &profile.0.system)
            && crate::ai::host::ai_operates(&sources.0, &profile.0.system)
            && !crate::ship::compartment_damage::operation_disabled(
                damage,
                hull,
                &profile.0.system,
            );
        let record = runtime
            .pending_ai_responses
            .get_mut(&key)
            .expect("same pending entry");
        let reading = record
            .request_reading
            .as_mut()
            .expect("selected request entry");
        if reading
            .validate(profile.0.request_reading_ticks, tick.0)
            .is_err()
        {
            continue;
        }
        match reading.advance(tick.0, eligible) {
            Ok(true) => {
                if mark_read(&mut inbox, &key.ship_uuid, &key.message_id, tick.0) {
                    runtime.pending_ai_responses.remove(&key);
                    runtime.needs_broadcast = true;
                }
            }
            Ok(false) => {
                record.due_tick = reading.due_tick().expect("checked reading clock");
            }
            Err(_) => {}
        }
    }
}

pub(crate) fn register(app: &mut App) {
    use crate::sim_sets::{FixedStep, SimSet};
    app.add_systems(
        FixedUpdate,
        (
            handle_read_commands
                .in_set(SimSet::Input)
                .in_set(FixedStep::HandleCommsRead)
                .before(advance_reading),
            advance_reading
                .in_set(SimSet::Input)
                .in_set(FixedStep::TickCommsRequestReading)
                .after(FixedStep::HandleHail)
                .before(FixedStep::OperateCommsResponseAi),
        ),
    );
}
