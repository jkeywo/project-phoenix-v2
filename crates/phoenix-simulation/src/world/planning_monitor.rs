//! Actual own-Console monitoring. This owner observes; physical Jobs stay in
//! Repair and Logistics, and authored consequences use the normal script host.
use super::opportunities::OpportunityState;
use super::{config::WorldConfig, server::WorldContentRuntime};
use phoenix_model::{activity::RepairGoal, damage::RepairWorkTarget, observation::*};
use phoenix_sim_world::world::planning::{opportunities::CompiledMaintenanceSearch, *};

#[derive(Clone, Debug)]
pub(crate) struct NodeActivation {
    pub reference: phoenix_model::comms_request::OpportunityRef,
    pub node: String,
    pub call: phoenix_sim_world::world::activities::ScenarioCall,
    pub actor: String,
    pub owner: phoenix_model::identity::SystemId,
    pub promise: String,
}
/// Recompute only from the same permitted typed view used by the operator.
fn reading(
    source: &CompiledMaintenanceSearch,
    required_parts: i64,
    graph: &Decomposition,
    node: &str,
    config: &crate::ship_plugin::ShipConfigComponent,
    boards: &crate::server_app::ShipSystemBlackboards,
    now: u64,
) -> Option<PlanNodeReading> {
    use crate::core::messages::SystemBlackboard;
    let phoenix_sim_world::world::activities::ActivityRequest::Repair {
        owner,
        team,
        target: RepairWorkTarget::Compartment { compartment },
        goal: RepairGoal::Structure,
        ..
    } = &graph.leaves.get(node)?.request
    else {
        return None;
    };
    let known = config.0.observations_for_system(owner, &boards.0);
    let SystemBlackboard::Repair(board) = known.get(owner)? else {
        return None;
    };
    let (subject, hp, max_hp) = board
        .system_hull
        .iter()
        .filter_map(|row| {
            let physical = row.physical.as_ref()?;
            (physical.compartment == *compartment).then_some((
                row.system_id.clone(),
                f64::from(physical.structural_hp),
                f64::from(physical.structural_max_hp),
            ))
        })
        .min_by(|left, right| left.0.cmp(&right.0))?;
    let available = known
        .get(&phoenix_model::logistics_ui::stock_publication(owner))
        .and_then(|value| match value {
            SystemBlackboard::Stock(stock) => stock
                .stores
                .iter()
                .find(|store| {
                    store.id == source.config.destination_store
                        && store.compartment == *compartment
                        && store.resource == source.config.commodity
                })
                .map(|store| store.available),
            _ => None,
        });
    let route = board.interior.as_ref().and_then(|layout| {
        // An absent boundary read is unknown, never an implicitly open door.
        if layout
            .connections
            .iter()
            .filter_map(|link| link.boundary.as_ref())
            .any(|boundary| !board.boundaries.iter().any(|row| &row.system == boundary))
        {
            return None;
        }
        let blocked = layout
            .connections
            .iter()
            .filter(|link| {
                link.boundary.as_ref().is_some_and(|id| {
                    board.boundaries.iter().any(|row| {
                        &row.system == id
                            && (row.seal_requested
                                || row.actuator_sealed
                                || row.phase != phoenix_model::interior::BoundaryPhase::Open)
                    })
                })
            })
            .map(|link| link.id.clone())
            .collect();
        let graph = phoenix_sim_gameplay::ship::interior::InteriorGraph::compile(
            layout,
            &config.0,
            board.teams.len(),
        )
        .ok()?;
        match &board.navigation.get(usize::try_from(*team).ok()?)?.location {
            phoenix_model::interior::TeamLocation::Compartment { compartment: from } => {
                Some(graph.route(from, compartment, &blocked).is_some())
            }
            _ => None,
        }
    });
    let other_work = board
        .jobs
        .iter()
        .filter(|work| {
            work.owner == *owner
                && work.outcome.is_none()
                && matches!(
                    work.phase,
                    phoenix_model::job::JobPhase::Travelling
                        | phoenix_model::job::JobPhase::Working
                )
        })
        .filter(|work| match &work.target_work {
            Some(RepairWorkTarget::Compartment {
                compartment: target,
            }) => target == compartment,
            Some(RepairWorkTarget::Component { system, .. }) => board
                .interior
                .as_ref()
                .is_some_and(|layout| layout.placements.get(system) == Some(compartment)),
            None => work.target_system.as_ref().is_some_and(|system| {
                board
                    .interior
                    .as_ref()
                    .is_some_and(|layout| layout.placements.get(system) == Some(compartment))
            }),
        })
        .min_by_key(|work| work.id)
        .cloned();
    let reason = if hp == max_hp {
        PlanNodeReason::RestoredExternally
    } else {
        // Expected supply wait is already explained by original prerequisites;
        // it is not a blocked resource decision before those prerequisites land.
        if other_work.is_some() {
            PlanNodeReason::WorkInProgress
        } else if available.is_some_and(|amount| amount < required_parts) {
            PlanNodeReason::PartsUnavailable
        } else if available.is_some() && route == Some(false) {
            PlanNodeReason::RouteUnavailable
        } else if available.is_some() && route == Some(true) {
            PlanNodeReason::Available
        } else {
            return None;
        }
    };
    Some(PlanNodeReading {
        observation: ObservationMetadata::at(
            now,
            ObservationSubject::System { id: subject },
            ObservationProvenance {
                kind: ObservationSource::Evidence,
                system: Some(owner.clone()),
                label: String::new(),
            },
            ObservationUncertainty::Qualitative,
        )
        .received(now),
        hp,
        max_hp,
        local_available: available,
        required_parts,
        route_available: route,
        other_work,
        reason,
    })
}
pub(crate) fn observe(
    runtime: &mut WorldContentRuntime,
    base: &WorldConfig,
    actor: &str,
    config: &crate::ship_plugin::ShipConfigComponent,
    boards: &crate::server_app::ShipSystemBlackboards,
    now: u64,
) -> Result<(), String> {
    let Some(source) = &base.compiled_opportunity_search else {
        return Ok(());
    };
    for index in 0..runtime.opportunities.len() {
        let record = runtime.opportunities[index].clone();
        if record.reference.actor != actor {
            continue;
        }
        let OpportunityState::Accepted {
            input, mut choice, ..
        } = record.state
        else {
            continue;
        };
        let graph = source.derive(&record.reference, &input, &record.proposal)?;
        for (node, leaf) in &graph.leaves {
            if runtime
                .situations
                .instances
                .get(&leaf.id)
                .is_some_and(|row| row.accepted.is_some())
                || choice
                    .node_dispositions
                    .get(node)
                    .is_some_and(|row| row.latest.terminal())
            {
                continue;
            }
            let Some(observation) = reading(
                source,
                input.required_parts,
                &graph,
                node,
                config,
                boards,
                now,
            ) else {
                continue;
            };
            if !observation.terminal() {
                let intent = graph
                    .intent_requests(&choice, &runtime.situations, now)?
                    .into_iter()
                    .find(|row| row.config.id == leaf.id);
                if intent.is_none_or(|row| {
                    row.prerequisites
                        .iter()
                        .any(|prerequisite| !prerequisite.satisfied)
                }) {
                    continue;
                }
            }
            graph.observe_node(&mut choice, &runtime.situations, node, observation, now)?;
        }
        if let OpportunityState::Accepted { choice: stored, .. } =
            &mut runtime.opportunities[index].state
        {
            *stored = choice;
        }
    }
    Ok(())
}
pub(crate) fn ready(runtime: &WorldContentRuntime, base: &WorldConfig) -> Vec<NodeActivation> {
    let Some(source) = &base.compiled_opportunity_search else {
        return Vec::new();
    };
    runtime
        .opportunities
        .iter()
        .flat_map(|record| {
            let OpportunityState::Accepted { choice, .. } = &record.state else {
                return Vec::new();
            };
            choice
                .node_dispositions
                .iter()
                .filter(|(_, row)| row.latest.terminal() && row.consequence.is_none())
                .map(|(node, _)| NodeActivation {
                    reference: record.reference.clone(),
                    node: node.clone(),
                    call: source.config.changed_work.clone(),
                    actor: record.reference.actor.clone(),
                    owner: source.config.repair_owner.clone(),
                    promise: choice
                        .required_commitment
                        .clone()
                        .expect("accepted generated promise"),
                })
                .collect()
        })
        .collect()
}
pub(crate) fn finish(
    runtime: &mut WorldContentRuntime,
    base: &WorldConfig,
    activation: &NodeActivation,
    result: phoenix_sim_world::world::activities::ScenarioCallResult,
    now: u64,
) -> Result<(), String> {
    let source = base
        .compiled_opportunity_search
        .as_ref()
        .ok_or("changed-work source disappeared")?;
    let record = runtime
        .opportunities
        .iter_mut()
        .find(|row| row.reference == activation.reference)
        .ok_or("changed-work execution disappeared")?;
    let OpportunityState::Accepted { input, choice, .. } = &mut record.state else {
        return Err("changed-work execution is not accepted".into());
    };
    let graph = source.derive(&record.reference, input, &record.proposal)?;
    if activation.call != source.config.changed_work
        || activation.promise != choice.required_commitment.clone().unwrap_or_default()
        || activation.actor != choice.actor_uuid
        || activation.owner != source.config.repair_owner
    {
        return Err("changed-work callback binding differs from original source".into());
    }
    graph.finish_node_consequence(choice, &runtime.situations, &activation.node, result, now)
}
pub(crate) fn withheld_instances(
    runtime: &WorldContentRuntime,
) -> std::collections::BTreeSet<String> {
    runtime
        .opportunities
        .iter()
        .filter_map(|record| match &record.state {
            OpportunityState::Accepted { choice, .. } => Some(choice),
            _ => None,
        })
        .flat_map(|choice| {
            choice
                .node_dispositions
                .iter()
                .filter(|(_, row)| row.withheld())
                .map(|(node, _)| format!("{}/{node}", choice.execution.namespace(&choice.id)))
        })
        .collect()
}
