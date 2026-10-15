//! Fitting borrows an ordinary repair team and its physical route. Equipment,
//! parts and fitting-stage progress remain with the fitting transaction owner.
use super::*;
use crate::ship::interior::TeamLocation;
use phoenix_model::job::JobId;

impl RepairTeams {
    pub fn is_refit_team(&self, team: usize) -> bool {
        self.state
            .teams
            .get(team)
            .and_then(Option::as_ref)
            .is_some_and(|job| {
                job.outcome.is_none() && matches!(job.activity, RepairActivity::Refit { .. })
            })
    }

    pub fn refit_job(&self, request: u64) -> Option<JobId> {
        self.state
            .teams
            .iter()
            .flatten()
            .find_map(|job| match &job.activity {
                RepairActivity::Refit { request: value, .. }
                    if *value == request && job.outcome.is_none() =>
                {
                    Some(job.id)
                }
                _ => None,
            })
    }

    pub fn refit_claims_system(&self, system: &SystemId) -> bool {
        self.state.teams.iter().flatten().any(|job| {
            job.outcome.is_none()
                && matches!(&job.activity, RepairActivity::Refit { target, .. } if target == system)
        })
    }

    /// Assignment requires a physical home and a genuinely free existing team.
    /// Repair on the same System must first finish or be recalled.
    pub fn claim_refit(
        &mut self,
        team: usize,
        request: u64,
        target: SystemId,
        label: String,
    ) -> Option<JobId> {
        if request == 0
            || self.refit_job(request).is_some()
            || self
                .layout
                .as_ref()?
                .compartment_for_system(&target)
                .is_none()
            || !self.free_team_indices(None).contains(&team)
            || self.state.physical_work.values().any(|work| {
                work.system == target || work.target.iter().chain(work.requested_target.iter()).any(|target_work|
                    matches!(target_work, phoenix_model::damage::RepairWorkTarget::Component { system, .. } if system == &target))
            })
            || self.state.teams.iter().flatten().any(|job| {
                job.outcome.is_none()
                    && match &job.activity {
                        RepairActivity::Internal { phase, .. } => {
                            phase_targets(phase).any(|system| system == &target)
                        }
                        RepairActivity::Refit { target: other, .. } => other == &target,
                        _ => false,
                    }
            })
        {
            return None;
        }
        if !self.state.assign(
            team,
            RepairActivity::Refit {
                request,
                target: target.clone(),
                cancelled: false,
                phase: TeamSlot::Travelling {
                    system_id: Some(target),
                    display_name: Some(label),
                    elapsed: 0.0,
                    priority: None,
                },
            },
        ) {
            return None;
        }
        self.refresh();
        self.state.teams[team].as_ref().map(|job| job.id)
    }

    /// Call once from the fitting adapter, separately from ordinary repair.
    /// Arrival consumes this tick; on-site permission is evaluated next tick.
    pub fn tick_refits(&mut self, dt: f32, blocked: &BTreeSet<String>) {
        if !dt.is_finite() || dt <= 0.0 {
            return;
        }
        let (Some(layout), Some(navigation)) = (&self.layout, &mut self.state.navigation) else {
            return;
        };
        for (team, entry) in self.state.jobs.teams.iter_mut().enumerate() {
            let Some(job) = entry.as_mut().filter(|job| {
                job.outcome.is_none() && matches!(job.activity, RepairActivity::Refit { .. })
            }) else {
                continue;
            };
            let enabled = evaluate(&self.program, job);
            if job
                .runtime
                .root
                .as_ref()
                .is_some_and(|root| root.result.is_some())
                && !matches!(
                    job.activity,
                    RepairActivity::Refit {
                        phase: TeamSlot::Returning { .. },
                        ..
                    }
                )
            {
                job.workflow_error = true;
                job.paused = true;
            }
            let RepairActivity::Refit {
                target,
                phase,
                cancelled,
                ..
            } = &mut job.activity
            else {
                unreachable!()
            };
            let cursor = &mut navigation[team];
            if !enabled || job.paused {
                // A committed crossing clears even when the workflow pauses.
                if let TeamLocation::Crossing { to, .. } = cursor.location.clone() {
                    layout.advance(cursor, &to, dt, blocked);
                }
                continue;
            }
            match phase.clone() {
                TeamSlot::Travelling { display_name, .. } => {
                    let Some(room) = layout.compartment_for_system(target) else {
                        job.workflow_error = true;
                        job.paused = true;
                        continue;
                    };
                    if layout.advance(cursor, room, dt, blocked) {
                        *phase = TeamSlot::Repairing {
                            system_id: Some(target.clone()),
                            display_name,
                            priority: None,
                            priority_system_id: None,
                        };
                        // Do not reuse advance_travel as permission for machinery.
                        job.paused = true;
                    }
                }
                TeamSlot::Returning { .. } => {
                    if layout.advance(cursor, &layout.config.team_homes[team], dt, blocked) {
                        job.outcome = Some(if job.workflow_error {
                            JobOutcome::Failed
                        } else if *cancelled {
                            JobOutcome::Cancelled
                        } else {
                            JobOutcome::Completed
                        });
                        job.paused = false;
                    }
                }
                TeamSlot::Repairing { .. } => {
                    if !matches!(&cursor.location, TeamLocation::Compartment { compartment } if Some(compartment) == layout.compartment_for_system(target))
                    {
                        job.workflow_error = true;
                        job.paused = true;
                    }
                }
                TeamSlot::Idle => {
                    job.workflow_error = true;
                    job.paused = true;
                }
            }
        }
        self.refresh();
    }

