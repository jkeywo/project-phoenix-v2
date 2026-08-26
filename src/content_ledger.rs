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
            acc = crate::sim_digest::fold_str(acc, path);
            acc = crate::sim_digest::fold_u64(acc, *digest);
        }
        acc
    }
}

/// Eagerly resolve and record every entity template a world can spawn — its
/// `[[entity]]` roster AND the templates its inline scripts name — recursively
/// through nested asteroid-field variants.
///
/// Native's answer to the browser's JS-driven preload: without this, native only
/// learns a template's content lazily, the first time something spawns it, and a
/// streamed world's declared set would not be fully known until streaming
/// finished — which is precisely the load-order sensitivity [`freeze`] exists
/// to avoid. Native's compiled-aware entry point also sees sibling-script
/// literals; browser inline roots have the preload equivalent, while browser
/// root-sibling pre-init parity is tracked as #1248. Call this BEFORE [`freeze`],
/// after the world config is parsed and before anything spawns.
///
/// # Scripted spawns are part of the declared set (issue #1047)
///
/// It walked `entities` alone, so a hull only a script spawned never entered the
/// ledger and never entered the frozen digest — and a save taken in such a world
/// loaded happily after that template changed on disk, while the same edit to a
/// declaratively-listed hull refused it. Issue #864 closed the script SOURCE half
/// of that gap (`CompiledScripts::content_hash` binds a save to the exact script
/// text); this is the template FILES half.
///
/// # Why here rather than on the `LedgerPlan`
///
/// Issue #1241 made [`crate::world::load::load`] return its ledger writes as data
/// instead of making them, and this walk deliberately stays out of that plan: it
/// needs a [`TemplateLoader`](crate::entities::loader::TemplateLoader) to resolve
/// each path's composed bytes, and `load` has none — it reads world TOML through
/// a `WorldReader` and knows nothing about entity templates. Putting the walk on
/// the plan would mean giving the load sequence a second I/O seam to do work its
/// one caller already does here, immediately after applying that plan and
/// immediately before [`freeze`]. The static ENUMERATION is pure and shared; only
/// the resolution is I/O, and the I/O stays with the eager walk that owns it.
///
/// # What it still cannot see
///
/// A COMPUTED `template_path` — `duel.toml`'s `spawn_slot(ctx, name, template,
/// …)`, whose hull comes from `--side-a`/`--side-b` — is invisible to any static
/// scan and always will be, so it cannot be in the frozen set.
/// [`note_uncovered_spawn`] is what makes that residual visible at the moment it
/// bites; see its docs for why such a template is deliberately NOT folded in
/// late.
#[cfg(not(target_arch = "wasm32"))]
pub fn eager_record_world_entities(world_config: &crate::world::config::WorldConfig) {
    eager_record_world_entities_with_scripts(world_config, None);
}

