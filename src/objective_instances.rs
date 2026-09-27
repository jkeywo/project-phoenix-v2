//! Explicitly shared Objective instances (issue #1523).
//!
//! Legacy Objectives remain in `objectives::ObjectiveManager`; this module is
//! the additive multi-ship contract. Recipients are ship slots, not subject
//! targets, and every mutation addresses `(objective_id, instance_id)`.

use std::collections::{BTreeMap, BTreeSet};

use crate::core::messages::ObjectiveStatus;
use crate::core::messages::{ObjectiveSnapshot, ScoredObjective};
use bevy::prelude::*;

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

#[derive(Clone, Debug)]
pub struct ObjectiveCommandOrigin {
    pub source: Option<String>,
    pub line: Option<usize>,
    pub tick: u64,
}

impl ObjectiveCommandOrigin {
    fn refused(&self, world: &mut World, message: String) {
        crate::recipients::report(
            world,
            crate::recipients::RecipientDiagnostic {
                tick: self.tick,
                source: self.source.clone(),
                line: self.line,
                action: "objective-instance".into(),
                message,
            },
        );
    }
}

fn activation_refusal_message(reason: &ActivationRefusal) -> String {
    match reason {
        ActivationRefusal::EmptyRecipients => "Choose at least one Objective recipient".into(),
        ActivationRefusal::UnknownShipSlot(slot) => format!("Unknown Objective ship slot '{slot}'"),
        ActivationRefusal::UnknownFaction(faction) => format!("Unknown Objective faction '{faction}'"),
        ActivationRefusal::StaticShipSlotConflict { objective_id, slot_id, instance_ids } =>
            format!("Objective '{objective_id}' gives ship slot '{slot_id}' equal-specificity assignments in instances {}. Change one recipient selector", instance_ids.join(", ")),
        ActivationRefusal::Conflict(conflict) =>
            format!("Objective '{}' gives ship '{}' equal-specificity assignments in instances {}. Change one recipient selector", conflict.objective_id, conflict.ship_id, conflict.instance_ids.join(", ")),
    }
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

fn match_specificity(selectors: &[RecipientSelector], ship: &PlayerShipMembership) -> Option<u8> {
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

/// Deterministic live fleet projection used by every Objective-instance
/// lifecycle mutation. Authored slot identity is never reconstructed from
/// transport `HostSlot` ordering.
pub fn player_ship_memberships(world: &mut World) -> Vec<PlayerShipMembership> {
    let mut query = world.query_filtered::<(
        &crate::entities::spawner::EntityUuid,
        &crate::ship_slots::AuthoredShipSlotId,
        Option<&crate::entities::spawner::FactionComponent>,
    ), With<crate::server_app::Ship>>();
    let raw: Vec<_> = query
        .iter(world)
        .map(|(uuid, slot, faction)| (uuid.0.clone(), slot.0.clone(), faction.map(|f| f.0)))
        .collect();
    let registry = world.get_resource::<crate::entities::config_cache::FactionRegistryResource>();
    let mut fleet: Vec<_> = raw
        .into_iter()
        .map(|(ship_id, slot_id, faction)| PlayerShipMembership {
            ship_id,
            slot_id,
            faction: faction
                .and_then(|id| registry.and_then(|registry| registry.get(&id)))
                .map(|faction| faction.name.clone())
                .unwrap_or_default(),
        })
        .collect();
    fleet.sort_by(|a, b| a.ship_id.cmp(&b.ship_id).then(a.slot_id.cmp(&b.slot_id)));
    fleet
}

pub fn reconcile_memberships(world: &mut World) {
    let fleet = player_ship_memberships(world);
    let refused = {
        let mut manager = world.resource_mut::<crate::world::server::ObjectiveInstanceManagerRes>();
        manager
            .0
            .reconcile(&fleet)
            .err()
            .map(|conflict| (conflict, manager.0.accepted_fleet().to_vec()))
    };
    if let Some((conflict, accepted)) = refused {
        // The manager refused the candidate without changing its history. A
        // live faction edit must likewise leave the ship on its old faction;
        // otherwise combat and Objectives would disagree about membership.
        // Use an exclusive system so the rollback lands before any later
        // fixed-step reader, rather than through deferred Commands.
        let restore: BTreeMap<_, _> = accepted
            .iter()
            .map(|member| {
                let faction = world
                    .get_resource::<crate::entities::config_cache::FactionRegistryResource>()
                    .and_then(|registry| registry.uuid_by_name(&member.faction));
                (member.ship_id.as_str(), faction)
            })
            .collect();
        let mut query = world.query_filtered::<(Entity, &crate::entities::spawner::EntityUuid), With<crate::server_app::Ship>>();
        let entities: Vec<_> = query
            .iter(world)
            .filter_map(|(entity, uuid)| {
                restore
                    .get(uuid.0.as_str())
                    .map(|faction| (entity, *faction))
            })
            .collect();
        for (entity, faction) in entities {
            if let Some(faction) = faction {
                world
                    .entity_mut(entity)
                    .insert(crate::entities::spawner::FactionComponent(faction));
            } else {
                world
                    .entity_mut(entity)
                    .remove::<crate::entities::spawner::FactionComponent>();
            }
        }
        bevy::log::warn!(
            "Objective instance reconciliation refused: objective '{}' is ambiguous for ship '{}' across {:?}",
            conflict.objective_id,
            conflict.ship_id,
            conflict.instance_ids
        );
    }
}

/// Apply the instance-only objective commands queued by the shared world
/// dispatcher. Legacy commands never enter this function and therefore retain
/// their existing reducer byte-for-byte.
pub fn apply_command(
    world: &mut World,
    command: crate::world::dispatch::ActionCmd,
    origin: &ObjectiveCommandOrigin,
) {
    use crate::world::dispatch::ActionCmd;

    let fleet = player_ship_memberships(world);
    let known_slots: BTreeSet<_> = world
        .get_resource::<crate::world::config::WorldConfig>()
        .map(|config| {
            config
                .effective_ship_slots()
                .into_iter()
                .map(|slot| slot.id)
                .collect()
        })
        .unwrap_or_default();
    let known_factions: BTreeSet<_> = world
        .get_resource::<crate::entities::config_cache::FactionRegistryResource>()
        .map(|registry| {
            registry
                .iter()
                .map(|faction| faction.name.clone())
                .collect()
        })
        .unwrap_or_default();
    let mut transition: Option<(String, ObjectiveStatus)> = None;
    match command {
        ActionCmd::AddObjectiveInstance {
            spec,
            text,
            text_params,
            mandatory,
            targets,
            directive,
            utility,
            source,
            command_stance,
            origin_layer,
        } => {
            let objective_id = spec.key.objective_id.clone();
            let instance_key = spec.key.clone();
            let instance_directive = directive.clone();
            let activation = world
                .resource_mut::<crate::world::server::ObjectiveInstanceManagerRes>()
                .0
                .activate_validated(spec, &fleet, &known_slots, &known_factions);
            let activated = match activation {
                Ok(changed) => changed,
                Err(reason) => {
                    bevy::log::warn!("Objective instance activation refused: {reason:?}");
                    origin.refused(world, activation_refusal_message(&reason));
                    return;
                }
            };
            if activated {
                world
                    .resource_mut::<crate::world::server::ObjectiveInstanceManagerRes>()
                    .0
                    .set_directive(&instance_key, instance_directive);
            }
            let inserted = world
                .resource_mut::<crate::world::server::ObjectiveManagerRes>()
                .0
                .add_full_with_params(
                    objective_id.clone(),
                    text,
                    text_params,
                    mandatory,
                    targets,
                    directive,
                    utility,
                    source,
                    command_stance,
                );
            if inserted {
                if let Some(path) = origin_layer {
                    if let Some(mut layers) =
                        world.get_resource_mut::<crate::world::server::WorldLayerMap>()
                    {
                        if let Some(layer) = layers.0.get_mut(&path) {
                            layer.owned_objective_ids.push(objective_id.clone());
                        }
                    }
                }
            }
            if activated {
                transition = Some((objective_id, ObjectiveStatus::Active));
            }
        }
        ActionCmd::CompleteObjectiveInstance { key } => {
            let known = world
                .resource::<crate::world::server::ObjectiveInstanceManagerRes>()
                .0
                .records()
                .iter()
                .any(|row| row.spec.key == key);
            let completion = world
                .resource_mut::<crate::world::server::ObjectiveInstanceManagerRes>()
                .0
                .complete(&key, &fleet);
            match completion {
                Ok(true) => transition = Some((key.objective_id, ObjectiveStatus::Completed)),
                Ok(false) if !known => origin.refused(
                    world,
                    format!(
                        "Unknown Objective instance '{}:{}'; check the authored id and instance_id",
                        key.objective_id, key.instance_id
                    ),
                ),
                Ok(false) => {}
                Err(conflict) => {
                    bevy::log::warn!(
                        "Objective instance completion refused: objective '{}' is ambiguous for ship '{}' across {:?}",
                        conflict.objective_id,
                        conflict.ship_id,
                        conflict.instance_ids
                    );
                    origin.refused(
                        world,
                        activation_refusal_message(&ActivationRefusal::Conflict(conflict)),
                    );
                }
            }
        }
        ActionCmd::FailObjectiveInstance { key } => {
            let known = world
                .resource::<crate::world::server::ObjectiveInstanceManagerRes>()
                .0
                .records()
                .iter()
                .any(|row| row.spec.key == key);
            let failure = world
                .resource_mut::<crate::world::server::ObjectiveInstanceManagerRes>()
                .0
                .fail(&key, &fleet);
            match failure {
                Ok(true) => transition = Some((key.objective_id, ObjectiveStatus::Failed)),
                Ok(false) if !known => origin.refused(
                    world,
                    format!(
                        "Unknown Objective instance '{}:{}'; check the authored id and instance_id",
                        key.objective_id, key.instance_id
                    ),
                ),
                Ok(false) => {}
                Err(conflict) => {
                    bevy::log::warn!(
                        "Objective instance failure refused: objective '{}' is ambiguous for ship '{}' across {:?}",
                        conflict.objective_id,
                        conflict.ship_id,
                        conflict.instance_ids
                    );
                    origin.refused(
                        world,
                        activation_refusal_message(&ActivationRefusal::Conflict(conflict)),
                    );
                }
            }
        }
        ActionCmd::SetObjectiveInstanceProgress { key, progress } => {
            let changed = world
                .resource_mut::<crate::world::server::ObjectiveInstanceManagerRes>()
                .0
                .set_progress(&key, progress);
            if !changed {
                origin.refused(world, format!(
                    "Objective instance '{}:{}' was not active; check the authored ids and progress",
                    key.objective_id, key.instance_id
                ));
            }
        }
        _ => return,
    }

    if let Some((objective_id, status)) = transition {
        let targets = world
            .resource::<crate::world::server::ObjectiveManagerRes>()
            .0
            .targets(&objective_id)
            .unwrap_or_default()
            .to_vec();
        if let Some(mut messages) = world
            .get_resource_mut::<bevy::ecs::message::Messages<crate::core::balance::BalanceEvent>>()
        {
            if status == ObjectiveStatus::Completed {
                messages.write(crate::core::balance::BalanceEvent::ObjectiveCompleted {
                    objective_id: objective_id.clone(),
                });
            }
            messages.write(crate::core::balance::BalanceEvent::ObjectiveChanged {
                objective_id,
                status,
                targets,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::messages::ObjectiveSource;
    use bevy::ecs::system::RunSystemOnce;

    fn snapshot(id: &str) -> ObjectiveSnapshot {
        ObjectiveSnapshot {
            unassigned: false,
            id: id.into(),
            text: format!("objective.{id}"),
            text_params: Default::default(),
            mandatory: false,
            status: ObjectiveStatus::Active,
            targets: Vec::new(),
            source: ObjectiveSource::Mission,
        }
    }

    fn key(instance: &str) -> ObjectiveInstanceKey {
        ObjectiveInstanceKey {
            objective_id: "survive".into(),
            instance_id: instance.into(),
        }
    }
    fn ship(id: &str, slot: &str, faction: &str) -> PlayerShipMembership {
        PlayerShipMembership {
            ship_id: id.into(),
            slot_id: slot.into(),
            faction: faction.into(),
        }
    }
    fn spec(instance: &str, recipients: Vec<RecipientSelector>) -> ObjectiveInstanceSpec {
        ObjectiveInstanceSpec {
            key: key(instance),
            recipients,
        }
    }

    #[test]
    fn multi_ship_explicit_slot_beats_faction_and_instances_progress_independently() {
        let fleet = [ship("a", "lead", "alliance"), ship("b", "wing", "alliance")];
        let mut manager = ObjectiveInstanceManager::default();
        manager
            .activate(
                spec("fleet", vec![RecipientSelector::Faction("alliance".into())]),
                &fleet,
            )
            .unwrap();
        manager
            .activate(
                spec("lead", vec![RecipientSelector::ShipSlot("lead".into())]),
                &fleet,
            )
            .unwrap();
        manager.set_progress(&key("fleet"), 0.25);
        manager.set_progress(&key("lead"), 0.75);
        assert_eq!(manager.view_for_ship("a")[0].key.instance_id, "lead");
        assert_eq!(manager.view_for_ship("a")[0].progress, 0.75);
        assert_eq!(manager.view_for_ship("b")[0].key.instance_id, "fleet");
    }

    #[test]
    fn multi_ship_completion_credit_is_fixed_and_restore_replays_nothing() {
        let fleet = [ship("a", "lead", "alliance"), ship("b", "wing", "alliance")];
        let mut manager = ObjectiveInstanceManager::default();
        manager
            .activate(
                spec("fleet", vec![RecipientSelector::Faction("alliance".into())]),
                &fleet,
            )
            .unwrap();
        manager.drain_transitions();
        assert!(manager.complete(&key("fleet"), &fleet).unwrap());
        let transitions = manager.drain_transitions();
        assert_eq!(transitions.len(), 1);
        assert_eq!(transitions[0].completion_members, ["a", "b"]);
        assert!(!manager.complete(&key("fleet"), &fleet).unwrap());
        let json = serde_json::to_string(&manager).unwrap();
        let mut restored: ObjectiveInstanceManager = serde_json::from_str(&json).unwrap();
        assert!(restored.drain_transitions().is_empty());
        let changed = [
            ship("a", "lead", "dynasty"),
            ship("b", "wing", "alliance"),
            ship("c", "reserve", "alliance"),
        ];
        restored.reconcile(&changed).unwrap();
        assert_eq!(restored.records()[0].completion_members, ["a", "b"]);
        assert!(!restored.complete(&key("fleet"), &changed).unwrap());
        assert!(!restored.view_for_ship("a")[0].assigned);
        assert_eq!(
            restored.view_for_ship("c")[0].status,
            ObjectiveStatus::Completed
        );
    }

    #[test]
    fn multi_ship_equal_specificity_conflict_is_atomic() {
        let fleet = [ship("a", "lead", "alliance")];
        let mut manager = ObjectiveInstanceManager::default();
        manager
            .activate(
                spec("one", vec![RecipientSelector::Faction("alliance".into())]),
                &fleet,
            )
            .unwrap();
        let before = manager.clone();
        let conflict = manager
            .activate(
                spec("two", vec![RecipientSelector::Faction("alliance".into())]),
                &fleet,
            )
            .unwrap_err();
        assert_eq!(conflict.instance_ids, ["one", "two"]);
        assert_eq!(manager, before);
    }

    #[test]
    fn live_faction_conflict_restores_the_ship_and_keeps_frozen_progress() {
        let alpha = uuid::Uuid::parse_str("aaaaaaaa-0000-0000-0000-000000000001").unwrap();
        let beta = uuid::Uuid::parse_str("bbbbbbbb-0000-0000-0000-000000000002").unwrap();
        let mut registry = crate::ai::faction::FactionRegistry::new();
        for (uuid, name) in [(alpha, "alpha"), (beta, "beta")] {
            registry.insert(crate::ai::faction::FactionConfig {
                uuid,
                name: name.into(),
                display_name: None,
                enemies: Vec::new(),
                compliance: None,
            });
        }
        let original = [ship("ship-a", "lead", "alpha")];
        let mut manager = ObjectiveInstanceManager::default();
        manager
            .activate(
                spec("alpha", vec![RecipientSelector::Faction("alpha".into())]),
                &original,
            )
            .unwrap();
        manager
            .activate(
                spec("beta-one", vec![RecipientSelector::Faction("beta".into())]),
                &original,
            )
            .unwrap();
        manager
            .activate(
                spec("beta-two", vec![RecipientSelector::Faction("beta".into())]),
                &original,
            )
            .unwrap();
        manager.set_progress(&key("alpha"), 0.6);
        let restored: ObjectiveInstanceManager =
            serde_json::from_str(&serde_json::to_string(&manager).unwrap()).unwrap();
        // Save data intentionally excludes transient story edges and dirty.
        let before = restored.clone();
        let peer_manager = restored.clone();
        let peer_registry = registry.clone();

        let mut world = World::new();
        world.insert_resource(crate::entities::config_cache::FactionRegistryResource(
            registry,
        ));
        world.insert_resource(crate::world::server::ObjectiveInstanceManagerRes(restored));
        let entity = world
            .spawn((
                crate::server_app::Ship,
                crate::entities::spawner::EntityUuid("ship-a".into()),
                crate::ship_slots::AuthoredShipSlotId("lead".into()),
                crate::entities::spawner::FactionComponent(beta),
            ))
            .id();

        world.run_system_once(reconcile_memberships).unwrap();
        assert_eq!(
            world
                .get::<crate::entities::spawner::FactionComponent>(entity)
                .unwrap()
                .0,
            alpha
        );
        assert_eq!(
            world
                .resource::<crate::world::server::ObjectiveInstanceManagerRes>()
                .0,
            before
        );
        assert_eq!(
            world
                .resource::<crate::world::server::ObjectiveInstanceManagerRes>()
                .0
                .view_for_ship("ship-a")[0]
                .progress,
            0.6
        );

        // A second deterministic host applies the same candidate against a
        // restored manager. Both reject the faction edit and retain identical
        // projections, including the previous accepted membership.
        let mut peer = World::new();
        peer.insert_resource(crate::entities::config_cache::FactionRegistryResource(
            peer_registry,
        ));
        peer.insert_resource(crate::world::server::ObjectiveInstanceManagerRes(
            peer_manager,
        ));
        let peer_entity = peer
            .spawn((
                crate::server_app::Ship,
                crate::entities::spawner::EntityUuid("ship-a".into()),
                crate::ship_slots::AuthoredShipSlotId("lead".into()),
                crate::entities::spawner::FactionComponent(beta),
            ))
            .id();
        peer.run_system_once(reconcile_memberships).unwrap();
        assert_eq!(
            peer.get::<crate::entities::spawner::FactionComponent>(peer_entity)
                .unwrap()
                .0,
            alpha
        );
        assert_eq!(
            peer.resource::<crate::world::server::ObjectiveInstanceManagerRes>()
                .0,
            world
                .resource::<crate::world::server::ObjectiveInstanceManagerRes>()
                .0,
        );
    }

    #[test]
    fn switching_instances_replaces_displayed_progress_without_leaking_old_updates() {
        let alpha = [ship("ship-a", "lead", "alpha")];
        let beta = [ship("ship-a", "lead", "beta")];
        let outside = [ship("ship-a", "lead", "outside")];
        let mut manager = ObjectiveInstanceManager::default();
        manager
            .activate(
                spec("alpha", vec![RecipientSelector::Faction("alpha".into())]),
                &alpha,
            )
            .unwrap();
        manager
            .activate(
                spec("beta", vec![RecipientSelector::Faction("beta".into())]),
                &alpha,
            )
            .unwrap();
        manager.set_progress(&key("alpha"), 0.25);
        manager.set_progress(&key("beta"), 0.75);

        manager.reconcile(&beta).unwrap();
        assert_eq!(manager.view_for_ship("ship-a")[0].key, key("beta"));
        assert_eq!(manager.view_for_ship("ship-a")[0].progress, 0.75);
        manager.set_progress(&key("alpha"), 0.9);
        assert_eq!(manager.view_for_ship("ship-a")[0].progress, 0.75);

        manager.reconcile(&outside).unwrap();
        assert!(!manager.view_for_ship("ship-a")[0].assigned);
        assert_eq!(manager.view_for_ship("ship-a")[0].progress, 0.75);
        manager.set_progress(&key("beta"), 0.8);
        assert_eq!(manager.view_for_ship("ship-a")[0].progress, 0.75);
    }

    #[test]
    fn multi_ship_activation_refuses_empty_and_dangling_selectors() {
        let fleet = [ship("a", "lead", "alliance")];
        let slots = BTreeSet::from(["lead".to_string()]);
        let factions = BTreeSet::from(["alliance".to_string()]);
        let mut manager = ObjectiveInstanceManager::default();
        assert_eq!(
            manager.activate_validated(spec("empty", vec![]), &fleet, &slots, &factions),
            Err(ActivationRefusal::EmptyRecipients)
        );
        assert_eq!(
            manager.activate_validated(
                spec("missing", vec![RecipientSelector::ShipSlot("wing".into())]),
                &fleet,
                &slots,
                &factions,
            ),
            Err(ActivationRefusal::UnknownShipSlot("wing".into()))
        );
        assert!(manager.records().is_empty());
    }

    #[test]
    fn duplicate_explicit_slot_is_rejected_before_that_ship_joins() {
        let slots = BTreeSet::from(["lead".to_string()]);
        let mut manager = ObjectiveInstanceManager::default();
        assert_eq!(
            manager.activate_validated(
                spec("one", vec![RecipientSelector::ShipSlot("lead".into())]),
                &[],
                &slots,
                &BTreeSet::new(),
            ),
            Ok(true)
        );
        let before = manager.clone();
        assert_eq!(
            manager.activate_validated(
                spec("two", vec![RecipientSelector::ShipSlot("lead".into())]),
                &[],
                &slots,
                &BTreeSet::new(),
            ),
            Err(ActivationRefusal::StaticShipSlotConflict {
                objective_id: "survive".into(),
                slot_id: "lead".into(),
                instance_ids: vec!["one".into(), "two".into()],
            })
        );
        assert_eq!(manager, before);
    }

    #[test]
    fn multi_ship_projection_preserves_unrelated_legacy_and_frozen_history() {
        let fleet = [ship("a", "lead", "alliance")];
        let mut manager = ObjectiveInstanceManager::default();
        manager
            .activate(
                spec("lead", vec![RecipientSelector::ShipSlot("lead".into())]),
                &fleet,
            )
            .unwrap();

        let projected =
            manager.project_snapshots_for_ship("a", vec![snapshot("survive"), snapshot("legacy")]);
        assert_eq!(projected.len(), 2);
        assert_eq!(projected[0].id, "survive::lead");
        assert!(!projected[0].unassigned);
        assert_eq!(projected[1].id, "legacy");

        manager
            .reconcile(&[ship("a", "reserve", "alliance")])
            .unwrap();
        let projected = manager.project_snapshots_for_ship("a", vec![snapshot("survive")]);
        assert_eq!(projected.len(), 1);
        assert_eq!(projected[0].id, "survive::lead");
        assert!(projected[0].unassigned);
        assert!(manager.is_unassigned_display_key("a", "survive::lead"));
    }

    #[test]
    fn multi_ship_toml_action_parses_typed_instance_recipients_and_progress() {
        use crate::world::config::{parse_action_entry, RawActionEntry, TriggerAction};

        let add = parse_action_entry(&RawActionEntry {
            kind: "add_objective".into(),
            id: Some("survive".into()),
            instance_id: Some("lead".into()),
            text: Some("objective.survive".into()),
            recipient_ship_slots: Some(vec!["lead".into()]),
            ..Default::default()
        })
        .unwrap();
        assert!(matches!(
            add,
            TriggerAction::AddObjectiveInstance { spec, .. }
                if spec.key == key("lead")
                    && spec.recipients == [RecipientSelector::ShipSlot("lead".into())]
        ));

        let progress = parse_action_entry(&RawActionEntry {
            kind: "set_objective_progress".into(),
            id: Some("survive".into()),
            instance_id: Some("lead".into()),
            progress: Some(0.5),
            ..Default::default()
        })
        .unwrap();
        assert!(matches!(
            progress,
            TriggerAction::SetObjectiveInstanceProgress { key: parsed, progress }
                if parsed == key("lead") && progress == 0.5
        ));
    }
}
