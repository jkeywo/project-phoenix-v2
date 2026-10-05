use crate::entities::config::EntityConfig;
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

// ── Mod-pack overlay stack: pure precedence (issues #760, #987) ──────
//
// The resolution + conflict logic is pure over `&[ActivePack]`, so these
// exercise it directly without touching the thread-local session state.

fn pack_with(id: &str, files: &[(&str, &str)]) -> super::ActivePack {
    let mut map = HashMap::new();
    for (p, t) in files {
        map.insert((*p).to_string(), (*t).to_string());
    }
    super::ActivePack {
        id: id.to_string(),
        name: format!("Pack {id}"),
        version: "1.0.0".to_string(),
        files: map,
        manifest_toml: String::new(),
        ..Default::default()
    }
}

#[test]
fn later_pack_wins_a_shared_path_and_reorder_flips_it() {
    // Both A and B carry the SAME path; B is loaded last, so B wins.
    let a = pack_with("a", &[("assets/entities/x.toml", "id = \"A\"\n")]);
    let b = pack_with("b", &[("assets/entities/x.toml", "id = \"B\"\n")]);
    let stack = vec![a.clone(), b.clone()];
    assert_eq!(
        super::overlay_lookup(&stack, "assets/entities/x.toml"),
        Some("id = \"B\"\n")
    );
    assert_eq!(
        super::overlay_source_in(&stack, "assets/entities/x.toml"),
        Some("b")
    );
    // Reordered so A is last → A now wins the same path (pure fn of order).
    let reordered = vec![b, a];
    assert_eq!(
        super::overlay_lookup(&reordered, "assets/entities/x.toml"),
        Some("id = \"A\"\n")
    );
    assert_eq!(
        super::overlay_source_in(&reordered, "assets/entities/x.toml"),
        Some("a")
    );
}

#[test]
fn a_path_only_one_pack_carries_resolves_to_that_pack() {
    let a = pack_with("a", &[("assets/entities/only_a.toml", "id = \"A\"\n")]);
    let b = pack_with("b", &[("assets/entities/only_b.toml", "id = \"B\"\n")]);
    let stack = vec![a, b];
    assert_eq!(
        super::overlay_lookup(&stack, "assets/entities/only_a.toml"),
        Some("id = \"A\"\n")
    );
    assert_eq!(
        super::overlay_lookup(&stack, "assets/entities/only_b.toml"),
        Some("id = \"B\"\n")
    );
    assert_eq!(
        super::overlay_lookup(&stack, "assets/entities/none.toml"),
        None
    );
}

#[test]
fn overlay_conflicts_names_winner_and_losers_in_load_order() {
    let a = pack_with("a", &[("assets/entities/x.toml", "A")]);
    let b = pack_with("b", &[("assets/entities/y.toml", "B")]);
    let c = pack_with("c", &[("assets/entities/x.toml", "C")]);
    // x is carried by a (oldest) and c (newest); y only by b → no conflict.
    let conflicts = super::overlay_conflicts(&[a, b, c]);
    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].path, "assets/entities/x.toml");
    assert_eq!(conflicts[0].winner, "c");
    assert_eq!(conflicts[0].losers, vec!["a".to_string()]);
}

// ── Mod-pack overlay stack: session state (issues #760, #987, #1366) ──
//
// Off the browser the stack is process-global (Bevy systems run on worker
// threads), so these take [`overlay_test_guard`] rather than relying on
// libtest's thread-per-test. Unique pack ids + paths on top of that, so a
// failure names one test rather than the order they happened to run in.

