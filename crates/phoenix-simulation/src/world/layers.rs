//! Pure world-layer load/unload decision layer (issue #821).
//!
//! Pure Rust module — no Bevy. `LoadWorld` / `UnloadWorld` effects queue
//! `WorldLayerChange`s; the `apply_world_layer_changes` applier in
//! `world::server` performs the I/O (TOML read, entity spawn/despawn) and
//! resource mutation, while every decision — de-duplication, parse handling,
//! name→UUID assignment — lives here as plain functions over plain data.
//!
//! # A layer contributes ENTITIES and SCRIPTS, and nothing else
//!
//! A loaded layer used to merge its own `[[trigger]]` blocks into the live
//! `WorldContentRuntime` — origin-tagged with the layer path so `UnloadWorld`
//! could take exactly them back out — and that was the ONLY way a layer carried
//! scenario logic: scripts compiled on the standalone/base-world path only.
//! Issue #985 deleted the `[[trigger]]` parser, so a layer TOML has no way to
//! author a trigger at all, `evaluate_layer_load` has none to return, and
//! `evaluate_layer_unload` had nothing left to compute. Both went.
//!
//! Issue #1045 gives the capability back the way the deletion note said it would:
//! through the layer's `[script]` block, which compiles here and hands the applier
//! `ScriptTrigger`s to merge rather than parsed `Trigger`s. No shipped layer is
//! affected — `reinforcements.toml`, the one layer any shipped world loads,
//! authors neither a trigger nor a script.
//!
//! # The supporting-world script route, end to end
//!
//! This evaluation runs the layer through the one world-load sequence
//! ([`crate::world::load::load`]) under [`LoadPolicy::Merge`], which compiles the
//! layer's `[script]` block just as the base-world path compiles the base world's.
//! The compiled [`CompiledScripts`] is carried out on
//! [`LoadedLayer::scripts`], and the applier
//! (`world::server::apply_world_layer_changes`) merges it into the live
//! `WorldScriptRuntime` — creating that resource when the base world authored no
//! script of its own. A layer whose scripts carry an ERROR finding is refused
//! outright ([`LayerLoadOutcome::ParseFailed`]) rather than half-merged: the
//! boot-time `ScriptActivationGate` belongs to base-world activation and
//! must not be tripped by a layer arriving mid-run.
//!
//! ## What the merge does NOT touch
//!
//! The `LoadedWorld.ledger`'s TOML records are dropped here — the applier already
//! recorded the layer's text via `load_scenario_toml`. Its script DIGEST is not:
//! since issue #1241 that write comes back as data instead of being made from
//! inside `load_world_scripts`, so it rides out on
//! [`LayerLoadResult::ledger`] for the applier to apply at the same moment the
//! eager write used to happen. Whether a layer that arrives *after* `freeze()`
//! should move the frozen content set at all is a ledger-policy question owned by
//! issue #1047, not something this route decides — and it cannot, because the
//! frozen snapshot is what the digest folds. No shipped layer authors a script, so
//! the frozen digest of every shipped world is untouched either way.
//!
//! # Purity boundaries
//!
//! * **TOML loading is I/O and stays in the applier.** `load_scenario_toml`
//!   reads the filesystem (native) or the WASM pending-world queue (side
//!   effect), so the applier resolves it first and passes the result in as
//!   `Option<&str>`; this module only decides what the `None` / parse-failure /
//!   success branches mean.
//! * **UUID generation is injected.** Named `[[entity]]` blocks receive UUIDs
//!   from the caller-supplied `uuid_source` (production passes
//!   `entity_loader::assign_uuid`; tests pass a counter).
//! * **Sibling-script reading is injected too.** A layer's top-level
//!   `script = "wave.rhai"` resolves through the caller-supplied
//!   [`ScriptResolver`] (production passes
//!   [`crate::entities::config_cache::production_script_resolver`]; tests pass a
//!   fake or [`NoSiblingScripts`](crate::world::script::load::NoSiblingScripts)),
//!   so this module still touches neither filesystem nor bridge.
//! * **Entities are never held.** Spawned `Entity` handles, `WorldLayerMap`
//!   insertion, comms merge/removal, and despawning stay in the applier.
//! * **Logging becomes data.** Failure paths push onto `warnings`; the applier
//!   logs them.

use crate::world::config::{assign_named_entity_uuids, WorldConfig};
use crate::world::load::{load, LedgerPlan, LoadError, LoadPolicy, LoadRequest, MemoryReader};
use crate::world::script::load::{CompiledScripts, ScriptResolver};

