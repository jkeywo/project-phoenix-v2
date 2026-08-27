//! The strict native entity-template preload (issue #1121).
//!
//! The browser fills the entity-template cache from a JS-driven preload before
//! Bevy starts. A native process has to do the equivalent itself, because
//! several simulation paths read
//! [`config_cache`](crate::entities::config_cache) with **no filesystem
//! fallback of their own** — `lobby::server::update_session_with_config`
//! (the hull's helm radar range, impulse-charge duration and hostile-arc
//! colour), `server::radar`, `server::reference_grid`, `server::asset_preload`
//! and `asteroids::lifecycle`. With an unpopulated cache those paths do not
//! fail; they quietly answer with `Default` values, which is the worst failure
//! shape available.
//!
//! This walk used to live inside `headless::app`, behind the `headless`
//! feature, which meant the one native host that most needed it — the windowed
//! authoritative host of issue #1121 — could not reach it. It moved here so
//! that **every** native process runs the same populate: recursive, sorted (the
//! load order is observable through
//! [`content_ledger::record`](crate::content_ledger::record)), skipping the
//! non-spawnable `fragments/` tree, validating the model-marker contract, and
//! gathering the AI-declaration manifest.
//!
//! # What the move deliberately STRENGTHENED
//!
//! Two behaviours changed, and neither is an accident of relocation — say so
//! here rather than let "moved unchanged" quietly cover them:
//!
//! 1. **The cache key is canonicalised.** It was
//!    `path.to_string_lossy().replace('\\', "/")`; it is now that string through
//!    [`canonical_template_path`](crate::entities::include_resolve::canonical_template_path),
//!    the same normalisation the content ledger keys by. Headless only ever
//!    passed a root derived from `--ship`'s own directory, so in practice its
//!    keys were already canonical; a root spelled `./assets/entities` would have
//!    keyed every entry under `./assets/entities/…` and matched nothing a world
//!    TOML authors — a full cache that answers every lookup with a miss. Safe
//!    for the headless caller because canonicalising a key that was already
//!    canonical is the identity.
//! 2. **A walk that caches zero templates is now an error**, where it returned
//!    `Ok((0, …))`. Headless's caller never observed a zero: it walks the
//!    directory of a `--ship` it is about to load, so a zero means the hull it
//!    was handed does not exist and the run was going to fail a step later
//!    anyway — now it fails here, naming the directory, instead of one step on
//!    with every cache-only reader answering `Default`.
//!
//! It is deliberately NOT the same walk as
//! [`delivery::serve::preload_templates`](crate::delivery::serve::preload_templates),
//! which is unsorted, validates no markers and silently skips anything that
//! fails to parse. That one exists to enrich a published catalogue, where a
//! missing field is cosmetic. This one feeds a simulation, where a missing
//! template is a wrong answer — so a caller wanting both wants **this** one,
//! once (both write the same process-global cache).
//!
//! **Process-global.** Everything here ends in
//! [`insert_native_config`](crate::entities::config_cache::insert_native_config),
//! so callers belong in a binary or an *integration* test, never an inline
//! `#[cfg(test)] mod tests` — see AGENTS.md's testing strategy.

use bevy::log::{debug, info, warn};

use crate::entities::ai_declaration_manifest;
use crate::entities::config::EntityConfig;
use crate::entities::marker_validate::MarkerFinding;

/// Everything one [`preload_entity_templates`] walk produced.
///
/// A caller cannot build one of these by hand, which is the point: it is the
/// *receipt* that the native template cache was populated, and
/// [`crate::native_host::build_native_host_app`] takes one by reference so that
/// "I forgot to preload" is a type error rather than a silently defaulted hull.
#[derive(Debug)]
pub struct TemplatePreload {
    /// How many templates parsed and reached the cache.
    loaded: usize,
    /// Model-marker contract findings across every discovered template — see
    /// [`TemplatePreload::marker_gate`].
    marker_findings: Vec<MarkerFinding>,
    /// The fleet-wide AI-declaration manifest (issue #885a), held until a
    /// `tracing` subscriber exists to receive it.
    ai_declarations: AiDeclarationReport,
}

impl TemplatePreload {
    /// How many templates reached the cache.
    pub fn loaded(&self) -> usize {
        self.loaded
    }

    /// The model-marker contract gate (issue #758).
    ///
    /// Validates EVERY template the walk discovered — not just the ones this
    /// run will actually spawn — and a single error must abort the build before
    /// an `App` is composed.
    ///
    /// That is deliberately stricter than the preload's own parse-skip policy
    /// (a template that fails to *parse* is skipped so one bad cosmetic
    /// asteroid cannot stop a combat test). The asymmetry is the point: a parse
    /// failure is loud and self-limiting — the template simply isn't in the
    /// cache, so anything that needs it fails visibly — whereas an unresolved
    /// marker is silent by construction. It attaches the beam, exhaust, or
    /// camera to the ship's centre and produces a plausible-looking run whose
    /// numbers are wrong.
    pub fn marker_gate(&self) -> Result<(), String> {
        if !crate::entities::marker_validate::has_error(&self.marker_findings) {
            return Ok(());
        }
        let errors: Vec<String> = self
            .marker_findings
            .iter()
            .filter(|f| f.is_error())
            .map(MarkerFinding::describe)
            .collect();
        Err(format!(
            "model-marker contract violated; spawning blocked ({} error(s)): {}",
            errors.len(),
            errors.join("; ")
        ))
    }

