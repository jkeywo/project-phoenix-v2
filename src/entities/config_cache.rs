// WASM/JS bridge for config preloading — all public functions are #[wasm_bindgen] exports.
//
// This module implements the preload sequence for the unified world TOML,
// entity templates, and faction TOML files. It uses thread-local storage
// to cache configs before the Bevy app is initialised.
//
// Preload sequence:
// 1. JS calls set_config_request_callback(cb)
// 2. JS fetches the chosen world TOML
// 3. JS calls wasm_load_world(path, toml_str) which parses it via the unified
//    `world::config::parse_world` pass and stores the resulting `WorldConfig`
// 4. wasm_load_world fires the callback for each referenced entity template path
// 5. For each entity path, JS fetches the TOML and calls wasm_load_config(path, toml_str)
// 6. When all pending configs are loaded, returns Ok(true) and JS calls wasm_init()

#[cfg(target_arch = "wasm32")]
use {
    crate::entity_config::EntityConfig, bevy::prelude::*, js_sys::Function,
    std::collections::VecDeque, wasm_bindgen::prelude::*,
};

// `RefCell` + `HashMap` are used by both the WASM thread-locals AND the
// cross-target sidecar inbox below, so they live outside the cfg gate.
// `HashSet` is only used inside the WASM block (the `tests` module imports
// it locally).
use std::cell::RefCell;
use std::collections::HashMap;
#[cfg(target_arch = "wasm32")]
use std::collections::HashSet;

#[cfg(not(target_arch = "wasm32"))]
use bevy::prelude::Resource;

// ── Pure helpers (native + wasm) ─────────────────────────────────────────────

/// Collect template paths nested inside an `EntityConfig` that the preload
/// pipeline must also fetch.
///
/// When an entity template carries an `[asteroid_field]` section, its
/// `asteroid_type_paths` and `cosmetic_type_paths` reference further entity
/// templates (the asteroid variants). Those need to be enqueued for fetch
/// alongside the top-level instance template paths.
pub fn nested_template_paths(config: &crate::entity_config::EntityConfig) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(field) = &config.asteroid_field {
        for p in &field.asteroid_type_paths {
            out.push(p.clone());
        }
        for p in &field.cosmetic_type_paths {
            out.push(p.clone());
        }
    }
    out
}

// ── Thread-local preload state ────────────────────────────────────────────────

#[cfg(target_arch = "wasm32")]
thread_local! {
    /// Cache of loaded entity configs by path.
    static CONFIG_CACHE: RefCell<HashMap<String, EntityConfig>> = RefCell::new(HashMap::new());

    /// Queue of entity paths that need to be loaded.
    static PENDING_QUEUE: RefCell<VecDeque<String>> = RefCell::new(VecDeque::new());

    /// Set of paths currently being fetched to prevent duplicates.
    static IN_FLIGHT: RefCell<HashSet<String>> = RefCell::new(HashSet::new());

    /// Cache of loaded faction configs by uuid.
    static FACTION_REGISTRY: RefCell<crate::faction::FactionRegistry> =
        RefCell::new(crate::faction::FactionRegistry::new());

    /// JS callback for requesting config fetches. Set by set_config_request_callback.
    static CONFIG_REQUEST_CB: RefCell<Option<Function>> = const { RefCell::new(None) };

    /// Whether all pending configs have been loaded.
    static PRELOAD_COMPLETE: RefCell<bool> = const { RefCell::new(false) };

    /// The loaded unified `WorldConfig` (PRD #337/#338 slice 1).
    /// Set by `wasm_load_world` from a single-pass parse of the world TOML.
    /// Sole world storage after PRD #341 — the old per-half thread-locals
    /// were retired.
    static WORLD_CONFIG: RefCell<Option<crate::world::config::WorldConfig>> =
        const { RefCell::new(None) };

    /// Queue of runtime-loaded world TOML strings pushed by JS via
    /// `wasm_push_world_toml` in response to a `request_world_fetch` call.
    /// Keyed by path so `pop_pending_world_toml` can retrieve by exact path.
    static PENDING_WORLD_TOML: RefCell<HashMap<String, String>> =
        RefCell::new(HashMap::new());

    /// Optional JS callback for requesting a runtime world TOML fetch.
    /// Set by `set_world_fetch_callback` from server.html.
    static WORLD_FETCH_CB: RefCell<Option<Function>> = const { RefCell::new(None) };

    /// Set of paths already requested via `request_world_fetch` to avoid
    /// duplicate JS fetch calls.
    static WORLD_FETCH_REQUESTED: RefCell<HashSet<String>> =
        RefCell::new(HashSet::new());

    /// Set of sidecar paths already requested to avoid duplicate JS fetches.
    static SIDECAR_FETCH_REQUESTED: RefCell<HashSet<String>> =
        RefCell::new(HashSet::new());
}

