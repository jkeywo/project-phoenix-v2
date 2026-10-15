//! Validate the existing pending response owner against candidate request copies.
//! The inbox is the one lineage store; this adds no timer or historical archive.
use super::*;
use std::collections::BTreeSet;

pub(super) fn validate(world: &World, snapshot: &PhoenixSnapshot) -> Result<(), String> {
    let Some(comms) = &snapshot.comms else {
        return Ok(());
    };
    // This first closed request reports only its controlling Comms capability.
    // Historical claims survive actor removal; no live state is used as proof.
    for message in &comms.inbox {
        let retained = message
            .request_progress
            .as_ref()
            .and_then(|progress| progress.facts.report.as_ref());
        let returned = message
            .transmission
            .as_ref()
            .and_then(|wire| match &wire.content {
                phoenix_model::comms_request::CommsRadioContent::Outcome { outcome } => {
                    outcome.facts.report.as_ref()
                }
                _ => None,
            });
        if retained
            .into_iter()
            .chain(returned)
            .any(|report| report.system != crate::ship::system_registry::comms_system_id())
        {
            return Err("Report Status cannot claim another System's unbound facts".into());
        }
    }
    let mut keys = BTreeSet::new();
    for (key, record) in &comms.pending_ai_responses {
        if !phoenix_model::comms_channel::identity(&key.ship_uuid)
            || key.message_id.trim().is_empty()
            || key.message_id.chars().any(char::is_control)
            || !keys.insert(key)
        {
            return Err(
                "pending Comms response has duplicate or malformed actual owner/message".into(),
            );
        }
        let message = comms.inbox.iter().find(|message| {
            message.id == key.message_id
                && message
                    .recipient_ship
                    .as_ref()
                    .is_some_and(|recipient| recipient.0 == key.ship_uuid)
        });
        let Some(reading) = &record.request_reading else {
            if key.planned_request || message.is_some_and(crate::comms::requests::is_request) {
                return Err("structured request cannot acquire a legacy weighted wait".into());
            }
            continue;
        };
        let heard = if key.planned_request {
            None
        } else {
            let message =
                message.ok_or("pending request reading has no actual recipient inbox copy")?;
            if !crate::comms::requests::is_request(message)
                || message.is_read
                || message.selected_response.is_some()
            {
                return Err("pending request reading must own an unfinished heard request".into());
            }
            Some(
                message
                    .transmission
                    .as_ref()
                    .and_then(|wire| wire.received_tick)
                    .ok_or("request reading cannot precede physical delivery")?,
            )
        };
        let row = snapshot
            .entities
            .iter()
            .find(|row| row.uuid == key.ship_uuid)
            .ok_or("pending request reading has no candidate actor")?;
        let live = world
            .try_query_filtered::<(Entity, &EntityUuid), Allow<bevy::ecs::entity_disabling::Disabled>>()
            .and_then(|mut query| {
                query
                    .iter(world)
                    .find_map(|(entity, uuid)| (uuid.0 == key.ship_uuid).then_some(entity))
            });
        let accepted = match &row.spawn {
            Some(origin) => {
                let mut warnings = Vec::new();
                let config = origin
                    .resolve(&crate::entities::loader::WasmTemplateLoader, &mut warnings)
                    .ok_or("pending request actor's accepted source recipe cannot resolve")?;
                if !warnings.is_empty() {
                    return Err(
                        "pending request actor's accepted source recipe did not apply exactly"
                            .into(),
                    );
                }
                Some(config)
            }
            None => None,
        };
        // A declared replacement recipe is the candidate's exclusive capability
        // authority. Missing fields must not borrow the old bootstrap equipment.
        let (profile, graph, declared) = if let Some(config) = &accepted {
            (
                config.comms.as_ref().and_then(|comms| comms.radio.as_ref()),
                config.ship_config.as_ref(),
                config
                    .comms_console
                    .as_ref()
                    .is_some_and(|console| console.ai.is_some()),
            )
        } else {
            (
                live.and_then(|entity| world.get::<crate::comms::radio::RadioProfile>(entity))
                    .map(|profile| &profile.0),
                live.and_then(|entity| {
                    world.get::<crate::ship_plugin::ShipConfigComponent>(entity)
                })
                .map(|config| &config.0),
                live.is_some_and(|entity| {
                    world
                        .get::<crate::console::comms::server::CommsResponseAiPolicy>(entity)
                        .is_some()
                }),
            )
        };
        let profile = profile.ok_or("pending request reading has no actual fitted radio")?;
        let graph = graph.ok_or("pending request reading has no actual Console/System graph")?;
        if !declared
            || graph.system(&profile.system).is_none_or(|system| {
                system.kind != crate::ship::system_registry::COMMS_KIND
                    || graph.console_for_system(&system.id).is_none()
            })
        {
            return Err(
                "request reading is not owned by the actual declared Comms controller".into(),
            );
        }
        reading
            .validate(profile.request_reading_ticks, snapshot.tick)
            .map_err(str::to_string)?;
        let heard = if key.planned_request {
            crate::world::activities::validate_planned_communication_reading(
                snapshot,
                &key.ship_uuid,
                &profile.system,
                &key.message_id,
            )?
        } else {
            heard.expect("validated incoming receipt")
        };
        if (!key.planned_request && reading.remaining_ticks == 0)
            || reading.last_tick < heard
            || u64::from(reading.required_ticks - reading.remaining_ticks)
                > reading.last_tick - heard
            || record.due_tick != reading.due_tick().expect("validated reading frontier")
            || record.response_fingerprint != 0
        {
            return Err(
                "pending request reading has a completed, premature or mismatched clock".into(),
            );
        }
    }
    Ok(())
}
