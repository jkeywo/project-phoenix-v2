//! Bridge real committed owner results into the existing pending World owner.
use crate::world::server::WorldContentRuntime;
#[path = "activity_communication.rs"]
mod communication;
pub(crate) use communication::{
    bind_communication_request, communication_request_release, communication_requests,
    observe_communication_receipts, prepare_communication_request,
    validate_planned_communication_reading,
};
use phoenix_model::{
    activity::{ActivityFact, ActivityJob, ActivityOutcome},
    identity::SystemId,
    job::JobId,
};
use phoenix_sim_world::world::{activities::ActivityRequest, content::WorldEvent};

/// Resolve only this actual acting ship at the ordinary consumer boundary.
/// Authored ship-slot identities are shared across peers; LocalShip is never
/// used to choose a situation actor.
pub(crate) fn arm_for_actor(
    runtime: &mut WorldContentRuntime,
    base: &crate::world::config::WorldConfig,
    actor: &str,
    slot: Option<&str>,
    tick: u64,
    supports: impl Fn(&ActivityRequest) -> bool,
) {
    for config in runtime.situation_configs.clone() {
        if runtime.situations.instances.contains_key(&config.id) {
            continue;
        }
        let requested = config.request.actor();
        if let Err(error) = base.validate_situation_actor(requested) {
            bevy::log::warn!(instance=%config.id,%error,"Unsupported activity binding refused before arming");
            continue;
        }
        let matches = base.name_to_uuid.get(requested).map_or_else(
            || {
                requested
                    .strip_prefix("slot:")
                    .is_some_and(|id| Some(id) == slot)
            },
            |uuid| uuid == actor,
        );
        if !matches {
            continue;
        }
        if !supports(&config.request) {
            bevy::log::warn!(instance=%config.id,actor,"Activity binding refused: request/required goal is outside actual supported equipment scope");
            continue;
        }
        let mut bound = config.clone();
        match &mut bound.request {
            ActivityRequest::Repair { actor: target, .. }
            | ActivityRequest::Delivery { actor: target, .. }
            | ActivityRequest::Communication { actor: target, .. } => *target = actor.into(),
        }
        if let Some(definition) = runtime
            .situation_definitions
            .iter()
            .find(|definition| definition.id == bound.definition)
        {
            if let Err(error) = runtime.situations.arm(&bound, definition, tick) {
                bevy::log::warn!(instance=%bound.id,%error,"Situation arming refused");
            }
        }
    }
}

pub(crate) fn record(
    runtime: &mut WorldContentRuntime,
    actor: &str,
    owner: &SystemId,
    tick: u64,
    outcomes: impl IntoIterator<Item = (JobId, ActivityOutcome)>,
) {
    for (job, outcome) in outcomes {
        let scope = ActivityJob {
            actor: actor.into(),
            owner: owner.clone(),
            job,
        };
        let Some(fact) = runtime.situations.fact(&scope, tick, outcome) else {
            continue;
        };
        retain_fact(runtime, fact);
    }
}

/// Same bounded pending owner for physical and communication achievements and
/// terminal results. First actual receipt wins; no repeated event refresh.
fn retain_fact(runtime: &mut WorldContentRuntime, fact: ActivityFact) {
    let occupied = runtime.pending_world_events.iter().any(|event| matches!(event,
        WorldEvent::Activity(existing) if existing.instance == fact.instance && existing.is_achievement() == fact.is_achievement()));
    if !occupied {
        runtime
            .pending_world_events
            .push(WorldEvent::Activity(fact));
    }
}

/// The bound pure World consumer acknowledges before the legacy collector can
/// replace its transient buffer. Private retained facts never enter that buffer.
pub(crate) fn acknowledge_pending(runtime: &mut WorldContentRuntime, now: u64) {
    let mut pending = std::mem::take(&mut runtime.pending_world_events);
    pending.retain(|event|match event {
        WorldEvent::Activity(fact)=>match runtime.situations.acknowledge(fact,now) {
            Ok(true)=>false,
            Ok(false)=>true,
            Err(error)=>{bevy::log::warn!(instance=%fact.instance,%error,"Activity fact retained without acknowledgement");true}
        },
        _=>true,
    });
    runtime.pending_world_events = pending;
}

pub(crate) fn is_activity(event: &WorldEvent) -> bool {
    matches!(event, WorldEvent::Activity(_))
}

pub(crate) fn repair_binding_supported(
    binding: &ActivityRequest,
    config: &crate::ship::config::ShipConfig,
    damage: &crate::ship::compartment_damage::ShipDamage,
    team_owner: &SystemId,
    team_count: usize,
) -> bool {
    let ActivityRequest::Repair {
        owner,
        team,
        request,
        target,
        goal,
        ..
    } = binding
    else {
        return false;
    };
    crate::jobs::activity_bindings::repair_supported(
        crate::jobs::activity_bindings::RepairBinding {
            owner,
            team: *team,
            request,
            target,
            goal: *goal,
        },
        config,
        &damage.definition,
        team_owner,
        team_count,
    )
}
pub(crate) fn delivery_binding_supported(
    binding: &ActivityRequest,
    profile: &crate::resources::logistics::CompiledLogistics,
) -> bool {
    let ActivityRequest::Delivery {
        owner,
        source,
        destination,
        commodity,
        amount,
        ..
    } = binding
    else {
        return false;
    };
    crate::jobs::activity_bindings::delivery_supported(
        profile,
        owner,
        source,
        destination,
        commodity,
        *amount,
    )
}

/// Candidate-only pending check reused by snapshot preflight. No live owner is
/// read or changed; original historical results are never compared to damage.
pub fn validate_pending(
    runtime: &phoenix_sim_world::world::activities::SituationInstances,
    pending: &[ActivityFact],
    now: u64,
) -> Result<(), String> {
    let mut candidate = runtime.clone();
    let mut keys = std::collections::BTreeSet::new();
    for fact in pending {
        let slot = (fact.instance.clone(), fact.is_achievement());
        if !keys.insert(slot) {
            return Err("duplicate pending activity evidence slot".into());
        }
        if runtime
            .instances
            .get(&fact.instance)
            .is_some_and(|instance| {
                if fact.is_achievement() {
                    instance.achieved.is_some()
                } else {
                    instance.settled.is_some()
                }
            })
        {
            return Err("activity evidence is both pending and acknowledged".into());
        }
        candidate.acknowledge(fact, now)?;
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn declared_actor_fixture(
    actor: &str,
    template: &str,
) -> crate::world::config::WorldConfig {
    crate::world::config::WorldConfig {
        entities: vec![crate::world::config::WorldEntity {
            name: Some(actor.into()),
            template_path: template.into(),
            ..Default::default()
        }],
        name_to_uuid: std::collections::HashMap::from([(actor.into(), actor.into())]),
        ..Default::default()
    }
}

#[cfg(test)]
#[path = "activity_bridge_tests.rs"]
mod tests;

/// A body-only inverse cannot roll back the physical owner under this World's
/// already accepted job/evidence. Unbound and merely offered work is unchanged.
pub(crate) fn has_accepted_binding(
    instances: &phoenix_sim_world::world::activities::SituationInstances,
    actor: &str,
) -> bool {
    instances
        .instances
        .values()
        .any(|row| row.request.actor() == actor && row.accepted.is_some())
}
