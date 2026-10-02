use super::*;
use crate::world::config::GmPaletteVariant;

fn entry(id: &str) -> GmPaletteEntry {
    GmPaletteEntry {
        id: id.to_string(),
        label: format!("world.test.gm_palette.{id}.label"),
        template_path: format!("assets/entities/{id}.toml"),
        name_prefix: None,
        groups: vec!["hostiles".to_string()],
        variants: Vec::new(),
    }
}

#[test]
fn a_placement_outside_the_coordinate_bound_is_not_a_placement() {
    assert!(placement_is_valid([10_000, 0, -20_000], 45_000));
    assert!(!placement_is_valid([MAX_GM_SPAWN_COORD_MM + 1, 0, 0], 0));
    assert!(!placement_is_valid(
        [0, 0, 0],
        MAX_GM_SPAWN_HEADING_MDEG + 1
    ));
    assert!(!placement_is_valid(
        [0, 0, 0],
        -MAX_GM_SPAWN_HEADING_MDEG - 1
    ));
}

#[test]
fn fixed_point_placement_converts_to_world_space_exactly() {
    assert_eq!(
        placement_metres([120_500, 0, -40_250]),
        [120.5, 0.0, -40.25]
    );
    assert_eq!(heading_degrees(90_000), 90.0);
    assert_eq!(heading_degrees(-1_500), -1.5);
}

#[test]
fn a_palette_id_resolves_by_authored_order_and_nothing_else() {
    let entries = vec![entry("raider"), entry("tender")];
    assert_eq!(
        palette_entry(&entries, "tender").map(|e| e.id.as_str()),
        Some("tender")
    );
    assert!(palette_entry(&entries, "assets/entities/raider.toml").is_none());
    assert!(palette_entry(&entries, "").is_none());
}

#[test]
fn the_spawn_action_is_the_ordinary_scenario_one() {
    let mut raider = entry("raider");
    raider.name_prefix = Some("gm_raider".to_string());
    raider.variants.push(GmPaletteVariant {
        id: "blood_eagle".to_string(),
        label: "world.test.gm_palette.raider.blood_eagle.label".to_string(),
        overrides: Some(toml::Value::Table(toml::map::Map::new())),
    });
    let pending = PendingGmSpawn {
        palette: "raider".to_string(),
        variant: Some("blood_eagle".to_string()),
        name: PendingGmSpawn::derive_name(&raider, 7),
        position_mm: [120_000, 0, -40_000],
        heading_mdeg: 90_000,
    };
    assert_eq!(pending.name, "gm_raider_7");
    let TriggerAction::SpawnEntity {
        template_path,
        name,
        anchor,
        position,
        rotation,
        groups,
        overrides,
        ..
    } = spawn_action(&pending, &raider)
    else {
        panic!("a GM placement is an ordinary scenario spawn");
    };
    assert_eq!(template_path, "assets/entities/raider.toml");
    assert_eq!(name, "gm_raider_7");
    assert_eq!(anchor, None, "a GM placement carries resolved coordinates");
    assert_eq!(position, Some([120.0, 0.0, -40.0]));
    let rotation = rotation.expect("a placed heading is an authored rotation");
    assert!((rotation[1] + std::f32::consts::FRAC_PI_2).abs() < 1e-5);
    assert_eq!(groups, vec!["hostiles".to_string()]);
    assert!(
        overrides.is_some(),
        "the chosen variant's overrides ride along"
    );
}

#[test]
fn an_unknown_variant_id_contributes_no_overrides() {
    let raider = entry("raider");
    let pending = PendingGmSpawn {
        palette: "raider".to_string(),
        variant: Some("not-authored".to_string()),
        name: "raider_1".to_string(),
        position_mm: [0, 0, 0],
        heading_mdeg: 0,
    };
    let TriggerAction::SpawnEntity { overrides, .. } = spawn_action(&pending, &raider) else {
        panic!("spawn action");
    };
    assert!(overrides.is_none());
}

#[test]
fn the_projected_palette_never_carries_a_template_path() {
    let mut raider = entry("raider");
    raider.variants.push(GmPaletteVariant {
        id: "blood_eagle".to_string(),
        label: "world.test.gm_palette.raider.blood_eagle.label".to_string(),
        overrides: None,
    });
    let options = palette_options(std::slice::from_ref(&raider));
    assert_eq!(options.len(), 1);
    assert_eq!(options[0].id, "raider");
    assert_eq!(options[0].label, "world.test.gm_palette.raider.label");
    assert_eq!(options[0].variants.len(), 1);
    assert_eq!(options[0].variants[0].id, "blood_eagle");
    // Exhaustive destructuring rather than a string search: this fails to
    // COMPILE if a future field (a template path, an override document)
    // joins the projected row, which is the guarantee worth having.
    let GmPaletteOption {
        id: _,
        label: _,
        variants,
    } = &options[0];
    let GmPaletteVariantOption { id: _, label: _ } = &variants[0];
}
