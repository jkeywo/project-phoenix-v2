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
