//! Own retained intent is valid even while its contact is not observable.
//! Restore validates authored ownership and shape, never foreign scene facts.
use super::*;
use phoenix_sim_gameplay::weapons::assignments::{is_weapon, WeaponTargetAssignments};

fn valid_contact(contact: &str) -> bool {
    !contact.is_empty() && contact.len() <= 512 && !contact.chars().any(char::is_control)
}

pub(super) fn validate_roster(
    world: &World,
    snapshot: &PhoenixSnapshot,
) -> Result<(), (String, String)> {
    for row in &snapshot.entities {
        let result = (|| {
            let live = world
                .try_query_filtered::<(Entity, &EntityUuid), Allow<bevy::ecs::entity_disabling::Disabled>>()
                .and_then(|mut query| {
                    query
                        .iter(world)
                        .find_map(|(entity, uuid)| (uuid.0 == row.uuid).then_some(entity))
                });
            let config = if let Some(profile) = live.and_then(|entity| {
                world.get::<crate::resources::fitting_server::CompiledShipFitting>(entity)
            }) {
                Some(
                    profile.0.effective_config(
                        &row.fitting
                            .as_ref()
                            .ok_or("weapon assignments require saved fitted equipment")?
                            .equipment,
                    )?,
                )
            } else if let Some(origin) = &row.spawn {
                let base = origin
                    .resolve(
                        &crate::entities::loader::WasmTemplateLoader,
                        &mut Vec::new(),
                    )
                    .ok_or("cannot resolve saved weapon equipment")?;
                if let Some(saved) = &row.fitting {
                    let profile = crate::resources::fitting::CompiledFitting::compile(&base)?
                        .ok_or("saved weapon fit has no accepted fitting profile")?;
                    Some(profile.effective_config(&saved.equipment)?)
                } else {
                    Some(base)
                }
            } else {
                None
            };
            let ship = config
                .as_ref()
                .and_then(|config| config.ship_config.as_ref());
            let live_ship = live
                .and_then(|entity| world.get::<crate::ship_plugin::ShipConfigComponent>(entity));
            let ship = ship.or_else(|| live_ship.map(|config| &config.0));
            let expected =
                ship.is_some_and(|ship| ship.systems.iter().any(|system| is_weapon(&system.kind)));
            if expected != row.weapon_targets.is_some() {
                return Err("weapon assignment owner disagrees with accepted equipment".to_owned());
            }
            let Some(saved) = &row.weapon_targets else {
                return Ok(());
            };
            let ship = ship.ok_or("weapon intent has no owning ship configuration")?;
            let live_weapons = live.and_then(|entity| {
                world.get::<crate::entities::spawner::WeaponsConsoleSection>(entity)
            });
            let groups = config
                .as_ref()
                .and_then(|config| config.weapons_console.as_ref())
                .map(|config| config.groups.as_slice())
                .or_else(|| live_weapons.map(|weapons| weapons.0.groups.as_slice()))
                .unwrap_or_default();
            validate(saved, ship, groups)
        })();
        result.map_err(|reason| (row.uuid.clone(), reason))?;
    }
    Ok(())
}

fn validate(
    saved: &WeaponTargetAssignments,
    ship: &crate::ship::config::ShipConfig,
    groups: &[phoenix_model::weapon_targets::WeaponGroupConfig],
) -> Result<(), String> {
    use phoenix_model::weapon_targets::WeaponIndividualAssignment;
    for (owner, assignment) in &saved.0 {
        if !ship
            .system(owner)
            .is_some_and(|system| is_weapon(&system.kind))
            || assignment.is_empty()
        {
            return Err("invalid or redundant weapon assignment owner".into());
        }
        if assignment.automatic.is_some()
            && ship
                .system(owner)
                .is_none_or(|system| system.kind != crate::ship::system_registry::BLASTER_BANK_KIND)
        {
            return Err("automatic fallback requires an actual supported blaster owner".into());
        }
        if assignment
            .automatic
            .as_deref()
            .is_some_and(|contact| !valid_contact(contact))
        {
            return Err("invalid saved automatic weapon contact".into());
        }
        if let Some(group) = &assignment.group {
            if !groups
                .iter()
                .any(|prepared| prepared.id == group.group && prepared.members.contains(owner))
                || group
                    .target
                    .as_deref()
                    .is_some_and(|target| !valid_contact(target))
            {
                return Err(
                    "saved weapon group is not an authored member or has an invalid contact".into(),
                );
            }
        }
        if let Some(WeaponIndividualAssignment::Target { contact }) = &assignment.individual {
            if !valid_contact(contact) {
                return Err("invalid saved weapon contact".into());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "weapon_targets_tests.rs"]
mod tests;
