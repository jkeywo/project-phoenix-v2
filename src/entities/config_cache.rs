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

// `RefCell`, `HashMap` and `HashSet` are used by both the WASM thread-locals
// AND the cross-target sidecar inbox / composable-template preload state below,
// so they live outside the cfg gate.
use std::cell::RefCell;
use std::collections::HashMap;
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

    /// The base scenario manifest TOML (`assets/scenarios.toml`), pushed by JS
    /// during preload via `wasm_push_scenario_manifest`. Read by the pre-load
    /// scenario/ship catalog accessor before any world is activated (issue
    /// #754).
    static SCENARIO_MANIFEST_TOML: RefCell<Option<String>> =
        const { RefCell::new(None) };

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

thread_local! {
    /// Raw TOML text for every template path the host has delivered, keyed by
    /// canonical path. Holds BOTH entity templates and include fragments —
    /// a fragment is never parsed on its own, so this is the only place its
    /// text lives.
    static RAW_TEMPLATE_TOML: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());

    /// Paths requested as ENTITY templates (world instances, nested asteroid
    /// variants), as `(requested, canonical)`. Only these become config-cache
    /// entries; a fragment is authoring input and must never be spawnable.
    static ENTITY_TEMPLATE_PATHS: RefCell<Vec<(String, String)>> = const { RefCell::new(Vec::new()) };

    /// Canonical entity paths already emitted by [`drain_resolved_templates`],
    /// successfully or not, so neither result is produced twice.
    static SETTLED_TEMPLATE_PATHS: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
}

/// What the host should do next, after delivering one template's text.
#[derive(Debug, Default)]
pub struct TemplatePreloadProgress {
    /// Fully resolved entity templates, paired with the path they were
    /// requested under (which is the config-cache key world TOML looks up).
    pub ready: Vec<(String, crate::entities::include_resolve::ResolvedTemplate)>,
    /// Canonical fragment paths that must still be fetched. Deduplicated.
    pub fetch: Vec<String>,
    /// Composition failures. Every one of these is a load error: the entity
    /// never enters the config cache, so nothing partially composed spawns.
    pub errors: Vec<crate::entities::include_resolve::IncludeError>,
}

/// Record the raw TOML the host delivered for `path`.
pub fn record_raw_template(path: &str, toml_str: String) {
    let key = crate::entities::include_resolve::canonical_template_path(path);
    RAW_TEMPLATE_TOML.with(|m| {
        m.borrow_mut().insert(key, toml_str);
    });
}

/// The raw TOML the host delivered for `path`, if any.
///
/// The read side of [`record_raw_template`], and the browser's only source of
/// fragment text: `entity_includes::HostFragmentSource` consults it so include
/// resolution — and the composition validation built on it (issue #906) — works
/// on WASM, where there is no filesystem to fall back to.
pub fn raw_template_text(path: &str) -> Option<String> {
    let key = crate::entities::include_resolve::canonical_template_path(path);
    RAW_TEMPLATE_TOML.with(|m| m.borrow().get(&key).cloned())
}

/// Note that `path` was requested as an entity template, not as a fragment.
pub fn mark_entity_template(path: &str) {
    let canonical = crate::entities::include_resolve::canonical_template_path(path);
    ENTITY_TEMPLATE_PATHS.with(|v| {
        let mut v = v.borrow_mut();
        if !v.iter().any(|(_, c)| c == &canonical) {
            v.push((path.to_string(), canonical));
        }
    });
}

/// Has the host already delivered text for `path`?
///
/// The queue guard for fragments: they never enter the config cache, so the
/// cache-membership check that stops an entity template being fetched twice
/// would not stop a fragment shared by five hulls being fetched five times.
pub fn is_raw_template_delivered(path: &str) -> bool {
    let key = crate::entities::include_resolve::canonical_template_path(path);
    RAW_TEMPLATE_TOML.with(|m| m.borrow().contains_key(&key))
}

