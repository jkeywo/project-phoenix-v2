//! Shared pure mutation, also used to preview atomic GM bulk actions.
use super::*;
use crate::world::dispatch::ActionCmd;

#[derive(Clone, Debug, Default)]
pub struct InstanceApply {
    pub changed: bool,
    pub transition: Option<(String, ObjectiveStatus)>,
    pub owned: Option<(String, String)>,
}

#[derive(Clone, Debug)]
pub enum InstanceRefusal {
    Activation(ActivationRefusal),
    Unknown(ObjectiveInstanceKey),
    InvalidProgress(ObjectiveInstanceKey),
}

impl InstanceRefusal {
    pub fn message(&self) -> String {
        match self {
            Self::Activation(reason) => activation_refusal_message(reason),
            Self::Unknown(key) => format!(
                "Unknown Objective instance '{}:{}'; check the authored id and instance_id",
                key.objective_id, key.instance_id
            ),
            Self::InvalidProgress(key) => format!(
                "Objective instance '{}:{}' was not active; check the authored ids and progress",
                key.objective_id, key.instance_id
            ),
        }
    }
}

pub fn apply_instance_command(
    command: ActionCmd,
    instances: &mut ObjectiveInstanceManager,
    objectives: &mut crate::objectives::ObjectiveManager,
    fleet: &[PlayerShipMembership],
    known_slots: &BTreeSet<String>,
    known_factions: &BTreeSet<String>,
) -> Result<InstanceApply, InstanceRefusal> {
    let complete = matches!(&command, ActionCmd::CompleteObjectiveInstance { .. });
    let mut result = InstanceApply::default();
    match command {
        ActionCmd::AddObjectiveInstance {
            spec,
            text,
            text_params,
            mandatory,
            targets,
            directive,
            utility,
            source,
            command_stance,
            origin_layer,
        } => {
            let key = spec.key.clone();
            result.changed = instances
                .activate_validated(spec, fleet, known_slots, known_factions)
                .map_err(InstanceRefusal::Activation)?;
            if result.changed {
                instances.set_directive(&key, directive.clone());
            }
            let inserted = objectives.add_full_with_params(
                key.objective_id.clone(),
                text,
                text_params,
                mandatory,
                targets,
                directive,
                utility,
                source,
                command_stance,
            );
            if inserted {
                result.owned = origin_layer.map(|layer| (layer, key.objective_id.clone()));
            }
            if result.changed {
                result.transition = Some((key.objective_id, ObjectiveStatus::Active));
            }
        }
        ActionCmd::CompleteObjectiveInstance { key } | ActionCmd::FailObjectiveInstance { key } => {
            if !instances.records().iter().any(|row| row.spec.key == key) {
                return Err(InstanceRefusal::Unknown(key));
            }
            result.changed = (if complete {
                instances.complete(&key, fleet)
            } else {
                instances.fail(&key, fleet)
            })
            .map_err(|conflict| {
                InstanceRefusal::Activation(ActivationRefusal::Conflict(conflict))
            })?;
            if result.changed {
                result.transition = Some((
                    key.objective_id,
                    if complete {
                        ObjectiveStatus::Completed
                    } else {
                        ObjectiveStatus::Failed
                    },
                ));
            }
        }
        ActionCmd::SetObjectiveInstanceProgress { key, progress } => {
            result.changed = instances.set_progress(&key, progress);
            if !result.changed {
                return Err(InstanceRefusal::InvalidProgress(key));
            }
        }
        _ => {}
    }
    Ok(result)
}

pub fn publish_instance_apply(
    result: &InstanceApply,
    targets: &[String],
    balance: Option<&mut Messages<crate::core::balance::BalanceEvent>>,
    layers: Option<&mut crate::world::server::WorldLayerMap>,
) {
    if let (Some((path, id)), Some(layers)) = (&result.owned, layers) {
        if let Some(layer) = layers.0.get_mut(path) {
            layer.owned_objective_ids.push(id.clone());
        }
    }
    if let (Some((id, status)), Some(balance)) = (&result.transition, balance) {
        if *status == ObjectiveStatus::Completed {
            balance.write(crate::core::balance::BalanceEvent::ObjectiveCompleted {
                objective_id: id.clone(),
            });
        }
        balance.write(crate::core::balance::BalanceEvent::ObjectiveChanged {
            objective_id: id.clone(),
            status: status.clone(),
            targets: targets.to_vec(),
        });
    }
}
