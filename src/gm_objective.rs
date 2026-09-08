//! Authored Objective controls. Recipients are ships; targets remain subjects.
use crate::core::messages::ObjectiveStatus;
use crate::gm_action::{GmActionOutcome, GmActionRefusalReason};
use crate::world::dispatch::ActionCmd;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ObjectiveVerb {
    Activate,
    Complete,
    Fail,
}

pub(crate) const MAX_OBJECTIVE_RECIPIENTS: usize = 32;

fn valid_request_vocabulary(id: &str, recipients: &[String]) -> bool {
    crate::gm_action::GmAction::ObjectiveAction {
        objective: id.to_string(),
        verb: ObjectiveVerb::Activate,
        recipients: recipients.to_vec(),
    }
    .validate()
    .is_ok()
}

#[derive(Clone, Debug, Deserialize)]
pub struct RawObjectivePaletteEntry {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub recipients: Vec<String>,
    #[serde(flatten)]
    pub fields: toml::Table,
}

#[derive(Clone, Debug)]
pub struct ObjectivePaletteEntry {
    pub id: String,
    pub label: String,
    pub recipients: Vec<String>,
    pub action: crate::world::config::TriggerAction,
    pub origin_layer: Option<String>,
}

pub fn parse_palette(
    raw: &[RawObjectivePaletteEntry],
) -> Result<Vec<ObjectivePaletteEntry>, String> {
    let mut ids = std::collections::BTreeSet::new();
    raw.iter().map(|row| {
        if row.id.is_empty() || row.label.is_empty() || !ids.insert(&row.id) {
            return Err(format!("invalid or duplicate gm_objective_palette id '{}'", row.id));
        }
        let mut fields = row.fields.clone();
        if fields.contains_key("type") { return Err("Objective palette cannot choose an action type".into()); }
        fields.insert("type".into(), toml::Value::String("add_objective".into()));
        fields.insert("id".into(), toml::Value::String(row.id.clone()));
        let raw_action: crate::world::config::RawActionEntry = toml::Value::Table(fields).try_into().map_err(|e| format!("Objective palette: {e}"))?;
        let action = crate::world::config::parse_action_entry(&raw_action)?;
        if !matches!(&action, crate::world::config::TriggerAction::AddObjective { text, .. } if !text.is_empty()) {
            return Err("Objective palette requires authored text".into());
        }
        let mut recipients = row.recipients.clone();
        recipients.sort(); recipients.dedup();
        if row.recipients.len() > MAX_OBJECTIVE_RECIPIENTS || !valid_request_vocabulary(&row.id, &recipients) {
            return Err("Objective palette id or recipients exceed the GM action vocabulary".into());
        }
        Ok(ObjectivePaletteEntry { id: row.id.clone(), label: row.label.clone(), recipients, action, origin_layer: None })
    }).collect()
}

/// Shared normal script/GM lifecycle, including fictional transition log and
/// balance observers. Return false on any repeat, without duplicate events.
pub fn apply_command(
    manager: &mut crate::objectives::ObjectiveManager,
    command: ActionCmd,
    balance: Option<&mut Messages<crate::core::balance::BalanceEvent>>,
    layers: Option<&mut crate::world::server::WorldLayerMap>,
) -> bool {
    let (id, status, changed) = match command {
        ActionCmd::AddObjective {
            id,
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
            let changed = manager.add_full_with_params(
                id.clone(),
                text,
                text_params,
                mandatory,
                targets,
                directive,
                utility,
                source,
                command_stance,
            );
            if changed {
                if let (Some(path), Some(layers)) = (origin_layer, layers) {
                    if let Some(layer) = layers.0.get_mut(&path) {
                        layer.owned_objective_ids.push(id.clone());
                    }
                }
            }
            (id, ObjectiveStatus::Active, changed)
        }
        ActionCmd::CompleteObjective { id } => {
            let changed = manager.complete(&id);
            (id, ObjectiveStatus::Completed, changed)
        }
        ActionCmd::FailObjective { id } => {
            let changed = manager.fail(&id);
            (id, ObjectiveStatus::Failed, changed)
        }
        _ => return false,
    };
    if changed {
        if let Some(events) = balance {
            if status == ObjectiveStatus::Completed {
                events.write(crate::core::balance::BalanceEvent::ObjectiveCompleted {
                    objective_id: id.clone(),
                });
            }
            events.write(crate::core::balance::BalanceEvent::ObjectiveChanged {
                objective_id: id.clone(),
                status,
                targets: manager.targets(&id).unwrap_or_default().to_vec(),
            });
        }
    }
    changed
}