/// Resolve every entity template whose include closure is now complete, and
/// report what still has to be fetched.
///
/// Each entity template is emitted at most once — as `ready` or as an `error`,
/// never both, never twice. Templates whose text has not arrived yet are simply
/// not considered; absence of a *fragment* is reported as something to fetch,
/// which is the whole point of the split.
pub fn drain_resolved_templates() -> TemplatePreloadProgress {
    let raw = RAW_TEMPLATE_TOML.with(|m| m.borrow().clone());
    let entities = ENTITY_TEMPLATE_PATHS.with(|v| v.borrow().clone());
    let mut progress = TemplatePreloadProgress::default();

    for (requested, canonical) in entities {
        let already = SETTLED_TEMPLATE_PATHS.with(|s| s.borrow().contains(&canonical));
        if already || !raw.contains_key(&canonical) {
            continue;
        }
        match crate::entities::include_resolve::preload_step(&canonical, &raw) {
            Ok(crate::entities::include_resolve::PreloadStep::Ready(resolved)) => {
                SETTLED_TEMPLATE_PATHS.with(|s| s.borrow_mut().insert(canonical));
                progress.ready.push((requested, *resolved));
            }
            Ok(crate::entities::include_resolve::PreloadStep::AwaitingIncludes(paths)) => {
                for p in paths {
                    if !progress.fetch.contains(&p) {
                        progress.fetch.push(p);
                    }
                }
            }
            Err(e) => {
                // Permanent: a cycle, an unparseable fragment or a malformed
                // `includes` list never becomes resolvable by fetching more.
                SETTLED_TEMPLATE_PATHS.with(|s| s.borrow_mut().insert(canonical));
                progress.errors.push(e);
            }
        }
    }
    progress
}

/// Discard the composable-template preload state. Test seam, and the natural
/// companion to [`clear_mod_pack_overlay`] for a same-page next round.
pub fn clear_template_preload_state() {
    RAW_TEMPLATE_TOML.with(|m| m.borrow_mut().clear());
    ENTITY_TEMPLATE_PATHS.with(|v| v.borrow_mut().clear());
    SETTLED_TEMPLATE_PATHS.with(|s| s.borrow_mut().clear());
}

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
// HTTP fetch otherwise (AC2). The overlay only ever ADDS or REPLACES supported
// authored paths — it never touches disk. It is host-session-scoped: a page
// reload clears the thread-local naturally, and [`clear_mod_pack_overlay`]
// discards the WHOLE stack for the same-page return-to-lobby / next-upload seam
// (AC4).
//
// Ungated (native + wasm) so the ordered-stack resolution + conflict computation
// is unit-testable on native without dragging in wasm-bindgen, exactly like the
// sidecar inbox above. The resolution and conflict logic are PURE functions over
// `&[ActivePack]` ([`overlay_lookup`], [`overlay_source_in`],
// [`overlay_conflicts`]); the thread-local wrappers are a thin session-state
// layer over them.

/// One installed pack in the ordered overlay stack (issue #987).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ActivePack {
    /// Stable pack id from the manifest `[pack] id`.
    pub id: String,
    /// Display name from `[pack] name`.
    pub name: String,
    /// Version string from `[pack] version`.
    pub version: String,
    /// Exact authored path -> TOML for every supported file the pack carries.
    pub files: HashMap<String, String>,
    /// The pack's raw `scenarios.toml` manifest.
    pub manifest_toml: String,
}

/// A single authored path carried by more than one active pack (issue #987).
///
/// `winner` is the pack id that wins the path under the precedence policy (the
/// latest-loaded pack carrying it); `losers` are the shadowed pack ids, in load
/// order (earliest first).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PathConflict {
    pub path: String,
    pub winner: String,
    pub losers: Vec<String>,
}

/// Resolve `path` against an ordered pack stack (oldest → newest); later wins.
///
/// Pure: the returned content is a function only of the stack contents and their
/// order, so precedence is deterministic and testable without session state.
pub fn overlay_lookup<'a>(packs: &'a [ActivePack], path: &str) -> Option<&'a str> {
    packs
        .iter()
        .rev()
        .find_map(|p| p.files.get(path).map(String::as_str))
}

