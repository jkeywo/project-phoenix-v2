use super::*;
use std::collections::BTreeMap;

fn temporary(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "phoenix-lod-generation-{name}-{}",
        uuid::Uuid::new_v4()
    ))
}

#[test]
fn progress_is_a_bounded_tail_and_start_refuses_a_sidecar_without_generation() {
    let root = temporary("root");
    let stages = temporary("stages");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&stages).unwrap();
    let log = root.join("progress.log");
    fs::write(
        &log,
        (0..80)
            .map(|index| format!("{index}:{}\n", "x".repeat(300)))
            .collect::<String>(),
    )
    .unwrap();
    let lines = progress(&log);
    assert_eq!(lines.len(), 64);
    assert!(lines.iter().all(|line| line.chars().count() <= 240));
    assert!(lines.first().unwrap().starts_with("16:"));

    let documents = HostedDocuments::default();
    let mut run = LodGeneration::new(
        documents,
        "http://127.0.0.1:7".into(),
        root.clone(),
        stages.clone(),
    )
    .unwrap();
    let refused = run
        .start(
            BTreeMap::from([("assets/models/ship.model.toml".into(), b"[base]\n".to_vec())]),
            "assets/models/ship.model.toml".into(),
            3,
            false,
        )
        .unwrap_err();
    assert!(refused.contains("declares no generated LOD"));
    assert!(stages.read_dir().unwrap().next().is_none());
    let _ = fs::remove_dir_all(root);
    let _ = fs::remove_dir_all(stages);
}

#[test]
fn generated_members_stay_in_the_runtime_model_namespace() {
    assert!(model_glb("assets/models/ship_lod1.glb"));
    assert!(!model_glb("assets/models/../private.glb"));
    assert!(!model_glb("scripts/output.glb"));
    assert!(generation_source_glb("scripts/art/lod-sources/ship.glb"));
    assert!(!generation_source_glb(
        "scripts/art/lod-sources/../private.glb"
    ));
    let required = BTreeSet::from(["assets/models/ship_lod1.glb".into()]);
    assert!(!complete_review(
        &required,
        &["assets/models/ship.remesh.glb".into(), MANIFEST.into()]
    ));
}
