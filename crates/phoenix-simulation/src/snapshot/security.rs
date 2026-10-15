//! Validate canonical Security commitments before any restore writes.
use super::*;
use bevy::ecs::entity_disabling::Disabled;
use bevy::ecs::query::Allow;
use std::collections::BTreeMap;

fn validate_row(
    row: &EntityState,
    config: Option<&crate::security::SecurityConfig>,
    targets: &BTreeMap<String, Option<crate::security::SecurityTargetConfig>>,
    tick: u64,
) -> Result<(), String> {
    let Some(state) = &row.security else {
        return Ok(());
    };
    let config = config.ok_or("Security jobs have no accepted finite team owner")?;
    state.validate_config(config, tick)?;
    for job in state.jobs.teams.iter().flatten() {
        let team = &job.activity.team;
        if !team.is_committed() {
            continue;
        }
        let Some(target) = team.target.as_ref() else {
            continue;
        };
        // Missing targets are legitimate interruption/return continuations. They
        // cannot emit a completion: the ordinary tick withdraws before work.
        let Some(target) = targets.get(target) else {
            continue;
        };
        let offered = target
            .as_ref()
            .and_then(|profile| team.action.and_then(|action| profile.action(action)))
            .ok_or("Security commitment has no matching accepted target action")?;
        if team.risk != offered.risk
            || (team.state == crate::security::SecurityTeamState::Working
                && team.phase_duration != offered.duration_secs)
        {
            return Err("Security work terms differ from the actual accepted target action".into());
        }
    }
    Ok(())
}

fn entity_for(world: &World, uuid: &str) -> Option<Entity> {
    world
        .try_query_filtered::<(Entity, &EntityUuid), Allow<Disabled>>()
        .and_then(|mut q| {
            q.iter(world)
                .find_map(|(entity, id)| (id.0 == uuid).then_some(entity))
        })
}

pub(super) fn validate_roster(
    world: &World,
    snapshot: &PhoenixSnapshot,
) -> Result<(), (String, String)> {
    let mut profiles = BTreeMap::new();
    let mut targets = BTreeMap::new();
    for row in &snapshot.entities {
        let live = entity_for(world, &row.uuid);
        let (profile, target) = if let Some(entity) = live {
            (
                world
                    .get::<crate::security::ShipSecurityTeams>(entity)
                    .map(|teams| teams.config.clone()),
                world
                    .get::<crate::security::SecurityTargetActions>(entity)
                    .map(|target| target.0.clone()),
            )
        } else if let Some(origin) = &row.spawn {
            let config = origin
                .resolve(
                    &crate::entities::loader::WasmTemplateLoader,
                    &mut Vec::new(),
                )
                .ok_or_else(|| {
                    (
                        row.uuid.clone(),
                        "cannot resolve Security equipment for restore".into(),
                    )
                })?;
            (config.security.clone(), config.security_target.clone())
        } else {
            (None, None)
        };
        profiles.insert(row.uuid.clone(), profile);
        targets.insert(row.uuid.clone(), target);
    }
    for row in &snapshot.entities {
        validate_row(
            row,
            profiles.get(&row.uuid).and_then(Option::as_ref),
            &targets,
            snapshot.tick,
        )
        .map_err(|reason| (row.uuid.clone(), reason))?;
    }
    Ok(())
}

pub(super) fn validate_live(
    world: &World,
    entity: Entity,
    row: &EntityState,
    tick: u64,
) -> Result<(), String> {
    let mut targets = BTreeMap::new();
    if let Some(mut q) = world.try_query_filtered::<(&EntityUuid, Option<&crate::security::SecurityTargetActions>), Allow<Disabled>>() {
        for (uuid, target) in q.iter(world) { targets.insert(uuid.0.clone(), target.map(|target| target.0.clone())); }
    }
    validate_row(
        row,
        world
            .get::<crate::security::ShipSecurityTeams>(entity)
            .map(|teams| &teams.config),
        &targets,
        tick,
    )
}

#[cfg(test)]
#[path = "security_tests.rs"]
mod tests;
