//! Mechanical sustained return delivery inside the sole actual radio tick.
//! No receiver viability result may change local payment, emission or decisions.
use super::{
    radio::{copy_message, Endpoint, Sources},
    server::{CommsInboxRes, CommsRuntime},
};
use phoenix_model::{
    comms_channel::{CommsAudience, CommsTransmission},
    comms_request::{CommsRadioContent, CommsRequestOutcome, CommsRequestReference, OUTCOME_BODY},
};

#[allow(clippy::too_many_arguments)]
pub(super) fn settle(
    endpoints: &[Endpoint],
    sources: &mut Sources<'_, '_>,
    inbox: &mut CommsInboxRes,
    runtime: &mut CommsRuntime,
    paths: &crate::regions::signal_path::RegionSignalQuery<'_, '_>,
    now: u64,
    new_id: impl Fn() -> String,
) {
    // Own received copies and events determine local transmission. Private
    // delivered frontiers and remote endpoints are deliberately absent here.
    let requests: Vec<_> = inbox
        .0
        .iter()
        .filter(|message| {
            super::requests::is_request(message)
                && message
                    .transmission
                    .as_ref()
                    .is_some_and(|wire| wire.received_tick.is_some())
        })
        .cloned()
        .collect();
    for request in requests {
        let (Some(wire), Some(progress), Some(recipient)) = (
            &request.transmission,
            &request.request_progress,
            &request.recipient_ship,
        ) else {
            continue;
        };
        let Some(source) = endpoints
            .iter()
            .find(|endpoint| endpoint.uuid == recipient.0)
        else {
            continue;
        };
        let CommsRadioContent::Request { request: kind } = wire.content else {
            continue;
        };
        let stage = progress.facts.stage();
        let already_sent = inbox.0.iter().any(|message| {
            message.transmission.as_ref().is_some_and(|sent| {
                sent.received_tick.is_none()
                    && matches!(&sent.content,
                CommsRadioContent::Outcome {outcome} if outcome.request.origin_id == wire.origin_id
                && outcome.request.recipient_ship == source.uuid && outcome.facts.stage() == stage)
            })
        });
        if already_sent || source.listening <= 0.0 {
            continue;
        }
        let Some(channel) = source
            .radio
            .channels
            .iter()
            .find(|channel| channel.id == wire.channel)
        else {
            continue;
        };
        let Ok((_, _, _, _, _, _, _, _, mut power, _, _, _)) = sources.get_mut(source.entity)
        else {
            continue;
        };
        if !power
            .0
            .try_work_discrete_count(&source.radio.system, channel.work_units as usize)
            .unwrap_or(false)
        {
            continue;
        }
        let outcome = CommsRequestOutcome {
            request: CommsRequestReference {
                origin_id: wire.origin_id.clone(),
                source_ship: request.sender_uuid.clone(),
                recipient_ship: source.uuid.clone(),
                emitted_tick: wire.emitted_tick,
                heard_tick: wire.received_tick.expect("heard request"),
                request: kind,
            },
            facts: progress.facts.clone(),
        };
        let metadata = CommsTransmission {
            origin_id: new_id(),
            channel: wire.channel.clone(),
            audience: CommsAudience::Contact {
                target_uuid: request.sender_uuid.clone(),
            },
            emitted_tick: now,
            received_tick: None,
            content: CommsRadioContent::Outcome { outcome },
        };
        inbox.0.inject(copy_message(
            new_id(),
            source,
            &source.uuid,
            OUTCOME_BODY,
            metadata,
        ));
        runtime.needs_broadcast = true;
    }
    // All admitted transmissions remain on their own existing sent copy. Later
    // delivery uses continuously paid operation, without another discrete cost.
    let returns: Vec<_> = inbox
        .0
        .iter()
        .filter(|message| {
            message.transmission.as_ref().is_some_and(|wire| {
                wire.received_tick.is_none()
                    && matches!(wire.content, CommsRadioContent::Outcome { .. })
            })
        })
        .cloned()
        .collect();
    for sent in returns {
        let wire = sent.transmission.as_ref().expect("outgoing outcome");
        let CommsRadioContent::Outcome { outcome } = &wire.content else {
            continue;
        };
        let Some(source) = endpoints
            .iter()
            .find(|endpoint| endpoint.uuid == sent.sender_uuid && endpoint.listening > 0.0)
        else {
            continue;
        };
        let Some(receiver) = endpoints.iter().find(|endpoint| {
            endpoint.uuid == outcome.request.source_ship && endpoint.listening > 0.0
        }) else {
            continue;
        };
        let copy_id = format!("radio-copy/{}/{}", wire.origin_id, receiver.uuid);
        if inbox.0.iter().any(|message| message.id == copy_id) {
            continue;
        }
        if !super::radio::route_available(endpoints, source, receiver, &wire.channel, paths) {
            continue;
        }
        let mut heard = wire.clone();
        heard.received_tick = Some(now);
        inbox.0.inject(copy_message(
            copy_id,
            source,
            &receiver.uuid,
            OUTCOME_BODY,
            heard,
        ));
        // Internal idempotence frontier never enters either Console's view.
        let original = requests_for_return(inbox, outcome);
        if let Some(id) = original {
            if let Some(request) = inbox.0.request_copy_mut(&id, &source.uuid) {
                let progress = request
                    .request_progress
                    .as_mut()
                    .expect("structured request");
                progress.last_returned = Some(
                    progress
                        .last_returned
                        .map_or(outcome.facts.stage(), |stage| {
                            stage.max(outcome.facts.stage())
                        }),
                );
            }
        }
        runtime.needs_broadcast = true;
    }
}

fn requests_for_return(inbox: &CommsInboxRes, outcome: &CommsRequestOutcome) -> Option<String> {
    inbox
        .0
        .iter()
        .find(|message| {
            message.sender_uuid == outcome.request.source_ship
                && message
                    .recipient_ship
                    .as_ref()
                    .is_some_and(|recipient| recipient.0 == outcome.request.recipient_ship)
                && message.transmission.as_ref().is_some_and(|wire| {
                    wire.origin_id == outcome.request.origin_id
                        && wire.received_tick == Some(outcome.request.heard_tick)
                })
        })
        .map(|message| message.id.clone())
}