    /// Emit everything the walk gathered before any `tracing` subscriber
    /// existed: the non-error marker findings and the AI-declaration manifest.
    ///
    /// Call this AFTER the boot's `LogPlugin::build` has installed the global
    /// subscriber — anything emitted before it is silently dropped.
    pub fn report(&self) {
        for f in self.marker_findings.iter().filter(|f| !f.is_error()) {
            warn!(target: "assets", "marker validation [warn] {}", f.describe());
        }
        self.ai_declarations.emit();
    }
}

/// Every spawnable template under `dir`, recursively, EXCEPT the fragment tree.
///
/// Recursive since issue #954, which moved the three-weapon RNG-coverage escort
/// to `assets/entities/test/rng_coverage_lancer.toml` so that no *shipped fleet*
/// hull carries all three weapon kinds. That relocation is invisible to the
/// fleet walks, which read the top level only — but it must NOT be invisible
/// here.
///
/// **The reason has changed since #973, and the old one is no longer true.** It
/// used to be that the spawn path was cache-only, so a world naming a template
/// this walk skipped logged "entity template not found in cache" and silently
/// spawned nothing. `entity_loader::resolve_entity_via` now falls back to
/// `WasmTemplateLoader`, which on native reads the filesystem, so that
/// particular hole is closed at the spawn rather than here. What the recursive
/// walk still buys is the model-marker contract gate, which is scoped to
/// exactly the templates this walk discovers and validates them *before*
/// `App::new()` — a template it skips is a template whose markers nobody
/// checks. Keeping the cache complete is a second, larger benefit than it once
/// was: the cache-only readers named in this module's docs have no filesystem
/// fallback at all.
///
/// `fragments/` is the one subdirectory excluded, and it is excluded for a
/// reason that is a property of its contents rather than of its name: nothing in
/// it is spawnable. They are partial documents that hulls compose FROM (see
/// `include_resolve::tests::the_fragments_live_outside_the_shipped_template_directory`),
/// so caching them as templates would offer the world loader entities that are
/// not entities.
fn spawnable_templates_under(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<std::path::PathBuf> = entries.flatten().map(|e| e.path()).collect();
    // Sorted so the cache is populated in the same order on every filesystem —
    // the load order is observable through `content_ledger::record`.
    paths.sort();
    for path in paths {
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n == "fragments") {
                continue;
            }
            spawnable_templates_under(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("toml") {
            out.push(path);
        }
    }
}

