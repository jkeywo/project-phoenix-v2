//! Candidate-only joins for the single retained personnel owner.
use super::*;
use phoenix_sim_gameplay::personnel::{
    CompiledPersonnel, OfficerLocation, PersonnelOrigin, PersonnelRoster,
};
use std::collections::BTreeMap;

fn live_actor(world: &World, uuid: &str) -> Option<Entity> {
    world.try_query_filtered::<(Entity,&EntityUuid),bevy::ecs::query::Allow<bevy::ecs::entity_disabling::Disabled>>()?
        .iter(world).find_map(|(entity,id)|(id.0==uuid).then_some(entity))
}
fn source_definition(
    source: &phoenix_model::personnel::PersonnelSource,
) -> Result<CompiledPersonnel, String> {
    use crate::entities::loader::TemplateLoader;
    let loader = crate::entities::loader::WasmTemplateLoader;
    let config = loader
        .load_template(&source.template_path)
        .ok_or("retained personnel template is unavailable")?;
    let config = if let Some(overrides) = &source.overrides {
        crate::entities::loader::apply_overrides(&config, overrides)
            .map_err(|_| "retained personnel overrides are invalid")?
    } else {
        config
    };
    phoenix_sim_gameplay::personnel::compile_entity(&config)?
        .ok_or_else(|| "retained source has no accepted personnel".to_owned())
}
fn validate_active(
    origin: &PersonnelOrigin,
    profile: &CompiledPersonnel,
    row: &EntityState,
    tick: u64,
) -> Result<(), String> {
    if origin.removed_tick.is_some() || &origin.definition != profile {
        return Err("active personnel differs from its immutable accepted definition".into());
    }
    let jobs = row
        .repair
        .as_ref()
        .and_then(|repair| repair.jobs.as_ref())
        .ok_or("personnel requires its actual candidate Repair owner")?;
    origin.state.validate(profile, jobs, tick)
}

