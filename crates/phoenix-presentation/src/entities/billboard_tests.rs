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
