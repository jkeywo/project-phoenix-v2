//! Imported maintenance HTN consumer at the existing World fact boundary.
use super::{config::WorldConfig, server::WorldContentRuntime};
use phoenix_sim_world::world::activities::{ActivityRequest, SituationConfig};

fn matches_actor(base: &WorldConfig, requested: &str, actor: &str, slot: Option<&str>) -> bool {
    base.validate_situation_actor(requested).is_ok()
        && base.name_to_uuid.get(requested).map_or_else(
            || {
                requested
                    .strip_prefix("slot:")
                    .is_some_and(|id| Some(id) == slot)
            },
            |uuid| uuid == actor,
        )
}
/// Immutable supported equipment selects the method once. Current damage,
/// stock and AI preference never enter decomposition. No physical command runs.
pub(crate) fn advance_for_actor(
    runtime: &mut WorldContentRuntime,
    base: &WorldConfig,
    actor: &str,
    slot: Option<&str>,
    tick: u64,
    mut supports: impl FnMut(&ActivityRequest) -> bool,
) -> Result<(), String> {
    let Some(compiled) = base.compiled_planning.as_ref() else {
        return Ok(());
    };
    if !matches_actor(base, compiled.actor_reference()?, actor, slot) {
        return Ok(());
    }
    let (mut choice, graph) = if let Some(choice) = &runtime.plan_choice {
        (
            choice.clone(),
            compiled.validate_history(choice, &runtime.situations, tick)?,
        )
    } else {
        let selected = compiled.choose(actor, tick, |request| {
            if supports(request) {
                Ok(())
            } else {
                Err("primitive unsupported by this actor's actual accepted equipment".into())
            }
        })?;
        (selected.choice, selected.graph)
    };
    // An unavailable recipe does not trigger a different method or erase
    // released history. Its future leaves wait for this same owner capability.
    if graph.leaves.values().any(|row| !supports(&row.request)) {
        return Ok(());
    }
    let mut instances = runtime.situations.clone();
    compiled.release(&mut choice, &mut instances, actor, tick)?;
    // Definition identity and released bindings commit with the choice, never
    // before successful capacity/disjoint-binding checks on the shared owner.
    let mut definitions = runtime.situation_definitions.clone();
    for definition in graph.definitions.values() {
        if let Some(existing) = definitions.iter().find(|row| row.id == definition.id) {
            if existing != definition {
                return Err("HTN Scenario definition identity collision".into());
            }
        } else {
            definitions.push(definition.clone());
        }
    }
    runtime.situations = instances;
    runtime.situation_definitions = definitions;
    runtime.plan_choice = Some(choice);
    Ok(())
}
/// Only released unaccepted mission instructions for the exact producing
/// owner. This projection contains no current physical condition/stock claims.
pub(crate) fn requests(
    runtime: Option<&WorldContentRuntime>,
    base: Option<&WorldConfig>,
    actor: &str,
    owner: &phoenix_model::identity::SystemId,
    tick: u64,
) -> Vec<SituationConfig> {
    let (Some(runtime), Some(base)) = (runtime, base) else {
        return Vec::new();
    };
    let (Some(compiled), Some(choice)) = (&base.compiled_planning, &runtime.plan_choice) else {
        return Vec::new();
    };
    compiled
        .ready_requests(choice, &runtime.situations, tick)
        .unwrap_or_default()
        .into_iter()
        .filter(|row| {
            row.request.actor() == actor
                && match &row.request {
                    ActivityRequest::Repair { owner: target, .. }
                    | ActivityRequest::Delivery { owner: target, .. } => target == owner,
                }
        })
        .collect()
}

pub(crate) fn repair_requests(
    runtime: Option<&WorldContentRuntime>,
    base: Option<&WorldConfig>,
    actor: &str,
    owner: &phoenix_model::identity::SystemId,
    tick: u64,
) -> Vec<phoenix_model::activity::PlannedRepairRequest> {
    requests(runtime, base, actor, owner, tick)
        .into_iter()
        .filter_map(|row| {
            let released_tick = runtime?.situations.instances.get(&row.id)?.armed_tick;
            let ActivityRequest::Repair {
                team,
                request,
                target,
                goal,
                ..
            } = row.request
            else {
                return None;
            };
            Some(phoenix_model::activity::PlannedRepairRequest {
                id: row.id,
                released_tick,
                team: u8::try_from(team).ok()?,
                request,
                target,
                goal,
            })
        })
        .collect()
}
pub(crate) fn delivery_requests(
    runtime: Option<&WorldContentRuntime>,
    base: Option<&WorldConfig>,
    actor: &str,
    owner: &phoenix_model::identity::SystemId,
    tick: u64,
) -> Vec<phoenix_model::activity::PlannedDeliveryRequest> {
    requests(runtime, base, actor, owner, tick)
        .into_iter()
        .filter_map(|row| {
            let released_tick = runtime?.situations.instances.get(&row.id)?.armed_tick;
            let ActivityRequest::Delivery {
                source,
                destination,
                commodity,
                amount,
                ..
            } = row.request
            else {
                return None;
            };
            Some(phoenix_model::activity::PlannedDeliveryRequest {
                id: row.id,
                released_tick,
                source,
                destination,
                commodity,
                amount,
            })
        })
        .collect()
}

#[cfg(test)]
#[path = "planning_tests.rs"]
mod tests;
