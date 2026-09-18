use super::*;

#[test]
fn project_and_pack_sound_catalogs_require_their_captured_decodable_bytes() {
    const SOUND: &str = "assets/sounds/custom/sonar ping.ogg";
    let catalog = include_str!("../../tests/fixtures/sound-cue-pack.toml");
    let sound = include_bytes!("../../assets/sounds/ui_click.ogg").to_vec();
    let mut files = BTreeMap::from([
        ("assets/scenarios.toml".into(), b"[content]\nid=\"phoenix-base\"\nepoch=1\n[[scenario]]\nid=\"sound\"\nworld=\"assets/worlds/sound.toml\"\n".to_vec()),
        ("assets/worlds/sound.toml".into(), b"[global]\ntitle=\"Sound\"\n".to_vec()),
        (crate::sound_cues::PATH.into(), catalog.as_bytes().to_vec()),
    ]);
    for bytes in [None, Some(b"corrupt".to_vec()), Some(sound.clone())] {
        files.remove(SOUND);
        if let Some(bytes) = bytes.as_ref() {
            files.insert(SOUND.into(), bytes.clone());
        }
        let report = validate_project(&files);
        assert_eq!(
            report.accepted,
            bytes.as_ref() == Some(&sound),
            "{:?}",
            report.findings
        );
        if !report.accepted {
            assert!(report
                .findings
                .iter()
                .any(|finding| finding.category == "invalid-sound-cues"
                    && finding.file == crate::sound_cues::PATH
                    && finding.message.contains(SOUND)));
        }
    }
    let mut members = crate::world::mod_pack::read_store_zip_bytes(include_bytes!(
        "../../tests/fixtures/mod-packs/valid-v1.zip"
    ))
    .unwrap();
    members.insert(crate::sound_cues::PATH.into(), catalog.as_bytes().to_vec());
    let zip = crate::workshop::archive::store_zip(&members).unwrap();
    let mut dependencies = dependencies();
    assert!(!validate_pack(&zip, &dependencies).accepted);
    dependencies.base_assets.insert(SOUND.into(), sound);
    assert!(validate_pack(&zip, &dependencies).accepted);
    dependencies
        .base_assets
        .insert(SOUND.into(), b"corrupt".to_vec());
    assert!(!validate_pack(&zip, &dependencies).accepted);
}

#[test]
fn sound_catalog_requires_captured_asset_and_cannot_hide_informative_metadata() {
    let catalog = r#"version=1
[[assets]]
file="assets/sounds/ui_click.ogg"
category="interface"
informative=false
[[cues]]
id="click"
label="Click"
file="assets/sounds/ui_click.ogg"
category="interface"
audience="gm"
volume=0.12
"#;
    let mut files = BTreeMap::from([
        (
            "assets/scenarios.toml".into(),
            b"[content]\nid=\"base\"\nepoch=1\n".to_vec(),
        ),
        (crate::sound_cues::PATH.into(), catalog.as_bytes().to_vec()),
    ]);
    let missing = validate_project(&files);
    assert!(missing
        .findings
        .iter()
        .any(|finding| finding.category == "invalid-sound-cues"));
    files.insert(
        "assets/sounds/ui_click.ogg".into(),
        include_bytes!("../../assets/sounds/ui_click.ogg").to_vec(),
    );
    assert!(!validate_project(&files)
        .findings
        .iter()
        .any(|finding| finding.category == "invalid-sound-cues"));
    files.insert(
        crate::sound_cues::PATH.into(),
        catalog
            .replace("audience=\"gm\"", "audience=\"hidden\"")
            .into_bytes(),
    );
    assert!(validate_project(&files)
        .findings
        .iter()
        .any(|finding| finding.category == "invalid-sound-cues"));
}

fn dependencies() -> WorkshopDependencies {
    WorkshopDependencies {
        base_files: BTreeMap::from([(
            "assets/scenarios.toml".into(),
            "[content]\nid = \"phoenix-base\"\nepoch = 1\n".into(),
        )]),
        packs: Vec::new(),
        base_assets: BTreeMap::new(),
    }
}

