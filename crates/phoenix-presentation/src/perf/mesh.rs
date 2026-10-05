//! The mesh interior, read through Bevy's own loader (issue #905).
//!
//! [`assets`](super::assets) measures the *outside* of a `.glb` — how many
//! bytes a player downloads — because that is knowable from `stat`. This module
//! measures the *inside*: how many triangles and how much texture the engine
//! actually uploads once the file is open.
//!
//! **Why a Bevy app rather than a glTF parser.** Reading the counts out of the
//! binary with the `gltf` crate would be quicker and would need no App, and it
//! was rejected on purpose (issue #868 recorded the reasoning, #905 acts on
//! it): a parallel reader is a *second opinion* about a file whose first
//! opinion is the only one that ships. Bevy's loader is what decides how many
//! primitives become how many `Mesh` assets, what a sparse or strip-topology
//! accessor turns into, and which of a glTF's images are materialised at all.
//! A number that disagrees with the engine is worse than no number, so the
//! measurement runs the loader and reads what it produced. The repository
//! therefore has no direct `gltf` dependency, and should not gain one.
//!
//! What is measured, and why it is only this:
//!
//! - `assets.mesh.triangles` — one sample per runtime-reachable GLB level, its
//!   whole triangle count. `max` finds the level that dominates a draw.
//! - `assets.mesh.triangles.total` — one sample: the deduplicated first (near)
//!   GLB level of each runtime model. Mutually-exclusive lower levels do not
//!   inflate this population budget.
//! - `assets.texture.count` — one sample per `.glb`: how many distinct images
//!   the loader produced for it.
//! - `assets.texture.pixels` — one sample per loaded image, width × height.
//!   `max` finds the one 4K sheet paying for a model nobody looks at closely.
//!
//! Texture *bytes* are deliberately not here: what a GPU allocates depends on
//! the format it is uploaded in, which depends on the compressed formats the
//! device supports, and this pass runs with none (there is no GPU). Pixels are
//! the honest thing a headless pass can say.
//!
//! **Attribution is the asset server's, not ours.** Every mesh and image the
//! loader creates is a labelled sub-asset of the file it came from, so
//! `AssetServer::get_path` says which `.glb` produced it. That is why this
//! module does not walk `StandardMaterial`'s fifteen optional texture slots:
//! enumerating them would re-implement, and eventually disagree with, the
//! loader's own record of what it made.
//!
//! This is a separate scenario from [`assets`](super::assets) rather than more
//! metrics on the same capture, because provenance is the contract that stops
//! two captures being compared when they should not be. An asset capture
//! records `static-inventory` — nothing ran. This one records `bevy-loader`,
//! because something did, and a Bevy release that changes how a primitive is
//! decoded moves these numbers without a single asset changing.
//!
//! Native-only. There is no wasm build of a measurement pass: the browser host
//! loads models to *render* them, and a page that also counted them would be
//! measuring the thing it is part of.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use bevy::app::TaskPoolPlugin;
use bevy::asset::{AssetApp, AssetPlugin, AssetServer, Assets, LoadState};
use bevy::gltf::{Gltf, GltfPlugin};
use bevy::image::Image;
use bevy::mesh::Mesh;
use bevy::pbr::StandardMaterial;
use bevy::prelude::*;
use bevy::scene::ScenePlugin;
use bevy::shader::{Shader, ShaderLoader};

use vellum_perf::{Capture, Profile, Recorder, Unit};

/// One sample per `.glb`: its whole triangle count.
pub const TRIANGLES_METRIC: &str = "assets.mesh.triangles";
/// One sample: triangles across the deduplicated first/near runtime levels.
pub const TRIANGLES_TOTAL_METRIC: &str = "assets.mesh.triangles.total";
/// One sample per `.glb`: how many distinct images the loader produced.
pub const TEXTURE_COUNT_METRIC: &str = "assets.texture.count";
/// One sample per loaded image: width × height.
pub const TEXTURE_PIXELS_METRIC: &str = "assets.texture.pixels";

/// The scenario mesh-interior captures are filed under.
pub const SCENARIO: &str = "assets-mesh";
/// The runtime this capture records: Bevy's asset loader ran, nothing else did.
pub const RUNTIME: &str = "bevy-loader";

