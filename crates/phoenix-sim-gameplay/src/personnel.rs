//! Exclusive officer custody references the existing Repair team's physical cursor.
use crate::jobs::repair::{RepairActivity, RepairJobState};
use crate::ship::interior::{InteriorConfig, TeamLocation};
use bevy::prelude::{Component, Resource};
pub use phoenix_model::personnel::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompiledPersonnel {
    pub care_compartment: Option<String>,
    pub config: PersonnelConfig,
    team_homes: Vec<String>,
}

impl CompiledPersonnel {
    pub fn compile(
        config: &PersonnelConfig,
        interior: &InteriorConfig,
        teams: usize,
    ) -> Result<Self, String> {
        config.validate()?;
        if interior.team_homes.len() != teams {
            return Err("personnel requires the actual finite Repair team homes".into());
        }
        for team in &config.repair_teams {
            if team.team_idx as usize >= teams {
                return Err("personnel team does not bind an existing Repair team".into());
            }
        }
        for officer in &config.officers {
            if !interior
                .compartments
                .iter()
                .any(|room| room.id == officer.home_compartment)
                || !config.repair_teams.iter().any(|team| {
                    interior.team_homes[team.team_idx as usize] == officer.home_compartment
                        && compatible(team, officer)
                })
            {
                return Err(
                    "officer requires a compatible actual team at its authored home compartment"
                        .into(),
                );
            }
        }
        let care_compartment = config
            .care
            .as_ref()
            .map(|care| {
                interior
                    .placements
                    .get(&care.system)
                    .cloned()
                    .ok_or("care equipment needs an actual compartment")
            })
            .transpose()?;
        if care_compartment.is_some()
            && !config
                .repair_teams
                .iter()
                .any(|team| team.competencies.contains(&DutyCompetency::MedicalCare))
        {
            return Err("care requires an actual finite team with MedicalCare competency".into());
        }
        Ok(Self {
            care_compartment,
            config: config.clone(),
            team_homes: interior.team_homes.clone(),
        })
    }
}

/// Accepted-content join: this first consumer uses the installed Repair owner,
/// real finite team count, actual interior and actual physical remedy definition.
pub fn compile_entity(
    entity: &crate::entities::config::EntityConfig,
) -> Result<Option<CompiledPersonnel>, String> {
    let Some(config) = &entity.personnel else {
        return Ok(None);
    };
    let repair = entity
        .repair
        .as_ref()
        .filter(|repair| repair.declares_teams())
        .ok_or("personnel requires actual finite Repair teams")?;
    let interior = entity
        .interior
        .as_ref()
        .ok_or("personnel requires actual team navigation")?;
    let ship = entity
        .ship_config
        .as_ref()
        .ok_or("personnel requires actual owned Repair machinery")?;
    let owner = crate::ship::system_registry::repair_system_id();
    if ship
        .system(&owner)
        .is_none_or(|system| system.kind != crate::ship::system_registry::REPAIR_KIND)
        || entity.damage.is_none()
    {
        return Err("personnel requires the installed canonical Repair owner and actual physical remedy definition".into());
    }
    if let Some(care) = &config.care {
        if care.system != owner
            || ship.system(&care.system).is_none()
            || !entity.energy.as_ref().is_some_and(|energy| {
                energy.consumers.iter().any(|consumer| {
                    consumer.system == care.system && consumer.continuous_cost_per_sec > 0.0
                })
            })
        {
            return Err(
                "care needs installed service machinery and actual positive paid work".into(),
            );
        }
        for part in &care.requirements {
            let endpoint = crate::resources::service::resolve_endpoint(
                &part.resource,
                &entity.resource_services,
                &crate::resources::service::ResourceLink {
                    service: Some(part.service.clone()),
                    provider: None,
                },
            )?;
            let crate::resources::service::ResourceEndpoint::Capacity { id } = endpoint else {
                return Err("care medication needs a real finite capacity stock".into());
            };
            if !entity.logistics.as_ref().is_some_and(|logistics| {
                logistics.stores.iter().any(|store| {
                    store.resource == part.resource
                        && store.endpoint
                            == (crate::resources::service::ResourceEndpoint::Capacity {
                                id: id.clone(),
                            })
                        && store.role == phoenix_model::logistics::StoreRole::Local
                        && store.system.as_ref() == Some(&care.system)
                        && store.source.is_some()
                })
            }) {
                return Err("care medication requires a managed local stock and actual replenishment source".into());
            }
            if !entity.infrastructure.as_ref().is_some_and(|stock| {
                stock.capacities.iter().any(|capacity| {
                    capacity.id == id && capacity.ceiling.unwrap_or(capacity.amount) >= part.amount
                })
            }) {
                return Err(
                    "care medication service lacks a sufficient actual stock capacity".into(),
                );
            }
        }
    }
    CompiledPersonnel::compile(config, interior, repair.repair_team_count as usize).map(Some)
}

