//! Exact real Security admission and post-commit evidence; no physical owner.
use crate::world::{config::WorldConfig, server::WorldContentRuntime};
use phoenix_model::{
    activity::*,
    identity::SystemId,
    job::{JobId, JobOutcome},
    vessel_ownership::VesselOperationalOwnerData,
};
use phoenix_sim_world::world::activities::{ActivityRequest, SituationInstances};

#[allow(clippy::too_many_arguments)]
pub(crate) fn prepare(
    runtime: Option<&WorldContentRuntime>,
    base: Option<&WorldConfig>,
    actor: &str,
    owner: &SystemId,
    team: u32,
    target: &str,
    job: JobId,
    tick: u64,
) -> Result<Option<SituationInstances>, String> {
    let Some(runtime) = runtime else {
        return Ok(None);
    };
    if base.is_none() && runtime.situations.instances.values().any(|row| matches!(&row.request, ActivityRequest::AcquireVessel { actor:a, owner:o, team:t, .. } if row.accepted.is_none() && a == actor && o == owner && *t == team)) { return Err("released acquisition lost accepted World identity".into()); }
    let mut eligible: std::collections::BTreeSet<_> =
        crate::world::planning::requests(Some(runtime), base, actor, owner, tick)
            .into_iter()
            .map(|row| row.id)
            .collect();
    if let Some(base) = base {
        eligible.extend(base.situations.iter().map(|row| row.id.clone()));
    }
    let references: Vec<_> = runtime
        .situations
        .instances
        .iter()
        .filter_map(|(id, row)| match &row.request {
            ActivityRequest::AcquireVessel {
                actor: a,
                owner: o,
                team: t,
                target: reference,
            } if eligible.contains(id)
                && row.accepted.is_none()
                && a == actor
                && o == owner
                && *t == team
                && base
                    .and_then(|base| base.name_to_uuid.get(reference))
                    .is_some_and(|uuid| uuid == target) =>
            {
                Some(reference.clone())
            }
            _ => None,
        })
        .collect();
    if references.len() > 1 {
        return Err("ambiguous released acquisition binding".into());
    }
    let Some(reference) = references.first() else {
        return Ok(None);
    };
    let mut next = runtime.situations.clone();
    next.accept_acquisition(actor, owner, team, reference, job, tick)?;
    Ok(Some(next))
}
fn reference(
    runtime: &WorldContentRuntime,
    actor: &str,
    owner: &SystemId,
    job: JobId,
) -> Option<String> {
    let scope: ActivityExecution = ActivityJob {
        actor: actor.into(),
        owner: owner.clone(),
        job,
    }
    .into();
    runtime
        .situations
        .instances
        .values()
        .find_map(|row| match &row.request {
            ActivityRequest::AcquireVessel { target, .. }
                if row.accepted.as_ref() == Some(&scope) =>
            {
                Some(target.clone())
            }
            _ => None,
        })
}
pub(crate) fn acquired(
    runtime: &mut WorldContentRuntime,
    ownership: VesselOperationalOwnerData,
    tick: u64,
) {
    let phoenix_model::vessel_ownership::VesselAcquisitionCause::SecurityBoard { scope, .. } =
        &ownership.cause;
    let scope = scope.clone();
    let Some(vessel) = reference(runtime, &scope.actor, &scope.owner, scope.job) else {
        return;
    };
    super::record(
        runtime,
        &scope.actor,
        &scope.owner,
        tick,
        [(
            scope.job,
            ActivityOutcome::VesselAcquired { vessel, ownership },
        )],
    );
}
pub(crate) fn settled(
    runtime: &mut WorldContentRuntime,
    actor: &str,
    owner: &SystemId,
    job: JobId,
    outcome: JobOutcome,
    tick: u64,
) {
    let Some(vessel) = reference(runtime, actor, owner, job) else {
        return;
    };
    super::record(
        runtime,
        actor,
        owner,
        tick,
        [(job, ActivityOutcome::SecuritySettled { vessel, outcome })],
    );
}
pub(crate) fn planned(
    runtime: Option<&WorldContentRuntime>,
    base: Option<&WorldConfig>,
    actor: &str,
    owner: &SystemId,
    tick: u64,
    alias: impl Fn(&str) -> Option<String>,
) -> Vec<PlannedAcquisition> {
    let Some(base) = base else {
        return Vec::new();
    };
    crate::world::planning::requests(runtime, Some(base), actor, owner, tick)
        .into_iter()
        .filter_map(|row| {
            let ActivityRequest::AcquireVessel { team, target, .. } = row.request else {
                return None;
            };
            let uuid = base.name_to_uuid.get(&target)?;
            Some(PlannedAcquisition {
                id: row.id.clone(),
                released_tick: runtime?.situations.instances.get(&row.id)?.armed_tick,
                team,
                target: alias(uuid)?,
            })
        })
        .collect()
}