    pub fn refit_effect_enabled(&self, id: JobId) -> bool {
        self.state.teams.iter().enumerate().any(|(team, entry)| {
            let Some(job) = entry.as_ref().filter(|job| job.id == id && job.outcome.is_none() && job.execution_tick > 0 && !job.paused && !job.workflow_error) else { return false; };
            let RepairActivity::Refit { target, phase: TeamSlot::Repairing { .. }, cancelled: false, .. } = &job.activity else { return false; };
            matches!((self.layout.as_ref(), self.state.navigation.as_ref().and_then(|rows| rows.get(team))),
                (Some(layout), Some(cursor)) if matches!(&cursor.location, TeamLocation::Compartment { compartment } if Some(compartment) == layout.compartment_for_system(target)))
        })
    }

    /// Start the physical trip home after installation commits. Until arrival,
    /// this JobId still owns the team and cannot be reused by repair or refit.
    pub fn finish_refit(&mut self, id: JobId) -> bool {
        if !self.refit_effect_enabled(id) {
            return false;
        }
        let team = self.state.teams.iter().position(|entry| {
            entry.as_ref().is_some_and(|job| {
                job.id == id
                    && job.outcome.is_none()
                    && matches!(job.activity, RepairActivity::Refit { .. })
            })
        });
        team.is_some_and(|team| self.return_refit(team, false))
    }

    /// Cancellation is valid during travel as well as on site. It preserves
    /// the actual cursor; the fitting owner releases its unconsumed claims.
    pub fn recall_refit(&mut self, id: JobId) -> bool {
        let team = self.state.teams.iter().position(|entry| {
            entry.as_ref().is_some_and(|job| {
                job.id == id
                    && job.outcome.is_none()
                    && matches!(job.activity, RepairActivity::Refit { .. })
            })
        });
        team.is_some_and(|team| self.return_refit(team, true))
    }

    pub(super) fn return_refit(&mut self, team: usize, cancel: bool) -> bool {
        let Some(job) = self
            .state
            .teams
            .get_mut(team)
            .and_then(Option::as_mut)
            .filter(|job| job.outcome.is_none())
        else {
            return false;
        };
        let RepairActivity::Refit {
            target,
            phase,
            cancelled,
            ..
        } = &mut job.activity
        else {
            return false;
        };
        if matches!(phase, TeamSlot::Returning { .. }) {
            return false;
        }
        let display_name = match phase {
            TeamSlot::Travelling { display_name, .. }
            | TeamSlot::Repairing { display_name, .. } => display_name.clone(),
            _ => None,
        };
        *phase = TeamSlot::Returning {
            remaining: self.timings.travel_duration,
            system_id: Some(target.clone()),
            display_name,
            queued_system_id: None,
            queued_display_name: None,
        };
        *cancelled = cancel;
        job.paused = true;
        self.refresh();
        true
    }

