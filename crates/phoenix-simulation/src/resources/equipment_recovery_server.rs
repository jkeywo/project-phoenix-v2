//! Intact salvage is work by the fitted Recovery tool. The same actual assembly
//! changes owner once; there is no loot roll, crew teleport or second inventory.
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

use crate::core::messages::{AdmittedCommands, SystemControlPayload as Control, SystemId};
use crate::entities::spawner::EntityUuid;
use crate::jobs::Job;
use crate::resources::fitting::{CompiledFitting, EquipmentLocation};
use crate::resources::fitting_server::{CompiledShipFitting, ShipFitting};
use crate::resources::recovery_server::CompiledShipRecovery;
use crate::ship::compartment_damage::ShipDamage;
use crate::ship::power::ShipPowerSystem;
use crate::ship_plugin::{ShipConfigComponent, ShipSystemControlSources};
use phoenix_model::equipment_recovery_ui::{EquipmentRecoveryOption, EquipmentRecoveryView};
use phoenix_model::job::{JobId, JobOutcome};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EquipmentRecoveryWork {
    pub target_uuid: String,
    pub source_slot: String,
    pub source_system: SystemId,
    pub source_component: String,
    pub assembly: String,
    pub definition: String,
    pub destination_locker: String,
    pub work_secs: f32,
    pub progress: f32,
    pub waiting: String,
}

#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShipEquipmentRecovery {
    pub owner: SystemId,
    pub next_id: u64,
    pub job: Option<Job<EquipmentRecoveryWork>>,
    pub last_tick: Option<u64>,
}

pub(crate) fn install(
    config: &crate::entities::config::EntityConfig,
    commands: &mut EntityCommands,
) {
    if let Some(tool) = &config.recovery {
        commands.insert(ShipEquipmentRecovery {
            owner: tool.system.clone(),
            next_id: 1,
            job: None,
            last_tick: None,
        });
    }
}

pub(crate) fn register(app: &mut App) {
    use crate::authoritative::{DeclareState, StateClass};
    app.declare_state::<ShipEquipmentRecovery>(
        StateClass::Folded,
        "equipment-fitting-and-refitting",
    );
    // The existing Recovery consumer admits these closed payloads. It remains
    // the single controller; explicit equipment selection owns its work budget.
    app.add_systems(
        FixedUpdate,
        tick.in_set(crate::sim_sets::SimSet::Modifiers)
            .after(crate::infrastructure::server::tick_infrastructure_condition)
            .after(crate::resources::recovery_server::tick)
            .before(crate::resources::server::tick_resource_transfers),
    );
}

pub fn active(state: &ShipEquipmentRecovery) -> bool {
    state.job.as_ref().is_some_and(|job| job.outcome.is_none())
}

/// A real receiving-space claim, shared with ordinary fitting preparation.
pub fn reserved_space(
    state: &ShipEquipmentRecovery,
    locker: &str,
    profile: &CompiledFitting,
) -> u64 {
    state
        .job
        .as_ref()
        .filter(|job| job.outcome.is_none() && job.activity.destination_locker == locker)
        .and_then(|job| profile.equipment(&job.activity.definition).ok())
        .map_or(0, |definition| u64::from(definition.storage_space))
}

pub fn source_claimed(world: &World, source_uuid: &str, system: &SystemId) -> bool {
    world
        .try_query::<&ShipEquipmentRecovery>()
        .is_some_and(|mut query| {
            query.iter(world).any(|state| {
                state.job.as_ref().is_some_and(|job| {
                    job.outcome.is_none()
                        && job.activity.target_uuid == source_uuid
                        && &job.activity.source_system == system
                })
            })
        })
}

fn find(world: &World, uuid: &str) -> Option<Entity> {
    world
        .try_query::<(Entity, &EntityUuid)>()?
        .iter(world)
        .find_map(|(entity, id)| (id.0 == uuid).then_some(entity))
}

fn known(world: &World, actor: Entity, retained: Option<&str>) -> BTreeSet<String> {
    picture(world, actor).map_or_else(BTreeSet::new, |picture| picture.known_for_work(retained))
}

