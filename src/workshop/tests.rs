use super::*;

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
