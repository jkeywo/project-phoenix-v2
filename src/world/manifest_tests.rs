use super::*;
use std::collections::HashMap;

const MANIFEST: &str = r#"
[[scenario]]
id = "default"
world = "assets/worlds/default.toml"

[[scenario]]
id = "combat_test"
world = "assets/worlds/combat_test.toml"
"#;

fn world_with_ships() -> String {
    r#"
[global]
title = "world.default.title"
description = "world.default.description"

[[available_ships]]
template_path = "assets/entities/alliance_cruiser.toml"
label = "Cruiser"

[[available_ships]]
template_path = "assets/entities/alliance_destroyer.toml"
"#
    .to_string()
}

fn combat_world() -> String {
    r#"
[global]
title = "world.combat.title"

[[available_ships]]
template_path = "assets/entities/alliance_battleship.toml"
"#
    .to_string()
}

fn resolver(map: HashMap<String, String>) -> impl Fn(&str) -> Option<String> {
    move |path: &str| map.get(path).cloned()
}

fn full_map() -> HashMap<String, String> {
    let mut m = HashMap::new();
    m.insert("assets/worlds/default.toml".to_string(), world_with_ships());
    m.insert("assets/worlds/combat_test.toml".to_string(), combat_world());
    m
}

// -- parse ---------------------------------------------------------------

#[test]
fn parse_manifest_reads_scenario_entries() {
    let m = parse_manifest(MANIFEST).expect("must parse");
    assert_eq!(m.scenarios.len(), 2);
    assert_eq!(m.scenarios[0].id, "default");
    assert_eq!(m.scenarios[0].world, "assets/worlds/default.toml");
    assert_eq!(m.scenarios[1].id, "combat_test");
}

#[test]
fn parse_manifest_reads_optional_label() {
    let toml = r#"
[[scenario]]
id = "x"
world = "assets/worlds/x.toml"
label = "Custom"
"#;
    let m = parse_manifest(toml).expect("must parse");
    assert_eq!(m.scenarios[0].label.as_deref(), Some("Custom"));
}

#[test]
fn parse_manifest_ships_defaults_to_empty() {
    let m = parse_manifest(MANIFEST).expect("must parse");
    assert!(m.scenarios[0].ships.is_empty());
}

#[test]
fn parse_manifest_reads_optional_ships() {
    let toml = r#"
[[scenario]]
id = "x"
world = "assets/worlds/x.toml"
ships = ["assets/entities/alliance_destroyer.toml"]
"#;
    let m = parse_manifest(toml).expect("must parse");
    assert_eq!(
        m.scenarios[0].ships,
        vec!["assets/entities/alliance_destroyer.toml".to_string()]
    );
}

#[test]
fn parse_manifest_empty_is_empty_manifest() {
    let m = parse_manifest("").expect("empty parses");
    assert!(m.scenarios.is_empty());
}

#[test]
fn parse_manifest_rejects_toml_syntax_error() {
    assert!(parse_manifest("nope [").is_err());
}

// -- validation ----------------------------------------------------------

#[test]
fn valid_manifest_produces_no_findings() {
    let m = parse_manifest(MANIFEST).unwrap();
    let findings = validate_manifest(&m, MANIFEST, resolver(full_map()));
    assert!(findings.is_empty(), "unexpected findings: {findings:?}");
}

#[test]
fn empty_manifest_is_a_finding() {
    let m = Manifest::default();
    let findings = validate_manifest(&m, "", resolver(full_map()));
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].category, "empty-manifest");
    assert!(findings[0].is_error());
}

#[test]
fn missing_world_file_is_source_located_error() {
    let m = parse_manifest(MANIFEST).unwrap();
    // Only default resolves; combat_test is missing.
    let mut map = HashMap::new();
    map.insert("assets/worlds/default.toml".to_string(), world_with_ships());
    let findings = validate_manifest(&m, MANIFEST, resolver(map));
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].category, "missing-scenario-world");
    assert_eq!(findings[0].source.file, "assets/scenarios.toml");
    assert_eq!(
        findings[0].source.reference,
        "assets/worlds/combat_test.toml"
    );
    assert!(findings[0].source.line.is_some(), "line should be located");
}

