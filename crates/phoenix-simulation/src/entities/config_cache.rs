// WASM/JS bridge for config preloading — all public functions are #[wasm_bindgen] exports.
//
// This module implements the preload sequence for the unified world TOML,
// entity templates, and faction TOML files. It uses thread-local storage
// to cache configs before the Bevy app is initialised.
//
// Preload sequence:
// 1. JS calls set_config_request_callback(cb)
// 2. JS fetches the chosen world TOML
// 3. JS calls wasm_load_world(path, toml_str, curated_ships) which parses it
//    via the unified `world::config::parse_world` pass and stores the
//    resulting `WorldConfig`. `curated_ships` is the locked scenario's
//    playable-hull allowlist (issue #917) — empty when unrestricted.
// 4. wasm_load_world fires the callback for each referenced entity template
//    path, restricted to `curated_ships` for available_ships (issue #917)
// 5. For each entity path, JS fetches the TOML and calls wasm_load_config(path, toml_str)
// 6. When all pending configs are loaded, returns Ok(true) and JS calls wasm_init()

#[cfg(target_arch = "wasm32")]
use {
    crate::entities::config::EntityConfig, bevy::prelude::*, js_sys::Function,
    std::collections::VecDeque, wasm_bindgen::prelude::*,
};

// The sidecar inbox and template preload state share RefCell/HashMap across
// targets; the browser-only preload sets additionally need HashSet.
use std::cell::RefCell;
use std::collections::HashMap;
#[cfg(target_arch = "wasm32")]
use std::collections::HashSet;

// ── Pure helpers (native + wasm) ─────────────────────────────────────────────

/// Collect template paths nested inside an `EntityConfig` that the preload
/// pipeline must also fetch.
///
/// When an entity template carries an `[asteroid_field]` section, its
/// `asteroid_type_paths` and `cosmetic_type_paths` reference further entity
/// templates (the asteroid variants). Those need to be enqueued for fetch
/// alongside the top-level instance template paths.
///
/// # Its sibling: the include closure
///
/// This walks the transitive references a *parsed* config makes. Issue #869
/// adds a second kind of transitive reference — a template's ordered
/// `includes` — which cannot be discovered from a parsed config, because the
/// key never reaches `EntityConfig` (it is `deny_unknown_fields`, and an
/// include is authoring input, not runtime state). That closure is walked from
/// the raw TOML instead, by [`drain_resolved_templates`] below, and the two
/// feed the SAME `PENDING_QUEUE`/`IN_FLIGHT` pair so the preload-complete
/// condition is unchanged.
pub fn nested_template_paths(config: &crate::entities::config::EntityConfig) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(field) = &config.asteroid_field {
        for p in &field.asteroid_type_paths {
            out.push(p.path().to_string());
        }
        for p in &field.cosmetic_type_paths {
            out.push(p.path().to_string());
        }
    }
    out
}

// ── Thread-local preload state ────────────────────────────────────────────────

#[cfg(target_arch = "wasm32")]
thread_local! {
    /// Cache of loaded entity configs by path.
    static CONFIG_CACHE: RefCell<HashMap<String, EntityConfig>> = RefCell::new(HashMap::new());
    static CONFIG_REVISION: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };

    /// Queue of entity paths that need to be loaded.
    static PENDING_QUEUE: RefCell<VecDeque<String>> = RefCell::new(VecDeque::new());

    /// Set of paths currently being fetched to prevent duplicates.
    static IN_FLIGHT: RefCell<HashSet<String>> = RefCell::new(HashSet::new());

    /// Cache of loaded faction configs by uuid.
    static FACTION_REGISTRY: RefCell<crate::ai::faction::FactionRegistry> =
        RefCell::new(crate::ai::faction::FactionRegistry::new());

    /// JS callback for requesting config fetches. Set by set_config_request_callback.
    static CONFIG_REQUEST_CB: RefCell<Option<Function>> = const { RefCell::new(None) };

    /// Whether all pending configs have been loaded.
    static PRELOAD_COMPLETE: RefCell<bool> = const { RefCell::new(false) };
    static WORLD_PRELOAD_ROOT: RefCell<Option<(String, String, Vec<String>)>> = const { RefCell::new(None) };
    static WORLD_PRELOAD_PENDING: RefCell<bool> = const { RefCell::new(false) };
    static PRELOAD_FAILURE: RefCell<Option<String>> = const { RefCell::new(None) };

    /// The loaded unified `WorldConfig` (PRD #337/#338 slice 1).
    /// Set by `wasm_load_world` from a single-pass parse of the world TOML.
    /// Sole world storage after PRD #341 — the old per-half thread-locals
    /// were retired.
    static WORLD_CONFIG: RefCell<Option<crate::world::config::WorldConfig>> =
        const { RefCell::new(None) };

    /// Durable base-content cache for runtime-loaded world TOML and sibling
    /// Rhai source pushed by JS via `wasm_push_world_toml`.
    ///
    /// A successful HTTP response remains readable for the browser session:
    /// additive-layer unload/reload and the asset preloader may both need the
    /// same bytes. Mod-pack overlay content is resolved above this cache at
    /// read time and is deliberately never copied into it.
    static FETCHED_WORLD_SOURCE: RefCell<HashMap<String, String>> =
        RefCell::new(HashMap::new());

    /// Terminal failures reported by the JS fetch edge. Kept distinct from the
    /// content map because an empty Rhai source is valid content, not failure.
    static WORLD_FETCH_FAILURES: RefCell<HashMap<String, String>> =
        RefCell::new(HashMap::new());

    /// Optional JS callback for requesting a runtime world TOML fetch.
    /// Set by `set_world_fetch_callback` from server.html.
    static WORLD_FETCH_CB: RefCell<Option<Function>> = const { RefCell::new(None) };

    /// Set of paths already requested via `request_world_fetch` to avoid
    /// duplicate JS fetch calls.
    static WORLD_FETCH_REQUESTED: RefCell<HashSet<String>> =
        RefCell::new(HashSet::new());

    /// The base scenario manifest TOML (`assets/scenarios.toml`), pushed by JS
    /// during preload via `wasm_push_scenario_manifest`. Read by the pre-load
    /// scenario/ship catalog accessor before any world is activated (issue
    /// #754).
    static SCENARIO_MANIFEST_TOML: RefCell<Option<String>> =
        const { RefCell::new(None) };

    /// Set of sidecar paths already requested to avoid duplicate JS fetches.
    static SIDECAR_FETCH_REQUESTED: RefCell<HashSet<String>> =
        RefCell::new(HashSet::new());

    /// Canonical primary-rig paths discovered from preloaded entity templates
    /// but not yet delivered. This participates in the same one-shot preload
    /// completion as entity TOML so boot cannot freeze content identity first.
    static SIDECAR_PRELOAD_PENDING: RefCell<HashSet<String>> =
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

