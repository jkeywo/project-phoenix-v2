use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

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
    pub ready: Vec<(String, crate::include_resolve::ResolvedTemplate)>,
    /// Canonical fragment paths that must still be fetched. Deduplicated.
    pub fetch: Vec<String>,
    /// Composition failures. Every one of these is a load error: the entity
    /// never enters the config cache, so nothing partially composed spawns.
    pub errors: Vec<crate::include_resolve::IncludeError>,
}

/// Record the raw TOML the host delivered for `path`.
pub fn record_raw_template(path: &str, toml_str: String) {
    let key = crate::include_resolve::canonical_template_path(path);
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
    let key = crate::include_resolve::canonical_template_path(path);
    RAW_TEMPLATE_TOML.with(|m| m.borrow().get(&key).cloned())
}

/// Note that `path` was requested as an entity template, not as a fragment.
pub fn mark_entity_template(path: &str) {
    let canonical = crate::include_resolve::canonical_template_path(path);
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
    let key = crate::include_resolve::canonical_template_path(path);
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
        match crate::include_resolve::preload_step(&canonical, &raw) {
            Ok(crate::include_resolve::PreloadStep::Ready(resolved)) => {
                SETTLED_TEMPLATE_PATHS.with(|s| s.borrow_mut().insert(canonical));
                progress.ready.push((requested, *resolved));
            }
            Ok(crate::include_resolve::PreloadStep::AwaitingIncludes(paths)) => {
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
/// companion to [`crate::overlay::clear_mod_pack_overlay`] for a same-page next round.
pub fn clear_template_preload_state() {
    RAW_TEMPLATE_TOML.with(|m| m.borrow_mut().clear());
    ENTITY_TEMPLATE_PATHS.with(|v| v.borrow_mut().clear());
    SETTLED_TEMPLATE_PATHS.with(|s| s.borrow_mut().clear());
}
