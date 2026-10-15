//! Derived presence of the SAME actual fixed probe, including backend handles.
use super::*;
use bevy::ecs::entity_disabling::Disabled;
use bevy_rapier3d::prelude::{RapierColliderHandle, RapierRigidBodyHandle};

fn supported_body(world: &World, entity: Entity) -> Result<String, String> {
    let uuid = world
        .get::<EntityUuid>(entity)
        .ok_or("probe missing actual identity")?;
    let profile = world
        .get::<PhysicalProbe>(entity)
        .ok_or("target is not a supported actual probe")?;
    profile.0.validate()?;
    let physics = world
        .get::<crate::ship::state::ShipPhysics>(entity)
        .ok_or("probe missing its actual handled pose")?;
    if ![physics.x, physics.y, physics.z, physics.yaw, physics.roll]
        .iter()
        .all(|value| value.is_finite())
        || physics.roll != 0.0
        || physics.forward_speed != 0.0
        || physics.lateral_speed != 0.0
        || physics.vertical_speed != 0.0
        || world
            .get::<Children>(entity)
            .is_some_and(|children| !children.is_empty())
    {
        return Err("first actual probe requires finite fixed pose and no descendants".into());
    }
    Ok(uuid.0.clone())
}

/// Only called by the validated bay/bootstrap completion transaction. Roster,
/// compatible physical space, current contact/access and paid work are staged
/// by that caller. This function neither resolves a guessed foreign identity nor
/// publishes the actual carried actor's hidden state.
pub(crate) fn set_carried(
    world: &mut World,
    entity: Entity,
    parent: &str,
    bay: &SystemId,
) -> Result<(), String> {
    let uuid = supported_body(world, entity)?;
    let location = CraftLocation::Carried {
        parent_uuid: parent.into(),
        bay: bay.clone(),
    };
    location.validate(&uuid)?;
    if world.get::<ProbeCustody>(entity).is_none() {
        return Err("probe missing its sole custody owner".into());
    }
    withdraw_current_support(world, &uuid, entity);
    let mut actual = world.entity_mut(entity);
    actual.remove::<(RapierColliderHandle, RapierRigidBodyHandle)>();
    actual.insert((ProbeCustody(location), Disabled));
    Ok(())
}

pub(crate) fn set_deployed(
    world: &mut World,
    entity: Entity,
    position: [f32; 3],
    yaw: f32,
) -> Result<(), String> {
    supported_body(world, entity)?;
    if !position.iter().all(|value| value.is_finite()) || !yaw.is_finite() {
        return Err("release pose is not representable".into());
    }
    if world.get::<ProbeCustody>(entity).is_none() {
        return Err("probe missing its sole custody owner".into());
    }
    let mut physics = world
        .get_mut::<crate::ship::state::ShipPhysics>(entity)
        .expect("validated actual pose");
    physics.x = position[0];
    physics.y = position[1];
    physics.z = position[2];
    physics.yaw = yaw;
    physics.roll = 0.0;
    // The first probe has no motion actuator. Release writes the same pose once.
    physics.forward_speed = 0.0;
    physics.lateral_speed = 0.0;
    physics.vertical_speed = 0.0;
    drop(physics);
    if let Some(mut transform) = world.get_mut::<Transform>(entity) {
        transform.translation = Vec3::from_array(position);
        transform.rotation = Quat::from_euler(EulerRot::YXZ, -yaw, 0.0, 0.0);
    }
    let mut actual = world.entity_mut(entity);
    actual.remove::<Disabled>();
    actual.insert(ProbeCustody(CraftLocation::Deployed {}));
    // Preserved Collider/RigidBody re-enter ordinary Rapier initialization.
    Ok(())
}

/// Current routing is ephemeral. Original measurements/uncertainty/history stay
/// unchanged; neither storage nor this physical loss boundary buys a sample.
fn withdraw_current_support(world: &mut World, uuid: &str, probe: Entity) {
    let mut observers = world
        .query_filtered::<(Entity, &mut crate::science::broad::BroadTracks), Allow<Disabled>>();
    for (observer, mut states) in observers.iter_mut(world) {
        for state in states.0.values_mut() {
            if observer == probe {
                state.current.clear();
            } else {
                state.current.remove(uuid);
            }
            for track in state.tracks.values_mut() {
                if let Some(received) = track.received.as_mut() {
                    if observer == probe || received.route.as_deref() == Some(uuid) {
                        received.available = false;
                        received.route = None;
                    }
                }
            }
        }
    }
}

/// Canonical full-roster restore projects inactivity without changing knowledge.
/// All shape, clocks, space and parent/Job bindings were checked before writes.
pub(crate) fn restore_presence(world: &mut World, entity: Entity, location: &CraftLocation) {
    let mut actual = world.entity_mut(entity);
    match location {
        CraftLocation::Carried { .. } => {
            actual.remove::<(RapierColliderHandle, RapierRigidBodyHandle)>();
            actual.insert(Disabled);
        }
        CraftLocation::Deployed {} => {
            actual.remove::<Disabled>();
        }
    }
}
