// Pure module: resolve an WorldEntity into a concrete EntityConfig.
// No Bevy dependency — fully unit-testable on native.

use crate::entities::config::EntityConfig;
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

/// The template lookup a `[[entity]]` spawn actually performs: one caller-held
/// [`crate::entities::config_cache::ConfigCache`] first, then the host loader behind it
/// (issue #973).
///
/// # Why this type exists rather than two ad-hoc lookups
///
/// Before #973 the spawn path and the validator asked different questions.
/// Spawning went through a cache-only `resolve_entity` (since removed);
/// validation went through
/// [`WasmTemplateLoader`], which falls back to the filesystem on native. On
/// native, validation could therefore see a template the spawn path could not —
/// validation passed, the spawn logged and `continue`d, and the world came up
/// one entity short. That is the same silent-drop defect one layer up from the
/// one #973 is about.
///
/// Both sides now hold *this* loader, built from the very cache the spawn will
/// read, so "validation passed" and "the spawn will find it" are the same
/// sentence rather than two that happen to usually agree. Note the deliberate
/// ordering: the caller's cache wins, because a world layer's scoped cache
/// (`world::server::build_layer_config_cache`) is the authority for the layer
/// it was built for.
pub struct SpawnTemplateLoader<'a> {
    /// The cache the spawn will read — the global one at `Startup`, a layer's
    /// own when a layer is spawning.
    pub cache: &'a crate::entities::config_cache::ConfigCache,
    /// What the host can serve behind that cache. Production passes
    /// [`WasmTemplateLoader`].
    pub host: &'a dyn TemplateLoader,
}

impl TemplateLoader for SpawnTemplateLoader<'_> {
    fn load_template(&self, path: &str) -> Option<EntityConfig> {
        match self.cache.get(path) {
            Some(config) => Some(config.clone()),
            None => self.host.load_template(path),
        }
    }

    /// The cache only ever ADDS to what the host can serve, so the host's
    /// answer is the binding one: a cache hit cannot make a blind host
    /// authoritative about everything it is still missing.
    fn absence_is_final(&self) -> bool {
        self.host.absence_is_final()
    }
}

/// Does the template at `path` declare an `[asteroid_field]`?
///
/// Asked through the *same* lookup [`resolve_entity_via`] performs — the
/// caller's cache, then the host loader — because this predicate decides
/// **which spawn path an entry takes**, and a predicate narrower than the spawn
/// mis-routes the entry rather than dropping it (issue #973 review, F1).
///
/// # Why the two must agree
///
/// `world::config::partition_immediate_entities_three_way` and
/// `world::config::is_owned_by_unified_pipeline` split the immediate `[[entity]]`
/// list between the two `Startup` halves: asteroid fields and named entries go
/// to `world::server::spawn_immediate_entities_internal`, the anonymous
/// remainder to `server_app::spawn_anonymous_entities_internal`. While the
/// predicate was cache-only and the spawn was cache-then-disk, a field template
/// that lived on disk but not in the cache answered `false` here, landed in the
/// anonymous bucket, and spawned down the wrong path: no
/// `[asteroid_field] anchor -> anchor_offset` resolution, and an
/// `upsert_world_entity` the field path deliberately omits. A belt at the world
/// origin instead of its anchor is exactly the quiet-wrong-world outcome #973
/// exists to turn loud.
///
/// Reachable rather than theoretical: `headless::app::build_headless_app`
/// derives its preload directory from `--ship`'s parent, so
/// `--ship assets/entities/test/rng_coverage_lancer.toml --world
/// assets/worlds/combat_test.toml` preloads only `assets/entities/test/` and
/// both `asteroid_field_main.toml` belts miss the cache.
///
/// A template that does not resolve at all is not an asteroid field here; the
/// activation gate has already refused that world on any host authoritative
/// enough to say so (see [`TemplateLoader::absence_is_final`]).
pub fn template_is_asteroid_field(
    path: &str,
    config_cache: &crate::entities::config_cache::ConfigCache,
    host: &dyn TemplateLoader,
) -> bool {
    SpawnTemplateLoader {
        cache: config_cache,
        host,
    }
    .load_template(path)
    .is_some_and(|c| c.asteroid_field.is_some())
}

/// Resolve a `WorldEntity` to a concrete `EntityConfig`:
/// 1. Look `entity_inst.template_path` up in `config_cache`, then — on a miss —
///    in the `host` loader behind it (issue #973).
/// 2. Optionally merge `entity_inst.overrides` on top of the template.
///
/// The **only** entity-instance resolver; all four `[[entity]]` spawn sites use
/// it, and it is the lookup [`crate::world::validate`]'s `unresolvable-template`
/// gate is handed, so "validation passed" and "the spawn will find it" are one
/// sentence.
///
/// # Errors
///
/// Two shapes, and both are silent drops at the call sites, which log and
/// `continue`: the template did not resolve at all, or `overrides` did not
/// merge (see [`apply_overrides`]). The activation gate refuses a world for
/// either before anything spawns.
pub fn resolve_entity_via(
    entity_inst: &WorldEntity,
    config_cache: &crate::entities::config_cache::ConfigCache,
    host: &dyn TemplateLoader,
) -> Result<EntityConfig, String> {
    let loader = SpawnTemplateLoader {
        cache: config_cache,
        host,
    };
    let template = loader
        .load_template(&entity_inst.template_path)
        .ok_or_else(|| {
            format!(
                "entity template not found in cache: '{}'",
                entity_inst.template_path
            )
        })?;

    match &entity_inst.overrides {
        None => Ok(template),
        Some(overrides) => apply_overrides(&template, overrides),
    }
}

#[cfg(test)]
#[path = "loader_tests.rs"]
mod tests;

#[cfg(not(target_arch = "wasm32"))]
use crate::entities::include_resolve::ParseEntityTemplate as _;

pub use phoenix_sim_gameplay::entities::loader::{apply_overrides, TemplateLoader};