#[test]
fn pushing_pack_b_after_a_does_not_evict_a() {
    let _overlay = super::overlay_test_guard();
    super::push_mod_pack(pack_with(
        "sess-a",
        &[("assets/entities/__sess_x.toml", "A")],
    ));
    super::push_mod_pack(pack_with(
        "sess-b",
        &[("assets/entities/__sess_x.toml", "B")],
    ));
    // B wins the shared path, but A is still present (only shadowed).
    assert_eq!(
        super::mod_pack_overlay_get("assets/entities/__sess_x.toml"),
        Some("B".to_string())
    );
    assert_eq!(super::active_packs().len(), 2);
    assert_eq!(
        super::overlay_source("assets/entities/__sess_x.toml"),
        Some("sess-b".to_string())
    );

    // Removing B re-resolves precedence: A now wins the path (AC).
    assert!(super::remove_mod_pack("sess-b"));
    assert_eq!(
        super::mod_pack_overlay_get("assets/entities/__sess_x.toml"),
        Some("A".to_string())
    );
    assert_eq!(
        super::overlay_source("assets/entities/__sess_x.toml"),
        Some("sess-a".to_string())
    );

    super::clear_mod_pack_overlay();
    assert!(super::active_packs().is_empty());
    assert_eq!(
        super::mod_pack_overlay_get("assets/entities/__sess_x.toml"),
        None
    );
}

/// The whole reason the native stack is not a `thread_local!` (issue #1366).
///
/// `apply_mod_pack_choice` is an ordinary Bevy `Update` system on a
/// multi-threaded host, so it installs on whichever compute-pool worker took
/// the frame — while `feed_mod_pack_shelf`, the spawn path's fragment source
/// and the delivery thread all read from somewhere else. A per-thread stack
/// made every one of those an intermittent, silent miss: a panel reporting an
/// install nothing else could see.
#[test]
#[cfg(not(target_arch = "wasm32"))]
fn a_pack_installed_on_one_thread_is_visible_from_another() {
    let _overlay = super::overlay_test_guard();
    let path = "assets/entities/__cross_thread_x.toml";
    std::thread::spawn(move || {
        super::push_mod_pack(pack_with("cross-thread", &[(path, "installed elsewhere")]));
    })
    .join()
    .expect("the installing thread must not panic");

    assert_eq!(
        super::mod_pack_overlay_get(path),
        Some("installed elsewhere".to_string()),
        "a pack installed on a worker thread has to be visible to every reader"
    );
    assert_eq!(super::active_packs().len(), 1);
    assert_eq!(
        std::thread::spawn(|| super::active_packs().len())
            .join()
            .expect("the reading thread must not panic"),
        1,
        "and to a third thread, which is what the delivery thread is"
    );
}

#[test]
fn reorder_mod_packs_reassigns_the_winner() {
    let _overlay = super::overlay_test_guard();
    super::push_mod_pack(pack_with("ord-a", &[("assets/entities/__ord_x.toml", "A")]));
    super::push_mod_pack(pack_with("ord-b", &[("assets/entities/__ord_x.toml", "B")]));
    // Newest (ord-b) wins by default.
    assert_eq!(
        super::mod_pack_overlay_get("assets/entities/__ord_x.toml"),
        Some("B".to_string())
    );
    // Reorder so ord-a is last → ord-a wins.
    super::reorder_mod_packs(&["ord-b".to_string(), "ord-a".to_string()]);
    assert_eq!(
        super::mod_pack_overlay_get("assets/entities/__ord_x.toml"),
        Some("A".to_string())
    );
    super::clear_mod_pack_overlay();
}

// ── Overlay-backed script resolution (issue #988) ────────────────────
//
// A world's sibling `.rhai` resolves through the overlay first, and the
// resolved text is recorded in the content ledger so a script-carrying pack
// moves the content digest. Both are exercised through the loader's real
// resolution path (`world::script::load::lift_world_scripts`).

