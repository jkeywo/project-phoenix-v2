pub use phoenix_content_preparation::world::load::*;
/// Native reader with the session mod-pack overlay in front of the filesystem
/// (issue #1366).
///
/// The native twin of what [`WasmReader`] already does: the browser resolves a
/// world through `config_cache::resolved_world_source`, which consults the
/// winning pack before the delivered base text, so a pack's world — or a pack's
/// override of a base world — is what actually loads. Off the browser
/// [`FsReader`] alone could never see either, because a pack's files live only in
/// [`ActivePack::files`](crate::entities::config_cache::ActivePack) and are never
/// written to the content tree: a host would offer the pack's scenario in its
/// lobby and then fail to load the world behind it.
///
/// Overlay first, disk second — the same order
/// [`FsFragmentSource`](crate::entities::include_resolve::FsFragmentSource) and
/// [`OverlayScriptResolver`](crate::entities::config_cache::OverlayScriptResolver)
/// resolve in, over the same authored repo-relative key. With no pack installed
/// it IS [`FsReader`], byte for byte, so a headless or `--world` boot is
/// unchanged.
#[cfg(not(target_arch = "wasm32"))]
pub struct OverlayFsReader;

#[cfg(not(target_arch = "wasm32"))]
impl WorldReader for OverlayFsReader {
    fn read(&self, path: &str) -> Option<String> {
        crate::entities::config_cache::mod_pack_overlay_get(path)
            .or_else(|| crate::content_fs::read_to_string(path).ok())
    }
}

/// Browser reader: resolve world TOML from the live overlay or durable
/// JS-delivered base cache, firing a fetch request when neither is present.
///
/// Mirrors `world::server::load_scenario_toml_text`'s wasm arm. Both
/// [`resolved_world_source`](crate::entities::config_cache::resolved_world_source) and
/// [`request_world_fetch`](crate::entities::config_cache::request_world_fetch) have native
/// no-op stubs, so this adapter compiles on every target (and simply reads
/// `None` off-browser) without a `cfg` gate of its own.
pub struct WasmReader;

impl WorldReader for WasmReader {
    fn read(&self, path: &str) -> Option<String> {
        crate::entities::config_cache::resolved_world_source(path).or_else(|| {
            crate::entities::config_cache::request_world_fetch(path.to_string());
            None
        })
    }
}

pub fn load(request: LoadRequest) -> Result<LoadedWorld, LoadError> {
    phoenix_content_preparation::world::load::load(
        request,
        &crate::entities::loader::WasmTemplateLoader,
        &crate::entities::include_resolve::HostFragmentSource,
    )
}
#[cfg(test)]
mod tests;