#[test]
fn unparseable_world_is_a_finding() {
    let m = parse_manifest(MANIFEST).unwrap();
    let mut map = full_map();
    map.insert(
        "assets/worlds/combat_test.toml".to_string(),
        "not valid [".to_string(),
    );
    let findings = validate_manifest(&m, MANIFEST, resolver(map));
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].category, "unparseable-scenario-world");
}

#[test]
fn curated_ship_offered_by_world_produces_no_finding() {
    let toml = r#"
[[scenario]]
id = "combat_test"
world = "assets/worlds/combat_test.toml"
ships = ["assets/entities/alliance_battleship.toml"]
"#;
    let m = parse_manifest(toml).unwrap();
    let findings = validate_manifest(&m, toml, resolver(full_map()));
    assert!(findings.is_empty(), "unexpected findings: {findings:?}");
}

#[test]
fn curated_ship_not_offered_by_world_is_a_finding() {
    let toml = r#"
[[scenario]]
id = "combat_test"
world = "assets/worlds/combat_test.toml"
ships = ["assets/entities/alliance_destroyer.toml"]
"#;
    // combat_world() only offers the battleship — the destroyer is not one
    // of its [[available_ships]], so curating it is a source-located error.
    let m = parse_manifest(toml).unwrap();
    let findings = validate_manifest(&m, toml, resolver(full_map()));
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].category, "unknown-scenario-ship");
    assert_eq!(
        findings[0].source.reference,
        "assets/entities/alliance_destroyer.toml"
    );
    assert!(findings[0].is_error());
}

#[test]
fn duplicate_scenario_id_is_a_finding() {
    let toml = r#"
[[scenario]]
id = "dup"
world = "assets/worlds/default.toml"

[[scenario]]
id = "dup"
world = "assets/worlds/combat_test.toml"
"#;
    let m = parse_manifest(toml).unwrap();
    let findings = validate_manifest(&m, toml, resolver(full_map()));
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].category, "duplicate-scenario-id");
}

#[test]
fn empty_id_and_empty_world_are_findings() {
    let toml = r#"
[[scenario]]
id = ""
world = "assets/worlds/default.toml"

[[scenario]]
id = "noworld"
world = ""
"#;
    let m = parse_manifest(toml).unwrap();
    let findings = validate_manifest(&m, toml, resolver(full_map()));
    let cats: Vec<&str> = findings.iter().map(|f| f.category).collect();
    assert!(cats.contains(&"invalid-manifest-entry"));
    assert_eq!(
        cats.iter()
            .filter(|c| **c == "invalid-manifest-entry")
            .count(),
        2
    );
}

// -- catalog -------------------------------------------------------------

#[test]
fn catalog_curates_ships_to_the_manifest_allowlist() {
    // world_with_ships() offers cruiser then destroyer; curate down to just
    // the destroyer without touching the world file at all (issue #917).
    let toml = r#"
[[scenario]]
id = "default"
world = "assets/worlds/default.toml"
ships = ["assets/entities/alliance_destroyer.toml"]
"#;
    let m = parse_manifest(toml).unwrap();
    let catalog = build_catalog(&m, resolver(full_map()));
    assert_eq!(catalog.scenarios.len(), 1);
    assert_eq!(catalog.scenarios[0].ships.len(), 1);
    assert_eq!(
        catalog.scenarios[0].ships[0].template_path,
        "assets/entities/alliance_destroyer.toml"
    );
}

