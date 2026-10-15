//! Candidate-only communication lineage checks; historical evidence is not a
//! new remote-state census or a request dispatcher.
use super::*;
use phoenix_model::{activity::*, comms_channel::CommsAudience, comms_request::CommsRadioContent};
use phoenix_sim_world::world::activities::{ActivityRequest, SituationConfig, SituationInstance};
#[allow(clippy::too_many_arguments)]
pub(super) fn validate_instance(
    world: &World,
    snapshot: &PhoenixSnapshot,
    instance: &SituationInstance,
    authored: &SituationConfig,
    source: &crate::entities::config::EntityConfig,
    pending: &[ActivityFact],
    id: &str,
) -> Result<(), String> {
    let ActivityRequest::Communication {
        owner,
        channel,
        request,
        ..
    } = &instance.request
    else {
        return Err("communication validation requires its actual request kind".into());
    };
    let ActivityRequest::Communication {
        recipient: declared,
        ..
    } = &authored.request
    else {
        return Err("communication history has another immutable request kind".into());
    };
    let base = world
        .get_resource::<crate::world::config::WorldConfig>()
        .ok_or("communication history lacks its accepted base World")?;
    if declared.starts_with("slot:") {
        return Err("first communication recipient requires accepted base-world identity".into());
    }
    base.validate_situation_actor(declared)?;
    let ship = source
        .ship_config
        .as_ref()
        .ok_or("communication activity lacks accepted ship equipment")?;
    let radio = source
        .comms
        .as_ref()
        .and_then(|row| row.radio.as_ref())
        .ok_or("communication activity lacks actual radio")?;
    if !crate::jobs::activity_bindings::communication_supported(owner, channel, ship, radio) {
        return Err(
            "communication activity disagrees with its actual issuing radio/channel".into(),
        );
    }
    let target = super::activities::accepted_recipe(world, snapshot, declared)?;
    let receiver = target
        .comms
        .as_ref()
        .and_then(|row| row.radio.as_ref())
        .ok_or("communication recipient lacks its accepted radio")?;
    let Some(scope) = &instance.accepted else {
        return Ok(());
    };
    let ActivityExecutionIdentity::Communication {
        origin_id,
        recipient,
        emitted_tick,
    } = &scope.identity
    else {
        return Err("communication activity has a fabricated physical JobId".into());
    };
    let expected_recipient = base
        .name_to_uuid
        .get(declared)
        .ok_or("accepted communication recipient has no immutable base identity")?;
    if recipient != expected_recipient || instance.accepted_tick != Some(*emitted_tick) {
        return Err("communication emission disagrees with accepted recipient/clock".into());
    }
    let terminal = instance.settled.is_some()
        || pending.iter().any(|fact| {
            fact.instance == id
                && matches!(fact.outcome, ActivityOutcome::CommunicationReturned { .. })
        });
    let messages = &snapshot
        .comms
        .as_ref()
        .ok_or("accepted communication lost its Comms owner")?
        .inbox;
    let source_message = messages.iter().find(|message| {
        message.sender_uuid == scope.actor
            && message
                .recipient_ship
                .as_ref()
                .is_some_and(|id| id.0 == scope.actor)
            && message
                .transmission
                .as_ref()
                .is_some_and(|wire| wire.origin_id == *origin_id && wire.received_tick.is_none())
    });
    if let Some(message) = source_message {
        let wire = message.transmission.as_ref().expect("matched wire");
        if !wire.sustained
            || wire.emitted_tick != *emitted_tick
            || wire.channel != *channel
            || !matches!(&wire.audience, CommsAudience::Contact { target_uuid } if target_uuid == recipient)
            || !matches!(&wire.content, CommsRadioContent::Request { request: actual } if actual == request)
        {
            return Err(
                "accepted communication lacks its exact paid source Request lineage".into(),
            );
        }
    } else if !terminal {
        return Err("unsettled communication lost its actual sent Request".into());
    }
    for fact in instance
        .achieved
        .iter()
        .chain(instance.settled.iter())
        .chain(pending.iter().filter(|fact| fact.instance == id))
    {
        let (receipt, report) = match &fact.outcome {
            ActivityOutcome::CommunicationRead {
                receipt_origin_id, ..
            } => (receipt_origin_id, None),
            ActivityOutcome::CommunicationReturned {
                receipt_origin_id,
                outcome,
            } => (receipt_origin_id, outcome.facts.report.as_ref()),
            _ => return Err("communication activity has physical evidence".into()),
        };
        if report.is_some_and(|report| report.system != receiver.system) {
            return Err("communication report names foreign receiving equipment".into());
        }
        let actual = messages.iter().find(|message| {
            message.sender_uuid == *recipient
                && message
                    .recipient_ship
                    .as_ref()
                    .is_some_and(|id| id.0 == scope.actor)
                && message
                    .transmission
                    .as_ref()
                    .is_some_and(|wire| wire.origin_id == *receipt && wire.received_tick.is_some())
        });
        if let Some(message) = actual {
            let wire = message.transmission.as_ref().expect("matched receipt");
            let CommsRadioContent::Outcome { outcome } = &wire.content else {
                return Err("communication evidence receipt has different content".into());
            };
            let matches = match &fact.outcome {
                ActivityOutcome::CommunicationRead {
                    reference,
                    read_tick,
                    ..
                } => reference == &outcome.request && Some(*read_tick) == outcome.facts.read_tick,
                ActivityOutcome::CommunicationReturned {
                    outcome: expected, ..
                } => expected == outcome,
                _ => false,
            };
            if !matches || wire.received_tick != Some(fact.tick) {
                return Err("communication evidence rewrites actual heard returned lineage".into());
            }
        } else if !terminal {
            return Err(
                "unsettled communication evidence lacks its actual own heard receipt".into(),
            );
        }
    }
    Ok(())
}

