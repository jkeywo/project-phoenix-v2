//! Actual intercepted evidence on the existing paid radio pass. Nothing here
//! injects an addressed inbox copy, responds to a request or publishes plaintext
//! before the canonical decoder frontier completes.
use super::radio::{Endpoint, Sources};
use super::server::{CommsInboxRes, CommsRuntime};
use crate::core::messages::{SystemBlackboard, SystemControlPayload};
use bevy::prelude::*;
use phoenix_sim_gameplay::electronic::capture::{CaptureState, CapturedPayload};

#[derive(Component, Clone, Debug, PartialEq, Eq)]
pub struct ShipCommsCapture(pub CaptureState);

pub(crate) fn install(
    config: &crate::entities::config::EntityConfig,
    commands: &mut EntityCommands,
) {
    if let Some(radio) = config
        .comms
        .as_ref()
        .and_then(|row| row.radio.as_ref())
        .filter(|radio| radio.capture.is_some())
    {
        commands.insert(ShipCommsCapture(CaptureState::new(radio.system.clone())));
    }
}

fn direct_measurement(
    source: &Endpoint,
    receiver: &Endpoint,
    channel: &str,
    paths: &crate::regions::signal_path::RegionSignalQuery<'_, '_>,
) -> bool {
    let Some(transmitting) = source.radio.channels.iter().find(|row| row.id == channel) else {
        return false;
    };
    let Some(receiving) = receiver.radio.channels.iter().find(|row| row.id == channel) else {
        return false;
    };
    source.uuid != receiver.uuid
        && source.listening > 0.0
        && receiver.listening > 0.0
        && super::server::signal_comms_in_range(
            paths,
            super::server::SignalEndpoint {
                point: source.point,
                range: transmitting.range * source.listening,
                receiver: Some((&source.uuid, &source.radio.system)),
            },
            super::server::SignalEndpoint {
                point: receiver.point,
                range: receiving.range * receiver.listening,
                receiver: Some((&receiver.uuid, &receiver.radio.system)),
            },
        )
}

/// Same explicit own Comms publication for the Human and its ordinary AI. A
/// selected handle is information only and cannot identify a source actor.
pub(crate) fn views(
    state: &ShipCommsCapture,
    radio: &phoenix_model::comms_channel::CommsRadioConfig,
) -> Option<phoenix_model::comms_capture::CommsCaptureControlsView> {
    let config = radio.capture.as_ref()?;
    (state.0.owner() == &radio.system).then(|| {
        phoenix_model::comms_capture::CommsCaptureControlsView {
            rows: state.0.views(config, &radio.keys),
            controllable: false,
        }
    })
}

/// Called after normal emissions/returned service, before the final media paid
/// carrier derivation. The source list is the same actual paid endpoint pass.
/// Only original emissions stamped by actual paid source-copy commit count.
pub(crate) fn settle(
    endpoints: &[Endpoint],
    sources: &mut Sources<'_, '_>,
    captured: &mut Query<&mut ShipCommsCapture>,
    inbox: &CommsInboxRes,
    runtime: &mut CommsRuntime,
    paths: &crate::regions::signal_path::RegionSignalQuery<'_, '_>,
    now: u64,
    dt: f32,
    mut outbox: Option<&mut crate::server_app::SimOutbox>,
) {
    let emissions: Vec<_> = inbox
        .0
        .iter()
        .filter(|message| {
            message
                .transmission
                .as_ref()
                .is_some_and(|wire| wire.received_tick.is_none() && wire.emitted_tick == now)
        })
        .collect();
    for receiver in endpoints {
        let Some(decoder) = &receiver.radio.capture else {
            continue;
        };
        let Ok(mut captured) = captured.get_mut(receiver.entity) else {
            continue;
        };
        if captured.0.owner() != &receiver.radio.system {
            continue;
        }
        let Ok((_, _, _, _, _, config, _, admitted, mut power, boards, (damage, _, _), _)) =
            sources.get_mut(receiver.entity)
        else {
            continue;
        };
        let permitted = config
            .0
            .observations_for_system(&receiver.radio.system, &boards.0);
        let board = match permitted.get(&receiver.radio.system) {
            Some(SystemBlackboard::Comms(board)) => Some(board),
            _ => None,
        };
        for command in admitted.for_target(&receiver.radio.system.0) {
            let SystemControlPayload::SelectCommsCapture { capture } = &command.payload else {
                continue;
            };
            let visible = board.is_some_and(|board| {
                board.captured.as_ref().is_some_and(|view| {
                    capture.is_none_or(|capture| {
                        view.rows.iter().any(|row| {
                            row.capture == capture && row.feasible && row.evidence.is_none()
                        })
                    })
                })
            });
            let applied = visible && captured.0.select(*capture, decoder, &receiver.radio.keys);
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
            runtime.needs_broadcast |= applied;
        }
        if receiver.listening <= 0.0 {
            continue;
        }
        // Only original actual source records committed THIS emission tick are
        // eligible. Archive/carrier availability never implies retransmission;
        // final-recipient copy/delivery state is deliberately absent here.
        for message in &emissions {
            let wire = message
                .transmission
                .as_ref()
                .expect("selected current source emission");
            let Some(source) = endpoints
                .iter()
                .find(|source| source.uuid == message.sender_uuid)
            else {
                continue;
            };
            if !message
                .recipient_ship
                .as_ref()
                .is_some_and(|ship| ship.0 == source.uuid)
                || !direct_measurement(source, receiver, &wire.channel, paths)
            {
                continue;
            }
            let Some(channel) = source
                .radio
                .channels
                .iter()
                .find(|row| row.id == wire.channel)
            else {
                continue;
            };
            runtime.needs_broadcast |= captured.0.observe(
                CapturedPayload {
                    source: source.uuid.clone(),
                    name: message.sender_name.clone(),
                    body: message.body.clone(),
                    literal_body: message.literal_body,
                    transmission: wire.clone(),
                    level: if channel.key.is_some() {
                        channel.protection_level
                    } else {
                        0
                    },
                    key: channel.key.clone(),
                },
                now,
            );
        }
        if captured.0.selected().is_some_and(|sequence| {
            captured
                .0
                .required_work(sequence, decoder, &receiver.radio.keys)
                .is_some()
        }) {
            let operation = damage
                .map_or(1.0, |damage| {
                    damage.factor(
                        &receiver.radio.system,
                        phoenix_model::damage::DamageBinding::Operation,
                    )
                })
                .clamp(0.0, 1.0);
            if let Some(grant) = power.0.work_continuous(&receiver.radio.system, dt, 1.0) {
                runtime.needs_broadcast |= captured.0.advance(
                    now,
                    grant.fraction * operation,
                    decoder,
                    &receiver.radio.keys,
                );
            }
        }
    }
}

#[cfg(test)]
#[path = "capture_tests.rs"]
mod tests;
