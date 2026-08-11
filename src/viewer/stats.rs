//! What the thing on screen actually costs.
//!
//! Triangles and textures, counted off the loaded assets with
//! [`crate::entities::mesh_stats`] — the same two functions the perf pass uses
//! to build the per-model baselines (issue #905). One counting rule, so a
//! number read here and a number read from `assets-mesh.ron` mean the same
//! thing.
//!
//! This is the half of the LOD panel that says whether a decimation worked.
//! Bytes come from the dev server (it can `stat` the file); triangles and
//! texture dimensions cannot be read off a `.glb` without parsing glTF, so they
//! are read here, from what Bevy loaded.
//!
//! The counting walks every mesh in the world rather than the subject's
//! descendants specifically: the viewer renders exactly one subject, and the
//! only other entities are the camera and its skybox, which own no `Mesh3d`.
//! Descendant-walking would be more precise about a thing that cannot happen
//! and would miss meshes attached at depths the scene chooses.

use std::cell::RefCell;
use std::collections::HashSet;

use bevy::prelude::*;

use crate::entities::mesh_stats::{pixels_in, triangles_in};

use super::camera::OrbitCamera;
use super::lod::{LadderState, LodMode};
use super::subject::SubjectState;

// The last measurement, as the JSON the panel polls. A string rather than a
// channel because the reader is `viewer_stats()`, a `wasm_bindgen` export with
// no access to the `World`.
thread_local! {
    static STATS_JSON: RefCell<String> = const { RefCell::new(String::new()) };
}

/// Geometry and texture totals for the current visual.
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq)]
pub struct SubjectStats {
    pub triangles: u64,
    pub meshes: u32,
    /// Distinct images referenced by the subject's materials.
    pub textures: u32,
    /// Pixels across those images that are still resident in the main world.
    /// Bevy may release an image once it is uploaded, so this can be lower than
    /// the reference count implies; `measured_textures` says how many it read.
    pub texture_pixels: u64,
    pub measured_textures: u32,
    /// The largest single image dimension seen, in pixels.
    pub largest_texture: u32,
}

/// Recount when the subject has settled into something new.
///
/// Gated on `settled` and the `measured` flag rather than run every frame: a
/// hull is hundreds of meshes, and the totals cannot change while the same
/// visual is on screen.
pub fn measure_subject(
    mut stats: ResMut<SubjectStats>,
    mut subject: ResMut<SubjectState>,
    meshes: Res<Assets<Mesh>>,
    images: Res<Assets<Image>>,
    materials: Res<Assets<StandardMaterial>>,
    rendered: Query<(&Mesh3d, Option<&MeshMaterial3d<StandardMaterial>>)>,
) {
    if !subject.settled || subject.measured {
        return;
    }
    subject.measured = true;

    let mut found = SubjectStats::default();
    let mut seen_images: HashSet<AssetId<Image>> = HashSet::new();

    for (mesh, material) in &rendered {
        found.meshes += 1;
        if let Some(mesh) = meshes.get(&mesh.0) {
            found.triangles += triangles_in(mesh);
        }
        let Some(material) = material.and_then(|m| materials.get(&m.0)) else {
            continue;
        };
        for handle in material_images(material) {
            seen_images.insert(handle.id());
        }
    }

    found.textures = seen_images.len() as u32;
    for id in &seen_images {
        let Some(image) = images.get(*id) else {
            continue; // uploaded and released from the main world
        };
        found.measured_textures += 1;
        found.texture_pixels += pixels_in(image);
        found.largest_texture = found.largest_texture.max(image.width()).max(image.height());
    }

    *stats = found;
}

/// Every image a standard material references.
///
/// Listed explicitly. The perf pass deliberately does *not* do this — it asks
/// the asset server which images a given `.glb` produced, which is the loader's
/// own record and cannot drift from it. That route is not open here: the
/// subject is a spawned scene, not a file being measured, and what is on screen
/// is exactly the set of materials these entities reference. The question the
/// panel is asked ("did the texture resize actually shrink anything") is a
/// question about these maps.
fn material_images(material: &StandardMaterial) -> impl Iterator<Item = &Handle<Image>> {
    [
        material.base_color_texture.as_ref(),
        material.emissive_texture.as_ref(),
        material.metallic_roughness_texture.as_ref(),
        material.normal_map_texture.as_ref(),
        material.occlusion_texture.as_ref(),
    ]
    .into_iter()
    .flatten()
}

