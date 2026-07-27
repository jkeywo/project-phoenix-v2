// Pure module: resolve an WorldEntity into a concrete EntityConfig.
// No Bevy dependency — fully unit-testable on native.

use crate::entity_config::EntityConfig;
use crate::world::config::WorldEntity;

/// Resolve an `WorldEntity` to a concrete `EntityConfig` by:
/// 1. Looking up `entity_inst.template_path` in the config cache.
/// 2. Optionally merging `entity_inst.overrides` on top of the template TOML.
///
/// Returns `Err` if the template is not found in the cache.
pub fn resolve_entity(
    entity_inst: &WorldEntity,
    config_cache: &crate::config_cache::ConfigCache,
) -> Result<EntityConfig, String> {
    let template = config_cache
        .get(&entity_inst.template_path)
        .ok_or_else(|| {
            format!(
                "entity template not found in cache: '{}'",
                entity_inst.template_path
            )
        })?;

    let config = match &entity_inst.overrides {
        None => template.clone(),
        Some(overrides) => apply_overrides(template, overrides)?,
    };

    Ok(config)
}

/// Merge an authored `overrides` table on top of a resolved template.
///
/// Shared by every path that resolves an entity instance against its template:
/// the `[[entity]]` loader above, and the world-composition validator's
/// doctrine read (issue #888), which has to see the *effective* doctrine —
/// `assets/worlds/probe_artillery_standoff.toml` adds a doctrine entry by
/// override, and a validator reading the raw template would be judging content
/// no scenario ever runs.
///
/// Serialises the template to a `toml::Value` **losslessly** (issue #838):
/// `to_toml_value` re-emits the `[[station]]`/`[[system]]`/`[power_groups]`/
/// `[[shield_arc]]` blocks that a plain `toml::to_string` would drop (they live
/// in `#[serde(skip)]` fields), so the merged config keeps the template's whole
/// system suite instead of spawning a hull with no stations or weapons.
pub fn apply_overrides(
    template: &EntityConfig,
    overrides: &toml::Value,
) -> Result<EntityConfig, String> {
    let template_value = template
        .to_toml_value()
        .map_err(|e| format!("template serialise error: {e}"))?;

    let merged = crate::entity_override::merge_entity_config_toml(&template_value, overrides);
    let merged_str =
        toml::to_string(&merged).map_err(|e| format!("merged serialise error: {e}"))?;
    EntityConfig::from_toml(&merged_str).map_err(|e| format!("merged parse error: {e:?}"))
}

/// Generate a new UUID string for a spawned entity.
pub fn assign_uuid() -> String {
    uuid::Uuid::new_v4().to_string()
}

// ── Template resolution ──────────────────────────────────────────────────────