    pub(super) fn validate_refit_work(&self, saved: &RepairJobState) -> Result<(), String> {
        let mut requests = BTreeSet::new();
        let mut targets = BTreeSet::new();
        for (team, entry) in saved.teams.iter().enumerate() {
            let Some(job) = entry else {
                continue;
            };
            let RepairActivity::Refit {
                request,
                target,
                phase,
                cancelled,
            } = &job.activity
            else {
                continue;
            };
            let layout = self
                .layout
                .as_ref()
                .ok_or("refit team requires physical interior")?;
            let room = layout
                .compartment_for_system(target)
                .ok_or("unknown refit System home")?;
            if *request == 0
                || phase_system(phase) != Some(target)
                || saved.physical_work.contains_key(&job.id.0)
                || saved.external_physical_work.contains_key(&job.id.0)
                || matches!(phase, TeamSlot::Idle)
                || (*cancelled && !matches!(phase, TeamSlot::Returning { .. }))
                || matches!(
                    phase,
                    TeamSlot::Repairing {
                        priority: Some(_),
                        ..
                    } | TeamSlot::Repairing {
                        priority_system_id: Some(_),
                        ..
                    } | TeamSlot::Travelling {
                        priority: Some(_),
                        ..
                    } | TeamSlot::Returning {
                        queued_system_id: Some(_),
                        ..
                    } | TeamSlot::Returning {
                        queued_display_name: Some(_),
                        ..
                    }
                )
            {
                return Err("invalid refit team commitment".into());
            }
            if job.outcome.is_none() && (!requests.insert(*request) || !targets.insert(target)) {
                return Err("duplicate refit request or System claim".into());
            }
            let cursor = saved
                .navigation
                .as_ref()
                .and_then(|rows| rows.get(team))
                .ok_or("refit team has no physical cursor")?;
            if job.outcome.is_none()
                && matches!(phase, TeamSlot::Repairing { .. })
                && !matches!(&cursor.location, TeamLocation::Compartment { compartment } if compartment == room)
            {
                return Err("refit work is not in the target compartment".into());
            }
            if job.outcome.is_some()
                && (!matches!(phase, TeamSlot::Returning { .. })
                    || !matches!(&cursor.location, TeamLocation::Compartment { compartment } if Some(compartment) == layout.config.team_homes.get(team)))
            {
                return Err("finished refit team has not returned home".into());
            }
        }
        for job in saved
            .teams
            .iter()
            .flatten()
            .filter(|job| job.outcome.is_none())
        {
            if let RepairActivity::Internal { phase, .. } = &job.activity {
                let conflict = match phase {
                    TeamSlot::Returning {
                        queued_system_id, ..
                    } => queued_system_id
                        .as_ref()
                        .is_some_and(|system| targets.contains(system)),
                    _ => phase_targets(phase).any(|system| targets.contains(system)),
                };
                if conflict {
                    return Err("repair and refit claim the same System".into());
                }
            }
        }
        for work in saved.physical_work.values() {
            if targets.contains(&work.system) || work.target.iter().chain(work.requested_target.iter()).any(|target|
                matches!(target, phoenix_model::damage::RepairWorkTarget::Component { system, .. } if targets.contains(system))) {
                return Err("physical repair and refit claim the same hardware".into());
            }
        }
        Ok(())
    }
}

fn phase_system(phase: &TeamSlot) -> Option<&SystemId> {
    match phase {
        TeamSlot::Travelling { system_id, .. }
        | TeamSlot::Repairing { system_id, .. }
        | TeamSlot::Returning { system_id, .. } => system_id.as_ref(),
        TeamSlot::Idle => None,
    }
}

fn phase_targets(phase: &TeamSlot) -> impl Iterator<Item = &SystemId> {
    let secondary = match phase {
        TeamSlot::Repairing {
            priority_system_id, ..
        } => priority_system_id.as_ref(),
        TeamSlot::Returning {
            queued_system_id, ..
        } => queued_system_id.as_ref(),
        _ => None,
    };
    phase_system(phase).into_iter().chain(secondary)
}

#[cfg(test)]
#[path = "repair_refit_tests.rs"]
mod tests;
