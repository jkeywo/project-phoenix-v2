//! Repair's canonical job activity and mechanical adapter. The temporary phase
//! executor is rebuilt from jobs for each operation; its slots are never saved.
use super::repair_workflow;
use super::{Job, TeamJobs};
use crate::ai::state_tree::{self, AiTreeConfig, Program, TreeError};
use crate::core::messages::{SystemId, TeamSlot};
use crate::modifiers::repair_teams::{RepairPhases, RepairTimings};
use crate::ship::{config::ShipConfig, damage::SystemHull};
use phoenix_model::job::{JobOutcome, JobPhase, JobWorkItem};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum RepairActivity {
    Internal { phase: TeamSlot, cancelled: bool },
    External { target: String },
}
pub type RepairJobState = TeamJobs<RepairActivity>;
#[derive(Clone, Debug)]
pub struct RepairTeams {
    state: RepairJobState,
    timings: RepairTimings,
    program: Program,
    readout: Vec<TeamSlot>,
}
impl RepairTeams {
    pub fn new(count: usize) -> Self {
        Self::new_with_timings(count, RepairPhases::new(count).timings())
    }
    pub fn new_with_timings(count: usize, timings: RepairTimings) -> Self {
        Self::new_with_workflow(count, timings, SystemId("repair".into()), None)
            .expect("supported repair workflow")
    }
    pub fn new_with_workflow(
        count: usize,
        timings: RepairTimings,
        owner: SystemId,
        workflow: Option<&AiTreeConfig>,
    ) -> Result<Self, TreeError> {
        Ok(Self {
            state: TeamJobs::new(count, owner),
            timings,
            program: repair_workflow::compile_workflow(workflow)?,
            readout: vec![TeamSlot::Idle; count],
        })
    }
    pub fn timings(&self) -> RepairTimings {
        self.timings
    }
    pub fn slots(&self) -> &[TeamSlot] {
        &self.readout
    }
    fn project_slots(&self) -> Vec<TeamSlot> {
        self.state
            .teams
            .iter()
            .map(|job| match job {
                Some(Job {
                    activity: RepairActivity::Internal { phase, .. },
                    outcome: None,
                    ..
                }) => phase.clone(),
                _ => TeamSlot::Idle,
            })
            .collect()
    }
    fn refresh(&mut self) {
        self.readout = self.project_slots();
    }
    fn phases(&self) -> RepairPhases {
        let mut phases = RepairPhases::new_with_timings(self.state.teams.len(), self.timings);
        phases.restore_slots(&self.project_slots());
        phases
    }
    fn commit_phases(&mut self, phases: &RepairPhases) {
        for (team, phase) in phases.slots().iter().enumerate() {
            if let Some(job) = self.state.teams[team].as_mut() {
                if job.outcome.is_none() {
                    if let RepairActivity::Internal {
                        phase: current,
                        cancelled,
                    } = &mut job.activity
                    {
                        if matches!(phase, TeamSlot::Idle) {
                            job.outcome = Some(if job.workflow_error {
                                JobOutcome::Failed
                            } else if *cancelled {
                                JobOutcome::Cancelled
                            } else {
                                JobOutcome::Completed
                            });
                            job.paused = false;
                        } else {
                            *current = phase.clone();
                            if job
                                .runtime
                                .root
                                .as_ref()
                                .is_some_and(|root| root.result.is_some())
                            {
                                // A terminal workflow cannot abandon physical work.
                                job.workflow_error = true;
                                job.paused = true;
                            }
                        }
                    }
                }
            }
        }
        self.refresh();
    }
    pub fn has_job_history(&self) -> bool {
        self.state.next_id != 1 || self.state.teams.iter().any(Option::is_some)
    }
    pub fn save_state(&self) -> RepairJobState {
        self.state.clone()
    }
    pub fn restore_state(&mut self, state: &RepairJobState) -> Result<(), String> {
        if state.teams.len() != self.state.teams.len()
            || state.owner != self.state.owner
            || state.next_id == 0
        {
            return Err("repair job owner/team count mismatch".into());
        }
        let mut ids = BTreeSet::new();
        let mut external = 0;
        for job in state.teams.iter().flatten() {
            if job.owner != state.owner
                || job.id.0 == 0
                || job.id.0 >= state.next_id
                || !ids.insert(job.id.0)
            {
                return Err("invalid repair job identity".into());
            }
            match &job.activity {
                RepairActivity::External { target } => {
                    if target.is_empty() {
                        return Err("empty external target".into());
                    }
                    if job.outcome.is_none() {
                        external += 1;
                    }
                }
                RepairActivity::Internal { phase, .. } => {
                    if matches!(phase, TeamSlot::Idle) {
                        return Err("internal job has no activity".into());
                    }
                    let valid = match phase {
                        TeamSlot::Travelling { elapsed, .. } => {
                            elapsed.is_finite() && *elapsed >= 0.0
                        }
                        TeamSlot::Returning { remaining, .. } => {
                            remaining.is_finite() && *remaining >= 0.0
                        }
                        _ => true,
                    };
                    if !valid {
                        return Err("invalid repair job timer".into());
                    }
                }
            }
            if match job.runtime.last_tick {
                Some(last) => last.checked_add(1) != Some(job.execution_tick),
                None => job.execution_tick != 0,
            } {
                return Err("invalid repair workflow clock".into());
            }
            state_tree::validate_runtime(&self.program, &job.runtime)
                .map_err(|error| error.to_string())?;
        }
        if external > 1 {
            return Err("multiple external team claims".into());
        }
        self.state = state.clone();
        self.refresh();
        Ok(())
    }
    /// Fixture-only phase seeding; runtime continuation uses restore_state.
    #[cfg(any(test, feature = "test-support"))]
    pub fn restore_slots(&mut self, slots: &[TeamSlot]) {
        for (team, phase) in slots.iter().take(self.state.teams.len()).enumerate() {
            self.state.teams[team] = None;
            if !matches!(phase, TeamSlot::Idle) {
                self.state.assign(
                    team,
                    RepairActivity::Internal {
                        phase: phase.clone(),
                        cancelled: false,
                    },
                );
            }
        }
        self.refresh();
    }
    pub fn on_site_systems(&self) -> impl Iterator<Item = &SystemId> {
        self.readout.iter().filter_map(|slot| match slot {
            TeamSlot::Repairing {
                system_id: Some(id),
                ..
            } => Some(id),
            _ => None,
        })
    }
    pub fn lowest_free_team(&self) -> Option<usize> {
        self.free_team_indices(None).first().copied()
    }
    pub fn free_team_indices(&self, abroad: Option<usize>) -> Vec<usize> {
        self.phases()
            .free_team_indices(abroad)
            .into_iter()
            .filter(|team| self.external_team() != Some(*team))
            .collect()
    }
    pub fn free_team_indices_reserving(&self, abroad: Option<usize>, reserved: u8) -> Vec<usize> {
        let mut free = self.free_team_indices(abroad);
        free.truncate(free.len().saturating_sub(reserved as usize));
        free
    }
    pub fn is_committed_to_operation(&self, team: usize, abroad: Option<usize>) -> bool {
        self.external_team() == Some(team) || self.phases().is_committed_to_operation(team, abroad)
    }
    pub fn dispatch(&mut self, team: usize, target: SystemId, label: String) {
        if team >= self.state.teams.len() || self.external_team() == Some(team) {
            return;
        }
        let mut phases = self.phases();
        phases.dispatch(team, target, label);
        if self.state.teams[team]
            .as_ref()
            .is_none_or(|job| job.outcome.is_some())
        {
            self.state.assign(
                team,
                RepairActivity::Internal {
                    phase: phases.slots()[team].clone(),
                    cancelled: false,
                },
            );
        } else if let Some(job) = self.state.teams[team].as_mut() {
            if let RepairActivity::Internal { cancelled, .. } = &mut job.activity {
                *cancelled = matches!(
                    &phases.slots()[team],
                    TeamSlot::Returning {
                        queued_system_id: None,
                        ..
                    }
                );
            }
        }
        self.commit_phases(&phases);
    }
    pub fn recall(&mut self, team: usize) -> bool {
        let mut phases = self.phases();
        let mut changed = phases.recall(team);
        if !changed {
            if let Some(Some(job)) = self.state.teams.get_mut(team) {
                if job.workflow_error {
                    if let RepairActivity::Internal {
                        phase:
                            TeamSlot::Returning {
                                queued_system_id,
                                queued_display_name,
                                ..
                            },
                        cancelled,
                    } = &mut job.activity
                    {
                        *queued_system_id = None;
                        *queued_display_name = None;
                        *cancelled = true;
                        changed = true;
                        phases = self.phases();
                    }
                }
            }
        }
        if changed {
            if let Some(Some(Job {
                activity: RepairActivity::Internal { cancelled, .. },
                ..
            })) = self.state.teams.get_mut(team)
            {
                *cancelled = true;
            }
            self.commit_phases(&phases);
        }
        changed
    }
    pub fn set_priority(&mut self, team: usize, priority: u8) -> bool {
        let mut p = self.phases();
        let result = p.set_priority(team, priority);
        self.commit_phases(&p);
        result
    }
    pub fn prioritisable_systems(&self, hull: &SystemHull, config: &ShipConfig) -> Vec<SystemId> {
        self.phases().prioritisable_systems(hull, config)
    }
    pub fn prioritise_system(
        &mut self,
        target: &SystemId,
        hull: &SystemHull,
        config: &ShipConfig,
    ) -> Option<usize> {
        let mut p = self.phases();
        let result = p.prioritise_system(target, hull, config);
        self.commit_phases(&p);
        result
    }
    pub fn tick(&mut self, dt: f32, hull: &mut SystemHull, config: Option<&ShipConfig>) {
        self.tick_with_work(dt, hull, config, |_, _| 1.0);
    }