/// How long one model may take to load before the pass gives up on it.
/// Generous because the largest shipped model is seventeen megabytes and every
/// embedded texture is decoded on the way in; a pass that timed out under load
/// would report a *smaller* asset set, which is the most misleading possible
/// failure.
const LOAD_DEADLINE: Duration = Duration::from_secs(300);

/// How many updates the app is pumped after a model is harvested, to let Bevy
/// process the dropped handle and free what that model owned.
const RELEASE_UPDATES: usize = 4;

/// What one model's interior turned out to be.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ModelInterior {
    /// Triangles across every `Mesh` the loader produced for this file.
    pub triangles: u64,
    /// Width × height of each distinct `Image` the loader produced, ascending
    /// so the sample order — and therefore the capture bytes — is stable.
    pub texture_pixels: Vec<u64>,
}

/// Every shipped model's interior, before it becomes a capture.
///
/// Kept as data so the capture shape is testable without loading a hundred
/// megabytes of GLB.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Interior {
    /// Asset-server-relative GLB path → what the loader made of it.
    pub models: BTreeMap<String, ModelInterior>,
    /// Runtime model paths whose first/near level contributes to the aggregate
    /// triangle population. Shared levels are deliberately deduplicated.
    pub base_models: BTreeSet<String>,
}

#[derive(Debug)]
pub enum MeasureError {
    Io(String, std::io::Error),
    /// A model the loader refused. Reported rather than skipped: a model that
    /// failed to load would otherwise contribute zero triangles and read as an
    /// optimisation.
    Failed(String),
    /// A model still in flight when [`LOAD_DEADLINE`] passed.
    TimedOut(String),
}

impl std::fmt::Display for MeasureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MeasureError::Io(path, e) => write!(f, "could not read {path:?}: {e}"),
            MeasureError::Failed(model) => write!(f, "Bevy's loader rejected {model}"),
            MeasureError::TimedOut(model) => write!(
                f,
                "{model} was still loading after {}s",
                LOAD_DEADLINE.as_secs(),
            ),
        }
    }
}

// The counting itself moved to `entities::mesh_stats` when the model viewer
// grew a triangle/texture readout (it runs in wasm, and this module is native
// and `--features perf`). Re-exported rather than reimplemented: a budget and
// the panel a person tunes against it must not be able to disagree about what
// a triangle is.
pub use crate::entities::mesh_stats::{pixels_in, triangles_in};

/// Discover the GLBs reachable from top-level entity templates and load them
/// through Bevy.
///
/// Builds a Bevy app with the asset server, the glTF loader and nothing else —
/// no window, no renderer, no simulation. The app is driven by hand for the
/// same reason [`crate::headless::app::run`] drives its own: with no
/// `WinitPlugin` and no `ScheduleRunnerPlugin`, `App::run` would update once
/// and return before a single asset finished loading.
///
/// **One model at a time, on purpose.** Loading all of them at once would be
/// faster and would hold every decoded texture in memory simultaneously: the
/// shipped set is over 150 MB of GLB, and a 4K texture costs sixty-seven
/// megabytes once it is RGBA rather than PNG. A measurement pass that a
/// CI runner can only sometimes afford is not a measurement pass. Loading
/// serially bounds the peak at one model and makes a failure name the file
/// that caused it.
pub fn measure(root: &Path) -> Result<Interior, MeasureError> {
    let reachable = reachable_models(root)?;

    // Absolute, because Bevy resolves a relative asset root against the
    // executable's directory (or `CARGO_MANIFEST_DIR`), neither of which is
    // the `--root` this was asked to measure.
    let assets_dir = absolute(&root.join("assets"))?;

    let mut app = App::new();
    app.add_plugins((
        TaskPoolPlugin::default(),
        AssetPlugin {
            file_path: assets_dir.to_string_lossy().into_owned(),
            ..default()
        },
        ScenePlugin,
    ));
    // The asset types the glTF loader produces. `RenderPlugin` would register
    // these; there is no `RenderPlugin` here, and an unregistered type is a
    // load failure rather than a missing statistic.
    app.init_asset::<Shader>()
        .init_asset_loader::<ShaderLoader>()
        .init_asset::<Mesh>()
        .init_asset::<Image>()
        .init_asset::<StandardMaterial>()
        .add_plugins(GltfPlugin::default());
    app.finish();
    app.cleanup();

    let mut found = Interior {
        base_models: reachable.base_models,
        ..default()
    };
    for name in &reachable.models {
        let handle: Handle<Gltf> = app.world().resource::<AssetServer>().load(name.clone());
        wait_for(&mut app, name, &handle)?;
        // Every requested model gets an entry even if the loader produced
        // nothing for it, so a model that quietly stopped contributing
        // geometry shows up as a zero rather than as an absence.
        found.models.insert(name.clone(), collect_one(&app, name));

        // Release what this model owned before the next one is opened: the
        // `Gltf` holds the handles to its own meshes and images, so dropping
        // the root drops them, and Bevy frees them on the following updates.
        drop(handle);
        for _ in 0..RELEASE_UPDATES {
            app.update();
        }
    }

    Ok(found)
}

