//! Far-LOD billboards: a camera-facing quad textured from a yaw-ring atlas.
//!
//! The farthest band of a model's LOD ladder is a `billboard` level (see
//! [`crate::entities::config::LodLevel::billboard`]): a camera-facing quad,
//! textured from an atlas of pre-rendered views of the hull packed left→right
//! around a horizontal yaw ring. At 400+ units a captured silhouette of the real
//! ship reads far better than a coloured sphere, and the PNG is small enough to
//! stand in while the multi-MB near levels stream.
//!
//! A billboard is a small hierarchy, not one entity: the ROOT carries
//! [`LodBillboard`], the quad's world width/height, and the facing rotation;
//! under it hang TWO coplanar pose quads ([`BillboardPose`]), each with its own
//! material instance. Two, because the ring is discrete — every shipped atlas
//! packs eight views, so a hull crossing a tile boundary used to JUMP 45° of
//! apparent yaw in one frame. The pair lets the ring be sampled between its
//! poses instead of snapped to one.
//!
//! Three things move each frame ([`orient_lod_billboards`]):
//!   - **Facing.** The root's `+Z` is aimed at the camera. It is a CHILD of the
//!     entity (so it tracks the hull's position), and the hull's rotation is
//!     simulation state, so the facing is computed in world space and pushed
//!     back through the parent's inverse rotation into the root's local frame.
//!   - **Tiles.** Which two of the `views` yaw tiles bracket the current view
//!     angle ([`yaw_blend`]) — applied through `StandardMaterial::uv_transform`,
//!     one tile per pose quad, so a ship flying away shows its stern and a
//!     broadside shows its flank.
//!   - **Weights.** How far between those two tiles the view actually is, as the
//!     two quads' alphas. Both quads carry the atlas's own `AlphaMode::Blend`,
//!     so the pair reads as a cross-dissolve between adjacent poses rather than
//!     a snap to the nearest.
//!
//! The parent entity keeps a UNIFORM scale for a billboard level; the quad's
//! world width/height live on the root's own scale, so rotating it to face the
//! camera never shears it.

// Presentation-only: billboards face the camera and pick their yaw tile at render
// time, never in the deterministic sim, so platform-varying std transcendentals
// are fine here (issue #908, simmath.rs; same opt-out as src/viewer/camera.rs).
#![allow(clippy::disallowed_methods)]

use bevy::prelude::*;

use crate::entities::config::LodLevel;
use crate::entities::visual_fade::{SelfDrivenAlpha, VisualFade};

/// Marks a far-LOD billboard's ROOT and records how many yaw tiles its atlas
/// packs (left→right). Lives on the root, not the entity, and not on the pose
/// quads that hang beneath it.
#[derive(Component, Debug, Clone, Copy)]
pub struct LodBillboard {
    pub views: u32,
}

/// One of a billboard's two pose quads. Slot `0` shows the nearer yaw tile of
/// the bracketing pair, slot `1` the next one round the ring; the blend weight
/// between them is written to their alphas each frame.
#[derive(Component, Debug, Clone, Copy)]
pub struct BillboardPose {
    pub slot: u8,
}

/// How far behind slot 0 the second pose quad sits, in the root's local frame
/// (world units — the root's scale carries width and height, and leaves `z` at
/// 1). Two exactly coplanar alpha-blended quads have no defined draw order, and
/// the dissolve only reads correctly with the outgoing pose composited OVER the
/// incoming one. A separation this small is invisible at the 400+ units a
/// billboard band starts at, and only has to break the tie.
const POSE_QUAD_SEPARATION: f32 = 0.001;

