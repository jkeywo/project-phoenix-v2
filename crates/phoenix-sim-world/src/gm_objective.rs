use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ObjectiveVerb {
    Activate,
    Complete,
    Fail,
}

/// Explicit control scope. `All` is never inferred from an empty recipient list.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ObjectiveInstanceScope {
    Instance(String),
    All,
}

impl ObjectiveInstanceScope {
    pub fn valid(&self) -> bool {
        match self {
            Self::All => true,
            Self::Instance(id) => {
                !id.is_empty() && id.len() <= 128 && !id.chars().any(char::is_control)
            }
        }
    }
}

pub const MAX_OBJECTIVE_RECIPIENTS: usize = 32;

pub fn valid_request_vocabulary(id: &str, recipients: &[String]) -> bool {
    crate::gm_npc::bounded_id(id)
        && recipients.len() <= MAX_OBJECTIVE_RECIPIENTS
        && recipients.iter().all(|id| crate::gm_npc::bounded_id(id))
        && recipients.windows(2).all(|pair| pair[0] < pair[1])
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

impl ObjectivePaletteEntry {
    pub fn instance_id(&self) -> Option<&str> {
        match &self.action {
            crate::world::config::TriggerAction::AddObjectiveInstance { spec, .. } => {
                Some(&spec.key.instance_id)
            }
            _ => None,
        }
    }
}

pub fn parse_palette(
    raw: &[RawObjectivePaletteEntry],
) -> Result<Vec<ObjectivePaletteEntry>, String> {
    let mut ids = std::collections::BTreeSet::new();
    raw.iter().map(|row| {
        let instance_id = row.fields.get("instance_id").and_then(toml::Value::as_str);
        if row.id.is_empty() || row.label.is_empty() || !ids.insert((&row.id, instance_id)) {
            return Err(format!("invalid or duplicate gm_objective_palette id '{}'", row.id));
        }
        let mut fields = row.fields.clone();
        if fields.contains_key("type") { return Err("Objective palette cannot choose an action type".into()); }
        fields.insert("type".into(), toml::Value::String("add_objective".into()));
        fields.insert("id".into(), toml::Value::String(row.id.clone()));
        let raw_action: crate::world::config::RawActionEntry = toml::Value::Table(fields).try_into().map_err(|e| format!("Objective palette: {e}"))?;
        let action = crate::world::config::parse_action_entry(&raw_action)?;
        if !matches!(&action, crate::world::config::TriggerAction::AddObjective { text, .. }
            | crate::world::config::TriggerAction::AddObjectiveInstance { text, .. } if !text.is_empty()) {
            return Err("Objective palette requires authored text".into());
        }
        if let Some(id) = instance_id {
            if !ObjectiveInstanceScope::Instance(id.to_owned()).valid() || !row.recipients.is_empty() {
                return Err("Objective instance palette uses bounded instance_id and authored recipient selectors, not legacy recipients".into());
            }
        }
        let mut recipients = row.recipients.clone();
        recipients.sort(); recipients.dedup();
        if row.recipients.len() > MAX_OBJECTIVE_RECIPIENTS || !valid_request_vocabulary(&row.id, &recipients) {
            return Err("Objective palette id or recipients exceed the GM action vocabulary".into());
        }
        Ok(ObjectivePaletteEntry { id: row.id.clone(), label: row.label.clone(), recipients, action, origin_layer: None })
    }).collect()
}