/// Pump the app until one model has settled, or the deadline passes.
fn wait_for(app: &mut App, name: &str, handle: &Handle<Gltf>) -> Result<(), MeasureError> {
    let started = Instant::now();
    loop {
        app.update();

        {
            let server = app.world().resource::<AssetServer>();
            // Recursive, because a `Gltf` reports itself loaded before the
            // meshes and images labelled under it are in their collections —
            // and those are the whole measurement.
            match server.recursive_dependency_load_state(handle) {
                bevy::asset::RecursiveDependencyLoadState::Loaded => return Ok(()),
                bevy::asset::RecursiveDependencyLoadState::Failed(_) => {
                    return Err(MeasureError::Failed(name.to_string()))
                }
                // A root that failed on its own account, which the recursive
                // state reports as `Failed` too — kept as a belt-and-braces
                // read of the direct state so a loader that ever distinguishes
                // the two cannot spin here.
                _ => {
                    if let LoadState::Failed(_) = server.load_state(handle) {
                        return Err(MeasureError::Failed(name.to_string()));
                    }
                }
            }
        }

        if started.elapsed() > LOAD_DEADLINE {
            return Err(MeasureError::TimedOut(name.to_string()));
        }
        // Loading happens on the IO task pool; this loop only harvests the
        // results, so yielding beats spinning a core to no purpose.
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// Attribute the loaded meshes and images belonging to `name` back to it.
///
/// The asset server's own path record is the attribution, not a walk of the
/// material graph — see the module documentation. Filtering by file name (and
/// not by "everything currently loaded") is what makes the serial pass above
/// safe: an asset a previous model has not finished releasing cannot be
/// counted against this one.
fn collect_one(app: &App, name: &str) -> ModelInterior {
    let world = app.world();
    let server = world.resource::<AssetServer>();
    let meshes = world.resource::<Assets<Mesh>>();
    let images = world.resource::<Assets<Image>>();

    let owns = |id: bevy::asset::UntypedAssetId| -> bool {
        server
            .get_path(id)
            .map(|path| path.path() == Path::new(name))
            .unwrap_or(false)
    };

    let mut interior = ModelInterior::default();
    for (id, mesh) in meshes.iter() {
        if owns(id.into()) {
            interior.triangles += triangles_in(mesh);
        }
    }
    // One entry per stored `Image`, not per material slot: the loader makes
    // one asset per glTF image, however many slots reach it, and that asset is
    // what a GPU is handed once. Two textures that happen to be the same size
    // are two textures, so this counts rather than de-duplicates.
    for (id, image) in images.iter() {
        if owns(id.into()) {
            interior.texture_pixels.push(pixels_in(image));
        }
    }
    // Ascending, because `Assets::iter` has no defined order and the sample
    // order is capture bytes.
    interior.texture_pixels.sort_unstable();
    interior
}

#[derive(Debug, Default, PartialEq, Eq)]
struct ReachableModels {
    models: BTreeSet<String>,
    base_models: BTreeSet<String>,
}

/// Filesystem source rooted at the repository selected by `--root`.
///
/// Production's native source reads the same canonical asset paths relative to
/// the process working directory. The perf command permits another repository
/// root, so it supplies that root at this I/O seam while retaining the exact
/// runtime include resolver and final `EntityConfig` parser.
struct RootedFragmentSource<'a> {
    root: &'a Path,
}

impl crate::entities::include_resolve::FragmentSource for RootedFragmentSource<'_> {
    fn read(&self, path: &str) -> Option<String> {
        std::fs::read_to_string(self.root.join(path)).ok()
    }

    fn absence_is_final(&self) -> bool {
        true
    }
}