// The sidecar TOML inbox is a pure-Rust thread-local map: JS pushes here on
// WASM, but on native we expose the same `take_*`/`is_*` API so unit tests
// can simulate the prefetch ↔ renderer race without dragging in wasm-bindgen.
// (The JS callback wiring — `request_sidecar_fetch` — remains WASM-only.)
thread_local! {
    /// Queue of runtime-loaded model-rig sidecar TOML strings pushed by JS via
    /// `wasm_push_sidecar_toml` in response to a sidecar fetch request. Keyed
    /// by sidecar path. Kept separate from the world queue so the two fetch
    /// flows never alias even though they share the same JS fetch callback.
    static PENDING_SIDECAR_TOML: RefCell<HashMap<String, String>> =
        RefCell::new(HashMap::new());
}

// ── Public WASM API ──────────────────────────────────────────────────────────

/// Set the JavaScript callback for config fetch requests.
/// This must be called before wasm_load_world or wasm_init.
///
/// The callback signature is: `callback(path: string)`
#[cfg(target_arch = "wasm32")]
pub fn set_config_request_callback(callback: Function) {
    CONFIG_REQUEST_CB.with(|slot| {
        *slot.borrow_mut() = Some(callback);
    });
}

/// Load an entity config from a TOML string and insert it into the cache.
///
/// Returns Ok(true) when the last pending config is loaded (preload complete).
/// Returns Ok(false) while there are still pending configs.
/// Returns Err(JsValue) on parse failure (without crashing).
#[cfg(target_arch = "wasm32")]
pub fn wasm_load_config(path: String, toml_str: String) -> Result<JsValue, JsValue> {
    match EntityConfig::from_toml(&toml_str) {
        Ok(config) => {
            // Discover any nested template paths this config references
            // (e.g. an [asteroid_field] section's asteroid_type_paths) and
            // enqueue them for fetch before recording the config.
            let nested = nested_template_paths(&config);

            CONFIG_CACHE.with(|cache| {
                cache.borrow_mut().insert(path.clone(), config);
            });

            IN_FLIGHT.with(|in_flight| {
                in_flight.borrow_mut().remove(&path);
            });

            // Also remove from pending queue in case it wasn't drained before
            // the fetch completed.
            PENDING_QUEUE.with(|q| {
                q.borrow_mut().retain(|p| p != &path);
            });

            for nested_path in nested {
                queue_and_fire(nested_path);
            }
            // Check if preload is complete
            let has_pending = PENDING_QUEUE.with(|q| !q.borrow().is_empty());
            let has_in_flight = IN_FLIGHT.with(|q| !q.borrow().is_empty());

            if !has_pending && !has_in_flight {
                PRELOAD_COMPLETE.with(|flag| {
                    *flag.borrow_mut() = true;
                });
                Ok(JsValue::TRUE)
            } else {
                Ok(JsValue::FALSE)
            }
        }
        Err(e) => {
            web_sys::console::error_1(&JsValue::from_str(&format!(
                "Failed to parse entity config at {}: {:?}",
                path, e
            )));
            // Remove from in-flight and pending so it doesn't block preload
            IN_FLIGHT.with(|in_flight| {
                in_flight.borrow_mut().remove(&path);
            });
            PENDING_QUEUE.with(|q| {
                q.borrow_mut().retain(|p| p != &path);
            });
            // Even on parse failure, check if all configs are now done.
            // A failed parse still counts as "processed"; we must not let a
            // bad TOML permanently block finishInit().
            let has_pending = PENDING_QUEUE.with(|q| !q.borrow().is_empty());
            let has_in_flight = IN_FLIGHT.with(|q| !q.borrow().is_empty());
            if !has_pending && !has_in_flight {
                PRELOAD_COMPLETE.with(|flag| {
                    *flag.borrow_mut() = true;
                });
                // Return TRUE so handleConfigRequest calls finishInit().
                Ok(JsValue::TRUE)
            } else {
                Err(JsValue::from_str(&format!(
                    "Entity config parse error at {}: {:?}",
                    path, e
                )))
            }
        }
    }
}