#[test]
fn multi_ship_catalogue_refuses_a_curated_out_default() {
    let manifest_toml = r#"
[[scenario]]
id = "fleet"
world = "assets/worlds/fleet.toml"
ships = ["destroyer.toml"]
"#;
    let world = r#"
[[ship_slot]]
id = "lead"
default_ship = "cruiser.toml"

[[ship_slot.ships]]
template_path = "cruiser.toml"

[[ship_slot.ships]]
template_path = "destroyer.toml"
"#;
    let manifest = parse_manifest(manifest_toml).unwrap();
    let map = HashMap::from([("assets/worlds/fleet.toml".into(), world.into())]);
    let findings = validate_manifest(&manifest, manifest_toml, resolver(map.clone()));
    assert!(findings
        .iter()
        .any(|finding| finding.category == "excluded-curated-slot-default"));
    assert!(build_catalog(&manifest, resolver(map)).scenarios.is_empty());
}

#[test]
fn multi_ship_catalogue_refuses_a_slot_emptied_by_curation() {
    let manifest_toml = r#"
[[scenario]]
id = "fleet"
world = "assets/worlds/fleet.toml"
ships = ["destroyer.toml"]
"#;
    let world = r#"
[[available_ships]]
template_path = "destroyer.toml"

[[ship_slot]]
id = "lead"
default_ship = "cruiser.toml"

[[ship_slot.ships]]
template_path = "cruiser.toml"
"#;
    let manifest = parse_manifest(manifest_toml).unwrap();
    let map = HashMap::from([("assets/worlds/fleet.toml".into(), world.into())]);
    let findings = validate_manifest(&manifest, manifest_toml, resolver(map.clone()));
    assert!(findings
        .iter()
        .any(|finding| finding.category == "empty-curated-ship-slot"));
    assert!(build_catalog(&manifest, resolver(map)).scenarios.is_empty());
}

#[test]
fn catalog_ship_curation_preserves_world_authored_order() {
    // Curation lists the ships out of order; the catalog keeps the WORLD's
    // order, not the manifest's — the manifest only filters membership.
    let toml = r#"
[[scenario]]
id = "default"
world = "assets/worlds/default.toml"
ships = ["assets/entities/alliance_destroyer.toml", "assets/entities/alliance_cruiser.toml"]
"#;
    let m = parse_manifest(toml).unwrap();
    let catalog = build_catalog(&m, resolver(full_map()));
    assert_eq!(catalog.scenarios[0].ships.len(), 2);
    // world_with_ships() authors cruiser first, then destroyer.
    assert_eq!(
        catalog.scenarios[0].ships[0].template_path,
        "assets/entities/alliance_cruiser.toml"
    );
    assert_eq!(
        catalog.scenarios[0].ships[1].template_path,
        "assets/entities/alliance_destroyer.toml"
    );
}

#[test]
fn catalog_exposes_only_scenario_ships() {
    let m = parse_manifest(MANIFEST).unwrap();
    let catalog = build_catalog(&m, resolver(full_map()));
    assert_eq!(catalog.scenarios.len(), 2);

    let default = &catalog.scenarios[0];
    assert_eq!(default.id, "default");
    // Falls back to the world's [global] title.
    assert_eq!(default.label.as_deref(), Some("world.default.title"));
    assert_eq!(
        default.description.as_deref(),
        Some("world.default.description")
    );
    // Only the default world's two ships — not the combat world's.
    assert_eq!(default.ships.len(), 2);
    assert_eq!(
        default.ships[0].template_path,
        "assets/entities/alliance_cruiser.toml"
    );
    assert_eq!(
        default.ships[1].template_path,
        "assets/entities/alliance_destroyer.toml"
    );

    let combat = &catalog.scenarios[1];
    assert_eq!(combat.ships.len(), 1);
    assert_eq!(
        combat.ships[0].template_path,
        "assets/entities/alliance_battleship.toml"
    );
}

#[test]
fn catalog_entry_label_override_wins_over_world_title() {
    let toml = r#"
[[scenario]]
id = "default"
world = "assets/worlds/default.toml"
label = "Override"
"#;
    let m = parse_manifest(toml).unwrap();
    let catalog = build_catalog(&m, resolver(full_map()));
    assert_eq!(catalog.scenarios[0].label.as_deref(), Some("Override"));
}

