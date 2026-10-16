//! Imported maintenance HTN consumer at the existing World fact boundary.
use super::{config::WorldConfig, server::WorldContentRuntime};
use phoenix_sim_world::world::activities::{ActivityRequest, SituationConfig};
#[path = "planning_inputs.rs"]
mod inputs;
pub(crate) use inputs::{validate_original as validate_forecast_input, ForecastSources};

pub(crate) const UNSUPPORTED_PRIMITIVE: &str =
    "primitive unsupported by this actor's actual accepted equipment";

/// Immutable recipe-only capability projection used by saved-choice preflight.
/// No live actor, current condition, stock or resource reserve participates.
pub(crate) fn supports_recipe(
    request: &ActivityRequest,
    config: &crate::entities::config::EntityConfig,
) -> Result<bool, String> {
    Ok(match request {
        ActivityRequest::AcquireVessel { owner, team, .. } => config
            .ship_config
            .as_ref()
            .zip(config.security.as_ref())
            .is_some_and(|(ship, security)| {
                crate::jobs::activity_bindings::acquisition_supported(owner, *team, ship, security)
            }),
        ActivityRequest::Communication { owner, channel, .. } => config
            .ship_config
            .as_ref()
            .zip(config.comms.as_ref().and_then(|row| row.radio.as_ref()))
            .is_some_and(|(ship, radio)| {
                crate::jobs::activity_bindings::communication_supported(owner, channel, ship, radio)
            }),
        ActivityRequest::Repair { .. } => {
            let Some((ship, repair)) = config.ship_config.as_ref().zip(config.repair.as_ref())
            else {
                return Ok(false);
            };
            let Some(damage) = crate::ship::compartment_damage::ShipDamage::from_config(config)?
            else {
                return Ok(false);
            };
            super::activities::repair_binding_supported(
                request,
                ship,
                &damage,
                &crate::ship::system_registry::repair_system_id(),
                repair.repair_team_count as usize,
            )
        }
        ActivityRequest::Delivery { .. } => {
            crate::resources::logistics_profile::compile_entity_logistics(config)?.is_some_and(
                |profile| super::activities::delivery_binding_supported(request, &profile),
            )
        }
    })
}

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
#[cfg(test)]
pub(crate) fn advance_for_actor(
    runtime: &mut WorldContentRuntime,
    base: &WorldConfig,
    actor: &str,
    slot: Option<&str>,
    tick: u64,
    mut supports: impl FnMut(&ActivityRequest) -> bool,
) -> Result<(), String> {
    advance_with_forecast(runtime, base, actor, slot, tick, None, &mut supports)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn advance_with_forecast(
    runtime: &mut WorldContentRuntime,
    base: &WorldConfig,
    actor: &str,
    slot: Option<&str>,
    tick: u64,
    forecast: Option<&ForecastSources<'_>>,
    mut supports: impl FnMut(&ActivityRequest) -> bool,
) -> Result<(), String> {
    let Some(compiled) = base.compiled_planning.as_ref() else {
        return Ok(());
    };
    if !matches_actor(base, compiled.actor_reference()?, actor, slot) {
        return Ok(());
    }
    let (mut choice, graph) = if let Some(choice) = &runtime.plan_choice {
        if !choice.execution.is_primary() {
            return Err("primary plan contains an optional execution".into());
        }
        compiled.validate_required_commitment(choice, &runtime.commitments, tick)?;
        (
            choice.clone(),
            compiled.validate_history(choice, &runtime.situations, tick)?,
        )
    } else {
        if !compiled.commitment_ready(&runtime.commitments, tick)? {
            return Ok(());
        }
        // A missing bootstrap owner is a readiness barrier before choosing,
        // never an accepted default-Human refusal or a method retry.
        if compiled.config().temporal.is_some() && forecast.is_none() {
            return Ok(());
        }
        let mut selected = compiled.choose(actor, tick, |request| {
            if supports(request) {
                Ok(())
            } else {
                Err(UNSUPPORTED_PRIMITIVE.into())
            }
        })?;
        if compiled.config().temporal.is_some() {
            let input = forecast
                .expect("checked bootstrap readiness")
                .project(&selected.graph);
            validate_forecast_input(
                &input,
                &forecast.expect("checked bootstrap readiness").config.0,
            )?;
            compiled.record_schedule_decision(&mut selected.choice, input)?;
        }
        (selected.choice, selected.graph)
    };
    // An unavailable recipe does not trigger a different method or erase
    // released history. Its future leaves wait for this same owner capability.
    if graph.leaves.values().any(|row| !supports(&row.request)) {
        return Ok(());
    }
    if choice
        .effective_temporal()
        .is_some_and(|decision| decision.refused())
    {
        if let Some(forecast) = forecast {
            let input = forecast.project(&graph);
            validate_forecast_input(&input, &forecast.config.0)?;
            compiled.reconsider_schedule(&mut choice, input, tick)?;
        }
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
    let mut rows = Vec::new();
    if let (Some(compiled), Some(choice)) = (&base.compiled_planning, &runtime.plan_choice) {
        rows.extend(
            compiled
                .ready_requests(choice, &runtime.situations, tick)
                .unwrap_or_default(),
        );
    }
    if let Some(source) = &base.compiled_opportunity_search {
        for record in &runtime.opportunities {
            if let crate::world::opportunities::OpportunityState::Accepted {
                input, choice, ..
            } = &record.state
            {
                if let Ok(graph) = source.derive(&record.reference, input, &record.proposal) {
                    rows.extend(
                        graph
                            .ready_requests(choice, &runtime.situations, tick)
                            .unwrap_or_default(),
                    );
                }
            }
        }
    }
    rows.into_iter()
        .filter(|row| row.request.actor() == actor && row.request.owner() == owner)
        .collect()
}

/// Accepted own-owner intent uses the same frozen graph as release; a future
/// leaf is not admitted machinery and contains no current damage diagnosis.
pub(crate) fn repair_intents(
    runtime: Option<&WorldContentRuntime>,
    base: Option<&WorldConfig>,
    actor: &str,
    owner: &phoenix_model::identity::SystemId,
    now: u64,
) -> Vec<phoenix_model::activity::PlannedRepairIntent> {
    let (Some(runtime), Some(base)) = (runtime, base) else {
        return Vec::new();
    };
    let mut rows = Vec::new();
    if let (Some(compiled), Some(choice)) = (&base.compiled_planning, &runtime.plan_choice) {
        if choice
            .effective_temporal()
            .is_none_or(|schedule| schedule.nodes().is_some())
        {
            if let Ok(graph) = compiled.validate_history(choice, &runtime.situations, now) {
                rows.extend(
                    graph
                        .intent_requests(choice, &runtime.situations, now)
                        .unwrap_or_default(),
                );
            }
        }
    }
    if let Some(source) = &base.compiled_opportunity_search {
        for record in &runtime.opportunities {
            if let crate::world::opportunities::OpportunityState::Accepted {
                input, choice, ..
            } = &record.state
            {
                if let Ok(graph) = source.derive(&record.reference, input, &record.proposal) {
                    rows.extend(
                        graph
                            .intent_requests(choice, &runtime.situations, now)
                            .unwrap_or_default(),
                    );
                }
            }
        }
    }
    rows.into_iter()
        .filter_map(project_repair_intent)
        .filter(|(target_actor, target_owner, _)| target_actor == actor && target_owner == owner)
        .map(|(_, _, intent)| intent)
        .collect()
}

pub(crate) fn project_repair_intent(
    row: phoenix_sim_world::world::planning::PlanIntentRequest,
) -> Option<(
    String,
    phoenix_model::identity::SystemId,
    phoenix_model::activity::PlannedRepairIntent,
)> {
    let ActivityRequest::Repair {
        actor,
        owner,
        team,
        request,
        target,
        goal,
    } = row.config.request
    else {
        return None;
    };
    Some((
        actor,
        owner,
        phoenix_model::activity::PlannedRepairIntent {
            id: row.config.id,
            chosen_tick: row.chosen_tick,
            released_tick: row.released_tick,
            team: u8::try_from(team).ok()?,
            request,
            target,
            goal,
            prerequisites: row.prerequisites,
            disposition: row.disposition,
        },
    ))
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