fn compatible(team: &DutyTeamConfig, officer: &DutyOfficerConfig) -> bool {
    team.competencies
        .iter()
        .any(|competency| officer.competencies.contains(competency))
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersonnelState {
    pub officers: BTreeMap<String, DutyOfficerState>,
}

/// The sole canonical personnel owner. Origin identity survives actual actor
/// removal; individual Console publication must still select its permitted row.
#[derive(Resource, Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersonnelRoster {
    pub origins: BTreeMap<String, PersonnelOrigin>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersonnelOrigin {
    pub definition: CompiledPersonnel,
    pub source: Option<PersonnelSource>,
    pub accepted_tick: u64,
    pub removed_tick: Option<u64>,
    pub state: PersonnelState,
}

impl PersonnelRoster {
    pub fn accept(
        &mut self,
        actor: &str,
        profile: CompiledPersonnel,
        source: Option<PersonnelSource>,
        tick: u64,
    ) -> Result<(), String> {
        if actor.trim().is_empty() {
            return Err("personnel origin requires a stable actor identity".into());
        }
        if let Some(existing) = self.origins.get(actor) {
            return if existing.definition == profile && existing.removed_tick.is_none() {
                Ok(())
            } else {
                Err("personnel identity cannot be replaced or rerolled".into())
            };
        }
        self.origins.insert(
            actor.into(),
            PersonnelOrigin {
                state: PersonnelState::new(&profile),
                definition: profile,
                source,
                accepted_tick: tick,
                removed_tick: None,
            },
        );
        Ok(())
    }
    pub fn state(&self, actor: &str) -> Option<&PersonnelState> {
        self.origins
            .get(actor)
            .filter(|origin| origin.removed_tick.is_none())
            .map(|origin| &origin.state)
    }
    pub fn state_mut(&mut self, actor: &str) -> Option<&mut PersonnelState> {
        self.origins
            .get_mut(actor)
            .filter(|origin| origin.removed_tick.is_none())
            .map(|origin| &mut origin.state)
    }
    pub fn remove_actor(&mut self, actor: &str, tick: u64) {
        let Some(origin) = self.origins.get_mut(actor) else {
            return;
        };
        if origin.removed_tick.is_some() {
            return;
        }
        origin.removed_tick = Some(tick);
        for state in origin.state.officers.values_mut() {
            // All currently supported custody is physically aboard its origin.
            // Future off-ship variants must retain their actual location here.
            state.location = OfficerLocation::Lost {
                actor: actor.into(),
            };
            state.changed_tick = tick;
        }
    }
}

fn at_idle_home<'a>(
    jobs: &RepairJobState,
    interior: &'a InteriorConfig,
    team: usize,
) -> Option<&'a str> {
    let home = interior.team_homes.get(team)?;
    let job = jobs.teams.get(team)?;
    if job.as_ref().is_some_and(|job| job.outcome.is_none()) {
        return None;
    }
    let navigation = jobs.navigation.as_ref()?.get(team)?;
    matches!(&navigation.location, TeamLocation::Compartment { compartment } if compartment == home)
        .then_some(home.as_str())
}