/// A fallback resolver serving a fixed sentinel, so a test can prove the
/// overlay won (or that the fallback was reached).
struct FixedFallback(Option<&'static str>);
impl crate::world::script::load::ScriptResolver for FixedFallback {
    fn read(&self, _path: &str) -> Option<String> {
        self.0.map(str::to_string)
    }
}

#[test]
fn a_pack_script_resolves_from_the_overlay_not_the_fallback() {
    use crate::world::script::load::lift_world_scripts;
    let _overlay = super::overlay_test_guard();
    crate::content_ledger::reset();

    // The overlay carries the sibling; the fallback would serve DIFFERENT
    // text, so a pass proves resolution went through the overlay.
    super::push_mod_pack(pack_with(
        "script-pack",
        &[("assets/worlds/on_combat.rhai", "fn from_pack(ctx) { }")],
    ));
    let world: toml::Value = toml::from_str(r#"script = "on_combat.rhai""#).unwrap();
    let resolver =
        super::OverlayScriptResolver::new(FixedFallback(Some("fn from_fallback(ctx) { }")));

    let (sources, findings) =
        lift_world_scripts("assets/worlds/combat_test.toml", &world, &resolver);
    assert!(findings.is_empty(), "{:?}", findings);
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0].path, "assets/worlds/on_combat.rhai");
    assert_eq!(
        sources[0].source, "fn from_pack(ctx) { }",
        "the overlay's script must win over the fallback"
    );

    super::clear_mod_pack_overlay();
    crate::content_ledger::reset();
}