/// Atomic record scope: a terminal change always resolves the whole Objective.
pub fn validate(
    verb: ObjectiveVerb,
    status: Option<&ObjectiveStatus>,
    authored_scope: &[String],
    requested_scope: &[String],
    live_ships: &[String],
) -> Result<GmActionOutcome, GmActionRefusalReason> {
    if authored_scope != requested_scope
        || requested_scope.iter().any(|id| !live_ships.contains(id))
    {
        return Err(GmActionRefusalReason::ObjectiveScopeMismatch);
    }
    match (verb, status) {
        (ObjectiveVerb::Activate, None) => Ok(GmActionOutcome::Applied),
        (ObjectiveVerb::Activate, Some(_)) => Ok(GmActionOutcome::NoOp),
        (_, Some(ObjectiveStatus::Active)) => Ok(GmActionOutcome::Applied),
        (_, Some(_)) => Err(GmActionRefusalReason::ObjectiveNotActive),
        (_, None) => Err(GmActionRefusalReason::UnknownObjective),
    }
}

#[derive(bevy::ecs::system::SystemParam)]
pub struct ObjectiveControl<'w, 's> {
    pub manager: Option<ResMut<'w, crate::world::server::ObjectiveManagerRes>>,
    pub balance: Option<ResMut<'w, Messages<crate::core::balance::BalanceEvent>>>,
    pub layers: Option<ResMut<'w, crate::world::server::WorldLayerMap>>,
    pub removal_targets: crate::gm_despawn::RemovalQuery<'w, 's>,
    pub fleet_ships: Query<
        'w,
        's,
        &'static crate::entities::spawner::EntityUuid,
        With<crate::lockstep::FleetSlotOf>,
    >,
}

pub fn resolve_recipients(
    entry: &ObjectivePaletteEntry,
    runtime: &crate::world::server::WorldContentRuntime,
    live: &[String],
) -> Option<Vec<String>> {
    let mut resolved = Vec::new();
    for name in &entry.recipients {
        let uuid = runtime.name_to_uuid.get(name).unwrap_or(name);
        if !live.contains(uuid) {
            return None;
        }
        resolved.push(uuid.clone());
    }
    resolved.sort();
    resolved.dedup();
    valid_request_vocabulary(&entry.id, &resolved).then_some(resolved)
}

