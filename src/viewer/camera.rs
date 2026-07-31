//! Orbit camera for the viewer.
//!
//! Left-drag orbits, right-drag pans, wheel dollies. The camera frames the
//! subject once from its rig extents, so a 12 m courier and a 400 m starbase
//! both arrive on screen at a usable size without per-model fiddling.

// Dev-tool camera, never part of the shipped simulation: platform-varying std
// transcendentals are fine here (issue #908, simmath.rs).
#![allow(clippy::disallowed_methods)]

use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll};
use bevy::prelude::*;

use super::subject::SubjectState;

/// Spherical orbit state around a focus point.
#[derive(Component)]
pub struct OrbitCamera {
    pub focus: Vec3,
    pub radius: f32,
    /// Radians around Y.
    pub yaw: f32,
    /// Radians of elevation, clamped just shy of the poles to avoid gimbal flip.
    pub pitch: f32,
    /// Set once the subject's size is known, so framing does not fight the user.
    pub framed: bool,
}

impl Default for OrbitCamera {
    fn default() -> Self {
        Self {
            focus: Vec3::ZERO,
            radius: 30.0,
            yaw: 0.6,
            pitch: 0.35,
            framed: false,
        }
    }
}

const PITCH_LIMIT: f32 = std::f32::consts::FRAC_PI_2 - 0.01;
const ORBIT_SENSITIVITY: f32 = 0.005;
const PAN_SENSITIVITY: f32 = 0.0015;
const ZOOM_SENSITIVITY: f32 = 0.12;

/// Frame the subject the first time its extents become known.
///
/// Distance comes from the largest extent and the camera's vertical FOV, with a
/// margin so the model does not touch the screen edge.
pub fn frame_subject_once(
    state: Res<SubjectState>,
    mut cameras: Query<(&mut OrbitCamera, &Projection)>,
) {
    let Some(size) = state.extents else {
        return;
    };
    for (mut orbit, projection) in &mut cameras {
        if orbit.framed {
            continue;
        }
        let largest = size.max_element().max(0.1);
        let fov = match projection {
            Projection::Perspective(p) => p.fov,
            _ => std::f32::consts::FRAC_PI_4,
        };
        orbit.radius = (largest * 0.5) / (fov * 0.5).tan() * 1.8;
        orbit.focus = Vec3::ZERO;
        orbit.framed = true;
        bevy::log::info!(
            "viewer: framed subject (extent {largest:.1}) at radius {:.1}",
            orbit.radius
        );
    }
}

/// Apply mouse input to the orbit state, then rebuild the camera transform.
pub fn orbit_camera(
    buttons: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    mut cameras: Query<(&mut OrbitCamera, &mut Transform)>,
) {
    for (mut orbit, mut transform) in &mut cameras {
        let delta = motion.delta;

        if buttons.pressed(MouseButton::Left) && delta != Vec2::ZERO {
            orbit.yaw -= delta.x * ORBIT_SENSITIVITY;
            orbit.pitch =
                (orbit.pitch + delta.y * ORBIT_SENSITIVITY).clamp(-PITCH_LIMIT, PITCH_LIMIT);
        }

        if buttons.pressed(MouseButton::Right) && delta != Vec2::ZERO {
            // Pan in the camera's own plane, scaled by distance so the model
            // tracks the cursor at any zoom level.
            let right = *transform.right();
            let up = *transform.up();
            let scale = orbit.radius * PAN_SENSITIVITY;
            orbit.focus += (-right * delta.x + up * delta.y) * scale;
        }

        if scroll.delta.y != 0.0 {
            // Multiplicative so each notch is the same proportional step
            // whether you are 5 m or 500 m out.
            orbit.radius = (orbit.radius * (1.0 - scroll.delta.y * ZOOM_SENSITIVITY)).max(0.1);
        }

        let rotation = Quat::from_euler(EulerRot::YXZ, orbit.yaw, -orbit.pitch, 0.0);
        transform.rotation = rotation;
        transform.translation = orbit.focus + rotation * Vec3::new(0.0, 0.0, orbit.radius);
    }
}