fn picture(
    world: &World,
    actor: Entity,
) -> Option<crate::resources::recovery_server::contacts::Picture> {
    let tool = world.get::<CompiledShipRecovery>(actor)?;
    let config = world.get::<ShipConfigComponent>(actor)?;
    let boards = world.get::<crate::server_app::ShipSystemBlackboards>(actor)?;
    let observer = world.get::<EntityUuid>(actor)?;
    Some(
        crate::resources::recovery_server::contacts::Picture::capture(
            &config.0,
            &tool.0.system,
            &boards.0,
            world.get::<crate::science::broad::BroadTracks>(actor),
            world.get::<crate::science::broad::SensingControls>(actor),
            &observer.0,
            world.get_resource::<crate::world::server::WorldContentRuntime>(),
        ),
    )
}

fn observed_in_range(
    world: &World,
    actor: Entity,
    contact: &crate::resources::recovery_server::contacts::Contact,
) -> bool {
    let Some(position) = contact.position else {
        // Existing reduced fixtures without sensing use the physical check in prepare.
        return true;
    };
    let (Some(origin), Some(tool)) = (
        world.get::<Transform>(actor),
        world.get::<CompiledShipRecovery>(actor),
    ) else {
        return false;
    };
    let distance = (f64::from(origin.translation.x) - f64::from(position[0]))
        .hypot(f64::from(origin.translation.z) - f64::from(position[1]));
    distance.is_finite() && distance <= f64::from(tool.0.range)
}

fn exact_component(
    target: &phoenix_model::damage::RepairWorkTarget,
    system: &SystemId,
    component: &str,
) -> bool {
    matches!(target, phoenix_model::damage::RepairWorkTarget::Component { system: target_system, component: target_component }
        if target_system == system && target_component == component)
}

fn locally_claimed(world: &World, source: Entity, system: &SystemId, component: &str) -> bool {
    let Some(teams) = world.get::<crate::console::repair::server::ShipRepairTeams>(source) else {
        return false;
    };
    if teams.0.claims_personnel_bay(system) {
        return true;
    }
    let state = teams.0.save_state();
    let active = |id| {
        state
            .teams
            .iter()
            .flatten()
            .any(|job| job.id.0 == id && job.outcome.is_none())
    };
    state.teams.iter().flatten().any(|job| job.outcome.is_none()
        && matches!(&job.activity, crate::jobs::repair::RepairActivity::Refit { target, .. } if target == system))
        || state.physical_work.iter().any(|(id,work)| active(*id)
            && work.target.iter().chain(work.requested_target.iter()).any(|target| exact_component(target, system, component)))
}

fn foreign_component_claimed(
    world: &World,
    source_uuid: &str,
    system: &SystemId,
    component: &str,
) -> bool {
    world.try_query::<&crate::console::repair::server::ShipRepairTeams>().is_some_and(|mut query|
        query.iter(world).any(|teams| {
            let state = teams.0.save_state();
            state.external_physical_work.iter().any(|(id,work)| work.target_uuid == source_uuid
                && work.target.as_ref().is_some_and(|target| exact_component(target, system, component))
                && state.teams.iter().flatten().any(|job| job.id.0 == *id && job.outcome.is_none()
                    && matches!(&job.activity, crate::jobs::repair::RepairActivity::External { target } if target == source_uuid)))
        }))
}

fn available_space(
    profile: &CompiledFitting,
    state: &ShipFitting,
    locker: &str,
) -> Result<u64, String> {
    let capacity = u64::from(profile.locker(locker)?.space);
    let mut used = 0u64;
    for item in state.equipment.assemblies.values().filter(|item| matches!(&item.location, EquipmentLocation::Stored { locker: current } if current == locker)) {
        used = used.checked_add(u64::from(profile.equipment(&item.definition)?.storage_space)).ok_or("equipment space overflow")?;
    }
    used = used
        .checked_add(crate::resources::fitting_server::reserved_space(
            profile, state, locker,
        ))
        .ok_or("equipment claim overflow")?;
    Ok(capacity.saturating_sub(used))
}

struct Prepared {
    source: Entity,
    source_fitting: ShipFitting,
    destination_fitting: ShipFitting,
    damage: ShipDamage,
    source_power: Option<ShipPowerSystem>,
    effective: crate::entities::config::EntityConfig,
}