/// Check if the preload sequence is complete (all configs loaded).
/// This can be called to verify before calling wasm_init.
#[cfg(target_arch = "wasm32")]
pub fn wasm_is_preload_complete() -> bool {
    PRELOAD_COMPLETE.with(|flag| *flag.borrow())
}

/// Unified world loader (PRD #337/#338 slice 1, PRD #341 slice 3).
///
/// Parses the world TOML into a `WorldConfig` via `parse_world` and stores it
/// in the `WORLD_CONFIG` thread-local. All entity template paths referenced
/// by the world (asteroid-field instances, named [[entity]] instances, etc.)
/// are queued via the JS preload callback so the runtime has every
/// `EntityConfig` available before `wasm_init`.
///
/// This is the sole world loader after PRD #341 — the legacy two-loader
/// split (one per half of the world TOML) was retired together with the
/// map/scenario config types.
#[cfg(target_arch = "wasm32")]
pub fn wasm_load_world(path: String, toml_str: String) -> Result<JsValue, JsValue> {
    let world_config = crate::world::config::parse_world(&toml_str).map_err(|e| {
        web_sys::console::error_1(&JsValue::from_str(&format!(
            "Failed to parse world TOML at {}: {}",
            path, e
        )));
        JsValue::from_str(&format!("World parse error at {}: {}", path, e))
    })?;

    // Queue every entity template path discovered by the unified pipeline.
    let entity_paths = crate::world::config::entity_template_paths(&world_config);

    WORLD_CONFIG.with(|slot| {
        *slot.borrow_mut() = Some(world_config);
    });

    for p in entity_paths {
        queue_and_fire(p);
    }

    Ok(JsValue::TRUE)
}

/// Get a clone of the loaded unified `WorldConfig`, if any.
#[cfg(target_arch = "wasm32")]
pub fn get_world_config() -> Option<crate::world::config::WorldConfig> {
    WORLD_CONFIG.with(|slot| slot.borrow().clone())
}

/// Register the JS callback used to request a runtime world TOML fetch.
///
/// server.html should call this once at startup with a function that accepts
/// a path string, fetches the TOML, and delivers it via `wasm_push_world_toml`.
#[cfg(target_arch = "wasm32")]
pub fn set_world_fetch_callback(callback: Function) {
    WORLD_FETCH_CB.with(|slot| {
        *slot.borrow_mut() = Some(callback);
    });
}

/// Push a runtime-fetched world TOML into the pending queue.
///
/// Called by JS after it has fetched a world at a path that Rust requested
/// via the world-fetch callback.
#[cfg(target_arch = "wasm32")]
pub fn wasm_push_world_toml(path: String, toml_str: String) {
    PENDING_WORLD_TOML.with(|m| {
        m.borrow_mut().insert(path, toml_str);
    });
}

/// Take the TOML for a previously-requested world path, if available.
///
/// Returns `Some(toml)` and removes the entry; returns `None` if the JS fetch
/// has not yet delivered the TOML.
#[cfg(target_arch = "wasm32")]
pub fn pop_pending_world_toml(path: &str) -> Option<String> {
    PENDING_WORLD_TOML.with(|m| m.borrow_mut().remove(path))
}