/// Follow the runtime route: a top-level entity's model+variant selects one
/// sidecar, and only that sidecar's `[[lod]]` list supplies visual levels.
fn reachable_models(root: &Path) -> Result<ReachableModels, MeasureError> {
    let entities = root.join("assets/entities");
    let mut paths: Vec<PathBuf> = std::fs::read_dir(&entities)
        .map_err(|e| MeasureError::Io(entities.display().to_string(), e))?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().and_then(|e| e.to_str()) == Some("toml"))
        .collect();
    paths.sort();

    let mut found = ReachableModels::default();
    let source = RootedFragmentSource { root };
    for path in paths {
        let template_path = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        let config = crate::entities::include_resolve::resolve_template(&template_path, &source)
            .and_then(|resolved| resolved.parse())
            .map_err(|error| {
                MeasureError::Failed(format!("entity template {template_path}: {error}"))
            })?;
        let Some(mesh) = config.mesh else {
            continue;
        };
        let Some(flat_model) = mesh.model.filter(|model| model.ends_with(".glb")) else {
            continue;
        };
        let variant = mesh.variant;
        let flat_model = asset_server_model_path(&flat_model);
        let sidecar = crate::entities::model_rig::sidecar_path(
            &format!("assets/{flat_model}"),
            variant.as_deref(),
        );
        let rig = std::fs::read_to_string(root.join(&sidecar))
            .ok()
            .and_then(|text| crate::entities::model_rig::parse_model_rig(&text).ok());

        let Some(rig) = rig.filter(|rig| !rig.lod.is_empty()) else {
            if !is_remesh_model(&flat_model) {
                found.models.insert(flat_model.clone());
                found.base_models.insert(flat_model);
            }
            continue;
        };

        for (index, level) in rig.lod.iter().enumerate() {
            let Some(model) = level.model.as_deref() else {
                continue;
            };
            let model = asset_server_model_path(model);
            if is_remesh_model(&model) {
                continue;
            }
            found.models.insert(model.clone());
            if index == 0 {
                found.base_models.insert(model);
            }
        }
    }
    Ok(found)
}

fn asset_server_model_path(model: &str) -> String {
    let normalized = model.replace('\\', "/");
    normalized
        .strip_prefix("assets/")
        .unwrap_or(&normalized)
        .to_string()
}

fn is_remesh_model(model: &str) -> bool {
    model.ends_with(".remesh.glb")
}

fn absolute(path: &Path) -> Result<PathBuf, MeasureError> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    let cwd =
        std::env::current_dir().map_err(|e| MeasureError::Io(path.display().to_string(), e))?;
    Ok(cwd.join(path))
}

/// Turn a measured interior into a capture.
pub fn capture(found: &Interior, profile: Profile) -> Capture {
    let mut recorder = Recorder::new();
    let mut total = 0u64;
    for (name, interior) in &found.models {
        recorder.sample(TRIANGLES_METRIC, Unit::Count, interior.triangles as f64);
        if found.base_models.contains(name) {
            total += interior.triangles;
        }
        recorder.sample(
            TEXTURE_COUNT_METRIC,
            Unit::Count,
            interior.texture_pixels.len() as f64,
        );
        for pixels in &interior.texture_pixels {
            recorder.sample(TEXTURE_PIXELS_METRIC, Unit::Count, *pixels as f64);
        }
    }
    recorder.sample(TRIANGLES_TOTAL_METRIC, Unit::Count, total as f64);
    recorder.finish(SCENARIO, profile)
}

#[cfg(test)]
#[path = "mesh_tests.rs"]
mod tests;

use crate::entities::include_resolve::ParseEntityTemplate as _;
