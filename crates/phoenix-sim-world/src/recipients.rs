use crate::objective_instances::{
    ObjectiveInstanceKey, ObjectiveInstanceManager, PlayerShipMembership, RecipientSelector,
};
use std::collections::BTreeSet;
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
    pub fn from_rhai_map(spec: &rhai::Map) -> Result<Option<Self>, String> {
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