/// Dry-run every actual owner before charging the tool or committing a stage.
fn prepare(
    world: &World,
    actor: Entity,
    work: &EquipmentRecoveryWork,
    id: JobId,
    retained: bool,
) -> Result<Prepared, String> {
    let actor_uuid = &world
        .get::<EntityUuid>(actor)
        .ok_or("missing recovery ship")?
        .0;
    if work.target_uuid == *actor_uuid
        || !known(world, actor, retained.then_some(work.target_uuid.as_str()))
            .contains(&work.target_uuid)
    {
        return Err("unknown_target".into());
    }
    let source = find(world, &work.target_uuid).ok_or("source_unavailable")?;
    let tool = &world
        .get::<CompiledShipRecovery>(actor)
        .ok_or("no_recovery_tool")?
        .0;
    let distance = world
        .get::<Transform>(actor)
        .zip(world.get::<Transform>(source))
        .map_or(f32::INFINITY, |(a, b)| {
            a.translation.distance(b.translation)
        });
    if !distance.is_finite() || distance > tool.range {
        return Err("out_of_range".into());
    }
    let source_profile = &world
        .get::<CompiledShipFitting>(source)
        .ok_or("unsupported_source")?
        .0;
    let destination_profile = &world
        .get::<CompiledShipFitting>(actor)
        .ok_or("no_equipment_storage")?
        .0;
    let slot = source_profile.slot(&work.source_slot)?;
    if slot.system != work.source_system || slot.operation_component != work.source_component {
        return Err("source_slot_changed".into());
    }
    let source_state = world
        .get::<ShipFitting>(source)
        .ok_or("source_unavailable")?;
    let mut destination_state = world
        .get::<ShipFitting>(actor)
        .ok_or("no_equipment_storage")?
        .clone();
    let item = source_state
        .equipment
        .installed(&work.source_slot)
        .ok_or("source_vacant")?;
    if item.id != work.assembly || item.definition != work.definition {
        return Err("source_equipment_changed".into());
    }
    let definition = source_profile.equipment(&work.definition)?;
    if destination_profile.equipment(&work.definition)? != definition
        || definition.work.work_secs != work.work_secs
    {
        return Err("incompatible_equipment_definition".into());
    }
    let tool_room = world
        .get::<crate::ship::access::InteriorLayout>(actor)
        .and_then(|layout| layout.0.placements.get(&tool.system))
        .ok_or("no_recovery_compartment")?;
    if &destination_profile
        .locker(&work.destination_locker)?
        .compartment
        != tool_room
    {
        return Err("destination_not_at_tool".into());
    }
    if available_space(
        destination_profile,
        &destination_state,
        &work.destination_locker,
    )? < u64::from(definition.storage_space)
    {
        return Err("no_room".into());
    }
    let mut damage = world
        .get::<ShipDamage>(source)
        .ok_or("source_has_no_physical_condition")?
        .clone();
    let config = &world
        .get::<ShipConfigComponent>(source)
        .ok_or("source_has_no_topology")?
        .0;
    if !damage.neutralized(config, world.get::<ShipPowerSystem>(source)) {
        return Err("source_active".into());
    }
    if source_state.work.contains_key(&work.source_slot)
        || locally_claimed(world, source, &slot.system, &slot.operation_component)
        || foreign_component_claimed(
            world,
            &work.target_uuid,
            &slot.system,
            &slot.operation_component,
        )
    {
        return Err("physical_task_claimed".into());
    }
    if slot.system.0.strip_prefix("phaser-").is_some_and(|bank| {
        world
            .get::<crate::console::weapons::ActiveBeam>(source)
            .is_some_and(|beams| beams.is_bank_firing(bank))
    }) {
        return Err("safe_shutdown".into());
    }
    if world
        .try_query::<(Entity, &ShipEquipmentRecovery)>()
        .is_some_and(|mut query| {
            query.iter(world).any(|(other, state)| {
                other != actor
                    && state.job.as_ref().is_some_and(|job| {
                        job.outcome.is_none()
                            && job.activity.target_uuid == work.target_uuid
                            && job.activity.source_system == work.source_system
                    })
            })
        })
    {
        return Err("physical_task_claimed".into());
    }
    if destination_state
        .equipment
        .assemblies
        .contains_key(&work.assembly)
    {
        return Err("duplicate_equipment_identity".into());
    }
    let condition = damage
        .state
        .components
        .get_mut(&slot.system)
        .and_then(|rows| rows.get_mut(&slot.operation_component))
        .ok_or("source_operation_missing")?;
    if condition.condition == phoenix_model::damage::ComponentCondition::Destroyed {
        return Err("equipment_destroyed".into());
    }
    if definition.work.class != crate::resources::fitting::RefitClass::Field {
        return Err("major_equipment_requires_facility".into());
    }
    let mut source_state = source_state.clone();
    source_state.equipment.remove_installed(
        source_profile,
        &work.source_slot,
        id.0,
        condition.clone(),
    )?;
    let mut item = source_state
        .equipment
        .assemblies
        .remove(&work.assembly)
        .ok_or("source_equipment_missing")?;
    item.location = EquipmentLocation::Stored {
        locker: work.destination_locker.clone(),
    };
    destination_state
        .equipment
        .assemblies
        .insert(item.id.clone(), item);
    source_state.equipment.validate(source_profile)?;
    destination_state.equipment.validate(destination_profile)?;
    *condition = phoenix_model::damage::ComponentState {
        emp: None,
        condition: phoenix_model::damage::ComponentCondition::Healthy,
        dose: 0.0,
        last_source: None,
    };
    damage.state.validate(&damage.definition)?;
    let mut source_power = world.get::<ShipPowerSystem>(source).cloned();
    let effective = crate::resources::fitting_server::stage_capability(
        source_profile,
        &source_state.equipment,
        source_power.as_mut(),
    )?;
    Ok(Prepared {
        source,
        source_fitting: source_state,
        destination_fitting: destination_state,
        damage,
        source_power,
        effective,
    })
}