/// The world width and height of a billboard level's quad.
///
/// The authored `scale` on a billboard level IS the quad's world size, on both
/// ladder conventions, because `scripts/capture-billboards.mjs` records it that
/// way on both. It writes what the `capture-billboard` tool measured, and that
/// tool renders the model under the primary sidecar's `[base]` rig and reads the
/// composed `GlobalTransform` Aabb — so a hull's number already carries its
/// `[base].scale`. An asteroid is the one case that needs arithmetic, and the
/// script does it there: rocks ship no `<stem>.model.toml`, so the tool renders
/// them at identity and the script multiplies each variant's quad by that
/// variant's own `[base].scale` before writing it. Either way the number on
/// disk is world units, and this function's whole job is to not touch it.
///
/// So `tier_parent_scale` — which the GLB tiers genuinely need, because THEY
/// resolve their own absent sidecars to an identity rig — must NOT be folded in
/// on top. bf4c4b02 folded it, on the stated claim that a hull atlas is captured
/// off RAW model extents; it is not, and the result was that every hull ladder's
/// far billboard rendered its `[base].scale` times too large. That is invisible
/// on the hulls authored at scale 1 and worth 1.5x–4x on the rest, and on
/// `alliance_starbase` — `[base].scale` `[15, 18, 18]`, against an atlas whose
/// size was still the one captured back when the model was `[5, 6, 6]` — it
/// came to 6x: a 204-unit-wide imposter for a 34-unit-wide station. That is the
/// starbase John saw drawn huge at distance and blinking down to its true size
/// the moment the approach crossed inside the 400-unit billboard band.
///
/// Width rides the horizontal (z) axis, height the vertical (y). A level that
/// authors no size states no world size, and falls back to the tier scale
/// itself — what the renderer has always done for an unsized billboard.
///
/// Lives here, next to the spawn it feeds, because the game's LOD swap and the
/// standalone model viewer both need the answer and a second copy of this rule
/// in the viewer is exactly how the viewer came to show a size the game did not
/// (see [`crate::entities::glb_visual::tier_parent_scale`]).
pub fn billboard_quad_size(level_scale: Option<[f32; 3]>, tier_parent_scale: Vec3) -> [f32; 2] {
    let bs = tier_parent_scale;
    level_scale.map(|s| [s[0], s[1]]).unwrap_or([bs.z, bs.y])
}

/// How many yaw tiles a billboard level's atlas packs.
///
/// Recorded by the capture tool in `[lod.capture] yaw_views`. A level that
/// records none is a single-view atlas: one tile, no ring, no blending.
pub fn billboard_yaw_views(level: &LodLevel) -> u32 {
    level
        .capture
        .as_ref()
        .and_then(|c| c.yaw_views)
        .unwrap_or(1)
        .max(1)
}

/// Spawn the billboard root (with its two pose quads) for a level and return it.
/// The caller (`update_mesh_lod`, or the viewer's subject builder) parents it to
/// the entity and records it as the LOD's scene child. `width`/`height` are
/// world units — see [`billboard_quad_size`].
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
    let views = views.max(1);

    let root = commands
        .spawn((
            Transform::from_scale(Vec3::new(width, height, 1.0)),
            Visibility::default(),
            LodBillboard { views },
        ))
        .id();

    for slot in 0..2u8 {
        let material = materials.add(StandardMaterial {
            base_color_texture: Some(texture.clone()),
            // Slot 0 carries the whole billboard until `orient_lod_billboards`
            // picks this frame's pair, so a billboard is never invisible on the
            // frame it appears.
            base_color: Color::srgba(1.0, 1.0, 1.0, if slot == 0 { 1.0 } else { 0.0 }),
            // Captured views are already lit; draw them flat so scene lighting
            // does not double-shade a picture of a lit hull. Alpha-blended for
            // the transparent background the capture writes — and for the
            // pose-to-pose dissolve, which rides the same channel.
            unlit: true,
            alpha_mode: AlphaMode::Blend,
            // Show tile 0 until this frame's pair is picked: scale U to one
            // tile's width.
            uv_transform: tile_uv_transform(0, views),
            ..default()
        });
        let pose = commands
            .spawn((
                Mesh3d(quad.clone()),
                MeshMaterial3d(material),
                Transform::from_xyz(0.0, 0.0, -POSE_QUAD_SEPARATION * slot as f32),
                BillboardPose { slot },
                // This system is the only writer of a pose quad's alpha; a
                // cross-fade over the billboard reaches it through `fade` below
                // rather than through the fade driver's own material walk.
                SelfDrivenAlpha,
            ))
            .id();
        commands.entity(root).add_child(pose);
    }
    root
}

/// The `uv_transform` that shows tile `k` of a 1×`views` horizontal atlas:
/// scale U by `1/views`, offset by `k/views`.
fn tile_uv_transform(k: u32, views: u32) -> bevy::math::Affine2 {
    let inv = 1.0 / views as f32;
    bevy::math::Affine2::from_scale_angle_translation(
        Vec2::new(inv, 1.0),
        0.0,
        Vec2::new(k as f32 * inv, 0.0),
    )
}

