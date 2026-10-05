#[derive(serde::Deserialize)]
pub struct TextureSource {
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub source: String,
    pub fallback: String,
}
