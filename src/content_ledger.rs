//! The loaded-content ledger (issue #935).
//!
//! `snapshot::content_digest` used to hash the scenario TOML text alone.
//! That left entity templates, fragments, and sidecars free to drift under a
//! save: `apply_hull` on restore trusts the fresh world's authored maxima, so
//! an edit to `assets/entities/*.toml` moved nothing in the version a save was
//! checked against — the exact silent-drift case a *content* dimension exists
//! to refuse. This module is the fix: every authored file the loader actually
//! reads is recorded here, keyed by its canonical path, and
//! [`ContentLedger::fold`] is what `content_digest` folds instead of a lone
//! string.
//!
//! # Where this is filled
//!
//! * The scenario/world TOML — `world::server::load_scenario_toml` (layers)
//!   and the two per-target world-load entry points (`server::bridge::
//!   wasm_load_world` on wasm, `headless::app::build_headless_app` on native).
//! * Entity templates and their `includes` fragments —
//!   `entity_loader::FsTemplateLoader::load_template` on native,
//!   `config_cache::wasm_load_config`'s resolved-template loop on wasm. Both
//!   record the same thing: [`crate::entities::include_resolve::ResolvedTemplate::toml`],
//!   the byte-stable composed document, keyed by the template's canonical
//!   path — so a shared fragment moving the digest is visible on either
//!   target without the two recording different shapes.
//! * Model-rig sidecars — `entities::glb_visual::load_sidecar_toml`.
//! * Pack-supplied Rhai scripts — `config_cache::OverlayScriptResolver` records
//!   every script it resolves (issue #988), so a scenario loaded with a
//!   script-carrying mod pack folds a different content digest than the same
//!   scenario without it, exactly as an edited entity template does.
//!
//! Deliberately NOT recorded from: the diagnostic bulk preload in
//! `headless::app::preload_entity_templates` (walks every file under
//! `assets/entities/`, not the set THIS scenario consumes — recording it would
//! turn the content digest into a repo-wide hash and break native/wasm parity,
//! since the browser only ever fetches the scenario's own declared set).
//!
//! # Live ledger vs. frozen digest
//!
//! Templates spawn lazily as a world streams (issue #904's Combat Test belts
//! are the canonical case), so the *live* ledger's fold would drift with how
//! far a session has gotten — two loads of the same, unedited scenario could
//! disagree on content merely because one had streamed further than the
//! other. That is not the drift this ledger exists to report. [`freeze`]
//! snapshots the ledger once the world's *declared* file set is fully known —
//! after wasm's JS-driven preload completes (`wasm_init`), after native's
//! eager walk of the world's referenced templates
//! ([`eager_record_world_entities`]) — and [`frozen_or_live`] is what
//! `content_digest` callers read, so the digest a save is checked against is
//! fixed at load time regardless of how much of the world has since streamed
//! in.
//!
//! # Reset semantics
//!
//! [`reset`] clears both the live ledger and any frozen snapshot. It must be
//! called at the START of a new scenario/world load, not at its end — a
//! ledger that kept yesterday's world's files in it while today's loads
//! record over them would be the same silent-drift bug wearing a new hat.

use std::cell::RefCell;
use std::collections::BTreeMap;

thread_local! {
    /// Canonical path -> `fnv1a` digest of the text last recorded for it.
    /// Grows as the loader consumes files; never shrinks except via [`reset`].
    static LEDGER: RefCell<BTreeMap<String, u64>> = const { RefCell::new(BTreeMap::new()) };

    /// The ledger's state at the moment [`freeze`] was last called, or `None`
    /// before the first freeze (and after [`reset`]).
    static FROZEN: RefCell<Option<ContentLedger>> = const { RefCell::new(None) };
}

/// Canonicalise a ledger key exactly the way `entity_includes::
/// canonical_template_path` does — forward slashes, normalised segments — so
/// a world-TOML path and an entity-template path collapse to the same key
/// shape a designer would recognise, and so native and wasm agree on the key
/// for identical authored paths regardless of which slash style the host
/// delivered.
pub fn normalize_key(path: &str) -> String {
    crate::entities::include_resolve::canonical_template_path(path)
}

/// Record that the loader consumed `text` at `path`. Stores only `text`'s
/// digest, not the bytes — the ledger can hold every template a large world
/// touches without holding a second copy of the asset tree in memory.
pub fn record(path: &str, text: &str) {
    record_digest(path, vellum_digest::fnv1a(text.as_bytes()));
}

/// Record an already-computed digest for `path`. The lower-level primitive
/// [`record`] is built on; exposed so a caller that already has a digest
/// (or a test simulating a content change) never has to round-trip through
/// text it does not otherwise need.
pub fn record_digest(path: &str, digest: u64) {
    let key = normalize_key(path);
    LEDGER.with(|l| {
        l.borrow_mut().insert(key, digest);
    });
}

/// Clear the live ledger and any frozen snapshot. Call at the START of a new
/// scenario/world load — see the module docs' reset-semantics section.
pub fn reset() {
    LEDGER.with(|l| l.borrow_mut().clear());
    FROZEN.with(|f| *f.borrow_mut() = None);
}

