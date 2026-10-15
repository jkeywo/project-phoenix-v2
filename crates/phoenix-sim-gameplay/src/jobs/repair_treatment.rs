//! Local care borrows the one actual Repair team commitment and physical cursor.
use super::*;
use crate::personnel::{CompiledPersonnel, PersonnelCondition, PersonnelState};
use crate::ship::interior::TeamLocation;
use phoenix_model::job::JobId;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TreatmentWork {
    pub patient: String,
    pub condition: PersonnelCondition,
    pub progress: f32,
    pub work_secs: f32,
    pub medication_used: bool,
    pub waiting_medication: bool,
    pub completed: bool,
}
impl RepairTeams {
    pub fn is_treatment_team(&self, team: usize) -> bool {
        self.state
            .teams
            .get(team)
            .and_then(Option::as_ref)
            .is_some_and(|job| {
                job.outcome.is_none() && matches!(job.activity, RepairActivity::Treatment { .. })
            })
    }
    pub fn claim_treatment(
        &mut self,
        profile: &CompiledPersonnel,
        personnel: &PersonnelState,
        patient: &str,
        team: usize,
    ) -> Option<JobId> {
        if !personnel
            .treatment_teams(profile, &self.state, patient)
            .contains(&(team as u32))
            || !self.free_team_indices(None).contains(&team)
        {
            return None;
        }
        let care = profile.config.care.as_ref()?;
        let room = profile.care_compartment.as_ref()?;
        if self.refit_claims_system(&care.system) {
            return None;
        }
        if !self.state.assign(
            team,
            RepairActivity::Treatment {
                phase: TeamSlot::Travelling {
                    system_id: Some(care.system.clone()),
                    display_name: None,
                    elapsed: 0.0,
                    priority: None,
                },
                cancelled: false,
                work: TreatmentWork {
                    patient: patient.into(),
                    condition: personnel.officers.get(patient)?.condition,
                    work_secs: care.work_secs(personnel.officers.get(patient)?.condition)?,
                    progress: 0.0,
                    medication_used: false,
                    waiting_medication: false,
                    completed: false,
                },
            },
        ) {
            return None;
        }
        let cursor = self.state.navigation.as_mut()?.get_mut(team)?;
        cursor.destination = Some(room.clone());
        cursor.route.clear();
        cursor.blocked = false;
        self.refresh();
        self.state.teams[team].as_ref().map(|job| job.id)
    }
    pub fn treatment_claims_system(&self, system: &SystemId) -> bool {
        self.state.teams.iter().flatten().any(|job|job.outcome.is_none() && matches!(&job.activity,RepairActivity::Treatment {phase:TeamSlot::Travelling {system_id:Some(target),..} | TeamSlot::Repairing {system_id:Some(target),..},..} if target==system))
    }
    pub(super) fn return_treatment(&mut self, team: usize) -> bool {
        let Some(job) = self
            .state
            .teams
            .get_mut(team)
            .and_then(Option::as_mut)
            .filter(|job| job.outcome.is_none())
        else {
            return false;
        };
        let RepairActivity::Treatment {
            phase,
            cancelled,
            work,
        } = &mut job.activity
        else {
            return false;
        };
        if matches!(phase, TeamSlot::Returning { .. }) {
            return false;
        }
        *cancelled = true;
        work.waiting_medication = false;
        *phase = TeamSlot::Returning {
            remaining: 0.0,
            system_id: None,
            display_name: None,
            queued_system_id: None,
            queued_display_name: None,
        };
        self.refresh();
        true
    }
    pub(super) fn validate_treatment_shape(&self, state: &RepairJobState) -> Result<(), String> {
        let mut patients = BTreeSet::new();
        for (index, entry) in state.teams.iter().enumerate() {
            let Some(job) = entry else {
                continue;
            };
            let RepairActivity::Treatment {
                phase,
                cancelled,
                work,
            } = &job.activity
            else {
                continue;
            };
            if work.patient.is_empty()
                || work.condition == PersonnelCondition::Fit
                || !work.work_secs.is_finite()
                || work.work_secs <= 0.0
                || !work.progress.is_finite()
                || work.progress < 0.0
                || work.progress > work.work_secs
                || (work.progress > 0.0 && !work.medication_used)
                || (work.completed && (!work.medication_used || *cancelled))
                || (job.outcome.is_none() && !patients.insert(&work.patient))
            {
                return Err("invalid treatment patient, work or exclusive claim".into());
            }
            let cursor = state
                .navigation
                .as_ref()
                .and_then(|rows| rows.get(index))
                .ok_or("treatment requires actual navigation")?;
            let layout = self
                .layout
                .as_ref()
                .ok_or("treatment requires accepted actual layout")?;
            if matches!(cursor.location, TeamLocation::External { .. })
                || (job.outcome.is_some()
                    && !matches!(&cursor.location,TeamLocation::Compartment {compartment} if compartment==&layout.config.team_homes[index]))
            {
                return Err("treatment release requires actual home custody".into());
            }
            match phase {
                TeamSlot::Travelling {
                    elapsed,
                    system_id: Some(_),
                    priority: None,
                    ..
                } if *elapsed == 0.0
                    && !*cancelled
                    && !work.completed
                    && work.progress == 0.0
                    && !work.medication_used => {}
                TeamSlot::Repairing {
                    system_id: Some(_),
                    priority: None,
                    priority_system_id: None,
                    ..
                } if !*cancelled && !work.completed => {}
                TeamSlot::Returning {
                    remaining,
                    system_id: None,
                    queued_system_id: None,
                    ..
                } if *remaining == 0.0
                    && (*cancelled || work.completed)
                    && !work.waiting_medication => {}
                _ => return Err("treatment uses a closed physical work/return lifecycle".into()),
            }
            if job.outcome.is_some_and(|outcome| {
                outcome
                    != if work.completed {
                        JobOutcome::Completed
                    } else {
                        JobOutcome::Cancelled
                    }
            }) {
                return Err("treatment terminal outcome disagrees with actual work".into());
            }
        }
        Ok(())
    }
    /// The normal repair adapter supplies real medication transactions and paid
    /// machinery fractions. Arrival consumes a tick; work never advances in transit.
    pub fn tick_treatments(
        &mut self,
        profile: &CompiledPersonnel,
        personnel: &mut PersonnelState,
        tick: u64,
        dt: f32,
        blocked: &BTreeSet<String>,
        mut medication: impl FnMut(bool, &str, &[phoenix_model::damage::PartRequirement]) -> bool,
        mut grant: impl FnMut(&SystemId, f32) -> f32,
    ) {
        if !dt.is_finite() || dt <= 0.0 || personnel.validate(profile, &self.state, tick).is_err() {
            return;
        }
        let (Some(care), Some(room), Some(layout), Some(navigation)) = (
            &profile.config.care,
            &profile.care_compartment,
            &self.layout,
            &mut self.state.navigation,
        ) else {
            return;
        };
        for (index, entry) in self.state.jobs.teams.iter_mut().enumerate() {
            let Some(job) = entry.as_mut().filter(|job| {
                job.outcome.is_none() && matches!(job.activity, RepairActivity::Treatment { .. })
            }) else {
                continue;
            };
            let enabled = evaluate(&self.program, job);
            let RepairActivity::Treatment {
                phase,
                cancelled,
                work,
            } = &mut job.activity
            else {
                unreachable!()
            };
            let cursor = &mut navigation[index];
            if !enabled && !matches!(phase, TeamSlot::Returning { .. }) {
                if let TeamLocation::Crossing { to, .. } = cursor.location.clone() {
                    layout.advance(cursor, &to, dt, blocked);
                    cursor.destination = Some(room.clone());
                    cursor.route.clear();
                }
                continue;
            }
            match phase {
                TeamSlot::Travelling { .. } => {
                    if layout.advance(cursor, room, dt, blocked) {
                        *phase = TeamSlot::Repairing {
                            system_id: Some(care.system.clone()),
                            display_name: None,
                            priority: None,
                            priority_system_id: None,
                        };
                    }
                }
                TeamSlot::Repairing { .. } => {
                    if !matches!(&cursor.location,TeamLocation::Compartment {compartment} if compartment==room)
                    {
                        job.workflow_error = true;
                        job.paused = true;
                        continue;
                    }
                    let claim = format!("treatment/{}", job.id.0);
                    work.waiting_medication =
                        !work.medication_used && !medication(false, &claim, &care.requirements);
                    if work.waiting_medication {
                        job.paused = true;
                        continue;
                    }
                    let remaining = (care.work_secs(work.condition).expect("validated condition")
                        - work.progress)
                        .max(0.0);
                    let offered = dt.min(remaining);
                    let fraction = grant(&care.system, offered);
                    if !fraction.is_finite() || fraction <= 0.0 {
                        job.paused = true;
                        continue;
                    }
                    if !work.medication_used {
                        if !medication(true, &claim, &care.requirements) {
                            work.waiting_medication = true;
                            job.paused = true;
                            continue;
                        }
                        work.medication_used = true;
                    }
                    work.progress = (work.progress + offered * fraction.min(1.0))
                        .min(care.work_secs(work.condition).expect("validated condition"));
                    job.paused = false;
                    if work.progress >= care.work_secs(work.condition).expect("validated condition")
                    {
                        let patient = personnel
                            .officers
                            .get_mut(&work.patient)
                            .expect("validated actual patient");
                        patient.condition = PersonnelCondition::Fit;
                        patient.changed_tick = tick;
                        work.completed = true;
                        *phase = TeamSlot::Returning {
                            remaining: 0.0,
                            system_id: None,
                            display_name: None,
                            queued_system_id: None,
                            queued_display_name: None,
                        };
                    }
                }
                TeamSlot::Returning { .. } => {
                    if layout.advance(cursor, &layout.config.team_homes[index], dt, blocked) {
                        job.outcome = Some(if work.completed {
                            JobOutcome::Completed
                        } else {
                            JobOutcome::Cancelled
                        });
                        job.paused = false;
                    }
                }
                TeamSlot::Idle => {
                    *cancelled = true;
                    job.workflow_error = true;
                    job.paused = true;
                }
            }
        }
        self.refresh();
    }
}

#[cfg(test)]
#[path = "repair_treatment_tests.rs"]
mod tests;