#[test]
fn ordinary_exporter_fixtures_reach_the_runtime_gate() {
    for bytes in [
        include_bytes!("../../tests/fixtures/mod-packs/valid-v1.zip").as_slice(),
        include_bytes!("../../tests/fixtures/mod-packs/script-valid.zip").as_slice(),
    ] {
        let result = validate_pack(bytes, &dependencies());
        assert!(result.accepted, "{:?}", result.findings);
    }
    for bytes in [
        include_bytes!("../../tests/fixtures/mod-packs/script-denied-capability.zip").as_slice(),
        include_bytes!("../../tests/fixtures/mod-packs/schema-invalid-world.zip").as_slice(),
        include_bytes!("../../tests/fixtures/mod-packs/unresolved-manifest-world.zip").as_slice(),
        include_bytes!("../../tests/fixtures/mod-packs/content-epoch-mismatch.zip").as_slice(),
    ] {
        let result = validate_pack(bytes, &dependencies());
        assert!(!result.accepted);
        assert!(result
            .findings
            .iter()
            .any(|finding| finding.severity == "error"));
    }
}

#[test]
fn missing_dependency_bundle_never_means_a_clean_draft() {
    let result = validate_pack(
        include_bytes!("../../tests/fixtures/mod-packs/valid-v1.zip"),
        &WorkshopDependencies::default(),
    );
    assert!(!result.accepted);
    assert_eq!(result.findings[0].category, "missing-base-content");
}

#[test]
fn read_only_manifest_world_compiles_its_actual_script_and_child() {
    let zip = include_bytes!("../../tests/fixtures/mod-packs/unresolved-manifest-world.zip");
    let files = read_store_zip(zip).unwrap();
    let manifest = parse_manifest(&files["scenarios.toml"]).unwrap();
    let world = &manifest.scenarios[0].world;
    let mut dependencies = dependencies();
    dependencies.base_files.insert(
        world.clone(),
        "extra_worlds = [\"assets/worlds/child.toml\"]\n[global]\n".into(),
    );
    dependencies.base_files.insert(
        "assets/worlds/child.toml".into(),
        "script = \"bad.rhai\"\n[global]\n".into(),
    );
    dependencies
        .base_files
        .insert("assets/worlds/bad.rhai".into(), "fn broken( {".into());
    let refused = validate_pack(zip, &dependencies);
    assert!(
        !refused.accepted,
        "an external static child's script must be compiled"
    );
    assert!(
        refused
            .findings
            .iter()
            .any(|finding| finding.category == "script-parse-error"
                && finding.file == "assets/worlds/bad.rhai"),
        "{:?}",
        refused.findings
    );
    dependencies.base_files.insert(
        "assets/worlds/bad.rhai".into(),
        "fn harmless(ctx) {}".into(),
    );
    let accepted = validate_pack(zip, &dependencies);
    assert!(accepted.accepted, "{:?}", accepted.findings);
}

#[test]
fn missing_templates_are_final_even_without_a_running_host() {
    let zip = include_bytes!("../../tests/fixtures/mod-packs/unresolved-manifest-world.zip");
    let files = read_store_zip(zip).unwrap();
    let manifest = parse_manifest(&files["scenarios.toml"]).unwrap();
    let mut dependencies = dependencies();
    dependencies.base_files.insert(
        manifest.scenarios[0].world.clone(),
        "[global]\n[[entity]]\nname = \"lost\"\ntemplate_path = \"assets/entities/missing.toml\"\n"
            .into(),
    );
    let result = validate_pack(zip, &dependencies);
    assert!(!result.accepted);
    assert!(
        result
            .findings
            .iter()
            .any(|finding| finding.message.contains("missing.toml")),
        "{:?}",
        result.findings
    );
}

#[test]
fn validation_does_not_change_the_active_overlay() {
    let before = crate::entities::config_cache::active_packs();
    assert!(
        validate_pack(
            include_bytes!("../../tests/fixtures/mod-packs/valid-v1.zip"),
            &dependencies()
        )
        .accepted
    );
    assert_eq!(before, crate::entities::config_cache::active_packs());
}

// ── Definition findings through the validators (issue #1474) ─────────────────

const ALLIANCE: &str = "aaaaaaaa-1111-4111-8111-aaaaaaaaaaaa";
const ROGUE: &str = "eeeeeeee-5555-4555-8555-eeeeeeeeeeee";
const NOBODY: &str = "ffffffff-6666-4666-8666-ffffffffffff";

