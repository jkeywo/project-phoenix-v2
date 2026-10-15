//! Security's finite commitments use the same JobId mint and team-slot owner.
use super::TeamJobs;
use crate::core::messages::SystemId;
use crate::security::teams::{SecurityAction, SecurityTeam, SecurityTeamState};
use phoenix_model::job::JobOutcome;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

const IDLE: SecurityTeam = SecurityTeam {
    state: SecurityTeamState::Available,
    target: None,
    action: None,
    risk: 0.0,
    elapsed: 0.0,
    phase_duration: 0.0,
};

const UNAVAILABLE: SecurityTeam = SecurityTeam {
    state: SecurityTeamState::Unavailable,
    ..IDLE
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecurityActivity {
    pub team: SecurityTeam,
    /// A work result is not a released physical team. Move it to Job.outcome
    /// only once that same team has completed its actual return.
    pub returning_outcome: Option<JobOutcome>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecurityJobs {
    pub jobs: TeamJobs<SecurityActivity>,
    /// The sole physical route owner for explicitly local profiles.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub navigation: Option<Vec<phoenix_model::interior::RouteCursor>>,
    /// Readout only, re-derived from actual machinery availability each tick.
    #[serde(skip)]
    unavailable: bool,
}

impl SecurityJobs {
    pub fn new(count: usize, owner: SystemId) -> Self {
        Self {
            jobs: TeamJobs::new(count, owner),
            navigation: None,
            unavailable: false,
        }
    }
    fn read_team<'a>(&'a self, job: Option<&'a super::Job<SecurityActivity>>) -> &'a SecurityTeam {
        let team = job.map_or(&IDLE, |job| &job.activity.team);
        if self.unavailable && team.is_available() {
            &UNAVAILABLE
        } else {
            team
        }
    }
    pub fn saved(&self) -> Self {
        let mut state = self.clone();
        state.unavailable = false;
        state
    }
    pub fn set_unavailable(&mut self, value: bool) {
        self.unavailable = value;
    }
    pub fn get(&self, index: usize) -> Option<&SecurityTeam> {
        self.jobs
            .teams
            .get(index)
            .map(|job| self.read_team(job.as_ref()))
    }
    pub fn get_mut(&mut self, index: usize) -> Option<&mut SecurityTeam> {
        self.jobs
            .teams
            .get_mut(index)?
            .as_mut()
            .map(|job| &mut job.activity.team)
    }
    pub fn iter(&self) -> impl ExactSizeIterator<Item = &SecurityTeam> {
        self.jobs
            .teams
            .iter()
            .map(|job| self.read_team(job.as_ref()))
    }
    pub fn len(&self) -> usize {
        self.jobs.teams.len()
    }
    pub fn is_empty(&self) -> bool {
        self.jobs.teams.is_empty()
    }
    pub fn has_history(&self) -> bool {
        self.jobs.next_id != 1 || self.jobs.teams.iter().any(Option::is_some)
    }
    pub fn deploy(
        &mut self,
        index: usize,
        target: String,
        action: SecurityAction,
        risk: f32,
        crossing: f32,
    ) -> bool {
        if !self.get(index).is_some_and(SecurityTeam::is_available)
            || target.is_empty()
            || !risk.is_finite()
            || !(0.0..=1.0).contains(&risk)
            || !crossing.is_finite()
            || crossing < 0.0
        {
            return false;
        }
        let mut team = SecurityTeam::default();
        team.deploy(target, action, risk, crossing);
        self.jobs.assign(
            index,
            SecurityActivity {
                team,
                returning_outcome: None,
            },
        )
    }
    pub fn withdraw(&mut self, index: usize, duration: f32, outcome: JobOutcome) {
        if let Some(job) = self.jobs.teams.get_mut(index).and_then(Option::as_mut) {
            if job.outcome.is_none() && job.activity.team.is_interruptible() {
                job.activity.team.withdraw(duration);
                job.activity.returning_outcome = Some(outcome);
                job.paused = false;
            }
        }
    }
    pub fn home(&mut self, index: usize) {
        if let Some(job) = self.jobs.teams.get_mut(index).and_then(Option::as_mut) {
            if job.outcome.is_some() {
                return;
            }
            job.activity.team.arrive_home();
            job.outcome = Some(
                job.activity
                    .returning_outcome
                    .take()
                    .unwrap_or(JobOutcome::Failed),
            );
            job.paused = false;
        }
    }
    pub fn work_list(&self) -> Vec<phoenix_model::job::JobWorkItem> {
        use phoenix_model::job::{JobPhase, JobWorkItem};
        self.jobs
            .teams
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| {
                let job = entry.as_ref()?;
                let team = &job.activity.team;
                Some(JobWorkItem {
                    id: job.id,
                    owner: job.owner.clone(),
                    team_idx: index as u32,
                    phase: match team.state {
                        SecurityTeamState::Deploying => JobPhase::Travelling,
                        SecurityTeamState::Working => JobPhase::Working,
                        SecurityTeamState::Withdrawing => JobPhase::Returning,
                        _ => JobPhase::Finished,
                    },
                    target_system: None,
                    target_entity: team.target.clone(),
                    outcome: job.outcome,
                    paused: job.paused
                        || self
                            .navigation
                            .as_ref()
                            .and_then(|rows| rows.get(index))
                            .is_some_and(|cursor| cursor.blocked),
                    workflow_error: job.workflow_error,
                    target_work: None,
                    work_progress: None,
                    waiting_parts: false,
                })
            })
            .collect()
    }
    pub fn validate_config(
        &self,
        config: &crate::security::teams::SecurityConfig,
        tick: u64,
    ) -> Result<(), String> {
        self.validate(
            config.team_count as usize,
            &crate::ship::system_registry::security_system_id(),
            tick,
        )?;
        if config.local.is_some() != self.navigation.is_some() {
            return Err("Security route owner disagrees with accepted local/external mode".into());
        }
        for job in self.jobs.teams.iter().flatten() {
            let team = &job.activity.team;
            let duration = match team.state {
                SecurityTeamState::Deploying => Some(if config.local.is_some() {
                    0.0
                } else {
                    config.deploy_duration_secs
                }),
                SecurityTeamState::Withdrawing => Some(if config.local.is_some() {
                    0.0
                } else {
                    config.withdraw_duration_secs
                }),
                _ => None,
            };
            if duration.is_some_and(|duration| team.phase_duration != duration) {
                return Err("Security crossing duration differs from accepted equipment".into());
            }
        }
        Ok(())
    }
    pub fn validate(&self, count: usize, owner: &SystemId, tick: u64) -> Result<(), String> {
        if &self.jobs.owner != owner || self.len() != count || self.jobs.next_id == 0 {
            return Err(
                "Security job owner, team count or mint disagrees with accepted equipment".into(),
            );
        }
        let mut ids = BTreeSet::new();
        for job in self.jobs.teams.iter().flatten() {
            let team = &job.activity.team;
            if job.owner != *owner
                || job.id.0 == 0
                || job.id.0 >= self.jobs.next_id
                || !ids.insert(job.id)
                || job.execution_tick > tick
                || job.execution_tick != 0
                || job.runtime != Default::default()
                || job.workflow_error
                || job.paused
                || !team.risk.is_finite()
                || !(0.0..=1.0).contains(&team.risk)
                || !team.elapsed.is_finite()
                || team.elapsed < 0.0
                || !team.phase_duration.is_finite()
                || team.phase_duration < 0.0
            {
                return Err("invalid Security job identity, workflow or finite phase".into());
            }
            match team.state {
                SecurityTeamState::Available
                    if job.outcome.is_some()
                        && job.activity.returning_outcome.is_none()
                        && !job.paused
                        && team == &IDLE => {}
                SecurityTeamState::Deploying
                | SecurityTeamState::Working
                | SecurityTeamState::Withdrawing
                    if job.outcome.is_none()
                        && (team.elapsed < team.phase_duration
                            || (team.elapsed == 0.0 && team.phase_duration == 0.0))
                        && team.target.as_ref().is_some_and(|id| !id.is_empty())
                        && team.action.is_some()
                        && (team.state == SecurityTeamState::Withdrawing)
                            == job.activity.returning_outcome.is_some()
                        && (team.state != SecurityTeamState::Working
                            || team.phase_duration > 0.0) => {}
                _ => return Err("Security job phase and physical commitment disagree".into()),
            }
        }
        if ids.iter().map(|id| id.0).max().unwrap_or(0) != self.jobs.next_id - 1 {
            return Err("Security job mint has no matching latest commitment".into());
        }
        Ok(())
    }
}

impl std::ops::Index<usize> for SecurityJobs {
    type Output = SecurityTeam;
    fn index(&self, index: usize) -> &Self::Output {
        self.get(index).expect("Security team index")
    }
}

#[cfg(test)]
#[path = "security_tests.rs"]
mod tests;
