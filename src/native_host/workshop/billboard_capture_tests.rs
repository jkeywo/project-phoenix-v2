use super::*;
use std::collections::BTreeMap;

#[test]
fn capture_paths_are_confined_runtime_model_members() {
    assert!(runtime_model_path("assets/models/ship.png", ".png"));
    assert!(runtime_model_path("assets/models/ship.glb", ".glb"));
    assert!(!runtime_model_path(
        "assets/models/../../../../target.png",
        ".png"
    ));
    assert!(!runtime_model_path("C:/target.png", ".png"));
    assert!(!runtime_model_path("assets/worlds/target.png", ".png"));
}

#[test]
fn authored_output_traversal_is_refused_before_a_tool_can_start() {
    let directory = std::env::temp_dir().join(format!(
        "phoenix-billboard-path-test-{}",
        uuid::Uuid::new_v4()
    ));
    fs::create_dir_all(&directory).unwrap();
    let sidecar = "assets/models/ship.model.toml";
    let source = "assets/models/ship.glb";
    let files = BTreeMap::from([
            (source.into(), vec![1, 2, 3]),
            (sidecar.into(), format!(
                "[[lod]]\nbillboard='assets/models/../../../../target.png'\n[lod.capture]\nsource='{source}'\nyaw_views=8\nresolution=64\npitch=20\n"
            ).into_bytes()),
        ]);
    let mut capture = BillboardCapture::with_executable(
        HostedDocuments::default(),
        directory.clone(),
        directory.join("unused-capture-tool"),
    );
    assert!(capture
        .start(files, sidecar.into(), 0, 0)
        .unwrap_err()
        .contains("runtime model assets"));
    assert!(directory.read_dir().unwrap().next().is_none());
    let _ = fs::remove_dir_all(directory);
}
