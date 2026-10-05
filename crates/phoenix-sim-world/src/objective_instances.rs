use crate::core::messages::{ObjectiveSnapshot, ObjectiveStatus, ScoredObjective};
use std::collections::{BTreeMap, BTreeSet};
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub struct ObjectiveInstanceKey {
    pub objective_id: String,
    pub instance_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecipientSelector {
    ShipSlot(String),
    Faction(String),
    AllPlayerShips,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ObjectiveInstanceSpec {
    pub key: ObjectiveInstanceKey,
    pub recipients: Vec<RecipientSelector>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PlayerShipMembership {
    pub ship_id: String,
    pub slot_id: String,
    pub faction: String,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ObjectiveInstanceRecord {
    pub spec: ObjectiveInstanceSpec,
    pub status: ObjectiveStatus,
    pub progress: f32,
    /// Per-instance machine directive. Crew-facing prose remains shared on the
    /// legacy definition, but two named jobs may send their ships elsewhere.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub directive: Option<crate::core::messages::AiDirective>,
    /// Effective members at the one successful completion transition.
    #[serde(default)]
    pub completion_members: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ShipObjectiveInstanceView {
    pub key: ObjectiveInstanceKey,
    pub status: ObjectiveStatus,
    pub progress: f32,
    pub assigned: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssignmentConflict {
    pub objective_id: String,
    pub ship_id: String,
    pub instance_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ActivationRefusal {
    EmptyRecipients,
    UnknownShipSlot(String),
    UnknownFaction(String),
    StaticShipSlotConflict {
        objective_id: String,
        slot_id: String,
        instance_ids: Vec<String>,
    },
    Conflict(AssignmentConflict),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectiveInstanceTransition {
    pub key: ObjectiveInstanceKey,
    pub status: ObjectiveStatus,
    pub completion_members: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ObjectiveInstanceManager {
    instances: Vec<ObjectiveInstanceRecord>,
    /// Frozen per-ship last-known views, keyed by Objective definition.
    history: BTreeMap<String, BTreeMap<String, ShipObjectiveInstanceView>>,
    /// Last fleet whose membership was accepted. This is retained through a
    /// snapshot so an ambiguous faction change can be rolled back atomically
    /// after restore as well as during uninterrupted play.
    #[serde(default)]
    accepted_fleet: Vec<PlayerShipMembership>,
    #[serde(skip)]
    dirty: bool,
    /// Transient story edges. Deliberately excluded from save data so restore
    /// cannot replay a completion or reward.
    #[serde(skip)]
    transitions: Vec<ObjectiveInstanceTransition>,
}

impl ObjectiveInstanceManager {
    pub fn activate_validated(
        &mut self,
        spec: ObjectiveInstanceSpec,
        fleet: &[PlayerShipMembership],
        known_slots: &BTreeSet<String>,
        known_factions: &BTreeSet<String>,
    ) -> Result<bool, ActivationRefusal> {
        if spec.recipients.is_empty() {
            return Err(ActivationRefusal::EmptyRecipients);
        }
        for selector in &spec.recipients {
            match selector {
                RecipientSelector::ShipSlot(slot) if !known_slots.contains(slot) => {
                    return Err(ActivationRefusal::UnknownShipSlot(slot.clone()));
                }
                RecipientSelector::Faction(faction) if !known_factions.contains(faction) => {
                    return Err(ActivationRefusal::UnknownFaction(faction.clone()));
                }
                _ => {}
            }
        }
        // An authored slot present in two instances of one Objective always
        // ties at the highest specificity whenever that slot launches. This
        // can be rejected before any ship exists; faction overlaps may be
        // hidden by a more specific slot and remain a live-fleet check.
        for row in &self.instances {
            if row.spec.key.objective_id != spec.key.objective_id || row.spec.key == spec.key {
                continue;
            }
            for selector in &spec.recipients {
                if let RecipientSelector::ShipSlot(slot_id) = selector {
                    if row.spec.recipients.contains(selector) {
                        let mut instance_ids = vec![
                            row.spec.key.instance_id.clone(),
                            spec.key.instance_id.clone(),
                        ];
                        instance_ids.sort();
                        return Err(ActivationRefusal::StaticShipSlotConflict {
                            objective_id: spec.key.objective_id.clone(),
                            slot_id: slot_id.clone(),
                            instance_ids,
                        });
                    }
                }
            }
        }
        self.activate(spec, fleet)
            .map_err(ActivationRefusal::Conflict)
    }

    pub fn activate(
        &mut self,
        spec: ObjectiveInstanceSpec,
        fleet: &[PlayerShipMembership],
    ) -> Result<bool, AssignmentConflict> {
        if self.instances.iter().any(|row| row.spec.key == spec.key) {
            return Ok(false);
        }
        let candidate = ObjectiveInstanceRecord {
            spec,
            status: ObjectiveStatus::Active,
            progress: 0.0,
            directive: None,
            completion_members: Vec::new(),
        };
        self.instances.push(candidate);
        if let Err(conflict) = self.assignments(fleet) {
            self.instances.pop();
            return Err(conflict);
        }
        self.reconcile(fleet)?;
        self.dirty = true;
        let row = self.instances.last().expect("activated instance retained");
        self.transitions.push(ObjectiveInstanceTransition {
            key: row.spec.key.clone(),
            status: ObjectiveStatus::Active,
            completion_members: Vec::new(),
        });
        Ok(true)
    }

    pub fn set_progress(&mut self, key: &ObjectiveInstanceKey, progress: f32) -> bool {
        let Some(row) = self.instances.iter_mut().find(|row| &row.spec.key == key) else {
            return false;
        };
        if row.status != ObjectiveStatus::Active || !progress.is_finite() || progress < 0.0 {
            return false;
        }
        row.progress = progress;
        for views in self.history.values_mut() {
            if let Some(view) = views
                .values_mut()
                .find(|view| view.assigned && &view.key == key)
            {
                view.progress = progress;
            }
        }
        self.dirty = true;
        true
    }

    pub fn set_directive(
        &mut self,
        key: &ObjectiveInstanceKey,
        directive: crate::core::messages::AiDirective,
    ) -> bool {
        let Some(row) = self.instances.iter_mut().find(|row| &row.spec.key == key) else {
            return false;
        };
        row.directive = Some(directive);
        self.dirty = true;
        true
    }

    /// Returns true exactly once. Callers attach lifecycle effects/rewards only
    /// to true, so restore and later joins cannot replay completion.
    pub fn complete(
        &mut self,
        key: &ObjectiveInstanceKey,
        fleet: &[PlayerShipMembership],
    ) -> Result<bool, AssignmentConflict> {
        let assignments = self.assignments(fleet)?;
        let Some(row) = self.instances.iter_mut().find(|row| &row.spec.key == key) else {
            return Ok(false);
        };
        if row.status != ObjectiveStatus::Active {
            return Ok(false);
        }
        row.status = ObjectiveStatus::Completed;
        row.completion_members = assignments
            .iter()
            .filter(|(_, assigned)| *assigned == key)
            .map(|((ship, _), _)| ship.clone())
            .collect();
        row.completion_members.sort();
        let transition = ObjectiveInstanceTransition {
            key: row.spec.key.clone(),
            status: ObjectiveStatus::Completed,
            completion_members: row.completion_members.clone(),
        };
        self.reconcile(fleet)?;
        self.dirty = true;
        self.transitions.push(transition);
        Ok(true)
    }

    pub fn fail(
        &mut self,
        key: &ObjectiveInstanceKey,
        fleet: &[PlayerShipMembership],
    ) -> Result<bool, AssignmentConflict> {
        self.assignments(fleet)?;
        let Some(row) = self.instances.iter_mut().find(|row| &row.spec.key == key) else {
            return Ok(false);
        };
        if row.status != ObjectiveStatus::Active {
            return Ok(false);
        }
        row.status = ObjectiveStatus::Failed;
        let transition = ObjectiveInstanceTransition {
            key: row.spec.key.clone(),
            status: ObjectiveStatus::Failed,
            completion_members: Vec::new(),
        };
        self.reconcile(fleet)?;
        self.dirty = true;
        self.transitions.push(transition);
        Ok(true)
    }

    /// Re-resolve live faction membership atomically. On ambiguity neither the
    /// instances nor any ship's last-known projection changes.
    pub fn reconcile(&mut self, fleet: &[PlayerShipMembership]) -> Result<(), AssignmentConflict> {
        let assignments = self.assignments(fleet)?;
        let mut next = self.history.clone();
        for history in next.values_mut() {
            for view in history.values_mut() {
                view.assigned = false;
            }
        }
        for ((ship_id, objective_id), key) in &assignments {
            let row = self
                .instances
                .iter()
                .find(|row| &row.spec.key == key)
                .expect("assignment names retained instance");
            next.entry(ship_id.clone()).or_default().insert(
                objective_id.clone(),
                ShipObjectiveInstanceView {
                    key: key.clone(),
                    status: row.status.clone(),
                    progress: row.progress,
                    assigned: true,
                },
            );
        }
        if self.history != next {
            self.history = next;
            self.dirty = true;
        }
        if self.accepted_fleet != fleet {
            self.accepted_fleet = fleet.to_vec();
            self.dirty = true;
        }
        Ok(())
    }

    pub fn accepted_fleet(&self) -> &[PlayerShipMembership] {
        &self.accepted_fleet
    }

    pub fn view_for_ship(&self, ship_id: &str) -> Vec<&ShipObjectiveInstanceView> {
        self.history
            .get(ship_id)
            .map(|rows| rows.values().collect())
            .unwrap_or_default()
    }

    /// A departed instance is retained for display, never for Captain control.
    pub fn is_unassigned_display_key(&self, ship_id: &str, id: &str) -> bool {
        self.view_for_ship(ship_id)
            .into_iter()
            .any(|view| !view.assigned && display_key(&view.key) == id)
    }

    pub fn records(&self) -> &[ObjectiveInstanceRecord] {
        &self.instances
    }

    /// Current effective recipient UUIDs for GM/report projections. This is
    /// deliberately distinct from `completion_members`, which freezes once at
    /// the successful terminal transition.
    pub fn current_members(&self, key: &ObjectiveInstanceKey) -> Vec<String> {
        self.history
            .iter()
            .filter(|(_, definitions)| {
                definitions
                    .get(&key.objective_id)
                    .is_some_and(|view| view.assigned && &view.key == key)
            })
            .map(|(ship_id, _)| ship_id.clone())
            .collect()
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn mark_clean(&mut self) {
        self.dirty = false;
    }

    pub fn drain_transitions(&mut self) -> Vec<ObjectiveInstanceTransition> {
        std::mem::take(&mut self.transitions)
    }

    /// Project legacy definition snapshots through this ship's effective named
    /// instances. With no instances, returns the input unchanged for complete
    /// backward compatibility. Subject `targets` remain definition metadata;
    /// recipient membership is used only to choose the instance.
    pub fn project_snapshots_for_ship(
        &self,
        ship_id: &str,
        snapshots: Vec<ObjectiveSnapshot>,
    ) -> Vec<ObjectiveSnapshot> {
        if self.instances.is_empty() {
            return snapshots;
        }
        snapshots
            .into_iter()
            .filter_map(|mut snapshot| {
                // Definitions with no named instances remain legacy rows even
                // when some other definition is instanced.
                if !self.has_definition_instances(&snapshot.id) {
                    return Some(snapshot);
                }
                let view = self.history.get(ship_id)?.get(&snapshot.id)?;
                // Keep the ship's frozen last-known row after reassignment.
                // `assigned` controls current actionability, not history.
                snapshot.id = display_key(&view.key);
                snapshot.status = view.status.clone();
                snapshot.unassigned = !view.assigned;
                snapshot.progress = Some(view.progress);
                Some(snapshot)
            })
            .collect()
    }

    /// AI/Captain/Comms twin of [`Self::project_snapshots_for_ship`]. Terminal
    /// instances are not actionable even when their legacy definition remains
    /// active, and each surviving directive carries the instance key.
    pub fn project_scored_for_ship(
        &self,
        ship_id: &str,
        scored: Vec<ScoredObjective>,
    ) -> Vec<ScoredObjective> {
        if self.instances.is_empty() {
            return scored;
        }
        scored
            .into_iter()
            .filter_map(|mut row| {
                if !self.has_definition_instances(&row.id) {
                    return Some(row);
                }
                let view = self
                    .history
                    .get(ship_id)?
                    .get(&row.id)
                    .filter(|view| view.assigned && view.status == ObjectiveStatus::Active)?;
                row.id = display_key(&view.key);
                row.snapshot.id = row.id.clone();
                row.snapshot.status = view.status.clone();
                row.snapshot.progress = Some(view.progress);
                if let Some(directive) = self
                    .instances
                    .iter()
                    .find(|record| record.spec.key == view.key)
                    .and_then(|record| record.directive.clone())
                {
                    row.directive = directive;
                    row.relevance = crate::objectives::directive_relevance(&row.directive);
                }
                Some(row)
            })
            .collect()
    }

    pub fn has_definition_instances(&self, objective_id: &str) -> bool {
        self.instances
            .iter()
            .any(|row| row.spec.key.objective_id == objective_id)
    }

    pub fn is_assigned_active(&self, ship_id: &str, objective_id: &str) -> bool {
        self.history
            .get(ship_id)
            .and_then(|rows| rows.get(objective_id))
            .is_some_and(|view| view.assigned && view.status == ObjectiveStatus::Active)
    }

    /// Resolve a projected row identity by comparing against the live typed
    /// keys. Never split on `::`: authored ids may contain that sequence.
    pub fn key_for_display(&self, display: &str) -> Option<ObjectiveInstanceKey> {
        self.instances
            .iter()
            .map(|record| &record.spec.key)
            .find(|key| display_key(key) == display)
            .cloned()
    }

    fn assignments(
        &self,
        fleet: &[PlayerShipMembership],
    ) -> Result<BTreeMap<(String, String), ObjectiveInstanceKey>, AssignmentConflict> {
        let mut result = BTreeMap::new();
        let objectives: BTreeSet<_> = self
            .instances
            .iter()
            .map(|row| row.spec.key.objective_id.as_str())
            .collect();
        for ship in fleet {
            for objective_id in &objectives {
                let mut matches: Vec<_> = self
                    .instances
                    .iter()
                    .filter(|row| row.spec.key.objective_id == *objective_id)
                    .filter_map(|row| {
                        match_specificity(&row.spec.recipients, ship)
                            .map(|score| (score, &row.spec.key))
                    })
                    .collect();
                let Some(best) = matches.iter().map(|(score, _)| *score).max() else {
                    continue;
                };
                matches.retain(|(score, _)| *score == best);
                if matches.len() > 1 {
                    let mut ids: Vec<_> = matches
                        .iter()
                        .map(|(_, key)| key.instance_id.clone())
                        .collect();
                    ids.sort();
                    return Err(AssignmentConflict {
                        objective_id: (*objective_id).to_string(),
                        ship_id: ship.ship_id.clone(),
                        instance_ids: ids,
                    });
                }
                result.insert(
                    (ship.ship_id.clone(), (*objective_id).to_string()),
                    matches[0].1.clone(),
                );
            }
        }
        Ok(result)
    }
}

fn display_key(key: &ObjectiveInstanceKey) -> String {
    format!("{}::{}", key.objective_id, key.instance_id)
}

pub fn match_specificity(
    selectors: &[RecipientSelector],
    ship: &PlayerShipMembership,
) -> Option<u8> {
    selectors
        .iter()
        .filter_map(|selector| match selector {
            RecipientSelector::ShipSlot(slot) if slot == &ship.slot_id => Some(3),
            RecipientSelector::Faction(faction) if faction == &ship.faction => Some(2),
            RecipientSelector::AllPlayerShips => Some(1),
            _ => None,
        })
        .max()
}
