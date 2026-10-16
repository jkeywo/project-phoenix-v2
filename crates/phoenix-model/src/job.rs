//! Permitted job work-list vocabulary; execution remains in Gameplay.
use crate::messages::SystemId;
use serde::{Deserialize, Serialize};

/// Assignment identity scoped to its ship and owning System. A redirect keeps
/// its committed team's job; unrelated System owners have independent mints.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct JobId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobPhase {
    Travelling,
    Working,
    Returning,
    External,
    Finished,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobOutcome {
    Completed,
    Cancelled,
    Failed,
}

/// A work-list row contains issued assignment information, never hidden damage,
/// resource truth or private workflow memory. It grants no command authority.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobWorkItem {
    pub id: JobId,
    pub owner: SystemId,
    pub team_idx: u32,
    pub phase: JobPhase,
    pub target_system: Option<SystemId>,
    pub target_entity: Option<String>,
    pub outcome: Option<JobOutcome>,
    pub paused: bool,
    pub workflow_error: bool,
    pub target_work: Option<crate::damage::RepairWorkTarget>,
    /// Original exact order, distinct from the currently executing remedy stage.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_work: Option<crate::damage::RepairWorkTarget>,
    /// A real generic physical-repair visit; absent exact target alone is not one.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub sweeps_structure: bool,
    /// Completed thousandths of the current bounded stage, not System HP.
    pub work_progress: Option<u16>,
    pub waiting_parts: bool,
}
