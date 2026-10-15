//! Accepted finite personnel attached to existing physical Repair teams.
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DutyCompetency {
    MechanicalRepair,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersonnelConfig {
    #[serde(default, rename = "repair_team")]
    pub repair_teams: Vec<DutyTeamConfig>,
    #[serde(default, rename = "officer")]
    pub officers: Vec<DutyOfficerConfig>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DutyTeamConfig {
    pub id: String,
    pub label: String,
    /// Binds an existing finite Repair team; never creates an additional team.
    pub team_idx: u32,
    pub competencies: Vec<DutyCompetency>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DutyOfficerConfig {
    pub id: String,
    pub label: String,
    pub home_compartment: String,
    pub competencies: Vec<DutyCompetency>,
    /// Applies only to actual compatible internal physical remedy progress.
    pub repair_work_multiplier: f32,
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}
fn valid_competencies(values: &[DutyCompetency]) -> bool {
    !values.is_empty() && values.iter().collect::<BTreeSet<_>>().len() == values.len()
}
impl PersonnelConfig {
    pub fn validate(&self) -> Result<(), String> {
        let mut ids = BTreeSet::new();
        let mut slots = BTreeSet::new();
        for team in &self.repair_teams {
            if !valid_id(&team.id)
                || team.label.trim().is_empty()
                || !ids.insert(&team.id)
                || !slots.insert(team.team_idx)
                || !valid_competencies(&team.competencies)
            {
                return Err(
                    "personnel requires unique actual Repair team bindings and competencies".into(),
                );
            }
        }
        let mut officers = BTreeSet::new();
        for officer in &self.officers {
            if !valid_id(&officer.id)
                || officer.label.trim().is_empty()
                || !valid_id(&officer.home_compartment)
                || !officers.insert(&officer.id)
                || !valid_competencies(&officer.competencies)
                || !officer.repair_work_multiplier.is_finite()
                || officer.repair_work_multiplier < 1.0
            {
                return Err(
                    "personnel requires unique finite officers with supported repair expertise"
                        .into(),
                );
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum OfficerLocation {
    AtCompartment {
        compartment: String,
    },
    /// Location is read through the one actual team navigation cursor.
    WithRepairTeam {
        team_idx: u32,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DutyOfficerState {
    pub location: OfficerLocation,
    pub changed_tick: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersonnelView {
    pub teams: Vec<DutyTeamView>,
    pub officers: Vec<DutyOfficerView>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DutyTeamView {
    pub id: String,
    pub label: String,
    pub team_idx: u32,
    pub competencies: Vec<DutyCompetency>,
    pub officer: Option<String>,
    pub active_work_multiplier: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DutyOfficerView {
    pub id: String,
    pub label: String,
    pub competencies: Vec<DutyCompetency>,
    pub location: OfficerLocation,
    pub changed_tick: u64,
    pub repair_work_multiplier: f32,
    pub available_teams: Vec<u32>,
    pub can_release: bool,
}

#[cfg(test)]
#[path = "personnel_tests.rs"]
mod tests;