fn start(
    world: &mut World,
    actor: Entity,
    owner: &SystemId,
    target: &str,
    slot: &str,
    locker: &str,
) -> Result<(), String> {
    let mut state = world
        .get::<ShipEquipmentRecovery>(actor)
        .ok_or("no_recovery_tool")?
        .clone();
    if &state.owner != owner || active(&state) || state.next_id == u64::MAX {
        return Err("recovery_tool_busy".into());
    }
    let picture = picture(world, actor).ok_or("unknown_target")?;
    let contact = picture.resolve(target).ok_or("unknown_target")?;
    if !observed_in_range(world, actor, contact) {
        return Err("out_of_range".into());
    }
    let target = contact.source.as_str();
    let source = find(world, target).ok_or("source_unavailable")?;
    let profile = &world
        .get::<CompiledShipFitting>(source)
        .ok_or("unsupported_source")?
        .0;
    let source_state = world
        .get::<ShipFitting>(source)
        .ok_or("source_unavailable")?;
    let assembly = source_state
        .equipment
        .installed(slot)
        .ok_or("source_vacant")?;
    let work = EquipmentRecoveryWork {
        target_uuid: target.into(),
        source_slot: slot.into(),
        source_system: profile.slot(slot)?.system.clone(),
        source_component: profile.slot(slot)?.operation_component.clone(),
        assembly: assembly.id.clone(),
        definition: assembly.definition.clone(),
        destination_locker: locker.into(),
        work_secs: profile.equipment(&assembly.definition)?.work.work_secs,
        progress: 0.0,
        waiting: String::new(),
    };
    prepare(world, actor, &work, JobId(state.next_id), false)?;
    let mut machinery = world
        .get::<crate::resources::recovery_server::ShipRecovery>(actor)
        .filter(|machinery| machinery.0.owner == *owner)
        .cloned()
        .ok_or("no_recovery_machinery")?;
    // Explicit selection is the same owner's start command. Subsequent AI
    // decisions continue to own this ordinary on/off control.
    machinery.0.set_enabled(true);
    state.job = Some(Job {
        id: JobId(state.next_id),
        owner: owner.clone(),
        activity: work,
        runtime: Default::default(),
        execution_tick: 0,
        outcome: None,
        paused: false,
        workflow_error: false,
    });
    state.next_id += 1;
    world.entity_mut(actor).insert((state, machinery));
    Ok(())
}

