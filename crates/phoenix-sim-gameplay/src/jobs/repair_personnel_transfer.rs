//! Actual personnel escort uses the existing exclusive team and physical cursor.
use super::*;
use crate::personnel::{
    CompiledPersonnel, DutyCompetency, OfficerLocation, PersonnelCondition, PersonnelDestination,
    PersonnelDestinationRequest, PersonnelRoster, PersonnelState, PersonnelTransferStage,
};
use crate::ship::interior::TeamLocation;
use phoenix_model::job::JobId;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersonnelTransferPlan {
    pub craft: String,
    pub berth: String,
    pub bay: SystemId,
    pub pickup: OfficerLocation,
    pub pickup_compartment: String,
    pub delivery_compartment: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersonnelTransferWork {
    pub officer: String,
    pub request: PersonnelDestinationRequest,
    pub plan: PersonnelTransferPlan,
    pub stage: PersonnelTransferStage,
    pub completed: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransferAvailability {
    Ready,
    Paused,
    Missing,
}
impl RepairTeams {
    pub fn is_personnel_transfer_team(&self, team: usize) -> bool {
        self.state
            .teams
            .get(team)
            .and_then(Option::as_ref)
            .is_some_and(|job| {
                job.outcome.is_none()
                    && matches!(job.activity, RepairActivity::PersonnelTransfer { .. })
            })
    }
    pub fn claims_personnel_bay(&self, bay: &SystemId) -> bool {
        self.state.teams.iter().flatten().any(|job|job.outcome.is_none() && matches!(&job.activity,RepairActivity::PersonnelTransfer {cancelled:false,work,..} if !work.completed && &work.plan.bay==bay))
    }
    pub fn claims_personnel_craft(&self, craft: &str) -> bool {
        self.state.teams.iter().flatten().any(|job|job.outcome.is_none() && matches!(&job.activity,RepairActivity::PersonnelTransfer {cancelled:false,work,..} if !work.completed && work.plan.craft==craft))
    }
    pub fn personnel_transfer_teams(
        &self,
        profile: &CompiledPersonnel,
        personnel: &PersonnelState,
        officer: &str,
    ) -> Vec<u32> {
        let Some(person) = personnel.officers.get(officer) else {
            return Vec::new();
        };
        if !matches!(person.location,OfficerLocation::AtCompartment {..}|OfficerLocation::AboardCraft {..}) || self.state.teams.iter().flatten().any(|job|job.outcome.is_none() && matches!(&job.activity,RepairActivity::PersonnelTransfer {work,..} if work.officer==officer && !work.completed)) {return Vec::new();}
        if self.state.teams.iter().flatten().any(|job|job.outcome.is_none() && matches!(&job.activity,RepairActivity::Treatment {work,..} if work.patient==officer)) {return Vec::new();}
        let Some(layout) = self.layout.as_ref() else {
            return Vec::new();
        };
        let free = self.free_team_indices(None);
        profile.config.repair_teams.iter().filter_map(|team| {
            let index=team.team_idx as usize;
            (free.contains(&index) && team.competencies.contains(&DutyCompetency::PersonnelTransport)
                && (person.condition!=PersonnelCondition::Severe || team.competencies.contains(&DutyCompetency::MedicalCare))
                && self.state.navigation.as_ref().and_then(|rows|rows.get(index)).is_some_and(|cursor|matches!(&cursor.location,TeamLocation::Compartment {compartment} if layout.config.team_homes.get(index)==Some(compartment))))
                .then_some(team.team_idx)
        }).collect()
    }
    pub fn claim_personnel_transfer(
        &mut self,
        profile: &CompiledPersonnel,
        personnel: &PersonnelState,
        officer: &str,
        team: usize,
        request: PersonnelDestinationRequest,
        plan: PersonnelTransferPlan,
    ) -> Option<JobId> {
        if !self
            .personnel_transfer_teams(profile, personnel, officer)
            .contains(&(team as u32))
            || self.claims_personnel_bay(&plan.bay)
            || self.refit_claims_system(&plan.bay)
            || personnel.officers.get(officer)?.location != plan.pickup
        {
            return None;
        }
        let layout = self.layout.as_ref()?;
        if !layout
            .config
            .compartments
            .iter()
            .any(|room| room.id == plan.pickup_compartment)
            || !layout
                .config
                .compartments
                .iter()
                .any(|room| room.id == plan.delivery_compartment)
            || plan.craft.is_empty()
            || plan.berth.is_empty()
            || plan.bay.0.is_empty()
        {
            return None;
        }
        match (&request.destination, &plan.pickup) {
            (
                PersonnelDestination::Craft { craft },
                OfficerLocation::AtCompartment { compartment },
            ) if craft == &plan.craft && compartment == &plan.pickup_compartment => {}
            (PersonnelDestination::Home, OfficerLocation::AboardCraft { craft, berth })
                if craft == &plan.craft && berth == &plan.berth => {}
            _ => return None,
        }
        // Refuse missing physical state before minting or replacing a Job.
        self.state.navigation.as_ref()?.get(team)?;
        if !self.state.assign(
            team,
            RepairActivity::PersonnelTransfer {
                phase: TeamSlot::Travelling {
                    system_id: Some(self.state.owner.clone()),
                    display_name: None,
                    elapsed: 0.0,
                    priority: None,
                },
                cancelled: false,
                work: PersonnelTransferWork {
                    officer: officer.into(),
                    request,
                    plan: plan.clone(),
                    stage: PersonnelTransferStage::Collecting,
                    completed: false,
                },
            },
        ) {
            return None;
        }
        let cursor = self.state.navigation.as_mut()?.get_mut(team)?;
        cursor.destination = Some(plan.pickup_compartment);
        cursor.route.clear();
        cursor.blocked = false;
        self.refresh();
        self.state.teams[team].as_ref().map(|job| job.id)
    }
    pub(super) fn recall_personnel_transfer(&mut self, team: usize) -> bool {
        let Some(job) = self
            .state
            .teams
            .get_mut(team)
            .and_then(Option::as_mut)
            .filter(|job| job.outcome.is_none())
        else {
            return false;
        };
        let RepairActivity::PersonnelTransfer {
            phase,
            cancelled,
            work,
        } = &mut job.activity
        else {
            return false;
        };
        if work.stage == PersonnelTransferStage::Returning {
            return false;
        }
        *cancelled = true;
        work.stage = PersonnelTransferStage::Returning;
        *phase = return_phase();
        self.refresh();
        true
    }
    pub(super) fn validate_personnel_transfer_shape(
        &self,
        state: &RepairJobState,
    ) -> Result<(), String> {
        let mut people = BTreeSet::new();
        let mut berths = BTreeSet::new();
        for (index, entry) in state.teams.iter().enumerate() {
            let Some(job) = entry else {
                continue;
            };
            let RepairActivity::PersonnelTransfer {
                phase,
                cancelled,
                work,
            } = &job.activity
            else {
                continue;
            };
            let graph = self
                .layout
                .as_ref()
                .ok_or("personnel transfer needs actual interior")?;
            let cursor = state
                .navigation
                .as_ref()
                .and_then(|rows| rows.get(index))
                .ok_or("personnel transfer needs actual team cursor")?;
            if work.officer.is_empty()
                || work.plan.craft.is_empty()
                || work.plan.berth.is_empty()
                || work.plan.bay.0.is_empty()
                || [
                    work.plan.pickup_compartment.as_str(),
                    work.plan.delivery_compartment.as_str(),
                ]
                .iter()
                .any(|id| !graph.config.compartments.iter().any(|room| room.id == *id))
                || (work.completed && *cancelled)
                || matches!(cursor.location, TeamLocation::External { .. })
            {
                return Err("invalid personnel transfer identity, route or result".into());
            }
            match (&work.request.destination, &work.plan.pickup) {
                (
                    PersonnelDestination::Craft { craft },
                    OfficerLocation::AtCompartment { compartment },
                ) if craft == &work.plan.craft && compartment == &work.plan.pickup_compartment => {}
                (PersonnelDestination::Home, OfficerLocation::AboardCraft { craft, berth })
                    if craft == &work.plan.craft && berth == &work.plan.berth => {}
                _ => {
                    return Err(
                        "personnel transfer differs from its accepted physical source/destination"
                            .into(),
                    )
                }
            }
            match (phase, &work.stage) {
                (
                    TeamSlot::Travelling {
                        system_id: Some(owner),
                        elapsed,
                        priority: None,
                        ..
                    },
                    PersonnelTransferStage::Collecting | PersonnelTransferStage::Escorting,
                ) if owner == &state.owner && *elapsed == 0.0 && !*cancelled && !work.completed => {
                }
                (
                    TeamSlot::Returning {
                        remaining,
                        system_id: None,
                        queued_system_id: None,
                        ..
                    },
                    PersonnelTransferStage::Returning,
                ) if *remaining == 0.0 && (*cancelled || work.completed) => {}
                _ => return Err("personnel transfer uses a closed physical lifecycle".into()),
            }
            if job.outcome.is_none() && !work.completed && !people.insert(&work.officer) {
                return Err("personnel transfer duplicates an actual person claim".into());
            }
            if job.outcome.is_none()
                && !work.completed
                && !*cancelled
                && matches!(work.request.destination, PersonnelDestination::Craft { .. })
                && !berths.insert((&work.plan.craft, &work.plan.berth))
            {
                return Err("personnel transfers duplicate an actual berth claim".into());
            }
            if job.outcome.is_some()
                && (!matches!(&cursor.location,TeamLocation::Compartment {compartment} if graph.config.team_homes.get(index)==Some(compartment))
                    || job.outcome
                        != Some(if work.completed {
                            JobOutcome::Completed
                        } else {
                            JobOutcome::Cancelled
                        }))
            {
                return Err("personnel transfer releases only after actual team return".into());
            }
        }
        Ok(())
    }
    /// The adapter supplies actual bay/access readiness. One dt advances only
    /// one route or handoff; requests and claims never create another position.
    pub fn tick_personnel_transfers(
        &mut self,
        actor: &str,
        profile: &CompiledPersonnel,
        roster: &mut PersonnelRoster,
        tick: u64,
        dt: f32,
        blocked: &BTreeSet<String>,
        mut available: impl FnMut(&PersonnelTransferWork, &PersonnelRoster) -> TransferAvailability,
    ) {
        if !dt.is_finite() || dt <= 0.0 {
            return;
        }
        let (Some(graph), Some(navigation)) = (&self.layout, &mut self.state.navigation) else {
            return;
        };
        for (index, entry) in self.state.jobs.teams.iter_mut().enumerate() {
            let Some(job) = entry.as_mut().filter(|job| {
                job.outcome.is_none()
                    && matches!(job.activity, RepairActivity::PersonnelTransfer { .. })
            }) else {
                continue;
            };
            let enabled = evaluate(&self.program, job);
            let RepairActivity::PersonnelTransfer {
                phase,
                cancelled,
                work,
            } = &mut job.activity
            else {
                unreachable!()
            };
            let cursor = &mut navigation[index];
            let readiness = if work.stage == PersonnelTransferStage::Returning {
                TransferAvailability::Ready
            } else {
                available(work, roster)
            };
            let Some(person) = roster
                .state_mut(actor)
                .and_then(|state| state.officers.get_mut(&work.officer))
            else {
                job.workflow_error = true;
                job.paused = true;
                continue;
            };
            let Some(definition) = profile
                .config
                .officers
                .iter()
                .find(|person| person.id == work.officer)
            else {
                job.workflow_error = true;
                job.paused = true;
                continue;
            };
            let carrying = person.location
                == OfficerLocation::WithTransfer {
                    actor: actor.into(),
                    team_idx: index as u32,
                    job: job.id,
                };
            if work.stage != PersonnelTransferStage::Returning
                && (job.workflow_error
                    || matches!(person.location, OfficerLocation::Lost { .. })
                    || readiness == TransferAvailability::Missing)
            {
                *cancelled = true;
                work.stage = PersonnelTransferStage::Returning;
                *phase = return_phase();
            }
            if work.stage != PersonnelTransferStage::Returning
                && (!enabled || readiness == TransferAvailability::Paused)
            {
                if let TeamLocation::Crossing { to, .. } = cursor.location.clone() {
                    graph.advance(cursor, &to, dt, blocked);
                    cursor.route.clear();
                }
                job.paused = true;
                continue;
            }
            match work.stage {
                PersonnelTransferStage::Collecting => {
                    if graph.advance(cursor, &work.plan.pickup_compartment, dt, blocked) {
                        if person.location != work.plan.pickup {
                            job.workflow_error = true;
                            job.paused = true;
                            continue;
                        }
                        person.location = OfficerLocation::WithTransfer {
                            actor: actor.into(),
                            team_idx: index as u32,
                            job: job.id,
                        };
                        person.changed_tick = tick;
                        work.stage = PersonnelTransferStage::Escorting;
                        cursor.destination = Some(work.plan.delivery_compartment.clone());
                        cursor.route.clear();
                    }
                    job.paused = cursor.blocked;
                }
                PersonnelTransferStage::Escorting => {
                    if !carrying {
                        job.workflow_error = true;
                        job.paused = true;
                        continue;
                    }
                    if graph.advance(cursor, &work.plan.delivery_compartment, dt, blocked) {
                        person.location = match &work.request.destination {
                            PersonnelDestination::Craft { craft } => OfficerLocation::AboardCraft {
                                craft: craft.clone(),
                                berth: work.plan.berth.clone(),
                            },
                            PersonnelDestination::Home => OfficerLocation::AtCompartment {
                                compartment: definition.home_compartment.clone(),
                            },
                        };
                        person.changed_tick = tick;
                        work.completed = true;
                        work.stage = PersonnelTransferStage::Returning;
                        *phase = return_phase();
                    }
                    job.paused = cursor.blocked;
                }
                PersonnelTransferStage::Returning => {
                    // Cancellation after collection returns the actual escorted
                    // officer home before releasing the team's own commitment.
                    if carrying {
                        if graph.advance(cursor, &definition.home_compartment, dt, blocked) {
                            person.location = OfficerLocation::AtCompartment {
                                compartment: definition.home_compartment.clone(),
                            };
                            person.changed_tick = tick;
                            cursor.route.clear();
                        }
                        job.paused = cursor.blocked;
                        continue;
                    }
                    if graph.advance(cursor, &graph.config.team_homes[index], dt, blocked) {
                        job.outcome = Some(if work.completed {
                            JobOutcome::Completed
                        } else {
                            JobOutcome::Cancelled
                        });
                        job.paused = false;
                    } else {
                        job.paused = cursor.blocked;
                    }
                }
            }
        }
        self.refresh();
    }
}
fn return_phase() -> TeamSlot {
    TeamSlot::Returning {
        remaining: 0.0,
        system_id: None,
        display_name: None,
        queued_system_id: None,
        queued_display_name: None,
    }
}

#[cfg(test)]
#[path = "repair_personnel_transfer_tests.rs"]
mod tests;
