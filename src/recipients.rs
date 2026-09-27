//! Shared action/Comms recipient vocabulary (issue #1533).
//!
//! An absent selection retains a caller's legacy behaviour. An explicit empty
//! selection is a valid empty set. Callers must never turn either a resolution
//! error or an empty result into an unaddressed action.

use std::collections::BTreeSet;

use crate::objective_instances::{
    ObjectiveInstanceKey, ObjectiveInstanceManager, PlayerShipMembership, RecipientSelector,
};

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RecipientDiagnostic {
    pub tick: u64,
    pub source: Option<String>,
    pub line: Option<usize>,
    pub action: String,
    pub message: String,
}

/// Bounded observation only: never snapshotted, replayed, or used by gameplay.
#[derive(bevy::prelude::Resource, Default)]
pub struct RecipientDiagnostics(pub std::collections::VecDeque<RecipientDiagnostic>);

pub(crate) fn report(world: &mut bevy::prelude::World, diagnostic: RecipientDiagnostic) {
    if let Some(mut trace) = world.get_resource_mut::<crate::workshop::test_trace::TestTrace>() {
        trace.push(
            diagnostic.tick,
            diagnostic.source.as_deref(),
            diagnostic.line,
            crate::workshop::test_protocol::TestTraceKind::RecipientDiagnostic {
                action: diagnostic.action.clone(),
                message: diagnostic.message.clone(),
            },
        );
    }
    let mut diagnostics = world.get_resource_or_insert_with(RecipientDiagnostics::default);
    // Authoring feedback must remain bounded even for a repeat trigger that
    // continues selecting an absent slot indefinitely.
    const CAPACITY: usize = 64;
    if diagnostics.0.len() == CAPACITY {
        diagnostics.0.pop_front();
    }
    diagnostics.0.push_back(diagnostic);
}

pub(crate) fn queue_report(
    commands: &mut bevy::prelude::Commands,
    tick: u64,
    source: &str,
    line: Option<usize>,
    action: &str,
    message: String,
) {
    let diagnostic = RecipientDiagnostic {
        tick,
        source: Some(source.into()),
        line,
        action: action.into(),
        message,
    };
    commands.queue(move |world: &mut bevy::prelude::World| report(world, diagnostic));
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RecipientSelection {
    #[serde(default)]
    pub selectors: Vec<RecipientSelector>,
    #[serde(default)]
    pub objective_instances: Vec<ObjectiveInstanceKey>,
}

/// Only actions with one receiving ship can be addressed. World-global actions
/// must never run once per selected ship.
pub fn retarget_action(
    action: &crate::world::config::TriggerAction,
    recipient: &str,
) -> Result<crate::world::config::TriggerAction, String> {
    use crate::world::config::TriggerAction;
    let mut action = action.clone();
    let destination = match &mut action {
        TriggerAction::Presentation { ship, .. }
        | TriggerAction::SetContactInformation { ship, .. } => ship,
        TriggerAction::SetNpcDoctrine { entity, .. }
        | TriggerAction::SetAiState { entity, .. }
        | TriggerAction::ApplyModifier { entity, .. }
        | TriggerAction::RemoveModifier { entity, .. }
        | TriggerAction::ApplyFlag { entity, .. }
        | TriggerAction::RemoveFlag { entity, .. }
        | TriggerAction::ApplyIntModifier { entity, .. }
        | TriggerAction::RemoveIntModifier { entity, .. }
        | TriggerAction::DestroyEntity { entity } => entity,
        _ => return Err("this action has no single receiving ship and cannot be addressed".into()),
    };
    *destination = recipient.to_string();
    Ok(action)
}

/// Authored names are separate from the live fleet: an absent slot or an
/// inactive but declared instance is valid and may resolve to no ships.
/// Built for each resolution from content and Objective records; this is not
/// independently stored simulation state or an injectable authority override.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RecipientCatalog {
    pub ship_slots: BTreeSet<String>,
    pub factions: BTreeSet<String>,
    pub objective_instances: BTreeSet<ObjectiveInstanceKey>,
}

