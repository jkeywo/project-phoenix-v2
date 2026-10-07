//! Host catalogue preparation and live content adapter.
use crate::delivery::{args::ClientSource, http::Request};
use crate::delivery::{stamp::DeliveryStamp, DeliveryManifest};
use crate::world::manifest::{
    build_catalog, build_merged_catalog, parse_manifest, validate_manifest, Manifest,
    MergedCatalog, ScenarioCatalog,
};
pub use phoenix_delivery::serve::*;
use std::path::{Path, PathBuf};
/// One scenario manifest, read off disk, with the world-resolver every
/// catalogue build over it shares.
///
/// The native answer to the two things the browser gets from its JS preload:
/// the manifest TOML (`wasm_push_scenario_manifest`) and a way to resolve each
/// `[[scenario]] world` to its raw text (`wasm_push_world_toml`, read back by
/// `resolved_world_source`). Native has neither — `config_cache::
/// resolved_world_source` is a `None` stub off the browser — so the resolver is
/// a filesystem read rooted at `content_dir`, and it lives HERE rather than
/// being re-typed at each call site: [`load_content`] publishes a catalogue over
/// HTTP and [`Self::merged_catalog`] hands one to the native host's lobby
/// (issue #1326), and the two must never resolve a world differently.
///
/// Reads the session mod-pack overlay (see [`Self::resolve_world`]) and writes
/// nothing process-global of its own.
pub struct ManifestSource {
    /// The content tree every relative path resolves against.
    root: PathBuf,
    /// The manifest's raw text — also this host's content identity, which
    /// [`DeliveryStamp::for_manifest`] is taken over.
    pub toml: String,
    /// The path the manifest was read from, as given (relative to `root`).
    pub manifest_rel: String,
    /// The parsed manifest.
    pub manifest: Manifest,
}

impl ManifestSource {
    /// Read and parse `<content_dir>/<manifest_rel>`.
    pub fn read(content_dir: &str, manifest_rel: &str) -> Result<Self, String> {
        let root = Path::new(content_dir).to_path_buf();
        let manifest_path = root.join(manifest_rel);
        let toml = std::fs::read_to_string(&manifest_path).map_err(|e| {
            format!(
                "cannot read scenario manifest {}: {e}",
                manifest_path.display()
            )
        })?;
        let manifest = parse_manifest(&toml).map_err(|e| {
            format!(
                "scenario manifest {} is malformed: {e}",
                manifest_path.display()
            )
        })?;
        Ok(Self {
            root,
            toml,
            manifest_rel: manifest_rel.to_string(),
            manifest,
        })
    }

    /// The raw text of one manifest-listed world, or `None` when it cannot be
    /// read. The whole of native's `resolve_world`.
    ///
    /// **Overlay first, disk second**, which is not a nicety: it is the order
    /// [`build_merged_catalog`]'s contract requires of its caller, the order the
    /// browser twin already resolves in (`bridge`'s
    /// `config_cache::resolved_world_source`), and the order every other native
    /// content channel uses — `include_resolve::FsFragmentSource`,
    /// `config_cache::OverlayScriptResolver`, `world::load::OverlayFsReader`. A
    /// pack that ADDS a scenario carries that scenario's world only in its own
    /// [`ActivePack::files`](crate::entities::config_cache::ActivePack), never on
    /// disk, and `build_catalog` SKIPS an entry whose world will not resolve — so
    /// a disk-only resolver would accept the pack, log it installed, and then
    /// silently drop the very scenario it was installed for.
    ///
    /// The two lookups share a key: the overlay is keyed by the authored
    /// repo-relative path (`assets/worlds/x.toml`) the manifest already names,
    /// which is exactly what `self.root.join(rel)` is built from.
    pub fn resolve_world(&self, rel: &str) -> Option<String> {
        crate::entities::config_cache::mod_pack_overlay_get(rel)
            .or_else(|| std::fs::read_to_string(self.root.join(rel)).ok())
    }

    /// `validate_manifest`'s findings, flattened to one line each for the
    /// startup summary.
    pub fn findings(&self) -> Vec<String> {
        validate_manifest(&self.manifest, &self.toml, |rel| self.resolve_world(rel))
            .into_iter()
            .map(|f| format!("[{}] {}: {}", f.category, f.source.reference, f.message))
            .collect()
    }

    /// The published catalogue — base manifest only, which is what a delivery
    /// host serves.
    pub fn catalog(&self) -> ScenarioCatalog {
        build_catalog(&self.manifest, |rel| self.resolve_world(rel))
    }