/// Which two yaw tiles bracket the hull as seen from `view_dir` (world,
/// camera→hull reversed) given the hull's `forward`, and how far between them
/// the view actually is.
///
/// Returns `(near, next, blend)` where `blend` is in `[0, 1)`: at `0` the view
/// is exactly `near`'s captured pose, and as it approaches `1` the view
/// approaches `next`'s. Both are flattened to the ring plane. Pure, so the
/// blend is unit-testable without a camera.
///
/// The ring is coarse — every shipped atlas packs eight views, 45° apart — so
/// which of these two tiles is *nearest* is not enough information to draw with:
/// picking one and snapping to it is a 45° jump in apparent yaw, taken in a
/// single frame, every time a hull drifts across a tile boundary. The caller
/// draws BOTH and dissolves between them by `blend`.
pub fn yaw_blend(forward: Vec3, view_dir: Vec3, views: u32) -> (u32, u32, f32) {
    let views = views.max(1);
    let f = Vec3::new(forward.x, 0.0, forward.z);
    let v = Vec3::new(view_dir.x, 0.0, view_dir.z);
    if f.length_squared() < 1e-6 || v.length_squared() < 1e-6 {
        return (0, 0, 0.0);
    }
    let f = f.normalize();
    let v = v.normalize();
    // Signed angle from forward to view direction, around +Y, as a continuous
    // position on the ring measured in tiles.
    let angle = f.cross(v).y.atan2(f.dot(v));
    let step = std::f32::consts::TAU / views as f32;
    let position = angle / step;
    let near = position.floor();
    let blend = (position - near).clamp(0.0, 1.0);
    let near = near as i32;
    let wrap = |k: i32| k.rem_euclid(views as i32) as u32;
    (wrap(near), wrap(near + 1), blend)
}

