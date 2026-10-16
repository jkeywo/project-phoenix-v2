//! The same World evidence and progression owner as the snapshot payload.
use super::*;
use phoenix_model::{
    activity::*,
    damage::{ComponentCondition, RepairWorkTarget},
    job::JobOutcome,
};
use phoenix_sim_world::world::activities::*;
fn outcome(acc: u64, value: JobOutcome) -> u64 {
    fold_u64(
        acc,
        match value {
            JobOutcome::Completed => 0,
            JobOutcome::Cancelled => 1,
            JobOutcome::Failed => 2,
        },
    )
}
fn target(mut acc: u64, value: &RepairWorkTarget) -> u64 {
    match value {
        RepairWorkTarget::Compartment { compartment } => {
            acc = fold_u64(acc, 0);
            fold_str(acc, compartment)
        }
        RepairWorkTarget::Component { system, component } => {
            acc = fold_u64(acc, 1);
            acc = fold_str(acc, &system.0);
            fold_str(acc, component)
        }
    }
}
fn condition(mut acc: u64, value: &ComponentCondition) -> u64 {
    match value {
        ComponentCondition::Healthy => fold_u64(acc, 0),
        ComponentCondition::Damaged => fold_u64(acc, 1),
        ComponentCondition::Destroyed => fold_u64(acc, 2),
        ComponentCondition::Patched { factor } => {
            acc = fold_u64(acc, 3);
            fold_f32(acc, *factor)
        }
    }
}
fn scope(mut acc: u64, value: &ActivityExecution) -> u64 {
    acc = fold_str(acc, &value.actor);
    acc = fold_str(acc, &value.owner.0);
    match &value.identity {
        ActivityExecutionIdentity::Physical { job } => fold_u64(fold_u64(acc, 0), job.0),
        ActivityExecutionIdentity::Communication {
            origin_id,
            recipient,
            emitted_tick,
        } => {
            acc = fold_str(fold_u64(acc, 1), origin_id);
            acc = fold_str(acc, recipient);
            fold_u64(acc, *emitted_tick)
        }
    }
}
fn request_target(mut acc: u64, value: &phoenix_model::messages::RepairTarget) -> u64 {
    use phoenix_model::messages::RepairTarget;
    match value {
        RepairTarget::Work(work) => target(fold_u64(acc, 0), work),
        RepairTarget::Station(id) => {
            acc = fold_u64(acc, 1);
            fold_str(acc, &id.0)
        }
        RepairTarget::Console(id) => {
            acc = fold_u64(acc, 2);
            fold_str(acc, &id.0)
        }
        RepairTarget::System(id) => {
            acc = fold_u64(acc, 3);
            fold_str(acc, &id.0)
        }
        RepairTarget::Core => fold_u64(acc, 4),
        RepairTarget::External => fold_u64(acc, 5),
    }
}
pub(super) fn fact(mut acc: u64, value: &ActivityFact) -> u64 {
    acc = fold_str(acc, &value.instance);
    acc = scope(acc, &value.scope);
    acc = fold_u64(acc, value.tick);
    match &value.outcome {
        ActivityOutcome::VesselAcquired { .. } => fold_serde(fold_u64(acc, 5), &value.outcome),
        ActivityOutcome::SecuritySettled { .. } => fold_serde(fold_u64(acc, 6), &value.outcome),
        ActivityOutcome::CommunicationRead { .. } => fold_serde(fold_u64(acc, 3), &value.outcome),
        ActivityOutcome::CommunicationReturned { .. } => {
            fold_serde(fold_u64(acc, 4), &value.outcome)
        }
        ActivityOutcome::RepairAchieved {
            stage,
            target: work,
            remedy,
            result,
        } => {
            acc = fold_u64(acc, 0);
            acc = fold_u64(acc, *stage);
            acc = target(acc, work);
            acc = fold_str(acc, remedy);
            match result {
                RepairAchievement::Structure { restored_hp } => {
                    acc = fold_u64(acc, 0);
                    fold_u64(acc, restored_hp.to_bits())
                }
                RepairAchievement::Component { before, after } => {
                    acc = fold_u64(acc, 1);
                    acc = condition(acc, before);
                    condition(acc, after)
                }
            }
        }
        ActivityOutcome::RepairSettled { outcome: value } => outcome(fold_u64(acc, 1), *value),
        ActivityOutcome::Delivery {
            source,
            destination,
            commodity,
            amount,
            returned,
            outcome: value,
        } => {
            acc = fold_u64(acc, 2);
            acc = fold_str(acc, source);
            acc = fold_str(acc, destination);
            acc = fold_str(acc, commodity);
            acc = fold_u64(acc, *amount as u64);
            acc = fold_u64(acc, u64::from(*returned));
            outcome(acc, *value)
        }
    }
}
pub(super) fn instances(mut acc: u64, values: &SituationInstances) -> u64 {
    acc = fold_str(acc, "scenario-activities");
    acc = fold_u64(acc, values.instances.len() as u64);
    for (id, value) in &values.instances {
        acc = fold_str(acc, id);
        acc = fold_str(acc, &value.definition);
        acc = fold_str(acc, value.request.actor());
        acc = fold_str(acc, &value.request.owner().0);
        match &value.request {
            ActivityRequest::AcquireVessel { .. } => {
                acc = fold_serde(fold_u64(acc, 3), &value.request);
            }
            ActivityRequest::Communication { .. } => {
                acc = fold_serde(fold_u64(acc, 2), &value.request);
            }
            ActivityRequest::Repair {
                team,
                request,
                target: work,
                goal,
                ..
            } => {
                acc = fold_u64(acc, 0);
                acc = fold_u64(acc, *team as u64);
                acc = request_target(acc, request);
                acc = target(acc, work);
                acc = fold_u64(
                    acc,
                    match goal {
                        RepairGoal::Structure => 0,
                        RepairGoal::Patch => 1,
                        RepairGoal::Full => 2,
                    },
                );
            }
            ActivityRequest::Delivery {
                source,
                destination,
                commodity,
                amount,
                ..
            } => {
                acc = fold_u64(acc, 1);
                acc = fold_str(acc, source);
                acc = fold_str(acc, destination);
                acc = fold_str(acc, commodity);
                acc = fold_u64(acc, *amount as u64);
            }
        }
        acc = fold_u64(acc, value.armed_tick);
        acc = fold_optional_u64(acc, value.accepted_tick);
        if let Some(value) = &value.accepted {
            acc = scope(fold_u64(acc, 1), value);
        } else {
            acc = fold_u64(acc, 0);
        }
        for value in [&value.achieved, &value.settled] {
            if let Some(value) = value {
                acc = fact(fold_u64(acc, 1), value);
            } else {
                acc = fold_u64(acc, 0);
            }
        }
        acc = fold_optional_str(acc, value.state.as_deref());
        acc = fold_u64(acc, value.activations.len() as u64);
        for (state, result) in &value.activations {
            acc = fold_str(acc, state);
            acc = fold_u64(
                acc,
                match result {
                    ScenarioCallResult::Applied => 0,
                    ScenarioCallResult::Refused => 1,
                },
            );
        }
        acc = fold_u64(acc, value.activation_ticks.len() as u64);
        for (state, tick) in &value.activation_ticks {
            acc = fold_str(acc, state);
            acc = fold_u64(acc, *tick);
        }
    }
    acc
}

