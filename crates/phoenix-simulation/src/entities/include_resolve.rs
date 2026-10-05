//! Phoenix adapters over content's template composition engine.
use crate::entities::config::EntityConfig;
use crate::world::validate::WorldFinding;
pub use phoenix_content::include_resolve::*;

/// Parse composed content with Phoenix's full entity schema and validation.
pub trait ParseEntityTemplate {
    fn parse(&self) -> Result<EntityConfig, IncludeError>;
}
impl ParseEntityTemplate for ResolvedTemplate {
    fn parse(&self) -> Result<EntityConfig, IncludeError> {
        self.parse_with(EntityConfig::from_toml)
    }
}

pub fn composition_finding(path: &str, source: &dyn FragmentSource) -> Option<WorldFinding> {
    composition_finding_with(path, source, EntityConfig::from_toml)
}

/// Filesystem adapter for the pure resolver above.
///
/// One of the two I/O-touching items in this file (see also
/// [`HostFragmentSource`]), mirroring how `entity_loader::FsTemplateLoader` sits
/// beside the pure resolution in `loader.rs`. The mod-pack overlay is consulted FIRST so an uploaded pack's
/// fragment wins over the shipped one, matching how every other content channel
/// resolves an authored path (issue #760 AC2, #869 US7).
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Default, Clone, Copy)]
pub struct FsFragmentSource;

#[cfg(not(target_arch = "wasm32"))]
impl FragmentSource for FsFragmentSource {
    fn read(&self, path: &str) -> Option<String> {
        crate::entities::config_cache::mod_pack_overlay_get(path)
            .or_else(|| crate::content_fs::read_to_string(path).ok())
    }

    /// The filesystem is authoritative and the overlay is installed whole, so
    /// a fragment neither can serve genuinely does not exist.
    fn absence_is_final(&self) -> bool {
        true
    }
}

/// Resolve a template off disk (mod-pack overlay first). Native only — the
/// browser resolves out of its preload cache via [`preload_step`].
#[cfg(not(target_arch = "wasm32"))]
pub fn resolve_from_disk(path: &str) -> Result<ResolvedTemplate, IncludeError> {
    resolve_template(path, &FsFragmentSource)
}

/// Resolve a template off disk and parse it. The single native entry point for
/// "give me the `EntityConfig` this path denotes", include closure and all.
#[cfg(not(target_arch = "wasm32"))]
pub fn load_entity_config(path: &str) -> Result<EntityConfig, IncludeError> {
    resolve_from_disk(path)?.parse()
}

// ── Composition as a world finding (issue #906) ──────────────────────────────

/// The fragment source a HOST resolves against, on either target.
///
/// Mirrors [`crate::entities::loader::WasmTemplateLoader`]'s three-step lookup, one
/// layer lower down (raw text rather than parsed configs):
///
/// 1. the session mod-pack overlay, so an uploaded pack's fragment wins;
/// 2. the raw templates the host has already delivered — on WASM this is the
///    *only* source, since there is no filesystem;
/// 3. the filesystem, on native.
///
/// Compiles on both targets so callers can name it unconditionally, which is
/// what lets [`crate::world::validate::validate_composition`] carry a default
/// source without a `cfg` split at every call site.
#[derive(Debug, Default, Clone, Copy)]
pub struct HostFragmentSource;

impl FragmentSource for HostFragmentSource {
    fn read(&self, path: &str) -> Option<String> {
        if let Some(text) = crate::entities::config_cache::mod_pack_overlay_get(path) {
            return Some(text);
        }
        if let Some(text) = crate::entities::config_cache::raw_template_text(path) {
            return Some(text);
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            crate::content_fs::read_to_string(path).ok()
        }
        #[cfg(target_arch = "wasm32")]
        {
            None
        }
    }

    /// Native: the filesystem is authoritative, so absence is final.
    ///
    /// Browser: it is NOT. `RAW_TEMPLATE_TOML` fills one delivery at a time, so
    /// a fragment this source cannot serve may simply be still in flight. The
    /// initial preload waits for the queue to drain before it declares itself
    /// complete, but the runtime layer load does not: `world::server` builds the
    /// layer cache and spawns the moment the layer TOML arrives, while that
    /// layer's entity templates were only just queued. Treating that race as a
    /// composition fault would blank the entire world — permanently, since the
    /// layer is marked loaded and never retried.
    fn absence_is_final(&self) -> bool {
        !cfg!(target_arch = "wasm32")
    }
}

#[cfg(test)]
#[path = "include_resolve_tests.rs"]
mod tests;