/// Load every spawnable template under `dir` into the native template cache.
///
/// Templates that fail to parse are reported and skipped rather than aborting
/// the run — `assets/entities/` holds a lot of files and one bad cosmetic
/// asteroid should not stop a combat test. A template whose `includes` closure
/// fails to *compose* does abort: a template that declares includes has said it
/// is incomplete on its own, so skipping it would silently drop content the
/// author explicitly assembled.
///
/// A directory that does not exist, or that yields **zero** templates, is an
/// error rather than an empty success. A preload that silently caches nothing
/// is the worst possible way to report a wrong `--content-dir`: every one of
/// the cache-only readers this module exists for would answer `Default` and the
/// run would look plausible.
pub fn preload_entity_templates(dir: &str) -> Result<TemplatePreload, String> {
    // Trailing slash trimmed for the same reason the old `format!`-built key did
    // it: the cache key is this path with separators normalised, and
    // `"assets/entities/"` would key everything under `assets/entities//…`,
    // which matches nothing a world file authors.
    let root = std::path::Path::new(dir.trim_end_matches('/'));
    std::fs::read_dir(root).map_err(|e| format!("could not list {dir:?}: {e}"))?;
    let mut entries: Vec<std::path::PathBuf> = Vec::new();
    spawnable_templates_under(root, &mut entries);

    let mut loaded = 0;
    let mut marker_findings: Vec<MarkerFinding> = Vec::new();
    // Accumulated across the whole template set so the summary is a fleet total
    // rather than a per-file trickle.
    let mut ai_declarations = AiDeclarationReport::default();
    for path in entries {
        // Key on the repo-relative path the world TOML uses, with forward
        // slashes — on Windows `Path::display` would emit backslashes and every
        // lookup would miss. Built from the WHOLE path rather than
        // `dir` + file name, so a template in a subdirectory is keyed by the
        // path a world actually names it with
        // (`assets/entities/test/rng_coverage_lancer.toml`, not
        // `assets/entities/rng_coverage_lancer.toml`).
        //
        // Through `canonical_template_path` because the cache is looked up by
        // the path a WORLD authors, and a caller whose root is spelled `./…`
        // would otherwise key everything under `./assets/entities/…` and match
        // nothing — a full cache that answers every lookup with a miss, which
        // is the exact silent-default failure this module exists to prevent.
        // The same normalisation the content ledger keys by.
        let key = crate::entities::include_resolve::canonical_template_path(
            &path.to_string_lossy().replace('\\', "/"),
        );
        if std::fs::read_to_string(&path).is_err() {
            warn!(target: "config", "template unreadable, skipping: {key}");
            continue;
        }
        // Resolve the template's `includes` closure BEFORE parsing (issue
        // #869): only the fully composed document is ever validated, and it is
        // the composed text that marker validation and the AI-declaration
        // manifest must read.
        let resolved = match crate::entities::include_resolve::resolve_from_disk(&key) {
            Ok(resolved) => resolved,
            Err(e) => return Err(format!("template composition failed: {e}")),
        };
        let composed = resolved.is_composed();
        let toml = resolved.toml.clone();
        match resolved.parse() {
            Ok(cfg) => {
                marker_findings.extend(validate_template_markers(&key, &toml, &cfg));
                let stem = path.file_stem().unwrap_or_default().to_string_lossy();
                let missing = ai_declaration_manifest::undeclared_keys(&cfg).len();
                if missing > 0 {
                    ai_declarations.undeclared += missing;
                    ai_declarations.templates_with_gaps += 1;
                    ai_declarations
                        .lines
                        .extend(ai_declaration_manifest::manifest_lines(&stem, &cfg));
                }
                crate::entities::config_cache::insert_native_config(key, cfg);
                loaded += 1;
            }
            Err(e) if composed => {
                // A composed template that does not validate is a load error:
                // the offending combination exists in no single authored file,
                // so skipping it would hide the one thing composition can get
                // wrong that authoring cannot.
                return Err(format!("composed template is invalid: {e}"));
            }
            Err(e) => warn!(target: "config", "template failed to parse, skipping: {key}: {e}"),
        }
    }

    if loaded == 0 {
        return Err(format!(
            "no entity templates loaded from {dir:?} — a native host booted from \
             here would read Default hull, radar and asteroid configuration \
             instead of the authored ones"
        ));
    }

    Ok(TemplatePreload {
        loaded,
        marker_findings,
        ai_declarations,
    })
}

/// The fleet-wide AI-declaration manifest gathered while preloading templates
/// (issue #885a), held until a `tracing` subscriber exists to receive it.
#[derive(Debug, Default)]
struct AiDeclarationReport {
    /// AI-capable fine systems that declared neither a policy nor an explicit
    /// idle state, across every template loaded.
    undeclared: usize,
    /// How many templates contributed at least one of those.
    templates_with_gaps: usize,
    /// One rendered line per (template, fine system) — the per-slot worklist.
    lines: Vec<String>,
}

impl AiDeclarationReport {
    /// Emit the manifest. `info` for the fleet total, `debug` for the per-slot
    /// worklist: both sit under the default `warn` filter, so a normal run is
    /// unchanged and `--log config=debug` is what asks for the breakdown.
    fn emit(&self) {
        if self.undeclared == 0 {
            return;
        }
        info!(
            target: "config",
            "AI-declaration manifest: {} AI-capable fine system(s) across {} \
             template(s) declare neither a policy nor an explicit idle state, so a \
             Rust-side synthesiser supplies their automation (PRD #774 US7; issue \
             #885b's worklist). Run with `--log config=debug` for the \
             per-(template, system) breakdown.",
            self.undeclared,
            self.templates_with_gaps
        );
        for line in &self.lines {
            debug!(target: "config", "{line}");
        }
    }
}

/// Model-marker contract check for one parsed template: resolve its rig
/// sidecar off disk (identity rig when genuinely absent, mirroring
/// `glb_visual::resolve_sidecar_rig` on native) and validate every authored
/// marker reference against it, plus the sidecar's own duplicate declarations.
fn validate_template_markers(key: &str, toml: &str, cfg: &EntityConfig) -> Vec<MarkerFinding> {
    let mut findings = Vec::new();
    let rig = cfg.mesh.as_ref().and_then(|mesh| {
        let model = mesh.model.as_deref()?;
        let path = crate::entities::model_rig::sidecar_path(model, mesh.variant.as_deref());
        let sidecar = std::fs::read_to_string(&path).unwrap_or_default();
        findings.extend(crate::entities::marker_validate::duplicate_marker_findings(
            &path, &sidecar,
        ));
        match crate::entities::model_rig::ModelRig::from_toml(&sidecar) {
            Ok(rig) => Some(rig),
            Err(e) => {
                warn!(target: "config", "rig sidecar {path} failed to parse: {e}");
                None
            }
        }
    });
    findings.extend(crate::entities::marker_validate::validate_entity_markers(
        key,
        toml,
        cfg,
        rig.as_ref(),
    ));
    findings
}
