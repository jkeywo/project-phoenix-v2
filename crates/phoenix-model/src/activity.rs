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

/// One accepted execution reference. Machinery and radio retain their own
/// actual runtime state; the mission stores only this scoped identity.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActivityExecution {
    pub actor: String,
    pub owner: SystemId,
    pub identity: ActivityExecutionIdentity,
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ActivityExecutionIdentity {
    Physical {
        job: JobId,
    },
    Communication {
        origin_id: String,
        recipient: String,
        emitted_tick: u64,
    },
}
impl From<ActivityJob> for ActivityExecution {
    fn from(value: ActivityJob) -> Self {
        Self {
            actor: value.actor,
            owner: value.owner,
            identity: ActivityExecutionIdentity::Physical { job: value.job },
        }
    }
}
impl ActivityExecution {
    pub fn physical_job(&self) -> Option<JobId> {
        match self.identity {
            ActivityExecutionIdentity::Physical { job } => Some(job),
            _ => None,
        }
    }
    pub fn validate(&self, now: u64) -> Result<(), String> {
        let identity = |value: &str| {
            !value.trim().is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
        };
        if !identity(&self.actor) || !identity(&self.owner.0) {
            return Err("invalid activity execution owner".into());
        }
        match &self.identity {
            ActivityExecutionIdentity::Physical { job } if job.0 > 0 => Ok(()),
            ActivityExecutionIdentity::Communication {
                origin_id,
                recipient,
                emitted_tick,
            } if identity(origin_id)
                && identity(recipient)
                && recipient != &self.actor
                && *emitted_tick <= now =>
            {
                Ok(())
            }
            _ => Err("invalid activity execution identity/clock".into()),
        }
    }
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
    CommunicationRead {
        reference: crate::comms_request::CommsRequestReference,
        read_tick: u64,
        receipt_origin_id: String,
    },
    CommunicationReturned {
        outcome: crate::comms_request::CommsRequestOutcome,
        receipt_origin_id: String,
    },
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
    CommunicationRead {},
    CommunicationReturned {},
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActivityFact {
    pub instance: String,
    pub scope: ActivityExecution,
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
            ActivityOutcome::CommunicationRead { .. } => ActivityEvidenceKey::CommunicationRead {},
            ActivityOutcome::CommunicationReturned { .. } => {
                ActivityEvidenceKey::CommunicationReturned {}
            }
        }
    }
    pub fn is_achievement(&self) -> bool {
        matches!(
            self.outcome,
            ActivityOutcome::RepairAchieved { .. } | ActivityOutcome::CommunicationRead { .. }
        )
    }
    pub fn validate(&self, now: u64) -> Result<(), String> {
        if self.instance.trim().is_empty()
            || self.scope.actor.trim().is_empty()
            || self.scope.owner.0.trim().is_empty()
            || self.tick > now
        {
            return Err("invalid activity evidence identity/tick".into());
        }
        self.scope.validate(now)?;
        match &self.outcome {
            ActivityOutcome::CommunicationRead {
                reference,
                read_tick,
                receipt_origin_id,
            } => {
                self.validate_communication(reference, receipt_origin_id)?;
                if *read_tick < reference.heard_tick || *read_tick > self.tick {
                    return Err(
                        "communication read evidence must precede its actual heard receipt".into(),
                    );
                }
                return Ok(());
            }
            ActivityOutcome::CommunicationReturned {
                outcome,
                receipt_origin_id,
            } => {
                self.validate_communication(&outcome.request, receipt_origin_id)?;
                outcome.validate(self.tick).map_err(str::to_owned)?;
                if outcome.facts.decision.is_none() {
                    return Err("communication terminal evidence requires an actual reported or declined decision".into());
                }
                return Ok(());
            }
            _ if self.scope.physical_job().is_none() => {
                return Err("physical outcome has a nonphysical execution identity".into());
            }
            _ => {}
        }
        match &self.outcome {
            ActivityOutcome::CommunicationRead { .. }
            | ActivityOutcome::CommunicationReturned { .. } => {
                unreachable!("handled communication evidence")
            }
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
    fn validate_communication(
        &self,
        reference: &crate::comms_request::CommsRequestReference,
        receipt: &str,
    ) -> Result<(), String> {
        reference.validate(self.tick).map_err(str::to_owned)?;
        let ActivityExecutionIdentity::Communication {
            origin_id,
            recipient,
            emitted_tick,
        } = &self.scope.identity
        else {
            return Err("communication evidence has a physical execution identity".into());
        };
        if reference.origin_id != *origin_id
            || reference.source_ship != self.scope.actor
            || reference.recipient_ship != *recipient
            || reference.emitted_tick != *emitted_tick
            || receipt.trim().is_empty()
            || receipt.len() > 256
            || receipt.chars().any(char::is_control)
        {
            return Err("communication evidence disagrees with its actual emission lineage".into());
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