/// Every retained request/copy must belong to its actual one World execution.
/// This runs even when no ordinary activity owner was otherwise installed.
pub(super) fn validate_emissions(snapshot: &PhoenixSnapshot) -> Result<(), String> {
    let Some(comms) = snapshot.comms.as_ref() else {
        return Ok(());
    };
    for message in &comms.inbox {
        let Some(wire) = message.transmission.as_ref().filter(|wire| wire.sustained) else {
            continue;
        };
        let CommsRadioContent::Request { request } = &wire.content else {
            return Err("only an actual planned Request can retain sustained delivery".into());
        };
        let CommsAudience::Contact { target_uuid } = &wire.audience else {
            return Err("sustained Request must retain one exact recipient".into());
        };
        // Durable Formation has its own ordinary source sequence/history, not
        // a fabricated World ReportStatus execution. Model validates that full
        // immutable lineage before this World-specific candidate check.
        if request.is_formation() {
            continue;
        }
        let rows = &snapshot
            .scenario
            .as_ref()
            .ok_or("sustained Request lost its actual World execution owner")?
            .situations;
        let matches = rows
            .instances
            .values()
            .filter(|row| {
                let Some(scope) = &row.accepted else {
                    return false;
                };
                let ActivityExecutionIdentity::Communication {
                    origin_id,
                    recipient,
                    emitted_tick,
                } = &scope.identity
                else {
                    return false;
                };
                let ActivityRequest::Communication {
                    owner,
                    channel,
                    request: expected,
                    ..
                } = &row.request
                else {
                    return false;
                };
                scope.actor == message.sender_uuid
                    && &scope.owner == owner
                    && origin_id == &wire.origin_id
                    && recipient == target_uuid
                    && *emitted_tick == wire.emitted_tick
                    && row.accepted_tick == Some(*emitted_tick)
                    && channel == &wire.channel
                    && expected == request
                    && message.recipient_ship.as_ref().is_some_and(|copy| {
                        copy.0
                            == if wire.received_tick.is_none() {
                                scope.actor.as_str()
                            } else {
                                recipient.as_str()
                            }
                    })
            })
            .count();
        if matches != 1 {
            return Err(
                "sustained Request lacks its exact unique accepted communication activity".into(),
            );
        }
    }
    Ok(())
}
