//! Join actual paid radio origins and heard receipts to the existing World
//! situation owner. No dispatcher, simulated reading or foreign knowledge.
use crate::world::{config::WorldConfig, server::WorldContentRuntime};
use phoenix_model::{
    activity::{ActivityExecution, ActivityExecutionIdentity},
    comms_channel::CommsAudience,
    comms_request::{GenericCommsRequest, PlannedCommsRequest},
    identity::SystemId,
};
use phoenix_sim_world::world::activities::{
    ActivityRequest, SituationInstance, SituationInstances,
};

fn released<'a>(
    instances: &'a SituationInstances,
    actor: &str,
    owner: &SystemId,
    id: &str,
    now: u64,
) -> Option<&'a SituationInstance> {
    let row = instances.instances.get(id)?;
    (id.len() <= 2048
        && row.accepted.is_none()
        && row.accepted_tick.is_none()
        && row.armed_tick <= now
        && row.request.actor() == actor
        && row.request.owner() == owner
        && row.state.is_some()
        && matches!(row.request, ActivityRequest::Communication { .. }))
    .then_some(row)
}
pub(crate) fn communication_request_release(
    runtime: &WorldContentRuntime,
    actor: &str,
    owner: &SystemId,
    instance: &str,
    now: u64,
) -> Option<u64> {
    released(&runtime.situations, actor, owner, instance, now).map(|row| row.armed_tick)
}
pub(crate) fn validate_planned_communication_reading(
    snapshot: &crate::snapshot::PhoenixSnapshot,
    actor: &str,
    owner: &SystemId,
    instance: &str,
) -> Result<u64, String> {
    let rows = &snapshot
        .scenario
        .as_ref()
        .ok_or("planned reading lacks candidate World owner")?
        .situations;
    released(rows, actor, owner, instance, snapshot.tick)
        .map(|row| row.armed_tick)
        .ok_or_else(|| {
            "planned reading lacks its exact released unaccepted communication binding".into()
        })
}
fn recipient<'a>(base: &'a WorldConfig, reference: &str) -> Option<&'a str> {
    (!reference.starts_with("slot:"))
        .then(|| base.name_to_uuid.get(reference))
        .flatten()
        .map(String::as_str)
}
/// Only already-known contacts enter the ordinary Console request view.
pub(crate) fn communication_requests(
    runtime: Option<&WorldContentRuntime>,
    base: Option<&WorldConfig>,
    actor: &str,
    owner: &SystemId,
    now: u64,
    known: impl Fn(&str) -> bool,
) -> Vec<PlannedCommsRequest> {
    let (Some(runtime), Some(base)) = (runtime, base) else {
        return Vec::new();
    };
    let mut rows = runtime
        .situations
        .instances
        .iter()
        .filter_map(|(id, _)| {
            let row = released(&runtime.situations, actor, owner, id, now)?;
            let ActivityRequest::Communication {
                recipient: reference,
                channel,
                request,
                ..
            } = &row.request
            else {
                return None;
            };
            let uuid = recipient(base, reference)?;
            if !known(uuid) {
                return None;
            }
            let value = PlannedCommsRequest {
                id: id.clone(),
                released_tick: row.armed_tick,
                recipient_uuid: uuid.into(),
                channel: channel.clone(),
                request: *request,
            };
            value.validate(now).ok()?;
            Some(value)
        })
        .collect::<Vec<_>>();
    rows.sort_by(|a, b| (a.released_tick, &a.id).cmp(&(b.released_tick, &b.id)));
    rows
}
/// Exclusive borrowed permit, not saved state. Dropping after an ordinary
/// operation/energy refusal makes no World write.
pub(crate) struct PreparedCommunicationRequest<'a> {
    instance: &'a mut SituationInstance,
    actor: String,
    owner: SystemId,
    recipient: String,
    tick: u64,
}
#[allow(clippy::too_many_arguments)]
pub(crate) fn prepare_communication_request<'a>(
    runtime: Option<&'a mut WorldContentRuntime>,
    base: Option<&WorldConfig>,
    actor: &str,
    owner: &SystemId,
    context: Option<&str>,
    audience: &CommsAudience,
    channel: &str,
    request: &GenericCommsRequest,
    now: u64,
) -> Result<Option<PreparedCommunicationRequest<'a>>, String> {
    let Some(id) = context else {
        return Ok(None);
    };
    let runtime = runtime.ok_or("planned communication requires its actual World owner")?;
    let base = base.ok_or("planned communication requires its accepted base World")?;
    let row = released(&runtime.situations, actor, owner, id, now)
        .ok_or("stale or foreign planned communication context")?;
    let ActivityRequest::Communication {
        recipient: reference,
        channel: wanted_channel,
        request: wanted_request,
        ..
    } = &row.request
    else {
        unreachable!("released communication kind checked");
    };
    let target = recipient(base, reference)
        .ok_or("planned communication recipient has no actual accepted base identity")?;
    if wanted_channel != channel
        || wanted_request != request
        || !matches!(audience, CommsAudience::Contact { target_uuid } if target_uuid == target)
    {
        return Err(
            "planned communication command disagrees with its exact declared binding".into(),
        );
    }
    let target = target.to_owned();
    Ok(Some(PreparedCommunicationRequest {
        instance: runtime
            .situations
            .instances
            .get_mut(id)
            .expect("exclusive prepared row exists"),
        actor: actor.into(),
        owner: owner.clone(),
        recipient: target,
        tick: now,
    }))
}
/// Called only after the same caller commits ordinary paid source emission.
/// The exclusive permit keeps acceptance infallible after that payment.
pub(crate) fn bind_communication_request(
    prepared: PreparedCommunicationRequest<'_>,
    origin_id: &str,
    emitted_tick: u64,
) {
    assert_eq!(
        emitted_tick, prepared.tick,
        "paid emission must consume its same-tick prepared binding"
    );
    let scope = ActivityExecution {
        actor: prepared.actor,
        owner: prepared.owner,
        identity: ActivityExecutionIdentity::Communication {
            origin_id: origin_id.into(),
            recipient: prepared.recipient,
            emitted_tick,
        },
    };
    scope
        .validate(emitted_tick)
        .expect("actual minted communication identity is valid");
    prepared.instance.accepted = Some(scope);
    prepared.instance.accepted_tick = Some(emitted_tick);
}

