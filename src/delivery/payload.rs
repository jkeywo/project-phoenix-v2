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
        source: entry
            .origin
            .clone()
            .unwrap_or_else(crate::core::messages::base_scenario_source),
    }
}

pub fn catalog_payload(catalog: &ScenarioCatalog) -> Vec<ScenarioPayload> {
    catalog.scenarios.iter().map(scenario_payload).collect()
}

impl From<&ActivePack> for ActivePackWire {
    fn from(pack: &ActivePack) -> Self {
        Self {
            id: pack.id.clone(),
            name: pack.name.clone(),
            version: pack.version.clone(),
        }
    }
}

/// Assemble the full replacement snapshot. Locks are supplied by the host's
/// arbiter (after pinned-hull precedence); presentation never arbitrates.
pub fn catalogue_snapshot(
    scenarios: Vec<ScenarioPayload>,
    active: &[ActivePack],
    locked_scenario: Option<String>,
    locked_ship: Option<String>,
) -> ScenarioCatalogPayload {
    ScenarioCatalogPayload {
        scenarios,
        locked_scenario,
        locked_ship,
        active_packs: active.iter().map(ActivePackWire::from).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ship(path: &str, label: Option<&str>) -> AvailableShipEntry {
        AvailableShipEntry {
            template_path: path.into(),
            label: label.map(str::to_string),
        }
    }

    #[test]
    fn hull_label_fallback_and_optional_enrichment_are_preserved() {
        let path = "assets/entities/__catalogue/missing.toml";
        let payload = ship_payload(&ship(path, None));
        assert_eq!(payload.template_path, path);
        assert_eq!(payload.label.as_deref(), Some(path));
        assert_eq!(payload.mass, None);
        assert_eq!(
            ship_payload(&ship(path, Some("Sabre"))).label.as_deref(),
            Some("Sabre")
        );
    }

    #[test]
    fn catalogue_only_templates_supply_the_hull_cards_details() {
        use crate::entities::config_cache as cache;
        cache::clear_catalog_templates();
        let path = "assets/entities/__payload_enrich/destroyer.toml";
        cache::push_catalog_template(
            path.into(),
            r#"class = "destroyer"
hull_id = "AEV-0741"
mass = 14000.0
power_rating = 70
name = "AEV Phoenix"
"#
            .into(),
            true,
        );
        let p = ship_payload(&ship(path, Some("Destroyer")));
        assert_eq!(p.class.as_deref(), Some("destroyer"));
        assert_eq!(p.hull_id.as_deref(), Some("AEV-0741"));
        assert_eq!(p.mass, Some(14000.0));
        assert_eq!(p.power_rating, Some(70.0));
        assert_eq!(p.name.as_deref(), Some("AEV Phoenix"));
        cache::clear_catalog_templates();
        assert_eq!(ship_payload(&ship(path, None)).mass, None);
    }

    #[test]
    fn projection_preserves_provenance_manifest_order_and_curated_hulls() {
        let catalog = ScenarioCatalog {
            scenarios: [("base", None), ("mod", Some("pack-b"))]
                .into_iter()
                .map(|(id, origin)| ScenarioCatalogEntry {
                    id: id.into(),
                    world: format!("{id}.toml"),
                    label: None,
                    description: None,
                    ships: vec![ship("curated.toml", Some("Curated"))],
                    origin: origin.map(str::to_string),
                })
                .collect(),
        };
        let payload = catalog_payload(&catalog);
        assert_eq!(
            payload.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
            ["base", "mod"]
        );
        assert_eq!(payload[0].source, "base");
        assert_eq!(payload[1].source, "pack-b");
        assert_eq!(payload[0].ships.len(), 1);
        assert_eq!(payload[0].ships[0].template_path, "curated.toml");
    }
}
