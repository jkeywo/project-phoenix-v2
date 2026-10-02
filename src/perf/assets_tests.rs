use super::*;
use crate::perf::profile;

/// The join the budget now makes: a template points at a sidecar, and the
/// sidecar is what carries the ladder (issue #914).
#[test]
fn a_template_resolves_the_sidecar_that_carries_its_ladder() {
    let text = "[mesh]\nmodel = \"assets/models/rock.glb\"\nvariant = \"large\"\n";
    assert_eq!(
        mesh_sidecar(text).as_deref(),
        Some("assets/models/rock.large.toml")
    );
}

#[test]
fn a_template_without_a_variant_resolves_the_default_sidecar() {
    let text = "[mesh]\nmodel = \"assets/models/ship.glb\"\nshape = \"sphere\"\n";
    assert_eq!(
        mesh_sidecar(text).as_deref(),
        Some("assets/models/ship.model.toml")
    );
}

#[test]
fn a_procedural_template_names_no_glb() {
    assert_eq!(
        mesh_sidecar("[mesh]\nshape = \"sphere\"\nradius = 4\n"),
        None
    );
}

#[test]
fn a_template_with_no_mesh_is_skipped_entirely() {
    assert_eq!(mesh_sidecar("name = \"thing\"\n"), None);
}

#[test]
fn a_sidecar_with_a_ladder_reports_its_level_count() {
    let text = r#"
[base]
offset = [0.0, 0.0, 0.0]

[[lod]]
max_distance = 50.0

[[lod]]
max_distance = 100.0
"#;
    assert_eq!(sidecar_lod_levels(text), 2);
}

#[test]
fn a_sidecar_with_no_ladder_reports_zero_levels() {
    assert_eq!(sidecar_lod_levels("[base]\nscale = [1.0, 1.0, 1.0]\n"), 0);
}

/// A commented-out ladder is the case grepping would get wrong.
#[test]
fn commented_out_lod_is_not_counted() {
    assert_eq!(sidecar_lod_levels("# [[lod]]\n# max_distance = 50.0\n"), 0);
}

#[test]
fn the_capture_totals_the_glb_bytes() {
    let mut found = Inventory::default();
    found.glb_bytes.insert("a.glb".into(), 100);
    found.glb_bytes.insert("b.glb".into(), 400);
    found.lod_levels.insert("rock.toml".into(), 3);
    found.without_lod.push("ship.toml".into());

    let capture = capture(&found, profile(RUNTIME));
    assert_eq!(capture.summaries[GLB_TOTAL_METRIC].summary.max, 500.0);
    assert_eq!(capture.summaries[GLB_BYTES_METRIC].summary.count, 2);
    assert_eq!(capture.summaries[GLB_BYTES_METRIC].summary.max, 400.0);
    assert_eq!(capture.summaries[LOD_LEVELS_METRIC].summary.max, 3.0);
    assert_eq!(capture.summaries[WITHOUT_LOD_METRIC].summary.max, 1.0);
}

/// The real tree, because an extractor that works only on fixtures is an
/// extractor that has never met the assets it budgets.
#[test]
fn the_repository_inventory_finds_models_and_ladders() {
    let found = inventory(Path::new(".")).expect("assets directories exist");
    assert!(
        !found.glb_bytes.is_empty(),
        "no .glb files found under assets/models"
    );
    assert!(
        !found.lod_levels.is_empty(),
        "no entity template declares a LOD ladder"
    );
}

#[test]
fn a_missing_tree_is_an_error_not_an_empty_inventory() {
    assert!(inventory(Path::new("no/such/root")).is_err());
}

#[test]
fn remesh_intermediates_are_not_shipped_glb_inventory() {
    let root = std::env::temp_dir().join(format!(
        "phoenix_perf_assets_remesh_fixture_{}",
        std::process::id()
    ));
    let models = root.join("assets/models");
    let entities = root.join("assets/entities");
    std::fs::create_dir_all(&models).expect("fixture models directory");
    std::fs::create_dir_all(&entities).expect("fixture entities directory");
    std::fs::write(models.join("rock.glb"), [0u8; 7]).expect("runtime model fixture");
    std::fs::write(models.join("rock.remesh.glb"), [0u8; 19])
        .expect("generator intermediate fixture");

    let found = inventory(&root).expect("fixture inventory");
    assert_eq!(found.glb_bytes, BTreeMap::from([("rock.glb".into(), 7)]));

    std::fs::remove_dir_all(&root).ok();
}

/// The generated LOD files and this budget agree about their size
/// (issue #919).
///
/// `scripts/generate-lods.mjs` writes a manifest recording, per generated
/// `.glb`, the source it came from, the parameters that made it, and how
/// big it turned out. That last number is *this* module's measurement —
/// asserted here rather than re-derived, so the pipeline cannot grow a
/// second opinion about file size. It also means a generated LOD that was
/// hand-edited, truncated or reverted without regenerating fails `cargo
/// test`, not only the Node drift check in CI.
///
/// Triangle and texture budgets live in [`super::super::mesh`]; nothing
/// here counts them.
#[test]
fn the_lod_manifest_records_the_bytes_this_inventory_measures() {
    let text = std::fs::read_to_string("scripts/lod-manifest.toml")
        .expect("the LOD manifest is committed alongside the generated files");
    let doc: toml::Value = toml::from_str(&text).expect("the manifest parses as TOML");
    let outputs = doc
        .get("output")
        .and_then(|o| o.as_array())
        .expect("the manifest lists [[output]] records");
    assert!(
        !outputs.is_empty(),
        "the shipped tree generates at least one LOD level"
    );

    let found = inventory(Path::new(".")).expect("assets directories exist");
    let mut problems: Vec<String> = Vec::new();
    for output in outputs {
        let path = output
            .get("path")
            .and_then(|p| p.as_str())
            .expect("every record names its file");
        let recorded = output
            .get("output_bytes")
            .and_then(|b| b.as_integer())
            .expect("every record carries the size it was written at")
            as u64;
        let key = path.rsplit('/').next().unwrap_or(path);
        match found.glb_bytes.get(key) {
            None => problems.push(format!("{path}: recorded in the manifest, missing on disk")),
            Some(&bytes) if bytes != recorded => problems.push(format!(
                "{path}: manifest recorded {recorded} bytes, disk has {bytes}"
            )),
            Some(_) => {}
        }
    }
    assert!(
        problems.is_empty(),
        "generated LODs have drifted from scripts/lod-manifest.toml — regenerate with \
             `npm run lods` (or re-baseline with `--adopt`) and commit both:\n{}",
        problems.join("\n")
    );
}