/// Publish the current numbers for `viewer_stats()` to hand to the panel.
pub fn publish_stats(
    stats: Res<SubjectStats>,
    ladder: Res<LadderState>,
    mode: Res<LodMode>,
    subject: Res<SubjectState>,
    cameras: Query<&OrbitCamera>,
) {
    // The subject's largest world extent. Distance is in absolute world units —
    // the same units `select_lod` compares — so 5 is a close-up of a courier and
    // the inside of a starbase. Reporting the size next to it is what makes the
    // distance readable rather than arbitrary.
    let extent = subject.extents.map(|e| e.max_element()).unwrap_or(0.0);
    let camera = cameras.iter().next().copied();
    let json = render_stats(&stats, &ladder, *mode, subject.settled, extent, camera);
    STATS_JSON.with(|cell| *cell.borrow_mut() = json);
}

/// The panel's payload.
///
/// Hand-written rather than serialised: `serde_json` is confined to `codec.rs`
/// (Key Constraint 1), and every value here is a number or one of this module's
/// own fixed mode names, so there is nothing to escape.
pub fn render_stats(
    stats: &SubjectStats,
    ladder: &LadderState,
    mode: LodMode,
    settled: bool,
    extent: f32,
    camera: Option<OrbitCamera>,
) -> String {
    let level = match ladder.current {
        Some(i) => i.to_string(),
        None => "null".to_string(),
    };
    format!(
        concat!(
            r#"{{"triangles":{},"meshes":{},"textures":{},"measuredTextures":{},"#,
            r#""texturePixels":{},"largestTexture":{},"distance":{:.2},"extent":{:.2},"#,
            r#""mode":"{}","level":{},"levels":{},"settled":{},"camera":{}}}"#
        ),
        stats.triangles,
        stats.meshes,
        stats.textures,
        stats.measured_textures,
        stats.texture_pixels,
        stats.largest_texture,
        ladder.distance,
        extent,
        mode.name(),
        level,
        ladder.levels.len(),
        settled,
        camera.map_or_else(
            || "null".to_string(),
            |camera| format!(
                r#"{{"focus":[{:.4},{:.4},{:.4}],"radius":{:.4},"yaw":{:.4},"pitch":{:.4}}}"#,
                camera.focus.x,
                camera.focus.y,
                camera.focus.z,
                camera.radius,
                camera.yaw,
                camera.pitch,
            ),
        ),
    )
}

/// The last published measurement.
///
/// Its one caller is the `viewer_stats()` export, which is gated on wasm — so
/// this is gated to match, exactly as `push_command` is in the parent module,
/// rather than carrying an `allow(dead_code)` for a native build where it is
/// genuinely unreachable.
#[cfg(target_arch = "wasm32")]
pub fn stats_json() -> String {
    STATS_JSON.with(|cell| cell.borrow().clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_payload_reports_the_level_it_is_showing() {
        let stats = SubjectStats {
            triangles: 138_790,
            meshes: 12,
            textures: 3,
            measured_textures: 3,
            texture_pixels: 786_432,
            largest_texture: 512,
        };
        let ladder = LadderState {
            distance: 87.5,
            current: Some(1),
            levels: vec![Default::default(), Default::default()],
            ..Default::default()
        };
        let json = render_stats(&stats, &ladder, LodMode::Auto, true, 8.0, None);
        assert!(json.contains(r#""triangles":138790"#), "{json}");
        assert!(json.contains(r#""distance":87.50"#), "{json}");
        assert!(json.contains(r#""mode":"auto""#), "{json}");
        assert!(json.contains(r#""level":1"#), "{json}");
        assert!(json.contains(r#""levels":2"#), "{json}");
        assert!(json.contains(r#""settled":true"#), "{json}");
        assert!(json.contains(r#""extent":8.00"#), "{json}");
    }

    /// The base model is "no level", and JSON has a word for that.
    #[test]
    fn no_level_is_null_rather_than_a_number() {
        let json = render_stats(
            &SubjectStats::default(),
            &LadderState::default(),
            LodMode::Base,
            false,
            0.0,
            None,
        );
        assert!(json.contains(r#""level":null"#), "{json}");
    }
}
