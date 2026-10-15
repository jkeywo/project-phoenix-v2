//! Closed initial custody through the ordinary accepted World and hull recipes.
//! Tags remain physical classification; they are not a player/custody switch.
use crate::{
    entities::{config::EntityConfig, loader::TemplateLoader},
    world::{config::WorldEntitySpawnOn, validate::*},
};
use std::collections::BTreeMap;

fn resolve(
    row: &crate::world::config::WorldEntity,
    loader: &dyn TemplateLoader,
) -> Result<EntityConfig, String> {
    let base = loader
        .load_template(&row.template_path)
        .ok_or_else(|| format!("cannot resolve template {}", row.template_path))?;
    row.overrides
        .as_ref()
        .map_or(Ok(base.clone()), |overrides| {
            crate::entities::loader::apply_overrides(&base, overrides)
        })
}

/// Content checks every offered parent hull, not the currently selected live
/// equipment. Actual bootstrap additionally refuses a carried crew placement.
pub fn validate_initial(
    source: &WorldSource,
    loader: &dyn TemplateLoader,
    base_world: bool,
) -> Result<(), String> {
    if !source
        .config
        .entities
        .iter()
        .any(|row| row.carried_by.is_some())
    {
        return Ok(());
    }
    if !base_world {
        return Err("initial Carried craft supports unconditional base World rows only; additive/layer custody is unsupported".into());
    }
    phoenix_sim_world::world::craft::validate_initial_rows(source.config)?;
    let game_start_ships: Vec<_> = source
        .config
        .entities
        .iter()
        .filter(|row| row.spawn_on == WorldEntitySpawnOn::GameStart)
        .map(|row| resolve(row, loader).map(|config| (row, config)))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .filter(|(_, config)| config.tags.iter().any(|tag| tag == "ship"))
        .map(|(row, _)| row)
        .collect();
    let mut used: BTreeMap<(String, phoenix_model::identity::SystemId), u64> = BTreeMap::new();
    for row in &source.config.entities {
        let Some(custody) = &row.carried_by else {
            continue;
        };
        if game_start_ships
            .iter()
            .position(|candidate| candidate.name.as_deref() == Some(custody.parent.as_str()))
            .and_then(|index| source.config.ship_slots.get(index))
            .is_some_and(|slot| slot.unclaimed == crate::world::config::UnclaimedSlotPolicy::Absent)
        {
            return Err("initial Carried craft requires its parent launch slot to remain present when unclaimed".into());
        }
        let craft = resolve(row, loader)?;
        if row.spawn_on == WorldEntitySpawnOn::GameStart
            && craft.tags.iter().any(|tag| tag == "ship")
        {
            // Legacy fleet-size fallback may consume later ship rows. First
            // supported custody uses an explicit, unambiguous launch roster.
            let slots = source.config.ship_slots.len();
            if slots == 0
                || game_start_ships
                    .iter()
                    .position(|candidate| std::ptr::eq(*candidate, row))
                    .is_none_or(|index| index < slots)
                || game_start_ships.len() < slots
                || game_start_ships.iter().take(slots).any(|slot| {
                    slot.when.is_some()
                        || slot.when_predicate.is_some()
                        || slot.carried_by.is_some()
                })
            {
                return Err("Carried GameStart craft must follow all explicit unconditional crew launch-slot rows; it cannot itself be offered/selected as a crew hull".into());
            }
        }
        phoenix_sim_gameplay::ship::craft_profile::validate_entity(&craft)?;
        let probe = craft
            .probe
            .as_ref()
            .ok_or("initial Carried actor is not a supported physical probe")?;
        let occupied = used
            .entry((custody.parent.clone(), custody.bay.clone()))
            .or_default();
        *occupied = occupied
            .checked_add(probe.space)
            .ok_or("initial craft bay space overflow")?;
        for parent in super::activity_bindings::candidates(&custody.parent, source, loader)? {
            let bay = parent
                .ship_config
                .as_ref()
                .and_then(|ship| ship.system(&custody.bay))
                .and_then(|system| {
                    phoenix_sim_gameplay::ship::composition::craft_bay_composition(
                        &system.components,
                    )
                })
                .ok_or(
                    "initial Carried actor requires that actual bay on every offered parent hull",
                )?;
            phoenix_sim_gameplay::ship::craft_profile::validate_entity(&parent)?;
            if *occupied > bay.config.capacity_space {
                return Err(
                    "initial Carried craft exceeds its actual dedicated bay capacity".into(),
                );
            }
        }
    }
    Ok(())
}

pub(super) fn findings(
    source: &WorldSource,
    loader: &dyn TemplateLoader,
    base: bool,
) -> Vec<WorldFinding> {
    match validate_initial(source, loader, base) {
        Ok(()) => Vec::new(),
        Err(error) if !loader.absence_is_final() && error.starts_with("cannot resolve") => {
            Vec::new()
        }
        Err(error) => vec![WorldFinding::error(
            "unsupported-initial-craft-custody",
            &source.path,
            source.toml,
            "carried_by",
            error,
        )],
    }
}

#[cfg(test)]
#[path = "craft_tests.rs"]
mod tests;
