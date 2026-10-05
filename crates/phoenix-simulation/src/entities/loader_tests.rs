use super::*;
use crate::entities::config::EntityConfig;
use crate::entities::config_cache::ConfigCache;
use crate::world::load::MemoryTemplateLoader;
use std::collections::HashMap;

fn make_cache(path: &str, toml: &str) -> ConfigCache {
    let mut m = HashMap::new();
    m.insert(path.to_string(), EntityConfig::from_toml(toml).unwrap());
    ConfigCache::from(m)
}

/// A one-entry cache holding a SHIPPED hull, loaded exactly as the runtime
/// loads it — include closure resolved first (issue #906). Reading the file
/// and calling `from_toml` on it would silently start asserting on
/// unresolved text the day the hull declares `includes`.
fn cache_from_disk(path: &str) -> ConfigCache {
    let config = crate::entities::include_resolve::load_entity_config(path)
        .unwrap_or_else(|e| panic!("{path} must compose and parse: {e}"));
    let mut m = HashMap::new();
    m.insert(path.to_string(), config);
    ConfigCache::from(m)
}

/// A host that can serve nothing, so [`resolve_entity_via`] answers the
/// "cache and nothing else" question the removed `resolve_entity` used to
/// (issue #973 review, F5). Using it rather than `WasmTemplateLoader` keeps
/// these fixtures off the filesystem, which is what they were written
/// against; using `resolve_entity_via` rather than a second resolver keeps
/// them pinning the function production actually calls. `absence_is_final`
/// is irrelevant to `resolve_entity_via`, which never asks — pinned to the
/// browser's `blind` answer so nothing here can accidentally depend on it.
fn no_host() -> MemoryTemplateLoader {
    MemoryTemplateLoader::blind()
}

#[test]
fn resolve_missing_template_returns_err() {
    let cache = ConfigCache::from(HashMap::new());
    let inst = WorldEntity {
        template_path: "assets/entities/missing.toml".to_string(),
        ..Default::default()
    };
    let result = resolve_entity_via(&inst, &cache, &no_host());
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
    let config = resolve_entity_via(&inst, &cache, &no_host()).unwrap();
    assert_eq!(config.tags, vec!["asteroid"]);
}