/// Fire the JS world-fetch callback for `path` if not already requested.
#[cfg(target_arch = "wasm32")]
pub fn request_world_fetch(path: String) {
    let already = WORLD_FETCH_REQUESTED.with(|s| s.borrow().contains(&path));
    if already {
        return;
    }
    WORLD_FETCH_REQUESTED.with(|s| s.borrow_mut().insert(path.clone()));
    WORLD_FETCH_CB.with(|slot| {
        if let Some(cb) = slot.borrow().as_ref() {
            let _ = cb.call1(&JsValue::NULL, &JsValue::from_str(&path));
        }
    });
}

// ── Model-rig sidecar fetch bridge (mirrors the world-toml flow) ─────────────
//
// The sidecar fetch reuses the same JS fetch callback registered via
// `set_world_fetch_callback` (server.html's callback simply `fetch(path)`s any
// path, so a single callback serves both world TOMLs and rig sidecars). Only
// the pending-queue + requested-set are sidecar-specific so the two flows stay
// independent.

/// Push a runtime-fetched sidecar TOML into the pending queue.
///
/// Called by JS after it has fetched a rig sidecar at a path that Rust
/// requested via `request_sidecar_fetch`. An empty string signals "absent"
/// (404) so the caller can proceed with an identity rig and stop re-requesting.
///
/// Available on native too so unit tests can simulate JS delivery without
/// dragging in wasm-bindgen.
pub fn wasm_push_sidecar_toml(path: String, toml_str: String) {
    PENDING_SIDECAR_TOML.with(|m| {
        m.borrow_mut().insert(path, toml_str);
    });
}

/// Get the TOML for a previously-requested sidecar path, if available.
///
/// Returns `Some(toml)` (possibly an empty string meaning "absent") and leaves
/// the entry in the cache; returns `None` if the JS fetch has not yet
/// delivered the TOML.
///
/// The sidecar cache is persistent: once a path is fetched it remains
/// available so that multiple entities sharing the same sidecar (e.g. many
/// asteroid rocks of the same type) can all read it without the first
/// consumer destroying it for the rest.
pub fn take_pending_sidecar_toml(path: &str) -> Option<String> {
    PENDING_SIDECAR_TOML.with(|m| m.borrow().get(path).cloned())
}

/// Non-destructive check: has JS delivered the sidecar TOML for `path` yet?
///
/// Use this when you only need to know that the fetch has completed (e.g. the
/// preload poller marking a sidecar as ready) and you do not need the TOML
/// contents. Leaves the entry in `PENDING_SIDECAR_TOML` so the renderer can
/// still consume it via [`take_pending_sidecar_toml`].
pub fn is_pending_sidecar_delivered(path: &str) -> bool {
    PENDING_SIDECAR_TOML.with(|m| m.borrow().contains_key(path))
}

/// Fire the JS fetch callback for a sidecar `path` if not already requested.
#[cfg(target_arch = "wasm32")]
pub fn request_sidecar_fetch(path: String) {
    let already = SIDECAR_FETCH_REQUESTED.with(|s| s.borrow().contains(&path));
    if already {
        return;
    }
    SIDECAR_FETCH_REQUESTED.with(|s| s.borrow_mut().insert(path.clone()));
    WORLD_FETCH_CB.with(|slot| {
        if let Some(cb) = slot.borrow().as_ref() {
            let _ = cb.call1(&JsValue::NULL, &JsValue::from_str(&path));
        }
    });
}

#[cfg(target_arch = "wasm32")]
pub fn wasm_load_faction(_path: String, toml_str: String) -> Result<JsValue, JsValue> {
    match crate::faction::parse_faction_config(&toml_str) {
        Ok(config) => {
            FACTION_REGISTRY.with(|reg| {
                reg.borrow_mut().insert(config);
            });
            Ok(JsValue::TRUE)
        }
        Err(e) => {
            web_sys::console::error_1(&JsValue::from_str(&format!(
                "Failed to parse faction TOML: {}",
                e
            )));
            Err(JsValue::from_str(&format!("Faction parse error: {}", e)))
        }
    }
}