#[test]
fn catalog_skips_unresolvable_worlds() {
    let m = parse_manifest(MANIFEST).unwrap();
    let mut map = HashMap::new();
    map.insert("assets/worlds/default.toml".to_string(), world_with_ships());
    let catalog = build_catalog(&m, resolver(map));
    // combat_test could not be resolved, so only default is catalogued.
    assert_eq!(catalog.scenarios.len(), 1);
    assert_eq!(catalog.scenarios[0].id, "default");
}

#[test]
fn catalog_scenario_with_no_ships_is_empty_not_missing() {
    let toml = r#"
[[scenario]]
id = "story"
world = "assets/worlds/story.toml"
"#;
    let mut map = HashMap::new();
    map.insert(
        "assets/worlds/story.toml".to_string(),
        "[global]\ntitle = \"world.story.title\"\n".to_string(),
    );
    let m = parse_manifest(toml).unwrap();
    let catalog = build_catalog(&m, resolver(map));
    assert_eq!(catalog.scenarios.len(), 1);
    assert!(catalog.scenarios[0].ships.is_empty());
}

// -- merged catalog (issue #760, AC3) ------------------------------------

#[test]
fn merged_catalog_contains_only_manifest_listed_scenarios() {
    let base = parse_manifest(MANIFEST).unwrap();
    let mod_manifest = parse_manifest(
        r#"
[[scenario]]
id = "mod_skirmish"
world = "assets/worlds/mod_skirmish.toml"
"#,
    )
    .unwrap();

    // Overlay holds the base worlds, the listed mod world, AND an extra
    // mod world that no manifest names — the latter must NOT appear.
    let mut map = full_map();
    map.insert(
        "assets/worlds/mod_skirmish.toml".to_string(),
        "[global]\ntitle = \"world.mod_skirmish.title\"\n".to_string(),
    );
    map.insert(
        "assets/worlds/unlisted_mod.toml".to_string(),
        "[global]\ntitle = \"world.unlisted.title\"\n".to_string(),
    );

    let merged = build_merged_catalog(&base, &[("modpack", &mod_manifest)], resolver(map));
    let ids: Vec<&str> = merged
        .catalog
        .scenarios
        .iter()
        .map(|s| s.id.as_str())
        .collect();
    assert_eq!(ids, ["default", "combat_test", "mod_skirmish"]);
    assert!(
        !ids.contains(&"unlisted_mod"),
        "an overlay world not named by a manifest must not be selectable"
    );
    // The appended mod scenario is stamped with its pack id (issue #987).
    let skirmish = merged
        .catalog
        .scenarios
        .iter()
        .find(|s| s.id == "mod_skirmish")
        .unwrap();
    assert_eq!(skirmish.origin.as_deref(), Some("modpack"));
}

#[test]
fn merged_catalog_mod_entry_replaces_base_id() {
    let base = parse_manifest(MANIFEST).unwrap();
    let mod_manifest = parse_manifest(
        r#"
[[scenario]]
id = "default"
world = "assets/worlds/mod_default.toml"
label = "Modded Default"
"#,
    )
    .unwrap();
    let mut map = full_map();
    map.insert(
        "assets/worlds/mod_default.toml".to_string(),
        "[global]\ntitle = \"world.mod_default.title\"\n".to_string(),
    );
    let merged = build_merged_catalog(&base, &[("modpack", &mod_manifest)], resolver(map));
    // Still two ids (default replaced in place, not duplicated).
    assert_eq!(merged.catalog.scenarios.len(), 2);
    let default = merged
        .catalog
        .scenarios
        .iter()
        .find(|s| s.id == "default")
        .unwrap();
    assert_eq!(default.world, "assets/worlds/mod_default.toml");
    assert_eq!(default.label.as_deref(), Some("Modded Default"));
    // The base-id-replacement case (issue #990): the entry that replaced the
    // base `default` now reports its PACK as the origin, not base — a player
    // must see it as mod-supplied even though it wears a base scenario id.
    assert_eq!(default.origin.as_deref(), Some("modpack"));
    // Replacing a BASE scenario is the sanctioned override — no warning.
    assert!(
        merged.findings.is_empty(),
        "base-vs-mod replacement must not warn: {:?}",
        merged.findings
    );
}

