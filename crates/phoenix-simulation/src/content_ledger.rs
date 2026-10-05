//! Enumerates the simulation content closure over the shared content ledger.
pub use phoenix_content::ledger::*;

/// Eagerly resolve and record every entity template a world can spawn — its
/// `[[entity]]` roster, available hulls, palette AND the templates its inline
/// scripts name — recursively
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
    for input in capture_world_entity_inputs(world_config, scripts) {
        input.apply();
    }
}

/// Capture the native template/sidecar inputs without changing the ledger.
#[cfg(not(target_arch = "wasm32"))]
pub fn capture_world_entity_inputs(
    world_config: &crate::world::config::WorldConfig,
    scripts: Option<&crate::world::script::load::CompiledScripts>,
) -> Vec<LedgerDigest> {
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
    // The `[[gm_palette]]` templates (issue #1305) join the same walk for the
    // same reason the scripted literals do: a Game Master's spawn resolves an
    // authored literal path mid-mission, so it belongs in the frozen content
    // set and an edit to it must refuse a stale save. Nothing here is computed,
    // so `note_uncovered_spawn` has nothing to say about a palette spawn.
    let palette_paths = world_config
        .gm_palette
        .iter()
        .map(|entry| entry.template_path.clone());
    // Every playable hull is part of the browser's preload and the native
    // template-cache gate, even when this crew selects a different hull. The
    // frozen identity must cover that same declared set before a save is made.
    let available_paths = world_config
        .available_ships
        .iter()
        .map(|ship| ship.template_path.clone());
    let mut queue: Vec<String> = world_config
        .entities
        .iter()
        .map(|e| e.template_path.clone())
        .chain(scripted_paths)
        .chain(palette_paths)
        .chain(available_paths)
        .collect();
    let mut visited: HashSet<String> = HashSet::new();

    let mut inputs = Vec::new();
    while let Some(path) = queue.pop() {
        let key = normalize_key(&path);
        if !visited.insert(key) {
            continue;
        }
        if let Ok(resolved) = crate::entities::include_resolve::resolve_from_disk(&path) {
            inputs.push(LedgerDigest {
                key: resolved.path.clone(),
                digest: vellum_digest::fnv1a(resolved.toml.as_bytes()),
            });
            if let Ok(config) = resolved.parse() {
                if let Some(sidecar) =
                    crate::entities::model_markers::capture_primary_sidecar_from_fs(&config)
                {
                    inputs.push(sidecar);
                }
                queue.extend(crate::entities::config_cache::nested_template_paths(
                    &config,
                ));
            }
        }
    }
    inputs
}

#[cfg(test)]
#[path = "content_ledger_tests.rs"]
mod tests;

#[cfg(not(target_arch = "wasm32"))]
use crate::entities::include_resolve::ParseEntityTemplate as _;
