//! The scenario/ship catalogue payload — ONE field list, two consumers.
//!
//! The browser host publishes the pre-load catalogue to `server.html` as a JS
//! array (`bridge::wasm_get_scenario_catalog`); the native host (issue #855)
//! publishes the same catalogue to a browser over HTTP as JSON. PRD #855's
//! load-bearing decision is that "native/browser hosts consume the same content
//! manifest, snapshots, and protocol contracts" — which is only true if there
//! is exactly one place that says what a catalogue entry's fields are called.
//!
//! That place is this module. [`ScenarioPayload`] and [`ShipPayload`] hold an
//! *ordered* `(key, value)` list; the wasm bridge walks it with `Reflect::set`
//! and `core::codec::encode_delivery_manifest` walks it into a JSON object.
//! Neither carries a key name of its own, so a field added to one surface
//! cannot skip the other — the drift this module exists to prevent.
//!
//! Bevy-free and target-free: the only host dependency is
//! `config_cache::get_cached_entity_config`, which has the identical signature
//! on both targets (the browser fills it by fetch, the native host by
//! `insert_native_config`), so this is genuinely the same code on both.

use crate::world::config::AvailableShipEntry;
use crate::world::manifest::{ScenarioCatalog, ScenarioCatalogEntry};

/// The key under which a scenario's hull list is nested. Both encoders read it
/// from here rather than spelling it out, for the same reason the scalar keys
/// live in [`ScenarioPayload::entries`].
pub const SHIPS_KEY: &str = "ships";

/// The literal `origin` value a base-manifest scenario is published under
/// (issue #990): `ScenarioCatalogEntry::origin` is `None` for base content, and
/// both surfaces flatten that to a present string so a client can badge a
/// mod-supplied scenario without a second lookup.
pub const BASE_ORIGIN: &str = "base";

/// One published field value. Deliberately only the two shapes the catalogue
/// actually uses — a string or a number — so both encoders stay total.
#[derive(Clone, Debug, PartialEq)]
pub enum PayloadValue {
    Text(String),
    Number(f64),
}

impl PayloadValue {
    /// The text of a `Text` value, or `None` for a number. Used by tests and by
    /// the HTTP layer; the encoders match on the enum directly.
    pub fn as_text(&self) -> Option<&str> {
        match self {
            PayloadValue::Text(s) => Some(s),
            PayloadValue::Number(_) => None,
        }
    }
}

/// One publishable hull: the ordered fields both surfaces emit for an
/// `[[available_ships]]` entry.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct ShipPayload {
    entries: Vec<(&'static str, PayloadValue)>,
}