#[test]
fn merged_catalog_base_scenarios_report_no_origin() {
    // A base-manifest scenario carries NO origin (issue #987/#990): `None`
    // is what the bridge flattens to the wire `source: "base"`, so a base
    // scenario is never badged as mod-supplied. Asserted both with a mod
    // pack present (to prove only the mod entry is stamped) and without.
    let base = parse_manifest(MANIFEST).unwrap();
    let mod_manifest = parse_manifest(
        r#"
[[scenario]]
id = "mod_skirmish"
world = "assets/worlds/mod_skirmish.toml"
"#,
    )
    .unwrap();
    let mut map = full_map();
    map.insert(
        "assets/worlds/mod_skirmish.toml".to_string(),
        "[global]\ntitle = \"world.mod_skirmish.title\"\n".to_string(),
    );
    let merged = build_merged_catalog(&base, &[("modpack", &mod_manifest)], resolver(map));
    let combat = merged
        .catalog
        .scenarios
        .iter()
        .find(|s| s.id == "combat_test")
        .unwrap();
    assert_eq!(combat.origin, None, "a base scenario reports base (None)");
    let skirmish = merged
        .catalog
        .scenarios
        .iter()
        .find(|s| s.id == "mod_skirmish")
        .unwrap();
    assert_eq!(skirmish.origin.as_deref(), Some("modpack"));
}

#[test]
fn merged_catalog_without_mods_is_base_only() {
    let base = parse_manifest(MANIFEST).unwrap();
    let merged = build_merged_catalog(&base, &[], resolver(full_map()));
    assert_eq!(merged.catalog.scenarios.len(), 2);
    assert!(merged.findings.is_empty());
}

#[test]
fn merged_catalog_duplicate_scenario_id_across_packs_resolves_by_load_order() {
    let base = parse_manifest(MANIFEST).unwrap();
    let mod_a =
        parse_manifest("[[scenario]]\nid = \"shared\"\nworld = \"assets/worlds/shared_a.toml\"\n")
            .unwrap();
    let mod_b =
        parse_manifest("[[scenario]]\nid = \"shared\"\nworld = \"assets/worlds/shared_b.toml\"\n")
            .unwrap();
    let mut map = full_map();
    map.insert(
        "assets/worlds/shared_a.toml".to_string(),
        "[global]\ntitle = \"world.shared_a.title\"\n".to_string(),
    );
    map.insert(
        "assets/worlds/shared_b.toml".to_string(),
        "[global]\ntitle = \"world.shared_b.title\"\n".to_string(),
    );
    // packA loaded first, packB last → packB wins the shared id.
    let merged = build_merged_catalog(
        &base,
        &[("packA", &mod_a), ("packB", &mod_b)],
        resolver(map),
    );
    let shared = merged
        .catalog
        .scenarios
        .iter()
        .find(|s| s.id == "shared")
        .unwrap();
    assert_eq!(shared.world, "assets/worlds/shared_b.toml");
    assert_eq!(shared.origin.as_deref(), Some("packB"));
    // The cross-pack collision raised a non-blocking warning naming both.
    assert_eq!(merged.findings.len(), 1);
    assert_eq!(merged.findings[0].category, "duplicate-scenario-id");
    assert!(!merged.findings[0].is_error());
}

// -- shipped manifest ----------------------------------------------------