/// The already-active composition facts needed to validate one additive layer
/// before it can mutate the runtime (issue #1046).
///
/// The two sources are injected to preserve the browser's pending-cache
/// authority rule (`absence_is_final = false` while content may still arrive).
/// Anchor names are owned, sorted, and deduplicated because the Bevy caller
/// rebuilds this context for every load from the root plus all currently-active
/// layers; a layer applied earlier in the same drain can therefore satisfy a
/// later layer deterministically.
pub struct LayerValidationContext<'a> {
    pub template_loader: &'a dyn crate::entities::loader::TemplateLoader,
    pub fragment_source: &'a dyn crate::entities::include_resolve::FragmentSource,
    pub declared_anchors: Vec<String>,
}

impl<'a> LayerValidationContext<'a> {
    pub fn new(
        template_loader: &'a dyn crate::entities::loader::TemplateLoader,
        fragment_source: &'a dyn crate::entities::include_resolve::FragmentSource,
        declared_anchors: impl IntoIterator<Item = String>,
    ) -> Self {
        let mut declared_anchors: Vec<String> = declared_anchors.into_iter().collect();
        declared_anchors.sort();
        declared_anchors.dedup();
        Self {
            template_loader,
            fragment_source,
            declared_anchors,
        }
    }
}

/// Decision produced by [`evaluate_layer_load`].
#[derive(Debug)]
pub struct LayerLoadResult {
    pub outcome: LayerLoadOutcome,
    /// Failure-path messages for the applier to log (`error` level).
    pub warnings: Vec<String>,
    /// Content-ledger writes this evaluation gathered, for the applier to apply
    /// (issue #1241) — the compiled script set's digest, and nothing else.
    ///
    /// The load's TOML `records` are deliberately NOT carried: the applier read
    /// that text itself through `load_scenario_toml`, which recorded it, so
    /// passing them on would be a second write of a record it already owns. What
    /// it could not own is the digest, because only the compile knows it.
    ///
    /// Carried on EVERY outcome, not just [`LayerLoadOutcome::Loaded`]. A layer
    /// refused for a script error still compiled its sources, and the eager write
    /// this replaced happened at compile time regardless of what the evaluation
    /// then decided — so a refused layer recorded its digest before, and records
    /// it now.
    pub ledger: crate::world::load::LedgerPlan,
}

/// The branch the applier must take for one `WorldLayerChange::Load`.
#[derive(Debug)]
pub enum LayerLoadOutcome {
    /// Path already present in `WorldLayerMap` — de-duplicate, no-op.
    AlreadyLoaded,
    /// TOML not yet available (WASM fetch in flight) — re-queue the change.
    TomlUnavailable,
    /// The layer is REFUSED — insert an empty `WorldRuntime` entry so the broken
    /// file is not retried. The reason is in `warnings`.
    ///
    /// Named for its first cause and since widened to every refusal this
    /// evaluation can reach, because the applier's response is the same for all
    /// of them: a TOML that will not parse, a root-world-only key a supporting
    /// world may not author (`scenario_detail_floor`), and — since issue #1045 —
    /// a `[script]` block that compiles with an error finding.
    ParseFailed,
    /// Layer is loadable; the applier merges/spawns from this decision.
    ///
    /// Boxed because it is also what
    /// [`WorldLayerChange::DeferredApply`](crate::world::server::WorldLayerChange::DeferredApply)
    /// carries across a tick (issue #1045): a layer whose scripts need a
    /// `WorldScriptRuntime` that does not exist yet is evaluated ONCE and its
    /// decision stashed, never re-evaluated. Re-evaluating would be wrong on
    /// wasm, where reading the TOML CONSUMES it from the pending-fetch queue and
    /// the fetch guard refuses to ask for it twice — a second evaluation would
    /// find nothing and re-queue forever.
    Loaded(Box<LoadedLayer>),
}

