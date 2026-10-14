//! Authored ship interiors and permitted physical team positions.
use crate::messages::SystemId;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct InteriorConfig {
    #[serde(rename = "deck", default)]
    pub decks: Vec<DeckConfig>,
    #[serde(rename = "compartment", default)]
    pub compartments: Vec<CompartmentConfig>,
    #[serde(rename = "connection", default)]
    pub connections: Vec<ConnectionConfig>,
    #[serde(default)]
    pub placements: BTreeMap<SystemId, String>,
    #[serde(default)]
    pub team_homes: Vec<String>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeckConfig {
    pub id: String,
    pub label: String,
    pub damage_weight: f32,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompartmentConfig {
    pub id: String,
    pub deck: String,
    pub label: String,
    pub x: f32,
    pub z: f32,
    pub width: f32,
    pub depth: f32,
    pub material: String,
    pub max_hp: f32,
    pub recovery_fraction: f32,
    pub structural_remedy: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionConfig {
    pub id: String,
    pub from: String,
    pub to: String,
    pub travel_secs: f32,
    pub boundary: Option<SystemId>,
}

/// Idle positions survive completion. A crossing commits its endpoint before
/// a redirect replans; externally committed teams cannot become internally idle.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TeamLocation {
    Compartment {
        compartment: String,
    },
    Crossing {
        connection: String,
        from: String,
        to: String,
        elapsed: f32,
    },
    External {
        target: String,
    },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TeamNavigation {
    pub location: TeamLocation,
    pub destination: Option<String>,
    pub route: Vec<String>,
    pub blocked: bool,
}

/// Generic route values retain the existing team continuation shape.
pub type RouteCursor = TeamNavigation;
pub type RouteLocation = TeamLocation;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BoundaryPhase {
    Open,
    Sealing,
    Sealed,
    Opening,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoundaryStatus {
    pub system: SystemId,
    pub seal_requested: bool,
    pub actuator_sealed: bool,
    pub phase: BoundaryPhase,
    pub elapsed: f32,
}
