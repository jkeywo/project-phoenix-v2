//! Exclusive officer custody references the existing Repair team's physical cursor.
use crate::jobs::repair::{RepairActivity, RepairJobState};
use crate::ship::interior::{InteriorConfig, TeamLocation};
use bevy::prelude::Component;
pub use phoenix_model::personnel::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Component, Clone, Debug)]
pub struct CompiledPersonnel {
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
        Ok(Self {
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
    CompiledPersonnel::compile(config, interior, repair.repair_team_count as usize).map(Some)
}

fn compatible(team: &DutyTeamConfig, officer: &DutyOfficerConfig) -> bool {
    team.competencies
        .contains(&DutyCompetency::MechanicalRepair)
        && officer
            .competencies
            .contains(&DutyCompetency::MechanicalRepair)
}

#[derive(Component, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersonnelState {
    pub officers: BTreeMap<String, DutyOfficerState>,
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
        let current = match &officer.location {
            OfficerLocation::AtCompartment { .. } => None,
            OfficerLocation::WithRepairTeam { team_idx } => Some(*team_idx),
        };
        if current == destination {
            return Ok(());
        }
        let compartment = match &officer.location {
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
                location,
                changed_tick: tick,
            },
        );
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
                && profile
                    .config
                    .repair_teams
                    .iter()
                    .any(|team| team.team_idx == *team_idx && compatible(team, definition))
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
