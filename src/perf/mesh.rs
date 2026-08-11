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

impl crate::entity_includes::FragmentSource for RootedFragmentSource<'_> {
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
        let config = crate::entity_includes::resolve_template(&template_path, &source)
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
        let sidecar =
            crate::model_rig::sidecar_path(&format!("assets/{flat_model}"), variant.as_deref());
        let rig = std::fs::read_to_string(root.join(&sidecar))
            .ok()
            .and_then(|text| crate::model_rig::ModelRig::from_toml(&text).ok());

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
mod tests {
    use super::*;
    use crate::perf::profile;
    use bevy::asset::RenderAssetUsages;
    use bevy::mesh::{Indices, PrimitiveTopology};

    fn indexed(topology: PrimitiveTopology, indices: Vec<u32>, vertices: usize) -> Mesh {
        let mut mesh = Mesh::new(topology, RenderAssetUsages::default());
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0f32, 0.0, 0.0]; vertices]);
        mesh.insert_indices(Indices::U32(indices));
        mesh
    }

    #[test]
    fn an_indexed_triangle_list_is_counted_from_its_indices() {
        let mesh = indexed(PrimitiveTopology::TriangleList, vec![0, 1, 2, 0, 2, 3], 4);
        assert_eq!(triangles_in(&mesh), 2);
    }

    /// Unindexed geometry has three vertices per triangle and no index buffer
    /// to count, which is the case a naive `indices().len()` reads as zero.
    #[test]
    fn an_unindexed_triangle_list_is_counted_from_its_vertices() {
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        );
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0f32, 0.0, 0.0]; 9]);
        assert_eq!(triangles_in(&mesh), 3);
    }

    #[test]
    fn a_triangle_strip_draws_two_fewer_triangles_than_it_has_vertices() {
        let mesh = indexed(PrimitiveTopology::TriangleStrip, vec![0, 1, 2, 3, 4], 5);
        assert_eq!(triangles_in(&mesh), 3);
    }

    /// A degenerate strip must not underflow into a colossal count.
    #[test]
    fn a_strip_too_short_to_draw_anything_counts_nothing() {
        let mesh = indexed(PrimitiveTopology::TriangleStrip, vec![0], 1);
        assert_eq!(triangles_in(&mesh), 0);
    }

    #[test]
    fn a_line_list_contributes_no_triangles() {
        let mesh = indexed(PrimitiveTopology::LineList, vec![0, 1, 2, 3], 4);
        assert_eq!(triangles_in(&mesh), 0);
    }

    #[test]
    fn the_capture_totals_triangles_and_keeps_textures_per_model() {
        let mut found = Interior::default();
        found.models.insert(
            "big.glb".into(),
            ModelInterior {
                triangles: 900,
                texture_pixels: vec![1024 * 1024, 4096 * 4096],
            },
        );
        found.models.insert(
            "small.glb".into(),
            ModelInterior {
                triangles: 100,
                texture_pixels: vec![256 * 256],
            },
        );
        found.base_models.insert("big.glb".into());

        let capture = capture(&found, profile(RUNTIME));
        assert_eq!(capture.scenario, SCENARIO);
        assert_eq!(capture.summaries[TRIANGLES_TOTAL_METRIC].summary.max, 900.0);
        assert_eq!(capture.summaries[TRIANGLES_METRIC].summary.count, 2);
        assert_eq!(capture.summaries[TRIANGLES_METRIC].summary.max, 900.0);
        // Three images across two models, largest 4K square.
        assert_eq!(capture.summaries[TEXTURE_PIXELS_METRIC].summary.count, 3);
        assert_eq!(
            capture.summaries[TEXTURE_PIXELS_METRIC].summary.max,
            (4096.0 * 4096.0)
        );
        assert_eq!(capture.summaries[TEXTURE_COUNT_METRIC].summary.max, 2.0);
    }

    /// A model the loader produced nothing for still reports, as a zero.
    #[test]
    fn a_model_with_no_geometry_is_a_zero_not_an_absence() {
        let mut found = Interior::default();
        found
            .models
            .insert("empty.glb".into(), ModelInterior::default());
        let capture = capture(&found, profile(RUNTIME));
        assert_eq!(capture.summaries[TRIANGLES_METRIC].summary.count, 1);
        assert_eq!(capture.summaries[TRIANGLES_METRIC].summary.max, 0.0);
        assert_eq!(capture.summaries[TEXTURE_COUNT_METRIC].summary.max, 0.0);
    }

    #[test]
    fn a_missing_model_directory_is_an_error_not_an_empty_pass() {
        assert!(measure(Path::new("no/such/root")).is_err());
    }

    /// A 2×2 RGBA PNG, assembled here rather than committed, so the fixture
    /// below is one file of readable code instead of an opaque binary nobody
    /// can review.
    const PNG_2X2: &[u8] = &[
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x02, 0x08, 0x06, 0x00, 0x00, 0x00, 0x72,
        0xb6, 0x0d, 0x24, 0x00, 0x00, 0x00, 0x11, 0x49, 0x44, 0x41, 0x54, 0x78, 0xda, 0x63, 0xf8,
        0xcf, 0xc0, 0xf0, 0x1f, 0x84, 0x19, 0x60, 0x0c, 0x00, 0x47, 0xca, 0x07, 0xf9, 0x1a, 0xb6,
        0xf1, 0xa9, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
    ];

    /// One indexed triangle with UVs and one 2×2 texture, as a glTF-binary
    /// container.
    ///
    /// Generated rather than committed because the point of the fixture is
    /// what it *contains* — one triangle, four texels — and a checked-in
    /// binary hides exactly that. Written by hand rather than with the `gltf`
    /// crate for the reason the module documentation gives: this repository
    /// does not carry a second glTF implementation, not even to make a test
    /// fixture.
    fn one_triangle_glb() -> Vec<u8> {
        // Positions (3 × vec3), UVs (3 × vec2), indices (3 × u16), then the
        // PNG. Every view starts 4-byte aligned, which glTF requires of
        // accessor data.
        let mut bin: Vec<u8> = Vec::new();
        for position in [[0.0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]] {
            for value in position {
                bin.extend_from_slice(&value.to_le_bytes());
            }
        }
        for uv in [[0.0f32, 0.0], [1.0, 0.0], [0.0, 1.0]] {
            for value in uv {
                bin.extend_from_slice(&value.to_le_bytes());
            }
        }
        for index in [0u16, 1, 2] {
            bin.extend_from_slice(&index.to_le_bytes());
        }
        while !bin.len().is_multiple_of(4) {
            bin.push(0);
        }
        let image_offset = bin.len();
        bin.extend_from_slice(PNG_2X2);
        while !bin.len().is_multiple_of(4) {
            bin.push(0);
        }

        let json = format!(
            r#"{{"asset":{{"version":"2.0"}},
"buffers":[{{"byteLength":{total}}}],
"bufferViews":[
 {{"buffer":0,"byteOffset":0,"byteLength":36}},
 {{"buffer":0,"byteOffset":36,"byteLength":24}},
 {{"buffer":0,"byteOffset":60,"byteLength":6}},
 {{"buffer":0,"byteOffset":{image_offset},"byteLength":{image_len}}}],
"accessors":[
 {{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0.0,0.0,0.0],"max":[1.0,1.0,0.0]}},
 {{"bufferView":1,"componentType":5126,"count":3,"type":"VEC2"}},
 {{"bufferView":2,"componentType":5123,"count":3,"type":"SCALAR"}}],
"images":[{{"bufferView":3,"mimeType":"image/png"}}],
"samplers":[{{}}],
"textures":[{{"sampler":0,"source":0}}],
"materials":[{{"pbrMetallicRoughness":{{"baseColorTexture":{{"index":0}}}}}}],
"meshes":[{{"primitives":[{{"attributes":{{"POSITION":0,"TEXCOORD_0":1}},"indices":2,"material":0}}]}}],
"nodes":[{{"mesh":0}}],
"scenes":[{{"nodes":[0]}}],
"scene":0}}"#,
            total = bin.len(),
            image_offset = image_offset,
            image_len = PNG_2X2.len(),
        );
        let mut json = json.into_bytes();
        // Chunks are 4-byte aligned; JSON pads with spaces, BIN with zeroes.
        while json.len() % 4 != 0 {
            json.push(b' ');
        }

        let mut glb: Vec<u8> = Vec::new();
        glb.extend_from_slice(b"glTF");
        glb.extend_from_slice(&2u32.to_le_bytes());
        glb.extend_from_slice(&((12 + 8 + json.len() + 8 + bin.len()) as u32).to_le_bytes());
        glb.extend_from_slice(&(json.len() as u32).to_le_bytes());
        glb.extend_from_slice(b"JSON");
        glb.extend_from_slice(&json);
        glb.extend_from_slice(&(bin.len() as u32).to_le_bytes());
        glb.extend_from_slice(b"BIN\0");
        glb.extend_from_slice(&bin);
        glb
    }

    /// Adler-32 of `data`, the checksum a zlib stream trails with. There is no
    /// CRC-32 pairing this repo already carries for zlib's own algorithm, so
    /// it is written out here rather than borrowed.
    fn adler32(data: &[u8]) -> u32 {
        let mut a: u32 = 1;
        let mut b: u32 = 0;
        for &byte in data {
            a = (a + u32::from(byte)) % 65521;
            b = (b + a) % 65521;
        }
        (b << 16) | a
    }

    /// Append one length-prefixed, CRC-suffixed PNG chunk to `out`.
    fn write_png_chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let mut tagged = Vec::with_capacity(4 + data.len());
        tagged.extend_from_slice(kind);
        tagged.extend_from_slice(data);
        // `crc32` is the same IEEE polynomial the store-only ZIP reader in
        // `world::mod_pack` already carries; PNG uses the identical
        // algorithm, so this borrows it rather than writing a second copy.
        out.extend_from_slice(&tagged);
        out.extend_from_slice(&crate::world::mod_pack::crc32(&tagged).to_be_bytes());
    }

    /// A `width`×`height` opaque RGBA PNG, built by hand like [`PNG_2X2`]
    /// above but sized to order.
    ///
    /// The pixel data is stored uncompressed (deflate's "stored block" type,
    /// `BTYPE=00`) rather than actually compressed: a real encoder is not the
    /// point, a decodable file of a chosen size is. One stored block is
    /// enough for every size this module's tests ask for, all well under the
    /// 65,535-byte block limit.
    fn png_of(width: u32, height: u32) -> Vec<u8> {
        let mut raw = Vec::new();
        for _ in 0..height {
            raw.push(0u8); // filter type: None
            let row = [0xff, 0x00, 0x00, 0xff].repeat(width as usize); // opaque red texels
            raw.extend_from_slice(&row);
        }
        assert!(
            raw.len() <= 0xffff,
            "png_of only writes one stored deflate block"
        );

        let mut zlib = vec![0x78, 0x01]; // zlib header: deflate, no preset dictionary
        zlib.push(0x01); // BFINAL=1, BTYPE=00 (stored)
        let len = raw.len() as u16;
        zlib.extend_from_slice(&len.to_le_bytes());
        zlib.extend_from_slice(&(!len).to_le_bytes()); // NLEN, the one's complement of LEN
        zlib.extend_from_slice(&raw);
        zlib.extend_from_slice(&adler32(&raw).to_be_bytes());

        let mut png = vec![0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&width.to_be_bytes());
        ihdr.extend_from_slice(&height.to_be_bytes());
        ihdr.extend_from_slice(&[8, 6, 0, 0, 0]); // 8-bit depth, RGBA, defaults
        write_png_chunk(&mut png, b"IHDR", &ihdr);
        write_png_chunk(&mut png, b"IDAT", &zlib);
        write_png_chunk(&mut png, b"IEND", &[]);
        png
    }

    /// Two indexed triangles (a quad) with a 4×4 texture — deliberately
    /// different from [`one_triangle_glb`] in both triangle count and texture
    /// size, so a test that loads both alongside each other can tell
    /// contamination from coincidence: a bleed would show up as either
    /// model's numbers picking up the other's.
    fn two_triangle_glb() -> Vec<u8> {
        let texture = png_of(4, 4);

        let mut bin: Vec<u8> = Vec::new();
        for position in [
            [0.0f32, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        ] {
            for value in position {
                bin.extend_from_slice(&value.to_le_bytes());
            }
        }
        for uv in [[0.0f32, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]] {
            for value in uv {
                bin.extend_from_slice(&value.to_le_bytes());
            }
        }
        // Same index pattern as `an_indexed_triangle_list_is_counted_from_its_indices`
        // above: two triangles sharing an edge across a four-vertex quad.
        for index in [0u16, 1, 2, 0, 2, 3] {
            bin.extend_from_slice(&index.to_le_bytes());
        }
        while !bin.len().is_multiple_of(4) {
            bin.push(0);
        }
        let image_offset = bin.len();
        bin.extend_from_slice(&texture);
        while !bin.len().is_multiple_of(4) {
            bin.push(0);
        }

        let json = format!(
            r#"{{"asset":{{"version":"2.0"}},
"buffers":[{{"byteLength":{total}}}],
"bufferViews":[
 {{"buffer":0,"byteOffset":0,"byteLength":48}},
 {{"buffer":0,"byteOffset":48,"byteLength":32}},
 {{"buffer":0,"byteOffset":80,"byteLength":12}},
 {{"buffer":0,"byteOffset":{image_offset},"byteLength":{image_len}}}],
"accessors":[
 {{"bufferView":0,"componentType":5126,"count":4,"type":"VEC3","min":[0.0,0.0,0.0],"max":[1.0,1.0,0.0]}},
 {{"bufferView":1,"componentType":5126,"count":4,"type":"VEC2"}},
 {{"bufferView":2,"componentType":5123,"count":6,"type":"SCALAR"}}],
"images":[{{"bufferView":3,"mimeType":"image/png"}}],
"samplers":[{{}}],
"textures":[{{"sampler":0,"source":0}}],
"materials":[{{"pbrMetallicRoughness":{{"baseColorTexture":{{"index":0}}}}}}],
"meshes":[{{"primitives":[{{"attributes":{{"POSITION":0,"TEXCOORD_0":1}},"indices":2,"material":0}}]}}],
"nodes":[{{"mesh":0}}],
"scenes":[{{"nodes":[0]}}],
"scene":0}}"#,
            total = bin.len(),
            image_offset = image_offset,
            image_len = texture.len(),
        );
        let mut json = json.into_bytes();
        while json.len() % 4 != 0 {
            json.push(b' ');
        }

        let mut glb: Vec<u8> = Vec::new();
        glb.extend_from_slice(b"glTF");
        glb.extend_from_slice(&2u32.to_le_bytes());
        glb.extend_from_slice(&((12 + 8 + json.len() + 8 + bin.len()) as u32).to_le_bytes());
        glb.extend_from_slice(&(json.len() as u32).to_le_bytes());
        glb.extend_from_slice(b"JSON");
        glb.extend_from_slice(&json);
        glb.extend_from_slice(&(bin.len() as u32).to_le_bytes());
        glb.extend_from_slice(b"BIN\0");
        glb.extend_from_slice(&bin);
        glb
    }

    /// The whole measurement, through the loader that ships.
    ///
    /// This is the test the module exists for: everything above asserts on
    /// pure functions, and a pure function cannot tell you whether Bevy
    /// attributes a labelled sub-asset back to the file it came from. One
    /// triangle in, one triangle out; one 2×2 image in, four pixels out.
    #[test]
    fn the_loader_reports_the_triangles_and_textures_a_glb_actually_contains() {
        let root =
            std::env::temp_dir().join(format!("phoenix_perf_mesh_fixture_{}", std::process::id()));
        let models = root.join("assets/models");
        let entities = root.join("assets/entities");
        std::fs::create_dir_all(&models).expect("the fixture tree is creatable");
        std::fs::create_dir_all(&entities).expect("the fixture tree is creatable");
        std::fs::write(models.join("one_triangle.glb"), one_triangle_glb())
            .expect("the fixture is writable");
        std::fs::write(
            entities.join("triangle.toml"),
            "[mesh]\nmodel = \"assets/models/one_triangle.glb\"\nshape = \"sphere\"\ncolour = []\n",
        )
        .expect("the fixture is writable");

        let found = measure(&root).expect("Bevy loads the fixture");

        let interior = &found.models["models/one_triangle.glb"];
        assert_eq!(interior.triangles, 1);
        assert_eq!(interior.texture_pixels, vec![4]);

        let capture = capture(&found, profile(RUNTIME));
        assert_eq!(capture.scenario, SCENARIO);
        assert_eq!(capture.summaries[TRIANGLES_TOTAL_METRIC].summary.max, 1.0);
        assert_eq!(capture.summaries[TEXTURE_COUNT_METRIC].summary.max, 1.0);
        assert_eq!(capture.summaries[TEXTURE_PIXELS_METRIC].summary.max, 4.0);

        std::fs::remove_dir_all(&root).ok();
    }

    /// Two distinct models measured in the same serial pass must not bleed
    /// into each other's counts.
    ///
    /// The single-fixture test above proves the loader attributes a mesh and
    /// an image back to *a* file; it cannot prove attribution survives a
    /// second model going through the same app, which is the actual claim
    /// [`measure`]'s "one model at a time" documentation makes. `one_triangle`
    /// and `two_triangle` are deliberately unequal on both axes this module
    /// measures — one triangle against two, four texels against sixteen — so
    /// a bleed in either direction (a stray triangle, a picked-up texture)
    /// changes a number this test checks, rather than hiding behind two
    /// fixtures that happened to agree.
    #[test]
    fn two_models_in_one_pass_do_not_contaminate_each_others_counts() {
        let root = std::env::temp_dir().join(format!(
            "phoenix_perf_mesh_fixture_two_{}",
            std::process::id()
        ));
        let models = root.join("assets/models");
        let entities = root.join("assets/entities");
        std::fs::create_dir_all(&models).expect("the fixture tree is creatable");
        std::fs::create_dir_all(&entities).expect("the fixture tree is creatable");
        std::fs::write(models.join("one_triangle.glb"), one_triangle_glb())
            .expect("the fixture is writable");
        std::fs::write(models.join("two_triangle.glb"), two_triangle_glb())
            .expect("the fixture is writable");
        std::fs::write(
            entities.join("one.toml"),
            "[mesh]\nmodel = \"assets/models/one_triangle.glb\"\nshape = \"sphere\"\ncolour = []\n",
        )
        .expect("the fixture is writable");
        std::fs::write(
            entities.join("two.toml"),
            "[mesh]\nmodel = \"assets/models/two_triangle.glb\"\nshape = \"sphere\"\ncolour = []\n",
        )
        .expect("the fixture is writable");

        let found = measure(&root).expect("Bevy loads both fixtures");

        let one = &found.models["models/one_triangle.glb"];
        assert_eq!(
            one.triangles, 1,
            "one_triangle.glb picked up a triangle that isn't its own"
        );
        assert_eq!(
            one.texture_pixels,
            vec![4],
            "one_triangle.glb picked up a texture that isn't its own"
        );

        let two = &found.models["models/two_triangle.glb"];
        assert_eq!(
            two.triangles, 2,
            "two_triangle.glb picked up a triangle that isn't its own"
        );
        assert_eq!(
            two.texture_pixels,
            vec![16],
            "two_triangle.glb picked up a texture that isn't its own"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    /// The real tree, because a pass that has never met the shipped models is
    /// a pass that budgets nothing. Names only — this asserts on what the
    /// entity/sidecar graph names, not on what a loader would make of them, so
    /// it costs no GLB decode.
    #[test]
    fn the_repository_ships_models_for_this_pass_to_measure() {
        let found = reachable_models(Path::new(".")).expect("the asset graph is readable");
        assert!(!found.models.is_empty(), "no runtime GLBs are reachable");
        assert!(
            !found.base_models.is_empty(),
            "no first/base GLBs are reachable"
        );
        assert!(found.base_models.is_subset(&found.models));
        assert!(found.models.iter().all(|model| !is_remesh_model(model)));
    }

    #[test]
    fn discovery_follows_only_top_level_sidecars_and_deduplicates_base_levels() {
        let root = std::env::temp_dir().join(format!(
            "phoenix_perf_mesh_graph_fixture_{}",
            std::process::id()
        ));
        let models = root.join("assets/models");
        let entities = root.join("assets/entities");
        std::fs::create_dir_all(&models).expect("fixture models directory");
        std::fs::create_dir_all(&entities).expect("fixture entities directory");

        for entity in ["one", "two"] {
            std::fs::write(
                entities.join(format!("{entity}.toml")),
                "[mesh]\nmodel = \"assets/models/ship.glb\"\nvariant = \"large\"\nshape = \"sphere\"\ncolour = []\n",
            )
            .expect("entity fixture");
        }
        std::fs::write(
            models.join("ship.large.toml"),
            "[[lod]]\nmodel = \"assets/models/ship.glb\"\n\n[[lod]]\nmodel = \"assets/models/ship_lod1.glb\"\n",
        )
        .expect("sidecar fixture");
        // If discovery incorrectly follows a level's sidecar recursively, this
        // unrelated file would become reachable.
        std::fs::write(
            models.join("ship_lod1.model.toml"),
            "[[lod]]\nmodel = \"assets/models/not_runtime_reachable.glb\"\n",
        )
        .expect("nested sidecar fixture");
        std::fs::write(
            entities.join("broken.toml"),
            "[mesh]\nmodel = \"assets/models/fallback.glb\"\nshape = \"sphere\"\ncolour = []\n",
        )
        .expect("fallback entity fixture");
        std::fs::write(models.join("fallback.model.toml"), "[[lod]\n")
            .expect("malformed sidecar fixture");

        let found = reachable_models(&root).expect("fixture graph is readable");
        assert_eq!(
            found.models,
            BTreeSet::from([
                "models/fallback.glb".to_string(),
                "models/ship.glb".to_string(),
                "models/ship_lod1.glb".to_string(),
            ])
        );
        assert_eq!(
            found.base_models,
            BTreeSet::from([
                "models/fallback.glb".to_string(),
                "models/ship.glb".to_string(),
            ])
        );

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn discovery_reads_a_mesh_inherited_entirely_from_an_include() {
        let root = std::env::temp_dir().join(format!(
            "phoenix_perf_mesh_include_fixture_{}",
            std::process::id()
        ));
        let entities = root.join("assets/entities");
        let fragments = entities.join("fragments");
        std::fs::create_dir_all(&fragments).expect("fixture fragments directory");
        std::fs::write(
            entities.join("composed.toml"),
            "includes = [\"fragments/visual.toml\"]\n",
        )
        .expect("composed entity fixture");
        std::fs::write(
            fragments.join("visual.toml"),
            "[mesh]\nmodel = \"assets/models/inherited.glb\"\nshape = \"sphere\"\ncolour = []\n",
        )
        .expect("visual fragment fixture");

        let found = reachable_models(&root).expect("included mesh resolves");
        assert_eq!(
            found.models,
            BTreeSet::from(["models/inherited.glb".to_string()])
        );
        assert_eq!(found.base_models, found.models);

        std::fs::remove_dir_all(&root).ok();
    }
}
