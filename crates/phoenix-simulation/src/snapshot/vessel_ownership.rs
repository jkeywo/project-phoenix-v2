//! Acquisition history validation precedes all restore writes. Current Human,
//! GM, damage and faction eligibility is intentionally not historical evidence.
use super::*;
use crate::security::acquisition::AcquisitionTargetDefinition;
use phoenix_model::vessel_ownership::VesselAcquisitionCause;
use std::collections::BTreeMap;

fn live_entity(world: &World, uuid: &str) -> Option<Entity> {
    world
        .try_query_filtered::<(Entity, &EntityUuid), Allow<bevy::ecs::entity_disabling::Disabled>>()
        .and_then(|mut query| {
            query
                .iter(world)
                .find_map(|(entity, id)| (id.0 == uuid).then_some(entity))
        })
}
fn accepted_actions(
    world: &World,
    live: Option<Entity>,
    row: &EntityState,
) -> Result<Option<crate::security::SecurityTargetConfig>, String> {
    if let Some(origin) = &row.spawn {
        let mut warnings = Vec::new();
        let config = origin
            .resolve(&crate::entities::loader::WasmTemplateLoader, &mut warnings)
            .ok_or("cannot resolve acquisition target recipe")?;
        if !warnings.is_empty() {
            return Err("acquisition recipe is not exact".into());
        }
        return Ok(config.security_target);
    }
    Ok(live
        .and_then(|entity| world.get::<crate::security::SecurityTargetActions>(entity))
        .map(|target| target.0.clone()))
}
fn validate_row(
    world: &World,
    live: Option<Entity>,
    row: &EntityState,
    tick: u64,
) -> Result<(), String> {
    let Some(owner) = &row.operational_owner else {
        return Ok(());
    };
    owner.validate(&row.uuid, tick)?;
    let offered =
        accepted_actions(world, live, row)?.ok_or("acquisition has no accepted target action")?;
    if offered
        .action(crate::security::SecurityAction::Board)
        .and_then(|action| action.acquisition.as_ref())
        .is_none_or(|config| config.doctrine != owner.doctrine)
    {
        return Err("acquisition doctrine differs from immutable accepted opt-in".into());
    }
    if row.spawn.is_none()
        && live.is_none_or(|entity| world.get::<AcquisitionTargetDefinition>(entity).is_none())
    {
        return Err("acquisition target lost its accepted uncrewed recipe".into());
    }
    if world
        .get_resource::<crate::entities::config_cache::FactionRegistryResource>()
        .is_none_or(|registry| registry.0.get(&owner.faction).is_none())
    {
        return Err("acquired frozen faction is not registered".into());
    }
    let config = world
        .get_resource::<crate::world::config::WorldConfig>()
        .ok_or("acquisition has no accepted World")?;
    let profile = config
        .gm_npc_doctrine_palette
        .iter()
        .find(|profile| profile.id == owner.doctrine)
        .ok_or("acquisition has no original authored post-acquisition doctrine")?;
    let runtime = world
        .get_resource::<crate::world::server::WorldContentRuntime>()
        .ok_or("acquisition has no World source bindings")?;
    if profile.origin_layer.is_some()
        || profile.doctrine.is_empty()
        || !profile.targets.iter().any(|target| {
            target == &row.uuid || runtime.name_to_uuid.get(target) == Some(&row.uuid)
        })
        || !profile.doctrine.iter().all(|objective| {
            matches!(
                crate::ai::core::parse_doctrine_directive(objective),
                Ok(crate::core::messages::AiDirective::None)
            )
        })
    {
        return Err("acquisition doctrine is not the supported explicit autonomous hold".into());
    }
    Ok(())
}
pub(super) fn validate_entity(
    world: &World,
    entity: Entity,
    row: &EntityState,
    tick: u64,
) -> Result<(), String> {
    validate_row(world, Some(entity), row, tick)
}
pub(super) fn validate_roster(
    world: &World,
    snapshot: &PhoenixSnapshot,
) -> Result<(), (String, String)> {
    let mut scopes = BTreeMap::new();
    let mut target_history = BTreeMap::new();
    for row in &snapshot.entities {
        validate_row(world, live_entity(world, &row.uuid), row, snapshot.tick)
            .map_err(|reason| (row.uuid.clone(), reason))?;
        if let Some(owner) = &row.operational_owner {
            target_history.insert(row.uuid.clone(), owner);
            let VesselAcquisitionCause::SecurityBoard { scope, team, .. } = &owner.cause;
            let key = (scope.actor.clone(), scope.owner.clone(), scope.job);
            if scopes.insert(key, (&row.uuid, owner)).is_some() {
                return Err((
                    row.uuid.clone(),
                    "one Security job cannot acquire multiple vessels".into(),
                ));
            }
            if let Some(actor) = snapshot
                .entities
                .iter()
                .find(|actor| actor.uuid == owner.controller)
            {
                let security = actor.security.as_ref().ok_or_else(|| {
                    (
                        row.uuid.clone(),
                        "acquiring actor lost its retained Security owner".into(),
                    )
                })?;
                if security.jobs.owner != scope.owner || security.jobs.next_id <= scope.job.0 {
                    return Err((
                        row.uuid.clone(),
                        "acquisition cause exceeds actual Security job mint".into(),
                    ));
                }
                if let Some(job) = security
                    .jobs
                    .teams
                    .get(*team as usize)
                    .and_then(Option::as_ref)
                {
                    if job.id == scope.job && job.activity.acquisition.as_ref() != Some(owner) {
                        return Err((
                            row.uuid.clone(),
                            "retained Security job contradicts acquisition cause".into(),
                        ));
                    }
                } else {
                    return Err((
                        row.uuid.clone(),
                        "acquisition names an absent finite Security team".into(),
                    ));
                }
            }
        }
    }
    // Retained physical results have the reverse exact binding too. Removal of
    // the vessel is allowed; it never reassigns its equipment or personnel.
    for actor in &snapshot.entities {
        for job in actor
            .security
            .iter()
            .flat_map(|security| security.jobs.teams.iter().flatten())
        {
            if let Some(owner) = &job.activity.acquisition {
                let VesselAcquisitionCause::SecurityBoard { target, scope, .. } = &owner.cause;
                if world
                    .get_resource::<crate::entities::config_cache::FactionRegistryResource>()
                    .is_none_or(|registry| registry.0.get(&owner.faction).is_none())
                {
                    return Err((
                        actor.uuid.clone(),
                        "retained acquisition faction is not registered".into(),
                    ));
                }
                if target_history
                    .insert(target.clone(), owner)
                    .is_some_and(|prior| prior != owner)
                {
                    return Err((
                        actor.uuid.clone(),
                        "conflicting acquisition histories for one vessel".into(),
                    ));
                }
                if owner.controller != actor.uuid
                    || scope.actor != actor.uuid
                    || snapshot
                        .entities
                        .iter()
                        .find(|row| &row.uuid == target)
                        .is_some_and(|row| row.operational_owner.as_ref() != Some(owner))
                {
                    return Err((
                        actor.uuid.clone(),
                        "retained Security acquisition has no matching operational owner".into(),
                    ));
                }
            }
        }
    }
    Ok(())
}
