//! Browser UASTC for the approved Gas Giant base map. Native uses the original.
//! A separate asset extension keeps retries inside one Image handle, so preload
//! and the material agree even if the worker or compressed asset fails.
use bevy::{
    asset::{io::Reader, AssetLoader, LoadContext},
    image::{CompressedImageFormatSupport, CompressedImageFormats, ImageLoaderSettings, ImageType},
    prelude::*,
};

#[cfg(any(target_arch = "wasm32", test))]
pub(super) fn browser_path(path: &str, srgb: bool) -> &str {
    if srgb && path == "planets/gas_giant/surface_colour.ktx2" {
        "planets/gas_giant/surface_colour.ptex"
    } else {
        path
    }
}

pub(super) struct PlanetTexturePlugin;
impl Plugin for PlanetTexturePlugin {
    fn build(&self, _app: &mut App) {}
    fn finish(&self, app: &mut App) {
        // RenderPlugin has now published the actual enabled device features.
        let formats = app
            .world()
            .get_resource::<CompressedImageFormatSupport>()
            .map_or(CompressedImageFormats::NONE, |support| support.0);
        app.register_asset_loader(PlanetTextureLoader { formats });
    }
}

#[derive(TypePath)]
struct PlanetTextureLoader {
    formats: CompressedImageFormats,
}

#[derive(serde::Deserialize)]
struct TextureSource {
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    source: String,
    fallback: String,
}

#[cfg(any(target_arch = "wasm32", test))]
fn target(formats: CompressedImageFormats) -> &'static str {
    if formats.contains(CompressedImageFormats::ASTC_LDR) {
        "astc"
    } else if formats.contains(CompressedImageFormats::BC) {
        "bc7"
    } else if formats.contains(CompressedImageFormats::ETC2) {
        "etc2"
    } else {
        "rgba"
    }
}

impl PlanetTextureLoader {
    fn decode(
        &self,
        bytes: &[u8],
        settings: &ImageLoaderSettings,
    ) -> Result<Image, bevy::image::TextureError> {
        Image::from_buffer(
            bytes,
            ImageType::Extension("ktx2"),
            self.formats,
            settings.is_srgb,
            settings.sampler.clone(),
            settings.asset_usage,
        )
    }
}

impl AssetLoader for PlanetTextureLoader {
    type Asset = Image;
    type Settings = ImageLoaderSettings;
    type Error = BevyError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        settings: &ImageLoaderSettings,
        context: &mut LoadContext<'_>,
    ) -> Result<Image, Self::Error> {
        // Descriptor is deliberately tiny and versioned with its fallback.
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await?;
        let source: TextureSource = serde_json::from_slice(&bytes)?;
        #[cfg(target_arch = "wasm32")]
        {
            let result = async {
                let bytes = context
                    .read_asset_bytes(source.source.clone())
                    .await
                    .map_err(|e| e.to_string())?;
                let converted = transcode(&bytes, target(self.formats)).await?;
                self.decode(&converted, settings).map_err(|e| e.to_string())
            }
            .await;
            match result {
                Ok(image) => {
                    info!("Planet UASTC loaded: {:?}", image.texture_descriptor.format);
                    return Ok(image);
                }
                Err(error) => warn!("Planet UASTC fallback: {error}"),
            }
        }
        let bytes = context.read_asset_bytes(source.fallback).await?;
        Ok(self.decode(&bytes, settings)?)
    }
    fn extensions(&self) -> &[&str] {
        &["ptex"]
    }
}

#[cfg(target_arch = "wasm32")]
async fn transcode(bytes: &[u8], format: &str) -> Result<Vec<u8>, String> {
    use wasm_bindgen::prelude::*;
    // Resolve against the page base, including deployments under a subdirectory.
    #[wasm_bindgen(
        inline_js = "export async function phoenixTranscode(bytes, target) { const m = await import(new URL('assets/texture-codecs/uastc.js', document.baseURI).href); return m.transcode(bytes, target); }"
    )]
    extern "C" {
        #[wasm_bindgen(catch)]
        async fn phoenixTranscode(
            bytes: js_sys::Uint8Array,
            target: &str,
        ) -> Result<JsValue, JsValue>;
    }
    let result = phoenixTranscode(js_sys::Uint8Array::from(bytes), format)
        .await
        .map_err(|error| format!("{error:?}"))?;
    Ok(js_sys::Uint8Array::new(&result).to_vec())
}

#[cfg(test)]
mod tests {
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
        let handle: Handle<Image> = app
            .world()
            .resource::<AssetServer>()
            .load("planets/gas_giant/surface_colour.ptex");
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

    #[test]
    fn only_the_approved_srgb_base_uses_uastc() {
        assert!(browser_path("planets/gas_giant/surface_colour.ktx2", true).ends_with(".ptex"));
        for (path, srgb) in [
            ("planets/ice/surface_colour.ktx2", true),
            ("planets/gas_giant/surface_colour.ktx2", false),
            ("planets/gas_giant/surface_normal.ktx2", false),
        ] {
            assert_eq!(browser_path(path, srgb), path);
        }
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
}