impl PersonnelState {
    pub fn new(profile: &CompiledPersonnel) -> Self {
        Self {
            officers: profile
                .config
                .officers
                .iter()
                .map(|officer| {
                    (
                        officer.id.clone(),
                        DutyOfficerState {
                            condition: officer.initial_condition,
                            location: OfficerLocation::AtCompartment {
                                compartment: officer.home_compartment.clone(),
                            },
                            changed_tick: 0,
                        },
                    )
                })
                .collect(),
        }
    }

    /// Called against the complete candidate Repair owner during preflight.
    /// External/refit custody follows that owner's actual cursor, without bonus.
    pub fn validate(
        &self,
        profile: &CompiledPersonnel,
        jobs: &RepairJobState,
        tick: u64,
    ) -> Result<(), String> {
        if self.officers.keys().collect::<BTreeSet<_>>()
            != profile
                .config
                .officers
                .iter()
                .map(|officer| &officer.id)
                .collect::<BTreeSet<_>>()
        {
            return Err(
                "personnel continuation differs from the finite accepted officer roster".into(),
            );
        }
        let mut posted = BTreeSet::new();
        for definition in &profile.config.officers {
            let officer = &self.officers[&definition.id];
            if officer.changed_tick > tick {
                return Err("officer posting is from a future logical tick".into());
            }
            match &officer.location {
                OfficerLocation::AtCompartment { compartment }
                    if compartment == &definition.home_compartment => {}
                OfficerLocation::WithRepairTeam { team_idx } => {
                    let team = profile
                        .config
                        .repair_teams
                        .iter()
                        .find(|team| team.team_idx == *team_idx)
                        .ok_or("officer names an unknown actual team")?;
                    if !compatible(team, definition)
                        || profile.team_homes.get(*team_idx as usize)
                            != Some(&definition.home_compartment)
                        || !posted.insert(*team_idx)
                        || *team_idx as usize >= jobs.teams.len()
                        || jobs
                            .navigation
                            .as_ref()
                            .and_then(|rows| rows.get(*team_idx as usize))
                            .is_none()
                    {
                        return Err(
                            "officer custody duplicates or lacks its compatible actual team cursor"
                                .into(),
                        );
                    }
                }
                _ => return Err("unposted officer is outside its supported home custody".into()),
            }
        }
        self.validate_treatment(profile, jobs)?;
        Ok(())
    }

