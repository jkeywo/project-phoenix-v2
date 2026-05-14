// WASM/JS bridge for config preloading — all public functions are #[wasm_bindgen] exports.
//
// This module implements the preload sequence for map, entity, and complexity TOML files.
// It uses thread-local storage to cache configs before the Bevy app is initialized.
//
// Preload sequence:
// 1. JS calls set_config_request_callback(cb)
// 2. JS fetches the default map TOML
// 3. JS calls wasm_load_map(toml_str) which parses and stores the map
// 4. wasm_load_map fires the callback for each referenced entity path
// 5. For each entity path, JS fetches the TOML and calls wasm_load_config(path, toml_str)
// 6. wasm_load_config parses and caches the entity config; if any console config
//    references a complexity_toml, that path is also queued for loading
// 7. JS fetches complexity TOML files and calls wasm_load_complexity(path, toml_str)
// 8. When all pending configs are loaded, returns Ok(true) and JS calls wasm_init()

#[cfg(target_arch = "wasm32")]
use {
    crate::entity_config::EntityConfig,
    crate::map_config::MapConfig,
    crate::scenario::ScenarioConfig,
    bevy::prelude::*,
    js_sys::Function,
    std::cell::RefCell,
    std::collections::{HashMap, HashSet, VecDeque},
    wasm_bindgen::prelude::*,
};

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
    /// The loaded MapConfig, if any. Set by wasm_load_map.
    static MAP_CONFIG: RefCell<Option<MapConfig>> = const { RefCell::new(None) };
    
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

    /// The loaded ScenarioConfig, if any. Set by wasm_load_scenario.
    static SCENARIO_CONFIG: RefCell<Option<ScenarioConfig>> = const { RefCell::new(None) };
}

// ── Public WASM API ──────────────────────────────────────────────────────────

/// Set the JavaScript callback for config fetch requests.
/// This must be called before wasm_load_map or wasm_init.
///
/// The callback signature is: `callback(path: string)`
#[cfg(target_arch = "wasm32")]
pub fn set_config_request_callback(callback: Function) {
    CONFIG_REQUEST_CB.with(|slot| {
        *slot.borrow_mut() = Some(callback);
    });
}

