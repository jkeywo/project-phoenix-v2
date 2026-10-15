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
#[path = "repair_physical.rs"]
mod physical;
pub use physical::{PhysicalRepairWork, RepairStockAction, RepairTickResult};
#[path = "repair_external_physical.rs"]
mod external_physical;
pub use external_physical::ExternalPhysicalRepairWork;
#[path = "repair_refit.rs"]
mod refit;
#[path = "repair_treatment.rs"]
mod treatment;
pub use treatment::TreatmentWork;
#[path = "repair_personnel_transfer.rs"]
mod personnel_transfer;
pub use personnel_transfer::{PersonnelTransferPlan, PersonnelTransferWork, TransferAvailability};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum RepairActivity {
    PersonnelTransfer {
        phase: TeamSlot,
        cancelled: bool,
        work: PersonnelTransferWork,
    },
    Internal {
        phase: TeamSlot,
        cancelled: bool,
    },
    External {
        target: String,
    },
    Treatment {
        phase: TeamSlot,
        cancelled: bool,
        work: TreatmentWork,
    },
    /// Physical fitting commitment; machinery stages belong to the fitting owner.
    Refit {
        request: u64,
        target: SystemId,
        #[serde(default)]
        destination_compartment: Option<String>,
        phase: TeamSlot,
        cancelled: bool,
    },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RepairJobState {
    pub jobs: TeamJobs<RepairActivity>,
    pub navigation: Option<Vec<crate::ship::interior::TeamNavigation>>,
    /// Typed bounded work owned by the existing assignment, keyed by JobId.
    pub physical_work: BTreeMap<u64, PhysicalRepairWork>,
    /// Physical work on a visited actor remains owned by this same external Job.
    pub external_physical_work: BTreeMap<u64, ExternalPhysicalRepairWork>,
}
impl std::ops::Deref for RepairJobState {
    type Target = TeamJobs<RepairActivity>;
    fn deref(&self) -> &Self::Target {
        &self.jobs
    }
}
impl std::ops::DerefMut for RepairJobState {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.jobs
    }
}
#[derive(Clone, Debug)]
pub struct RepairTeams {
    state: RepairJobState,
    timings: RepairTimings,
    program: Program,
    readout: Vec<TeamSlot>,
    layout: Option<crate::ship::interior::InteriorGraph>,
    damage_definition: Option<crate::ship::compartment_damage::DamageDefinition>,
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
            state: RepairJobState {
                jobs: TeamJobs::new(count, owner),
                navigation: None,
                physical_work: BTreeMap::new(),
                external_physical_work: BTreeMap::new(),
            },
            timings,
            program: repair_workflow::compile_workflow(workflow)?,
            readout: vec![TeamSlot::Idle; count],
            layout: None,
            damage_definition: None,
        })
    }
    pub fn timings(&self) -> RepairTimings {
        self.timings
    }
    /// Bind accepted immutable content once. Navigation remains in the canonical
    /// job state; this graph is a derived routing cache, never a second team owner.
    pub fn install_layout(&mut self, layout: crate::ship::interior::InteriorGraph) {
        self.state.navigation = Some(layout.initial_teams());
        self.layout = Some(layout);
    }
    pub fn layout(&self) -> Option<&crate::ship::interior::InteriorConfig> {
        self.layout.as_ref().map(|graph| &graph.config)
    }
    pub fn navigation(&self) -> Option<&[crate::ship::interior::TeamNavigation]> {
        self.state.navigation.as_deref()
    }
    pub fn owner(&self) -> &SystemId {
        &self.state.owner
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
                    activity:
                        RepairActivity::Internal { phase, .. }
                        | RepairActivity::Refit { phase, .. }
                        | RepairActivity::Treatment { phase, .. }
                        | RepairActivity::PersonnelTransfer { phase, .. },
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
            // The retained percentage-repair executor may select a sweep
            // destination. A fitting commitment excludes that next task too.
            let guarded = match phase {
                TeamSlot::Travelling {
                    system_id: Some(id),
                    display_name,
                    ..
                }
                | TeamSlot::Repairing {
                    system_id: Some(id),
                    display_name,
                    ..
                } if self.refit_claims_system(id) && !self.is_refit_team(team) => {
                    Some(TeamSlot::Returning {
                        remaining: self.timings.travel_duration,
                        system_id: Some(id.clone()),
                        display_name: display_name.clone(),
                        queued_system_id: None,
                        queued_display_name: None,
                    })
                }
                _ => None,
            };
            let phase = guarded.as_ref().unwrap_or(phase);
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
        self.state.navigation.is_some()
            || self.state.next_id != 1
            || self.state.teams.iter().any(Option::is_some)
    }
    /// Borrow the canonical job/navigation frontier for local personnel joins.
    pub fn job_state(&self) -> &RepairJobState {
        &self.state
    }
    pub fn save_state(&self) -> RepairJobState {
        self.state.clone()
    }
    pub fn restore_state(&mut self, state: &RepairJobState) -> Result<(), String> {
        self.validate_refit_work(state)?;
        self.validate_treatment_shape(state)?;
        self.validate_personnel_transfer_shape(state)?;
        self.validate_physical_work(state)?;
        self.validate_external_physical_shape(state)?;
        match (&self.layout, &state.navigation) {
            (Some(layout), Some(navigation)) => layout.validate_navigation(navigation)?,
            (None, None) => {}
            _ => return Err("repair navigation does not match authored interior".into()),
        }
        if state.teams.len() != self.state.teams.len()
            || state.owner != self.state.owner
            || state.next_id == 0
        {
            return Err("repair job owner/team count mismatch".into());
        }
        if let (Some(layout), Some(navigation)) = (&self.layout, &state.navigation) {
            use crate::ship::interior::TeamLocation;
            for (index, team) in navigation.iter().enumerate() {
                let active = state.teams[index]
                    .as_ref()
                    .filter(|job| job.outcome.is_none());
                match (active.map(|job| &job.activity), &team.location) {
                    (
                        Some(RepairActivity::External { target }),
                        TeamLocation::External { target: location },
                    ) if target == location => {}
                    (Some(RepairActivity::External { .. }), _)
                    | (_, TeamLocation::External { .. }) => {
                        return Err(
                            "external team location does not match its active commitment".into(),
                        )
                    }
                    (
                        Some(RepairActivity::Internal {
                            phase:
                                TeamSlot::Repairing {
                                    system_id: Some(id),
                                    ..
                                },
                            ..
                        }),
                        location,
                    ) => {
                        let physical_room = active
                            .and_then(|job| state.physical_work.get(&job.id.0))
                            .and_then(|work| work.target.as_ref())
                            .and_then(|target| match target {
                                phoenix_model::damage::RepairWorkTarget::Compartment {
                                    compartment,
                                } => Some(compartment.as_str()),
                                _ => None,
                            });
                        if !matches!(location,TeamLocation::Compartment {compartment} if physical_room.or_else(||layout.compartment_for_system(id).map(String::as_str))==Some(compartment.as_str()))
                        {
                            return Err(
                                "on-site repair phase does not match its physical compartment"
                                    .into(),
                            );
                        }
                    }
                    (None, TeamLocation::Crossing { .. }) => {
                        return Err("idle team cannot retain an unowned crossing".into())
                    }
                    _ => {}
                }
            }
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
                RepairActivity::Internal { phase, .. }
                | RepairActivity::Refit { phase, .. }
                | RepairActivity::Treatment { phase, .. }
                | RepairActivity::PersonnelTransfer { phase, .. } => {
                    if let Some(layout) = &self.layout {
                        let known = |id: &SystemId| layout.compartment_for_system(id).is_some();
                        let valid = match phase {
                            TeamSlot::Travelling { system_id, .. } => {
                                system_id.as_ref().is_some_and(known)
                            }
                            TeamSlot::Repairing {
                                system_id,
                                priority_system_id,
                                ..
                            } => {
                                system_id.as_ref().is_some_and(known)
                                    && priority_system_id.as_ref().is_none_or(known)
                            }
                            TeamSlot::Returning {
                                system_id,
                                queued_system_id,
                                ..
                            } => {
                                system_id.as_ref().is_none_or(known)
                                    && queued_system_id.as_ref().is_none_or(known)
                            }
                            TeamSlot::Idle => false,
                        };
                        if !valid {
                            return Err("repair phase refers to equipment without an authored physical home".into());
                        }
                    }
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
        self.state.teams.iter().filter_map(|entry| match entry {
            Some(Job {
                activity:
                    RepairActivity::Internal {
                        phase:
                            TeamSlot::Repairing {
                                system_id: Some(id),
                                ..
                            },
                        ..
                    },
                outcome: None,
                ..
            }) => Some(id),
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
            .filter(|team| {
                self.external_team() != Some(*team)
                    && !self.is_refit_team(*team)
                    && !self.is_treatment_team(*team)
                    && !self.is_personnel_transfer_team(*team)
            })
            .collect()
    }
    pub fn free_team_indices_reserving(&self, abroad: Option<usize>, reserved: u8) -> Vec<usize> {
        let mut free = self.free_team_indices(abroad);
        free.truncate(free.len().saturating_sub(reserved as usize));
        free
    }
    pub fn is_committed_to_operation(&self, team: usize, abroad: Option<usize>) -> bool {
        self.is_refit_team(team)
            || self.is_treatment_team(team)
            || self.is_personnel_transfer_team(team)
            || self.external_team() == Some(team)
            || self.phases().is_committed_to_operation(team, abroad)
    }
    pub fn dispatch(&mut self, team: usize, target: SystemId, label: String) {
        if team >= self.state.teams.len()
            || self.external_team() == Some(team)
            || self.is_refit_team(team)
            || self.is_treatment_team(team)
            || self.is_personnel_transfer_team(team)
            || self.refit_claims_system(&target)
        {
            return;
        }
        let mut phases = self.phases();
        phases.dispatch(team, target, label);
        if self.layout.is_some() {
            // Redirect directly from the physical position, after any crossing
            // already in progress. The phase executor's old return timer must
            // not introduce an invented trip home.
            let mut slots = phases.slots().to_vec();
            if let TeamSlot::Returning {
                queued_system_id: Some(id),
                queued_display_name,
                ..
            } = &slots[team]
            {
                slots[team] = TeamSlot::Travelling {
                    system_id: Some(id.clone()),
                    display_name: queued_display_name.clone(),
                    elapsed: 0.0,
                    priority: None,
                };
                phases.restore_slots(&slots);
            }
        }
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
        if self.is_personnel_transfer_team(team) {
            return self.recall_personnel_transfer(team);
        }
        if self.is_treatment_team(team) {
            return self.return_treatment(team);
        }
        if self.is_refit_team(team) {
            return self.return_refit(team, true);
        }
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
        if self.is_refit_team(team)
            || self.is_treatment_team(team)
            || self.is_personnel_transfer_team(team)
        {
            return false;
        }
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
        if self.refit_claims_system(target) {
            return None;
        }
        let mut p = self.phases();
        let result = p.prioritise_system(target, hull, config).filter(|team| {
            !self.is_refit_team(*team)
                && !self.is_treatment_team(*team)
                && !self.is_personnel_transfer_team(*team)
        });
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
        grant: impl FnMut(&SystemId, f32) -> f32,
    ) {
        self.tick_with_access(dt, hull, config, &BTreeSet::new(), grant);
    }
    pub fn tick_with_access(
        &mut self,
        dt: f32,
        hull: &mut SystemHull,
        config: Option<&ShipConfig>,
        blocked: &BTreeSet<String>,
        mut grant: impl FnMut(&SystemId, f32) -> f32,
    ) {
        let mut enabled: Vec<_> = self
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
        let skipped: BTreeSet<_> = (0..self.state.teams.len())
            .filter(|team| {
                self.is_refit_team(*team)
                    || self.is_treatment_team(*team)
                    || self.is_personnel_transfer_team(*team)
            })
            .collect();
        if let (Some(layout), Some(navigation)) = (&self.layout, &mut self.state.navigation) {
            super::repair_navigation::prepare(
                layout,
                navigation,
                &mut phases,
                dt,
                &mut enabled,
                blocked,
                &skipped,
            );
        }
        let work: Vec<_> = phases
            .slots()
            .iter()
            .zip(&enabled)
            .map(|(slot, enabled)| match slot {
                TeamSlot::Repairing {
                    system_id: Some(id),
                    ..
                } if *enabled && !hull.is_at_max(id) => {
                    let work_dt = hull.get(id).map_or(0.0, |entry| {
                        if self.timings.repair_rate_hp_per_sec > 0.0 {
                            dt.min(
                                (entry.max - entry.current).max(0.0)
                                    / self.timings.repair_rate_hp_per_sec,
                            )
                        } else {
                            0.0
                        }
                    });
                    if dt > 0.0 && work_dt > 0.0 {
                        (work_dt / dt) * grant(&self.state.owner, work_dt).clamp(0.0, 1.0)
                    } else {
                        0.0
                    }
                }
                _ => 0.0,
            })
            .collect();
        phases.tick_with_repair_work(dt, hull, config, &enabled, &work);
        if let (Some(layout), Some(navigation)) = (&self.layout, &self.state.navigation) {
            super::repair_navigation::guard_arrival(layout, navigation, &mut phases);
        }
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
        if result {
            let target = self
                .external_target()
                .expect("new external claim")
                .to_owned();
            if let Some(navigation) = &mut self.state.navigation {
                navigation[team].location =
                    crate::ship::interior::TeamLocation::External { target };
                navigation[team].destination = None;
                navigation[team].route.clear();
                navigation[team].blocked = false;
            }
        }
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
        if let (Some(layout), Some(navigation)) = (&self.layout, &mut self.state.navigation) {
            navigation[team] = layout.initial_teams()[team].clone();
        }
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
                    RepairActivity::PersonnelTransfer { phase, work, .. } => (
                        if matches!(phase, TeamSlot::Returning { .. }) { JobPhase::Returning } else { JobPhase::Travelling },
                        None,
                        Some(work.plan.craft.clone()),
                    ),
                    RepairActivity::Internal { phase, .. }
                    | RepairActivity::Refit { phase, .. } | RepairActivity::Treatment { phase, .. } => match phase {
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
                    target_work: self
                        .state
                        .physical_work
                        .get(&job.id.0)
                        .and_then(|work| work.target.clone())
                        .or_else(|| {
                            self.state
                                .external_physical_work
                                .get(&job.id.0)
                                .and_then(|work| work.target.clone())
                        }),
                    work_progress: if let RepairActivity::Treatment {work,..}=&job.activity {
                        Some(((work.progress/work.work_secs).clamp(0.0,1.0)*1000.0) as u16)
                    } else { self
                        .state
                        .physical_work
                        .get(&job.id.0)
                        .and_then(|work| {
                            let recipe = self
                                .damage_definition
                                .as_ref()?
                                .config
                                .remedies
                                .get(work.recipe.as_ref()?)?;
                            Some(
                                ((work.progress / recipe.work_secs).clamp(0.0, 1.0) * 1000.0)
                                    as u16,
                            )
                        })
                        .or_else(|| {
                            self.state
                                .external_physical_work
                                .get(&job.id.0)
                                .and_then(|work| {
                                    work.work_secs.map(|secs| {
                                        ((work.progress / secs).clamp(0.0, 1.0) * 1000.0) as u16
                                    })
                                })
                        }) },
                    waiting_parts: matches!(&job.activity,RepairActivity::Treatment {work,..} if work.waiting_medication) || self
                        .state
                        .physical_work
                        .get(&job.id.0)
                        .is_some_and(|work| work.waiting_parts)
                        || self
                            .state
                            .external_physical_work
                            .get(&job.id.0)
                            .is_some_and(|work| work.waiting_parts),
                })
            })
            .collect()
    }
}
fn facts(activity: &RepairActivity) -> BTreeMap<String, bool> {
    let internal = matches!(
        activity,
        RepairActivity::Internal { .. }
            | RepairActivity::Refit { .. }
            | RepairActivity::Treatment { .. }
            | RepairActivity::PersonnelTransfer { .. }
    );
    let (travelling, repairing, returning, target) = match activity {
        RepairActivity::Internal { phase, .. }
        | RepairActivity::Refit { phase, .. }
        | RepairActivity::Treatment { phase, .. }
        | RepairActivity::PersonnelTransfer { phase, .. } => match phase {
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
        // Successful care already changed the actual patient. Return cleanup
        // cannot cancel that outcome or strand the still-committed team.
        if matches!(
            &job.activity,
            RepairActivity::Treatment {
                phase: TeamSlot::Returning { .. },
                ..
            } | RepairActivity::PersonnelTransfer {
                phase: TeamSlot::Returning { .. },
                ..
            }
        ) {
            job.paused = false;
            return true;
        }
        let returning = if let RepairActivity::Internal {
            phase:
                TeamSlot::Returning {
                    queued_system_id,
                    queued_display_name,
                    ..
                },
            cancelled,
        }
        | RepairActivity::Refit {
            phase:
                TeamSlot::Returning {
                    queued_system_id,
                    queued_display_name,
                    ..
                },
            cancelled,
            ..
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
        }
        | RepairActivity::Refit {
            phase: TeamSlot::Travelling { .. },
            ..
        }
        | RepairActivity::Treatment {
            phase: TeamSlot::Travelling { .. },
            ..
        }
        | RepairActivity::PersonnelTransfer {
            phase: TeamSlot::Travelling { .. },
            ..
        } => "advance_travel",
        RepairActivity::Internal {
            phase: TeamSlot::Repairing { .. },
            ..
        }
        | RepairActivity::Refit {
            phase: TeamSlot::Repairing { .. },
            ..
        }
        | RepairActivity::Treatment {
            phase: TeamSlot::Repairing { .. },
            ..
        }
        | RepairActivity::PersonnelTransfer {
            phase: TeamSlot::Repairing { .. },
            ..
        } => "apply_repair",
        RepairActivity::Internal {
            phase: TeamSlot::Returning { .. },
            ..
        }
        | RepairActivity::Refit {
            phase: TeamSlot::Returning { .. },
            ..
        }
        | RepairActivity::Treatment {
            phase: TeamSlot::Returning { .. },
            ..
        }
        | RepairActivity::PersonnelTransfer {
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