    /// Assignment and release are physical handoffs at a common idle home.
    /// All checks precede the one write; repeated identical intent is a no-op.
    pub fn post(
        &mut self,
        profile: &CompiledPersonnel,
        jobs: &RepairJobState,
        interior: &InteriorConfig,
        officer_id: &str,
        destination: Option<u32>,
        tick: u64,
    ) -> Result<(), String> {
        self.validate(profile, jobs, tick)?;
        let definition = profile
            .config
            .officers
            .iter()
            .find(|officer| officer.id == officer_id)
            .ok_or("unknown officer")?;
        let officer = self
            .officers
            .get(officer_id)
            .ok_or("officer has no physical custody")?;
        if destination.is_some() && officer.condition != PersonnelCondition::Fit {
            return Err("unfit officers cannot be newly posted".into());
        }
        if jobs.teams.iter().flatten().any(|job|job.outcome.is_none() && matches!(&job.activity, RepairActivity::Treatment { work, .. } if work.patient == officer_id)) {
            return Err("officer is committed to actual treatment".into());
        }
        let current = match &officer.location {
            OfficerLocation::Lost { .. } => return Err("lost officer cannot be posted".into()),
            OfficerLocation::AtCompartment { .. } => None,
            OfficerLocation::WithRepairTeam { team_idx } => Some(*team_idx),
        };
        if current == destination {
            return Ok(());
        }
        let compartment = match &officer.location {
            OfficerLocation::Lost { .. } => {
                return Err("lost officer has no operational location".into());
            }
            OfficerLocation::AtCompartment { compartment } => compartment.as_str(),
            OfficerLocation::WithRepairTeam { team_idx } => {
                at_idle_home(jobs, interior, *team_idx as usize)
                    .ok_or("officer cannot leave a team that is away or committed")?
            }
        };
        let location = if let Some(team_idx) = destination {
            let team = profile
                .config
                .repair_teams
                .iter()
                .find(|team| team.team_idx == team_idx)
                .ok_or("unknown team posting")?;
            if !compatible(team, definition)
                || at_idle_home(jobs, interior, team_idx as usize) != Some(compartment)
                || self
                    .officers
                    .values()
                    .any(|officer| officer.location == OfficerLocation::WithRepairTeam { team_idx })
            {
                return Err(
                    "posting requires one free compatible co-located team at its idle home".into(),
                );
            }
            OfficerLocation::WithRepairTeam { team_idx }
        } else {
            if compartment != definition.home_compartment {
                return Err("officer release requires its actual home compartment".into());
            }
            OfficerLocation::AtCompartment {
                compartment: compartment.into(),
            }
        };
        self.officers.insert(
            officer_id.into(),
            DutyOfficerState {
                condition: officer.condition,
                location,
                changed_tick: tick,
            },
        );
        Ok(())
    }

