//! Closed post-commit physical evidence. This is neither a job nor a command.
use crate::{
    damage::RepairWorkTarget,
    identity::SystemId,
    job::{JobId, JobOutcome},
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepairGoal {
    Structure,
    Patch,
    Full,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActivityJob {
    pub actor: String,
    pub owner: SystemId,
    pub job: JobId,
}

/// Result of the actual committed remedy, distinct from assignment return.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RepairAchievement {
    Structure {
        restored_hp: f64,
    },
    Component {
        before: crate::damage::ComponentCondition,
        after: crate::damage::ComponentCondition,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ActivityOutcome {
    RepairAchieved {
        stage: u64,
        target: RepairWorkTarget,
        remedy: String,
        result: RepairAchievement,
    },
    RepairSettled {
        outcome: JobOutcome,
    },
    Delivery {
        source: String,
        destination: String,
        commodity: String,
        amount: i64,
        returned: bool,
        outcome: JobOutcome,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ActivityEvidenceKey {
    RepairAchieved { stage: u64 },
    RepairSettled,
    Delivery,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActivityFact {
    pub instance: String,
    pub scope: ActivityJob,
    pub tick: u64,
    pub outcome: ActivityOutcome,
}
impl ActivityFact {
    pub fn key(&self) -> ActivityEvidenceKey {
        match self.outcome {
            ActivityOutcome::RepairAchieved { stage, .. } => {
                ActivityEvidenceKey::RepairAchieved { stage }
            }
            ActivityOutcome::RepairSettled { .. } => ActivityEvidenceKey::RepairSettled,
            ActivityOutcome::Delivery { .. } => ActivityEvidenceKey::Delivery,
        }
    }
    pub fn validate(&self, now: u64) -> Result<(), String> {
        if self.instance.trim().is_empty()
            || self.scope.actor.trim().is_empty()
            || self.scope.owner.0.trim().is_empty()
            || self.scope.job.0 == 0
            || self.tick > now
        {
            return Err("invalid activity evidence identity/tick".into());
        }
        match &self.outcome {
            ActivityOutcome::RepairAchieved {
                stage,
                target,
                remedy,
                result,
            } => {
                if *stage == u64::MAX || remedy.trim().is_empty() {
                    return Err("invalid repair stage".into());
                }
                match (target, result) {
                    (
                        RepairWorkTarget::Compartment { compartment },
                        RepairAchievement::Structure { restored_hp },
                    ) if !compartment.trim().is_empty()
                        && restored_hp.is_finite()
                        && *restored_hp > 0.0 => {}
                    (
                        RepairWorkTarget::Component { system, component },
                        RepairAchievement::Component { before, after },
                    ) if !system.0.trim().is_empty()
                        && !component.trim().is_empty()
                        && before != after
                        && valid_condition(before)
                        && valid_condition(after)
                        && matches!(
                            after,
                            crate::damage::ComponentCondition::Healthy
                                | crate::damage::ComponentCondition::Patched { .. }
                        ) => {}
                    _ => return Err("invalid committed repair result".into()),
                }
            }
            ActivityOutcome::RepairSettled { .. } => {}
            ActivityOutcome::Delivery {
                source,
                destination,
                commodity,
                amount,
                returned,
                outcome,
            } => {
                if source.trim().is_empty()
                    || destination.trim().is_empty()
                    || commodity.trim().is_empty()
                    || *amount <= 0
                    || (*returned && (*outcome != JobOutcome::Cancelled || source != destination))
                    || (!*returned && (*outcome != JobOutcome::Completed || source == destination))
                {
                    return Err("invalid committed delivery result".into());
                }
            }
        }
        Ok(())
    }
}
fn valid_condition(value: &crate::damage::ComponentCondition) -> bool {
    match value {
        crate::damage::ComponentCondition::Patched { factor } => {
            factor.is_finite() && *factor > 0.0 && *factor <= 1.0
        }
        _ => true,
    }
}

#[cfg(test)]
#[path = "activity_tests.rs"]
mod tests;

/// A released mission instruction, not a report about current damage or stock.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlannedRepairRequest {
    pub id: String,
    pub released_tick: u64,
    pub team: u8,
    pub request: crate::messages::RepairTarget,
    pub target: RepairWorkTarget,
    pub goal: RepairGoal,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlannedDeliveryRequest {
    pub id: String,
    pub released_tick: u64,
    pub source: String,
    pub destination: String,
    pub commodity: String,
    pub amount: i64,
}