#[test]
fn a_script_carrying_pack_moves_the_content_digest() {
    use crate::content_ledger;
    use crate::world::script::load::lift_world_scripts;

    let _overlay = super::overlay_test_guard();
    let world: toml::Value = toml::from_str(r#"script = "on_combat.rhai""#).unwrap();

    // WITHOUT a pack: the sibling cannot resolve (no overlay, no fallback),
    // so only the world itself is in the ledger.
    content_ledger::reset();
    content_ledger::record(
        "assets/worlds/combat_test.toml",
        r#"script = "on_combat.rhai""#,
    );
    let _ = lift_world_scripts(
        "assets/worlds/combat_test.toml",
        &world,
        &super::OverlayScriptResolver::new(FixedFallback(None)),
    );
    let without = content_ledger::snapshot().fold();

    // WITH a script-carrying pack: the resolver serves the script from the
    // overlay AND records it, so the fold covers the script too.
    super::push_mod_pack(pack_with(
        "digest-pack",
        &[("assets/worlds/on_combat.rhai", "fn on_x(ctx) { }")],
    ));
    content_ledger::reset();
    content_ledger::record(
        "assets/worlds/combat_test.toml",
        r#"script = "on_combat.rhai""#,
    );
    let _ = lift_world_scripts(
        "assets/worlds/combat_test.toml",
        &world,
        &super::OverlayScriptResolver::new(FixedFallback(None)),
    );
    let with = content_ledger::snapshot().fold();

    assert_ne!(
        without, with,
        "a pack-supplied script must move the content digest"
    );

    super::clear_mod_pack_overlay();
    content_ledger::reset();
}

// ── Composable-template preload contract (issue #869) ────────────────
//
// This is the browser host's half of "included dependencies are
// preloaded": JS delivers one TOML at a time, and after each delivery the
// loader must say either "resolved, cache it" or "fetch these first".
// Driving it here rather than in a browser is the whole reason the state
// machine is ungated.

/// Paths are unique per test so the ungated thread-locals stay
/// order-independent even under `--test-threads=1`.
fn preload_path(test_name: &str, leaf: &str) -> String {
    format!("assets/entities/__pre_{test_name}/{leaf}")
}

#[test]
fn a_composed_template_awaits_its_fragment_then_resolves() {
    super::clear_template_preload_state();
    let hull = preload_path("await", "hull.toml");
    let fragment = preload_path("await", "frag/core.toml");

    // 1. The world names the hull; JS fetches and delivers it.
    super::mark_entity_template(&hull);
    super::record_raw_template(
        &hull,
        "includes = [\"frag/core.toml\"]\nhull_id = \"H\"\n".to_string(),
    );

    let progress = super::drain_resolved_templates();
    assert!(
        progress.ready.is_empty(),
        "a template whose fragment has not arrived must not be cached yet"
    );
    assert_eq!(
        progress.fetch,
        vec![fragment.clone()],
        "the host must be told the canonical fragment path to fetch"
    );
    assert!(progress.errors.is_empty(), "absence is not an error");

    // 2. JS fetches and delivers the fragment.
    super::record_raw_template(
        &fragment,
        "class = \"escort\"\ntags = [\"npc\"]\n".to_string(),
    );
    let progress = super::drain_resolved_templates();
    assert!(progress.fetch.is_empty());
    assert_eq!(progress.ready.len(), 1);
    let (requested, resolved) = &progress.ready[0];
    assert_eq!(
        requested, &hull,
        "the config-cache key is the path the world asked for"
    );
    let config = resolved.parse().expect("the composed hull must be valid");
    assert_eq!(config.class.as_deref(), Some("escort"));
    assert_eq!(config.hull_id.as_deref(), Some("H"));
    assert_eq!(config.tags, vec!["npc"]);
}

/// A fragment is authoring input. Delivering its text must never produce a
/// runtime template — only paths the world (or a nested reference) asked
/// for become config-cache entries.
#[test]
fn a_delivered_fragment_is_never_offered_as_an_entity_template() {
    super::clear_template_preload_state();
    let fragment = preload_path("frag_only", "core.toml");
    super::record_raw_template(&fragment, "class = \"escort\"\n".to_string());

    let progress = super::drain_resolved_templates();
    assert!(progress.ready.is_empty());
    assert!(progress.errors.is_empty());
    assert!(
        super::is_raw_template_delivered(&fragment),
        "its text is held for composition, but it is not a template"
    );
}

#[test]
fn an_entity_template_settles_exactly_once() {
    super::clear_template_preload_state();
    let hull = preload_path("once", "hull.toml");
    super::mark_entity_template(&hull);
    super::record_raw_template(&hull, "class = \"solo\"\n".to_string());

    assert_eq!(super::drain_resolved_templates().ready.len(), 1);
    assert!(
        super::drain_resolved_templates().ready.is_empty(),
        "a settled template must not be re-emitted on every later delivery"
    );
}

#[test]
fn a_cycle_settles_as_an_error_rather_than_an_endless_fetch() {
    super::clear_template_preload_state();
    let a = preload_path("cycle", "a.toml");
    let b = preload_path("cycle", "b.toml");
    super::mark_entity_template(&a);
    super::record_raw_template(&a, "includes = [\"b.toml\"]\n".to_string());
    super::record_raw_template(&b, "includes = [\"a.toml\"]\n".to_string());

    let progress = super::drain_resolved_templates();
    assert!(progress.ready.is_empty());
    assert!(
        progress.fetch.is_empty(),
        "a cycle is never resolved by fetching more"
    );
    assert_eq!(progress.errors.len(), 1);
    assert_eq!(progress.errors[0].category(), "include-cycle");
    assert!(
        super::drain_resolved_templates().errors.is_empty(),
        "the failure is reported once, then settled"
    );
}

/// The refetch guard `queue_and_fire` relies on: a fragment never enters
/// the config cache, so cache membership cannot be what stops it being
/// requested once per including hull.
#[test]
fn delivered_raw_text_is_the_refetch_guard_for_fragments() {
    super::clear_template_preload_state();
    let fragment = preload_path("guard", "core.toml");
    assert!(!super::is_raw_template_delivered(&fragment));
    super::record_raw_template(&fragment, "class = \"x\"\n".to_string());
    assert!(super::is_raw_template_delivered(&fragment));
    assert!(
        super::is_raw_template_delivered(&format!("./{fragment}")),
        "the guard is keyed canonically, so a differently spelled path still hits"
    );
}

/// Two hulls sharing one fragment: the second must resolve off the text the
/// first delivery already brought in, with no further fetch.
#[test]
fn a_shared_fragment_is_fetched_once_for_many_hulls() {
    super::clear_template_preload_state();
    let a = preload_path("shared", "a.toml");
    let b = preload_path("shared", "b.toml");
    let fragment = preload_path("shared", "core.toml");
    for hull in [&a, &b] {
        super::mark_entity_template(hull);
        super::record_raw_template(hull, "includes = [\"core.toml\"]\n".to_string());
    }

    let progress = super::drain_resolved_templates();
    assert_eq!(
        progress.fetch,
        vec![fragment.clone()],
        "both hulls want the same fragment, and it is requested once"
    );

    super::record_raw_template(&fragment, "class = \"shared\"\n".to_string());
    let progress = super::drain_resolved_templates();
    assert_eq!(progress.ready.len(), 2);
    for (_, resolved) in &progress.ready {
        assert_eq!(
            resolved.value.get("class").unwrap().as_str(),
            Some("shared")
        );
    }
}

/// The browser walks the closure the same way the filesystem does. Same
/// fixture files, same resolved bytes — that is what "resolution must be
/// identical on native and WASM" means operationally.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn the_browser_walk_of_the_shipped_fixture_matches_the_filesystem_walk() {
    super::clear_template_preload_state();
    const HULL: &str = "assets/entities/fragments/composed_escort.toml";
    let native = crate::entities::include_resolve::resolve_from_disk(HULL)
        .expect("the fixture hull resolves off disk");

    // Simulate the browser: only the hull's own text is delivered first,
    // and every further path comes from what the loader asks for.
    super::mark_entity_template(HULL);
    super::record_raw_template(
        HULL,
        crate::repo_fixtures::fs::read_to_string(HULL).unwrap(),
    );
    let mut fetches = 0;
    let resolved = loop {
        let progress = super::drain_resolved_templates();
        assert!(
            progress.errors.is_empty(),
            "unexpected composition error: {:?}",
            progress.errors
        );
        if let Some((_, resolved)) = progress.ready.into_iter().next() {
            break resolved;
        }
        assert!(
            !progress.fetch.is_empty(),
            "neither ready nor awaiting anything — the walk stalled"
        );
        for path in progress.fetch {
            fetches += 1;
            let body = crate::repo_fixtures::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("the loader asked for {path}, which must exist: {e}"));
            super::record_raw_template(&path, body);
        }
        assert!(fetches < 16, "closure walk did not terminate");
    };
    assert_eq!(
            fetches, 3,
            "the hull's fragment and that fragment's own TWO fragments (the Captain              policy and the fleet-baseline AI declarations #885b stage 5d made              mandatory), each fetched once"
        );
    assert_eq!(
        resolved.toml, native.toml,
        "the browser and the filesystem must resolve to the same bytes"
    );
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

