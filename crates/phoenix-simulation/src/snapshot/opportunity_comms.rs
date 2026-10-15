//! Immutable Comms offer context. World validates its actual retained offer;
//! this owner validates the addressed conversation, never an inferred target.
use super::*;
use phoenix_model::comms_request::OpportunityRef;
use std::collections::BTreeSet;
fn recipient(
    reference: &OpportunityRef,
    actor: Option<&crate::command_admission::log::ShipKey>,
) -> Result<(), String> {
    reference.validate().map_err(str::to_owned)?;
    if actor.is_none_or(|actor| actor.0 != reference.actor) {
        return Err("optional Comms offer lacks its exact immutable recipient".into());
    }
    Ok(())
}
pub(super) fn validate(snapshot: &PhoenixSnapshot) -> Result<(), String> {
    let Some(comms) = &snapshot.comms else {
        return Ok(());
    };
    let mut bindings = BTreeSet::new();
    for open in &comms.pending_opens {
        let Some(reference) = &open.opportunity else {
            continue;
        };
        recipient(reference, open.recipient_ship.as_ref())?;
        if open.from != "_self"
            || open.sender_uuid.is_some()
            || open.recipients.is_some()
            || open.origin_layer.is_some()
            || !bindings.insert((
                reference.source.clone(),
                reference.ordinal,
                reference.actor.clone(),
            ))
        {
            return Err(
                "optional work offer is not a unique own internal addressed conversation".into(),
            );
        }
        crate::world::opportunities::validate_offered_opportunity_reference(
            snapshot,
            reference,
            &reference.actor,
            snapshot.tick,
        )?;
    }
    for dialogue in &comms.dialogues {
        let Some(reference) = &dialogue.script.opportunity else {
            continue;
        };
        recipient(reference, dialogue.script.recipient_ship.as_ref())?;
        if dialogue.script.origin_layer.is_some()
            || !bindings.insert((
                reference.source.clone(),
                reference.ordinal,
                reference.actor.clone(),
            ))
            || !comms.inbox.iter().any(|message| {
                message.id == dialogue.message_id
                    && message.sender_uuid == "_self"
                    && message
                        .recipient_ship
                        .as_ref()
                        .is_some_and(|ship| ship.0 == reference.actor)
                    && message.transmission.is_none()
            })
        {
            return Err(
                "optional work offer dialogue lost its actual internal recipient lineage".into(),
            );
        }
        crate::world::opportunities::validate_opportunity_reference(
            snapshot,
            reference,
            &reference.actor,
            snapshot.tick,
        )?;
    }
    Ok(())
}
