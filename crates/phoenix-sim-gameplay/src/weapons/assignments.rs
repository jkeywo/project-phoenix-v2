//! One retained assignment per actual weapon. Prepared groups are an authoring
//! convenience; they never confer authority over another group's members.
use crate::core::messages::SystemId;
use phoenix_model::weapon_targets::{
    WeaponGroupAssignment, WeaponGroupConfig, WeaponIndividualAssignment, WeaponTargetAssignment,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct WeaponTargetAssignments(pub BTreeMap<SystemId, WeaponTargetAssignment>);

impl WeaponTargetAssignments {
    pub fn target_for<'a>(&'a self, owner: &SystemId, primary: Option<&'a str>) -> Option<&'a str> {
        self.0
            .get(owner)
            .map_or(primary, |assignment| assignment.target(primary))
    }

    pub fn set_individual(&mut self, owner: SystemId, target: Option<String>) {
        self.0.entry(owner).or_default().individual = Some(match target {
            Some(contact) => WeaponIndividualAssignment::Target { contact },
            None => WeaponIndividualAssignment::Hold,
        });
    }

    pub fn set_group(&mut self, owner: SystemId, group: String, target: Option<String>) {
        self.0.entry(owner).or_default().group = Some(WeaponGroupAssignment { group, target });
    }

    pub fn set_automatic(&mut self, owner: SystemId, target: Option<String>) {
        if let Some(contact) = target {
            self.0.entry(owner).or_default().automatic = Some(contact);
        } else if let Some(assignment) = self.0.get_mut(&owner) {
            assignment.automatic = None;
            if assignment.is_empty() {
                self.0.remove(&owner);
            }
        }
    }

    pub fn follow_group(&mut self, owner: &SystemId) {
        if let Some(assignment) = self.0.get_mut(owner) {
            assignment.individual = None;
            if assignment.is_empty() {
                self.0.remove(owner);
            }
        }
    }

    pub fn follow_primary(&mut self, owner: &SystemId) {
        if let Some(assignment) = self.0.get_mut(owner) {
            assignment.individual = None;
            assignment.group = None;
            if assignment.is_empty() {
                self.0.remove(owner);
            }
        }
    }
}

pub fn is_weapon(kind: &str) -> bool {
    use crate::ship::system_registry::*;
    matches!(
        kind,
        PHASER_BANK_KIND | BLASTER_BANK_KIND | TORPEDO_TUBE_KIND
    )
}

pub fn validate_groups(
    groups: &[WeaponGroupConfig],
    ship: &crate::ship::config::ShipConfig,
) -> Result<(), String> {
    let mut ids = BTreeSet::new();
    for group in groups {
        if group.id.is_empty()
            || !group
                .id
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            || !ids.insert(&group.id)
            || group.label.trim().is_empty()
            || group.members.is_empty()
        {
            return Err(
                "weapon groups require distinct nonempty ids, a label and actual weapon members"
                    .into(),
            );
        }
        let mut members = BTreeSet::new();
        for member in &group.members {
            if !members.insert(member)
                || ship
                    .system(member)
                    .is_none_or(|system| !is_weapon(&system.kind))
            {
                return Err(format!(
                    "weapon group '{}' has a missing, duplicate or non-weapon member '{}'",
                    group.id, member.0
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "assignments_tests.rs"]
mod tests;
