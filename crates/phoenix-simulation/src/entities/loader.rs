// Pure module: resolve an WorldEntity into a concrete EntityConfig.
// No Bevy dependency — fully unit-testable on native.

use crate::entities::config::EntityConfig;
#[cfg(test)]
use crate::world::config::WorldEntity;

// A cache-only `resolve_entity(&WorldEntity, &ConfigCache)` lived here. It is
// gone (issue #973 review, F5): after #973 routed all four spawn sites through
// `resolve_entity_via`, every remaining caller was a test, so it pinned
// behaviour nothing performed — and it was the *narrower* of the two lookups,
// the very narrowness that let a spawn silently drop an entity the world
// validator had passed. Leaving a `pub fn` around whose own doc warns callers
// off it is an invitation for the next spawn site to reach for the wrong one.
// Its tests now exercise `resolve_entity_via` behind a host that can serve
// nothing, which is the same "cache alone" question asked of the function
// production actually calls.

// `assign_uuid()` lived here and returned `Uuid::new_v4().to_string()`. It is
// gone (issue #907): a spawned entity's id is now minted from the tick-scoped
// counter in `crate::world_id`, which is the crate's single chokepoint for
// simulation identity. `Uuid::new_v4` is banned in sim code by clippy.toml.

// ── Template resolution ──────────────────────────────────────────────────────

/// Native loader: reads the template TOML straight off the filesystem.
///
/// Resolves the template's `includes` closure first (issue #869), so an
/// on-demand native template load sees exactly the same fully-composed document
/// the browser preload assembles. A composition failure — cycle, missing
/// fragment, invalid resolved template — collapses to `None` here for the same
/// reason a parse error does: this module must not log, and the caller knows
/// which spawn request asked. See the trait doc above.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Default, Clone, Copy)]
pub struct FsTemplateLoader;

#[cfg(not(target_arch = "wasm32"))]
impl TemplateLoader for FsTemplateLoader {
    fn load_template(&self, path: &str) -> Option<EntityConfig> {
        let resolved = crate::entities::include_resolve::resolve_from_disk(path).ok()?;
        // Issue #935: record the byte-stable composed document — the same
        // shape `config_cache::wasm_load_config` records on wasm — so an edit
        // to this template OR any fragment it includes moves the content
        // digest. Recorded here, at the loader that resolves what a spawn
        // actually consumes, rather than in `resolve_template`/`visit`
        // itself: that shared resolver is also what the headless diagnostic
        // preload (`headless::app::preload_entity_templates`) walks over
        // EVERY file in a directory, and recording there would turn the
        // digest into a repo-wide hash instead of "what this scenario used".
        crate::content_ledger::record(&resolved.path, &resolved.toml);
        let config = resolved.parse().ok()?;
        crate::entities::model_markers::record_primary_sidecar_from_fs(&config);
        Some(config)
    }

    /// The filesystem is authoritative: what it cannot serve does not exist.
    fn absence_is_final(&self) -> bool {
        true
    }
}

/// WASM loader: serves templates out of the preloaded config cache, falling
/// back to the filesystem on native.
///
/// Compiles on both targets so callers can name it unconditionally. On WASM
/// the cache is the only source (there is no filesystem); on native the cache
/// lookup always misses and the filesystem fallback does the work — which
/// makes the fallback path testable under `cargo test`.
#[derive(Debug, Default, Clone, Copy)]
pub struct WasmTemplateLoader;

impl TemplateLoader for WasmTemplateLoader {
    fn load_template(&self, path: &str) -> Option<EntityConfig> {
        // Single-path lookup, not `get_config_cache()` — the latter clones the
        // entire cache map on every call.
        if let Some(config) = crate::entities::config_cache::get_cached_entity_config(path) {
            return Some(config);
        }

        #[cfg(not(target_arch = "wasm32"))]
        {
            FsTemplateLoader.load_template(path)
        }
        #[cfg(target_arch = "wasm32")]
        {
            None
        }
    }

    /// Native: the filesystem fallback above makes absence a fact.
    ///
    /// Browser: it does not. See [`TemplateLoader::absence_is_final`] — the
    /// cache fills one delivery at a time, so an uncached template may simply
    /// be still in flight.
    fn absence_is_final(&self) -> bool {
        !cfg!(target_arch = "wasm32")
    }
}

pub use phoenix_content_preparation::entities::loader::{
    resolve_entity_via, template_is_asteroid_field, SpawnTemplateLoader,
};
#[cfg(test)]
#[path = "loader_tests.rs"]
mod tests;

#[cfg(not(target_arch = "wasm32"))]
use crate::entities::include_resolve::ParseEntityTemplate as _;

pub use phoenix_sim_gameplay::entities::loader::{apply_overrides, TemplateLoader};