/// Resolve at the deferred application boundary, after earlier Objective
/// commands have landed. Authored declarations augment the live registry.
pub(crate) fn resolve_in_world(
    world: &mut bevy::prelude::World,
    selection: &RecipientSelection,
) -> Result<Vec<String>, RecipientRefusal> {
    let mut fleet = crate::objective_instances::player_ship_memberships(world);
    // Lethal damage retains crew hull identity and Objective history. Delivery
    // excludes those hulls without changing their assignment or frozen view.
    let mut hulls = world.query::<(
        &crate::entities::spawner::EntityUuid,
        &crate::entities::spawner::EntitySystemHull,
    )>();
    let destroyed: BTreeSet<_> = hulls
        .iter(world)
        .filter(|(_, hull)| hull.0.total_current() <= 0.0)
        .map(|(uuid, _)| uuid.0.clone())
        .collect();
    fleet.retain(|ship| !destroyed.contains(&ship.ship_id));
    let mut catalog = RecipientCatalog::default();
    if let Some(script) = world.get_resource::<crate::world::server::WorldScriptRuntime>() {
        catalog
            .objective_instances
            .extend(script.recipient_declarations.iter().cloned());
    }
    if let Some(config) = world.get_resource::<crate::world::config::WorldConfig>() {
        catalog.ship_slots.extend(
            config
                .effective_ship_slots()
                .into_iter()
                .map(|slot| slot.id),
        );
    }
    if let Some(registry) =
        world.get_resource::<crate::entities::config_cache::FactionRegistryResource>()
    {
        catalog
            .factions
            .extend(registry.iter().map(|faction| faction.name.clone()));
    }
    let empty = ObjectiveInstanceManager::default();
    let instances = world
        .get_resource::<crate::world::server::ObjectiveInstanceManagerRes>()
        .map(|manager| &manager.0)
        .unwrap_or(&empty);
    catalog
        .objective_instances
        .extend(instances.records().iter().map(|row| row.spec.key.clone()));
    selection.resolve(&catalog, &fleet, instances)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecipientRefusal {
    UnknownShipSlot(String),
    UnknownFaction(String),
    UnknownObjectiveInstance(ObjectiveInstanceKey),
}

impl std::fmt::Display for RecipientRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownShipSlot(id) => write!(f, "unknown recipient ship slot '{id}'"),
            Self::UnknownFaction(id) => write!(f, "unknown recipient faction '{id}'"),
            Self::UnknownObjectiveInstance(key) => write!(
                f,
                "unknown recipient Objective instance '{}:{}'",
                key.objective_id, key.instance_id
            ),
        }
    }
}

impl RecipientSelection {
    /// The Comms and addressed-action host functions share this strict parser.
    /// A malformed present field must not be mistaken for omitted addressing.
    pub(crate) fn from_rhai_map(spec: &rhai::Map) -> Result<Option<Self>, String> {
        fn strings(spec: &rhai::Map, field: &str) -> Result<Option<Vec<String>>, String> {
            spec.get(field)
                .map(|value| {
                    let values = value
                        .clone()
                        .try_cast::<rhai::Array>()
                        .ok_or_else(|| format!("`{field}` must be an array of strings"))?;
                    values
                        .into_iter()
                        .map(|value| {
                            value
                                .try_cast::<rhai::ImmutableString>()
                                .map(|value| value.to_string())
                                .ok_or_else(|| format!("`{field}` must be an array of strings"))
                        })
                        .collect()
                })
                .transpose()
        }
        let all = spec
            .get("all_player_ships")
            .map(|value| {
                value
                    .clone()
                    .try_cast::<bool>()
                    .ok_or_else(|| "`all_player_ships` must be a boolean".to_string())
            })
            .transpose()?;
        let instances = spec.get("recipient_objective_instances").map(|value| {
            let values = value.clone().try_cast::<rhai::Array>()
                .ok_or_else(|| "`recipient_objective_instances` must be an array of maps".to_string())?;
            values.into_iter().map(|value| {
                let map = value.try_cast::<rhai::Map>()
                    .ok_or_else(|| "each recipient Objective instance must be a map".to_string())?;
                if map.keys().any(|key| !matches!(key.as_str(), "objective_id" | "instance_id")) {
                    return Err("recipient Objective instances accept only objective_id and instance_id".into());
                }
                let field = |name: &str| map.get(name)
                    .and_then(|value| value.clone().try_cast::<rhai::ImmutableString>())
                    .map(|value| value.to_string())
                    .ok_or_else(|| format!("recipient Objective instance requires string `{name}`"));
                Ok(ObjectiveInstanceKey { objective_id: field("objective_id")?, instance_id: field("instance_id")? })
            }).collect::<Result<Vec<_>, String>>()
        }).transpose()?;
        Self::from_fields(
            strings(spec, "recipient_ship_slots")?,
            strings(spec, "recipient_factions")?,
            all,
            instances,
        )
    }

