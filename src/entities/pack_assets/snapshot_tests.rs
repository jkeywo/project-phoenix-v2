use super::*;

#[test]
fn snapshot_sources_keep_exact_bytes_and_refuse_uncaptured_live_content() {
    let _guard = crate::entities::config_cache::overlay_test_guard();
    let captured = b"// captured shader\r\n".to_vec();
    let mut app = App::new();
    register(
        &mut app,
        Arc::new(BTreeMap::from([
            (
                "assets/shaders/reference_grid.wgsl".into(),
                Arc::from(captured.clone()),
            ),
            (
                "assets/models/local/buffer.bin".into(),
                Arc::from([0u8, 255, 13, 10]),
            ),
        ])),
    );
    // The ordinary boot registration must preserve explicitly selected
    // snapshot readers rather than reintroducing the live fallback.
    super::super::register(&mut app);
    app.add_plugins((
        bevy::app::TaskPoolPlugin::default(),
        bevy::asset::AssetPlugin::default(),
    ));
    let server = app.world().resource::<AssetServer>();
    for (source_id, prefix) in [
        (AssetSourceId::Default, String::new()),
        (
            AssetSourceId::from(versioned::SOURCE),
            format!("{}/", mod_pack_revision()),
        ),
    ] {
        let source = server.get_source(source_id).unwrap();
        let reader = source.reader();
        bevy::tasks::block_on(async {
            let shader_path = format!("{prefix}shaders/reference_grid.wgsl");
            let mut stream = reader.read(Path::new(&shader_path)).await.unwrap();
            let mut bytes = Vec::new();
            stream.read_to_end(&mut bytes).await.unwrap();
            assert_eq!(bytes, captured);
            assert!(
                matches!(
                    reader
                        .read(Path::new(&format!(
                            "{prefix}entities/alliance_cruiser.toml"
                        )))
                        .await,
                    Err(AssetReaderError::NotFound(_))
                ),
                "shipped disk content must remain absent"
            );
            assert!(reader.read_meta(Path::new(&shader_path)).await.is_err());
            let mut directory = reader
                .read_directory(Path::new(&format!("{prefix}models")))
                .await
                .unwrap();
            assert_eq!(
                directory.next().await,
                Some(Path::new(&format!("{prefix}models")).join("local"))
            );
            assert!(directory.next().await.is_none());
            assert!(reader
                .is_directory(Path::new(&format!("{prefix}models/local")))
                .await
                .unwrap());
            assert!(reader
                .read(Path::new(&format!("{prefix}../outside")))
                .await
                .is_err());
        });
    }
    let versioned = server.get_source(versioned::SOURCE).unwrap();
    assert!(bevy::tasks::block_on(versioned.reader().read(Path::new(
        "18446744073709551615/shaders/reference_grid.wgsl"
    )))
    .is_err());
}
