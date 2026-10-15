//! Canonical named-team commitments, with repair as the first workflow consumer.
pub mod activity_bindings;
pub mod repair;
mod repair_navigation;
pub mod repair_workflow;
pub mod security;
use crate::ai::state_tree::TreeRuntime;
use crate::core::messages::SystemId;
use phoenix_model::job::{JobId, JobOutcome};
pub use repair::RepairJobState;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Job<A> {
    pub id: JobId,
    pub owner: SystemId,
    pub activity: A,
    pub runtime: TreeRuntime,
    pub execution_tick: u64,
    pub outcome: Option<JobOutcome>,
    pub paused: bool,
    pub workflow_error: bool,
}
/// Team index is the unique commitment key. A terminal row retains only that
/// team's last outcome; accepting its next assignment replaces the row.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TeamJobs<A> {
    pub owner: SystemId,
    pub next_id: u64,
    pub teams: Vec<Option<Job<A>>>,
}
impl<A> TeamJobs<A> {
    pub fn new(count: usize, owner: SystemId) -> Self {
        Self {
            owner,
            next_id: 1,
            teams: (0..count).map(|_| None).collect(),
        }
    }
    pub fn assign(&mut self, team: usize, activity: A) -> bool {
        if team >= self.teams.len() || self.next_id == u64::MAX {
            return false;
        }
        if self.teams[team]
            .as_ref()
            .is_some_and(|job| job.outcome.is_none())
        {
            return false;
        }
        self.teams[team] = Some(Job {
            id: JobId(self.next_id),
            owner: self.owner.clone(),
            activity,
            runtime: TreeRuntime::default(),
            execution_tick: 0,
            outcome: None,
            paused: false,
            workflow_error: false,
        });
        self.next_id += 1;
        true
    }
}