/// The id of the pack that wins `path` under the precedence policy, if any
/// (issue #987 provenance). The pure core behind [`overlay_source`].
pub fn overlay_source_in<'a>(packs: &'a [ActivePack], path: &str) -> Option<&'a str> {
    packs
        .iter()
        .rev()
        .find(|p| p.files.contains_key(path))
        .map(|p| p.id.as_str())
}

/// Every authored path carried by two or more packs in the stack, with its
/// winner + shadowed losers (issue #987). Deterministic: paths are reported in
/// sorted order and each conflict's pack ids follow load order.
pub fn overlay_conflicts(packs: &[ActivePack]) -> Vec<PathConflict> {
    use std::collections::BTreeMap;
    let mut by_path: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for pack in packs {
        for path in pack.files.keys() {
            by_path
                .entry(path.as_str())
                .or_default()
                .push(pack.id.as_str());
        }
    }
    let mut conflicts = Vec::new();
    for (path, ids) in by_path {
        if ids.len() < 2 {
            continue;
        }
        // Load order is preserved because the outer loop walks `packs` in order;
        // the last id is the newest carrier (the winner).
        let (winner, losers) = ids.split_last().expect("len >= 2");
        conflicts.push(PathConflict {
            path: path.to_string(),
            winner: winner.to_string(),
            losers: losers.iter().map(|s| s.to_string()).collect(),
        });
    }
    conflicts
}

thread_local! {
    /// The ordered mod-pack overlay stack for the current host session
    /// (oldest → newest). See the precedence policy above.
    static ACTIVE_PACKS: RefCell<Vec<ActivePack>> = const { RefCell::new(Vec::new()) };
}

/// A snapshot of the active pack stack, oldest → newest (issue #987).
pub fn active_packs() -> Vec<ActivePack> {
    ACTIVE_PACKS.with(|s| s.borrow().clone())
}

/// Push a validated pack onto the top (newest end) of the stack (issue #987).
///
/// Called only after atomic validation accepts the pack, so nothing partial is
/// ever installed. Does NOT evict earlier packs — a later pack merely shadows an
/// earlier one for the paths they share (that is the whole point of the stack).
pub fn push_mod_pack(pack: ActivePack) {
    ACTIVE_PACKS.with(|s| s.borrow_mut().push(pack));
}

/// Remove the pack with `id` from the stack (issue #987). Precedence for every
/// path it owned re-resolves on the next lookup — the next pack down that carries
/// the path becomes the winner. Returns whether a pack was removed.
pub fn remove_mod_pack(id: &str) -> bool {
    ACTIVE_PACKS.with(|s| {
        let mut v = s.borrow_mut();
        let before = v.len();
        v.retain(|p| p.id != id);
        v.len() != before
    })
}

/// Reorder the stack so it matches `ids` (oldest → newest). Packs whose id is not
/// named keep their relative order after the named ones; unknown ids are ignored.
/// Precedence re-resolves on the next lookup (issue #987).
pub fn reorder_mod_packs(ids: &[String]) {
    ACTIVE_PACKS.with(|s| {
        let mut v = s.borrow_mut();
        let mut reordered: Vec<ActivePack> = Vec::with_capacity(v.len());
        for id in ids {
            if let Some(pos) = v.iter().position(|p| &p.id == id) {
                reordered.push(v.remove(pos));
            }
        }
        // Anything not named by `ids` keeps its (now-compacted) relative order.
        reordered.append(&mut v);
        *v = reordered;
    });
}

/// Look up an overridden authored path in the mod-pack overlay stack, if any.
///
/// Both content channels consult this before falling back to the normal fetch,
/// so the WINNING pack's file (the latest loaded carrying the path) is used for
/// any exact authored path it carries.
pub fn mod_pack_overlay_get(path: &str) -> Option<String> {
    ACTIVE_PACKS.with(|s| overlay_lookup(&s.borrow(), path).map(str::to_string))
}

/// The id of the pack that currently owns `path` in the overlay stack, if any
/// (issue #987 provenance). Lets any consumer name the owning pack — the host
/// conflict summary, a diagnostic log — without duplicating the walk.
///
/// Returns an owned `String` rather than the `&str` the pure [`overlay_source_in`]
/// yields, because the stack lives behind a thread-local `RefCell` that cannot
/// hand out a borrow past the `with` closure.
pub fn overlay_source(path: &str) -> Option<String> {
    ACTIVE_PACKS.with(|s| overlay_source_in(&s.borrow(), path).map(str::to_string))
}