/// The real shipped manifest must parse, list exactly the selectable roots
/// (`combat_test`, `falling_skyway`, `alliance_convoy_escort`, and
/// `cruiser_elimination`), and validate
/// cleanly against the shipped world files — the pre-load catalog is
/// authoritative, so a broken manifest must fail in CI rather than at host
/// startup.
#[test]
fn shipped_manifest_parses_and_validates() {
    let manifest_toml = include_str!("../../assets/scenarios.toml");
    let m = parse_manifest(manifest_toml).expect("scenarios.toml must parse");
    let ids: Vec<&str> = m.scenarios.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "combat_test",
            "falling_skyway",
            "alliance_convoy_escort",
            "cruiser_elimination",
        ]
    );

    let mut map = HashMap::new();
    map.insert(
        "assets/worlds/combat_test.toml".to_string(),
        include_str!("../../assets/worlds/combat_test.toml").to_string(),
    );
    map.insert(
        "assets/worlds/falling_skyway.toml".to_string(),
        include_str!("../../assets/worlds/falling_skyway.toml").to_string(),
    );
    map.insert(
        "assets/worlds/alliance_convoy_escort.toml".to_string(),
        include_str!("../../assets/worlds/alliance_convoy_escort.toml").to_string(),
    );
    map.insert(
        "assets/worlds/cruiser_elimination.toml".to_string(),
        include_str!("../../assets/worlds/cruiser_elimination.toml").to_string(),
    );

    let findings = validate_manifest(&m, manifest_toml, resolver(map.clone()));
    assert!(
        findings.is_empty(),
        "shipped manifest must validate cleanly: {findings:?}"
    );

    // The catalog exposes each scenario's own ships, drawn from its world.
    let catalog = build_catalog(&m, resolver(map));
    assert_eq!(catalog.scenarios.len(), 4);
    let elimination = catalog
        .scenarios
        .iter()
        .find(|scenario| scenario.id == "cruiser_elimination")
        .expect("the competitive reference world is selectable");
    assert_eq!(
        elimination
            .ships
            .iter()
            .map(|ship| ship.template_path.as_str())
            .collect::<Vec<_>>(),
        [
            "assets/entities/alliance_cruiser.toml",
            "assets/entities/dynasty_player_cruiser.toml",
        ]
    );
    assert_eq!(elimination.slots.len(), 4);
    assert!(elimination.slots.iter().all(|slot| {
        slot.unclaimed == crate::world::config::UnclaimedSlotPolicy::Backfill
            && slot.ships.len() == 1
            && slot.default_ship == slot.ships[0].template_path
    }));
    let convoy = catalog
        .scenarios
        .iter()
        .find(|s| s.id == "alliance_convoy_escort")
        .expect("convoy appears in the selectable catalogue");
    assert_eq!(
        convoy.ships.len(),
        2,
        "convoy exposes both permitted Alliance hulls"
    );
    // Falling Skyway offers exactly the destroyer — the small-crew hull the
    // mission is authored for (issue #1034). Read from the WORLD's own
    // `[[available_ships]]`, not curated in the manifest.
    let skyway = catalog
        .scenarios
        .iter()
        .find(|s| s.id == "falling_skyway")
        .expect("the manifest lists Falling Skyway");
    assert_eq!(
        skyway
            .ships
            .iter()
            .map(|s| s.template_path.as_str())
            .collect::<Vec<_>>(),
        ["assets/entities/alliance_destroyer.toml"]
    );
}