    /// Preserve presence separately from membership. In particular, `[]` and
    /// `all_player_ships: false` explicitly address nobody.
    pub fn from_fields(
        ship_slots: Option<Vec<String>>,
        factions: Option<Vec<String>>,
        all_player_ships: Option<bool>,
        objective_instances: Option<Vec<ObjectiveInstanceKey>>,
    ) -> Result<Option<Self>, String> {
        if ship_slots.is_none()
            && factions.is_none()
            && all_player_ships.is_none()
            && objective_instances.is_none()
        {
            return Ok(None);
        }
        let mut selectors = Vec::new();
        for slot in ship_slots.unwrap_or_default() {
            if slot.trim().is_empty() {
                return Err("recipient ship-slot ids must not be empty".into());
            }
            selectors.push(RecipientSelector::ShipSlot(slot));
        }
        for faction in factions.unwrap_or_default() {
            if faction.trim().is_empty() {
                return Err("recipient faction names must not be empty".into());
            }
            selectors.push(RecipientSelector::Faction(faction));
        }
        if all_player_ships.unwrap_or(false) {
            selectors.push(RecipientSelector::AllPlayerShips);
        }
        let objective_instances = objective_instances.unwrap_or_default();
        if objective_instances
            .iter()
            .any(|key| key.objective_id.trim().is_empty() || key.instance_id.trim().is_empty())
        {
            return Err(
                "recipient Objective instances require non-empty objective_id and instance_id"
                    .into(),
            );
        }
        Ok(Some(Self {
            selectors,
            objective_instances,
        }))
    }

    pub fn validate(&self, catalog: &RecipientCatalog) -> Result<(), RecipientRefusal> {
        for selector in &self.selectors {
            match selector {
                RecipientSelector::ShipSlot(id) if !catalog.ship_slots.contains(id) => {
                    return Err(RecipientRefusal::UnknownShipSlot(id.clone()));
                }
                RecipientSelector::Faction(id) if !catalog.factions.contains(id) => {
                    return Err(RecipientRefusal::UnknownFaction(id.clone()));
                }
                _ => {}
            }
        }
        for key in &self.objective_instances {
            if !catalog.objective_instances.contains(key) {
                return Err(RecipientRefusal::UnknownObjectiveInstance(key.clone()));
            }
        }
        Ok(())
    }

