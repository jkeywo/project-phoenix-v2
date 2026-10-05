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
//!   record the same thing: [`crate::include_resolve::ResolvedTemplate::toml`],
//!   the byte-stable composed document, keyed by the template's canonical
//!   path — so a shared fragment moving the digest is visible on either
//!   target without the two recording different shapes.
//! * Primary model-rig sidecars — entity-config preload on wasm and
//!   `entities::loader::FsTemplateLoader` on native bind their exact bytes
//!   before freeze; `entities::model_markers::resolve_sidecar_rig` reuses the
//!   delivered body at runtime.
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
//! after wasm's JS-driven preload AND root-script compilation complete (the
//! `WorldPlugin` Startup chain), after native's eager walk of the world's
//! referenced templates and available hulls
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

    /// Paths [`note_uncovered_spawn`] has already reported, so a wave that
    /// spawns the same computed hull sixty times says so once (issue #1047).
    /// Cleared by [`reset`] with the rest of the load's state.
    static NOTED_UNCOVERED: RefCell<std::collections::BTreeSet<String>> =
        const { RefCell::new(std::collections::BTreeSet::new()) };
}

/// Canonicalise a ledger key exactly the way `entity_includes::
/// canonical_template_path` does — forward slashes, normalised segments — so
/// a world-TOML path and an entity-template path collapse to the same key
/// shape a designer would recognise, and so native and wasm agree on the key
/// for identical authored paths regardless of which slash style the host
/// delivered.
pub fn normalize_key(path: &str) -> String {
    crate::include_resolve::canonical_template_path(path)
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

/// One [`record_digest`] write, returned as DATA for a caller to apply (issue
/// #1241).
///
/// The digest counterpart to [`crate::world::load::LedgerRecord`], which carries
/// the text a load read. This one carries a digest already computed below the
/// load — a compiled script set's `content_hash` — so it never round-trips
/// through bytes nobody needs.
///
/// It lives HERE rather than beside its sibling in `world::load` for a layering
/// reason: `world::script::load` produces it and sits *below* `world::load` (the
/// load sequence wraps the script loader, not the other way round), so the type
/// has to come from a module below both. `world::load` re-exports it so a caller
/// reading a [`LedgerPlan`](crate::world::load::LedgerPlan) finds both halves of
/// its vocabulary in one place.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LedgerDigest {
    /// The ledger key to store under, before canonicalisation.
    pub key: String,
    /// The already-computed digest.
    pub digest: u64,
}

impl LedgerDigest {
    /// Apply this write to the live ledger.
    pub fn apply(&self) {
        record_digest(&self.key, self.digest);
    }
}

/// Clear the live ledger and any frozen snapshot. Call at the START of a new
/// scenario/world load — see the module docs' reset-semantics section.
pub fn reset() {
    LEDGER.with(|l| l.borrow_mut().clear());
    FROZEN.with(|f| *f.borrow_mut() = None);
    NOTED_UNCOVERED.with(|n| n.borrow_mut().clear());
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

/// Whether [`freeze`] has run since the last [`reset`] — i.e. whether there is a
/// settled content digest for anything to be measured against.
pub fn is_frozen() -> bool {
    FROZEN.with(|f| f.borrow().is_some())
}

/// Whether the frozen content set already covers `path` — i.e. whether an edit
/// to that file would move the digest a save is checked against.
pub fn frozen_covers(path: &str) -> bool {
    let key = normalize_key(path);
    FROZEN.with(|f| match f.borrow().as_ref() {
        Some(frozen) => frozen.0.contains_key(&key),
        // Not frozen yet: the live ledger is what `frozen_or_live` would hand a
        // digest caller, so it is what "covered" means at this moment.
        None => LEDGER.with(|l| l.borrow().contains_key(&key)),
    })
}

/// Report — ONCE per path per load — that a spawn resolved a template the frozen
/// content set does not cover (issue #1047). Returns `true` the first time, so
/// the caller logs once rather than every wave.
///
/// # Why this reports rather than records
///
/// The template it names is real content this run depended on, so the obvious
/// move is to fold it in late and let a save taken afterwards bind to it. That
/// would be a bug, and a worse one than the gap it closes.
///
/// [`freeze`] exists to make the content digest a function of the WORLD, not of
/// how far a session got — see the module docs. A template first seen at spawn
/// time is by definition session-progress-dependent: fold it in and a save taken
/// after wave three carries a digest a freshly-booted resume (which has spawned
/// nothing) cannot reproduce, so the resume refuses a save that is in fact
/// perfectly valid. Trading a missed detection for a false refusal is not a trade
/// worth making — a false refusal costs the player their run.
///
/// So the residual stands, and this makes it VISIBLE instead of silent: the run
/// says, once, which template its content digest does not cover. Every
/// statically-visible path is already covered by
/// [`eager_record_world_entities`]; what reaches here is the genuinely computed
/// path, which no load-time scan can resolve. A designer who wants the binding
/// turns the computed path into a literal one, and this line is what tells them
/// there is something to turn.
pub fn note_uncovered_spawn(path: &str) -> bool {
    // No frozen set means no claim to make. "Uncovered" is a statement RELATIVE
    // to the content digest a save would be stamped with, and until [`freeze`]
    // has run there is no such digest — a bare-`App` fixture, a unit test
    // dispatching one action, or any host mid-load would otherwise be told every
    // spawn is uncovered, which is noise rather than news.
    if !is_frozen() {
        return false;
    }
    if frozen_covers(path) {
        return false;
    }
    let key = normalize_key(path);
    NOTED_UNCOVERED.with(|n| n.borrow_mut().insert(key))
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

    /// The digest stored under `path`, if any. Keyed the same way [`record`]
    /// keys it, so a caller passes the authored path rather than pre-normalising.
    pub fn get(&self, path: &str) -> Option<u64> {
        self.0.get(&normalize_key(path)).copied()
    }

    /// Every `(key, digest)` pair, path-sorted — the whole ledger as data, for a
    /// test that wants to compare two loads rather than fold them to one number.
    pub fn entries(&self) -> impl Iterator<Item = (&str, u64)> {
        self.0.iter().map(|(k, v)| (k.as_str(), *v))
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
            acc = vellum_digest::fold_digest(acc, vellum_digest::fnv1a(path.as_bytes()));
            acc = vellum_digest::fold_digest(acc, vellum_digest::fnv1a(&digest.to_le_bytes()));
        }
        acc
    }
}