/// The demo curation manifest (issue #917): must parse, curate the
/// catalogue down to exactly `combat_test`, and — without editing
/// `combat_test.toml`, which now authors five `[[available_ships]]` —
/// resolve the ship list to the Destroyer followed by the Cruiser.
#[test]
fn demo_manifest_curates_to_combat_test_destroyer_then_cruiser() {
    let manifest_toml = include_str!("../../assets/scenarios.demo.toml");
    let m = parse_manifest(manifest_toml).expect("scenarios.demo.toml must parse");
    let ids: Vec<&str> = m.scenarios.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids, ["combat_test"]);
    assert_eq!(
        m.scenarios[0]
            .ships
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        [
            "assets/entities/alliance_destroyer.toml",
            "assets/entities/alliance_cruiser.toml"
        ],
        "the demo allowlist keeps the Destroyer first/default"
    );

    let mut map = HashMap::new();
    map.insert(
        "assets/worlds/combat_test.toml".to_string(),
        include_str!("../../assets/worlds/combat_test.toml").to_string(),
    );

    let findings = validate_manifest(&m, manifest_toml, resolver(map.clone()));
    assert!(
        findings.is_empty(),
        "demo manifest must validate cleanly: {findings:?}"
    );

    let catalog = build_catalog(&m, resolver(map));
    assert_eq!(catalog.scenarios.len(), 1);
    assert_eq!(catalog.scenarios[0].id, "combat_test");
    assert_eq!(
        catalog.scenarios[0]
            .ships
            .iter()
            .map(|ship| ship.template_path.as_str())
            .collect::<Vec<_>>(),
        [
            "assets/entities/alliance_destroyer.toml",
            "assets/entities/alliance_cruiser.toml"
        ]
    );

    // combat_test.toml itself still authors all five hulls. Curation happens
    // only in the manifest's ships allowlist.
    let combat_toml = include_str!("../../assets/worlds/combat_test.toml");
    let world = parse_world(combat_toml).expect("combat_test.toml must parse");
    assert_eq!(world.available_ships.len(), 5);
}

// -- exported mod-pack manifest ------------------------------------------

/// The editor mod-pack exporter (issue #759) writes its `scenarios.toml`
/// with the SAME `[[scenario]]` schema this module parses — that is the
/// shared content-pack validation surface the upload path (#760) reuses.
/// A manifest shaped exactly like the exporter's `buildManifestToml`
/// output must parse and validate cleanly against the pack's own worlds.
#[test]
fn exported_mod_pack_manifest_parses_and_validates() {
    // Byte-for-byte the shape smol-toml emits for the exporter's
    // `{ scenario: [{ id, world, label? }] }` (see editor/mod-pack-export.js
    // buildManifestToml): one entry with a label, one without.
    let manifest_toml = concat!(
        "[[scenario]]\n",
        "id = \"default\"\n",
        "world = \"assets/worlds/default.toml\"\n",
        "label = \"Default\"\n\n",
        "[[scenario]]\n",
        "id = \"skirmish\"\n",
        "world = \"assets/worlds/skirmish.toml\"\n",
    );

    let m = parse_manifest(manifest_toml).expect("exported manifest must parse");
    assert_eq!(m.scenarios.len(), 2);
    assert_eq!(m.scenarios[0].label.as_deref(), Some("Default"));
    assert_eq!(m.scenarios[1].label, None);

    // The pack ships both referenced root worlds — resolve_world reads the
    // pack contents, exactly as an upload would resolve within the archive.
    let mut pack = HashMap::new();
    pack.insert(
        "assets/worlds/default.toml".to_string(),
        "[global]\ntitle = \"world.default.title\"\n".to_string(),
    );
    pack.insert(
        "assets/worlds/skirmish.toml".to_string(),
        "[global]\ntitle = \"world.skirmish.title\"\n".to_string(),
    );

    let findings = validate_manifest(&m, manifest_toml, resolver(pack));
    assert!(
        findings.is_empty(),
        "exported mod-pack manifest must validate cleanly: {findings:?}"
    );
}

/// A mod-pack manifest whose root world is absent from the pack must be a
/// blocking `missing-scenario-world` finding on the same surface — the
/// editor exporter refuses this case before writing, and the host upload
/// must reject it too.
#[test]
fn exported_manifest_with_unresolved_world_is_rejected() {
    let manifest_toml = concat!(
        "[[scenario]]\n",
        "id = \"ghost\"\n",
        "world = \"assets/worlds/ghost.toml\"\n",
    );
    let m = parse_manifest(manifest_toml).unwrap();
    let findings = validate_manifest(&m, manifest_toml, resolver(HashMap::new()));
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].category, "missing-scenario-world");
    assert!(findings[0].is_error());
}

// -- pack manifest + content identity (issue #986) -----------------------