/// A point-in-time copy of the live ledger.
pub fn snapshot() -> ContentLedger {
    LEDGER.with(|l| ContentLedger(l.borrow().clone()))
}

/// Snapshot the live ledger and hold it as the frozen digest input. Call once
/// the current load's declared file set is fully known — see the module
/// docs.
pub fn freeze() {
    let live = snapshot();
    FROZEN.with(|f| *f.borrow_mut() = Some(live));
}

/// The frozen snapshot if [`freeze`] has been called since the last
/// [`reset`], otherwise a snapshot of the live ledger — the fallback a unit
/// test or a not-yet-frozen caller gets rather than an empty ledger.
pub fn frozen_or_live() -> ContentLedger {
    FROZEN.with(|f| f.borrow().clone()).unwrap_or_else(snapshot)
}

/// A folded set of `(path, digest)` pairs, sorted by path.
///
/// `BTreeMap`-backed rather than a `Vec` recorded in load order: iteration is
/// already path-sorted, so [`ContentLedger::fold`] is deterministic
/// regardless of the order the loader happened to touch files in, on either
/// target.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ContentLedger(BTreeMap<String, u64>);

impl ContentLedger {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Fold every `(path, digest)` pair into one `u64`, path-sorted so the
    /// result does not depend on recording order.
    ///
    /// Reuses `sim_digest`'s fold helpers rather than a third digest
    /// primitive — `fold_str`/`fold_u64` are already the crate's answer to
    /// "fold a named field into an accumulator".
    pub fn fold(&self) -> u64 {
        let mut acc = vellum_digest::FOLD_SEED;
        for (path, digest) in &self.0 {
            acc = crate::sim_digest::fold_str(acc, path);
            acc = crate::sim_digest::fold_u64(acc, *digest);
        }
        acc
    }
}

/// Eagerly resolve and record every entity template a world's `[[entity]]`
/// list references, recursively through nested asteroid-field variants.
///
/// Native's answer to the browser's JS-driven preload (which fetches the same
/// declared set before `wasm_init`): without this, native only learns a
/// template's content lazily, the first time something spawns it, and a
/// streamed world's declared set would not be fully known until streaming
/// finished — which is precisely the load-order sensitivity [`freeze`]
/// exists to avoid. Call this BEFORE [`freeze`], after the world config is
/// parsed and before anything spawns.
#[cfg(not(target_arch = "wasm32"))]
pub fn eager_record_world_entities(world_config: &crate::world::config::WorldConfig) {
    use crate::entities::loader::TemplateLoader;
    use std::collections::HashSet;

    let mut queue: Vec<String> = world_config
        .entities
        .iter()
        .map(|e| e.template_path.clone())
        .collect();
    let mut visited: HashSet<String> = HashSet::new();

    while let Some(path) = queue.pop() {
        let key = normalize_key(&path);
        if !visited.insert(key) {
            continue;
        }
        if let Some(config) = crate::entities::loader::FsTemplateLoader.load_template(&path) {
            queue.extend(crate::entities::config_cache::nested_template_paths(
                &config,
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_then_reset_empties_the_ledger() {
        reset();
        record("assets/entities/a.toml", "a");
        assert!(!snapshot().is_empty());
        reset();
        assert!(snapshot().is_empty());
        assert!(frozen_or_live().is_empty());
    }

    #[test]
    fn fold_does_not_depend_on_record_order() {
        reset();
        record("assets/entities/a.toml", "a");
        record("assets/entities/b.toml", "b");
        let forward = snapshot().fold();

        reset();
        record("assets/entities/b.toml", "b");
        record("assets/entities/a.toml", "a");
        let backward = snapshot().fold();

        assert_eq!(forward, backward, "record order must not move the digest");
        reset();
    }

    #[test]
    fn different_content_moves_the_fold() {
        reset();
        record("assets/entities/a.toml", "a");
        let before = snapshot().fold();

        reset();
        record("assets/entities/a.toml", "a-edited");
        let after = snapshot().fold();

        assert_ne!(before, after, "an edited file's text must move the digest");
        reset();
    }

    #[test]
    fn freeze_is_stable_against_later_recording() {
        reset();
        record("assets/entities/a.toml", "a");
        freeze();
        let frozen = frozen_or_live().fold();

        // A later record — simulating a template streaming in after the
        // world's declared set was already frozen — must not move the
        // digest a save is checked against.
        record("assets/entities/b.toml", "b");
        assert_eq!(
            frozen_or_live().fold(),
            frozen,
            "recording after freeze must not move the frozen digest"
        );
        reset();
    }

    #[test]
    fn backslash_and_forward_slash_paths_key_the_same() {
        reset();
        record("assets\\entities\\a.toml", "a");
        record("assets/entities/a.toml", "a");
        assert_eq!(
            snapshot().len(),
            1,
            "the two spellings of the same path must collapse to one entry"
        );
        reset();
    }
}