// ── Composable-template preload (issue #869) ─────────────────────────────────
//
// Templates may declare an ordered `includes` list, and the fragments they name
// have to be fetched before the declaring template can be resolved, validated
// and cached. That is a *closure walk over raw TOML*, not over parsed configs,
// so it cannot live inside `wasm_load_config`'s parse branch the way
// `nested_template_paths` does.
//
// Ungated (native + wasm) for the same reason as the sidecar inbox above: the
// browser is the only host that drives it, but the decision — "resolve now, or
// fetch these first" — is the loader contract, and it must be assertable under
// `cargo test`. `thread_local!` is also what makes that safe: libtest runs each
// `#[test]` on its own thread, so these maps are per-test in the same way they
// are per-page on WASM.

pub use phoenix_content::template_preload::*;

// ── Session-scoped mod-pack overlay STACK (issues #760, #987) ────────────────
//
// A VALID host mod-pack upload installs an in-memory, exact-path -> TOML map
// plus the pack's scenario manifest. Issue #987 turns the single overlay slot
// into an ORDERED STACK ([`ACTIVE_PACKS`]): each accepted pack is PUSHED, and
// installing pack B after pack A never evicts A — both stay resolvable, and any
// path only A carries still resolves to A.
//
// ## Precedence policy (deterministic, pure function of load order)
//
// The stack is ordered OLDEST → NEWEST: index 0 is the earliest-loaded pack, the
// last element the most recently loaded. Resolution walks the stack from the
// LAST-LOADED END FIRST ([`overlay_lookup`]), so a LATER-loaded pack WINS any
// authored path an earlier pack also carries; base content (the normal HTTP
// fetch) loses to every pack. Nothing is cached — every lookup walks the live
// stack — so REMOVING a pack automatically re-resolves precedence (the next pack
// down that carries the path becomes the winner), and REORDERING the stack is the
// only other way a path's winner changes. Both are pure functions of the
// resulting order.
//
// Both content-resolution channels (world/catalog fetch and entity/faction
// config request) consult [`mod_pack_overlay_get`] FIRST, returning the WINNING
// pack's content for any overridden authored path and falling back to the normal
// HTTP fetch otherwise (AC2). Native resolves the same two channels the same way
// — `include_resolve::FsFragmentSource`, `world::load::OverlayFsReader` and
// `delivery::serve::ManifestSource::resolve_world` all consult the overlay before
// the filesystem — which is what lets a native host's accepted pack widen its
// lobby catalogue and load its own worlds. The overlay only ever ADDS or REPLACES
// supported authored paths — it never touches disk. It is host-session-scoped: a
// page reload clears the browser's stack naturally, and [`clear_mod_pack_overlay`]
// discards the WHOLE stack for the same-page return-to-lobby / next-upload seam
// (AC4).
//
// Ungated (native + wasm) so the ordered-stack resolution + conflict computation
// is unit-testable on native without dragging in wasm-bindgen, exactly like the
// sidecar inbox above. The resolution and conflict logic are PURE functions over
// `&[ActivePack]` ([`overlay_lookup`], [`overlay_source_in`],
// [`overlay_conflicts`]); the session-state wrappers over them are thin, and the
// storage they wrap is per-target — see the note above [`ACTIVE_PACKS`].

pub use phoenix_content::overlay::*;

// ── Overlay-backed script resolution (issue #988) ────────────────────────────
//
// A world's sibling `.rhai` (`world::script::load`) resolves through the mod-pack
// overlay FIRST, then a fallback — the script-loader twin of the world/config
// channels above, which already consult `mod_pack_overlay_get` before the normal
// fetch (#760 AC2). An accepted pack may carry `.rhai` (validated at upload by
// `world::mod_pack::validate_pack_scripts`), and this is what makes that script
// actually resolve at load time instead of being fetched over HTTP.

/// A [`ScriptResolver`](crate::world::script::load::ScriptResolver) that reads a
/// world's sibling script from the mod-pack overlay stack first, then `fallback`
/// (issue #988).
///
/// Every script it resolves — whether from the overlay or the fallback — is
/// recorded in [`content_ledger`](crate::content_ledger) under its exact path.
/// That is what makes a scenario loaded WITH a script-carrying pack fold a
/// DIFFERENT content digest than the same scenario without it, so a pack's script
/// can no longer drift under snapshot/replay determinism (issue #988, following
/// the entity-template coverage #935 added).
pub struct OverlayScriptResolver<R> {
    /// The resolver consulted when no active pack carries the path — the live
    /// session's HTTP/config-cache fetch, or a test fake.
    pub fallback: R,
}

impl<R> OverlayScriptResolver<R> {
    /// Wrap `fallback` so the overlay is tried first.
    pub fn new(fallback: R) -> Self {
        Self { fallback }
    }
}

impl<R: crate::world::script::load::ScriptResolver> crate::world::script::load::ScriptResolver
    for OverlayScriptResolver<R>
{
    fn read(&self, path: &str) -> Option<String> {
        let source = mod_pack_overlay_get(path).or_else(|| self.fallback.read(path));
        if let Some(text) = &source {
            // Record what the loader actually consumed (pack or fallback) so the
            // content digest covers pack-supplied scripts (issue #988).
            crate::content_ledger::record(path, text);
        }
        source
    }
}

// ── Production script-resolver fallbacks (issue #984, Rhai M6 phase 2a) ───────
//
// The concrete fallback `OverlayScriptResolver` wraps when no active mod pack
// carries a world's sibling `.rhai`. Per-target, because the two hosts read a
// sibling file differently — native/headless from the filesystem, wasm from the
// config-cache's JS-delivered content. Inline `[script]` blocks are lifted from
// the world TOML directly (`world::script::load::lift_world_scripts`) and never
// reach a resolver; sibling scripts use the asynchronous fetch state below.

/// Native/headless fallback: read a world's sibling `.rhai` from the filesystem.
#[cfg(not(target_arch = "wasm32"))]
pub struct FsScriptFallback;

#[cfg(not(target_arch = "wasm32"))]
impl crate::world::script::load::ScriptResolver for FsScriptFallback {
    fn read(&self, path: &str) -> Option<String> {
        crate::content_fs::read_to_string(path).ok()
    }
}

/// The production script resolver for this target: the mod-pack overlay first
/// (folding pack scripts into the content digest, issue #988), then the
/// filesystem.
#[cfg(not(target_arch = "wasm32"))]
pub fn production_script_resolver() -> OverlayScriptResolver<FsScriptFallback> {
    OverlayScriptResolver::new(FsScriptFallback)
}

/// WASM fallback: read a sibling `.rhai` from the durable JS-delivered base
/// content cache. The overlay-aware wrapper below resolves active mod-pack
/// content first. `None` here means the asynchronous sibling request has not
/// completed (or failed) and blocks activation at the caller.
#[cfg(target_arch = "wasm32")]
pub struct ConfigCacheScriptFallback;