/// Discard the WHOLE mod-pack overlay stack for the current session (issue #760
/// AC4, #987). Called before return-to-lobby, so uploaded state never leaks into
/// a fresh selection stage or a same-page next round. A page reload clears the
/// thread-local anyway; this covers the same-page seams.
pub fn clear_mod_pack_overlay() {
    ACTIVE_PACKS.with(|s| s.borrow_mut().clear());
}

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
// reach a resolver, so no shipped world exercises the wasm path yet.

/// Native/headless fallback: read a world's sibling `.rhai` from the filesystem.
#[cfg(not(target_arch = "wasm32"))]
pub struct FsScriptFallback;

#[cfg(not(target_arch = "wasm32"))]
impl crate::world::script::load::ScriptResolver for FsScriptFallback {
    fn read(&self, path: &str) -> Option<String> {
        std::fs::read_to_string(path).ok()
    }
}

/// The production script resolver for this target: the mod-pack overlay first
/// (folding pack scripts into the content digest, issue #988), then the
/// filesystem.
#[cfg(not(target_arch = "wasm32"))]
pub fn production_script_resolver() -> OverlayScriptResolver<FsScriptFallback> {
    OverlayScriptResolver::new(FsScriptFallback)
}

/// WASM fallback: read a sibling `.rhai` from the config cache's JS-delivered
/// world/script content. Async prefetch of siblings is a later slice; a world
/// that authors only inline `[script]` blocks (the tested path) never consults
/// this, and `None` here surfaces as a `script-file-missing` finding that
/// blocks activation.
#[cfg(target_arch = "wasm32")]
pub struct ConfigCacheScriptFallback;

#[cfg(target_arch = "wasm32")]
impl crate::world::script::load::ScriptResolver for ConfigCacheScriptFallback {
    fn read(&self, path: &str) -> Option<String> {
        peek_pending_world_toml(path)
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
                    CONFIG_CACHE.with(|cache| {
                        cache.borrow_mut().insert(requested, config);
                    });
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
        web_sys::console::error_1(&JsValue::from_str(&format!(
            "Entity template failed to load: {message}"
        )));
    }

