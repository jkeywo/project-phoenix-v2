use super::*;
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn native_descriptor_load_resolves_the_original_image() {
    use bevy::asset::{AssetMetaCheck, LoadState};
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        AssetPlugin {
            file_path: format!("{}/assets", env!("CARGO_MANIFEST_DIR")),
            meta_check: AssetMetaCheck::Never,
            ..default()
        },
    ))
    .init_asset::<Image>()
    .add_plugins(PlanetTexturePlugin);
    app.finish();
    app.cleanup();
    for descriptor in [
        "planets/gas_giant/surface_colour.ptex",
        "planets/ice_moon/surface_colour.ptex",
        "planets/ecumenopolis/city_albedo.ptex",
    ] {
        let handle: Handle<Image> = app.world().resource::<AssetServer>().load(descriptor);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        loop {
            app.update();
            if let Some(image) = app.world().resource::<Assets<Image>>().get(&handle) {
                assert_eq!(image.width(), 4096);
                assert_eq!(image.height(), 2048);
                assert_eq!(image.texture_descriptor.mip_level_count, 13);
                assert_eq!(
                    image.texture_descriptor.format,
                    bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb
                );
                break;
            }
            let state = app
                .world()
                .resource::<AssetServer>()
                .load_state(handle.id());
            assert!(!matches!(state, LoadState::Failed(_)), "{state:?}");
            assert!(
                std::time::Instant::now() < deadline,
                "texture load timed out"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
}

#[test]
fn only_the_approved_srgb_base_uses_uastc() {
    let _lock = crate::entities::config_cache::overlay_test_guard();
    for path in [
        "planets/gas_giant/surface_colour.ktx2",
        "planets/ice_moon/surface_colour.ktx2",
        "planets/ecumenopolis/city_albedo.ktx2",
    ] {
        assert!(browser_path(path, true).ends_with(".ptex"));
        assert_eq!(browser_path(path, false), path);
    }
    for (path, srgb) in [
        ("planets/ice/surface_colour.ktx2", true),
        ("planets/gas_giant/surface_colour.ktx2", false),
        ("planets/gas_giant/surface_normal.ktx2", false),
    ] {
        assert_eq!(browser_path(path, srgb), path);
    }
}
#[test]
fn an_overridden_original_does_not_use_the_shipped_compressed_sibling() {
    use crate::entities::config_cache::{push_mod_pack, remove_mod_pack, ActivePack};
    let _lock = crate::entities::config_cache::overlay_test_guard();
    let path = "planets/gas_giant/surface_colour.ktx2";
    push_mod_pack(ActivePack {
        id: "planet-original-replacement".into(),
        assets: [(format!("assets/{path}"), std::sync::Arc::from([1u8, 2, 3]))].into(),
        ..default()
    });
    assert_eq!(browser_path(path, true), path);
    remove_mod_pack("planet-original-replacement");
    assert!(browser_path(path, true).ends_with(".ptex"));
}
#[test]
fn unavailable_device_features_use_rgba() {
    assert_eq!(target(CompressedImageFormats::NONE), "rgba");
    assert_eq!(target(CompressedImageFormats::BC), "bc7");
    assert_eq!(target(CompressedImageFormats::ETC2), "etc2");
    assert_eq!(
        target(CompressedImageFormats::ASTC_LDR | CompressedImageFormats::BC),
        "astc"
    );
}