/// Get the loaded FactionRegistry.
///
/// Pre-populates from compile-time includes if the thread-local is still
/// empty (e.g. when `wasm_load_faction` was never called from JS due to
/// missing wiring).  This ensures the registry always contains the four
/// built-in factions — Federation, Pirate, Harrow, Requiem — on every
/// target, WASM included.
#[cfg(target_arch = "wasm32")]
pub fn get_faction_registry() -> crate::faction::FactionRegistry {
    FACTION_REGISTRY.with(|reg| {
        if reg.borrow().is_empty() {
            for toml_str in &[
                include_str!("../../assets/factions/federation.toml"),
                include_str!("../../assets/factions/pirate.toml"),
                include_str!("../../assets/factions/harrow.toml"),
                include_str!("../../assets/factions/requiem.toml"),
            ] {
                if let Ok(config) = crate::faction::parse_faction_config(toml_str) {
                    reg.borrow_mut().insert(config);
                }
            }
        }
        reg.borrow().clone()
    })
}

/// Get a reference to the config cache.
#[cfg(target_arch = "wasm32")]
pub fn get_config_cache() -> ConfigCache {
    ConfigCache(CONFIG_CACHE.with(|cache| cache.borrow().clone()))
}

/// Look up a single cached entity config by template path.
#[cfg(target_arch = "wasm32")]
pub fn get_cached_entity_config(path: &str) -> Option<crate::entity_config::EntityConfig> {
    CONFIG_CACHE.with(|cache| cache.borrow().get(path).cloned())
}

/// Queue a path for fetching and fire the callback.
#[cfg(target_arch = "wasm32")]
fn queue_and_fire(path: String) {
    let mut should_fire = false;

    CONFIG_CACHE.with(|cache| {
        if !cache.borrow().contains_key(&path) {
            PENDING_QUEUE.with(|q| {
                if !q.borrow().contains(&path) {
                    q.borrow_mut().push_back(path.clone());
                    should_fire = true;
                }
            });
        }
    });

    if should_fire {
        IN_FLIGHT.with(|in_flight| {
            in_flight.borrow_mut().insert(path.clone());
        });

        CONFIG_REQUEST_CB.with(|slot| {
            if let Some(cb) = slot.borrow().as_ref() {
                let _ = cb.call1(&JsValue::NULL, &JsValue::from_str(&path));
            }
        });
    }
}

// ── Bevy Setup ──────────────────────────────────────────────────────────────

/// Newtype wrapper so HashMap<String, EntityConfig> can be inserted as a Bevy Resource.
#[cfg(target_arch = "wasm32")]
#[derive(Resource)]
pub struct ConfigCache(pub HashMap<String, EntityConfig>);