    // Check if preload is complete. A failed template still counts as
    // "processed" — `drain_resolved_templates` settles it — so a bad TOML must
    // not permanently block finishInit().
    let has_pending = PENDING_QUEUE.with(|q| !q.borrow().is_empty());
    let has_in_flight = IN_FLIGHT.with(|q| !q.borrow().is_empty());
    if !has_pending && !has_in_flight {
        PRELOAD_COMPLETE.with(|flag| {
            *flag.borrow_mut() = true;
        });
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
    let world_config = crate::world::config::parse_world(&toml_str).map_err(|e| {
        web_sys::console::error_1(&JsValue::from_str(&format!(
            "Failed to parse world TOML at {}: {}",
            path, e
        )));
        JsValue::from_str(&format!("World parse error at {}: {}", path, e))
    })?;

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

/// Peek the TOML for a previously-fetched world path without removing it.
///
/// The pre-load scenario catalog (issue #754) resolves several world TOMLs to
/// read each scenario's `[global]` metadata and `[[available_ships]]`; unlike
/// the additive-load path it must not consume the cache, so worlds stay
/// available for the eventual `wasm_load_world`.
#[cfg(target_arch = "wasm32")]
pub fn peek_pending_world_toml(path: &str) -> Option<String> {
    PENDING_WORLD_TOML.with(|m| m.borrow().get(path).cloned())
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
    let already = WORLD_FETCH_REQUESTED.with(|s| s.borrow().contains(&path));
    if already {
        return;
    }
    WORLD_FETCH_REQUESTED.with(|s| s.borrow_mut().insert(path.clone()));
    // Mod-pack overlay wins for any overridden world path (issue #760, AC2):
    // satisfy the fetch directly from the uploaded pack instead of firing the
    // JS HTTP callback, so an additive extra_worlds load reads pack content.
    if let Some(toml_str) = mod_pack_overlay_get(&path) {
        PENDING_WORLD_TOML.with(|m| {
            m.borrow_mut().insert(path, toml_str);
        });
        return;
    }
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
///
/// `optional` is passed to the page as the callback's second argument: it says
/// this read is an EXISTENCE TEST, so a 404 is one of its expected answers and
/// the page must resolve it to the empty string without logging a failed fetch.
/// Only the legacy ladder-convention probe sets it (see
/// [`crate::entities::glb_visual::Absence`]); every other sidecar read still
/// wants a missing file reported.
#[cfg(target_arch = "wasm32")]
pub fn request_sidecar_fetch(path: String, optional: bool) {
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

/// Get the loaded FactionRegistry.
///
/// Pre-populates from compile-time includes if the thread-local is still
/// empty (e.g. when `wasm_load_faction` was never called from JS due to
/// missing wiring).  This ensures the registry always contains the four
/// built-in factions — Federation, Pirate, Harrow, Requiem — on every
/// target, WASM included.
#[cfg(target_arch = "wasm32")]
pub fn get_faction_registry() -> crate::ai::faction::FactionRegistry {
    FACTION_REGISTRY.with(|reg| {
        if reg.borrow().is_empty() {
            for toml_str in &[
                include_str!("../../assets/factions/federation.toml"),
                include_str!("../../assets/factions/pirate.toml"),
                include_str!("../../assets/factions/harrow.toml"),
                include_str!("../../assets/factions/requiem.toml"),
            ] {
                if let Ok(config) = crate::ai::faction::parse_faction_config(toml_str) {
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
pub fn get_cached_entity_config(path: &str) -> Option<crate::entities::config::EntityConfig> {
    CONFIG_CACHE.with(|cache| cache.borrow().get(path).cloned())
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

/// Newtype wrapper so `FactionRegistry` can be inserted as a Bevy Resource.
#[derive(Resource)]
pub struct FactionRegistryResource(pub crate::ai::faction::FactionRegistry);

#[cfg(target_arch = "wasm32")]
impl std::ops::Deref for FactionRegistryResource {
    type Target = crate::ai::faction::FactionRegistry;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl std::ops::Deref for FactionRegistryResource {
    type Target = crate::ai::faction::FactionRegistry;
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
pub fn pop_pending_world_toml(_path: &str) -> Option<String> {
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

#[cfg(not(target_arch = "wasm32"))]
pub fn get_faction_registry() -> crate::ai::faction::FactionRegistry {
    let mut registry = crate::ai::faction::FactionRegistry::new();
    for toml_str in &[
        include_str!("../../assets/factions/federation.toml"),
        include_str!("../../assets/factions/pirate.toml"),
        include_str!("../../assets/factions/harrow.toml"),
        include_str!("../../assets/factions/requiem.toml"),
    ] {
        if let Ok(config) = crate::ai::faction::parse_faction_config(toml_str) {
            registry.insert(config);
        }
    }
    registry
}

// ── Unit Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
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

    // ── Mod-pack overlay stack: session thread-local (issues #760, #987) ─
    //
    // Each test uses UNIQUE pack ids + paths and clears the stack at the end so
    // the process-wide `ACTIVE_PACKS` thread-local stays order-independent.

    #[test]
    fn pushing_pack_b_after_a_does_not_evict_a() {
        super::clear_mod_pack_overlay();
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

    #[test]
    fn reorder_mod_packs_reassigns_the_winner() {
        super::clear_mod_pack_overlay();
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
        super::clear_mod_pack_overlay();
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

        let world: toml::Value = toml::from_str(r#"script = "on_combat.rhai""#).unwrap();

        // WITHOUT a pack: the sibling cannot resolve (no overlay, no fallback),
        // so only the world itself is in the ledger.
        super::clear_mod_pack_overlay();
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
        super::record_raw_template(HULL, std::fs::read_to_string(HULL).unwrap());
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
                let body = std::fs::read_to_string(&path).unwrap_or_else(|e| {
                    panic!("the loader asked for {path}, which must exist: {e}")
                });
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
}
