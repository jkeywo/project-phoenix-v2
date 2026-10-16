//! Accepted receiving intent is local Comms knowledge, never remote machinery authority.
use crate::{
    core::messages::{SystemBlackboard, SystemId},
    server_app::ShipSystemBlackboards,
    ship_plugin::ShipConfigComponent,
};
use phoenix_model::{
    comms_request::{CommsRequestDecision, CommsRequestReference},
    craft::{CraftReceptionOrder, CraftReceptionReceipt},
};

/// The receiver's actual heard copies survive dismissal and source loss. Feed
/// permission is explicit; co-hosting a Console never grants access to this list.
pub(crate) fn accepted(
    config: &ShipConfigComponent,
    boards: &ShipSystemBlackboards,
    reader: &SystemId,
    actor: &str,
    inbox: &super::server::CommsInboxRes,
    now: u64,
) -> Vec<(CraftReceptionOrder, String)> {
    let permitted = config.0.observations_for_system(reader, &boards.0);
    if !config.0.systems.iter().any(|system| {
        system.kind == crate::ship::system_registry::COMMS_KIND
            && matches!(permitted.get(&system.id), Some(SystemBlackboard::Comms(_)))
    }) {
        return Vec::new();
    }
    let mut result = Vec::new();
    for message in inbox.0.iter() {
        let Some(reference) = CommsRequestReference::from_received(message) else {
            continue;
        };
        if reference.recipient_ship != actor || !reference.request.is_reception() {
            continue;
        }
        let Some(progress) = &message.request_progress else {
            continue;
        };
        let Some(decision) = progress
            .facts
            .decision
            .as_ref()
            .filter(|decision| decision.choice == CommsRequestDecision::Accept)
        else {
            continue;
        };
        if progress.facts.reception.is_some()
            || progress.validate(Some(reference.heard_tick), now).is_err()
        {
            continue;
        }
        let order = CraftReceptionOrder {
            request: reference,
            decision_tick: decision.tick,
        };
        if order.validate(now).is_ok() {
            result.push((order, message.sender_name.clone()));
        }
    }
    result.sort_by(|a, b| a.0.request.origin_id.cmp(&b.0.request.origin_id));
    result
}

/// Stage the exact local copy before a physical commit. No late lookup/failure
/// may turn a successful custody move into an unreported or fabricated handoff.
pub(crate) fn completed_copy(
    inbox: &super::server::CommsInboxRes,
    receipt: &CraftReceptionReceipt,
    now: u64,
) -> Option<crate::core::messages::CommsMessage> {
    receipt.validate(now).ok()?;
    let mut message = inbox
        .0
        .iter()
        .find(|message| {
            CommsRequestReference::from_received(message).as_ref() == Some(&receipt.order.request)
        })?
        .clone();
    let progress = message.request_progress.as_mut()?;
    if progress.facts.reception.is_some()
        || !progress.facts.decision.as_ref().is_some_and(|decision| {
            decision.choice == CommsRequestDecision::Accept
                && decision.tick == receipt.order.decision_tick
        })
    {
        return None;
    }
    progress.facts.reception = Some(receipt.clone());
    progress
        .validate(Some(receipt.order.request.heard_tick), now)
        .ok()?;
    Some(message)
}