const PACK_MANIFEST: &str = r#"
[pack]
format = 1
id = "aurora-skirmish"
version = "1.0.0"
name = "Aurora Skirmish"
author = "Fixture Author"
description = "A deterministic fixture pack."

[pack.requires]
content_id = "phoenix-base"
content_epoch = 1

[[scenario]]
id = "aurora_skirmish"
world = "assets/worlds/aurora_skirmish.toml"
"#;

#[test]
fn parse_pack_manifest_reads_header_and_scenarios() {
    let pm = parse_pack_manifest(PACK_MANIFEST).expect("must parse");
    let pack = pm.pack.expect("has a [pack] header");
    assert_eq!(pack.format, 1);
    assert_eq!(pack.id, "aurora-skirmish");
    assert_eq!(pack.version, "1.0.0");
    assert_eq!(pack.name, "Aurora Skirmish");
    assert_eq!(pack.author.as_deref(), Some("Fixture Author"));
    assert_eq!(pack.requires.content_id.as_deref(), Some("phoenix-base"));
    assert_eq!(pack.requires.content_epoch, Some(1));
    // The [[scenario]] half is exactly what the base reader produces.
    assert_eq!(pm.manifest.scenarios.len(), 1);
    assert_eq!(pm.manifest.scenarios[0].id, "aurora_skirmish");
}

#[test]
fn parse_pack_manifest_optional_fields_default() {
    // A minimal header: only format + id, no version/name/author/requires.
    let toml = "[pack]\nformat = 1\nid = \"x\"\n\n[[scenario]]\nid = \"s\"\nworld = \"assets/worlds/s.toml\"\n";
    let pm = parse_pack_manifest(toml).expect("must parse");
    let pack = pm.pack.expect("has a [pack] header");
    assert_eq!(pack.version, "");
    assert_eq!(pack.name, "");
    assert_eq!(pack.author, None);
    assert_eq!(pack.requires, PackRequires::default());
}

#[test]
fn parse_pack_manifest_without_pack_header_is_none() {
    // The base manifest shape (no [pack]) still parses through the pack
    // reader, with an absent header — the missing-pack-header seam.
    let pm = parse_pack_manifest(MANIFEST).expect("must parse");
    assert!(pm.pack.is_none());
    assert_eq!(pm.manifest.scenarios.len(), 2);
}

#[test]
fn base_manifest_still_parses_via_unchanged_parse_manifest() {
    // A manifest carrying [pack] + [content] must remain readable by the
    // untouched base reader, which ignores both and sees only [[scenario]].
    let m = parse_manifest(PACK_MANIFEST).expect("base reader ignores [pack]");
    assert_eq!(m.scenarios.len(), 1);
    assert_eq!(m.scenarios[0].id, "aurora_skirmish");
}

#[test]
fn parse_content_identity_reads_content_block() {
    let toml =
        "[content]\nid = \"phoenix-base\"\nepoch = 3\n\n[[scenario]]\nid = \"s\"\nworld = \"w\"\n";
    let id = parse_content_identity(toml).expect("reads [content]");
    assert_eq!(id.id, "phoenix-base");
    assert_eq!(id.epoch, 3);
}

#[test]
fn parse_content_identity_absent_is_none() {
    assert!(parse_content_identity(MANIFEST).is_none());
}

#[test]
fn shipped_base_manifest_declares_content_identity() {
    // The real shipped manifest must carry the host side of the mod-pack
    // compatibility contract, so an upload always has a base to match.
    let id = parse_content_identity(include_str!("../../assets/scenarios.toml"))
        .expect("assets/scenarios.toml must declare [content]");
    assert!(!id.id.trim().is_empty(), "content id must not be empty");
}

#[test]
fn shipped_demo_manifest_declares_content_identity() {
    let id = parse_content_identity(include_str!("../../assets/scenarios.demo.toml"))
        .expect("assets/scenarios.demo.toml must declare [content]");
    assert!(!id.id.trim().is_empty(), "content id must not be empty");
}
