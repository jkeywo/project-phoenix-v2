// WASM/JS bridge for config preloading — all public functions are #[wasm_bindgen] exports.
//
// This module implements the preload sequence for the unified world TOML,
// entity templates, complexity TOML files, and faction TOML files. It uses
// thread-local storage to cache configs before the Bevy app is initialised.
//
// Preload sequence:
// 1. JS calls set_config_request_callback(cb)
// 2. JS fetches the chosen world TOML
// 3. JS calls wasm_load_world(path, toml_str) which parses it via the unified
//    `world::config::parse_world` pass and stores the resulting `WorldConfig`
// 4. wasm_load_world fires the callback for each referenced entity template path
// 5. For each entity path, JS fetches the TOML and calls wasm_load_config(path, toml_str)
// 6. wasm_load_config parses and caches the entity config; if any console config
//    references a complexity_toml, that path is also queued for loading
// 7. JS fetches complexity TOML files and calls wasm_load_complexity(path, toml_str)
// 8. When all pending configs are loaded, returns Ok(true) and JS calls wasm_init()

#[cfg(target_arch = "wasm32")]
use {
    crate::entity_config::EntityConfig,
    bevy::prelude::*,
    js_sys::Function,
    std::cell::RefCell,
    std::collections::{HashMap, HashSet, VecDeque},
    wasm_bindgen::prelude::*,
};

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

    /// Cache of loaded complexity configs by path.
    static COMPLEXITY_CACHE: RefCell<Option<HashMap<String, crate::complexity::ComplexityConfig>>> =
        const { RefCell::new(None) };

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

            // Scan for complexity_toml references and queue them
            queue_complexity_refs(&config);

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
                Err(JsValue::from_str(&format!("Entity config parse error at {}: {:?}", path, e)))
            }
        }
    }
}

/// Load a complexity config TOML string and insert it into the cache.
///
/// Returns Ok(true) when the last pending config is loaded (preload complete).
/// Returns Ok(false) while there are still pending configs.
#[cfg(target_arch = "wasm32")]
pub fn wasm_load_complexity(path: String, toml_str: String) -> Result<JsValue, JsValue> {
    match crate::complexity::parse_complexity_config(&toml_str) {
        Ok(config) => {
            COMPLEXITY_CACHE.with(|cache| {
                cache.borrow_mut().get_or_insert_with(HashMap::new).insert(path.clone(), config);
            });

            IN_FLIGHT.with(|in_flight| {
                in_flight.borrow_mut().remove(&path);
            });
            PENDING_QUEUE.with(|q| {
                q.borrow_mut().retain(|p| p != &path);
            });

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
                "Failed to parse complexity config at {}: {}",
                path, e
            )));
            IN_FLIGHT.with(|in_flight| {
                in_flight.borrow_mut().remove(&path);
            });
            PENDING_QUEUE.with(|q| {
                q.borrow_mut().retain(|p| p != &path);
            });
            let has_pending = PENDING_QUEUE.with(|q| !q.borrow().is_empty());
            let has_in_flight = IN_FLIGHT.with(|q| !q.borrow().is_empty());
            if !has_pending && !has_in_flight {
                PRELOAD_COMPLETE.with(|flag| {
                    *flag.borrow_mut() = true;
                });
                Ok(JsValue::TRUE)
            } else {
                Err(JsValue::from_str(&format!("Complexity config parse error at {}: {}", path, e)))
            }
        }
    }
}

/// Scan an entity config for complexity_toml references and queue any found.
#[cfg(target_arch = "wasm32")]
fn queue_complexity_refs(config: &EntityConfig) {
    let paths = config.complexity_toml_paths();
    for p in paths {
        COMPLEXITY_CACHE.with(|cache| {
            if cache.borrow().as_ref().map_or(true, |m| !m.contains_key(&p)) {
                queue_and_fire(p);
            }
        });
    }
}