    /// Candidate destinations derive only from actual finite free team slots and
    /// the patient's one physical location, not a second medical availability pool.
    pub fn treatment_teams(
        &self,
        profile: &CompiledPersonnel,
        jobs: &RepairJobState,
        patient: &str,
    ) -> Vec<u32> {
        let Some(state) = self.officers.get(patient) else {
            return Vec::new();
        };
        let Some(room) = profile.care_compartment.as_ref() else {
            return Vec::new();
        };
        if profile.config.care.as_ref().is_some_and(|care|jobs.teams.iter().flatten().any(|job|job.outcome.is_none() && matches!(&job.activity,RepairActivity::Refit {target,..} if target==&care.system))) {return Vec::new();}
        if state.condition==PersonnelCondition::Fit || !matches!(&state.location,OfficerLocation::AtCompartment {compartment} if compartment==room)
            || jobs.teams.iter().flatten().any(|job|job.outcome.is_none() && matches!(&job.activity,RepairActivity::Treatment {work,..} if work.patient==patient)) {return Vec::new();}
        profile.config.repair_teams.iter().filter_map(|team| {
            let index=team.team_idx as usize;
            (team.competencies.contains(&DutyCompetency::MedicalCare) && jobs.teams.get(index).is_some_and(|entry|entry.as_ref().is_none_or(|job|job.outcome.is_some()))
                && jobs.navigation.as_ref().and_then(|rows|rows.get(index)).is_some_and(|cursor|matches!(&cursor.location,TeamLocation::Compartment {compartment} if profile.team_homes.get(index)==Some(compartment))))
                .then_some(team.team_idx)
        }).collect()
    }
    fn validate_treatment(
        &self,
        profile: &CompiledPersonnel,
        jobs: &RepairJobState,
    ) -> Result<(), String> {
        let mut patients = BTreeSet::new();
        for (index, entry) in jobs.teams.iter().enumerate() {
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
            let care = profile
                .config
                .care
                .as_ref()
                .ok_or("treatment has no accepted care profile")?;
            let room = profile
                .care_compartment
                .as_ref()
                .ok_or("care has no actual service compartment")?;
            let patient = self
                .officers
                .get(&work.patient)
                .ok_or("unknown actual treatment patient")?;
            let cursor = jobs
                .navigation
                .as_ref()
                .and_then(|rows| rows.get(index))
                .ok_or("missing actual treatment team cursor")?;
            let duration = care
                .work_secs(work.condition)
                .ok_or("treatment cannot claim a fit condition")?;
            if !profile.config.repair_teams.iter().any(|team| {
                team.team_idx as usize == index
                    && team.competencies.contains(&DutyCompetency::MedicalCare)
            }) || work.work_secs != duration
                || !work.progress.is_finite()
                || !(0.0..=duration).contains(&work.progress)
                || (work.progress > 0.0 && !work.medication_used)
                || (work.completed
                    && (work.progress != duration || !work.medication_used || *cancelled))
                || (!work.completed && work.progress >= duration)
                || (job.outcome.is_none() && !patients.insert(&work.patient))
            {
                return Err(
                    "treatment does not match actual patient, team, site or accepted work terms"
                        .into(),
                );
            }
            // Retained terminal history describes past work; a future assignment
            // may change patient condition/custody, so live joins stop at release.
            if job.outcome.is_some() {
                continue;
            }
            if !matches!(&patient.location,OfficerLocation::AtCompartment {compartment} if compartment==room)
            {
                return Err("active treatment patient left its actual service compartment".into());
            }
            if patient.condition
                != if work.completed {
                    PersonnelCondition::Fit
                } else {
                    work.condition
                }
            {
                return Err("treatment disagrees with the single actual patient condition".into());
            }
            if !matches!(phase,phoenix_model::messages::TeamSlot::Returning {..}) && jobs.teams.iter().flatten().any(|other|other.outcome.is_none() && matches!(&other.activity,RepairActivity::Refit {target,..} if target==&care.system)) {
                return Err("active treatment cannot also refit its actual service machinery".into());
            }
            match phase {
                phoenix_model::messages::TeamSlot::Travelling {
                    system_id: Some(system),
                    elapsed,
                    ..
                } if system == &care.system
                    && *elapsed == 0.0
                    && work.progress == 0.0
                    && !work.medication_used
                    && !*cancelled => {}
                phoenix_model::messages::TeamSlot::Repairing {
                    system_id: Some(system),
                    ..
                } if system == &care.system
                    && !*cancelled
                    && !work.completed
                    && matches!(&cursor.location,TeamLocation::Compartment {compartment} if compartment==room) =>
                    {}
                phoenix_model::messages::TeamSlot::Returning {
                    remaining,
                    system_id: None,
                    queued_system_id: None,
                    ..
                } if *remaining == 0.0 && (*cancelled || work.completed) => {}
                _ => return Err("treatment work phase is not at the accepted actual site".into()),
            }
        }
        Ok(())
    }

    /// Read-only derived work input, not a second bonus or movement owner.
    pub fn repair_work_multipliers(
        &self,
        profile: &CompiledPersonnel,
        jobs: &RepairJobState,
    ) -> Vec<f32> {
        let mut result = vec![1.0; jobs.teams.len()];
        for definition in &profile.config.officers {
            let Some(DutyOfficerState {
                location: OfficerLocation::WithRepairTeam { team_idx },
                ..
            }) = self.officers.get(&definition.id)
            else {
                continue;
            };
            if self.officers[&definition.id].condition != PersonnelCondition::Fit
                || !definition
                    .competencies
                    .contains(&DutyCompetency::MechanicalRepair)
            {
                continue;
            }
            let index = *team_idx as usize;
            let on_site = jobs
                .teams
                .get(index)
                .and_then(Option::as_ref)
                .is_some_and(|job| {
                    job.outcome.is_none()
                        && !job.paused
                        && !job.workflow_error
                        && matches!(
                            &job.activity,
                            RepairActivity::Internal {
                                phase: phoenix_model::messages::TeamSlot::Repairing { .. },
                                cancelled: false
                            }
                        )
                })
                && jobs
                    .navigation
                    .as_ref()
                    .and_then(|rows| rows.get(index))
                    .is_some_and(|team| matches!(team.location, TeamLocation::Compartment { .. }));
            if on_site
                && profile.config.repair_teams.iter().any(|team| {
                    team.team_idx == *team_idx
                        && team
                            .competencies
                            .contains(&DutyCompetency::MechanicalRepair)
                        && compatible(team, definition)
                })
            {
                if let Some(multiplier) = result.get_mut(index) {
                    *multiplier = definition.repair_work_multiplier;
                }
            }
        }
        result
    }

