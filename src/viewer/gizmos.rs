//! Rig overlay: draws what a `.model.toml` sidecar claims, on top of the model
//! it claims it about.
//!
//! Markers are authored in post-base-rig space, which is the space the
//! `SceneRoot` child lives in — so the gizmos hang off the subject's transform
//! and land wherever the sidecar says, making an authoring mistake visible
//! rather than silently wrong at runtime.

use bevy::prelude::*;

use super::subject::{Subject, SubjectState};
use super::ViewerArgs;
use crate::model_rig::ModelMarkers;

/// Length of a marker's axis cross and its direction arrow, as a fraction of
/// the model's largest extent — so the overlay stays legible on any hull size.
const MARKER_SCALE: f32 = 0.04;

pub fn draw_rig_gizmos(
    args: Res<ViewerArgs>,
    state: Res<SubjectState>,
    mut gizmos: Gizmos,
    subjects: Query<(&Transform, Option<&ModelMarkers>), With<Subject>>,
) {
    if !args.gizmos {
        return;
    }
    let Ok((transform, markers)) = subjects.single() else {
        return;
    };

    let size = state.extents.unwrap_or(Vec3::splat(10.0));
    let tick = (size.max_element() * MARKER_SCALE).max(0.05);

    // Extents box: the bounds the sidecar caches, drawn in the same space.
    gizmos.cube(
        Transform::from_translation(transform.translation).with_scale(size),
        Color::srgba(0.3, 0.8, 1.0, 0.35),
    );

    // World axes at the origin, so base-rig rotation errors are obvious.
    gizmos.axes(
        Transform::from_translation(transform.translation),
        tick * 3.0,
    );

    let Some(markers) = markers else {
        return;
    };

    for name in markers.marker_names() {
        let Some(marker) = markers.get(name) else {
            continue;
        };
        let position = transform.transform_point(Vec3::from_array(marker.position));
        let direction = transform
            .rotation
            .mul_vec3(Vec3::from_array(marker.direction))
            .normalize_or_zero();

        // Cross marking the mount point…
        gizmos.line(
            position - Vec3::X * tick,
            position + Vec3::X * tick,
            Color::srgb(1.0, 0.4, 0.4),
        );
        gizmos.line(
            position - Vec3::Y * tick,
            position + Vec3::Y * tick,
            Color::srgb(0.4, 1.0, 0.4),
        );
        gizmos.line(
            position - Vec3::Z * tick,
            position + Vec3::Z * tick,
            Color::srgb(0.4, 0.4, 1.0),
        );
        // …and an arrow along the direction it fires/emits.
        if direction != Vec3::ZERO {
            gizmos.arrow(
                position,
                position + direction * tick * 3.0,
                Color::srgb(1.0, 0.9, 0.3),
            );
        }
    }

    // Target points: where incoming phaser beams may land.
    for index in 0..markers.target_point_count() {
        let Some(point) = markers.target_point(index) else {
            continue;
        };
        let position = transform.transform_point(Vec3::from_array(point.position));
        gizmos.sphere(
            Isometry3d::from_translation(position),
            tick * 0.6,
            Color::srgb(1.0, 0.3, 0.8),
        );
    }
}