impl ShipPayload {
    /// The ordered `(key, value)` fields to publish. The ONLY list of hull
    /// field names in the repository.
    pub fn entries(&self) -> &[(&'static str, PayloadValue)] {
        &self.entries
    }

    /// Look a published field up by key — for assertions and for the catalogue
    /// restriction check, never for encoding.
    pub fn get(&self, key: &str) -> Option<&PayloadValue> {
        self.entries.iter().find(|(k, _)| *k == key).map(|(_, v)| v)
    }

    fn push_text(&mut self, key: &'static str, value: impl Into<String>) {
        self.entries.push((key, PayloadValue::Text(value.into())));
    }
}

/// One publishable scenario: ordered scalar fields plus the nested hull list.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct ScenarioPayload {
    entries: Vec<(&'static str, PayloadValue)>,
    ships: Vec<ShipPayload>,
}

impl ScenarioPayload {
    /// The ordered scalar `(key, value)` fields. The nested hull list is
    /// [`ScenarioPayload::ships`], published under [`SHIPS_KEY`].
    pub fn entries(&self) -> &[(&'static str, PayloadValue)] {
        &self.entries
    }

    /// The hulls this scenario offers, already curated by the manifest.
    pub fn ships(&self) -> &[ShipPayload] {
        &self.ships
    }

    /// Look a published scalar field up by key.
    pub fn get(&self, key: &str) -> Option<&PayloadValue> {
        self.entries.iter().find(|(k, _)| *k == key).map(|(_, v)| v)
    }

    fn push_text(&mut self, key: &'static str, value: impl Into<String>) {
        self.entries.push((key, PayloadValue::Text(value.into())));
    }
}

/// Build the publishable form of one hull entry.
///
/// `template_path` and `label` come from the world's own `[[available_ships]]`
/// entry; `class`, `hull_id`, `power_rating` and `name` are enrichment read
/// from the cached entity template and are simply absent when the template has
/// not been delivered yet (the browser fetches asynchronously) or does not
/// declare them.
pub fn ship_payload(ship: &AvailableShipEntry) -> ShipPayload {
    let mut out = ShipPayload::default();
    out.push_text("template_path", ship.template_path.clone());
    out.push_text(
        "label",
        ship.label.as_deref().unwrap_or(&ship.template_path),
    );
    if let Some(cfg) = crate::config_cache::get_cached_entity_config(&ship.template_path) {
        if let Some(ref class) = cfg.class {
            out.push_text("class", class.clone());
        }
        if let Some(ref hull_id) = cfg.hull_id {
            out.push_text("hull_id", hull_id.clone());
        }
        if let Some(rating) = cfg.power_rating {
            out.entries
                .push(("power_rating", PayloadValue::Number(rating as f64)));
        }
        // The picker card's TITLE is the ship's crew-facing proper name when the
        // hull authors one (e.g. "AEV Phoenix"), falling back to the identity
        // `name` otherwise (player-facing ship names). The class rides its own
        // `class` key above and becomes the card's subtitle badge, so the card
        // reads NAME + CLASS rather than a bare class or an "Unknown" subtitle.
        if let Some(name) = cfg.display_name.as_ref().or(cfg.name.as_ref()) {
            out.push_text("name", name.clone());
        }
    }
    out
}

/// Build the publishable form of one catalogue scenario.
pub fn scenario_payload(entry: &ScenarioCatalogEntry) -> ScenarioPayload {
    let mut out = ScenarioPayload::default();
    out.push_text("id", entry.id.clone());
    out.push_text("world", entry.world.clone());
    if let Some(ref label) = entry.label {
        out.push_text("label", label.clone());
    }
    if let Some(ref description) = entry.description {
        out.push_text("description", description.clone());
    }
    out.push_text(
        "source",
        entry
            .origin
            .clone()
            .unwrap_or_else(|| BASE_ORIGIN.to_string()),
    );
    out.ships = entry.ships.iter().map(ship_payload).collect();
    out
}

/// Build the publishable form of a whole catalogue, in manifest order.
pub fn catalog_payload(catalog: &ScenarioCatalog) -> Vec<ScenarioPayload> {
    catalog.scenarios.iter().map(scenario_payload).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::manifest::ScenarioCatalogEntry;

    fn ship(path: &str, label: Option<&str>) -> AvailableShipEntry {
        AvailableShipEntry {
            template_path: path.to_string(),
            label: label.map(str::to_string),
        }
    }

    #[test]
    fn a_hull_publishes_its_path_and_label_in_that_order() {
        let p = ship_payload(&ship(
            "assets/entities/alliance_destroyer.toml",
            Some("Sabre"),
        ));
        let keys: Vec<&str> = p.entries().iter().map(|(k, _)| *k).collect();
        assert_eq!(keys, vec!["template_path", "label"]);
        assert_eq!(
            p.get("template_path").and_then(PayloadValue::as_text),
            Some("assets/entities/alliance_destroyer.toml")
        );
        assert_eq!(
            p.get("label").and_then(PayloadValue::as_text),
            Some("Sabre")
        );
    }

    #[test]
    fn a_hull_without_a_label_falls_back_to_its_template_path() {
        let p = ship_payload(&ship("assets/entities/alliance_cruiser.toml", None));
        assert_eq!(
            p.get("label").and_then(PayloadValue::as_text),
            Some("assets/entities/alliance_cruiser.toml")
        );
    }

    #[test]
    fn a_base_scenario_publishes_source_base_rather_than_omitting_it() {
        let entry = ScenarioCatalogEntry {
            id: "combat_test".into(),
            world: "assets/worlds/combat_test.toml".into(),
            label: Some("Combat Test".into()),
            description: Some("A skirmish.".into()),
            ships: vec![ship("assets/entities/alliance_destroyer.toml", None)],
            origin: None,
        };
        let p = scenario_payload(&entry);
        assert_eq!(
            p.get("source").and_then(PayloadValue::as_text),
            Some(BASE_ORIGIN)
        );
        let keys: Vec<&str> = p.entries().iter().map(|(k, _)| *k).collect();
        assert_eq!(keys, vec!["id", "world", "label", "description", "source"]);
        assert_eq!(p.ships().len(), 1);
    }

    #[test]
    fn a_mod_scenario_publishes_its_pack_id_as_the_source() {
        let entry = ScenarioCatalogEntry {
            id: "extra".into(),
            world: "packs/extra.toml".into(),
            label: None,
            description: None,
            ships: vec![],
            origin: Some("my-pack".into()),
        };
        let p = scenario_payload(&entry);
        assert_eq!(
            p.get("source").and_then(PayloadValue::as_text),
            Some("my-pack")
        );
        // Absent label/description are omitted, not published as empty strings —
        // the browser surface has always omitted them and a JSON `""` would read
        // as an authored blank title.
        let keys: Vec<&str> = p.entries().iter().map(|(k, _)| *k).collect();
        assert_eq!(keys, vec!["id", "world", "source"]);
    }

    #[test]
    fn a_catalog_publishes_its_scenarios_in_manifest_order() {
        let catalog = ScenarioCatalog {
            scenarios: vec![
                ScenarioCatalogEntry {
                    id: "first".into(),
                    world: "a.toml".into(),
                    label: None,
                    description: None,
                    ships: vec![],
                    origin: None,
                },
                ScenarioCatalogEntry {
                    id: "second".into(),
                    world: "b.toml".into(),
                    label: None,
                    description: None,
                    ships: vec![],
                    origin: None,
                },
            ],
        };
        let ids: Vec<String> = catalog_payload(&catalog)
            .iter()
            .map(|s| {
                s.get("id")
                    .and_then(PayloadValue::as_text)
                    .unwrap()
                    .to_string()
            })
            .collect();
        assert_eq!(ids, vec!["first", "second"]);
    }
}
