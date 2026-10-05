//! Browser UASTC for approved planet base maps. Native uses the originals.
//! A separate asset extension keeps retries inside one Image handle, so preload
//! and the material agree even if the worker or compressed asset fails.
use bevy::{
    asset::{io::Reader, AssetLoader, LoadContext},
    image::{CompressedImageFormatSupport, CompressedImageFormats, ImageLoaderSettings, ImageType},
    prelude::*,
};

#[cfg(any(target_arch = "wasm32", test))]
pub(super) fn browser_path(path: &str, srgb: bool) -> &str {
    // The shipped compressed sibling describes the shipped original only.
    // A replacement original must not be bypassed by that unrelated sibling.
    if !srgb || super::config_cache::mod_pack_asset(&format!("assets/{path}")).is_some() {
        return path;
    }
    match path {
        "planets/gas_giant/surface_colour.ktx2" => "planets/gas_giant/surface_colour.ptex",
        "planets/ice_moon/surface_colour.ktx2" => "planets/ice_moon/surface_colour.ptex",
        "planets/ecumenopolis/city_albedo.ktx2" => "planets/ecumenopolis/city_albedo.ptex",
        _ => path,
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

pub use phoenix_model::texture::TextureSource;

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
        let source: TextureSource =
            crate::core::codec::from_json_bytes(&bytes).map_err(std::io::Error::other)?;
        #[cfg(target_arch = "wasm32")]
        {
            let result = async {
                let bytes = context
                    .read_asset_bytes(super::pack_assets::root_dependency(context, &source.source))
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
        let dependency = super::pack_assets::root_dependency(context, &source.fallback);
        let bytes = context.read_asset_bytes(dependency).await?;
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
#[path = "planet_texture_tests.rs"]
mod tests;