#[cfg(target_arch = "wasm32")]
impl std::ops::Deref for ConfigCache {
    type Target = HashMap<String, EntityConfig>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[cfg(target_arch = "wasm32")]
impl From<HashMap<String, EntityConfig>> for ConfigCache {
    fn from(map: HashMap<String, EntityConfig>) -> Self {
        ConfigCache(map)
    }
}

/// On non-wasm, ConfigCache is just a plain HashMap (no Bevy Resource needed).
#[cfg(not(target_arch = "wasm32"))]
pub type ConfigCache = std::collections::HashMap<String, crate::entity_config::EntityConfig>;

/// Newtype wrapper so `FactionRegistry` can be inserted as a Bevy Resource.
#[derive(Resource)]
pub struct FactionRegistryResource(pub crate::faction::FactionRegistry);

#[cfg(target_arch = "wasm32")]
impl std::ops::Deref for FactionRegistryResource {
    type Target = crate::faction::FactionRegistry;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl std::ops::Deref for FactionRegistryResource {
    type Target = crate::faction::FactionRegistry;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// Bevy plugin for setting up config resources from the preloaded state.
/// This should be added to the app in wasm_init().
#[cfg(target_arch = "wasm32")]
pub struct ConfigCachePlugin;

#[cfg(target_arch = "wasm32")]
impl Plugin for ConfigCachePlugin {
    fn build(&self, app: &mut App) {
        // Insert the ConfigCache resource
        app.insert_resource(get_config_cache());

        // Insert the FactionRegistry
        app.insert_resource(FactionRegistryResource(get_faction_registry()));
    }
}

// ── Native stubs ─────────────────────────────────────────────────────────────

#[cfg(not(target_arch = "wasm32"))]
use wasm_bindgen::prelude::*;

#[cfg(not(target_arch = "wasm32"))]
pub fn set_config_request_callback(_callback: JsValue) {}

#[cfg(not(target_arch = "wasm32"))]
pub fn set_world_fetch_callback(_callback: JsValue) {}

#[cfg(not(target_arch = "wasm32"))]
pub fn wasm_push_world_toml(_path: String, _toml_str: String) {}

#[cfg(not(target_arch = "wasm32"))]
pub fn wasm_load_config(_path: String, _toml_str: String) -> Result<JsValue, JsValue> {
    Ok(JsValue::from_bool(false))
}

#[cfg(not(target_arch = "wasm32"))]
pub fn wasm_is_preload_complete() -> bool {
    false
}

#[cfg(not(target_arch = "wasm32"))]
pub fn get_config_cache() -> ConfigCache {
    ConfigCache::new()
}

// Native twin of the WASM cache lookup. There is no preload cache on native —
// templates are read from disk on demand — so the lookup always misses and the
// caller is expected to fall back to the filesystem.
#[cfg(not(target_arch = "wasm32"))]
pub fn get_cached_entity_config(_path: &str) -> Option<crate::entity_config::EntityConfig> {
    None
}

#[cfg(not(target_arch = "wasm32"))]
pub fn wasm_load_world(_path: String, _toml_str: String) -> Result<JsValue, JsValue> {
    Ok(JsValue::from_bool(true))
}

#[cfg(not(target_arch = "wasm32"))]
pub fn get_world_config() -> Option<crate::world::config::WorldConfig> {
    None
}

// Native no-ops for the runtime world-fetch helpers (native uses std::fs directly).
#[cfg(not(target_arch = "wasm32"))]
pub fn pop_pending_world_toml(_path: &str) -> Option<String> {
    None
}
#[cfg(not(target_arch = "wasm32"))]
pub fn request_world_fetch(_path: String) {}

// Native no-op for the JS fetch trigger (native reads sidecars via std::fs
// directly in `load_sidecar_toml`). The pure-Rust take/is/push functions
// above are shared by both targets.
#[cfg(not(target_arch = "wasm32"))]
pub fn request_sidecar_fetch(_path: String) {}

#[cfg(not(target_arch = "wasm32"))]
pub fn wasm_load_faction(_path: String, _toml_str: String) -> Result<JsValue, JsValue> {
    Ok(JsValue::from_bool(false))
}

#[cfg(not(target_arch = "wasm32"))]
pub fn get_faction_registry() -> crate::faction::FactionRegistry {
    let mut registry = crate::faction::FactionRegistry::new();
    for toml_str in &[
        include_str!("../../assets/factions/federation.toml"),
        include_str!("../../assets/factions/pirate.toml"),
        include_str!("../../assets/factions/harrow.toml"),
        include_str!("../../assets/factions/requiem.toml"),
    ] {
        if let Ok(config) = crate::faction::parse_faction_config(toml_str) {
            registry.insert(config);
        }
    }
    registry
}

// ── Unit Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use crate::entity_config::EntityConfig;
    use std::collections::{HashMap, HashSet, VecDeque};

    // ── Helper for tests ───────────────────────────────────────────────────────

    fn default_entity_config() -> EntityConfig {
        EntityConfig::default()
    }

    // ── Integration Tests ──────────────────────────────────────────────────────

    #[test]
    fn entity_config_parsing_integration() {
        let toml = r#"
tags = ["asteroid", "small"]

[hull]
hull_integrity = 30

[collider]
shape = "Ball"
radius = 5.0
length = 0.0
"#;
        let result = EntityConfig::from_toml(toml);
        assert!(result.is_ok());
        let config = result.unwrap();
        assert_eq!(config.tags, vec!["asteroid", "small"]);
        assert!(config.hull.is_some());
        assert!((config.hull.as_ref().unwrap().hull_integrity - 30.0).abs() < 1e-6);
        assert!(config.collider.is_some());
    }

    // ── Native ConfigCache Tests ──────────────────────────────────────────────

    // A simple native version of ConfigCache for testing
    #[derive(Default)]
    struct TestConfigCache {
        cache: HashMap<String, EntityConfig>,
        pending: VecDeque<String>,
        in_flight: HashSet<String>,
    }

    impl TestConfigCache {
        fn new() -> Self {
            Self::default()
        }

        fn insert(&mut self, path: String, config: EntityConfig) {
            self.cache.insert(path.clone(), config);
            self.in_flight.remove(&path);
            // Remove from pending
            if let Some(pos) = self.pending.iter().position(|p| p == &path) {
                self.pending.remove(pos);
            }
        }

        fn has_pending(&self) -> bool {
            !self.pending.is_empty()
        }

        fn queue_fetch(&mut self, path: String) {
            if !self.cache.contains_key(&path)
                && !self.in_flight.contains(&path)
                && !self.pending.contains(&path)
            {
                self.pending.push_back(path);
            }
        }

        fn mark_in_flight(&mut self, path: String) {
            self.in_flight.insert(path);
        }

        fn all_pending(&self) -> Vec<String> {
            self.pending.iter().cloned().collect()
        }
    }

    #[test]
    fn config_cache_no_duplicate_queueing() {
        let mut cache = TestConfigCache::new();

        cache.queue_fetch("path1".to_string());
        cache.queue_fetch("path1".to_string()); // Duplicate

        assert_eq!(cache.all_pending(), vec!["path1"]);
    }

    #[test]
    fn config_cache_in_flight_prevents_queueing() {
        let mut cache = TestConfigCache::new();

        cache.mark_in_flight("path1".to_string());
        cache.queue_fetch("path1".to_string());

        assert!(!cache.has_pending());
    }

    #[test]
    fn config_cache_cached_prevents_queueing() {
        let mut cache = TestConfigCache::new();
        let config = default_entity_config();

        cache.insert("path1".to_string(), config);
        cache.queue_fetch("path1".to_string());

        assert!(!cache.has_pending());
    }

    #[test]
    fn config_cache_preload_complete_when_all_inserted() {
        let mut cache = TestConfigCache::new();

        // Queue multiple paths
        cache.queue_fetch("path1".to_string());
        cache.queue_fetch("path2".to_string());

        assert!(cache.has_pending());

        // Insert configs
        cache.insert("path1".to_string(), default_entity_config());
        cache.insert("path2".to_string(), default_entity_config());

        // Now no pending - preload complete
        assert!(!cache.has_pending());
    }

    #[test]
    fn config_cache_partial_preload_still_has_pending() {
        let mut cache = TestConfigCache::new();

        cache.queue_fetch("path1".to_string());
        cache.queue_fetch("path2".to_string());

        // Insert only one
        cache.insert("path1".to_string(), default_entity_config());

        // Still has pending
        assert!(cache.has_pending());
        assert_eq!(cache.all_pending(), vec!["path2"]);
    }

    // ── nested_template_paths ─────────────────────────────────────────────────

    #[test]
    fn nested_template_paths_empty_for_bare_config() {
        let config = default_entity_config();
        assert!(super::nested_template_paths(&config).is_empty());
    }

    #[test]
    fn nested_template_paths_returns_asteroid_field_type_paths() {
        let toml_str = r#"
tags = ["field"]

[asteroid_field]
inner_radius = 100.0
outer_radius = 200.0
density = 0.005
asteroid_type_paths = ["a.toml", "b.toml"]
cosmetic_type_paths = ["c.toml"]
"#;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let mut paths = super::nested_template_paths(&config);
        paths.sort();
        assert_eq!(paths, vec!["a.toml", "b.toml", "c.toml"]);
    }

    /// Simulate the preload pipeline: top-level instance template references
    /// an asteroid_field template, which itself references asteroid variants.
    /// The variant paths must be queued when the field template parses.
    #[test]
    fn loading_field_template_enqueues_nested_asteroid_paths() {
        let mut cache = TestConfigCache::new();
        // 1. Top-level: queue the field template.
        cache.queue_fetch("asteroid_field_main.toml".to_string());

        // 2. Parse the field template; insert it.
        let field_toml = r#"
tags = ["field"]

[asteroid_field]
inner_radius = 100.0
outer_radius = 200.0
density = 0.005
asteroid_type_paths = ["asteroid_small.toml"]
cosmetic_type_paths = ["asteroid_cosmetic.toml"]
"#;
        let field_config = EntityConfig::from_toml(field_toml).unwrap();

        // 3. Enqueue nested paths discovered in the parsed config.
        for nested in super::nested_template_paths(&field_config) {
            cache.queue_fetch(nested);
        }
        cache.insert("asteroid_field_main.toml".to_string(), field_config);

        // The variant paths must now be pending.
        let mut pending = cache.all_pending();
        pending.sort();
        assert_eq!(
            pending,
            vec!["asteroid_cosmetic.toml", "asteroid_small.toml"]
        );
    }

    // ── Sidecar cache: persistent read semantics ─────────────────────────
    //
    // The sidecar cache is persistent: once a TOML is pushed it stays so
    // that many entities sharing the same sidecar path (e.g. multiple rocks
    // of the same asteroid type) can all read it. `take_pending_sidecar_toml`
    // is non-destructive (returns a clone); `is_pending_sidecar_delivered`
    // checks presence. The preload poller uses the latter to track progress.

    /// Each test in this module mutates the process-wide
    /// `PENDING_SIDECAR_TOML` thread-local. Tests must use unique paths so
    /// they remain order-independent.
    fn unique_sidecar_path(test_name: &str) -> String {
        format!("assets/models/__test_{test_name}.model.toml")
    }

    #[test]
    fn is_pending_sidecar_delivered_false_before_push() {
        let path = unique_sidecar_path("not_pushed");
        assert!(!super::is_pending_sidecar_delivered(&path));
    }

    #[test]
    fn is_pending_sidecar_delivered_true_after_push() {
        let path = unique_sidecar_path("pushed");
        super::wasm_push_sidecar_toml(path.clone(), "anything".to_string());
        assert!(super::is_pending_sidecar_delivered(&path));
    }

    #[test]
    fn take_is_non_destructive_multiple_readers() {
        // Regression: many asteroid entities share the same sidecar path.
        // The first entity to call take_pending_sidecar_toml must not destroy
        // the entry — subsequent entities for the same sidecar must also get it.
        let path = unique_sidecar_path("multi_reader");
        super::wasm_push_sidecar_toml(path.clone(), "rig-toml-body".to_string());

        // First reader (entity 1).
        assert_eq!(
            super::take_pending_sidecar_toml(&path),
            Some("rig-toml-body".to_string()),
        );
        // Second reader (entity 2) must still see it.
        assert_eq!(
            super::take_pending_sidecar_toml(&path),
            Some("rig-toml-body".to_string()),
            "second entity for the same sidecar path must not get None"
        );
        // is_pending_sidecar_delivered also still true.
        assert!(super::is_pending_sidecar_delivered(&path));
    }

    #[test]
    fn is_pending_sidecar_delivered_is_non_destructive() {
        let path = unique_sidecar_path("non_destructive");
        super::wasm_push_sidecar_toml(path.clone(), "rig-toml-body".to_string());
        assert!(super::is_pending_sidecar_delivered(&path));
        assert!(super::is_pending_sidecar_delivered(&path));
        assert_eq!(
            super::take_pending_sidecar_toml(&path),
            Some("rig-toml-body".to_string()),
        );
        // Cache entry persists after take.
        assert!(super::is_pending_sidecar_delivered(&path));
    }

    #[test]
    fn empty_body_signals_absent_sidecar_and_is_delivered() {
        // JS pushes an empty string on 404 so the renderer can fall back to
        // an identity rig instead of re-requesting forever. The peek API
        // must still report the empty body as "delivered".
        let path = unique_sidecar_path("absent_404");
        super::wasm_push_sidecar_toml(path.clone(), String::new());
        assert!(super::is_pending_sidecar_delivered(&path));
        assert_eq!(super::take_pending_sidecar_toml(&path), Some(String::new()));
    }
}
