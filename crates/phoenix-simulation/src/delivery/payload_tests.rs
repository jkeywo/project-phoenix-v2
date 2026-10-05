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
                slots: Vec::new(),
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
