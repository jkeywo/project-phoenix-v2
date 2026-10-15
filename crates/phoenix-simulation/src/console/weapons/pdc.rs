//! Physical bolt contact, reusing the actual Rapier shape owner. Selection and
//! aiming happen before this stage and cannot query these physical bodies.
use bevy::prelude::*;
use bevy_rapier3d::{na, parry, prelude::Collider};

/// A detached carrier owns only emitted flight. It cannot charge a launcher,
/// advance an authored volley, debit ammo, mint an ID or draw new firing power.
pub(crate) fn advance_carrier_bolts(
    time: Res<Time>,
    mut carriers: Query<
        &mut super::BlasterSystemResource,
        (
            With<super::flight_carrier::WeaponFlightCarrier>,
            Without<crate::server_app::Ship>,
        ),
    >,
) {
    for mut banks in &mut carriers {
        for bank in &mut banks.0 {
            bank.advance_flight_only(time.delta_secs());
        }
    }
}

/// Read the same current ship pose as sync_ship_position, without waiting for
/// presentation Transform propagation. Bare physical bodies retain their pose.
pub(super) fn physical_pose(
    transform: &Transform,
    physics: Option<&crate::ship::state::ShipPhysics>,
) -> (Vec3, Quat) {
    physics.map_or((transform.translation, transform.rotation), |pose| {
        (
            Vec3::new(pose.x, pose.y, pose.z),
            Quat::from_euler(EulerRot::YXZ, -pose.yaw, 0.0, pose.roll),
        )
    })
}

pub(crate) fn collider_fraction(
    start: [f32; 3],
    end: [f32; 3],
    radius: f32,
    position: Vec3,
    rotation: Quat,
    collider: &Collider,
) -> Option<f32> {
    let velocity = Vec3::from_array(end) - Vec3::from_array(start);
    if !Vec3::from_array(start).is_finite()
        || !velocity.is_finite()
        || !position.is_finite()
        || !rotation.is_finite()
        || !radius.is_finite()
        || radius < 0.0
    {
        return None;
    }
    let bolt_pose = na::Isometry3::translation(start[0], start[1], start[2]);
    let body_pose = na::Isometry3::from_parts(
        na::Translation3::new(position.x, position.y, position.z),
        na::UnitQuaternion::from_quaternion(na::Quaternion::new(
            rotation.w, rotation.x, rotation.y, rotation.z,
        )),
    );
    parry::query::cast_shapes(
        &bolt_pose,
        &na::Vector3::new(velocity.x, velocity.y, velocity.z),
        &parry::shape::Ball::new(radius),
        &body_pose,
        &na::Vector3::zeros(),
        collider.raw.as_ref(),
        parry::query::ShapeCastOptions::with_max_time_of_impact(1.0),
    )
    .ok()
    .flatten()
    .map(|hit| hit.time_of_impact)
    .filter(|fraction| fraction.is_finite() && (0.0..=1.0).contains(fraction))
}

#[cfg(test)]
#[path = "pdc_tests.rs"]
mod tests;