/// Face every LOD billboard at the camera, pick the two yaw tiles bracketing the
/// view, and weight them. Runs in `Update`; a billboard with no resolvable
/// parent or camera is left as-is.
///
/// Generic over the camera marker so the game (`GameCamera`), the standalone
/// model viewer (`ViewerCamera`) and the offscreen capture tools drive
/// billboards through this one system rather than each growing its own copy of
/// the facing and pose rules. That the viewer could NOT do so — it had no
/// billboard to orient at all — is the tooling gap that let billboard pose
/// snapping ship unreviewed.
pub fn orient_lod_billboards<C: Component>(
    cam_q: Query<&GlobalTransform, With<C>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    parents: Query<&GlobalTransform>,
    children: Query<&Children>,
    fades: Query<&VisualFade>,
    mut roots: Query<(Entity, &mut Transform, &ChildOf, &LodBillboard)>,
    poses: Query<(&BillboardPose, &MeshMaterial3d<StandardMaterial>)>,
) {
    let Some(cam) = cam_q.iter().next() else {
        return;
    };
    let cam_pos = cam.translation();

    for (root, mut tf, child_of, bb) in roots.iter_mut() {
        let Ok(parent_gt) = parents.get(child_of.parent()) else {
            continue;
        };
        let parent_pos = parent_gt.translation();
        let to_cam = cam_pos - parent_pos;
        if to_cam.length_squared() < 1e-6 {
            continue;
        }

        // World rotation that aims the quad's +Z at the camera, then expressed
        // in the root's local frame by undoing the parent's world rotation.
        let world_rot = Quat::from_rotation_arc(Vec3::Z, to_cam.normalize());
        let parent_rot = parent_gt.rotation();
        tf.rotation = parent_rot.inverse() * world_rot;

        // Tiles from the hull's forward (its local -Z in world space) vs. the
        // direction TO the camera.
        let forward = parent_rot * Vec3::NEG_Z;
        let (near, next, blend) = yaw_blend(forward, to_cam, bb.views);
        // A billboard mid-cross-fade owns one alpha channel between the two
        // jobs, so the pose weights are scaled by the fade rather than fought
        // over. The quads carry `SelfDrivenAlpha` for the same reason: the fade
        // driver skips them, and this is the one writer.
        let fade = fades.get(root).map(|f| f.alpha()).unwrap_or(1.0);

        let Ok(kids) = children.get(root) else {
            continue;
        };
        for kid in kids.iter() {
            let Ok((pose, mat_handle)) = poses.get(kid) else {
                continue;
            };
            let (tile, weight) = if pose.slot == 0 {
                (near, 1.0 - blend)
            } else {
                (next, blend)
            };
            if let Some(mat) = materials.get_mut(&mat_handle.0) {
                mat.uv_transform = tile_uv_transform(tile, bb.views);
                mat.base_color = mat.base_color.with_alpha(weight * fade);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A view direction `deg` degrees round the ring from the hull's forward.
    /// The ring's positive sense is the one `yaw_blend` measures in, so a
    /// growing `deg` walks the tiles upward instead of wrapping backwards.
    fn view_at(deg: f32) -> Vec3 {
        let a = deg.to_radians();
        Vec3::new(-a.sin(), 0.0, -a.cos())
    }

    /// Looking straight down a captured pose's own axis is that pose, at full
    /// weight — blending must not smear a view that IS one of the captures.
    #[test]
    fn a_view_on_a_captured_pose_is_that_pose_alone() {
        let (near, _, blend) = yaw_blend(Vec3::NEG_Z, Vec3::NEG_Z, 8);
        assert_eq!(near, 0);
        assert!(
            blend < 1e-4,
            "expected no blend on an exact pose, got {blend}"
        );
    }

    /// Front and back of an eight-view ring are four tiles apart, and both are
    /// exact captures — the invariant the single-tile quantiser used to hold.
    #[test]
    fn front_and_back_are_opposite_captures_on_an_eight_ring() {
        let (front, _, front_blend) = yaw_blend(Vec3::NEG_Z, Vec3::NEG_Z, 8);
        let (back, _, back_blend) = yaw_blend(Vec3::NEG_Z, Vec3::Z, 8);
        assert_eq!(front, 0);
        assert_eq!(back, 4, "half-way round an 8-tile ring");
        assert!(front_blend < 1e-3 && back_blend < 1e-3);
    }

    /// The whole point: a view between two captures is a mix of them, not a
    /// snap to whichever is closer. Half a tile round an 8-view ring is 22.5°.
    #[test]
    fn a_view_between_two_poses_splits_between_them() {
        let (near, next, blend) = yaw_blend(Vec3::NEG_Z, view_at(360.0 / 8.0 / 2.0), 8);
        assert_eq!((near, next), (0, 1));
        assert!(
            (blend - 0.5).abs() < 1e-3,
            "half-way between two tiles must weight them evenly, got {blend}"
        );
    }

    /// A quarter of the way into a tile weights the pair a quarter/three
    /// quarters — the blend is a position, not a switch that happens to have
    /// three states.
    #[test]
    fn the_weight_tracks_how_far_into_the_tile_the_view_is() {
        let quarter = 360.0 / 8.0 / 4.0;
        let (near, next, blend) = yaw_blend(Vec3::NEG_Z, view_at(quarter), 8);
        assert_eq!((near, next), (0, 1));
        assert!((blend - 0.25).abs() < 1e-3, "got {blend}");
    }

    /// The pair always brackets the view, always wraps, and always stays inside
    /// the ring — swept round every degree, so no angle is left where the
    /// billboard fades out, doubles up, or indexes past its atlas.
    #[test]
    fn the_bracketing_pair_is_adjacent_and_in_range_all_the_way_round() {
        for deg in 0..360 {
            let view = view_at(deg as f32);
            let (near, next, blend) = yaw_blend(Vec3::NEG_Z, view, 8);
            assert!(near < 8 && next < 8, "tiles must stay inside the ring");
            assert_eq!(next, (near + 1) % 8, "the pair must be adjacent at {deg}°");
            assert!(
                (0.0..=1.0).contains(&blend),
                "weight must stay in range at {deg}°, got {blend}"
            );
        }
    }

    /// Walking the ring one degree at a time never jumps a tile: the pose the
    /// eye is looking at moves continuously, which is precisely what hard
    /// quantisation could not do.
    #[test]
    fn the_blended_pose_never_jumps_a_tile_between_adjacent_angles() {
        // The blended position on the ring, as a continuous tile coordinate.
        let position = |deg: f32| {
            let (near, _, blend) = yaw_blend(Vec3::NEG_Z, view_at(deg), 8);
            near as f32 + blend
        };
        for deg in 0..359 {
            let a = position(deg as f32);
            let b = position(deg as f32 + 1.0);
            // One degree is 1/45th of a tile; the only legitimate large step is
            // the wrap from just under 8 back to 0.
            let step = (b - a).abs();
            let wrapped = step > 7.0;
            assert!(
                step < 0.1 || wrapped,
                "the pose jumped {step} tiles between {deg}° and {}°",
                deg + 1
            );
        }
    }

    /// A single-view atlas has no ring to blend across: both slots are tile 0,
    /// so the pair cannot dissolve a pose against itself at partial alpha.
    #[test]
    fn a_single_view_atlas_never_blends() {
        for deg in (0..360).step_by(11) {
            let (near, next, _) = yaw_blend(Vec3::NEG_Z, view_at(deg as f32), 1);
            assert_eq!((near, next), (0, 0));
        }
    }

    #[test]
    fn degenerate_vectors_are_tile_zero() {
        assert_eq!(yaw_blend(Vec3::ZERO, Vec3::NEG_Z, 8), (0, 0, 0.0));
        assert_eq!(yaw_blend(Vec3::NEG_Z, Vec3::ZERO, 8), (0, 0, 0.0));
    }

    // ── Quad sizing ──────────────────────────────────────────────────────

    /// A HULL ladder's billboard is its authored size, NOT that size times the
    /// model's `[base].scale`: the capture tool rendered the model under its own
    /// `[base]` rig, so the recorded number is already world units. Folding the
    /// base scale in here is what drew `alliance_starbase` — scale `[15,18,18]` —
    /// as a 204-unit imposter for a 34-unit station.
    #[test]
    fn a_hull_billboard_is_its_authored_world_size() {
        let got = billboard_quad_size(Some([4.0, 2.0, 1.0]), Vec3::new(15.0, 18.0, 18.0));
        assert_eq!(
            got,
            [4.0, 2.0],
            "a hull billboard's authored scale is already world units"
        );
    }

    /// A PIPELINE ladder records its extents already in world units too, so the
    /// two conventions agree here and the authored size is the size. This is the
    /// case that was always right; the hull case now matches it.
    #[test]
    fn a_pipeline_billboard_is_its_authored_world_size_too() {
        assert_eq!(
            billboard_quad_size(Some([3.0, 5.0, 1.0]), Vec3::ONE),
            [3.0, 5.0]
        );
    }

    /// The rule is one rule: the same authored size answers regardless of what
    /// the ladder's GLB tiers need from their parent. A billboard that changed
    /// size with the tier scale is a billboard that changes size across the LOD
    /// crossing it exists to hide.
    #[test]
    fn a_billboards_size_does_not_depend_on_the_tier_scale() {
        let authored = Some([7.5, 2.25, 1.0]);
        for tier in [
            Vec3::ONE,
            Vec3::new(15.0, 18.0, 18.0),
            Vec3::splat(0.4),
            Vec3::splat(12.675_623),
        ] {
            assert_eq!(
                billboard_quad_size(authored, tier),
                [7.5, 2.25],
                "tier scale {tier:?} must not move the quad"
            );
        }
    }

    /// A level with no authored size falls back to the tier scale itself, which
    /// is what the renderer has always done for an unsized billboard.
    #[test]
    fn an_unsized_billboard_falls_back_to_the_tier_scale() {
        assert_eq!(
            billboard_quad_size(None, Vec3::new(2.0, 3.0, 4.0)),
            [4.0, 3.0]
        );
    }

    /// A level with no `[lod.capture]` block is a one-tile atlas, and a
    /// recorded `0` is not a usable ring either.
    #[test]
    fn a_level_with_no_capture_block_is_a_single_view_atlas() {
        assert_eq!(billboard_yaw_views(&LodLevel::default()), 1);
        let zero = LodLevel {
            capture: Some(crate::entities::config::LodCapture {
                yaw_views: Some(0),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert_eq!(billboard_yaw_views(&zero), 1);
    }

    /// Every shipped atlas records eight views — the ring the blend exists for.
    #[test]
    fn the_shipped_capture_block_reads_its_ring() {
        let level = LodLevel {
            capture: Some(crate::entities::config::LodCapture {
                yaw_views: Some(8),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert_eq!(billboard_yaw_views(&level), 8);
    }
}