// ── Pre-load catalogue enrichment ────────────────────────────────────
//
// The store the browser fills BEFORE any world is activated, so the ship
// picker's hull cards can carry a class, registry, mass and power rating.
// Ungated for the same reason the preload's own stores are, and asserted
// here rather than only in a browser: these are the same decisions
// `drain_resolved_templates` makes one screen later — what a delivery
// reveals, what an absence means, what never becomes resolvable — plus the
// one thing that must NEVER hold, a catalogue answer satisfying a spawn.

/// Per-test paths, for the reason [`preload_path`] gives; the mod-pack
/// overlay one of these touches is process-global on native.
fn catalog_path(test_name: &str, leaf: &str) -> String {
    format!("assets/entities/__cat_{test_name}/{leaf}")
}

#[test]
fn a_catalogue_root_resolves_once_its_fragment_arrives_in_a_later_round() {
    super::clear_catalog_templates();
    let hull = catalog_path("rounds", "hull.toml");
    let fragment = catalog_path("rounds", "frag/core.toml");

    // Round 1: the hull itself. Its fragment has not been fetched, so the
    // card is not enriched yet and the host is told what to fetch next.
    let awaiting = super::push_catalog_template(
        hull.clone(),
        "includes = [\"frag/core.toml\"]\nhull_id = \"AEV-0001\"\n".to_string(),
        true,
    );
    assert_eq!(awaiting, vec![fragment.clone()]);
    assert!(super::catalog_entity_config(&hull).is_none());

    // Round 2: the fragment. Now the closure is complete.
    let awaiting = super::push_catalog_template(
        fragment,
        "class = \"destroyer\"\npower_rating = 70\n".to_string(),
        false,
    );
    assert!(awaiting.is_empty(), "nothing left to fetch");
    let cfg = super::catalog_entity_config(&hull).expect("the composed hull is cached");
    assert_eq!(cfg.class.as_deref(), Some("destroyer"));
    assert_eq!(cfg.hull_id.as_deref(), Some("AEV-0001"));
    assert_eq!(cfg.power_rating, Some(70));
}

