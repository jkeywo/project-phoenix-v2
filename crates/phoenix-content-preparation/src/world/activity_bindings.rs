//! Resolve authored activity support through the same accepted equipment.
use crate::{
    entities::{config::EntityConfig, loader::TemplateLoader},
    world::{
        config::{WorldEntity, WorldEntitySpawnOn},
        validate::{WorldFinding, WorldSource},
    },
};
use phoenix_sim_gameplay::jobs::activity_bindings::{self, RepairBinding};
use phoenix_sim_world::world::activities::ActivityRequest;

fn resolve(row: &WorldEntity, loader: &dyn TemplateLoader) -> Result<EntityConfig, String> {
    let base = loader
        .load_template(&row.template_path)
        .ok_or_else(|| format!("cannot resolve template {}", row.template_path))?;
    row.overrides.as_ref().map_or(Ok(base.clone()), |value| {
        crate::entities::loader::apply_overrides(&base, value)
    })
}
pub(super) fn candidates(
    actor: &str,
    source: &WorldSource,
    loader: &dyn TemplateLoader,
) -> Result<Vec<EntityConfig>, String> {
    let slots = source.config.effective_ship_slots();
    let mut ship_rows = Vec::new();
    for row in source
        .config
        .entities
        .iter()
        .filter(|row| row.spawn_on == WorldEntitySpawnOn::GameStart)
    {
        let config = resolve(row, loader)?;
        if config.tags.iter().any(|tag| tag == "ship") {
            ship_rows.push(row);
        }
    }
    let (row, slot) = if let Some(id) = actor.strip_prefix("slot:") {
        let index = slots
            .iter()
            .position(|row| row.id == id)
            .ok_or("activity requires a declared launch slot")?;
        (
            *ship_rows
                .get(index)
                .ok_or("activity launch slot has no actual GameStart ship row")?,
            Some(&slots[index]),
        )
    } else {
        let row = source
            .config
            .entities
            .iter()
            .find(|row| row.name.as_deref() == Some(actor))
            .ok_or("activity requires a declared base-world actor")?;
        let slot = ship_rows
            .iter()
            .position(|candidate| std::ptr::eq(*candidate, row))
            .and_then(|index| slots.get(index));
        (row, slot)
    };
    let mut configs = if slot.is_some() {
        Vec::new()
    } else {
        vec![resolve(row, loader)?]
    };
    if let Some(slot) = slot {
        for offer in &slot.ships {
            let selected = loader
                .load_template(&offer.template_path)
                .ok_or_else(|| format!("cannot resolve launch hull {}", offer.template_path))?;
            // Same override merge used by actual player_hull_config. A refused
            // merge cannot silently validate a different untuned contract.
            configs.push(
                row.overrides
                    .as_ref()
                    .map_or(Ok(selected.clone()), |overrides| {
                        crate::entities::loader::apply_overrides(&selected, overrides)
                    })?,
            );
        }
    }
    if configs.is_empty() {
        return Err("activity actor has no accepted hull choices".into());
    }
    Ok(configs)
}

pub(super) fn supports(request: &ActivityRequest, config: &EntityConfig) -> Result<bool, String> {
    match request {
        ActivityRequest::AcquireVessel { owner, team, .. } => Ok(config
            .ship_config
            .as_ref()
            .zip(config.security.as_ref())
            .is_some_and(|(ship, security)| {
                activity_bindings::acquisition_supported(owner, *team, ship, security)
            })),
        ActivityRequest::Communication { owner, channel, .. } => Ok(config
            .ship_config
            .as_ref()
            .zip(config.comms.as_ref().and_then(|row| row.radio.as_ref()))
            .is_some_and(|(ship, radio)| {
                activity_bindings::communication_supported(owner, channel, ship, radio)
            })),
        ActivityRequest::Repair {
            owner,
            team,
            request,
            target,
            goal,
            ..
        } => {
            let Some(ship) = &config.ship_config else {
                return Ok(false);
            };
            let Some(repair) = &config.repair else {
                return Ok(false);
            };
            let (Some(interior), Some(damage)) = (&config.interior, &config.damage) else {
                return Ok(false);
            };
            let definition =
                phoenix_sim_gameplay::ship::compartment_damage::DamageDefinition::compile(
                    interior, damage, ship,
                )?;
            Ok(activity_bindings::repair_supported(
                RepairBinding {
                    owner,
                    team: *team,
                    request,
                    target,
                    goal: *goal,
                },
                ship,
                &definition,
                &phoenix_sim_gameplay::ship::system_registry::repair_system_id(),
                repair.repair_team_count as usize,
            ))
        }
        ActivityRequest::Delivery {
            owner,
            source,
            destination,
            commodity,
            amount,
            ..
        } => {
            let Some(profile) =
                phoenix_sim_gameplay::resources::logistics_profile::compile_entity_logistics(
                    config,
                )?
            else {
                return Ok(false);
            };
            Ok(activity_bindings::delivery_supported(
                &profile,
                owner,
                source,
                destination,
                commodity,
                *amount,
            ))
        }
    }
}