fn cancel(world: &mut World, actor: Entity, owner: &SystemId) -> Result<(), String> {
    let mut state = world
        .get::<ShipEquipmentRecovery>(actor)
        .ok_or("no_recovery_tool")?
        .clone();
    if &state.owner != owner {
        return Err("foreign_recovery_tool".into());
    }
    let job = state
        .job
        .as_mut()
        .filter(|job| job.outcome.is_none())
        .ok_or("no_equipment_recovery")?;
    job.outcome = Some(JobOutcome::Cancelled);
    job.paused = false;
    job.activity.waiting.clear();
    let mut machinery = world
        .get::<crate::resources::recovery_server::ShipRecovery>(actor)
        .filter(|machinery| &machinery.0.owner == owner)
        .cloned()
        .ok_or("no_recovery_machinery")?;
    machinery.0.set_enabled(false);
    world.entity_mut(actor).insert((state, machinery));
    Ok(())
}

fn advance(world: &mut World, actor: Entity, now: u64, dt: f32) {
    let Some(mut state) = world.get::<ShipEquipmentRecovery>(actor).cloned() else {
        return;
    };
    if state.last_tick.is_some_and(|tick| now <= tick) || !dt.is_finite() || dt <= 0.0 {
        return;
    }
    state.last_tick = Some(now);
    let Some(job) = state.job.as_mut().filter(|job| job.outcome.is_none()) else {
        world.entity_mut(actor).insert(state);
        return;
    };
    let prepared = prepare(world, actor, &job.activity, job.id, true);
    let allowed = world
        .get::<ShipSystemControlSources>(actor)
        .is_some_and(|sources| {
            let policy = sources.0.policy_for(&state.owner);
            policy.accept_human_input || policy.operate_ai
        });
    let factor = world
        .get::<ShipDamage>(actor)
        .filter(|damage| !damage.catastrophic())
        .map_or(0.0, |damage| {
            damage.factor(
                &state.owner,
                phoenix_model::damage::DamageBinding::Operation,
            )
        });
    let mut power = world.get::<ShipPowerSystem>(actor).cloned();
    let enabled = world
        .get::<crate::resources::recovery_server::ShipRecovery>(actor)
        .is_some_and(|machinery| machinery.0.owner == state.owner && machinery.0.enabled);
    let result = if !allowed || !enabled || factor <= 0.0 {
        Err("recovery_unavailable".to_owned())
    } else {
        prepared
    };
    match result {
        Err(reason) => {
            job.activity.waiting = waiting_reason(reason);
            job.paused = true;
        }
        Ok(prepared) => {
            let seconds =
                dt.min((job.activity.work_secs - job.activity.progress).max(0.0) / factor);
            let fraction = power
                .as_mut()
                .and_then(|power| power.0.work_continuous(&state.owner, seconds, 1.0).ok())
                .map_or(0.0, |grant| grant.fraction * factor);
            job.activity.progress =
                (job.activity.progress + seconds * fraction).min(job.activity.work_secs);
            job.paused = fraction <= 0.0;
            job.activity.waiting = if job.paused {
                "no_power".into()
            } else {
                String::new()
            };
            if job.activity.progress >= job.activity.work_secs {
                // All staged owners were validated before payment. Installation
                // is absent: the destination receives the actual stored item.
                crate::resources::fitting_server::apply_capability(
                    world,
                    prepared.source,
                    &prepared.effective,
                );
                world
                    .entity_mut(prepared.source)
                    .insert((prepared.source_fitting, prepared.damage));
                if let Some(power) = prepared.source_power {
                    world.entity_mut(prepared.source).insert(power);
                }
                world.entity_mut(actor).insert(prepared.destination_fitting);
                job.outcome = Some(JobOutcome::Completed);
                job.paused = false;
                if let Some(mut machinery) =
                    world.get_mut::<crate::resources::recovery_server::ShipRecovery>(actor)
                {
                    machinery.0.set_enabled(false);
                }
            }
        }
    }
    world.entity_mut(actor).insert(state);
    if let Some(power) = power {
        world.entity_mut(actor).insert(power);
    }
}