pub(super) fn validate_roster(
    world: &World,
    snapshot: &PhoenixSnapshot,
) -> Result<(), (String, String)> {
    let fail = |actor: &str, reason: String| (actor.to_owned(), reason);
    let known = world.get_resource::<PersonnelRoster>();
    let mut profiles = BTreeMap::new();
    for row in &snapshot.entities {
        let profile = if let Some(origin) = &row.spawn {
            let config = origin
                .resolve(
                    &crate::entities::loader::WasmTemplateLoader,
                    &mut Vec::new(),
                )
                .ok_or_else(|| fail(&row.uuid, "cannot resolve personnel actor recipe".into()))?;
            let profile = phoenix_sim_gameplay::personnel::compile_entity(&config)
                .map_err(|reason| fail(&row.uuid, reason))?;
            if live_actor(world, &row.uuid)
                .and_then(|entity| world.get::<CompiledPersonnel>(entity))
                .is_some_and(|live| Some(live) != profile.as_ref())
            {
                return Err(fail(
                    &row.uuid,
                    "personnel recipe changed an existing accepted actor".into(),
                ));
            }
            if let Some(profile) = &profile {
                let source = crate::console::repair::personnel::source(origin, &row.uuid)
                    .map_err(|reason| fail(&row.uuid, reason))?;
                if snapshot
                    .personnel
                    .origins
                    .get(&row.uuid)
                    .is_none_or(|saved| saved.source.as_ref() != Some(&source))
                {
                    return Err(fail(
                        &row.uuid,
                        "personnel source differs from actual actor recipe".into(),
                    ));
                }
                profiles.insert(row.uuid.clone(), profile.clone());
            }
            profile
        } else {
            live_actor(world, &row.uuid)
                .and_then(|entity| world.get::<CompiledPersonnel>(entity).cloned())
        };
        if let Some(profile) = profile {
            let saved = snapshot.personnel.origins.get(&row.uuid).ok_or_else(|| {
                fail(
                    &row.uuid,
                    "required global personnel origin is missing".into(),
                )
            })?;
            if !world.contains_resource::<SimTick>() {
                return Err(fail(
                    &row.uuid,
                    "personnel requires the actual logical clock".into(),
                ));
            }
            validate_active(saved, &profile, row, snapshot.tick)
                .map_err(|reason| fail(&row.uuid, reason))?;
            profiles.insert(row.uuid.clone(), profile);
        } else if row
            .repair
            .as_ref()
            .and_then(|repair| repair.jobs.as_ref())
            .is_some_and(|jobs| {
                jobs.teams.iter().flatten().any(|job| {
                    matches!(
                        job.activity,
                        crate::jobs::repair::RepairActivity::Treatment { .. }
                    )
                })
            })
        {
            return Err(fail(
                &row.uuid,
                "treatment has no accepted personnel owner".into(),
            ));
        }
    }
    // Boot-authored accepted populations cannot disappear from a continuation,
    // even when the corresponding actor was genuinely removed before capture.
    if let Some(known) = known {
        for (actor, origin) in &known.origins {
            if origin.source.is_none() && !snapshot.personnel.origins.contains_key(actor) {
                return Err(fail(
                    actor,
                    "snapshot omitted an accepted authored personnel population".into(),
                ));
            }
        }
    }
    for (actor, origin) in &snapshot.personnel.origins {
        if actor.trim().is_empty() || origin.accepted_tick > snapshot.tick {
            return Err(fail(
                actor,
                "personnel origin identity or clock is invalid".into(),
            ));
        }
        if let Some(source) = &origin.source {
            use crate::world_id::{IdNamespace, WorldId, SEQ_LIMIT};
            let receipt = WorldId::new(
                IdNamespace::Entity,
                source.birth_tick,
                source.birth_sequence,
            );
            let mint = snapshot.mint.as_ref().ok_or_else(|| {
                fail(
                    actor,
                    "runtime personnel requires its actual captured mint frontier".into(),
                )
            })?;
            if source.birth_sequence >= SEQ_LIMIT
                || receipt.render() != *actor
                || WorldId::parse(actor) != Some(receipt)
                || source.birth_tick != origin.accepted_tick
                || source.birth_tick > mint.tick
                || mint.tick > snapshot.tick
                || mint.next_seq.iter().any(|next| *next > SEQ_LIMIT)
                || (source.birth_tick == mint.tick
                    && source.birth_sequence >= mint.next_seq[IdNamespace::Entity.code() as usize])
            {
                return Err(fail(
                    actor,
                    "personnel birth receipt differs from actual origin identity or mint frontier"
                        .into(),
                ));
            }
            if source_definition(source).map_err(|reason| fail(actor, reason))? != origin.definition
            {
                return Err(fail(
                    actor,
                    "retained personnel definition differs from accepted source".into(),
                ));
            }
        } else if origin.accepted_tick != 0
            || !known
                .and_then(|known| known.origins.get(actor))
                .is_some_and(|accepted| {
                    accepted.source.is_none() && accepted.definition == origin.definition
                })
        {
            return Err(fail(
                actor,
                "personnel origin has no accepted authored provenance".into(),
            ));
        }
        if known
            .and_then(|known| known.origins.get(actor))
            .is_some_and(|accepted| {
                accepted.definition != origin.definition || accepted.source != origin.source
            })
        {
            return Err(fail(
                actor,
                "personnel immutable origin provenance changed".into(),
            ));
        }
        if profiles.contains_key(actor) {
            continue;
        }
        let removed = origin
            .removed_tick
            .ok_or_else(|| fail(actor, "living personnel origin has no actual actor".into()))?;
        if snapshot.entities.iter().any(|row| &row.uuid == actor)
            || removed < origin.accepted_tick
            || removed > snapshot.tick
        {
            return Err(fail(
                actor,
                "removed personnel origin conflicts with actual actor or clock".into(),
            ));
        }
        let expected: std::collections::BTreeSet<_> = origin
            .definition
            .config
            .officers
            .iter()
            .map(|officer| &officer.id)
            .collect();
        if origin
            .state
            .officers
            .keys()
            .collect::<std::collections::BTreeSet<_>>()
            != expected
            || origin.state.officers.values().any(|state| {
                state.location
                    != (OfficerLocation::Lost {
                        actor: actor.clone(),
                    })
                    || state.changed_tick != removed
            })
        {
            return Err(fail(
                actor,
                "removed origin must retain its exact finite terminal population".into(),
            ));
        }
    }
    Ok(())
}

/// A machinery row has no personnel payload. It may only be applied when its
/// candidate jobs remain consistent with the current, unreplayed global custody.
pub(super) fn validate_live(
    world: &World,
    entity: Entity,
    row: &EntityState,
    tick: u64,
) -> Result<(), String> {
    match (
        world.get::<CompiledPersonnel>(entity),
        world
            .get_resource::<PersonnelRoster>()
            .and_then(|roster| roster.origins.get(&row.uuid)),
    ) {
        (Some(profile), Some(origin)) => validate_active(origin, profile, row, tick),
        (None, None) => Ok(()),
        _ => Err("single-row restore cannot replace global personnel identity".into()),
    }
}
pub(super) fn apply(world: &mut World, snapshot: &PhoenixSnapshot) {
    world.insert_resource(snapshot.personnel.clone());
}

#[cfg(test)]
#[path = "personnel_tests.rs"]
mod tests;