/// Named base subjects use the accepted source, never a current scene lookup.
pub(super) fn acquisition_subject(
    request: &ActivityRequest,
    source: &WorldSource,
    loader: &dyn TemplateLoader,
) -> Result<(), String> {
    let ActivityRequest::AcquireVessel { actor, target, .. } = request else {
        return Ok(());
    };
    if target == actor || target.starts_with("slot:") {
        return Err("acquisition requires a distinct named base vessel".into());
    }
    let row = source
        .config
        .entities
        .iter()
        .find(|row| row.name.as_deref() == Some(target))
        .ok_or("acquisition subject is not a declared base vessel")?;
    // GameStart ship rows inside the launch roster are player/Backfill slots.
    if row.spawn_on == WorldEntitySpawnOn::GameStart {
        let mut index = 0;
        for candidate in &source.config.entities {
            if candidate.spawn_on != WorldEntitySpawnOn::GameStart
                || !resolve(candidate, loader)?
                    .tags
                    .iter()
                    .any(|tag| tag == "ship")
            {
                continue;
            }
            if std::ptr::eq(candidate, row) && index < source.config.effective_ship_slots().len() {
                return Err("acquisition subject cannot be a launch slot".into());
            }
            index += 1;
        }
    }
    let config = resolve(row, loader)?;
    let action = config
        .security_target
        .as_ref()
        .and_then(|target| target.action(phoenix_sim_gameplay::security::SecurityAction::Board))
        .and_then(|action| action.acquisition.as_ref())
        .ok_or("acquisition subject lacks an accepted Board acquisition effect")?;
    if !source.config.gm_npc_doctrine_palette.iter().any(|entry| {
        entry.id == action.doctrine
            && entry.origin_layer.is_none()
            && entry.targets.contains(target)
            && !entry.doctrine.is_empty()
            && entry.doctrine.iter().all(|objective| {
                matches!(
                    crate::ai::core::parse_doctrine_directive(objective),
                    Ok(phoenix_model::messages::AiDirective::None)
                )
            })
    }) {
        return Err("acquisition subject requires its accepted explicit all-None hold".into());
    }
    Ok(())
}

pub(super) fn findings(source: &WorldSource, loader: &dyn TemplateLoader) -> Vec<WorldFinding> {
    let mut findings = Vec::new();
    for situation in &source.config.situations {
        let result = (|| {
            acquisition_subject(&situation.request, source, loader)?;
            if let ActivityRequest::Communication { recipient, .. } = &situation.request {
                if recipient.starts_with("slot:") {
                    return Err(
                        "communication recipient requires a declared base-world actor".into(),
                    );
                }
                source.config.validate_situation_actor(recipient)?;
                for target in candidates(recipient, source, loader)? {
                    if target
                        .comms
                        .as_ref()
                        .and_then(|row| row.radio.as_ref())
                        .is_none()
                    {
                        return Err("communication recipient lacks actual radio equipment".into());
                    }
                }
            }
            candidates(situation.request.actor(),source,loader).and_then(|configs|{
            for config in configs {if !supports(&situation.request,&config)? {return Err("request/goal is outside the actual Repair visit/team/remedy or approved delivery route on an offered hull".into());}}
            Ok(())
        })
        })();
        if let Err(error) = result {
            // A browser preload which cannot yet answer a template lookup is
            // not evidence that the immutable equipment is absent. Its final
            // exact-source accepted validation supplies an authoritative loader.
            if !loader.absence_is_final() && error.starts_with("cannot resolve") {
                continue;
            }
            findings.push(WorldFinding::error(
                "unsupported-activity-binding",
                &source.path,
                source.toml,
                &situation.id,
                format!(
                    "Activity {:?} for {:?}: {error}",
                    situation.id,
                    situation.request.actor()
                ),
            ));
        }
    }
    findings
}

#[cfg(test)]
#[path = "activity_bindings_tests.rs"]
mod tests;
