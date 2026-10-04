use super::*;
use crate::workshop::test_protocol::TestSelection;

pub(super) fn snapshot() -> TestSnapshot {
    TestSnapshot {
        files: std::collections::BTreeMap::from([
            (
                "assets/worlds/test.toml".into(),
                b"# exact\r\n[global]\r\n".to_vec(),
            ),
            (
                "assets/gui/dpad-button-idle.png".into(),
                include_bytes!("../../../assets/gui/dpad-button-idle.png").to_vec(),
            ),
            (
                "assets/shaders/test.wgsl".into(),
                b"// captured support\r\n".to_vec(),
            ),
        ]),
        selection: TestSelection {
            world: "assets/worlds/test.toml".into(),
            slot: None,
            ship: "assets/entities/test.toml".into(),
            seed: 1,
        },
        revision: "fixture".into(),
        breakpoint: None,
    }
}
struct Fixture(PathBuf);
impl Fixture {
    #[allow(clippy::disallowed_methods)] // Isolated test filesystem identity.
    fn new() -> Self {
        Self(
            std::env::temp_dir().join(format!("phoenix-workshop-process-{}", uuid::Uuid::new_v4())),
        )
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn exact_staged_source_is_removed_on_drop_and_abandoned_owned_stages_are_reclaimed() {
    let fixture = Fixture::new();
    let (stage, _) = Stage::create(&fixture.0, snapshot()).unwrap();
    let path = stage.path.clone();
    assert_eq!(
        fs::read(path.join("assets/worlds/test.toml")).unwrap(),
        b"# exact\r\n[global]\r\n"
    );
    drop(stage);
    assert!(!path.exists());
    let (abandoned, _) = Stage::create(&fixture.0, snapshot()).unwrap();
    let abandoned_path = abandoned.path.clone();
    std::mem::forget(abandoned); // Hard shutdown does not run Drop.
    fs::create_dir_all(fixture.0.join("operator-files")).unwrap();
    let malformed = fixture.0.join("00000000-0000-0000-0000-000000000001");
    fs::create_dir(&malformed).unwrap();
    fs::write(malformed.join("workshop-test.json"), "invalid").unwrap();
    retire_abandoned_stages(&fixture.0);
    assert!(!abandoned_path.exists());
    assert!(fixture.0.join("operator-files").is_dir());
    assert!(malformed.is_dir());
}

#[test]
fn private_records_refuse_oversize_partial_and_extra_authority_fields() {
    assert!(bounded_line(&mut std::io::Cursor::new(vec![
        b'x';
        MAX_RECORD_BYTES as usize + 1
    ]))
    .is_err());
    assert!(bounded_line(&mut std::io::Cursor::new(b"partial")).is_err());
    assert_eq!(
        bounded_line(&mut std::io::Cursor::new(b"complete\r\n")).unwrap(),
        Some("complete".into())
    );
    for value in [
        r#"{"id":1,"control":{"command":"pause","path":"outside"}}"#,
        r#"{"id":1,"control":{"command":"step"},"world":"outside"}"#,
        r#"{"id":-1,"control":{"command":"pause"}}"#,
        r#"{"id":1,"control":{"command":"script","source":"arbitrary"}}"#,
    ] {
        assert!(
            crate::core::codec::from_json::<crate::workshop::test_protocol::ControlRecord>(value)
                .is_err(),
            "{value}"
        );
    }
}

#[test]
fn inherited_pipe_acknowledgement_and_drop_retire_the_real_child_and_stage() {
    let fixture = Fixture::new();
    let (mut process, path) = pipe_probe(&fixture.0);
    let status = process.control(TestControl::Pause {}).unwrap();
    assert!(status.running && status.paused && !status.starting);
    assert_eq!(status.acknowledged, 1);
    assert!(process
        .control(TestControl::Rate { multiplier: 255 })
        .is_err());
    assert!(path.exists());
    drop(process);
    assert!(!path.exists());
}

#[test]
#[ignore = "Child fixture launched only by inherited-pipe lifecycle tests"]
fn pipe_probe_child() {
    let Some(path) = std::env::var_os("PHOENIX_WORKSHOP_PIPE_PROBE") else {
        return;
    };
    let path = PathBuf::from(path);
    let launch = crate::core::codec::from_json_bytes::<crate::workshop::test_protocol::Launch>(
        &fs::read(&path).unwrap(),
    )
    .unwrap();
    let root = pin_test_content_root(&path).unwrap();
    assert_eq!(
        fs::canonicalize(std::env::current_dir().unwrap()).unwrap(),
        root
    );
    // Use the actual shared shell's AssetPlugin/AssetServer source, not a
    // direct filesystem read. Cargo's root and a deliberately inherited
    // BEVY_ASSET_ROOT must not make uncaptured shipped content available.
    let app = super::super::build_shell(crate::boot::NativeRenderSurface::Contract).unwrap();
    let source = app
        .world()
        .resource::<AssetServer>()
        .get_source(bevy::asset::io::AssetSourceId::Default)
        .unwrap();
    bevy::tasks::block_on(async {
        for (name, expected) in snapshot().files {
            let mut reader = source
                .reader()
                .read(Path::new(name.strip_prefix("assets/").unwrap()))
                .await
                .unwrap();
            let mut actual = Vec::new();
            reader.read_to_end(&mut actual).await.unwrap();
            assert_eq!(actual, expected);
        }
        assert!(source
            .reader()
            .read(Path::new("worlds/combat_test.toml"))
            .await
            .is_err());
    });
    let mut status = TestStatus::starting(&launch);
    status.starting = false;
    write_status(&status);
    let mut input = BufReader::new(std::io::stdin());
    while let Some(line) = bounded_line(&mut input).unwrap() {
        let record =
            crate::core::codec::from_json::<crate::workshop::test_protocol::ControlRecord>(&line)
                .unwrap();
        status.acknowledged = record.id;
        if matches!(record.control, TestControl::Pause {}) {
            status.paused = true;
        }
        if matches!(record.control, TestControl::Stop {}) {
            status.running = false;
        }
        write_status(&status);
        if !status.running {
            break;
        }
    }
}
