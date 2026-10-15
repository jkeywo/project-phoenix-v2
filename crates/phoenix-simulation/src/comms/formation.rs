//! Read views of genuinely heard coordination only. No Navigation/Helm action.
use crate::{
    core::messages::{SystemBlackboard, SystemId},
    server_app::ShipSystemBlackboards,
    ship_plugin::ShipConfigComponent,
};
use phoenix_model::comms_request::{CommsRadioContent, CommsRequestDecision, GenericCommsRequest};
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FormationSuggestion {
    pub message_id: String,
    pub origin_id: String,
    /// Heard Comms provenance, never evidence for a private Broad association.
    pub source_ship: String,
    pub source_name: String,
    pub sequence: u64,
    pub slot: [i32; 2],
    pub heard_tick: u64,
    pub accepted_tick: u64,
}
/// A receiving System must explicitly read the actual Comms publication. A
/// shared Station or this function's existence grants no additional knowledge.
pub fn accepted(
    config: &ShipConfigComponent,
    boards: &ShipSystemBlackboards,
    reader: &SystemId,
    actor: &str,
) -> Vec<FormationSuggestion> {
    let permitted = config.0.observations_for_system(reader, &boards.0);
    let mut latest = std::collections::BTreeMap::<String, FormationSuggestion>::new();
    for comms in config
        .0
        .systems
        .iter()
        .filter(|system| system.kind == crate::ship::system_registry::COMMS_KIND)
    {
        let Some(SystemBlackboard::Comms(board)) = permitted.get(&comms.id) else {
            continue;
        };
        for message in &board.messages {
            if !message
                .recipient_ship
                .as_ref()
                .is_some_and(|id| id.0 == actor)
            {
                continue;
            }
            let Some(wire) = &message.transmission else {
                continue;
            };
            let CommsRadioContent::Request {
                request: GenericCommsRequest::FormUp { slot },
            } = wire.content
            else {
                continue;
            };
            let (Some(sequence), Some(heard_tick), Some(progress)) = (
                wire.formation_sequence,
                wire.received_tick,
                &message.request_progress,
            ) else {
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
            if progress.facts.read_tick.is_none() || heard_tick > decision.tick {
                continue;
            }
            let suggestion = FormationSuggestion {
                message_id: message.id.clone(),
                origin_id: wire.origin_id.clone(),
                source_ship: message.sender_uuid.clone(),
                source_name: message.sender_name.clone(),
                sequence,
                slot,
                heard_tick,
                accepted_tick: decision.tick,
            };
            let held = latest
                .entry(message.sender_uuid.clone())
                .or_insert_with(|| suggestion.clone());
            if held.sequence < sequence {
                *held = suggestion;
            }
        }
    }
    latest.into_values().collect()
}