#[cfg(target_arch = "wasm32")]
impl crate::world::script::load::ScriptResolver for ConfigCacheScriptFallback {
    fn read(&self, path: &str) -> Option<String> {
        cached_base_world_source(path)
    }
}

/// The production script resolver for this target: the mod-pack overlay first
/// (issue #988), then the config-cache's JS-delivered content.
#[cfg(target_arch = "wasm32")]
pub fn production_script_resolver() -> OverlayScriptResolver<ConfigCacheScriptFallback> {
    OverlayScriptResolver::new(ConfigCacheScriptFallback)
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

#[cfg(target_arch = "wasm32")]
fn settle_preload_complete() -> bool {
    let complete = PENDING_QUEUE.with(|q| q.borrow().is_empty())
        && IN_FLIGHT.with(|q| q.borrow().is_empty())
        && SIDECAR_PRELOAD_PENDING.with(|q| q.borrow().is_empty())
        && !WORLD_PRELOAD_PENDING.with(|pending| *pending.borrow())
        && PRELOAD_FAILURE.with(|failure| failure.borrow().is_none());
    PRELOAD_COMPLETE.with(|flag| {
        *flag.borrow_mut() = complete;
    });
    complete
}

#[cfg(target_arch = "wasm32")]
pub fn preload_error() -> String {
    PRELOAD_FAILURE.with(|failure| failure.borrow().clone().unwrap_or_default())
}

/// The first failure is retained so no later completion can accept a partial
/// content ledger. Overlay-only templates legitimately have no HTTP body.
#[cfg(target_arch = "wasm32")]
pub fn fail_preload_fetch(path: String, message: String) {
    if mod_pack_overlay_get(&path).is_none() {
        PRELOAD_FAILURE.with(|failure| {
            failure
                .borrow_mut()
                .get_or_insert(format!("{path}: {message}"));
        });
        settle_preload_complete();
    }
}

#[cfg(target_arch = "wasm32")]
fn advance_world_preload() {
    let Some((path, text, curated)) = WORLD_PRELOAD_ROOT.with(|root| root.borrow().clone()) else {
        return;
    };
    if crate::content_ledger::is_frozen() || !preload_error().is_empty() {
        return;
    }
    let result = super::world_preload::discover(
        &path,
        &text,
        &curated,
        &|path| match world_fetch_state(path) {
            WorldFetchState::Ready(source) => Ok(Some(source)),
            WorldFetchState::Failed(message) => Err(message),
            _ => Ok(None),
        },
        &production_script_resolver(),
    );
    match result {
        Ok(dependencies) => {
            WORLD_PRELOAD_PENDING
                .with(|pending| *pending.borrow_mut() = !dependencies.pending.is_empty());
            for template in dependencies.templates {
                mark_entity_template(&template);
                // A sibling may promote an already-delivered include fragment
                // to an entity root. No HTTP completion remains to drain it.
                if let Some(source) = raw_template_text(&template) {
                    let _ = wasm_load_config(template, source);
                } else {
                    queue_and_fire(template);
                }
            }
            for source in dependencies.pending {
                request_world_fetch(source);
            }
        }
        Err(error) => {
            PRELOAD_FAILURE.with(|failure| {
                failure.borrow_mut().get_or_insert(error);
            });
        }
    }
    settle_preload_complete();
}

#[cfg(target_arch = "wasm32")]
fn queue_primary_sidecar_preload(path: String) {
    let path = crate::entities::include_resolve::canonical_template_path(&path);
    if is_pending_sidecar_delivered(&path) {
        return;
    }

    // A pack-supplied rig is already resident authoritative content; never
    // replace it with the base HTTP body merely because the template preload
    // reached it before the runtime sidecar loader did.
    if let Some(body) = mod_pack_overlay_get(&path) {
        let _ = wasm_push_sidecar_toml(path, body);
        return;
    }

    let inserted =
        SIDECAR_PRELOAD_PENDING.with(|pending| pending.borrow_mut().insert(path.clone()));
    if !inserted {
        return;
    }
    PRELOAD_COMPLETE.with(|flag| *flag.borrow_mut() = false);
    SIDECAR_FETCH_REQUESTED.with(|requested| {
        requested.borrow_mut().insert(path.clone());
    });
    CONFIG_REQUEST_CB.with(|slot| {
        if let Some(cb) = slot.borrow().as_ref() {
            let _ = cb.call1(&JsValue::NULL, &JsValue::from_str(&path));
        }
    });
}

/// Load an entity template from a TOML string, resolve its include closure and
/// insert the resolved config into the cache.
///
/// The delivered text is recorded raw first, because it may be an include
/// *fragment* rather than an entity template — a fragment is never parsed on
/// its own and never enters the config cache. Whatever the delivery, every
/// entity template whose closure is now complete is resolved and cached, and
/// any fragment still missing is queued for fetch through the same
/// `PENDING_QUEUE`/`IN_FLIGHT` pair as the entity templates.
///
/// Returns Ok(true) when the last pending config is loaded (preload complete).
/// Returns Ok(false) while there are still pending configs.
/// Returns Err(JsValue) on parse/composition failure (without crashing).
#[cfg(target_arch = "wasm32")]
pub fn wasm_load_config(path: String, toml_str: String) -> Result<JsValue, JsValue> {
    // Mod-pack overlay wins for any overridden authored path (issue #760, AC2):
    // when the session has an uploaded pack file at this exact path, use the
    // pack's content instead of the TOML JS fetched over HTTP. Fragments go
    // through here too, so a pack may override a fragment (issue #869 US7).
    let toml_str = mod_pack_overlay_get(&path).unwrap_or(toml_str);
    record_raw_template(&path, toml_str);

    IN_FLIGHT.with(|in_flight| {
        in_flight.borrow_mut().remove(&path);
    });
    // Also remove from pending queue in case it wasn't drained before the
    // fetch completed.
    PENDING_QUEUE.with(|q| {
        q.borrow_mut().retain(|p| p != &path);
    });

    let mut failures: Vec<String> = Vec::new();
    let mut to_fetch: Vec<String> = Vec::new();
    // Loop because caching one config can reveal NESTED entity templates
    // (asteroid variants) whose own text may already have been delivered.
    loop {
        let progress = drain_resolved_templates();
        for e in &progress.errors {
            failures.push(e.to_string());
        }
        for p in progress.fetch {
            if !to_fetch.contains(&p) {
                to_fetch.push(p);
            }
        }
        if progress.ready.is_empty() {
            break;
        }
        for (requested, resolved) in progress.ready {
            // Issue #935: record the byte-stable composed document under its
            // canonical path — the same shape `entity_loader::
            // FsTemplateLoader::load_template` records on native — so an
            // edit to this template OR any fragment it includes moves the
            // content digest identically on both targets.
            crate::content_ledger::record(&resolved.path, &resolved.toml);
            match resolved.parse() {
                Ok(config) => {
                    let nested = nested_template_paths(&config);
                    let primary_sidecar = config
                        .mesh
                        .as_ref()
                        .and_then(crate::entities::model_markers::primary_sidecar_path);
                    CONFIG_CACHE.with(|cache| {
                        cache.borrow_mut().insert(requested, config);
                    });
                    CONFIG_REVISION.with(|revision| revision.set(revision.get().wrapping_add(1)));
                    if let Some(path) = primary_sidecar {
                        queue_primary_sidecar_preload(path);
                    }
                    for nested_path in nested {
                        mark_entity_template(&nested_path);
                        queue_and_fire(nested_path);
                    }
                }
                Err(e) => failures.push(e.to_string()),
            }
        }
    }
    for fragment in to_fetch {
        queue_and_fire(fragment);
    }

    for message in &failures {
        PRELOAD_FAILURE.with(|failure| {
            failure
                .borrow_mut()
                .get_or_insert(format!("{path}: {message}"));
        });
        web_sys::console::error_1(&JsValue::from_str(&format!(
            "Entity template failed to load: {message}"
        )));
    }

    // Failed dependencies are settled but retain a terminal diagnostic. The
    // page displays it instead of starting with a partial frozen content set.
    if settle_preload_complete() {
        // Return TRUE so handleConfigRequest calls finishInit().
        Ok(JsValue::TRUE)
    } else if failures.is_empty() {
        Ok(JsValue::FALSE)
    } else {
        Err(JsValue::from_str(&format!(
            "Entity template error at {}: {}",
            path,
            failures.join("; ")
        )))
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
pub fn wasm_load_world(
    path: String,
    toml_str: String,
    curated_ships: Vec<String>,
) -> Result<JsValue, JsValue> {
    // The picker is over: the real preload owns the config cache from here, so
    // the pre-load catalogue store stops answering (see
    // `clear_catalog_templates`). Done before the parse so a world that fails
    // to parse still leaves the two caches unambiguous.
    clear_catalog_templates();
    WORLD_PRELOAD_ROOT.with(|root| {
        *root.borrow_mut() = Some((path.clone(), toml_str.clone(), curated_ships.clone()))
    });
    WORLD_PRELOAD_PENDING.with(|pending| *pending.borrow_mut() = true);
    PRELOAD_FAILURE.with(|failure| *failure.borrow_mut() = None);
    PRELOAD_COMPLETE.with(|complete| *complete.borrow_mut() = false);
    let mut world_config = crate::world::config::parse_world(&toml_str).map_err(|e| {
        web_sys::console::error_1(&JsValue::from_str(&format!(
            "Failed to parse world TOML at {}: {}",
            path, e
        )));
        JsValue::from_str(&format!("World parse error at {}: {}", path, e))
    })?;
    if !world_config.ship_slots.is_empty() {
        world_config.ship_slots =
            crate::ship_slots::curate_ship_slots(&world_config.ship_slots, &curated_ships)
                .map_err(|error| {
                    JsValue::from_str(&format!("Ship-slot curation error: {error}"))
                })?;
    }

    // Queue every entity template path discovered by the unified pipeline,
    // restricted to the locked scenario's curated playable-hull allowlist
    // when the host resolved one (issue #917) — empty means unrestricted,
    // matching `world::manifest::ScenarioEntry::ships`'s own semantics.
    let entity_paths = crate::world::config::entity_template_paths(&world_config, &curated_ships);

    WORLD_CONFIG.with(|slot| {
        *slot.borrow_mut() = Some(world_config);
    });

    for p in entity_paths {
        // These are ENTITY templates, not fragments: they become config-cache
        // entries once their include closure resolves (issue #869).
        mark_entity_template(&p);
        queue_and_fire(p);
    }
    advance_world_preload();
    Ok(JsValue::from_bool(settle_preload_complete()))
}

/// Get a clone of the loaded unified `WorldConfig`, if any.
#[cfg(target_arch = "wasm32")]
pub fn get_world_config() -> Option<crate::world::config::WorldConfig> {
    WORLD_CONFIG.with(|slot| slot.borrow().clone())
}

/// Register the JS callback used to request runtime world/script content.
///
/// server.html should call this once at startup with a function that accepts
/// a path string, fetches the TOML or Rhai source, and delivers it via
/// `wasm_push_world_toml`.
#[cfg(target_arch = "wasm32")]
pub fn set_world_fetch_callback(callback: Function) {
    WORLD_FETCH_CB.with(|slot| {
        *slot.borrow_mut() = Some(callback);
    });
}

/// Cache successful runtime-fetched world TOML or sibling Rhai source.
///
/// Called by JS after it has fetched a path that Rust requested via the shared
/// world/script callback. The source remains readable for the browser session.
#[cfg(target_arch = "wasm32")]
pub fn wasm_push_world_toml(path: String, toml_str: String) {
    // The first terminal HTTP completion is authoritative. A late duplicate
    // callback cannot turn a reported failure into success (or overwrite an
    // earlier successful body with different bytes).
    if WORLD_FETCH_FAILURES.with(|m| m.borrow().contains_key(&path)) {
        return;
    }
    FETCHED_WORLD_SOURCE.with(|m| {
        m.borrow_mut().entry(path).or_insert(toml_str);
    });
    advance_world_preload();
}

/// Record a terminal runtime-content fetch failure without overloading the
/// legitimate empty-string source value.
#[cfg(target_arch = "wasm32")]
pub fn wasm_fail_world_fetch(path: String, message: String) {
    // A delivered body (including the valid empty string) is already a
    // terminal success. Ignore any late failure callback for the same path.
    if FETCHED_WORLD_SOURCE.with(|m| m.borrow().contains_key(&path)) {
        return;
    }
    WORLD_FETCH_FAILURES.with(|m| {
        m.borrow_mut().entry(path).or_insert(message);
    });
    advance_world_preload();
}

/// Authoritative state of one runtime world/script fetch.
#[cfg(target_arch = "wasm32")]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorldFetchState {
    NotRequested,
    Pending,
    Ready(String),
    Failed(String),
}

#[cfg(target_arch = "wasm32")]
pub fn world_fetch_state(path: &str) -> WorldFetchState {
    if let Some(source) = mod_pack_overlay_get(path) {
        return WorldFetchState::Ready(source);
    }
    if let Some(source) = cached_base_world_source(path) {
        return WorldFetchState::Ready(source);
    }
    if let Some(message) = WORLD_FETCH_FAILURES.with(|m| m.borrow().get(path).cloned()) {
        return WorldFetchState::Failed(message);
    }
    if WORLD_FETCH_REQUESTED.with(|s| s.borrow().contains(path)) {
        WorldFetchState::Pending
    } else {
        WorldFetchState::NotRequested
    }
}

/// Read successful HTTP-fetched world/script content without consuming it.
///
/// This is the base-content seam used while validating a candidate mod pack:
/// it intentionally does not consult the active overlay stack. An empty source
/// is represented by `Some("")`, distinct from not-yet-delivered content.
#[cfg(target_arch = "wasm32")]
pub fn cached_base_world_source(path: &str) -> Option<String> {
    FETCHED_WORLD_SOURCE.with(|m| m.borrow().get(path).cloned())
}

/// Resolve the current authoritative world/script content without consuming it.
///
/// The live mod-pack overlay wins at read time. Removing or reordering a pack
/// therefore immediately reveals the newly authoritative overlay or the
/// durable HTTP-fetched base body; overlay bytes never become stale cache.
#[cfg(target_arch = "wasm32")]
pub fn resolved_world_source(path: &str) -> Option<String> {
    mod_pack_overlay_get(path).or_else(|| cached_base_world_source(path))
}

/// Store the base scenario manifest TOML (`assets/scenarios.toml`), pushed by
/// JS during preload (issue #754).
#[cfg(target_arch = "wasm32")]
pub fn set_scenario_manifest_toml(toml_str: String) {
    SCENARIO_MANIFEST_TOML.with(|slot| {
        *slot.borrow_mut() = Some(toml_str);
    });
}

/// Read the stored base scenario manifest TOML, if JS has pushed it.
#[cfg(target_arch = "wasm32")]
pub fn get_scenario_manifest_toml() -> Option<String> {
    SCENARIO_MANIFEST_TOML.with(|slot| slot.borrow().clone())
}

/// Fire the JS world-fetch callback for `path` if not already requested.
#[cfg(target_arch = "wasm32")]
pub fn request_world_fetch(path: String) {
    // Resolve overlay content without minting a base HTTP request. If that pack
    // is later removed, the now-unresolved path may request its base body once.
    if mod_pack_overlay_get(&path).is_some() {
        return;
    }

    // Ready, Failed, and an outstanding Pending request are all terminal for
    // request issuance. Only the absent state may transition to Pending.
    if !matches!(world_fetch_state(&path), WorldFetchState::NotRequested) {
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
// Primary rigs are discovered during entity-config preload and use its config
// callback so their exact bodies arrive before content identity freezes. Later
// runtime/generated reads reuse the JS callback registered through
// `set_world_fetch_callback`. The persistent inbox and requested sets are shared
// by both delivery routes, so each canonical path is fetched once.

/// Push a preloaded or runtime-fetched sidecar TOML into the persistent inbox.
///
/// Called by JS after it has fetched a rig sidecar at a path that Rust
/// requested via `request_sidecar_fetch`. An empty string signals "absent"
/// (404) so the caller can proceed with an identity rig and stop re-requesting.
///
/// Available on native too so unit tests can simulate JS delivery without
/// dragging in wasm-bindgen.
pub fn wasm_push_sidecar_toml(path: String, toml_str: String) -> bool {
    let path = crate::entities::include_resolve::canonical_template_path(&path);
    // Delivery is the browser's pre-freeze content-ingestion boundary. Record
    // the exact body before parsing: an empty 404 and malformed non-empty TOML
    // are both authoritative inputs even though both produce an identity rig.
    crate::content_ledger::record(&path, &toml_str);
    PENDING_SIDECAR_TOML.with(|m| {
        m.borrow_mut().insert(path.clone(), toml_str);
    });
    #[cfg(target_arch = "wasm32")]
    {
        SIDECAR_PRELOAD_PENDING.with(|pending| {
            pending.borrow_mut().remove(&path);
        });
        settle_preload_complete()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        false
    }
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
    let path = crate::entities::include_resolve::canonical_template_path(path);
    PENDING_SIDECAR_TOML.with(|m| m.borrow().get(&path).cloned())
}

/// Non-destructive check: has JS delivered the sidecar TOML for `path` yet?
///
/// Use this when you only need to know that the fetch has completed (e.g. the
/// preload poller marking a sidecar as ready) and you do not need the TOML
/// contents. Leaves the entry in `PENDING_SIDECAR_TOML` so the renderer can
/// still consume it via [`take_pending_sidecar_toml`].
pub fn is_pending_sidecar_delivered(path: &str) -> bool {
    let path = crate::entities::include_resolve::canonical_template_path(path);
    PENDING_SIDECAR_TOML.with(|m| m.borrow().contains_key(&path))
}

/// Fire the JS fetch callback for a sidecar `path` if not already requested.
///
/// `optional` is passed to the page as the callback's second argument: it says
/// this read is an EXISTENCE TEST, so a 404 is one of its expected answers and
/// the page must resolve it to the empty string without logging a failed fetch.
/// Only the legacy ladder-convention probe sets it (see
/// [`crate::entities::model_markers::Absence`]); every other sidecar read still
/// wants a missing file reported.
#[cfg(target_arch = "wasm32")]
pub fn request_sidecar_fetch(path: String, optional: bool) {
    let path = crate::entities::include_resolve::canonical_template_path(&path);
    if is_pending_sidecar_delivered(&path) {
        return;
    }
    if let Some(body) = mod_pack_overlay_get(&path) {
        let _ = wasm_push_sidecar_toml(path, body);
        return;
    }
    let already = SIDECAR_FETCH_REQUESTED.with(|s| s.borrow().contains(&path));
    if already {
        return;
    }
    SIDECAR_FETCH_REQUESTED.with(|s| s.borrow_mut().insert(path.clone()));
    WORLD_FETCH_CB.with(|slot| {
        if let Some(cb) = slot.borrow().as_ref() {
            let _ = cb.call2(
                &JsValue::NULL,
                &JsValue::from_str(&path),
                &JsValue::from_bool(optional),
            );
        }
    });
}

#[cfg(target_arch = "wasm32")]
pub fn wasm_load_faction(_path: String, toml_str: String) -> Result<JsValue, JsValue> {
    match crate::ai::faction::parse_faction_config(&toml_str) {
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

/// The effective FactionRegistry (issue #1474).
///
/// The base set is the thread-local one when something filled it — a
/// disposable Test's captured factions through [`replace_faction_registry`],
/// or `wasm_load_faction` — and otherwise the four compiled-in factions, so a
/// host page that never loaded a faction still has Alliance, Pirate, Harrow
/// and Requiem. Every active pack's `assets/factions/*.toml` is then inserted
/// in stack order, later winning by uuid, so a pack's factions reach the same
/// registry its hulls reference.
#[cfg(target_arch = "wasm32")]
pub fn get_faction_registry() -> crate::ai::faction::FactionRegistry {
    let mut registry = FACTION_REGISTRY.with(|reg| reg.borrow().clone());
    if registry.is_empty() {
        insert_built_in_factions(&mut registry);
    }
    overlay_pack_factions(&mut registry, &active_packs());
    registry
}

/// Replace the thread-local faction set entirely. A Test iframe captures the
/// draft's faction files before its App boots; a faction the draft deleted is
/// therefore gone in the Test rather than resurrected from the compiled-in
/// four.
#[cfg(target_arch = "wasm32")]
pub fn replace_faction_registry(
    configs: impl IntoIterator<Item = crate::ai::faction::FactionConfig>,
) {
    FACTION_REGISTRY.with(|reg| {
        let mut reg = reg.borrow_mut();
        *reg = crate::ai::faction::FactionRegistry::new();
        for config in configs {
            reg.insert(config);
        }
    });
}

/// Get a reference to the config cache.
#[cfg(target_arch = "wasm32")]
pub fn get_config_cache() -> ConfigCache {
    ConfigCache(CONFIG_CACHE.with(|cache| cache.borrow().clone()))
}

/// Look up a single cached entity config by template path.
#[cfg(target_arch = "wasm32")]
pub fn get_cached_entity_config(path: &str) -> Option<crate::entities::config::EntityConfig> {
    CONFIG_CACHE.with(|cache| cache.borrow().get(path).cloned())
}

// ── Pre-load catalogue enrichment ───────────────────────────────────────────
//
// `wasm_get_scenario_catalog` is read BEFORE any world is activated, so the
// picker's hull cards are built while `CONFIG_CACHE` is still empty:
// `ship_payload` finds no template and publishes `template_path` + `label`
// alone, which is why every card badged `[UNKNOWN]` and carried no registry,
// mass or power rating.
//
// This is a SEPARATE store, deliberately, rather than an early write into the
// preload machinery. Delivering a hull's text through `wasm_load_config` before
// `set_config_request_callback` has been wired would record it in
// `RAW_TEMPLATE_TOML`, and `queue_and_fire`'s `is_raw_template_delivered` guard
// would then skip it forever — so the real preload would never record the
// composed document in the content ledger and never queue the hull's primary
// rig sidecar. Writing straight into `CONFIG_CACHE` loses the same two things
// for the same reason (`queue_and_fire`'s other guard). Neither loss is visible
// in the picker and both are fatal later, so the enrichment gets its own store
// and touches nothing the preload owns: no `PENDING_QUEUE`, no `IN_FLIGHT`, no
// `PRELOAD_COMPLETE`, no config-request callback.
//
// It is also reached through a seam of its own, [`catalog_entity_config`],
// rather than by widening [`get_cached_entity_config`]: that lookup is the
// SPAWN path (`entities::loader::WasmTemplateLoader`, `server::reference_grid`),
// and a catalogue-cached hull standing in for one the locked scenario's `ships`
// curation (issue #917) deliberately left out of the preload would spawn
// content the curation excluded. With the store off the spawn lookup entirely,
// no "is a world loaded yet?" guard is needed to hold that line — which is what
// lets the enrichment keep working for the SECOND lobby round, where issue #756
// reuses the already-loaded world and `WORLD_CONFIG` never goes back to `None`.
//
// Ungated (native + wasm) for the same reason `RAW_TEMPLATE_TOML` and the
// mod-pack overlay are: the browser is the only host that drives it, but the
// decisions here — what an empty delivery means, when a root resolves, when it
// is left label-only — are the loader contract and must be assertable under
// `cargo test`. Only the `#[wasm_bindgen]` export in `server::bridge` is gated.

thread_local! {
    /// Raw TOML for templates and include fragments delivered ahead of the
    /// preload, keyed by CANONICAL path (what the resolver asks for).
    static CATALOG_TEMPLATE_RAW: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());

    /// Paths delivered as catalogue ROOTS, as `(requested, canonical)`. The
    /// requested half is the key the world authored, which is what
    /// `ship_payload` looks the enrichment up under.
    static CATALOG_TEMPLATE_ROOTS: RefCell<Vec<(String, String)>> = const { RefCell::new(Vec::new()) };

    /// Roots whose include closure resolved and parsed, keyed by requested path.
    static CATALOG_TEMPLATE_CACHE: RefCell<HashMap<String, crate::entities::config::EntityConfig>> =
        RefCell::new(HashMap::new());
}

/// Look up an entity config for CATALOGUE DISPLAY: the preload's cache first,
/// then the pre-load catalogue store.
///
/// The single consumer is `delivery::payload::ship_payload`, whose `class`,
/// `hull_id`, `mass`, `power_rating` and `name` are enrichment read out of a
/// cached template — and the catalogue is published before any world is
/// activated, so on the browser there is no preload to have populated
/// `CONFIG_CACHE` at all.
///
/// Deliberately NOT folded into [`get_cached_entity_config`]: that is the spawn
/// lookup, and it must keep missing for a hull the locked scenario's `ships`
/// curation (issue #917) left out of the preload.
pub fn catalog_entity_config(path: &str) -> Option<crate::entities::config::EntityConfig> {
    get_cached_entity_config(path)
        .or_else(|| CATALOG_TEMPLATE_CACHE.with(|cache| cache.borrow().get(path).cloned()))
}

/// Fragment source over [`CATALOG_TEMPLATE_RAW`].
///
/// `absence_is_final` is FALSE: this store fills incrementally, one JS fetch at
/// a time, exactly like `HostFragmentSource`. A fragment that has not arrived
/// yet is something to fetch, never a fault.
struct CatalogFragmentSource;

impl crate::entities::include_resolve::FragmentSource for CatalogFragmentSource {
    fn read(&self, path: &str) -> Option<String> {
        if let Some(text) = mod_pack_overlay_get(path) {
            return Some(text);
        }
        CATALOG_TEMPLATE_RAW.with(|m| m.borrow().get(path).cloned())
    }

    fn absence_is_final(&self) -> bool {
        false
    }
}

/// Deliver one template's raw TOML for catalogue enrichment and report which
/// include fragments are still missing.
///
/// `root` marks a hull the catalogue actually names; everything else delivered
/// here is a fragment answering a previous call's return value. The returned
/// paths are canonical and deduplicated — an empty return means every root
/// delivered so far has either resolved or failed, and there is nothing left to
/// fetch.
///
/// # An EMPTY `toml_str` is "no text came back", not "the file is empty"
///
/// The host calls this even when its `fetch` failed, exactly as
/// `handleConfigRequest` calls `wasm_load_config(path, '')` on a 404 — because a
/// mod pack's own hull lives in the session overlay and has NO URL to fetch, so
/// its 404 is the normal case and the overlay is the only place its text has
/// ever been. The overlay is consulted first here, so that delivery still
/// composes. With no overlay copy either, the call is a NO-OP: nothing is
/// recorded, so a missing fragment stays "still to fetch" rather than composing
/// as present-and-empty, and a missing root is simply never resolved.
///
/// Nothing here is fatal. A hull that never arrives, will not compose or will
/// not parse is simply not cached, and the card degrades to `template_path` +
/// `label` — exactly what every card showed before this existed.
pub fn push_catalog_template(path: String, toml_str: String, root: bool) -> Vec<String> {
    // The active mod-pack stack wins any authored path, as it does on every
    // other content channel (issue #760 AC2) — and is the ONLY source for a
    // pack's own hull, whose fetch always fails.
    let text = match mod_pack_overlay_get(&path) {
        Some(overlaid) => overlaid,
        None if toml_str.is_empty() => return resolve_catalog_templates(),
        None => toml_str,
    };
    let canonical = crate::entities::include_resolve::canonical_template_path(&path);
    CATALOG_TEMPLATE_RAW.with(|m| {
        m.borrow_mut().insert(canonical.clone(), text);
    });
    if root {
        CATALOG_TEMPLATE_ROOTS.with(|v| {
            let mut v = v.borrow_mut();
            if !v.iter().any(|(_, c)| c == &canonical) {
                v.push((path, canonical));
            }
        });
    }
    resolve_catalog_templates()
}

/// Resolve every catalogue root whose include closure is now complete, and
/// report the fragments the rest are still waiting on.
fn resolve_catalog_templates() -> Vec<String> {
    use crate::entities::include_resolve::{preload_step, PreloadStep};

    let roots = CATALOG_TEMPLATE_ROOTS.with(|v| v.borrow().clone());
    let mut awaiting: Vec<String> = Vec::new();
    for (requested, canonical) in roots {
        if CATALOG_TEMPLATE_CACHE.with(|m| m.borrow().contains_key(&requested)) {
            continue;
        }
        if !CATALOG_TEMPLATE_RAW.with(|m| m.borrow().contains_key(&canonical)) {
            continue;
        }
        match preload_step(&canonical, &CatalogFragmentSource) {
            Ok(PreloadStep::Ready(resolved)) => {
                if let Ok(config) = resolved.parse() {
                    CATALOG_TEMPLATE_CACHE.with(|m| {
                        m.borrow_mut().insert(requested, config);
                    });
                }
            }
            Ok(PreloadStep::AwaitingIncludes(paths)) => {
                for p in paths {
                    if !awaiting.contains(&p) {
                        awaiting.push(p);
                    }
                }
            }
            // A cycle or a malformed `includes` list never becomes resolvable
            // by fetching more. Leave the card label-only.
            Err(_) => {}
        }
    }
    awaiting
}

/// Discard the pre-load catalogue store.
///
/// Called from [`wasm_load_world`] as hygiene: once the real preload owns the
/// config cache, a stale pre-load answer for a hull is at best redundant. The
/// line that keeps a catalogue-cached hull out of a SPAWN is structural rather
/// than temporal — the store hangs off [`catalog_entity_config`] and no spawn
/// path reads it — so a round that never re-enters `wasm_load_world` (issue
/// #756 reuses the loaded world) is safe without this, and enriches again.
///
/// Also the test seam, the way [`clear_template_preload_state`] is for the
/// preload's own stores.
pub fn clear_catalog_templates() {
    CATALOG_TEMPLATE_RAW.with(|m| m.borrow_mut().clear());
    CATALOG_TEMPLATE_ROOTS.with(|v| v.borrow_mut().clear());
    CATALOG_TEMPLATE_CACHE.with(|m| m.borrow_mut().clear());
}

/// Queue a path for fetching and fire the callback.
///
/// Two membership guards, not one: the config cache stops an already-resolved
/// ENTITY template being refetched, and the raw-text store stops an include
/// FRAGMENT being refetched (issue #869). Fragments never reach the config
/// cache, so without the second guard a fragment shared by five hulls would be
/// fetched once per hull — and, worse, would be re-queued every time, so
/// `PENDING_QUEUE` would never drain and preload would never complete.
#[cfg(target_arch = "wasm32")]
fn queue_and_fire(path: String) {
    let mut should_fire = false;

    if is_raw_template_delivered(&path) {
        return;
    }
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
pub type ConfigCache = std::collections::HashMap<String, crate::entities::config::EntityConfig>;

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
pub fn wasm_fail_world_fetch(_path: String, _message: String) {}

#[cfg(not(target_arch = "wasm32"))]
pub fn wasm_load_config(_path: String, _toml_str: String) -> Result<JsValue, JsValue> {
    Ok(JsValue::from_bool(false))
}

#[cfg(not(target_arch = "wasm32"))]
pub fn wasm_is_preload_complete() -> bool {
    false
}

/// Native template cache.
///
/// The WASM side fills `CONFIG_CACHE` from a JS-driven preload. Native has no
/// preload, and most callers of [`get_config_cache`] have no filesystem
/// fallback of their own — `asteroids::lifecycle`, `server_app`'s spawn
/// helpers and `world::server::setup_world` all just see whatever the cache
/// holds. Leaving it permanently empty meant those paths silently did nothing
/// off-browser.
///
/// So native gets a real cache too, populated up front by whoever is driving
/// the app (the headless runner walks `assets/entities/`). A `RwLock` rather
/// than the WASM side's `thread_local!` because Bevy systems run on worker
/// threads. Unit tests that never populate it keep the previous behaviour: an
/// empty cache, and the on-demand disk fallback in
/// [`crate::entities::loader::WasmTemplateLoader`] does the work.
///
/// **This is process-global, so populating it in one test changes every other
/// test in the same binary.** A populated cache makes
/// `update_session_with_config` build a real `ShipClientConfigResource`, which
/// changes values (radar range among them) that unrelated unit tests assert on.
/// Anything that calls [`insert_native_config`] therefore belongs in an
/// integration test with its own process — see `tests/headless_runner.rs`.
#[cfg(not(target_arch = "wasm32"))]
static NATIVE_CONFIG_CACHE: std::sync::RwLock<
    Option<std::collections::HashMap<String, crate::entities::config::EntityConfig>>,
> = std::sync::RwLock::new(None);

#[cfg(not(target_arch = "wasm32"))]
static CONFIG_REVISION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Presentation-only invalidation token. Includes template replacement and the
/// mod overlay revision; never participates in simulation ordering or digests.
pub fn config_cache_revision() -> (u64, u64) {
    #[cfg(target_arch = "wasm32")]
    let templates = CONFIG_REVISION.with(std::cell::Cell::get);
    #[cfg(not(target_arch = "wasm32"))]
    let templates = CONFIG_REVISION.load(std::sync::atomic::Ordering::Acquire);
    (templates, mod_pack_revision())
}

/// Insert a parsed template into the native cache under `path`.
///
/// Keyed by the same repo-relative path the world TOML uses
/// (`assets/entities/foo.toml`), so lookups match the WASM cache exactly.
#[cfg(not(target_arch = "wasm32"))]
pub fn insert_native_config(path: String, config: crate::entities::config::EntityConfig) {
    let mut guard = NATIVE_CONFIG_CACHE
        .write()
        .expect("native config cache poisoned");
    guard
        .get_or_insert_with(Default::default)
        .insert(path, config);
    CONFIG_REVISION.fetch_add(1, std::sync::atomic::Ordering::Release);
}

#[cfg(not(target_arch = "wasm32"))]
pub fn get_config_cache() -> ConfigCache {
    NATIVE_CONFIG_CACHE
        .read()
        .expect("native config cache poisoned")
        .clone()
        .unwrap_or_default()
}

/// Native twin of the WASM cache lookup. Misses until something has called
/// [`insert_native_config`], at which point callers with a filesystem fallback
/// stop hitting the disk on every spawn.
#[cfg(not(target_arch = "wasm32"))]
pub fn get_cached_entity_config(path: &str) -> Option<crate::entities::config::EntityConfig> {
    NATIVE_CONFIG_CACHE
        .read()
        .expect("native config cache poisoned")
        .as_ref()
        .and_then(|m| m.get(path).cloned())
}

#[cfg(not(target_arch = "wasm32"))]
pub fn wasm_load_world(
    _path: String,
    _toml_str: String,
    _curated_ships: Vec<String>,
) -> Result<JsValue, JsValue> {
    Ok(JsValue::from_bool(true))
}

#[cfg(not(target_arch = "wasm32"))]
pub fn get_world_config() -> Option<crate::world::config::WorldConfig> {
    None
}

// Native no-ops for the runtime world-fetch helpers (native uses std::fs directly).
#[cfg(not(target_arch = "wasm32"))]
pub fn cached_base_world_source(_path: &str) -> Option<String> {
    None
}
#[cfg(not(target_arch = "wasm32"))]
pub fn resolved_world_source(_path: &str) -> Option<String> {
    None
}
#[cfg(not(target_arch = "wasm32"))]
pub fn request_world_fetch(_path: String) {}

// Native no-op for the JS fetch trigger (native reads sidecars via std::fs
// directly in `load_sidecar_toml`). The pure-Rust take/is/push functions
// above are shared by both targets.
#[cfg(not(target_arch = "wasm32"))]
pub fn request_sidecar_fetch(_path: String, _optional: bool) {}

#[cfg(not(target_arch = "wasm32"))]
pub fn wasm_load_faction(_path: String, _toml_str: String) -> Result<JsValue, JsValue> {
    Ok(JsValue::from_bool(false))
}

/// The effective FactionRegistry (issue #1474): the faction files under the
/// content root's `assets/factions/`, then every active pack's, later winning
/// by uuid. The content root is the cwd — a native host pins it and a
/// disposable Test child pins its stage — so the Test runs the exact unsaved
/// faction set without installing anything. A missing or unreadable directory
/// falls back to the four compiled-in factions.
#[cfg(not(target_arch = "wasm32"))]
pub fn get_faction_registry() -> crate::ai::faction::FactionRegistry {
    faction_registry_from(std::path::Path::new("assets/factions"), &active_packs())
}

/// The faction directory that sits beside a template directory: a content
/// tree keeps `assets/entities` and `assets/factions` as siblings, so a run
/// told where its hulls are (`phoenix-headless --ship`) reads the factions of
/// that same tree rather than of whatever directory it was launched from.
#[cfg(not(target_arch = "wasm32"))]
pub fn faction_directory_beside(template_dir: &std::path::Path) -> std::path::PathBuf {
    template_dir
        .parent()
        .map(|assets| assets.join("factions"))
        .unwrap_or_else(|| std::path::PathBuf::from("assets/factions"))
}

/// The pure half of [`get_faction_registry`], so a test can point it at a
/// directory without moving the process's cwd under other tests.
#[cfg(not(target_arch = "wasm32"))]
pub fn faction_registry_from(
    directory: &std::path::Path,
    packs: &[ActivePack],
) -> crate::ai::faction::FactionRegistry {
    let mut registry = crate::ai::faction::FactionRegistry::new();
    match crate::content_fs::read_dir(directory) {
        Ok(entries) => {
            // Sorted by name so two files declaring one uuid resolve the same
            // way on every filesystem, not in directory-listing order.
            let mut paths: Vec<std::path::PathBuf> = entries
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| {
                    path.is_file()
                        && path
                            .extension()
                            .is_some_and(|extension| extension == "toml")
                })
                .collect();
            paths.sort();
            for path in paths {
                match crate::content_fs::read_to_string(&path) {
                    Ok(source) => {
                        insert_faction_source(&mut registry, &path.display().to_string(), &source)
                    }
                    Err(error) => bevy::log::warn!(
                        target: crate::logging::LogCat::Config.target(),
                        "faction file {} is unreadable and was skipped: {error}",
                        path.display()
                    ),
                }
            }
        }
        Err(_) => insert_built_in_factions(&mut registry),
    }
    overlay_pack_factions(&mut registry, packs);
    registry
}

/// The four shipped factions, compiled in as the last resort on both targets.
fn insert_built_in_factions(registry: &mut crate::ai::faction::FactionRegistry) {
    for toml_str in &[
        include_str!("../../../../assets/factions/alliance.toml"),
        include_str!("../../../../assets/factions/pirate.toml"),
        include_str!("../../../../assets/factions/harrow.toml"),
        include_str!("../../../../assets/factions/requiem.toml"),
    ] {
        if let Ok(config) = crate::ai::faction::parse_faction_config(toml_str) {
            registry.insert(config);
        }
    }
}

fn insert_faction_source(
    registry: &mut crate::ai::faction::FactionRegistry,
    path: &str,
    source: &str,
) {
    match crate::ai::faction::parse_faction_config(source) {
        Ok(config) => registry.insert(config),
        // An unparsable file is skipped rather than fatal, as the compiled-in
        // loop always did; the Workshop refuses it before it can be saved.
        Err(error) => bevy::log::warn!(
            target: crate::logging::LogCat::Config.target(),
            "faction file {path} did not parse and was skipped: {error}"
        ),
    }
}

/// Every active pack's `assets/factions/*.toml`, oldest pack first so the
/// newest wins a shared uuid — the same precedence every other overlay read
/// applies.
fn overlay_pack_factions(registry: &mut crate::ai::faction::FactionRegistry, packs: &[ActivePack]) {
    for pack in packs {
        let mut files: Vec<(&String, &String)> = pack
            .files
            .iter()
            .filter(|(path, _)| path.starts_with("assets/factions/") && path.ends_with(".toml"))
            .collect();
        files.sort();
        for (path, source) in files {
            insert_faction_source(registry, path, source);
        }
    }
}

// ── Unit Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "config_cache_tests.rs"]
mod tests;

use crate::entities::include_resolve::ParseEntityTemplate as _;

pub use phoenix_sim_gameplay::entities::config_cache::*;
