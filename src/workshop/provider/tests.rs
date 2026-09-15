use super::*;

struct Fixture {
    directory: PathBuf,
    root: PathBuf,
    recovery: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let directory =
            std::env::temp_dir().join(format!("phoenix-workshop-{}", uuid::Uuid::new_v4()));
        let root = directory.join("project");
        let recovery = directory.join("private");
        fs::create_dir_all(root.join("assets/worlds")).unwrap();
        fs::write(root.join("assets/scenarios.toml"), b"[content]\nid='phoenix-base'\nepoch=1\n[[scenario]]\nid='test'\nworld='assets/worlds/test.toml'\n").unwrap();
        fs::write(
            root.join("assets/worlds/test.toml"),
            b"# Keep this\r\n[global]\r\ntitle='Test' # keep tail\r\n",
        )
        .unwrap();
        Self {
            directory,
            root,
            recovery,
        }
    }
    fn open(&self) -> NativeWorkshopProvider {
        NativeWorkshopProvider::open(
            WorkspaceKind::Project,
            &self.root,
            &self.recovery,
            WorkshopDependencies::default(),
        )
        .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

#[test]
fn dependencies_are_exposed_as_read_only_text_without_editable_assets() {
    let fixture = Fixture::new();
    let dependencies = WorkshopDependencies {
        base_files: BTreeMap::from([("assets/base.toml".into(), "immutable = true\n".into())]),
        base_assets: BTreeMap::from([("assets/base.glb".into(), vec![1, 2, 3])]),
        packs: vec![super::super::WorkshopDependencyPack {
            id: "other".into(),
            manifest_toml: "# exact\r\n[pack]\r\nid='other'\r\n".into(),
            files: BTreeMap::from([("assets/other.toml".into(), "value = 1\n".into())]),
            assets: BTreeMap::from([("assets/other.glb".into(), vec![4, 5, 6])]),
        }],
    };
    let mut provider = NativeWorkshopProvider::open(
        WorkspaceKind::Project,
        &fixture.root,
        &fixture.recovery,
        dependencies,
    )
    .unwrap();

    let json = provider.handle_json(r#"{"id":1,"op":"load-dependencies"}"#);
    assert!(json.contains(r#""status":"dependencies""#), "{json}");
    assert!(json.contains("assets/base.toml"));
    assert!(json.contains("assets/other.toml"));
    assert!(json.contains("\"manifest_toml\":\"# exact\\r\\n[pack]\\r\\nid='other'\\r\\n\""));
    assert!(!json.contains("base.glb"));
    assert!(!json.contains("other.glb"));
}

#[test]
fn private_json_bridge_loads_exact_source_and_roundtrips_a_runtime_validated_save() {
    let fixture = Fixture::new();
    let mut provider = fixture.open();
    let reply = provider.handle_json(r#"{"id":3,"op":"load"}"#);
    assert!(reply.contains("\"status\":\"loaded\""), "{reply}");
    assert!(reply.contains("\"id\":3"), "{reply}");
    assert!(matches!(
        crate::core::codec::decode_workshop_request(
            r#"{"id":9,"op":"save","files":{},"expected_revision":"test"}"#
        )
        .unwrap()
        .operation,
        Operation::Save { .. }
    ));
    for refused in [
        r#"{"id":4,"op":"load","root":"elsewhere"}"#,
        r#"{"id":4,"op":"validate","files":{},"root":"elsewhere"}"#,
        r#"{"op":"load"}"#,
        r#"{"id":-1,"op":"load"}"#,
        r#"{"id":4,"op":"unknown"}"#,
    ] {
        assert!(
            crate::core::codec::decode_workshop_request(refused).is_err(),
            "{refused}"
        );
    }
    let mut files = provider.baseline.clone();
    let original = files["assets/worlds/test.toml"].clone();
    files.insert(
        "assets/worlds/test.toml".into(),
        original
            .iter()
            .copied()
            .chain(b"# edited\r\n".iter().copied())
            .collect(),
    );
    fs::write(fixture.root.join("README.md"), "Unrelated project document").unwrap();
    let result = provider
        .apply(Operation::Save {
            files: files.clone(),
            expected_revision: provider.revision.clone(),
        })
        .unwrap();
    assert!(matches!(result, Response::Saved { .. }), "{result:?}");
    assert_eq!(
        fs::read(fixture.root.join("assets/worlds/test.toml")).unwrap(),
        files["assets/worlds/test.toml"]
    );
    assert_eq!(
        fs::read_to_string(fixture.root.join("README.md")).unwrap(),
        "Unrelated project document"
    );
    assert!(!provider.private.join("transaction.json").exists());
    assert!(provider
        .handle_json(r#"{"id":4,"op":"load","root":"elsewhere"}"#)
        .contains("refused"));
}

#[test]
fn runtime_script_refusal_and_external_edits_preserve_disk_and_provider_baseline() {
    let fixture = Fixture::new();
    let mut provider = fixture.open();
    let original = provider.baseline.clone();
    let mut files = original.clone();
    files.insert(
        "assets/worlds/test.toml".into(),
        b"script='bad.rhai'\n[global]\n".to_vec(),
    );
    files.insert("assets/worlds/bad.rhai".into(), b"fn broken( {".to_vec());
    assert!(matches!(
        provider
            .apply(Operation::Save {
                files,
                expected_revision: provider.revision.clone()
            })
            .unwrap(),
        Response::Refused {
            report: Some(_),
            ..
        }
    ));
    assert_eq!(provider.read_files().unwrap(), original);
    fs::write(
        fixture.root.join("assets/worlds/test.toml"),
        "# external\n[global]\n",
    )
    .unwrap();
    assert!(provider
        .apply(Operation::Save {
            files: original.clone(),
            expected_revision: provider.revision.clone()
        })
        .unwrap_err()
        .contains("changed on disk"));
    assert_eq!(provider.baseline, original);
    assert_eq!(
        fs::read_to_string(fixture.root.join("assets/worlds/test.toml")).unwrap(),
        "# external\n[global]\n"
    );
}

#[test]
fn selected_root_refuses_escape_aliases_and_broken_deletions_before_any_write() {
    let fixture = Fixture::new();
    let mut provider = fixture.open();
    for path in [
        "../outside.toml",
        "assets/../outside.toml",
        "C:/outside.toml",
        "assets/worlds/test.toml:stream",
        "assets/worlds/con.toml",
    ] {
        let mut files = provider.baseline.clone();
        files.insert(path.into(), Vec::new());
        assert!(
            provider
                .apply(Operation::Save {
                    files,
                    expected_revision: provider.revision.clone()
                })
                .is_err(),
            "{path}"
        );
    }
    let mut aliases = provider.baseline.clone();
    aliases.insert("assets/worlds/TEST.toml".into(), b"[global]".to_vec());
    assert!(provider
        .check_files(&aliases)
        .unwrap_err()
        .contains("Duplicate"));
    let mut missing = provider.baseline.clone();
    missing.remove("assets/worlds/test.toml");
    assert!(matches!(
        provider
            .apply(Operation::Save {
                files: missing,
                expected_revision: provider.revision.clone()
            })
            .unwrap(),
        Response::Refused {
            report: Some(_),
            ..
        }
    ));
}

#[test]
fn private_draft_survives_reopen_and_its_old_revision_cannot_overwrite_later_disk_edits() {
    let fixture = Fixture::new();
    let mut provider = fixture.open();
    assert!(NativeWorkshopProvider::open(
        WorkspaceKind::Project,
        &fixture.root,
        &fixture.recovery,
        WorkshopDependencies::default()
    )
    .is_err());
    let old_revision = provider.revision.clone();
    provider
        .apply(Operation::RecoverySave {
            record: "invalid source and chronological history".into(),
            expected_revision: old_revision.clone(),
        })
        .unwrap();
    drop(provider);
    fs::write(
        fixture.root.join("assets/worlds/test.toml"),
        "# outside edit\n[global]\n",
    )
    .unwrap();
    let mut reopened = fixture.open();
    let Response::Recovery {
        recovery: Some(record),
    } = reopened.apply(Operation::RecoveryLoad).unwrap()
    else {
        panic!("missing draft")
    };
    assert_eq!(record.revision, old_revision);
    assert_eq!(record.record, "invalid source and chronological history");
    assert!(reopened
        .apply(Operation::Save {
            files: reopened.baseline.clone(),
            expected_revision: old_revision
        })
        .is_err());
    reopened.apply(Operation::RecoveryClear).unwrap();
    assert!(matches!(
        reopened.apply(Operation::RecoveryLoad).unwrap(),
        Response::Recovery { recovery: None }
    ));
}

#[test]
fn crash_between_replacements_finishes_the_exact_validated_transaction_on_reopen() {
    let fixture = Fixture::new();
    let provider = fixture.open();
    let first = "assets/worlds/test.toml".to_string();
    let second = "assets/worlds/extra.toml".to_string();
    let transaction = Transaction {
        root: provider.root.to_string_lossy().into_owned(),
        before: BTreeMap::from([
            (first.clone(), provider.baseline.get(&first).cloned()),
            (second.clone(), None),
        ]),
        after: BTreeMap::from([
            (first.clone(), Some(b"# saved\n[global]\n".to_vec())),
            (second.clone(), Some(b"[global]\n".to_vec())),
        ]),
    };
    atomic_write(
        &provider.private.join("transaction.json"),
        crate::core::codec::encode_workshop_transaction(&transaction)
            .unwrap()
            .as_bytes(),
    )
    .unwrap();
    fs::write(
        fixture.root.join(&first),
        transaction.after[&first].as_ref().unwrap(),
    )
    .unwrap();
    drop(provider);
    let reopened = fixture.open();
    assert_eq!(
        &reopened.baseline[&first],
        transaction.after[&first].as_ref().unwrap()
    );
    assert_eq!(
        &reopened.baseline[&second],
        transaction.after[&second].as_ref().unwrap()
    );
    assert!(!reopened.private.join("transaction.json").exists());
}

#[test]
fn project_binary_members_are_exact() {
    for suffix in ["glb", "bin"] {
        let fixture = Fixture::new();
        fs::create_dir_all(fixture.root.join("assets/models")).unwrap();
        let bytes = if suffix == "glb" {
            include_bytes!("../../../assets/models/alliance_courier_recreated_lod2.glb").to_vec()
        } else {
            vec![0, 255, 13, 10, 128, 10]
        };
        fs::write(
            fixture.root.join(format!("assets/models/test.{suffix}")),
            &bytes,
        )
        .unwrap();
        let mut provider = fixture.open();
        assert_eq!(
            provider.baseline[&format!("assets/models/test.{suffix}")],
            bytes
        );
        let mut files = provider.baseline.clone();
        files.insert(format!("assets/models/new.{suffix}"), bytes.clone());
        assert!(matches!(
            provider
                .apply(Operation::Save {
                    files,
                    expected_revision: provider.revision.clone()
                })
                .unwrap(),
            Response::Saved { .. }
        ));
        assert_eq!(
            fs::read(fixture.root.join(format!("assets/models/new.{suffix}"))).unwrap(),
            bytes
        );
        drop(provider);
        assert_eq!(
            fixture.open().baseline[&format!("assets/models/new.{suffix}")],
            bytes
        );
        assert!(allowed_path(
            WorkspaceKind::Mod,
            &format!("assets/models/test.{suffix}")
        ));
        assert!(allowed_path(
            WorkspaceKind::Project,
            "assets/sounds/exploration.mp3"
        ));
    }
    assert!(!allowed_path(
        WorkspaceKind::Project,
        "assets/worlds/data.bin"
    ));
}

#[test]
fn compact_native_versions_survive_save_undo_and_reopen_without_sending_asset_bytes() {
    let fixture = Fixture::new();
    // Opaque external buffer bytes exercise chunking without claiming to be decoded audio.
    let asset = "assets/models/test.bin";
    fs::create_dir_all(fixture.root.join("assets/models")).unwrap();
    let original = vec![0xff; 150_000];
    fs::write(fixture.root.join(asset), &original).unwrap();
    let mut provider = fixture.open();
    let json = provider.handle_json(r#"{"id":1,"op":"load-sources"}"#);
    assert!(json.contains("\"status\":\"sources\""), "{json}");
    assert!(
        json.len() < 10_000,
        "large assets must not become JSON integer arrays"
    );
    assert!(crate::core::codec::decode_workshop_request(
        r#"{"id":1,"op":"load-sources","root":"elsewhere"}"#
    )
    .is_err());
    let Response::Sources { files: before, .. } = provider.apply(Operation::LoadSources).unwrap()
    else {
        panic!("sources");
    };
    let assets::Source::Asset(old) = &before[asset] else {
        panic!("reference");
    };
    let Response::AssetUpload { token } = provider
        .apply(Operation::AssetBegin { length: 150_003 })
        .unwrap()
    else {
        panic!("upload");
    };
    let replacement: Vec<u8> = (0..150_003).map(|index| (index % 251) as u8).collect();
    for (index, chunk) in replacement.chunks(assets::CHUNK_BYTES).enumerate() {
        provider
            .apply(Operation::AssetChunk {
                token: token.clone(),
                offset: index * assets::CHUNK_BYTES,
                bytes: chunk.to_vec(),
            })
            .unwrap();
    }
    let Response::AssetStored { reference } =
        provider.apply(Operation::AssetFinish { token }).unwrap()
    else {
        panic!("stored");
    };
    let mut after = before.clone();
    after.insert(asset.into(), assets::Source::Asset(reference.clone()));
    assert!(matches!(
        provider
            .apply(Operation::SaveSources {
                files: after,
                expected_revision: provider.revision.clone()
            })
            .unwrap(),
        Response::Saved { .. }
    ));
    assert_eq!(fs::read(fixture.root.join(asset)).unwrap(), replacement);
    drop(provider);
    let mut reopened = fixture.open();
    let mut read = Vec::new();
    while read.len() < reference.length {
        let Response::AssetChunk { bytes } = reopened
            .apply(Operation::AssetRead {
                reference: reference.clone(),
                offset: read.len(),
            })
            .unwrap()
        else {
            panic!("chunk");
        };
        assert!(bytes.len() <= assets::CHUNK_BYTES);
        read.extend(bytes);
    }
    assert_eq!(read, replacement);
    // An undo entry references the ORIGINAL version after a successful save,
    // and still resolves that version after a native process restart.
    assert_ne!(old, &reference);
    assert!(matches!(
        reopened
            .apply(Operation::SaveSources {
                files: before,
                expected_revision: reopened.revision.clone()
            })
            .unwrap(),
        Response::Saved { .. }
    ));
    assert_eq!(fs::read(fixture.root.join(asset)).unwrap(), original);
}

#[test]
fn native_asset_chunks_and_versions_refuse_corruption_without_changing_the_draft_source() {
    let fixture = Fixture::new();
    let mut provider = fixture.open();
    let before = provider.baseline.clone();
    let Response::AssetUpload { token } =
        provider.apply(Operation::AssetBegin { length: 3 }).unwrap()
    else {
        panic!("upload");
    };
    for (candidate, offset, bytes) in [
        ("wrong".into(), 0, vec![1]),
        (token.clone(), 1, vec![1]),
        (token.clone(), 0, vec![1; assets::CHUNK_BYTES + 1]),
    ] {
        assert!(provider
            .apply(Operation::AssetChunk {
                token: candidate,
                offset,
                bytes
            })
            .is_err());
    }
    assert!(provider
        .apply(Operation::AssetFinish {
            token: token.clone()
        })
        .is_err());
    provider
        .apply(Operation::AssetChunk {
            token: token.clone(),
            offset: 0,
            bytes: vec![0, 255, 10],
        })
        .unwrap();
    let Response::AssetStored { reference } =
        provider.apply(Operation::AssetFinish { token }).unwrap()
    else {
        panic!("stored");
    };
    assert!(provider
        .apply(Operation::AssetRead {
            reference: assets::AssetReference {
                asset: "../draft.json".into(),
                length: 3
            },
            offset: 0
        })
        .is_err());
    let mut files = provider.assets.compact(&before).unwrap();
    files.insert(
        "assets/worlds/test.toml".into(),
        assets::Source::Asset(reference.clone()),
    );
    assert!(provider
        .apply(Operation::SaveSources {
            files,
            expected_revision: provider.revision.clone()
        })
        .is_err());
    fs::write(
        provider
            .private
            .join("assets")
            .join(format!("{}.blob", reference.asset)),
        [1, 2, 3],
    )
    .unwrap();
    let mut files = provider.assets.compact(&before).unwrap();
    files.insert(
        "assets/sounds/test.mp3".into(),
        assets::Source::Asset(reference),
    );
    assert!(provider
        .apply(Operation::SaveSources {
            files,
            expected_revision: provider.revision.clone()
        })
        .is_err());
    assert_eq!(provider.read_files().unwrap(), before);
}

#[test]
fn mod_workspace_saves_exact_sources_and_retains_binary_members_while_runtime_refuses_them() {
    let fixture = Fixture::new();
    let files = crate::world::mod_pack::read_store_zip(include_bytes!(
        "../../../tests/fixtures/mod-packs/valid-v1.zip"
    ))
    .unwrap();
    for (path, text) in &files {
        let target = fixture.root.join(path);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(target, text).unwrap();
    }
    let dependencies = WorkshopDependencies {
        base_files: BTreeMap::from([(
            "assets/scenarios.toml".into(),
            "[content]\nid='phoenix-base'\nepoch=1\n".into(),
        )]),
        packs: Vec::new(),
        base_assets: BTreeMap::new(),
    };
    let open = || {
        NativeWorkshopProvider::open(
            WorkspaceKind::Mod,
            &fixture.root,
            &fixture.recovery,
            dependencies.clone(),
        )
        .unwrap()
    };
    let mut provider = open();
    let mut edited = provider.baseline.clone();
    edited
        .get_mut("scenarios.toml")
        .unwrap()
        .extend(b"# exact new comment\r\n");
    assert!(matches!(
        provider
            .apply(Operation::Save {
                files: edited.clone(),
                expected_revision: provider.revision.clone(),
            })
            .unwrap(),
        Response::Saved { .. }
    ));
    drop(provider);
    assert_eq!(open().baseline, edited);
    let path = "assets/sounds/test.mp3";
    let bytes = vec![0, 255, 13, 10, 128];
    fs::create_dir_all(fixture.root.join("assets/sounds")).unwrap();
    fs::write(fixture.root.join(path), &bytes).unwrap();
    let mut provider = open();
    assert_eq!(provider.baseline[path], bytes);
    let baseline = provider.baseline.clone();
    assert!(matches!(
        provider
            .apply(Operation::Save {
                files: baseline.clone(),
                expected_revision: provider.revision.clone(),
            })
            .unwrap(),
        Response::Refused {
            report: Some(_),
            ..
        }
    ));
    assert_eq!(provider.read_files().unwrap(), baseline);
}

#[test]
fn adding_saving_undoing_and_saving_removes_only_the_added_file_and_survives_reopen() {
    let fixture = Fixture::new();
    let mut provider = fixture.open();
    let original = provider.baseline.clone();
    let mut added = original.clone();
    added.insert("assets/worlds/added.toml".into(), b"[global]\n".to_vec());
    assert!(matches!(
        provider
            .apply(Operation::Save {
                files: added,
                expected_revision: provider.revision.clone()
            })
            .unwrap(),
        Response::Saved { .. }
    ));
    assert!(fixture.root.join("assets/worlds/added.toml").exists());
    assert!(matches!(
        provider
            .apply(Operation::Save {
                files: original.clone(),
                expected_revision: provider.revision.clone()
            })
            .unwrap(),
        Response::Saved { .. }
    ));
    drop(provider);
    assert_eq!(fixture.open().baseline, original);
    assert!(!fixture.root.join("assets/worlds/added.toml").exists());
}

#[test]
fn deletion_recovery_is_idempotent_and_preserves_an_external_replacement() {
    for conflict in [false, true] {
        let fixture = Fixture::new();
        let path = "assets/worlds/added.toml";
        let original = b"[global]\n".to_vec();
        fs::write(fixture.root.join(path), &original).unwrap();
        let provider = fixture.open();
        let transaction = Transaction {
            root: provider.root.to_string_lossy().into_owned(),
            before: BTreeMap::from([(path.into(), Some(original))]),
            after: BTreeMap::from([(path.into(), None)]),
        };
        atomic_write(
            &provider.private.join("transaction.json"),
            crate::core::codec::encode_workshop_transaction(&transaction)
                .unwrap()
                .as_bytes(),
        )
        .unwrap();
        if conflict {
            fs::write(fixture.root.join(path), "# external\n[global]\n").unwrap();
        } else {
            fs::remove_file(fixture.root.join(path)).unwrap();
        } // deleted before hard shutdown
        drop(provider);
        let reopened = NativeWorkshopProvider::open(
            WorkspaceKind::Project,
            &fixture.root,
            &fixture.recovery,
            WorkshopDependencies::default(),
        );
        if conflict {
            assert!(reopened.err().unwrap().contains("external edit"));
            assert_eq!(
                fs::read_to_string(fixture.root.join(path)).unwrap(),
                "# external\n[global]\n"
            );
        } else {
            assert!(!reopened.unwrap().baseline.contains_key(path));
        }
    }
}

#[test]
fn disposable_test_freezes_validated_unsaved_sources_without_touching_the_selected_root() {
    let fixture = Fixture::new();
    fs::create_dir_all(fixture.root.join("assets/entities")).unwrap();
    fs::create_dir_all(fixture.root.join("assets/models")).unwrap();
    fs::create_dir_all(fixture.root.join("assets/shaders")).unwrap();
    fs::write(
        fixture.root.join("assets/shaders/test.wgsl"),
        b"// captured shader\r\n",
    )
    .unwrap();
    fs::write(fixture.root.join("assets/entities/test.toml"), TEST_HULL).unwrap();
    let model = include_bytes!("../../../assets/gui/dpad-button-idle.png").to_vec();
    fs::write(fixture.root.join("assets/models/test.png"), &model).unwrap();
    let provider = fixture.open();
    let mut sources = provider.assets.compact(&provider.baseline).unwrap();
    assert!(!sources.contains_key("assets/shaders/test.wgsl"));
    fs::write(
        fixture.root.join("assets/shaders/test.wgsl"),
        b"// later external edit",
    )
    .unwrap();
    let authored = "# unsaved exact comment\r\n[global]\r\ntitle='Unsaved Test'\r\n";
    sources.insert(
        "assets/worlds/test.toml".into(),
        assets::Source::Text(authored.into()),
    );
    // Later disk edits are irrelevant to the immutable draft supplied to Test.
    fs::write(
        fixture.root.join("assets/models/test.png"),
        b"external replacement",
    )
    .unwrap();
    let selection = test_snapshot::TestSelection {
        world: "assets/worlds/test.toml".into(),
        ship: "assets/entities/test.toml".into(),
        seed: 42,
    };
    let snapshot = provider
        .prepare_test(sources.clone(), selection.clone())
        .unwrap();
    assert_eq!(snapshot.files[&selection.world], authored.as_bytes());
    assert_eq!(snapshot.files["assets/models/test.png"], model);
    assert_eq!(
        snapshot.files["assets/shaders/test.wgsl"],
        b"// captured shader\r\n"
    );
    assert_eq!(snapshot.selection.seed, 42);
    assert_eq!(
        fs::read(fixture.root.join(&selection.world)).unwrap(),
        provider.baseline[&selection.world]
    );
    assert_eq!(
        fs::read(fixture.root.join("assets/models/test.png")).unwrap(),
        b"external replacement"
    );
    let repeated = provider
        .prepare_test(sources.clone(), selection.clone())
        .unwrap();
    assert_eq!(snapshot.revision, repeated.revision);
    // This is a valid NPC entity but both native and browser player boot need
    // a ship configuration. Refuse it before creating any disposable child.
    let mut no_ship_config = sources.clone();
    no_ship_config.insert(
        selection.ship.clone(),
        assets::Source::Text("class='lancer'\n".into()),
    );
    let Err(Response::Refused {
        report: Some(report),
        ..
    }) = provider.prepare_test(no_ship_config, selection.clone())
    else {
        panic!("Test must refuse a hull the runtime cannot select")
    };
    assert!(report
        .findings
        .iter()
        .any(|finding| finding.file == selection.ship
            && finding.message.contains("ship configuration")));
    sources.insert(
        selection.world.clone(),
        assets::Source::Text("[global\n".into()),
    );
    assert!(matches!(
        provider.prepare_test(sources, selection),
        Err(Response::Refused {
            report: Some(_),
            ..
        })
    ));
    assert!(!fixture.recovery.join("test").exists());
}

const TEST_HULL: &str = "class='lancer'\nname='Test hull'\n\
[[station]]\nid='captain'\nname='Captain'\ndescription='Test station'\nrank='captain'\n\
[[system]]\nid='boost'\nkind='helm_boost'\nstation='captain'\n";

#[test]
fn test_catalog_resolves_read_only_hulls_and_unsaved_include_edits_without_binary_materialization()
{
    let fixture = Fixture::new();
    let dependencies = WorkshopDependencies {
        base_files: BTreeMap::from([
            ("assets/entities/base.toml".into(), TEST_HULL.into()),
            (
                "assets/entities/fragment.toml".into(),
                "name='Partial'\n".into(),
            ),
            ("assets/worlds/base.toml".into(), "[global]\n".into()),
            (
                "assets/entities/uncrewed.toml".into(),
                "class='lancer'\n".into(),
            ),
        ]),
        ..Default::default()
    };
    let provider = NativeWorkshopProvider::open(
        WorkspaceKind::Mod,
        &fixture.root,
        &fixture.recovery,
        dependencies,
    )
    .unwrap();
    let mut draft = BTreeMap::from([
        (
            "assets/entities/authored.toml".into(),
            "includes=['base.toml']\nname='Unsaved'\n".into(),
        ),
        ("assets/worlds/authored.toml".into(), "[global]\n".into()),
    ]);
    let catalog = provider.test_catalog(draft.clone()).unwrap();
    assert_eq!(
        catalog.worlds,
        ["assets/worlds/authored.toml", "assets/worlds/base.toml"]
    );
    assert_eq!(
        catalog.ships,
        ["assets/entities/authored.toml", "assets/entities/base.toml"]
    );
    // An invalid replacement shadows the base; it must not silently offer the
    // old cached hull or an includer which cannot compose from this draft.
    draft.insert("assets/entities/base.toml".into(), "[invalid".into());
    assert!(provider.test_catalog(draft).unwrap().ships.is_empty());
    assert!(provider
        .test_catalog(BTreeMap::from([("../elsewhere.toml".into(), "".into())]))
        .is_err());
}