/// Everything the applier needs to bring one evaluated layer into the world.
///
/// `Debug` is hand-rolled (not derived) because [`scripts`](Self::scripts) holds
/// a [`CompiledScripts`], which carries Rhai `AST`s and is not `Debug`; it prints
/// as a presence marker, exactly as [`crate::world::load::LoadedWorld`] does.
pub struct LoadedLayer {
    /// Named-entity `name → uuid` registrations for the live runtime's
    /// `name_to_uuid` map (already inserted into `scenario_config`).
    pub name_to_uuid_inserts: Vec<(String, String)>,
    /// Parsed layer config (with `name_to_uuid` filled in) for the impure steps:
    /// entity spawning and the anchor snapshot.
    pub scenario_config: WorldConfig,
    /// Emit `WorldEvent::WorldLoaded` so a base-world `on_world_loaded` handler
    /// can react to this layer arriving (issue #415).
    pub emit_world_loaded: bool,
    /// The layer's compiled `[script]` set, carried out of the `Merge` load
    /// (`None` for the entire shipped set — no shipped layer authors a script).
    /// The applier merges it into the live `WorldScriptRuntime` (issue #1045),
    /// creating that resource if the base world authored no script of its own.
    /// Guaranteed free of ERROR findings: a layer whose scripts do not compile
    /// never reaches this variant.
    pub scripts: Option<CompiledScripts>,
}

impl std::fmt::Debug for LoadedLayer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoadedLayer")
            .field("name_to_uuid_inserts", &self.name_to_uuid_inserts)
            .field("scenario_config", &self.scenario_config)
            .field("emit_world_loaded", &self.emit_world_loaded)
            .field("scripts", &self.scripts.as_ref().map(|_| "<compiled>"))
            .finish()
    }
}