/// Load a map TOML string, parse it, store it, and queue all referenced entity paths.
///
/// On success, fires the config request callback for each entity path referenced
/// in the map's asteroid fields. Returns Ok(true).
///
/// On parse failure, logs the error and returns Err(JsValue) without crashing.
#[cfg(target_arch = "wasm32")]
pub fn wasm_load_map(toml_str: String) -> Result<JsValue, JsValue> {
    match crate::map_config::parse_map_config(&toml_str) {
        Ok(map_config) => {
            // Store the map config
            MAP_CONFIG.with(|slot| {
                *slot.borrow_mut() = Some(map_config.clone());
            });
            
            // Collect all unique entity paths from asteroid fields
            // AND from entity instance templates (which may themselves reference
            // asteroid type paths in their asteroid_field sections).
            let mut entity_paths = HashSet::new();
            
            // 1. Collect from typed asteroid fields (legacy)
            for field in &map_config.asteroid_fields {
                for path in &field.asteroid_type_paths {
                    entity_paths.insert(path.clone());
                }
                for path in &field.cosmetic_type_paths {
                    entity_paths.insert(path.clone());
                }
            }
            
            // 2. Collect template paths from entity instances
            for entity_inst in &map_config.entities {
                entity_paths.insert(entity_inst.template_path.clone());
            }
            
            // Queue all entity paths and fire callbacks
            for path in &entity_paths {
                queue_and_fire(path.clone());
            }
            
            Ok(JsValue::TRUE)
        }
        Err(e) => {
            web_sys::console::error_1(&JsValue::from_str(&format!(
                "Failed to parse map TOML: {}",
                e
            )));
            Err(JsValue::from_str(&format!("Map parse error: {}", e)))
        }
    }
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

/// Get the loaded MapConfig, if any.
#[cfg(target_arch = "wasm32")]
pub fn get_map_config() -> Option<MapConfig> {
    MAP_CONFIG.with(|slot| slot.borrow().clone())
}

/// Load a scenario TOML string, parse it, store it, and queue entity paths from spawns.
///
/// On success, returns `Ok(JsValue::TRUE)`.
/// On parse failure, logs the error and returns `Err(JsValue)`.
#[cfg(target_arch = "wasm32")]
pub fn wasm_load_scenario(path: String, toml_str: String) -> Result<JsValue, JsValue> {
    match crate::scenario::parse_scenario(&toml_str) {
        Ok(scenario_config) => {
            // Queue entity paths from scenario spawns so they are preloaded.
            for spawn in &scenario_config.spawns {
                queue_and_fire(spawn.entity_path.clone());
            }
            SCENARIO_CONFIG.with(|slot| {
                *slot.borrow_mut() = Some(scenario_config);
            });
            Ok(JsValue::TRUE)
        }
        Err(e) => {
            web_sys::console::error_1(&JsValue::from_str(&format!(
                "Failed to parse scenario TOML at {}: {}",
                path, e
            )));
            Err(JsValue::from_str(&format!("Scenario parse error at {}: {}", path, e)))
        }
    }
}

/// Return the `default_scenario` path from the loaded map config, if any.
#[cfg(target_arch = "wasm32")]
pub fn wasm_get_default_scenario_path() -> Option<String> {
    MAP_CONFIG.with(|slot| {
        slot.borrow().as_ref().and_then(|m| m.default_scenario.clone())
    })
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

/// Get the loaded ScenarioConfig, if any.
#[cfg(target_arch = "wasm32")]
pub fn get_scenario_config() -> Option<ScenarioConfig> {
    SCENARIO_CONFIG.with(|slot| slot.borrow().clone())
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

/// Newtype wrapper so `ScenarioConfig` can be inserted as a Bevy Resource.
#[cfg(target_arch = "wasm32")]
#[derive(Resource)]
pub struct ScenarioResource(pub crate::scenario::ScenarioConfig);

#[cfg(target_arch = "wasm32")]
impl std::ops::Deref for ScenarioResource {
    type Target = crate::scenario::ScenarioConfig;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// Newtype wrapper so `FactionRegistry` can be inserted as a Bevy Resource.
#[cfg(target_arch = "wasm32")]
#[derive(Resource)]
pub struct FactionRegistryResource(pub crate::faction::FactionRegistry);

#[cfg(target_arch = "wasm32")]
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
        // Insert the MapConfig resource if loaded
        if let Some(map_config) = get_map_config() {
            app.insert_resource(map_config);
        }

        // Insert the ConfigCache resource
        app.insert_resource(get_config_cache());

        // Insert the ComplexityResources
        app.insert_resource(get_complexity_resources());

        // Insert the ScenarioConfig if one was loaded
        if let Some(scenario_config) = get_scenario_config() {
            app.insert_resource(ScenarioResource(scenario_config));
        }

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
pub fn wasm_load_map(_toml_str: String) -> Result<JsValue, JsValue> {
    Ok(JsValue::from_bool(true))
}

#[cfg(not(target_arch = "wasm32"))]
pub fn wasm_load_config(_path: String, _toml_str: String) -> Result<JsValue, JsValue> {
    Ok(JsValue::from_bool(false))
}

#[cfg(not(target_arch = "wasm32"))]
pub fn wasm_is_preload_complete() -> bool {
    false
}

#[cfg(not(target_arch = "wasm32"))]
pub fn get_map_config() -> Option<crate::map_config::MapConfig> {
    None
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
pub fn wasm_load_scenario(_path: String, _toml_str: String) -> Result<JsValue, JsValue> {
    Ok(JsValue::from_bool(false))
}

#[cfg(not(target_arch = "wasm32"))]
pub fn get_scenario_config() -> Option<crate::scenario::ScenarioConfig> {
    None
}

#[cfg(not(target_arch = "wasm32"))]
pub fn wasm_get_default_scenario_path() -> Option<String> {
    None
}

#[cfg(not(target_arch = "wasm32"))]
pub fn wasm_load_faction(_path: String, _toml_str: String) -> Result<JsValue, JsValue> {
    Ok(JsValue::from_bool(false))
}

#[cfg(not(target_arch = "wasm32"))]
pub fn get_faction_registry() -> crate::faction::FactionRegistry {
    crate::faction::FactionRegistry::new()
}

// ── Unit Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use crate::entity_config::EntityConfig;
    use std::collections::{HashMap, HashSet, VecDeque};
    
    // ── Helper for tests ───────────────────────────────────────────────────────
    
    fn default_entity_config() -> EntityConfig {
        EntityConfig {
            tags: vec![],
            hull: None,
            collider: None,
            appearance: None,
            helm_console: None,
            weapons_console: None,
            engineering_console: None,
            captain_console: None,
            power: None,
            science_console: None,
            sensors_console: None,
            shields_console: None,
            star: None,
            planet: None,
            asteroid_field: None,
            shape: None,
            effects: None,
            station: None,
            faction: None,
            behaviour: None,
        }
    }
    
    // ── Integration Tests ──────────────────────────────────────────────────────
    
    #[test]
    fn map_config_parsing_integration() {
        let toml = r#"
[global]
seed = 42

[[star]]
name = "Sun"
radius = 50.0
colour = [1.0, 0.8, 0.0]
position = [0.0, 0.0, 0.0]

[[asteroid_field]]
inner_radius = 100.0
outer_radius = 200.0
density = 0.005
asteroid_type_paths = ["assets/entities/asteroid.toml"]
"#;
        let result = crate::map_config::parse_map_config(toml);
        assert!(result.is_ok());
        let map_config = result.unwrap();
        assert_eq!(map_config.global.seed, 42);
        assert_eq!(map_config.stars.len(), 1);
        assert_eq!(map_config.asteroid_fields.len(), 1);
    }
    
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

    // ── wasm_load_scenario integration ────────────────────────────────────────

    #[test]
    fn scenario_toml_round_trip_via_parse_scenario() {
        let toml = r#"
[[spawn]]
name = "asteroid_alpha"
entity_path = "entities/asteroid_large.toml"
position = [100.0, 0.0, 200.0]
"#;
        let config = crate::scenario::parse_scenario(toml).expect("parse must succeed");
        assert_eq!(config.spawns.len(), 1);
        assert_eq!(config.spawns[0].name, "asteroid_alpha");
        assert_eq!(config.spawns[0].entity_path, "entities/asteroid_large.toml");
    }

    #[test]
    fn scenario_anchor_resolution_uses_map_anchors() {
        use std::collections::HashMap;
        let toml = r#"
[[spawn]]
name = "station"
entity_path = "entities/station.toml"
anchor = "alpha_point"
"#;
        let config = crate::scenario::parse_scenario(toml).expect("parse must succeed");
        let mut anchors: HashMap<String, Vec<f32>> = HashMap::new();
        anchors.insert("alpha_point".to_string(), vec![50.0, 0.0, 100.0]);
        let resolved = crate::scenario::resolve_positions(&config, &anchors)
            .expect("resolve must succeed");
        assert_eq!(resolved[0].position, [50.0, 0.0, 100.0]);
    }
}