/// Source-ordered typed arguments and original method/release frontiers only.
pub(super) fn plan(mut acc: u64, value: &phoenix_sim_world::world::planning::PlanChoice) -> u64 {
    use phoenix_sim_world::world::planning::Value;
    acc = fold_str(acc, "world-plan-choice");
    if !value.execution.is_primary() {
        acc = fold_serde(fold_str(acc, "optional-execution"), &value.execution);
    }

    acc = fold_str(acc, &value.id);
    acc = fold_u64(acc, value.chosen_tick);
    acc = fold_optional_str(acc, value.required_commitment.as_deref());
    acc = fold_str(acc, &value.actor_uuid);
    acc = fold_u64(acc, value.root_arguments.len() as u64);
    for (name, arg) in &value.root_arguments {
        acc = fold_str(acc, name);
        acc = fold_str(
            acc,
            match arg {
                Value::Actor(_) => "actor",
                Value::Vessel(_) => "vessel",
                Value::System(_) => "system",
                Value::Team(_) => "team",
                Value::RepairRequest(_) => "repair_request",
                Value::RepairTarget(_) => "repair_target",
                Value::RepairGoal(_) => "repair_goal",
                Value::Store(_) => "store",
                Value::Commodity(_) => "commodity",
                Value::Quantity(_) => "quantity",
            },
        );
        acc = match arg {
            Value::Actor(value)
            | Value::Vessel(value)
            | Value::Store(value)
            | Value::Commodity(value) => fold_str(acc, value),
            Value::System(value) => fold_str(acc, &value.0),
            Value::Team(value) => fold_u64(acc, u64::from(*value)),
            Value::Quantity(value) => fold_u64(acc, *value as u64),
            Value::RepairRequest(value) => request_target(acc, value),
            Value::RepairTarget(value) => target(acc, value),
            Value::RepairGoal(value) => fold_u64(
                acc,
                match value {
                    RepairGoal::Structure => 0,
                    RepairGoal::Patch => 1,
                    RepairGoal::Full => 2,
                },
            ),
        };
    }
    acc = fold_u64(acc, value.methods.len() as u64);
    for (node, method) in &value.methods {
        acc = fold_str(acc, node);
        acc = fold_str(acc, &method.task.library_path);
        acc = fold_str(acc, &method.task.task);
        acc = fold_str(acc, &method.method);
    }
    acc = fold_u64(acc, value.refused_methods.len() as u64);
    for reason in &value.refused_methods {
        acc = fold_str(acc, reason);
    }
    if let Some(saved) = &value.temporal {
        use phoenix_sim_world::world::planning::temporal::{OwnerInput, TemporalOutcome};
        acc = fold_str(acc, "temporal-maintenance-choice");
        acc = fold_u64(acc, saved.input.tick_nanos);
        acc = fold_u64(acc, u64::from(saved.profile_index));
        acc = fold_u64(acc, saved.deadline_tick);
        acc = fold_u64(acc, saved.input.owners.len() as u64);
        for (owner, input) in &saved.input.owners {
            acc = fold_str(acc, &owner.0);
            acc = match input {
                OwnerInput::Human { station, rating } => {
                    fold_str(fold_str(fold_str(acc, "human"), &station.0), rating)
                }
                OwnerInput::Ai {} => fold_str(acc, "ai"),
                OwnerInput::Simplified {} => fold_str(acc, "simplified"),
                OwnerInput::Unavailable { reason } => {
                    fold_serde(fold_str(acc, "unavailable"), reason)
                }
            };
        }
        match &saved.outcome {
            TemporalOutcome::Refused { reason } => {
                acc = fold_str(fold_str(acc, "refused"), reason);
            }
            TemporalOutcome::Committed { nodes } => {
                acc = fold_str(acc, "committed");
                acc = fold_u64(acc, nodes.len() as u64);
                for (node, interval) in nodes {
                    acc = fold_u64(
                        fold_u64(fold_str(acc, node), interval.start_tick),
                        interval.end_tick,
                    );
                }
            }
        }
    }
    if !value.temporal_reconsiderations.is_empty() {
        acc = fold_serde(
            fold_str(acc, "temporal-reconsiderations"),
            &value.temporal_reconsiderations,
        );
    }
    if !value.node_dispositions.is_empty() {
        let mut normalized = value.node_dispositions.clone();
        for row in normalized.values_mut() {
            for reading in [&mut row.first, &mut row.latest] {
                if reading.hp == 0.0 {
                    reading.hp = 0.0;
                }
                if reading.max_hp == 0.0 {
                    reading.max_hp = 0.0;
                }
            }
        }
        acc = fold_serde(fold_str(acc, "plan-node-dispositions"), &normalized);
    }
    acc = fold_u64(acc, value.released.len() as u64);
    for (node, tick) in &value.released {
        acc = fold_str(acc, node);
        acc = fold_u64(acc, *tick);
    }
    acc
}