/// The line issue #917 draws: a hull cached for the CARD must not be
/// spawnable. `get_cached_entity_config` is the spawn lookup
/// (`entities::loader::WasmTemplateLoader`, `server::reference_grid`), and
/// the catalogue store hangs off `catalog_entity_config` alone.
#[test]
fn the_catalogue_store_never_answers_the_spawn_lookup() {
    super::clear_catalog_templates();
    let hull = catalog_path("spawn_line", "hull.toml");
    super::push_catalog_template(hull.clone(), "class = \"cruiser\"\n".to_string(), true);

    assert!(
        super::catalog_entity_config(&hull).is_some(),
        "the card reads it"
    );
    assert!(
        super::get_cached_entity_config(&hull).is_none(),
        "a spawn must not: the curation deliberately left hulls out of the preload"
    );
}

/// A fetch that brought nothing is delivered as an EMPTY string, the way
/// `handleConfigRequest` calls `wasm_load_config(path, \'\')` on a 404. With
/// nothing in the overlay either that must be a NO-OP: the fragment stays
/// "still to fetch" rather than composing as present-and-empty.
#[test]
fn an_empty_delivery_with_no_overlay_copy_is_a_no_op() {
    super::clear_catalog_templates();
    let hull = catalog_path("empty_noop", "hull.toml");
    let fragment = catalog_path("empty_noop", "frag/core.toml");
    super::push_catalog_template(
        hull.clone(),
        "includes = [\"frag/core.toml\"]\nhull_id = \"H\"\n".to_string(),
        true,
    );

    let awaiting = super::push_catalog_template(fragment.clone(), String::new(), false);
    assert_eq!(
        awaiting,
        vec![fragment],
        "an absent fragment is still something to fetch, not an empty one"
    );
    assert!(
        super::catalog_entity_config(&hull).is_none(),
        "the card stays label-only rather than composing off a fragment that never arrived"
    );
}

/// The case the empty delivery EXISTS for: a mod pack\'s own hull lives in
/// the session overlay and has no URL at all, so its fetch always fails.
#[test]
fn a_pack_supplied_hull_composes_from_the_overlay_when_the_fetch_brought_nothing() {
    let _overlay = super::overlay_test_guard();
    super::clear_catalog_templates();
    let hull = catalog_path("pack_hull", "raider.toml");
    super::push_mod_pack(pack_with(
        "cat-pack",
        &[(hull.as_str(), "class = \"raider\"\nhull_id = \"MOD-1\"\n")],
    ));

    let awaiting = super::push_catalog_template(hull.clone(), String::new(), true);
    assert!(awaiting.is_empty());
    let cfg = super::catalog_entity_config(&hull).expect("the pack\'s own hull enriches");
    assert_eq!(cfg.class.as_deref(), Some("raider"));
    assert_eq!(cfg.hull_id.as_deref(), Some("MOD-1"));
}

