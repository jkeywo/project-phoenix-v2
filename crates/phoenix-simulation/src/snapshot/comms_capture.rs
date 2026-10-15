//! Exact receiving equipment and bounded retained emission consistency. Historical
//! emitters may be absent; this does not authenticate erased historical truth.
use super::*;
use phoenix_model::comms_channel::CommsRadioConfig;
use phoenix_sim_gameplay::electronic::capture::CapturedPayload;

fn actor(world: &World, uuid: &str) -> Option<Entity> {
    world
        .try_query_filtered::<(Entity, &EntityUuid), Allow<bevy::ecs::entity_disabling::Disabled>>()
        .and_then(|mut query| {
            query
                .iter(world)
                .find_map(|(entity, id)| (id.0 == uuid).then_some(entity))
        })
}

/// A saved origin is the accepted immutable recipe, including materialized rows
/// absent from bootstrap. Bare owner fixtures use their installed actual profile.
fn radio(
    world: &World,
    live: Option<Entity>,
    row: &EntityState,
) -> Result<Option<CommsRadioConfig>, String> {
    if let Some(origin) = &row.spawn {
        let mut warnings = Vec::new();
        let accepted = origin
            .resolve(&crate::entities::loader::WasmTemplateLoader, &mut warnings)
            .ok_or("capture candidate source recipe cannot resolve")?;
        if !warnings.is_empty() {
            return Err("capture candidate recipe is not exact".into());
        }
        return Ok(accepted.comms.and_then(|comms| comms.radio));
    }
    Ok(live
        .and_then(|entity| world.get::<crate::comms::radio::RadioProfile>(entity))
        .map(|profile| profile.0.clone()))
}

fn validate_row(
    world: &World,
    live: Option<Entity>,
    row: &EntityState,
    tick: u64,
) -> Result<(), String> {
    let profile = radio(world, live, row)?;
    match (
        profile
            .as_ref()
            .and_then(|radio| radio.capture.as_ref().map(|config| (radio, config))),
        &row.comms_capture,
    ) {
        (None, None) => Ok(()),
        (Some((radio, config)), Some(saved)) => saved
            .0
            .validate(&radio.system, config, &radio.keys, tick)
            .map_err(str::to_owned),
        _ => Err("saved capture owner differs from exact accepted receiving equipment".into()),
    }?;
    // A saved publication can withhold a view, but may never reveal plaintext or
    // a handle/header unsupported by this exact canonical receiving owner.
    for (owner, board) in &row.blackboards {
        let crate::core::messages::SystemBlackboard::Comms(board) = board else {
            continue;
        };
        let Some(view) = &board.captured else {
            continue;
        };
        let expected = row
            .comms_capture
            .as_ref()
            .zip(profile.as_ref())
            .and_then(|(saved, radio)| crate::comms::capture::views(saved, radio));
        if profile
            .as_ref()
            .is_none_or(|radio| radio.system.0 != *owner)
            || expected.as_ref() != Some(view)
        {
            return Err("saved capture publication contradicts its canonical paid frontier".into());
        }
    }
    Ok(())
}

fn source(payload: &CapturedPayload, profile: &CommsRadioConfig) -> Result<(), String> {
    let channel = profile
        .channels
        .iter()
        .find(|channel| channel.id == payload.transmission.channel)
        .ok_or("captured channel contradicts retained source equipment")?;
    let level = if channel.key.is_some() {
        channel.protection_level
    } else {
        0
    };
    if payload.key != channel.key || payload.level != level {
        return Err("captured security contradicts retained source equipment".into());
    }
    Ok(())
}

fn copies(
    payload: &CapturedPayload,
    messages: &[crate::core::messages::CommsMessage],
) -> Result<(), String> {
    for message in messages {
        let Some(wire) = message
            .transmission
            .as_ref()
            .filter(|wire| wire.origin_id == payload.transmission.origin_id)
        else {
            continue;
        };
        let mut original = wire.clone();
        original.received_tick = None;
        if message.sender_uuid != payload.source
            || message.sender_name != payload.name
            || message.body != payload.body
            || message.literal_body != payload.literal_body
            || original != payload.transmission
        {
            return Err("captured content contradicts a surviving actual emission copy".into());
        }
    }
    Ok(())
}

/// Common structural guard deliberately does not read bootstrap inbox/source
/// state while a complete candidate is being validated.
pub(super) fn validate_entity(
    world: &World,
    entity: Entity,
    row: &EntityState,
    tick: u64,
) -> Result<(), String> {
    validate_row(world, Some(entity), row, tick)
}

/// Single-row application has no replacement inbox: retained live evidence is
/// its candidate. Whole restore instead checks only the complete saved graph.
pub(super) fn validate_single(
    world: &World,
    entity: Entity,
    row: &EntityState,
    tick: u64,
) -> Result<(), String> {
    validate_row(world, Some(entity), row, tick)?;
    if let Some(saved) = &row.comms_capture {
        let messages = world
            .get_resource::<crate::comms::server::CommsInboxRes>()
            .map(|inbox| inbox.0.messages())
            .unwrap_or_default();
        for payload in saved.0.payloads() {
            if payload.source == row.uuid {
                return Err("capture cannot observe its own emitter".into());
            }
            if let Some(entity) = actor(world, &payload.source) {
                let profile = world
                    .get::<crate::comms::radio::RadioProfile>(entity)
                    .ok_or("captured source contradicts retained actual radio equipment")?;
                source(payload, &profile.0)?;
            }
            copies(payload, &messages)?;
        }
    }
    Ok(())
}

pub(super) fn validate_roster(
    world: &World,
    snapshot: &PhoenixSnapshot,
) -> Result<(), (String, String)> {
    let mut profiles = std::collections::BTreeMap::new();
    for row in &snapshot.entities {
        let live = actor(world, &row.uuid);
        validate_row(world, live, row, snapshot.tick)
            .map_err(|reason| (row.uuid.clone(), reason))?;
        profiles.insert(
            row.uuid.as_str(),
            radio(world, live, row).map_err(|reason| (row.uuid.clone(), reason))?,
        );
    }
    let messages = snapshot
        .comms
        .as_ref()
        .map_or(&[][..], |comms| comms.inbox.as_slice());
    for row in &snapshot.entities {
        let Some(saved) = &row.comms_capture else {
            continue;
        };
        let check = || -> Result<(), String> {
            for payload in saved.0.payloads() {
                if payload.source == row.uuid {
                    return Err("capture cannot observe its own emitter".into());
                }
                // Only genuinely retained candidate actors constrain equipment.
                // A bootstrap body absent from this candidate is historical.
                if let Some(profile) = profiles.get(payload.source.as_str()) {
                    source(
                        payload,
                        profile
                            .as_ref()
                            .ok_or("capture source has no accepted radio equipment")?,
                    )?;
                }
                copies(payload, messages)?;
            }
            Ok(())
        };
        check().map_err(|reason| (row.uuid.clone(), reason))?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "comms_capture_tests.rs"]
mod tests;