#[cfg(test)]
mod disposition_digest_tests {
    use super::*;
    use phoenix_sim_world::world::planning::*;
    #[test]
    fn node_readings_canonicalize_zero_and_retain_meaningful_changes() {
        let mut choice:PlanChoice=serde_json::from_value(serde_json::json!({"id":"maintenance","execution":{"kind":"optional","source":"maintenance","ordinal":1},"root_arguments":{},"chosen_tick":1,"actor_uuid":"actor","released":{},"methods":{}})).unwrap();
        let reading = PlanNodeReading {
            observation: phoenix_model::observation::ObservationMetadata::at(
                2,
                phoenix_model::observation::ObservationSubject::System {
                    id: phoenix_model::identity::SystemId("power".into()),
                },
                phoenix_model::observation::ObservationProvenance {
                    kind: phoenix_model::observation::ObservationSource::Evidence,
                    system: Some(phoenix_model::identity::SystemId("repair".into())),
                    label: String::new(),
                },
                phoenix_model::observation::ObservationUncertainty::Qualitative,
            )
            .received(2),
            hp: -0.0,
            max_hp: 100.0,
            local_available: Some(0),
            required_parts: 1,
            route_available: Some(true),
            other_work: None,
            reason: PlanNodeReason::PartsUnavailable,
        };
        choice.node_dispositions.insert(
            "step-0".into(),
            PlanNodeDisposition {
                first: reading.clone(),
                latest: reading,
                consequence: None,
            },
        );
        let original = plan(7, &choice);
        let mut positive = choice.clone();
        positive
            .node_dispositions
            .get_mut("step-0")
            .unwrap()
            .first
            .hp = 0.0;
        positive
            .node_dispositions
            .get_mut("step-0")
            .unwrap()
            .latest
            .hp = 0.0;
        assert_eq!(original, plan(7, &positive), "canonical zero equality");
        positive
            .node_dispositions
            .get_mut("step-0")
            .unwrap()
            .latest
            .hp = 1.0;
        assert_ne!(
            original,
            plan(7, &positive),
            "meaningful HP difference remains exact"
        );
        let mut clock = choice.clone();
        clock
            .node_dispositions
            .get_mut("step-0")
            .unwrap()
            .latest
            .observation
            .observed_tick = Some(3);
        assert_ne!(
            original,
            plan(7, &clock),
            "original evidence clocks participate"
        );
    }
}