/// Resolve an entity-template path to a concrete `EntityConfig`, abstracting
/// over *where* the template comes from: the filesystem on native
/// (`FsTemplateLoader`), the preloaded config cache on WASM
/// (`WasmTemplateLoader`).
///
/// Object-safe by design — `&self`, no generics on the method — so callers can
/// hold a `&dyn TemplateLoader` and tests can inject a fake without touching
/// the filesystem or the WASM thread-locals.
///
/// # Diagnostics are the caller's job
///
/// `load_template` returns `Option`, not `Result`: a missing template and a
/// malformed one both collapse to `None`, and any `toml::de::Error` is
/// intentionally dropped at this boundary. This module is deliberately free of
/// Bevy (see the file header), so it must not log. The caller — which knows
/// which entity, trigger, or spawn request asked for the template — is
/// responsible for emitting the warning.
pub trait TemplateLoader {
    /// Load and parse the template at `path`, or `None` if it cannot be found
    /// or parsed.
    fn load_template(&self, path: &str) -> Option<EntityConfig>;
}

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
        crate::entity_includes::load_entity_config(path).ok()
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
        if let Some(config) = crate::config_cache::get_cached_entity_config(path) {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config_cache::ConfigCache;
    use crate::entity_config::EntityConfig;
    use std::collections::HashMap;

    fn make_cache(path: &str, toml: &str) -> ConfigCache {
        let mut m = HashMap::new();
        m.insert(path.to_string(), EntityConfig::from_toml(toml).unwrap());
        ConfigCache::from(m)
    }

    #[test]
    fn resolve_missing_template_returns_err() {
        let cache = ConfigCache::from(HashMap::new());
        let inst = WorldEntity {
            template_path: "assets/entities/missing.toml".to_string(),
            ..Default::default()
        };
        let result = resolve_entity(&inst, &cache);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found in cache"));
    }

    #[test]
    fn resolve_template_no_overrides_returns_clone() {
        let cache = make_cache("assets/entities/rock.toml", r#"tags = ["asteroid"]"#);
        let inst = WorldEntity {
            template_path: "assets/entities/rock.toml".to_string(),
            overrides: None,
            ..Default::default()
        };
        let config = resolve_entity(&inst, &cache).unwrap();
        assert_eq!(config.tags, vec!["asteroid"]);
    }

    /// Verifies that a world override on `[behaviour]` round-trips cleanly.
    /// (#572) FSM dissolved — `initial_state`/`transition` fields removed.
    /// Overrides now target `waypoint_arrival_radius` or doctrine entries.
    /// This test confirms that overriding `waypoint_arrival_radius` merges
    /// correctly while preserving the template's doctrine objectives.
    #[test]
    fn harrow_destroyer_behaviour_override_merges_arrival_radius() {
        let template_toml = std::fs::read_to_string("assets/entities/ship_harrow_destroyer.toml")
            .expect("ship_harrow_destroyer.toml must exist");
        let cache = make_cache("assets/entities/ship_harrow_destroyer.toml", &template_toml);

        let override_value: toml::Value = toml::from_str(
            r#"
[behaviour]
waypoint_arrival_radius = 99.0
"#,
        )
        .unwrap();

        let inst = WorldEntity {
            template_path: "assets/entities/ship_harrow_destroyer.toml".to_string(),
            overrides: Some(override_value),
            ..Default::default()
        };
        let config = resolve_entity(&inst, &cache).unwrap();
        let beh = config
            .behaviour
            .expect("raider must keep [behaviour] section");
        assert!(
            (beh.waypoint_arrival_radius - 99.0).abs() < 1e-6,
            "overridden waypoint_arrival_radius must be 99.0"
        );
        assert!(
            !beh.doctrine.is_empty(),
            "template doctrine objectives must be preserved by override merge"
        );
    }

    /// End-to-end check that an inline-table `overrides` block parses through
    /// `parse_world` → `resolve_entity` and merges correctly.
    /// (#572) FSM dissolved — override now targets `waypoint_arrival_radius`.
    #[test]
    fn world_inline_override_round_trips_behaviour_merge() {
        let template_toml = std::fs::read_to_string("assets/entities/ship_harrow_destroyer.toml")
            .expect("ship_harrow_destroyer.toml must exist");
        let cache = make_cache("assets/entities/ship_harrow_destroyer.toml", &template_toml);

        let world_toml = r#"
[[entity]]
template_path = "assets/entities/ship_harrow_destroyer.toml"
name          = "raider_alpha"
transform     = { position = [150.0, 0.0, -20.0] }
overrides     = { behaviour = { waypoint_arrival_radius = 42.0 } }
"#;

        let world = crate::world::config::parse_world(world_toml).expect("world TOML must parse");
        assert_eq!(
            world.entities.len(),
            1,
            "expected exactly one [[entity]] block"
        );

        let inst = &world.entities[0];
        assert!(
            inst.overrides.is_some(),
            "overrides must round-trip through parse_world"
        );

        let config = resolve_entity(inst, &cache).unwrap();
        let beh = config
            .behaviour
            .expect("raider keeps behaviour section after merge");
        assert!(
            (beh.waypoint_arrival_radius - 42.0).abs() < 1e-6,
            "inline override must set waypoint_arrival_radius to 42.0"
        );
        assert!(
            !beh.doctrine.is_empty(),
            "template doctrine must survive an inline override"
        );
    }

    /// Regression for issue #838: an override on a system-bearing hull must
    /// preserve the template's whole `[[system]]` suite *and* apply a `faction`
    /// scalar and an inline (text-less) `[[behaviour.doctrine]]` directive.
    ///
    /// The bug was twofold, both in this resolve path: `EntityConfig::ship_config`
    /// is `#[serde(skip)]` so a plain `toml::to_string` dropped every system, and
    /// `DoctrineObjective.text` was required so a text-less inline directive made
    /// the merged config fail to re-parse. Either one left the resolved hull
    /// stripped of its systems (nothing under AI control) or reverted to the
    /// un-overridden template. `to_toml_value` + a defaulted `text` fix both.
    #[test]
    fn override_on_system_bearing_hull_preserves_systems_faction_and_textless_doctrine() {
        let template_toml = std::fs::read_to_string("assets/entities/alliance_destroyer.toml")
            .expect("alliance_destroyer.toml must exist");
        let cache = make_cache("assets/entities/alliance_destroyer.toml", &template_toml);
        let template_system_count = cache
            .get("assets/entities/alliance_destroyer.toml")
            .unwrap()
            .ship_config
            .as_ref()
            .expect("template has a ship_config")
            .systems
            .len();
        assert!(template_system_count > 0, "fixture must declare systems");

        let harrow = uuid::Uuid::parse_str("cccccccc-3333-4333-8333-cccccccccccc").unwrap();
        let override_value: toml::Value = toml::from_str(
            r#"
faction = "cccccccc-3333-4333-8333-cccccccccccc"
[behaviour]
[[behaviour.doctrine]]
id = "kill"
directive_kind = "Destroy"
base_priority = 80.0
"#,
        )
        .unwrap();

        let inst = WorldEntity {
            template_path: "assets/entities/alliance_destroyer.toml".to_string(),
            overrides: Some(override_value),
            ..Default::default()
        };
        let config = resolve_entity(&inst, &cache).expect("override must resolve");

        // Faction scalar overridden.
        assert_eq!(config.faction, Some(harrow), "faction override must apply");
        // Text-less inline doctrine merged in.
        let beh = config.behaviour.expect("behaviour must be present");
        assert!(
            beh.doctrine.iter().any(|d| d.id == "kill"),
            "inline Destroy doctrine must survive the merge"
        );
        // The whole system suite survived the round-trip — the regression.
        let ship = config
            .ship_config
            .expect("override-resolved hull must keep its ship_config");
        assert_eq!(
            ship.systems.len(),
            template_system_count,
            "every declared system must survive an override (issue #838)"
        );
        assert!(
            ship.systems.iter().any(|s| s.kind == "phaser_bank"),
            "the phaser bank must survive so the hull's tactical AI can run"
        );
    }

    #[test]
    fn assign_uuid_returns_valid_uuid() {
        let id = assign_uuid();
        assert!(
            uuid::Uuid::parse_str(&id).is_ok(),
            "assign_uuid should return a valid UUID v4"
        );
    }

    #[test]
    fn assign_uuid_returns_unique_values() {
        let a = assign_uuid();
        let b = assign_uuid();
        assert_ne!(a, b);
    }

    // ── TemplateLoader ───────────────────────────────────────────────────────

    /// Write a template TOML fixture to a unique temp path and return it.
    fn write_template_fixture(body: &str) -> String {
        use std::sync::atomic::{AtomicU32, Ordering};
        static C: AtomicU32 = AtomicU32::new(0);
        let tag = C.fetch_add(1, Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!("template_loader_fixture_{tag}.toml"));
        std::fs::write(&p, body).expect("write template fixture");
        p.to_string_lossy().into_owned()
    }

    // `r##"` (not `r#"`) — the `"#` in the colour literal would close a
    // single-hash raw string early.
    const VALID_TEMPLATE: &str = r##"
tags = ["npc"]

[appearance]
colour = "#ff8800"
size_min = 1.0
size_max = 2.0
"##;

    #[test]
    fn fs_template_loader_reads_and_parses_template_from_disk() {
        let path = write_template_fixture(VALID_TEMPLATE);
        let config = FsTemplateLoader
            .load_template(&path)
            .expect("valid template on disk must load");
        assert_eq!(config.tags, vec!["npc"]);
    }

    #[test]
    fn fs_template_loader_missing_path_returns_none() {
        let path = std::env::temp_dir().join("template_loader_definitely_absent.toml");
        assert!(
            FsTemplateLoader
                .load_template(&path.to_string_lossy())
                .is_none(),
            "a template that is not on disk must resolve to None"
        );
    }

    #[test]
    fn fs_template_loader_malformed_toml_returns_none() {
        let path = write_template_fixture("this is not = = valid toml");
        assert!(
            FsTemplateLoader.load_template(&path).is_none(),
            "a parse error must be swallowed into None, not panic"
        );
    }

    /// On native the config cache is always empty, so `WasmTemplateLoader` must
    /// fall through to the filesystem. This is the same fallback the WASM build
    /// compiles out.
    #[test]
    fn wasm_template_loader_falls_back_to_filesystem_on_native() {
        let path = write_template_fixture(VALID_TEMPLATE);
        let config = WasmTemplateLoader
            .load_template(&path)
            .expect("native fallback must read the template from disk");
        assert_eq!(config.tags, vec!["npc"]);
    }

    #[test]
    fn wasm_template_loader_missing_everywhere_returns_none() {
        let path = std::env::temp_dir().join("wasm_template_loader_definitely_absent.toml");
        assert!(
            WasmTemplateLoader
                .load_template(&path.to_string_lossy())
                .is_none(),
            "not in cache and not on disk must resolve to None"
        );
    }

    /// A hand-written fake proves the trait is object-safe and injectable —
    /// the entire reason this is a trait rather than a cfg-split free fn.
    /// #715's dispatch context will hold exactly this `&dyn TemplateLoader`.
    struct FakeTemplateLoader {
        templates: HashMap<String, String>,
    }

    impl TemplateLoader for FakeTemplateLoader {
        fn load_template(&self, path: &str) -> Option<EntityConfig> {
            EntityConfig::from_toml(self.templates.get(path)?).ok()
        }
    }

    /// Stand-in for #715's applier: takes the loader as a trait object.
    fn resolve_template_via(loader: &dyn TemplateLoader, path: &str) -> Option<EntityConfig> {
        loader.load_template(path)
    }

    #[test]
    fn dyn_template_loader_injection_resolves_via_fake() {
        let mut templates = HashMap::new();
        templates.insert(
            "assets/entities/fake.toml".to_string(),
            VALID_TEMPLATE.to_string(),
        );
        let fake = FakeTemplateLoader { templates };

        let config = resolve_template_via(&fake, "assets/entities/fake.toml")
            .expect("injected fake must serve its template");
        assert_eq!(config.tags, vec!["npc"]);

        assert!(
            resolve_template_via(&fake, "assets/entities/unknown.toml").is_none(),
            "injected fake must report unknown templates as None"
        );
    }

    // ── Composed templates on the on-demand native path (issue #869) ──────
    //
    // `FsTemplateLoader` is what resolves a template that no preload swept up
    // — a `spawn_entity` trigger naming a hull the world never instanced, for
    // one. If it did not resolve includes, a composed hull would spawn stripped
    // of everything its fragments supply, silently.

    const COMPOSED_FIXTURE: &str = "assets/entities/fragments/composed_escort.toml";

    #[test]
    fn fs_template_loader_resolves_a_composed_template_from_disk() {
        let config = FsTemplateLoader
            .load_template(COMPOSED_FIXTURE)
            .expect("a composed hull must load through the on-demand native path");
        assert_eq!(
            config.class.as_deref(),
            Some("escort"),
            "the hull's own field survives"
        );
        assert!(
            config
                .ship_config
                .as_ref()
                .is_some_and(|s| s.systems.iter().any(|sys| sys.kind == "helm_thrust")),
            "the shared fragment's system suite must reach the loaded config"
        );
        assert!(
            config
                .captain_console
                .as_ref()
                .and_then(|c| c.ai.as_ref())
                .is_some(),
            "the nested AI fragment's policy must reach the loaded config"
        );
    }

    /// The WASM loader's native fallback goes through the same resolution, so a
    /// composed hull behaves identically whichever loader a caller holds.
    #[test]
    fn wasm_template_loader_native_fallback_resolves_a_composed_template() {
        let via_wasm = WasmTemplateLoader
            .load_template(COMPOSED_FIXTURE)
            .expect("native fallback must resolve the include closure too");
        let via_fs = FsTemplateLoader.load_template(COMPOSED_FIXTURE).unwrap();
        assert_eq!(via_wasm, via_fs);
    }

    /// A composition failure collapses to `None` exactly as a parse failure
    /// does — this module cannot log, so the caller reports it (see the trait
    /// doc). What must never happen is a partially composed config.
    #[test]
    fn fs_template_loader_returns_none_for_a_broken_include() {
        let path = write_template_fixture("includes = [\"definitely_absent_fragment.toml\"]\n");
        assert!(
            FsTemplateLoader.load_template(&path).is_none(),
            "a missing fragment must not yield a config with the fragment's parts \
             silently missing"
        );
    }

    /// An instance override still applies on top of a COMPOSED template, and
    /// the two merges compose in the right order: fragments, then the hull,
    /// then the instance.
    #[test]
    fn an_instance_override_applies_on_top_of_a_composed_template() {
        let composed = FsTemplateLoader
            .load_template(COMPOSED_FIXTURE)
            .expect("fixture must load");
        let mut cache = HashMap::new();
        cache.insert(COMPOSED_FIXTURE.to_string(), composed);
        let cache = ConfigCache::from(cache);

        let inst = WorldEntity {
            template_path: COMPOSED_FIXTURE.to_string(),
            overrides: Some(
                toml::from_str("[behaviour]\nwaypoint_arrival_radius = 77.0\n").unwrap(),
            ),
            ..Default::default()
        };
        let config = resolve_entity(&inst, &cache).expect("override must resolve");
        let beh = config.behaviour.expect("composed hull keeps [behaviour]");
        assert!(
            (beh.waypoint_arrival_radius - 77.0).abs() < 1e-6,
            "the instance override wins over the composed template"
        );
        assert!(
            beh.doctrine.iter().any(|d| d.id == "destroy-hostiles"),
            "the fragment's doctrine survives the instance merge"
        );
        assert!(
            config
                .ship_config
                .is_some_and(|s| s.systems.iter().any(|sys| sys.kind == "helm_thrust")),
            "the fragment's systems survive the instance merge (issue #838's round trip)"
        );
    }

    /// The real impls must also be usable as `&dyn TemplateLoader`, not just
    /// the fake — otherwise injection buys nothing in production.
    #[test]
    fn real_loaders_are_usable_as_trait_objects() {
        let path = write_template_fixture(VALID_TEMPLATE);
        let loaders: Vec<&dyn TemplateLoader> = vec![&FsTemplateLoader, &WasmTemplateLoader];
        for loader in loaders {
            assert!(
                resolve_template_via(loader, &path).is_some(),
                "every real loader must resolve a valid on-disk template on native"
            );
        }
    }
}