    /// Resolve a union against the fleet and effective instance membership at
    /// execution. Stable UUID order gives every peer the same dispatch order.
    pub fn resolve(
        &self,
        catalog: &RecipientCatalog,
        fleet: &[PlayerShipMembership],
        instances: &ObjectiveInstanceManager,
    ) -> Result<Vec<String>, RecipientRefusal> {
        self.validate(catalog)?;
        let mut selected = BTreeSet::new();
        for ship in fleet {
            if self.selectors.iter().any(|selector| match selector {
                RecipientSelector::ShipSlot(id) => id == &ship.slot_id,
                RecipientSelector::Faction(id) => id == &ship.faction,
                RecipientSelector::AllPlayerShips => true,
            }) {
                selected.insert(ship.ship_id.clone());
            }
        }
        let live: BTreeSet<_> = fleet.iter().map(|ship| ship.ship_id.as_str()).collect();
        for key in &self.objective_instances {
            selected.extend(
                instances
                    .current_members(key)
                    .into_iter()
                    .filter(|id| live.contains(id.as_str())),
            );
        }
        Ok(selected.into_iter().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::objective_instances::ObjectiveInstanceSpec;

    fn fleet() -> Vec<PlayerShipMembership> {
        vec![
            PlayerShipMembership {
                ship_id: "z".into(),
                slot_id: "lead".into(),
                faction: "Alliance".into(),
            },
            PlayerShipMembership {
                ship_id: "a".into(),
                slot_id: "wing".into(),
                faction: "Dynasty".into(),
            },
        ]
    }

    fn catalog() -> RecipientCatalog {
        RecipientCatalog {
            ship_slots: ["lead", "wing", "absent"].map(String::from).into(),
            factions: ["Alliance", "Dynasty"].map(String::from).into(),
            objective_instances: [key()].into(),
        }
    }

    fn key() -> ObjectiveInstanceKey {
        ObjectiveInstanceKey {
            objective_id: "escort".into(),
            instance_id: "pair".into(),
        }
    }

    #[test]
    fn omitted_and_explicit_empty_have_different_meanings() {
        assert_eq!(
            RecipientSelection::from_fields(None, None, None, None).unwrap(),
            None
        );
        let empty = RecipientSelection::from_fields(Some(vec![]), None, None, None)
            .unwrap()
            .unwrap();
        assert!(empty
            .resolve(&catalog(), &fleet(), &Default::default())
            .unwrap()
            .is_empty());
        assert!(
            RecipientSelection::from_fields(None, None, Some(false), None)
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn malformed_present_fields_are_errors_instead_of_broadcast() {
        for source in [
            "#{ all_player_ships: 1 }",
            "#{ recipient_ship_slots: false }",
            "#{ recipient_factions: [1] }",
            "#{ recipient_objective_instances: [#{ objective_id: \"x\" }] }",
            "#{ recipient_objective_instances: [#{ objective_id: \"x\", instance_id: \"y\", typo: true }] }",
        ] {
            let map = crate::world::script::engine::runtime_engine().eval::<rhai::Map>(source).unwrap();
            assert!(RecipientSelection::from_rhai_map(&map).is_err(), "{source}");
        }
    }

    #[test]
    fn union_is_stable_and_invalid_names_never_broaden() {
        let mut selection = RecipientSelection {
            selectors: vec![
                RecipientSelector::AllPlayerShips,
                RecipientSelector::ShipSlot("lead".into()),
            ],
            ..Default::default()
        };
        assert_eq!(
            selection
                .resolve(&catalog(), &fleet(), &Default::default())
                .unwrap(),
            ["a", "z"]
        );
        selection
            .selectors
            .push(RecipientSelector::ShipSlot("typo".into()));
        assert_eq!(
            selection.resolve(&catalog(), &fleet(), &Default::default()),
            Err(RecipientRefusal::UnknownShipSlot("typo".into()))
        );
    }

    #[test]
    fn absent_slot_and_inactive_declared_instance_are_valid_empty() {
        let selection = RecipientSelection {
            selectors: vec![RecipientSelector::ShipSlot("absent".into())],
            objective_instances: vec![key()],
        };
        assert!(selection
            .resolve(&catalog(), &fleet(), &Default::default())
            .unwrap()
            .is_empty());
    }

    #[test]
    fn current_instance_membership_changes_and_survives_restore() {
        let mut manager = ObjectiveInstanceManager::default();
        let mut fleet = fleet();
        manager
            .activate(
                ObjectiveInstanceSpec {
                    key: key(),
                    recipients: vec![RecipientSelector::Faction("Alliance".into())],
                },
                &fleet,
            )
            .unwrap();
        let selection = RecipientSelection {
            objective_instances: vec![key()],
            ..Default::default()
        };
        assert_eq!(
            selection.resolve(&catalog(), &fleet, &manager).unwrap(),
            ["z"]
        );
        fleet[0].faction = "Dynasty".into();
        fleet[1].faction = "Alliance".into();
        manager.reconcile(&fleet).unwrap();
        let restored = serde_json::from_str(&serde_json::to_string(&manager).unwrap()).unwrap();
        assert_eq!(
            selection.resolve(&catalog(), &fleet, &restored).unwrap(),
            ["a"]
        );
    }
}
