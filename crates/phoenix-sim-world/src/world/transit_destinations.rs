//! Closed authored World destinations. Loading and physical travel keep their owners.
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum WorldTransitTarget {
    Base { anchor: String },
    Layer { path: String, anchor: String },
}
impl WorldTransitTarget {
    pub fn anchor(&self) -> &str {
        match self {
            Self::Base { anchor } | Self::Layer { anchor, .. } => anchor,
        }
    }
    pub fn layer_path(&self) -> Option<&str> {
        match self {
            Self::Base { .. } => None,
            Self::Layer { path, .. } => Some(path),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorldTransitDestination {
    pub id: String,
    /// Existing strings.csv key, never a path or an arbitrary command.
    pub label: String,
    pub target: WorldTransitTarget,
}
fn identity(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 512 && !value.chars().any(char::is_control)
}
pub fn validate_destinations(rows: &[WorldTransitDestination]) -> Result<(), String> {
    if rows.len() > 128 {
        return Err("transit_destination exceeds the technical 128-route catalogue bound".into());
    }
    let mut ids = BTreeSet::new();
    for row in rows {
        if !identity(&row.id)
            || !identity(&row.label)
            || !identity(row.target.anchor())
            || !ids.insert(&row.id)
        {
            return Err(
                "transit_destination needs unique nonempty bounded id, label and arrival anchor"
                    .into(),
            );
        }
        if let Some(path) = row.target.layer_path() {
            validate_layer_path(path)?;
        }
    }
    Ok(())
}

pub fn validate_layer_path(path: &str) -> Result<(), String> {
    if path.len() > 512
        || !path.starts_with("assets/worlds/")
        || !path.ends_with(".toml")
        || path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
        || path.contains('\\')
        || path.chars().any(char::is_control)
    {
        return Err(
            "transit_destination layer path must be a canonical accepted assets/worlds/*.toml path"
                .into(),
        );
    }
    Ok(())
}