/// Evaluate one `WorldLayerChange::Load` for `path`.
///
/// `already_loaded` is the applier's `WorldLayerMap.contains_key` check (done
/// before TOML resolution so a duplicate load never touches the WASM fetch
/// queue). `toml_str` is `None` when the TOML is not yet available.
/// `script_resolver` reads a top-level `script = "…"` sibling file for the layer
/// (issue #1045) — the same injected seam the boot path uses, so this function
/// stays free of filesystem and bridge access.
pub fn evaluate_layer_load<F>(
    path: &str,
    already_loaded: bool,
    toml_str: Option<&str>,
    script_resolver: &dyn ScriptResolver,
    validation: &LayerValidationContext,
    uuid_source: F,
) -> LayerLoadResult
where
    F: FnMut() -> String,
{
    if already_loaded {
        return LayerLoadResult {
            outcome: LayerLoadOutcome::AlreadyLoaded,
            warnings: Vec::new(),
            ledger: LedgerPlan::default(),
        };
    }
    let Some(toml_str) = toml_str else {
        return LayerLoadResult {
            outcome: LayerLoadOutcome::TomlUnavailable,
            warnings: Vec::new(),
            ledger: LedgerPlan::default(),
        };
    };

    // Route the parse and the script compile through the one world-load sequence
    // ([`load`]) under [`LoadPolicy::Merge`]. A [`MemoryReader`] seeded with the
    // TOML the applier already read — and already recorded into the content ledger
    // via `load_scenario_toml` — keeps this a PURE decision: no filesystem, no
    // bridge. The returned [`LedgerPlan`](crate::world::load::LedgerPlan) is
    // therefore dropped (the applier owns the one recording); a sibling `.rhai`
    // resolves through the INJECTED `script_resolver` for the same reason (issue
    // #1045), so the impurity stays the caller's.
    let reader = MemoryReader::new([(path.to_string(), toml_str.to_string())]);
    let request = LoadRequest::new(path, &reader, script_resolver, LoadPolicy::Merge);
    let loaded = match load(request) {
        Ok(loaded) => loaded,
        Err(LoadError::ParseFailed { message, .. }) => {
            return LayerLoadResult {
                outcome: LayerLoadOutcome::ParseFailed,
                warnings: vec![format!("failed to parse {path}: {message}")],
                ledger: LedgerPlan::default(),
            };
        }
        // Unreachable with a MemoryReader under Merge — the read cannot fail (the
        // text is in hand), the raw re-parse cannot fail once `parse_world`
        // succeeded, and there is no transform or child recursion — but map any
        // load failure to the same broken-file outcome rather than panic.
        Err(other) => {
            return LayerLoadResult {
                outcome: LayerLoadOutcome::ParseFailed,
                warnings: vec![format!("failed to load {path}: {other}")],
                ledger: LedgerPlan::default(),
            };
        }
    };

    let mut scenario_config = loaded.config;
    // The compiled `[script]` set the Merge load carried. Threaded out to the
    // applier on `Loaded`, which merges it into the live runtime (#1045).
    let scripts = loaded.scripts;
    // And the ledger writes it gathered (issue #1241). Only the digests: the
    // applier already recorded this layer's TOML itself — see the field docs.
    let ledger = LedgerPlan {
        records: Vec::new(),
        digests: loaded.ledger.digests,
    };

    // A layer whose scripts carry an ERROR finding is REFUSED whole, entities and
    // all, rather than merged with its logic missing — the same all-or-nothing the
    // boot path applies to a base world, expressed the one way a mid-run load can.
    //
    // It deliberately does NOT trip `ScriptActivationGate`: that per-world gate
    // stops the base world's Startup SPAWN pass, which is long finished by the time
    // a layer arrives, so setting it here would be a global refusal for one broken
    // supporting file. Refusing this layer (and marking it broken so it is not
    // retried) is the whole of the blast radius.
    if let Some(compiled) = &scripts {
        if crate::world::validate::has_error(&compiled.findings) {
            let detail = compiled
                .findings
                .iter()
                .filter(|f| f.is_error())
                .map(|f| format!("[{}] {}", f.category, f.message))
                .collect::<Vec<_>>()
                .join("; ");
            // A missing sibling is the one refusal a correctly-authored layer can
            // still hit, and only in the browser: nothing prefetches a layer's
            // `script = "wave.rhai"` into the wasm config cache, so the resolver
            // reads `None` and the layer is refused whole. Name that explicitly
            // rather than leave a designer reading "file could not be read" about
            // a file that is plainly there in the repo.
            let hint = if compiled
                .findings
                .iter()
                .any(|f| f.is_error() && f.category == "script-file-missing")
            {
                " (a supporting world's sibling .rhai is not prefetched in the \
                 browser — author the layer's script as an inline [script] table)"
            } else {
                ""
            };
            return LayerLoadResult {
                outcome: LayerLoadOutcome::ParseFailed,
                warnings: vec![format!(
                    "failed to load {path}: script error: {detail}{hint}"
                )],
                ledger,
            };
        }
    }

    if !scenario_config.scenario_detail_floor.is_empty() {
        return LayerLoadResult {
            outcome: LayerLoadOutcome::ParseFailed,
            warnings: vec![format!(
                "failed to load {path}: scenario_detail_floor is root-world-only; supporting worlds cannot override the selected scenario's crew detail floor"
            )],
            ledger,
        };
    }

    // Composition-gate the layer before UUID minting or any value can reach the
    // applier. The resolved compiled spawn set is authoritative when present:
    // it includes sibling `.rhai` units and replaces the config's inline-only
    // scan, avoiding duplicate inline findings.
    let composition_findings = {
        let mut source = crate::world::validate::WorldSource::new(path, toml_str, &scenario_config);
        if let Some(compiled) = scripts.as_ref() {
            source = source.with_resolved_script_spawns(&compiled.spawned_templates);
        }
        crate::world::validate::validate_supporting_world(
            &source,
            validation.template_loader,
            validation.fragment_source,
            &validation.declared_anchors,
        )
    };
    if crate::world::validate::has_error(&composition_findings) {
        let detail = composition_findings
            .iter()
            .filter(|finding| finding.is_error())
            .map(|finding| {
                let line = finding
                    .source
                    .line
                    .map(|line| format!(":{line}"))
                    .unwrap_or_default();
                format!(
                    "[{}] {}{}: {}",
                    finding.category, finding.source.file, line, finding.message
                )
            })
            .collect::<Vec<_>>()
            .join("; ");
        return LayerLoadResult {
            outcome: LayerLoadOutcome::ParseFailed,
            warnings: vec![format!(
                "failed to load {path}: composition error: {detail}"
            )],
            ledger,
        };
    }

    // Assign UUIDs to named entities in this layer's config; the registrations go
    // both into the returned config (for spawning) and to the applier (for the
    // live runtime map).
    let new_names = assign_named_entity_uuids(&scenario_config.entities, uuid_source);
    let mut name_to_uuid_inserts: Vec<(String, String)> = new_names.into_iter().collect();
    name_to_uuid_inserts.sort();
    for (name, uuid) in &name_to_uuid_inserts {
        scenario_config
            .name_to_uuid
            .insert(name.clone(), uuid.clone());
    }

    LayerLoadResult {
        outcome: LayerLoadOutcome::Loaded(Box::new(LoadedLayer {
            name_to_uuid_inserts,
            scenario_config,
            emit_world_loaded: true,
            scripts,
        })),
        warnings: Vec::new(),
        ledger,
    }
}

#[cfg(test)]
#[path = "layers_tests.rs"]
mod tests;
