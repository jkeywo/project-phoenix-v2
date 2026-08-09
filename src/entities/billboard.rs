//! Far-LOD billboards: a camera-facing quad textured from a yaw-ring atlas.
//!
//! The farthest band of a model's LOD ladder is a `billboard` level (see
//! [`crate::entity_config::LodLevel::billboard`]): a single quad, always turned
//! to face the camera, textured from an atlas of pre-rendered views of the hull
//! packed left→right around a horizontal yaw ring. At 400+ units a captured
//! silhouette of the real ship reads far better than a coloured sphere, and the
//! PNG is small enough to stand in while the multi-MB near levels stream.
//!
//! Two things move each frame ([`orient_lod_billboards`]):
//!   - **Facing.** The quad's `+Z` is aimed at the camera. It is a CHILD of the
//!     entity (so it tracks the hull's position), and the hull's rotation is
//!     simulation state, so the facing is computed in world space and pushed
//!     back through the parent's inverse rotation into the child's local frame.
//!   - **Tile.** Which of the `views` yaw tiles to show is the angle between the
//!     hull's forward and the direction to the camera, quantised to the ring.
//!     It is applied through `StandardMaterial::uv_transform` — a per-billboard
//!     material instance, no custom shader — so a ship flying away shows its
//!     stern and a broadside shows its flank.
//!
//! The parent entity keeps a UNIFORM scale for a billboard level; the quad's
//! world width/height live on the child's own scale, so rotating it to face the
//! camera never shears it.

use bevy::prelude::*;

use crate::server::renderer::GameCamera;

/// Marks a far-LOD billboard quad and records how many yaw tiles its atlas
/// packs (left→right). Lives on the quad child, not the entity.
#[derive(Component, Debug, Clone, Copy)]
pub struct LodBillboard {
    pub views: u32,
}

/// Spawn the billboard quad child for a level and return it. The caller
/// (`update_mesh_lod`) parents it to the entity and records it as the LOD's
/// scene child. `width`/`height` are world units (from the level's `scale`).
pub fn spawn_billboard_child(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    asset_server: &AssetServer,
    atlas_path: &str,
    width: f32,
    height: f32,
    views: u32,
) -> Entity {
    let quad = meshes.add(Rectangle::new(1.0, 1.0));
    // Asset paths are authored `assets/…`; the loader roots at `assets/`.
    let rel = atlas_path.strip_prefix("assets/").unwrap_or(atlas_path);
    let texture = asset_server.load(rel.to_string());
    let material = materials.add(StandardMaterial {
        base_color_texture: Some(texture),
        // Captured views are already lit; draw them flat so scene lighting does
        // not double-shade a picture of a lit hull. Alpha-blended for the
        // transparent background the capture writes.
        unlit: true,
        alpha_mode: AlphaMode::Blend,
        // Show tile 0 until `orient_lod_billboards` picks the right one this
        // frame: scale U to one tile's width.
        uv_transform: tile_uv_transform(0, views.max(1)),
        ..default()
    });
    commands
        .spawn((
            Mesh3d(quad),
            MeshMaterial3d(material),
            Transform::from_scale(Vec3::new(width, height, 1.0)),
            LodBillboard {
                views: views.max(1),
            },
        ))
        .id()
}

/// The `uv_transform` that shows tile `k` of a 1×`views` horizontal atlas:
/// scale U by `1/views`, offset by `k/views`. Pub so the `tune-lods` tool can
/// pick a billboard's yaw tile directly when it renders a billboard level.
pub fn tile_uv_transform(k: u32, views: u32) -> bevy::math::Affine2 {
    let inv = 1.0 / views as f32;
    bevy::math::Affine2::from_scale_angle_translation(
        Vec2::new(inv, 1.0),
        0.0,
        Vec2::new(k as f32 * inv, 0.0),
    )
}

/// Which yaw tile shows the hull as seen from `view_dir` (world, camera→hull
/// reversed) given the hull's `forward`. Both are flattened to the ring plane.
/// Pure so the quantisation is unit-testable without a camera.
pub fn yaw_tile(forward: Vec3, view_dir: Vec3, views: u32) -> u32 {
    let views = views.max(1);
    let f = Vec3::new(forward.x, 0.0, forward.z);
    let v = Vec3::new(view_dir.x, 0.0, view_dir.z);
    if f.length_squared() < 1e-6 || v.length_squared() < 1e-6 {
        return 0;
    }
    let f = f.normalize();
    let v = v.normalize();
    // Signed angle from forward to view direction, around +Y.
    let angle = f.cross(v).y.atan2(f.dot(v));
    let step = std::f32::consts::TAU / views as f32;
    let k = (angle / step).round() as i32;
    k.rem_euclid(views as i32) as u32
}

/// Face every LOD billboard at the camera and pick its yaw tile. Runs in
/// `Update`; a billboard with no resolvable parent or camera is left as-is.
pub fn orient_lod_billboards(
    cam_q: Query<&GlobalTransform, With<GameCamera>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    parents: Query<&GlobalTransform>,
    mut q: Query<(
        &mut Transform,
        &ChildOf,
        &MeshMaterial3d<StandardMaterial>,
        &LodBillboard,
    )>,
) {
    let Some(cam) = cam_q.iter().next() else {
        return;
    };
    let cam_pos = cam.translation();

    for (mut tf, child_of, mat_handle, bb) in q.iter_mut() {
        let Ok(parent_gt) = parents.get(child_of.parent()) else {
            continue;
        };
        let parent_pos = parent_gt.translation();
        let to_cam = cam_pos - parent_pos;
        if to_cam.length_squared() < 1e-6 {
            continue;
        }

        // World rotation that aims the quad's +Z at the camera, then expressed
        // in the child's local frame by undoing the parent's world rotation.
        let world_rot = Quat::from_rotation_arc(Vec3::Z, to_cam.normalize());
        let parent_rot = parent_gt.rotation();
        tf.rotation = parent_rot.inverse() * world_rot;

        // Tile from the hull's forward (its local -Z in world space) vs. the
        // direction TO the camera.
        let forward = parent_rot * Vec3::NEG_Z;
        let k = yaw_tile(forward, to_cam, bb.views);
        if let Some(mat) = materials.get_mut(&mat_handle.0) {
            mat.uv_transform = tile_uv_transform(k, bb.views);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yaw_tile_front_and_back_are_opposite_on_an_eight_ring() {
        // Camera directly ahead of the hull (view_dir along the hull's forward).
        let fwd = Vec3::NEG_Z;
        let front = yaw_tile(fwd, Vec3::NEG_Z, 8);
        let back = yaw_tile(fwd, Vec3::Z, 8);
        assert_eq!(front, 0);
        assert_eq!(back, 4); // half-way round an 8-tile ring
    }

    #[test]
    fn yaw_tile_wraps_and_never_exceeds_view_count() {
        for deg in (0..360).step_by(7) {
            let a = (deg as f32).to_radians();
            let view = Vec3::new(a.sin(), 0.0, -a.cos());
            assert!(yaw_tile(Vec3::NEG_Z, view, 8) < 8);
        }
    }

    #[test]
    fn degenerate_vectors_are_tile_zero() {
        assert_eq!(yaw_tile(Vec3::ZERO, Vec3::NEG_Z, 8), 0);
        assert_eq!(yaw_tile(Vec3::NEG_Z, Vec3::ZERO, 8), 0);
    }
}