fn tick(world: &mut World) {
    let now = world
        .get_resource::<crate::sim_tick::SimTick>()
        .map_or(0, |tick| tick.0);
    let dt = world
        .get_resource::<Time<Fixed>>()
        .map_or(0.0, |time| time.delta_secs());
    let mut query = world.query::<(
        Entity,
        &EntityUuid,
        &ShipEquipmentRecovery,
        &AdmittedCommands,
    )>();
    let mut actors: Vec<_> = query
        .iter(world)
        .map(|(entity, uuid, _, commands)| (uuid.0.clone(), entity, commands.0.clone()))
        .collect();
    actors.sort_by(|a, b| a.0.cmp(&b.0));
    for (_, actor, commands) in actors {
        for command in commands {
            let result = match &command.payload {
                Control::RecoverEquipment {
                    target_uuid,
                    slot,
                    destination_locker,
                } => start(
                    world,
                    actor,
                    &command.target,
                    target_uuid,
                    slot,
                    destination_locker,
                ),
                Control::CancelEquipmentRecovery {} => cancel(world, actor, &command.target),
                _ => continue,
            };
            if let Some(feedback) = crate::command_admission::feedback::admitted_action_feedback(
                &command,
                if result.is_ok() {
                    crate::core::messages::ActionFeedbackOutcome::Applied
                } else {
                    crate::core::messages::ActionFeedbackOutcome::Refused
                },
            ) {
                if let Some(mut outbound) =
                    world.get_resource_mut::<Messages<crate::lobby::OutboundMessage>>()
                {
                    outbound.write(feedback);
                }
            }
        }
        advance(world, actor, now, dt);
    }
}

pub fn validate_state(
    state: &ShipEquipmentRecovery,
    tool: &crate::resources::recovery::RecoveryToolConfig,
    profile: Option<&CompiledFitting>,
    tick: u64,
) -> Result<(), String> {
    if state.owner != tool.system
        || state.next_id == 0
        || state.last_tick.is_some_and(|last| last > tick)
    {
        return Err("invalid equipment recovery owner/clock".into());
    }
    if let Some(job) = &state.job {
        let work = &job.activity;
        if job.owner != state.owner
            || job.id.0 == 0
            || job.id.0 >= state.next_id
            || job.runtime != Default::default()
            || job.execution_tick != 0
            || job.workflow_error
            || !work.progress.is_finite()
            || work.progress < 0.0
            || !work.work_secs.is_finite()
            || work.work_secs <= 0.0
            || work.progress > work.work_secs
            || [
                &work.target_uuid,
                &work.source_slot,
                &work.source_system.0,
                &work.source_component,
                &work.assembly,
                &work.definition,
                &work.destination_locker,
            ]
            .iter()
            .any(|id| id.is_empty() || id.len() > 128 || id.chars().any(char::is_control))
            || work.waiting.len() > 128
            || work.waiting.chars().any(char::is_control)
            || (job.outcome == Some(JobOutcome::Completed) && work.progress != work.work_secs)
            || (job.outcome.is_none()
                && (work.progress >= work.work_secs || job.paused != !work.waiting.is_empty()))
            || (job.outcome.is_some() && (job.paused || !work.waiting.is_empty()))
            || job.outcome == Some(JobOutcome::Failed)
            || (work.progress > 0.0 && state.last_tick.is_none())
        {
            return Err("invalid equipment recovery job".into());
        }
        let profile = profile.ok_or("equipment recovery has no actual receiving profile")?;
        let definition = profile.equipment(&work.definition)?;
        if definition.work.work_secs != work.work_secs
            || definition.work.class != crate::resources::fitting::RefitClass::Field
        {
            return Err("equipment recovery work recipe changed".into());
        }
        profile.locker(&work.destination_locker)?;
    }
    Ok(())
}