fn pack_with_faction(enemy: &str) -> Vec<u8> {
    let mut members = crate::world::mod_pack::read_store_zip_bytes(include_bytes!(
        "../../tests/fixtures/mod-packs/valid-v1.zip"
    ))
    .unwrap();
    members.insert(
        "assets/factions/rogue.toml".into(),
        format!("uuid = \"{ROGUE}\"\nname = \"Rogue\"\nenemies = [\n    \"{enemy}\",\n]\n")
            .into_bytes(),
    );
    crate::workshop::archive::store_zip(&members).unwrap()
}

fn dependencies_with_base_factions() -> WorkshopDependencies {
    let mut dependencies = dependencies();
    dependencies.base_files.insert(
        "assets/factions/alliance.toml".into(),
        include_str!("../../assets/factions/alliance.toml").into(),
    );
    dependencies.base_files.insert(
        "assets/factions/pirate.toml".into(),
        include_str!("../../assets/factions/pirate.toml").into(),
    );
    dependencies
}

#[test]
fn a_pack_faction_naming_an_unknown_enemy_is_refused_with_the_entry_line() {
    let result = validate_pack(
        &pack_with_faction(NOBODY),
        &dependencies_with_base_factions(),
    );
    assert!(!result.accepted);
    let finding = result
        .findings
        .iter()
        .find(|finding| finding.category == "faction-unknown-enemy")
        .unwrap_or_else(|| panic!("{:?}", result.findings));
    assert_eq!(finding.file, "assets/factions/rogue.toml");
    assert_eq!(finding.line, Some(4));
    assert_eq!(finding.severity, "error");
}

#[test]
fn a_pack_faction_may_name_a_base_faction_as_its_enemy() {
    let result = validate_pack(
        &pack_with_faction(ALLIANCE),
        &dependencies_with_base_factions(),
    );
    assert!(result.accepted, "{:?}", result.findings);
    // Without the base set beneath it the same pack dangles: the resolution
    // really is against the dependency bundle, not a compiled-in list.
    let alone = validate_pack(&pack_with_faction(ALLIANCE), &dependencies());
    assert!(!alone.accepted);
    assert!(alone
        .findings
        .iter()
        .any(|finding| finding.category == "faction-unknown-enemy"));
}

#[test]
fn a_project_rung_naming_an_unowned_system_reports_the_rung_entry_line() {
    let hull = format!(
        "class = \"cruiser\"\nfaction = \"{ALLIANCE}\"\n\n[[station]]\nid = \"captain\"\nname = \"Captain\"\n\n[[station.rating]]\nname = \"Std\"\nautomated_systems = []\n\n[[station.rating]]\nname = \"Simplified\"\nautomated_systems = [\n    \"red-alert\",\n    \"phaser-fore\",\n]\n\n[[station]]\nid = \"tactical\"\nname = \"Tactical\"\n\n[[system]]\nid = \"red-alert\"\nkind = \"red_alert\"\nstation = \"captain\"\n\n[[system]]\nid = \"phaser-fore\"\nkind = \"phaser_bank\"\nstation = \"tactical\"\n"
    );
    let files = BTreeMap::from([
        (
            "assets/scenarios.toml".into(),
            b"[content]\nid=\"base\"\nepoch=1\n".to_vec(),
        ),
        ("assets/entities/probe_hull.toml".into(), hull.into_bytes()),
        (
            "assets/factions/alliance.toml".into(),
            include_str!("../../assets/factions/alliance.toml")
                .as_bytes()
                .to_vec(),
        ),
    ]);
    let report = validate_project(&files);
    assert!(!report.accepted);
    let finding = report
        .findings
        .iter()
        .find(|finding| finding.category == "rating-unowned-system")
        .unwrap_or_else(|| panic!("{:?}", report.findings));
    assert_eq!(finding.file, "assets/entities/probe_hull.toml");
    assert_eq!(finding.line, Some(16));
    assert!(finding.message.contains("RatingReferencesUnownedSystem"));
    assert!(
        !report
            .findings
            .iter()
            .any(|finding| finding.category == "entity-unknown-faction"),
        "{:?}",
        report.findings
    );
}