#[test]
fn a_cycle_leaves_the_card_label_only_rather_than_fetching_for_ever() {
    super::clear_catalog_templates();
    let a = catalog_path("cycle", "a.toml");
    let b = catalog_path("cycle", "b.toml");
    super::push_catalog_template(a.clone(), "includes = [\"b.toml\"]\n".to_string(), true);
    let awaiting = super::push_catalog_template(b, "includes = [\"a.toml\"]\n".to_string(), false);

    assert!(
        awaiting.is_empty(),
        "a cycle is never resolved by fetching more"
    );
    assert!(super::catalog_entity_config(&a).is_none());
}

#[test]
fn a_root_that_will_not_parse_leaves_the_card_label_only() {
    super::clear_catalog_templates();
    // Valid TOML, invalid entity: `EntityConfig` is `deny_unknown_fields`.
    let unknown = catalog_path("unparseable", "unknown_field.toml");
    super::push_catalog_template(unknown.clone(), "not_a_field = 1\n".to_string(), true);
    assert!(super::catalog_entity_config(&unknown).is_none());

    // Not even TOML: the composition step itself refuses it.
    let malformed = catalog_path("unparseable", "malformed.toml");
    let awaiting = super::push_catalog_template(malformed.clone(), "= not toml".to_string(), true);
    assert!(awaiting.is_empty());
    assert!(super::catalog_entity_config(&malformed).is_none());
}

#[test]
fn clearing_empties_the_raw_text_the_roots_and_the_resolved_cache() {
    super::clear_catalog_templates();
    let hull = catalog_path("clear", "hull.toml");
    let fragment = catalog_path("clear", "frag/core.toml");
    super::push_catalog_template(
        hull.clone(),
        "includes = [\"frag/core.toml\"]\n".to_string(),
        true,
    );
    super::push_catalog_template(fragment.clone(), "class = \"escort\"\n".to_string(), false);
    assert!(super::catalog_entity_config(&hull).is_some());

    super::clear_catalog_templates();
    assert!(
        super::catalog_entity_config(&hull).is_none(),
        "the resolved cache is empty"
    );
    // The roots are gone, so re-delivering the FRAGMENT alone resolves
    // nothing — and the raw text is gone, so the hull needs fetching again.
    let awaiting =
        super::push_catalog_template(fragment, "class = \"escort\"\n".to_string(), false);
    assert!(awaiting.is_empty(), "no root is waiting on anything");
    assert!(super::catalog_entity_config(&hull).is_none());
}

// ── Faction registry from effective content (issue #1474) ────────────

const ALPHA: &str = "aaaaaaaa-0000-4000-8000-00000000000a";
const BETA: &str = "bbbbbbbb-0000-4000-8000-00000000000b";
const GAMMA: &str = "cccccccc-0000-4000-8000-00000000000c";

fn faction_uuid(text: &str) -> uuid::Uuid {
    uuid::Uuid::parse_str(text).unwrap()
}

/// A private directory under the OS temp root; the pid and clock keep
/// parallel test binaries apart without an OS-entropy uuid.
fn private_directory(name: &str) -> std::path::PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or_default();
    std::env::temp_dir().join(format!("phoenix-{name}-{}-{nanos}", std::process::id()))
}