pub fn apply_control(
    runtime: &crate::world::server::WorldContentRuntime,
    manager: &mut crate::objectives::ObjectiveManager,
    id: &str,
    verb: ObjectiveVerb,
    recipients: &[String],
    live: &[String],
    balance: Option<&mut Messages<crate::core::balance::BalanceEvent>>,
    layers: Option<&mut crate::world::server::WorldLayerMap>,
) -> (GmActionOutcome, Option<GmActionRefusalReason>) {
    let entry = runtime
        .gm_objective_palette
        .iter()
        .find(|entry| entry.id == id);
    let scope = if verb == ObjectiveVerb::Activate {
        let Some(entry) = entry else {
            return (
                GmActionOutcome::Refused,
                Some(GmActionRefusalReason::UnknownObjective),
            );
        };
        let Some(scope) = resolve_recipients(entry, runtime, live) else {
            return (
                GmActionOutcome::Refused,
                Some(GmActionRefusalReason::ObjectiveScopeMismatch),
            );
        };
        scope
    } else {
        let Some(scope) = manager.recipients(id) else {
            return (
                GmActionOutcome::Refused,
                Some(GmActionRefusalReason::UnknownObjective),
            );
        };
        scope.to_vec()
    };
    match validate(verb, manager.status(id), &scope, recipients, live) {
        Err(reason) => return (GmActionOutcome::Refused, Some(reason)),
        Ok(GmActionOutcome::NoOp) => return (GmActionOutcome::NoOp, None),
        _ => {}
    }
    let command = match verb {
        ObjectiveVerb::Activate => {
            let entry = entry.expect("activation validated palette");
            let crate::world::config::TriggerAction::AddObjective {
                id,
                text,
                text_params,
                mandatory,
                targets,
                directive,
                utility,
                source,
                command_stance,
            } = &entry.action
            else {
                unreachable!("palette parser only accepts Objectives")
            };
            ActionCmd::AddObjective {
                id: id.clone(),
                text: text.clone(),
                text_params: text_params.clone(),
                mandatory: *mandatory,
                targets: targets.clone(),
                directive: directive.clone(),
                utility: utility.clone(),
                source: source.clone(),
                command_stance: command_stance.clone(),
                origin_layer: entry.origin_layer.clone(),
            }
        }
        ObjectiveVerb::Complete => ActionCmd::CompleteObjective { id: id.to_string() },
        ObjectiveVerb::Fail => ActionCmd::FailObjective { id: id.to_string() },
    };
    let changed = apply_command(manager, command, balance, layers);
    if changed && verb == ObjectiveVerb::Activate {
        manager.set_recipients(id, scope);
    }
    (
        if changed {
            GmActionOutcome::Applied
        } else {
            GmActionOutcome::NoOp
        },
        None,
    )
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ObjectiveRow {
    pub id: String,
    pub label: String,
    pub text: String,
    pub text_params: std::collections::BTreeMap<String, String>,
    pub recipients: Vec<String>,
    pub status: Option<ObjectiveStatus>,
    pub available: bool,
}

pub fn rows(
    runtime: Option<&crate::world::server::WorldContentRuntime>,
    manager: Option<&crate::objectives::ObjectiveManager>,
    live: &[String],
) -> (Vec<ObjectiveRow>, Vec<ObjectiveRow>) {
    let objectives = manager
        .map(|manager| {
            manager
                .sorted_snapshots()
                .into_iter()
                .map(|snapshot| {
                    let recipients = manager
                        .recipients(&snapshot.id)
                        .unwrap_or_default()
                        .to_vec();
                    ObjectiveRow {
                        available: valid_request_vocabulary(&snapshot.id, &recipients)
                            && recipients.iter().all(|id| live.contains(id)),
                        recipients,
                        label: snapshot.text.clone(),
                        id: snapshot.id,
                        text: snapshot.text,
                        text_params: snapshot.text_params,
                        status: Some(snapshot.status),
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    let palette = runtime
        .map(|runtime| {
            runtime
                .gm_objective_palette
                .iter()
                .map(|entry| {
                    let resolved = resolve_recipients(entry, runtime, live);
                    let crate::world::config::TriggerAction::AddObjective {
                        text, text_params, ..
                    } = &entry.action
                    else {
                        unreachable!()
                    };
                    ObjectiveRow {
                        id: entry.id.clone(),
                        label: entry.label.clone(),
                        text: text.clone(),
                        text_params: text_params.clone(),
                        available: resolved.is_some(),
                        recipients: resolved.unwrap_or_else(|| entry.recipients.clone()),
                        status: manager
                            .and_then(|manager| manager.status(&entry.id))
                            .cloned(),
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    (palette, objectives)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(id: String, recipients: Vec<String>) -> RawObjectivePaletteEntry {
        RawObjectivePaletteEntry {
            id,
            label: "objective.test".into(),
            recipients,
            fields: toml::toml! { text = "objective.test" },
        }
    }

    #[test]
    fn palette_authoring_obeys_the_same_byte_and_scope_bounds_as_actions() {
        for id in ["x".repeat(128), "é".repeat(64)] {
            let recipients = (0..32).map(|n| format!("ship-{n:02}")).collect();
            let entries = parse_palette(&[raw(id.clone(), recipients)]).unwrap();
            assert!(valid_request_vocabulary(&id, &entries[0].recipients));
        }
        for id in [
            String::new(),
            "x".repeat(129),
            "é".repeat(65),
            "bad\nid".into(),
        ] {
            assert!(parse_palette(&[raw(id, vec![])]).is_err());
        }
        for recipients in [
            (0..33).map(|n| format!("ship-{n}")).collect(),
            vec!["ship".into(); 33],
            vec![String::new()],
            vec!["x".repeat(129)],
            vec!["bad\rship".into()],
        ] {
            assert!(parse_palette(&[raw("objective".into(), recipients)]).is_err());
        }
        let entries = parse_palette(&[raw("objective".into(), vec!["x".repeat(128)])]).unwrap();
        assert!(valid_request_vocabulary(
            &entries[0].id,
            &entries[0].recipients
        ));
    }

    #[test]
    fn resolved_aliases_and_retained_records_never_advertise_impossible_requests() {
        let mut runtime = crate::world::server::WorldContentRuntime {
            gm_objective_palette: parse_palette(&[raw("objective".into(), vec!["alias".into()])])
                .unwrap(),
            ..Default::default()
        };
        for invalid in ["x".repeat(129), "bad\nship".into()] {
            runtime.name_to_uuid.insert("alias".into(), invalid.clone());
            let (palette, _) = rows(Some(&runtime), None, &[invalid]);
            assert!(!palette[0].available);
            assert_eq!(palette[0].recipients, ["alias"]);
        }
        runtime.name_to_uuid.insert("alias".into(), "ship-a".into());
        assert!(!rows(Some(&runtime), None, &[]).0[0].available);
        let valid = rows(Some(&runtime), None, &["ship-a".into()]).0.remove(0);
        assert!(valid.available);
        assert!(valid_request_vocabulary(&valid.id, &valid.recipients));

        let mut manager = crate::objectives::ObjectiveManager::default();
        let oversized_id = "x".repeat(129);
        manager.add(&oversized_id, "objective.test", true, vec![]);
        manager.add("too-many-ships", "objective.test", true, vec![]);
        let live: Vec<String> = (0..33).map(|n| format!("ship-{n:02}")).collect();
        manager.set_recipients("too-many-ships", live.clone());
        assert!(rows(None, Some(&manager), &live)
            .1
            .iter()
            .all(|row| !row.available));
    }
}