#[test]
fn the_native_provider_answers_definitions_edit_and_new_faction() {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or_default();
    let directory = std::env::temp_dir().join(format!(
        "phoenix-workshop-definitions-{}-{nanos}",
        std::process::id()
    ));
    let root = directory.join("project");
    std::fs::create_dir_all(root.join("assets/factions")).unwrap();
    std::fs::write(
        root.join("assets/scenarios.toml"),
        "[content]\nid='phoenix-base'\nepoch=1\n",
    )
    .unwrap();
    let source = format!("uuid = \"{ROGUE}\"\nname = \"Rogue\"\nenemies = []\n");
    std::fs::write(root.join("assets/factions/rogue.toml"), &source).unwrap();
    let mut provider = provider::NativeWorkshopProvider::open(
        provider::WorkspaceKind::Project,
        &root,
        directory.join("private"),
        WorkshopDependencies::default(),
    )
    .unwrap();

    let catalog = provider.handle_json(&format!(
        "{{\"id\":1,\"op\":\"definitions\",\"files\":{{\"assets/factions/rogue.toml\":\"uuid = \\\"{ROGUE}\\\"\\nname = \\\"Rogue\\\"\\nenemies = []\\n\"}}}}"
    ));
    assert!(catalog.contains("\"status\":\"definitions\""), "{catalog}");
    assert!(catalog.contains("\"origin\":\"draft\""), "{catalog}");
    assert!(
        catalog.contains("\"order_responses\":[\"comply\",\"refuse\"]"),
        "{catalog}"
    );
    assert!(
        catalog.contains("\"ai_rules\":[\"torpedo_auto_fire\"]"),
        "{catalog}"
    );

    let skeleton = provider.handle_json(&format!(
        "{{\"id\":2,\"op\":\"new-faction\",\"name\":\"Rogue\",\"uuid\":\"{ROGUE}\"}}"
    ));
    assert!(skeleton.contains("\"status\":\"patched\""), "{skeleton}");
    assert!(skeleton.contains("name = \\\"Rogue\\\""), "{skeleton}");
    let refused = provider
        .handle_json("{\"id\":3,\"op\":\"new-faction\",\"name\":\" \",\"uuid\":\"not-a-uuid\"}");
    assert!(refused.contains("\"status\":\"refused\""), "{refused}");

    let edited = provider.handle_json(&format!(
        "{{\"id\":4,\"op\":\"edit\",\"source\":\"uuid = 1\\n\",\"edit\":{{\"document_path\":\"assets/factions/rogue.toml\",\"expected_source\":\"uuid = 1\\n\",\"edits\":[{{\"op\":\"put\",\"path\":[\"name\"],\"value_source\":\"\\\"Renamed\\\"\"}},{{\"op\":\"insert\",\"path\":[\"enemies\"],\"index\":0,\"value_source\":\"\\\"{ALLIANCE}\\\"\"}}]}}}}"
    ));
    assert!(
        edited.contains("\"status\":\"refused\""),
        "an insert into a missing array refuses the whole group: {edited}"
    );
    let edited = provider.handle_json(
        "{\"id\":5,\"op\":\"edit\",\"source\":\"uuid = 1\\n\",\"edit\":{\"document_path\":\"assets/factions/rogue.toml\",\"expected_source\":\"uuid = 1\\n\",\"edits\":[{\"op\":\"put\",\"path\":[\"name\"],\"value_source\":\"\\\"Renamed\\\"\"}]}}",
    );
    assert!(edited.contains("\"status\":\"patched\""), "{edited}");
    assert!(
        edited.contains("\"source\":\"uuid = 1\\nname = \\\"Renamed\\\"\\n\""),
        "{edited}"
    );
    drop(provider);
    let _ = std::fs::remove_dir_all(&directory);
}

// ── Composition findings and the same validated catalogue (issue #1475) ──────

const COMPOSITION_HULL: &str = "class='lancer'\nname='Test hull'\n\
[[station]]\nid='captain'\nname='Captain'\ndescription='Test station'\nrank='captain'\n\
[[system]]\nid='boost'\nkind='helm_boost'\nstation='captain'\n";