    /// The catalogue a host *offers*: the base manifest merged with every
    /// active mod pack, in load order — the same
    /// [`build_merged_catalog`] call `wasm_get_scenario_catalog` makes
    /// (issue #1326).
    ///
    /// With no pack installed this returns exactly what [`Self::catalog`] does;
    /// going through the merge anyway is what keeps the native lobby's catalogue
    /// the browser's catalogue rather than a second derivation of it. Since issue
    /// #1366 the native stack is no longer always empty — `host_lobby`'s
    /// `apply_mod_pack_choice` pushes onto it — which is why
    /// [`Self::resolve_world`] has to be overlay-aware for this method to mean
    /// anything.
    pub fn merged_catalog(&self) -> MergedCatalog {
        let active = crate::entities::config_cache::active_packs();
        let parsed: Vec<(String, Manifest)> = active
            .iter()
            .filter_map(|p| {
                parse_manifest(&p.manifest_toml)
                    .ok()
                    .map(|m| (p.id.clone(), m))
            })
            .collect();
        let mods: Vec<(&str, &Manifest)> = parsed.iter().map(|(id, m)| (id.as_str(), m)).collect();
        build_merged_catalog(&self.manifest, &mods, |rel| self.resolve_world(rel))
    }

    /// The published document this manifest yields.
    pub fn loaded_content(&self) -> LoadedContent {
        LoadedContent {
            manifest: DeliveryManifest {
                stamp: DeliveryStamp::for_manifest(&self.toml),
                manifest_path: self.manifest_rel.clone(),
                scenarios: crate::delivery::payload::catalog_payload(&self.catalog()),
            },
            findings: self.findings(),
        }
    }
}

/// Read the manifest and its worlds off disk and build the published document.
///
/// Writes no process-global state, so it is safe from a unit test — unlike
/// [`preload_templates`], which is not. It does READ the session mod-pack
/// overlay through [`ManifestSource::resolve_world`], which is empty unless a
/// test installed a pack (and [`crate::entities::config_cache`]'s
/// `overlay_test_guard` is how one asks for that).
pub fn load_content(content_dir: &str, manifest_rel: &str) -> Result<LoadedContent, String> {
    Ok(ManifestSource::read(content_dir, manifest_rel)?.loaded_content())
}

/// Populate the process-global native entity-template cache so the published
/// catalogue carries each hull's class, hull id, power rating and name.
///
/// **Process-global.** Like everything else that calls
/// `config_cache::insert_native_config`, this belongs in a binary or an
/// *integration* test, never in an inline unit test — see that function's docs
/// and AGENTS.md's testing strategy. The catalogue is correct without it; the
/// enrichment fields are simply absent, exactly as they are in the browser
/// before the templates have been fetched.
///
/// Returns how many templates were loaded.
pub fn preload_templates(content_dir: &str) -> Result<usize, String> {
    let dir = Path::new(content_dir).join("assets/entities");
    let mut loaded = 0;
    let mut stack = vec![dir];
    while let Some(current) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&current) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().and_then(|e| e.to_str()) != Some("toml") {
                continue;
            }
            // Key on the repo-relative, forward-slashed path a world names the
            // template with — the same key `headless::app` uses, and the same
            // key `ship_payload` looks up.
            let Ok(rel) = path.strip_prefix(content_dir) else {
                continue;
            };
            let key = rel.to_string_lossy().replace('\\', "/");
            let Ok(resolved) = crate::entities::include_resolve::resolve_from_disk(&key) else {
                continue;
            };
            if let Ok(cfg) = resolved.parse() {
                crate::entities::config_cache::insert_native_config(key, cfg);
                loaded += 1;
            }
        }
    }
    Ok(loaded)
}

struct HostContent {
    content: LoadedContent,
}
impl phoenix_delivery::ContentSource for HostContent {
    fn prepared_catalogue(&self) -> Result<LoadedContent, String> {
        Ok(self.content.clone())
    }
    fn overlay_text(&self, path: &str) -> Option<String> {
        crate::entities::config_cache::mod_pack_overlay_get(path)
    }
    fn overlay_asset(&self, path: &str) -> Option<std::sync::Arc<[u8]>> {
        crate::entities::config_cache::mod_pack_asset(path)
    }
    fn revision(&self) -> u64 {
        crate::entities::config_cache::mod_pack_revision()
    }
}
impl phoenix_delivery::HostConfiguration for super::args::HostArgs {
    fn address(&self) -> &str {
        &self.addr
    }
    fn client(&self) -> &ClientSource {
        &self.client
    }
    fn skip_bundle_check(&self) -> bool {
        self.skip_bundle_check
    }
    fn content_source(
        &self,
    ) -> Result<std::sync::Arc<dyn phoenix_delivery::ContentSource>, String> {
        Ok(std::sync::Arc::new(HostContent {
            content: load_content(&self.content_dir, &self.manifest)?,
        }))
    }
}
/// Compatibility routing adapter with the current accepted-pack revision.
pub fn route(
    req: &Request,
    content: &LoadedContent,
    client: &ClientSource,
    documents: &HostedDocuments,
    peer: PeerOrigin,
) -> Route {
    phoenix_delivery::serve::route(
        req,
        content,
        client,
        documents,
        peer,
        crate::entities::config_cache::mod_pack_revision(),
    )
}
#[cfg(test)]
#[path = "serve_tests.rs"]
mod tests;

use crate::entities::include_resolve::ParseEntityTemplate as _;