/// Observe only genuinely heard own returned records. A foreign Request copy
/// with local reading progress is deliberately not an input.
pub(crate) fn observe_communication_receipts(
    runtime: &mut WorldContentRuntime,
    messages: &[phoenix_model::messages::CommsMessage],
    now: u64,
) {
    use phoenix_model::{activity::ActivityOutcome, comms_request::CommsRadioContent};
    let scopes = runtime
        .situations
        .instances
        .values()
        .filter_map(|row| {
            let scope = row.accepted.as_ref()?;
            matches!(
                scope.identity,
                ActivityExecutionIdentity::Communication { .. }
            )
            .then_some(scope.clone())
        })
        .collect::<Vec<_>>();
    for scope in scopes {
        let ActivityExecutionIdentity::Communication {
            origin_id,
            recipient,
            emitted_tick,
        } = &scope.identity
        else {
            unreachable!();
        };
        let has_sent = messages.iter().any(|message| message.sender_uuid == scope.actor
            && message.recipient_ship.as_ref().is_some_and(|id| id.0 == scope.actor)
            && message.transmission.as_ref().is_some_and(|wire| wire.origin_id == *origin_id && wire.sustained
                    && wire.received_tick.is_none()
                && wire.emitted_tick == *emitted_tick && matches!(&wire.audience, CommsAudience::Contact { target_uuid } if target_uuid == recipient)
                && matches!(wire.content, CommsRadioContent::Request { .. })));
        if !has_sent {
            continue;
        }
        let mut heard = messages
            .iter()
            .filter_map(|message| {
                if message
                    .recipient_ship
                    .as_ref()
                    .is_none_or(|id| id.0 != scope.actor)
                    || message.sender_uuid != *recipient
                {
                    return None;
                }
                let wire = message.transmission.as_ref()?;
                let received = wire.received_tick.filter(|tick| *tick <= now)?;
                let CommsRadioContent::Outcome { outcome } = &wire.content else {
                    return None;
                };
                let reference = &outcome.request;
                (reference.origin_id == *origin_id
                    && reference.source_ship == scope.actor
                    && reference.recipient_ship == *recipient
                    && reference.emitted_tick == *emitted_tick
                    && outcome.validate(received).is_ok())
                .then_some((received, &wire.origin_id, outcome))
            })
            .collect::<Vec<_>>();
        heard.sort_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)));
        for (received, receipt, outcome) in heard {
            if let Some(read_tick) = outcome.facts.read_tick {
                if let Some(fact) = runtime.situations.fact(
                    &scope,
                    received,
                    ActivityOutcome::CommunicationRead {
                        reference: outcome.request.clone(),
                        read_tick,
                        receipt_origin_id: receipt.clone(),
                    },
                ) {
                    super::retain_fact(runtime, fact);
                }
            }
            if outcome.facts.decision.is_some() {
                if let Some(fact) = runtime.situations.fact(
                    &scope,
                    received,
                    ActivityOutcome::CommunicationReturned {
                        outcome: outcome.clone(),
                        receipt_origin_id: receipt.clone(),
                    },
                ) {
                    super::retain_fact(runtime, fact);
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "activity_communication_tests.rs"]
mod tests;