/// The one member set both paths read: two roots, one curating its hull.
fn composition_members() -> BTreeMap<String, String> {
    BTreeMap::from([
        (
            "assets/worlds/alpha.toml".to_owned(),
            "[global]\ntitle = \"Alpha\"\ndescription = \"First\"\n\n[[available_ships]]\ntemplate_path = \"assets/entities/hull.toml\"\nlabel = \"Hull\"\n\n[[available_ships]]\ntemplate_path = \"assets/entities/other.toml\"\n".to_owned(),
        ),
        (
            "assets/worlds/beta.toml".to_owned(),
            "extra_worlds = [\"assets/worlds/alpha.toml\"]\n[global]\ntitle = \"Beta\"\n".to_owned(),
        ),
        ("assets/entities/hull.toml".to_owned(), COMPOSITION_HULL.to_owned()),
        ("assets/entities/other.toml".to_owned(), COMPOSITION_HULL.to_owned()),
    ])
}

const COMPOSITION_SCENARIOS: &str = "[[scenario]]\nid = \"alpha\"\nworld = \"assets/worlds/alpha.toml\"\nships = [\"assets/entities/hull.toml\"]\n\n[[scenario]]\nid = \"beta\"\nworld = \"assets/worlds/beta.toml\"\nlabel = \"Beta label\"\n";

#[test]
fn a_pack_and_a_project_of_the_same_members_validate_to_the_same_catalogue() {
    // The pack path: members inside a store zip plus a dependency bundle,
    // the candidate exactly as validate_pack assembles it.
    let mut pack_members = composition_members();
    pack_members.insert(
        "scenarios.toml".into(),
        format!(
            "[pack]\nformat = 1\nid = \"twin\"\nversion = \"1.0.0\"\nname = \"Twin\"\n\n[pack.requires]\ncontent_id = \"phoenix-base\"\ncontent_epoch = 1\n\n{COMPOSITION_SCENARIOS}"
        ),
    );
    let zip = crate::workshop::archive::store_zip(
        &pack_members
            .iter()
            .map(|(path, text)| (path.clone(), text.clone().into_bytes()))
            .collect(),
    )
    .unwrap();
    let dependencies = dependencies();
    let pack_report = validate_pack(&zip, &dependencies);
    assert!(pack_report.accepted, "{:?}", pack_report.findings);
    // The gate itself carries the catalogue it read, and it is the one the
    // pure function reads over the candidate the gate assembled.
    let pack_candidate = read_store_zip(&zip).unwrap();
    let mut beneath = dependencies.base_files.clone();
    for pack in &dependencies.packs {
        beneath.extend(pack.files.clone());
    }
    let through_pack = pack_report.catalogue.clone();
    assert_eq!(
        through_pack,
        composition::scenario_catalogue(&pack_candidate, &beneath)
    );

    // The project path: the same members as files, nothing beneath, the
    // base manifest inlined as assets/scenarios.toml.
    let mut project_members = composition_members();
    project_members.insert(
        "assets/scenarios.toml".into(),
        format!("[content]\nid = \"phoenix-base\"\nepoch = 1\n\n{COMPOSITION_SCENARIOS}"),
    );
    let project_files: BTreeMap<String, Vec<u8>> = project_members
        .iter()
        .map(|(path, text)| (path.clone(), text.clone().into_bytes()))
        .collect();
    let project_report = validate_project(&project_files);
    assert!(project_report.accepted, "{:?}", project_report.findings);
    let through_project = project_report.catalogue.clone();
    assert_eq!(
        through_project,
        composition::scenario_catalogue(&project_members, &BTreeMap::new())
    );

    assert_eq!(through_pack.len(), 2);
    for (pack_entry, project_entry) in through_pack.iter().zip(&through_project) {
        assert_eq!(pack_entry, project_entry);
    }
    assert_eq!(through_pack, through_project);
    assert_eq!(through_pack[0].label.as_deref(), Some("Alpha"));
    assert_eq!(through_pack[0].description.as_deref(), Some("First"));
    assert_eq!(
        through_pack[0]
            .ships
            .iter()
            .map(|ship| (ship.template_path.as_str(), ship.label.as_deref()))
            .collect::<Vec<_>>(),
        vec![("assets/entities/hull.toml", Some("Hull"))],
        "the manifest curates the world's offer"
    );
    assert_eq!(through_pack[1].label.as_deref(), Some("Beta label"));
}

