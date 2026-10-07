//! Typed catalogue projection shared by delivery, both hosts and the picker.
//!
//! Field names and defaults live in the serde wire types. Adapters serialize
//! those types instead of maintaining Reflect loops or JavaScript inventories.
//! Projection preserves manifest order, hull curation and overlay load order.

use crate::core::messages::{ActivePackWire, ScenarioCatalogPayload};
pub use crate::core::messages::{
    CatalogShipWire as ShipPayload, ScenarioCatalogWire as ScenarioPayload,
};
use crate::entities::config_cache::ActivePack;
use crate::world::config::AvailableShipEntry;
use crate::world::manifest::{ScenarioCatalog, ScenarioCatalogEntry};

/// Enrich an authored hull from the display-only catalogue cache, or the
/// runtime cache after world load. Missing templates leave enrichment absent.
pub fn ship_payload(ship: &AvailableShipEntry) -> ShipPayload {
    let mut out = ShipPayload {
        template_path: ship.template_path.clone(),
        label: Some(
            ship.label
                .clone()
                .unwrap_or_else(|| ship.template_path.clone()),
        ),
        ..Default::default()
    };
    if let Some(cfg) = crate::entities::config_cache::catalog_entity_config(&ship.template_path) {
        out.class = cfg.class.clone();
        out.hull_id = cfg.hull_id.clone();
        out.mass = Some(cfg.mass as f64);
        out.power_rating = cfg.power_rating.map(|rating| rating as f64);
        out.name = cfg.display_name.as_ref().or(cfg.name.as_ref()).cloned();
    }
    out
}

pub fn scenario_payload(entry: &ScenarioCatalogEntry) -> ScenarioPayload {
    ScenarioPayload {
        id: entry.id.clone(),
        world: entry.world.clone(),
        label: entry.label.clone(),
        description: entry.description.clone(),
        ships: entry.ships.iter().map(ship_payload).collect(),
        slots: entry
            .slots
            .iter()
            .map(|slot| crate::core::messages::CatalogShipSlotWire {
                id: slot.id.clone(),
                label: slot.label.clone(),
                ships: slot.ships.iter().map(ship_payload).collect(),
                default_ship: slot.default_ship.clone(),
                unclaimed: slot.unclaimed,
            })
            .collect(),
        source: entry
            .origin
            .clone()
            .unwrap_or_else(crate::core::messages::base_scenario_source),
    }
}

pub fn catalog_payload(catalog: &ScenarioCatalog) -> Vec<ScenarioPayload> {
    catalog.scenarios.iter().map(scenario_payload).collect()
}

/// Assemble the full replacement snapshot. Locks are supplied by the host's
/// arbiter (after pinned-hull precedence); presentation never arbitrates.
pub fn catalogue_snapshot(
    scenarios: Vec<ScenarioPayload>,
    active: &[ActivePack],
    locked_scenario: Option<String>,
    locked_slot: Option<String>,
    locked_ship: Option<String>,
) -> ScenarioCatalogPayload {
    ScenarioCatalogPayload {
        scenarios,
        locked_scenario,
        locked_slot,
        locked_ship,
        active_packs: active.iter().map(ActivePackWire::from).collect(),
    }
}

#[cfg(test)]
#[path = "payload_tests.rs"]
mod tests;