#[test]
fn faction_registry_reads_the_directory_then_the_pack_overlays_in_stack_order() {
    let directory = private_directory("factions");
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(
        directory.join("a.toml"),
        format!("uuid = \"{ALPHA}\"\nname = \"Alpha\"\n"),
    )
    .unwrap();
    std::fs::write(
        directory.join("b.toml"),
        format!("uuid = \"{BETA}\"\nname = \"Beta\"\nenemies = [\"{ALPHA}\"]\n"),
    )
    .unwrap();
    std::fs::write(directory.join("broken.toml"), "uuid = 1\n").unwrap();
    std::fs::write(directory.join("notes.txt"), "not a faction").unwrap();

    let from_disk = super::faction_registry_from(&directory, &[]);
    assert_eq!(from_disk.len(), 2, "the unparsable file is skipped");
    assert_eq!(from_disk.get(&faction_uuid(ALPHA)).unwrap().name, "Alpha");
    assert!(crate::ai::faction::is_enemy(
        Some(faction_uuid(BETA)),
        Some(faction_uuid(ALPHA)),
        &from_disk
    ));

    let older = pack_with(
        "older",
        &[(
            "assets/factions/alpha.toml",
            &format!("uuid = \"{ALPHA}\"\nname = \"AlphaOlder\"\n"),
        )],
    );
    let newer = pack_with(
        "newer",
        &[
            (
                "assets/factions/alpha.toml",
                &format!("uuid = \"{ALPHA}\"\nname = \"AlphaNewer\"\n"),
            ),
            (
                "assets/factions/gamma.toml",
                &format!("uuid = \"{GAMMA}\"\nname = \"Gamma\"\n"),
            ),
            ("assets/worlds/gamma.toml", "[global]\n"),
        ],
    );
    let overlaid = super::faction_registry_from(&directory, &[older, newer]);
    assert_eq!(overlaid.len(), 3);
    assert_eq!(
        overlaid.get(&faction_uuid(ALPHA)).unwrap().name,
        "AlphaNewer",
        "the newest pack wins a shared uuid"
    );
    assert_eq!(overlaid.get(&faction_uuid(GAMMA)).unwrap().name, "Gamma");
    assert_eq!(overlaid.get(&faction_uuid(BETA)).unwrap().name, "Beta");

    let _ = std::fs::remove_dir_all(&directory);
    let absent = super::faction_registry_from(&directory, &[]);
    assert_eq!(
        absent.len(),
        4,
        "an absent directory falls back to the compiled-in four"
    );
    assert!(absent.uuid_by_name("Alliance").is_some());
    assert!(absent.uuid_by_name("Requiem").is_some());
}

#[test]
fn the_checkout_registry_matches_the_compiled_in_set_with_no_pack_installed() {
    // Under `cargo test` the cwd is the crate root, whose assets/factions
    // holds exactly the four compiled-in files, so the effective registry
    // and the fallback describe the same set whichever arm was taken. This
    // is a smoke check of the thin wrapper from the crate root, not the
    // proof that the directory is read — that proof is the temp-directory
    // test above. The guard keeps a pack another test installs out of it.
    let _overlay = super::overlay_test_guard();
    let effective = super::get_faction_registry();
    let mut compiled = crate::ai::faction::FactionRegistry::new();
    super::insert_built_in_factions(&mut compiled);
    assert_eq!(effective.len(), compiled.len());
    for faction in compiled.iter() {
        assert_eq!(effective.get(&faction.uuid), Some(faction));
    }
}

#[test]
fn the_faction_directory_beside_a_template_directory_is_its_sibling() {
    assert_eq!(
        super::faction_directory_beside(std::path::Path::new("assets/entities")),
        std::path::PathBuf::from("assets/factions")
    );
    let directory = private_directory("beside");
    let templates = directory.join("assets").join("entities");
    std::fs::create_dir_all(&templates).unwrap();
    std::fs::create_dir_all(directory.join("assets").join("factions")).unwrap();
    std::fs::write(
        directory.join("assets").join("factions").join("only.toml"),
        format!("uuid = \"{ALPHA}\"\nname = \"Alpha\"\n"),
    )
    .unwrap();
    let registry = super::faction_registry_from(&super::faction_directory_beside(&templates), &[]);
    assert_eq!(
        registry.len(),
        1,
        "the factions beside the templates, not the cwd's"
    );
    assert_eq!(registry.get(&faction_uuid(ALPHA)).unwrap().name, "Alpha");
    let _ = std::fs::remove_dir_all(&directory);
}

use crate::entities::include_resolve::ParseEntityTemplate as _;