#[test]
fn a_project_world_declaring_a_missing_or_cyclic_child_is_refused_at_the_entry_line() {
    let mut members = composition_members();
    members.insert(
        "assets/scenarios.toml".into(),
        format!("[content]\nid = \"phoenix-base\"\nepoch = 1\n\n{COMPOSITION_SCENARIOS}"),
    );
    members.insert(
        "assets/worlds/beta.toml".into(),
        "extra_worlds = [\n    \"assets/worlds/alpha.toml\",\n    \"assets/worlds/gamma.toml\",\n]\n[global]\ntitle = \"Beta\"\n".into(),
    );
    members.insert(
        "assets/worlds/alpha.toml".into(),
        "extra_worlds = [\"assets/worlds/beta.toml\"]\n[global]\ntitle = \"Alpha\"\n".into(),
    );
    let files: BTreeMap<String, Vec<u8>> = members
        .iter()
        .map(|(path, text)| (path.clone(), text.clone().into_bytes()))
        .collect();
    let report = validate_project(&files);
    assert!(!report.accepted);
    let located = |category: &str| {
        report
            .findings
            .iter()
            .filter(|finding| finding.category == category)
            .map(|finding| (finding.file.clone(), finding.line, finding.severity.clone()))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        located("extra-worlds-missing"),
        vec![(
            "assets/worlds/beta.toml".to_owned(),
            Some(3),
            "error".to_owned()
        )]
    );
    assert_eq!(
        located("extra-worlds-cycle"),
        vec![
            (
                "assets/worlds/alpha.toml".to_owned(),
                Some(1),
                "error".to_owned()
            ),
            (
                "assets/worlds/beta.toml".to_owned(),
                Some(2),
                "error".to_owned()
            ),
        ]
    );
}

#[test]
fn a_pack_world_declaring_a_duplicate_child_is_refused_at_the_entry_line() {
    let mut members = crate::world::mod_pack::read_store_zip_bytes(include_bytes!(
        "../../tests/fixtures/mod-packs/valid-v1.zip"
    ))
    .unwrap();
    let manifest =
        parse_manifest(std::str::from_utf8(&members["scenarios.toml"]).unwrap()).unwrap();
    let root = manifest.scenarios[0].world.clone();
    members.insert(
        root.clone(),
        b"extra_worlds = [\n    \"assets/worlds/twin.toml\",\n    \"assets/worlds/twin.toml\",\n]\n[global]\n".to_vec(),
    );
    members.insert(
        "assets/worlds/twin.toml".into(),
        b"[global]\ntitle = \"Twin\"\n".to_vec(),
    );
    let zip = crate::workshop::archive::store_zip(&members).unwrap();
    let report = validate_pack(&zip, &dependencies());
    assert!(!report.accepted);
    let finding = report
        .findings
        .iter()
        .find(|finding| finding.category == "extra-worlds-duplicate")
        .unwrap_or_else(|| panic!("{:?}", report.findings));
    assert_eq!(
        (finding.file.as_str(), finding.line),
        (root.as_str(), Some(3))
    );
    assert!(
        !report
            .findings
            .iter()
            .any(|finding| finding.category == "member-disallowed"),
        "a pack's members are the archive gate's: {:?}",
        report.findings
    );
}

#[test]
fn partial_entity_include_is_validated_only_as_part_of_its_complete_template() {
    let bytes = include_bytes!("../../tests/fixtures/mod-packs/partial-entity-include.zip");
    let files = read_store_zip(bytes).unwrap();
    assert!(
        EntityConfig::from_toml(&files["assets/entities/partial_collider.toml"]).is_err(),
        "the regression fragment must not be a standalone entity"
    );
    let composed = resolve_template("assets/entities/composed_collider.toml", &files)
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(composed.collider.unwrap().radius, 3.0);
    let ordinary = validate_mod_pack(
        bytes,
        &parse_content_identity(&dependencies().base_files["assets/scenarios.toml"]).unwrap(),
        |_| None,
        &Sources(BTreeMap::new()),
        &[],
    );
    assert!(ordinary.is_accepted(), "{:?}", ordinary.findings);
    let result = validate_pack(bytes, &dependencies());
    assert!(result.accepted, "{:?}", result.findings);
}