/// Get complexity resources.
#[cfg(target_arch = "wasm32")]
pub fn get_complexity_resources() -> ComplexityResources {
    ComplexityResources(COMPLEXITY_CACHE.with(|cache| cache.borrow().clone().unwrap_or_default()))
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

/// Load a faction TOML string and insert it into the faction registry.
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
#[cfg(target_arch = "wasm32")]
pub fn get_faction_registry() -> crate::faction::FactionRegistry {
    FACTION_REGISTRY.with(|reg| reg.borrow().clone())
}

/// Get a reference to the config cache.
#[cfg(target_arch = "wasm32")]
pub fn get_config_cache() -> ConfigCache {
    ConfigCache(CONFIG_CACHE.with(|cache| cache.borrow().clone()))
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

/// On non-wasm, ConfigCache is just a plain HashMap (no Bevy Resource needed).
#[cfg(not(target_arch = "wasm32"))]
pub type ConfigCache = std::collections::HashMap<String, crate::entity_config::EntityConfig>;

/// Newtype wrapper so HashMap<String, ComplexityConfig> can be inserted as a Bevy Resource.
#[cfg(target_arch = "wasm32")]
#[derive(Resource)]
pub struct ComplexityResources(pub std::collections::HashMap<String, crate::complexity::ComplexityConfig>);

#[cfg(target_arch = "wasm32")]
impl std::ops::Deref for ComplexityResources {
    type Target = std::collections::HashMap<String, crate::complexity::ComplexityConfig>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// On non-wasm, ComplexityResources is just a plain HashMap.
#[cfg(not(target_arch = "wasm32"))]
pub type ComplexityResources = std::collections::HashMap<String, crate::complexity::ComplexityConfig>;

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

        // Insert the ComplexityResources
        app.insert_resource(get_complexity_resources());

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

#[cfg(not(target_arch = "wasm32"))]
pub fn wasm_load_complexity(_path: String, _toml_str: String) -> Result<JsValue, JsValue> {
    Ok(JsValue::from_bool(false))
}

#[cfg(not(target_arch = "wasm32"))]
pub fn get_complexity_resources() -> ComplexityResources {
    ComplexityResources::new()
}

#[cfg(not(target_arch = "wasm32"))]
pub fn wasm_load_world(_path: String, _toml_str: String) -> Result<JsValue, JsValue> {
    Ok(JsValue::from_bool(true))
}

#[cfg(not(target_arch = "wasm32"))]
pub fn get_world_config() -> Option<crate::world::config::WorldConfig> {
    None
}

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
        
        fn contains(&self, path: &str) -> bool {
            self.cache.contains_key(path)
        }
        
        fn get(&self, path: &str) -> Option<&EntityConfig> {
            self.cache.get(path)
        }
        
        fn has_pending(&self) -> bool {
            !self.pending.is_empty()
        }
        
        fn queue_fetch(&mut self, path: String) {
            if !self.cache.contains_key(&path) && !self.in_flight.contains(&path) {
                if !self.pending.contains(&path) {
                    self.pending.push_back(path);
                }
            }
        }
        
        fn mark_in_flight(&mut self, path: String) {
            self.in_flight.insert(path);
        }
        
        fn next_pending(&mut self) -> Option<String> {
            self.pending.pop_front()
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
    fn entity_config_complexity_toml_paths_discovered() {
        let toml = r#"
[helm_console]
complexity_toml = "assets/complexity/helm.toml"

[weapons_console]
complexity_toml = "assets/complexity/tactical.toml"
"#;
        let config = EntityConfig::from_toml(toml).expect("parse must succeed");
        let paths = config.complexity_toml_paths();
        assert_eq!(paths.len(), 2);
        assert!(paths.contains(&"assets/complexity/helm.toml".to_string()));
        assert!(paths.contains(&"assets/complexity/tactical.toml".to_string()));
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
        assert_eq!(pending, vec!["asteroid_cosmetic.toml", "asteroid_small.toml"]);
    }
}