    /// Pay only enabled, on-site repair work. Travel/return and workflow
    /// observation continue with elapsed time, even while tools lack energy.
    pub fn tick_with_work(
        &mut self,
        dt: f32,
        hull: &mut SystemHull,
        config: Option<&ShipConfig>,
        mut grant: impl FnMut(&SystemId, f32) -> f32,
    ) {
        let enabled: Vec<_> = self
            .state
            .teams
            .iter_mut()
            .map(|entry| match entry {
                Some(job)
                    if job.outcome.is_none()
                        && matches!(job.activity, RepairActivity::Internal { .. }) =>
                {
                    evaluate(&self.program, job)
                }
                _ => false,
            })
            .collect();
        let mut phases = self.phases();
        let work: Vec<_> = phases
            .slots()
            .iter()
            .zip(&enabled)
            .map(|(slot, enabled)| match slot {
                TeamSlot::Repairing {
                    system_id: Some(id),
                    ..
                } if *enabled && !hull.is_at_max(id) => {
                    grant(&self.state.owner, dt).clamp(0.0, 1.0)
                }
                _ => 0.0,
            })
            .collect();
        phases.tick_with_repair_work(dt, hull, config, &enabled, &work);
        self.commit_phases(&phases);
    }
    pub fn external_team(&self) -> Option<usize> {
        self.state.teams.iter().position(|job| {
            matches!(
                job,
                Some(Job {
                    activity: RepairActivity::External { .. },
                    outcome: None,
                    ..
                })
            )
        })
    }
    pub fn external_target(&self) -> Option<&str> {
        let job = self.state.teams.get(self.external_team()?)?.as_ref()?;
        match &job.activity {
            RepairActivity::External { target } => Some(target),
            _ => None,
        }
    }
    pub fn claim_external(&mut self, team: usize, target: String) -> bool {
        if target.is_empty() {
            return false;
        }
        if let Some(existing) = self.external_team() {
            if existing != team {
                return false;
            }
            if self.external_target() == Some(target.as_str()) {
                return true;
            }
            if self.state.next_id == u64::MAX {
                return false;
            }
            self.release_external();
        }
        if !self.free_team_indices(None).contains(&team) {
            return false;
        }
        let result = self.state.assign(team, RepairActivity::External { target });
        self.refresh();
        result
    }
    pub fn release_external(&mut self) -> bool {
        let Some(team) = self.external_team() else {
            return false;
        };
        let job = self.state.teams[team].as_mut().expect("external team");
        job.outcome = Some(if job.workflow_error {
            JobOutcome::Failed
        } else {
            JobOutcome::Cancelled
        });
        job.paused = false;
        self.refresh();
        true
    }
    pub fn step_external(&mut self) -> bool {
        let Some(team) = self.external_team() else {
            return false;
        };
        evaluate(
            &self.program,
            self.state.teams[team].as_mut().expect("external team"),
        )
    }
    pub fn external_effect_enabled(&self) -> bool {
        self.external_team()
            .and_then(|team| self.state.teams[team].as_ref())
            .is_some_and(|job| job.execution_tick > 0 && !job.paused && !job.workflow_error)
    }
    pub fn work_list(&self) -> Vec<JobWorkItem> {
        self.state
            .teams
            .iter()
            .enumerate()
            .filter_map(|(team, job)| {
                let job = job.as_ref()?;
                let (phase, target_system, target_entity) = match &job.activity {
                    RepairActivity::External { target } => {
                        (JobPhase::External, None, Some(target.clone()))
                    }
                    RepairActivity::Internal { phase, .. } => match phase {
                        TeamSlot::Travelling { system_id, .. } => {
                            (JobPhase::Travelling, system_id.clone(), None)
                        }
                        TeamSlot::Repairing { system_id, .. } => {
                            (JobPhase::Working, system_id.clone(), None)
                        }
                        TeamSlot::Returning { system_id, .. } => {
                            (JobPhase::Returning, system_id.clone(), None)
                        }
                        TeamSlot::Idle => (JobPhase::Finished, None, None),
                    },
                };
                Some(JobWorkItem {
                    id: job.id,
                    owner: job.owner.clone(),
                    team_idx: team as u32,
                    phase: if job.outcome.is_some() {
                        JobPhase::Finished
                    } else {
                        phase
                    },
                    target_system,
                    target_entity,
                    outcome: job.outcome,
                    paused: job.paused,
                    workflow_error: job.workflow_error,
                })
            })
            .collect()
    }
}
fn facts(activity: &RepairActivity) -> BTreeMap<String, bool> {
    let internal = matches!(activity, RepairActivity::Internal { .. });
    let (travelling, repairing, returning, target) = match activity {
        RepairActivity::Internal { phase, .. } => match phase {
            TeamSlot::Travelling { system_id, .. } => (true, false, false, system_id.is_some()),
            TeamSlot::Repairing { system_id, .. } => (false, true, false, system_id.is_some()),
            TeamSlot::Returning { system_id, .. } => (false, false, true, system_id.is_some()),
            TeamSlot::Idle => (false, false, false, false),
        },
        RepairActivity::External { target } => (false, false, false, !target.is_empty()),
    };
    [
        ("internal", internal),
        ("travelling", travelling),
        ("repairing", repairing),
        ("returning", returning),
        ("external", !internal),
        ("target_present", target),
    ]
    .into_iter()
    .map(|(key, value)| (key.into(), value))
    .collect()
}
fn channels() -> BTreeMap<String, bool> {
    repair_workflow::bindings()
        .channels
        .into_iter()
        .map(|key| (key, false))
        .collect()
}
fn evaluate(program: &Program, job: &mut Job<RepairActivity>) -> bool {
    if job.workflow_error {
        let returning = if let RepairActivity::Internal {
            phase:
                TeamSlot::Returning {
                    queued_system_id,
                    queued_display_name,
                    ..
                },
            cancelled,
        } = &mut job.activity
        {
            // Cleanup never executes repair or starts the queued redirect.
            *queued_system_id = None;
            *queued_display_name = None;
            *cancelled = true;
            true
        } else {
            false
        };
        job.paused = !returning;
        return returning;
    }
    let allowed_channel = match &job.activity {
        RepairActivity::External { .. } => "apply_external",
        RepairActivity::Internal {
            phase: TeamSlot::Travelling { .. },
            ..
        } => "advance_travel",
        RepairActivity::Internal {
            phase: TeamSlot::Repairing { .. },
            ..
        } => "apply_repair",
        RepairActivity::Internal {
            phase: TeamSlot::Returning { .. },
            ..
        } => "advance_return",
        _ => return false,
    };
    let Some(next) = job.execution_tick.checked_add(1) else {
        job.workflow_error = true;
        job.paused = true;
        return false;
    };
    match state_tree::step(
        program,
        &facts(&job.activity),
        &channels(),
        &mut job.runtime,
        job.execution_tick,
    ) {
        Ok(result) => {
            job.execution_tick = next;
            let terminal = job.runtime.root.as_ref().and_then(|root| root.result);
            if terminal == Some(state_tree::Completion::Failure)
                || (terminal.is_some() && matches!(job.activity, RepairActivity::External { .. }))
            {
                job.workflow_error = true;
                job.paused = true;
                return false;
            }
            let enabled = result
                .intents
                .get(allowed_channel)
                .copied()
                .unwrap_or(false);
            job.paused = !enabled;
            enabled
        }
        Err(_) => {
            job.workflow_error = true;
            job.paused = true;
            false
        }
    }
}
#[cfg(test)]
#[path = "repair_tests.rs"]
mod tests;