pub fn project(world: &World, actor: Entity) -> Option<EquipmentRecoveryView> {
    let owner = world.get::<ShipEquipmentRecovery>(actor)?;
    let profile = &world.get::<CompiledShipFitting>(actor)?.0;
    let picture = picture(world, actor)?;
    let selected = owner
        .job
        .as_ref()
        .and_then(|job| option(world, &job.activity, &picture));
    let mut options = Vec::new();
    if !active(owner) {
        for contact in picture
            .contacts
            .values()
            .filter(|contact| observed_in_range(world, actor, contact))
        {
            let uuid = &contact.source;
            let Some(source) = find(world, uuid) else {
                continue;
            };
            let (Some(source_profile), Some(source_state)) = (
                world.get::<CompiledShipFitting>(source),
                world.get::<ShipFitting>(source),
            ) else {
                continue;
            };
            for slot in &source_profile.0.config().slots {
                let Some(item) = source_state.equipment.installed(&slot.id) else {
                    continue;
                };
                for locker in &profile.config().lockers {
                    let Ok(definition) = source_profile.0.equipment(&item.definition) else {
                        continue;
                    };
                    let work = EquipmentRecoveryWork {
                        target_uuid: uuid.clone(),
                        source_slot: slot.id.clone(),
                        source_system: slot.system.clone(),
                        source_component: slot.operation_component.clone(),
                        assembly: item.id.clone(),
                        definition: item.definition.clone(),
                        destination_locker: locker.id.clone(),
                        work_secs: definition.work.work_secs,
                        progress: 0.0,
                        waiting: String::new(),
                    };
                    match prepare(world, actor, &work, JobId(owner.next_id), false) {
                        Ok(_) => {
                            if let Some(option) = option(world, &work, &picture) {
                                options.push(option);
                            }
                        }
                        Err(reason) if reason == "major_equipment_requires_facility" => {
                            if let Some(mut unavailable) = option(world, &work, &picture) {
                                unavailable.blocked_reason = Some(reason);
                                options.push(unavailable);
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    let job = owner.job.as_ref();
    Some(EquipmentRecoveryView {
        can_start: owner.next_id < u64::MAX
            && options.iter().any(|option| option.blocked_reason.is_none()),
        can_cancel: active(owner),
        options,
        selected,
        progress: job.map_or(0, |job| {
            ((job.activity.progress / job.activity.work_secs).clamp(0.0, 1.0) * 1000.0) as u16
        }),
        phase: job.map_or("idle".into(), |job| {
            job.outcome.map_or_else(
                || {
                    if job.paused {
                        job.activity.waiting.clone()
                    } else {
                        "working".into()
                    }
                },
                |outcome| {
                    match outcome {
                        JobOutcome::Completed => "completed",
                        JobOutcome::Cancelled => "cancelled",
                        JobOutcome::Failed => "failed",
                    }
                    .into()
                },
            )
        }),
    })
}

fn option(
    world: &World,
    work: &EquipmentRecoveryWork,
    picture: &crate::resources::recovery_server::contacts::Picture,
) -> Option<EquipmentRecoveryOption> {
    let handle = picture.handle_for_source(&work.target_uuid)?;
    let contact = picture.resolve(&handle)?;
    Some(EquipmentRecoveryOption {
        target_uuid: handle,
        target_name: if contact.position.is_some() {
            contact
                .name
                .clone()
                .unwrap_or_else(|| "console.common.unknown".into())
        } else {
            find(world, &work.target_uuid)
                .and_then(|source| world.get::<crate::entities::spawner::EntityName>(source))
                .map_or_else(|| "console.common.unknown".into(), |name| name.0.clone())
        },
        slot: work.source_slot.clone(),
        equipment: work.definition.clone(),
        destination_locker: work.destination_locker.clone(),
        blocked_reason: None,
    })
}

fn waiting_reason(reason: String) -> String {
    match reason.as_str() {
        "unknown_target"
        | "source_unavailable"
        | "out_of_range"
        | "no_room"
        | "source_active"
        | "physical_task_claimed"
        | "equipment_destroyed"
        | "recovery_unavailable"
        | "no_power"
        | "safe_shutdown"
        | "source_equipment_changed" => reason,
        _ => "unsupported_equipment".into(),
    }
}

#[cfg(test)]
#[path = "equipment_recovery_server_tests.rs"]
mod tests;
