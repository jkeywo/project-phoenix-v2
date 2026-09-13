//! Native content resolution; decoding is shared with offline Workshop.
pub use crate::audio_decode::{decode, Pcm};
use std::sync::Arc;

pub fn read(path: &str) -> Result<Arc<Pcm>, String> {
    let path = std::path::Path::new(path);
    // Authored audio is content, never an arbitrary host-local path.
    if path.is_absolute()
        || path
            .components()
            .any(|part| !matches!(part, std::path::Component::Normal(_)))
        || !path.starts_with("assets")
    {
        return Err("Audio must name a content asset under assets/".into());
    }
    let key = path.to_string_lossy().replace('\\', "/");
    let bytes = match crate::entities::config_cache::mod_pack_asset(&key) {
        Some(bytes) => bytes.to_vec(),
        None => std::fs::read(path).map_err(|e| e.to_string())?,
    };
    decode(
        bytes,
        path.extension()
            .and_then(|x| x.to_str())
            .unwrap_or_default(),
    )
}
