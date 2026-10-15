//! Whole-candidate source knowledge joins. Missing remote flights are ordinary
//! unresolved knowledge, never evidence that a launch record should disappear.
use super::*;
use crate::console::weapons::guidance_link::ShipGuidanceKnowledge;
use std::collections::BTreeSet;

/// Row-local joins are shared by full restore, materialization and GM inverse.
/// Historical row validation uses the caller's clock; it never joins other actors'
/// current physical vectors or treats a missing remote round as a knowledge error.
pub(super) fn validate_entity(
    world: &World,
    live: Option<Entity>,
    row: &EntityState,
    tick: u64,
) -> Result<(), String> {
    let accepted = row.spawn.as_ref().and_then(|origin| {
        origin.resolve(
            &crate::entities::loader::WasmTemplateLoader,
            &mut Vec::new(),
        )
    });
    let expected = live.is_some_and(|entity| world.get::<ShipGuidanceKnowledge>(entity).is_some())
        || accepted
            .as_ref()
            .is_some_and(|config| config.torpedoes.is_some());
    if expected != row.guided_source.is_some() {
        return Err(
            "source-owned guided knowledge disagrees with accepted source equipment".into(),
        );
    }
    let Some(knowledge) = &row.guided_source else {
        return Ok(());
    };
    if row.weapon_flight_carrier {
        return Err("a flight-only carrier cannot own source command knowledge".into());
    }
    let live_ship =
        live.and_then(|entity| world.get::<crate::ship_plugin::ShipConfigComponent>(entity));
    let ship = accepted
        .as_ref()
        .and_then(|config| config.ship_config.as_ref())
        .or_else(|| live_ship.map(|config| &config.0))
        .ok_or("guided source knowledge has no accepted owning System graph")?;
    phoenix_sim_gameplay::weapons::guided_orders::validate(knowledge, tick)?;
    for known in knowledge.launches.values() {
        if ship
            .system(&known.tube)
            .is_none_or(|system| system.kind != crate::ship::system_registry::TORPEDO_TUBE_KIND)
        {
            return Err("guided launch knowledge has no actual authored tube owner".into());
        }
    }
    let bursts = row
        .weapons
        .as_ref()
        .map(|weapons| weapons.bursts.as_slice())
        .unwrap_or_default();
    if knowledge.launch_commands.len() != bursts.len() {
        return Err("source launch command roster differs from actual pending bursts".into());
    }
    let mut launch_ids = BTreeSet::new();
    for burst in bursts {
        let command = knowledge
            .launch_commands
            .get(&burst.launch_uuid)
            .ok_or("pending burst has no original source launch command")?;
        if !launch_ids.insert(&burst.launch_uuid)
            || burst.source_uuid.as_deref() != Some(row.uuid.as_str())
            || crate::ship::system_registry::torpedo_tube_system_id(&burst.tube_id).as_ref()
                != Some(&command.tube)
        {
            return Err("burst launch command has a duplicate or foreign tube/source claim".into());
        }
    }
    Ok(())
}

pub(super) fn validate_roster(
    world: &World,
    snapshot: &PhoenixSnapshot,
) -> Result<(), (String, String)> {
    let mut known_ids = BTreeSet::new();
    let mut launch_ids = BTreeSet::new();
    for row in &snapshot.entities {
        let checked = (|| {
            let live = world
                .try_query_filtered::<(Entity, &EntityUuid), Allow<bevy::ecs::entity_disabling::Disabled>>()
                .and_then(|mut query| {
                    query
                        .iter(world)
                        .find_map(|(entity, id)| (id.0 == row.uuid).then_some(entity))
                });
            validate_entity(world, live, row, snapshot.tick)?;
            let Some(knowledge) = &row.guided_source else {
                return Ok(());
            };
            for (id, known) in &knowledge.launches {
                if !known_ids.insert(id) {
                    return Err("guided launch knowledge is claimed by two source actors".into());
                }
                if let Some(round) = snapshot
                    .entities
                    .iter()
                    .filter_map(|row| row.weapons.as_ref())
                    .flat_map(|weapons| &weapons.torpedoes_in_flight)
                    .find(|round| &round.uuid == id)
                {
                    if round.source_uuid.as_deref() != Some(row.uuid.as_str())
                        || crate::ship::system_registry::torpedo_tube_system_id(&round.tube_id)
                            .as_ref()
                            != Some(&known.tube)
                        || round.profile.receiver != known.receiver
                        || round.profile.lifespan.to_bits() != known.lifespan_secs.to_bits()
                    {
                        return Err("source launch knowledge disagrees with its actual frozen emitted owner".into());
                    }
                }
            }
            for id in knowledge.launch_commands.keys() {
                if !launch_ids.insert(id) {
                    return Err("burst launch command is claimed by two source actors".into());
                }
            }
            Ok(())
        })();
        checked.map_err(|reason| (row.uuid.clone(), reason))?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "guided_source_tests.rs"]
mod tests;