    pub fn view(
        &self,
        profile: &CompiledPersonnel,
        jobs: &RepairJobState,
        interior: &InteriorConfig,
        tick: u64,
    ) -> PersonnelView {
        let multipliers = self.repair_work_multipliers(profile, jobs);
        PersonnelView {
            teams: profile
                .config
                .repair_teams
                .iter()
                .map(|team| DutyTeamView {
                    id: team.id.clone(),
                    label: team.label.clone(),
                    team_idx: team.team_idx,
                    competencies: team.competencies.clone(),
                    officer: self.officers.iter().find_map(|(id, officer)| {
                        (officer.location
                            == OfficerLocation::WithRepairTeam {
                                team_idx: team.team_idx,
                            })
                        .then(|| id.clone())
                    }),
                    active_work_multiplier: multipliers
                        .get(team.team_idx as usize)
                        .copied()
                        .unwrap_or(1.0),
                })
                .collect(),
            officers: profile
                .config
                .officers
                .iter()
                .filter_map(|definition| {
                    let officer = self.officers.get(&definition.id)?;
                    let available_teams = profile
                        .config
                        .repair_teams
                        .iter()
                        .filter_map(|team| {
                            let mut candidate = self.clone();
                            candidate
                                .post(
                                    profile,
                                    jobs,
                                    interior,
                                    &definition.id,
                                    Some(team.team_idx),
                                    tick,
                                )
                                .is_ok()
                                .then_some(team.team_idx)
                        })
                        .collect();
                    let can_release =
                        matches!(officer.location, OfficerLocation::WithRepairTeam { .. })
                            && self
                                .clone()
                                .post(profile, jobs, interior, &definition.id, None, tick)
                                .is_ok();
                    Some(DutyOfficerView {
                        condition: officer.condition,
                        treatment_teams: self.treatment_teams(profile, jobs, &definition.id),
                        treatment: jobs.teams.iter().enumerate().find_map(|(index, entry)| {
                            let job = entry.as_ref().filter(|job| job.outcome.is_none())?;
                            let RepairActivity::Treatment { work, phase, .. } = &job.activity
                            else {
                                return None;
                            };
                            if work.patient != definition.id {
                                return None;
                            }
                            Some(OfficerTreatmentView {
                                team_idx: index as u32,
                                phase: match phase {
                                    phoenix_model::messages::TeamSlot::Travelling { .. } => {
                                        phoenix_model::job::JobPhase::Travelling
                                    }
                                    phoenix_model::messages::TeamSlot::Repairing { .. } => {
                                        phoenix_model::job::JobPhase::Working
                                    }
                                    _ => phoenix_model::job::JobPhase::Returning,
                                },
                                progress: ((work.progress / work.work_secs).clamp(0.0, 1.0)
                                    * 1000.0) as u16,
                                waiting_medication: work.waiting_medication,
                            })
                        }),
                        id: definition.id.clone(),
                        label: definition.label.clone(),
                        competencies: definition.competencies.clone(),
                        location: officer.location.clone(),
                        changed_tick: officer.changed_tick,
                        repair_work_multiplier: definition.repair_work_multiplier,
                        available_teams,
                        can_release,
                    })
                })
                .collect(),
        }
    }
}

#[cfg(test)]
#[path = "personnel_tests.rs"]
mod tests;