/// [`eager_record_world_entities`] using the exact script source set the loader
/// compiled when one exists.
///
/// Native boot passes its [`CompiledScripts`](crate::world::script::load::CompiledScripts)
/// here, so sibling `.rhai` files and inline virtual sources contribute the same
/// literal template references composition validation saw. A parsed config with
/// no compiled set — a direct unit fixture or another caller below the loader —
/// retains the inline [`script_spawned_templates`](crate::world::config::script_spawned_templates)
/// scan as its fallback. `Some` is authoritative even when its list is empty:
/// appending the fallback would reintroduce a second, less exact source set.
/// Static `extra_worlds` children are compiled for this pre-freeze enumeration;
/// their runtime layer activation still compiles and owns a fresh set of
/// registrations through the additive-layer path.
#[cfg(not(target_arch = "wasm32"))]
pub fn eager_record_world_entities_with_scripts(
    world_config: &crate::world::config::WorldConfig,
    scripts: Option<&crate::world::script::load::CompiledScripts>,
) {
    use crate::entities::loader::TemplateLoader;
    use std::collections::HashSet;

    let scripted_paths: Vec<String> = match scripts {
        Some(compiled) => compiled
            .spawned_templates
            .iter()
            .map(|spawn| spawn.template_path.clone())
            .collect(),
        None => crate::world::config::script_spawned_templates(world_config)
            .into_iter()
            .map(|spawn| spawn.template_path)
            .collect(),
    };

    // The `[[entity]]` roster AND every literal template the exact compiled
    // script set names (issue #1047). The fallback above preserves direct parsed-
    // config callers; production native boot supplies `CompiledScripts`.
    let mut queue: Vec<String> = world_config
        .entities
        .iter()
        .map(|e| e.template_path.clone())
        .chain(scripted_paths)
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
    // ── Script-spawned templates (issue #1047) ───────────────────────────────

    /// The issue's exact shape: a world where ONLY a script names a template.
    ///
    /// The eager walk used to see `[[entity]]` alone, so this hull never entered
    /// the ledger — and a save taken in such a world loaded happily after the
    /// template changed on disk, while the same edit to a declaratively-listed
    /// hull refused it.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn the_eager_walk_records_a_template_only_a_script_names() {
        const HULL: &str = "assets/entities/ship_harrow_destroyer.toml";
        let world = crate::world::config::parse_world(&format!(
            "[global]
seed = 1

[script]
setup = \"\"\"
             fn wave(ctx) {{
             ctx.effects.spawn_entity(#{{ template_path: \"{HULL}\", name: \"r1\" }});
             }}
\"\"\"
"
        ))
        .expect("fixture world parses");
        assert!(
            world.entities.is_empty(),
            "the point of the fixture: nothing declarative names the hull"
        );
        assert_eq!(
            crate::world::config::script_spawned_templates(&world)
                .into_iter()
                .map(|s| s.template_path)
                .collect::<Vec<_>>(),
            vec![HULL.to_string()],
            "and the shared enumeration does see it"
        );

        reset();
        eager_record_world_entities(&world);
        assert!(
            snapshot().get(HULL).is_some(),
            "a script-only hull must be recorded: {:?}",
            snapshot().entries().map(|(k, _)| k).collect::<Vec<_>>()
        );
        reset();
    }

    /// Production native boot must take its script-template set from the same
    /// resolved sources it compiled, not re-scan the parsed config's inline-only
    /// bodies. The two paths deliberately disagree here: the config names one
    /// hull inline while the compiled sibling names another.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn the_eager_walk_uses_the_compiled_sibling_source_set_when_available() {
        const INLINE_HULL: &str = "assets/entities/alliance_destroyer.toml";
        const SIBLING_HULL: &str = "assets/entities/ship_harrow_destroyer.toml";
        let world = crate::world::config::parse_world(&format!(
            "[global]
seed = 1

[script]
setup = \"\"\"
             fn inline_wave(ctx) {{
             ctx.effects.spawn_entity(#{{ template_path: \"{INLINE_HULL}\", name: \"inline\" }});
             }}
\"\"\"
"
        ))
        .expect("fixture world parses");
        let compiled = crate::world::script::load::compile_scripts(&[vellum_script::ScriptSource {
            path: "tests/fixtures/script_only_spawn.rhai".into(),
            source: format!(
                "fn sibling_wave(ctx) {{\n\
                 ctx.effects.spawn_entity(#{{ template_path: \"{SIBLING_HULL}\", name: \"sibling\" }});\n\
                 }}\n"
            ),
        }]);

        reset();
        eager_record_world_entities_with_scripts(&world, Some(&compiled));
        let recorded = snapshot();
        assert!(
            recorded.get(SIBLING_HULL).is_some(),
            "the resolved sibling's hull must enter the native frozen set"
        );
        assert!(
            recorded.get(INLINE_HULL).is_none(),
            "CompiledScripts is authoritative when present; the inline fallback must not append a second source set"
        );
        reset();
    }

    /// The acceptance, stated as the save-compat check sees it: editing that
    /// script-only template moves the content digest, which is what refuses the
    /// save. Before #1047 the two digests were equal and the save loaded.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn editing_a_script_only_template_moves_the_content_digest() {
        const HULL: &str = "assets/entities/ship_harrow_destroyer.toml";
        let world = crate::world::config::parse_world(&format!(
            "[global]
seed = 1

[script]
setup = \"\"\"
             fn wave(ctx) {{
             ctx.effects.spawn_entity(#{{ template_path: \"{HULL}\", name: \"r1\" }});
             }}
\"\"\"
"
        ))
        .expect("fixture world parses");

        // The digest a save would be stamped with at load time.
        reset();
        eager_record_world_entities(&world);
        freeze();
        let at_save = crate::snapshot::content_digest(&frozen_or_live());

        // The same world after the hull file changed on disk. Simulated by
        // recording different bytes under the same key — what the walk would do
        // for real on the next boot, without this test editing the repo.
        reset();
        eager_record_world_entities(&world);
        record(
            HULL,
            "# edited by a designer
",
        );
        freeze();
        let at_load = crate::snapshot::content_digest(&frozen_or_live());

        assert_ne!(
            at_save, at_load,
            "an edit to a script-spawned hull must move the content digest —              equality here is the #1047 bug"
        );
        reset();
    }

    /// The computed-path arm: a template the static walk cannot see is reported
    /// once, and only while it is genuinely uncovered.
    #[test]
    fn an_uncovered_spawn_is_reported_once_and_a_covered_one_never() {
        const COMPUTED: &str = "assets/entities/computed_hull.toml";
        const DECLARED: &str = "assets/entities/declared_hull.toml";

        reset();
        record(
            DECLARED, "[hull]
",
        );
        freeze();

        assert!(
            note_uncovered_spawn(COMPUTED),
            "the first spawn of an uncovered template reports"
        );
        assert!(
            !note_uncovered_spawn(COMPUTED),
            "and a wave that spawns it sixty more times says nothing further"
        );
        assert!(
            !note_uncovered_spawn(DECLARED),
            "a template the frozen set covers is never reported"
        );

        // Deliberately: reporting does NOT fold it in. A save taken after this
        // spawn must carry the same digest a freshly-booted resume computes, or
        // the resume refuses a valid save.
        assert!(
            !frozen_covers(COMPUTED),
            "note_uncovered_spawn must not quietly extend the frozen set"
        );
        reset();
    }

    /// `reset` clears the reported set with everything else, so a second world
    /// load in one process reports its own uncovered spawns rather than
    /// inheriting the previous world's silence.
    #[test]
    fn reset_re_arms_the_uncovered_spawn_report() {
        const COMPUTED: &str = "assets/entities/computed_hull.toml";
        reset();
        freeze();
        assert!(note_uncovered_spawn(COMPUTED));
        reset();
        freeze();
        assert!(
            note_uncovered_spawn(COMPUTED),
            "a new load must be able to report the same path again"
        );
        reset();
    }
}