/// Verifies that a world override on `[behaviour]` round-trips cleanly.
/// (#572) FSM dissolved — `initial_state`/`transition` fields removed.
/// Overrides now target `waypoint_arrival_radius` or doctrine entries.
/// This test confirms that overriding `waypoint_arrival_radius` merges
/// correctly while preserving the template's doctrine objectives.
#[test]
fn harrow_destroyer_behaviour_override_merges_arrival_radius() {
    let cache = cache_from_disk("assets/entities/ship_harrow_destroyer.toml");

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
    let config = resolve_entity_via(&inst, &cache, &no_host()).unwrap();
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
/// `parse_world` → `resolve_entity_via` and merges correctly.
/// (#572) FSM dissolved — override now targets `waypoint_arrival_radius`.
#[test]
fn world_inline_override_round_trips_behaviour_merge() {
    let cache = cache_from_disk("assets/entities/ship_harrow_destroyer.toml");

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

    let config = resolve_entity_via(inst, &cache, &no_host()).unwrap();
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
    let cache = cache_from_disk("assets/entities/alliance_destroyer.toml");
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
    let config = resolve_entity_via(&inst, &cache, &no_host()).expect("override must resolve");

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

// `assign_uuid_returns_valid_uuid` / `assign_uuid_returns_unique_values`
// were here. Both are gone with the function (issue #907); the properties
// they asserted are now `world_id`'s, where "valid uuid" is replaced by
// "parses back into its (namespace, tick, seq) tuple".

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

/// Stand-in for #715's applier: takes the loader as a trait object. Proves
/// the trait is object-safe and injectable — the entire reason this is a
/// trait rather than a cfg-split free fn.
fn resolve_template_via(loader: &dyn TemplateLoader, path: &str) -> Option<EntityConfig> {
    loader.load_template(path)
}

#[test]
fn dyn_template_loader_injection_resolves_via_fake() {
    let fake = MemoryTemplateLoader::from_toml([("assets/entities/fake.toml", VALID_TEMPLATE)]);

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
        overrides: Some(toml::from_str("[behaviour]\nwaypoint_arrival_radius = 77.0\n").unwrap()),
        ..Default::default()
    };
    let config = resolve_entity_via(&inst, &cache, &no_host()).expect("override must resolve");
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

// ── Authority over absence (issue #973) ──────────────────────────────────

/// The values behind the `unresolvable-template` gate. The browser answer
/// is the one that matters and the one `cargo test` can never observe, so
/// the structural guard is that
/// [`TemplateLoader::absence_is_final`] has no default — delete
/// `WasmTemplateLoader`'s override and the crate stops compiling on both
/// targets. This pins the values on top of that.
///
/// # Only the native arm is coverage
///
/// Said plainly rather than left to be inferred: **the `wasm32` arm below
/// is compiled by nothing.** CI builds wasm through `trunk build --release`,
/// which does not compile `#[cfg(test)]`, and no job runs
/// `cargo check --target wasm32-unknown-unknown --tests`. So the browser
/// assertion adds nothing beyond the missing-default guard above; it is
/// written as an assertion rather than a comment purely so that the claim
/// is already being checked if this crate ever grows a wasm test runner.
/// The same is true of its twin,
/// `entity_includes::tests::the_host_source_answers_absence_by_target` —
/// consistent precedent, not new coverage.
#[test]
fn the_host_loaders_answer_absence_by_target() {
    #[cfg(not(target_arch = "wasm32"))]
    {
        assert!(
            FsTemplateLoader.absence_is_final(),
            "the filesystem is authoritative"
        );
        assert!(
            WasmTemplateLoader.absence_is_final(),
            "on native the filesystem fallback makes absence a fact, so \
                 validation may hard-fail an unresolvable template"
        );
    }
    #[cfg(target_arch = "wasm32")]
    assert!(
        !WasmTemplateLoader.absence_is_final(),
        "in the browser an uncached template may still be in flight; calling \
             that final blanks the world permanently"
    );
}

/// The spawn-side half of #973's asymmetry fix: the lookup a `[[entity]]`
/// spawn performs is the cache PLUS the host loader, which is exactly what
/// the validator is handed. A template on disk but absent from the cache
/// used to resolve for the validator and not for the spawn.
#[test]
fn resolve_entity_via_falls_back_to_the_host_loader_behind_the_cache() {
    let path = write_template_fixture(VALID_TEMPLATE);
    let empty = ConfigCache::from(HashMap::new());
    let inst = WorldEntity {
        template_path: path.clone(),
        ..Default::default()
    };

    assert!(
        resolve_entity_via(&inst, &empty, &no_host()).is_err(),
        "precondition: the cache-only lookup cannot see it"
    );
    let config = resolve_entity_via(&inst, &empty, &WasmTemplateLoader)
        .expect("the host loader resolves it behind the empty cache");
    assert_eq!(config.tags, vec!["npc"]);
}

/// …and the caller's cache still wins over the host, so a layer's scoped
/// cache stays the authority for the layer it was built for.
#[test]
fn the_callers_cache_wins_over_the_host_loader() {
    let path = write_template_fixture(VALID_TEMPLATE);
    let cache = make_cache(&path, r#"tags = ["from-cache"]"#);
    let inst = WorldEntity {
        template_path: path,
        ..Default::default()
    };
    let config = resolve_entity_via(&inst, &cache, &WasmTemplateLoader).expect("resolves");
    assert_eq!(config.tags, vec!["from-cache"]);
}

/// A spawn loader is no more authoritative than the host behind it: a cache
/// hit says nothing about the templates the host still cannot see. Pinned
/// against a host that answers `false`, which is the browser's answer and
/// the one a native suite cannot otherwise reach.
#[test]
fn the_spawn_loader_inherits_the_hosts_authority() {
    let cache = make_cache("assets/entities/rock.toml", r#"tags = ["asteroid"]"#);
    let still_filling = MemoryTemplateLoader::still_filling();
    let loader = SpawnTemplateLoader {
        cache: &cache,
        host: &still_filling,
    };
    assert!(
        !loader.absence_is_final(),
        "a cache hit must not promote a host that is still filling"
    );
    assert!(loader.load_template("assets/entities/rock.toml").is_some());
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
