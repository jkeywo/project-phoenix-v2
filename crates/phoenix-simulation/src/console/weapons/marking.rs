//! Actual endpoint hull contact. Rapier geometry stays at the physical adapter;
//! no controller, pure flight or observation consumer receives foreign poses.
use crate::{
    entities::spawner::{ColliderSection, EntityUuid},
    ship::state::ShipPhysics,
};
use bevy::prelude::*;
use bevy_rapier3d::prelude::{Collider, ColliderScale};
use phoenix_sim_gameplay::weapons::marking::ShipMarkingAttachment;

#[derive(Clone, Debug)]
pub(crate) struct MarkingContact {
    pub round: String,
    pub subject: String,
    pub source: String,
    pub point: Vec3,
    pub offset: [f32; 3],
    pub payload: phoenix_model::marking::FrozenMarkingPayload,
    pub parent: ShipPhysics,
}

/// Endpoint containment only. No coarse radius, visual scale or swept/CCD claim.
fn surface(collider: &Collider, pose: &ShipPhysics, point: Vec3) -> Option<(Vec3, [f32; 3])> {
    let translation = Vec3::new(pose.x, pose.y, pose.z);
    let rotation = crate::ship::physics_systems::physical_rotation(pose);
    if !point.is_finite() || !collider.contains_point(translation, rotation, point) {
        return None;
    }
    let projection = collider
        .project_point(translation, rotation, point, false)
        .point;
    let offset = rotation.inverse() * (projection - translation);
    (projection.is_finite() && offset.is_finite()).then_some((projection, offset.to_array()))
}

#[derive(bevy::ecs::system::SystemParam)]
pub(crate) struct MarkingContacts<'w, 's> {
    hulls: Query<
        'w,
        's,
        (
            &'static EntityUuid,
            &'static ShipPhysics,
            &'static Collider,
            &'static ColliderScale,
            &'static ColliderSection,
        ),
        (
            With<crate::ship::compartment_damage::ShipDamage>,
            Without<ChildOf>,
            Without<ShipMarkingAttachment>,
            // Accepted ProbeProfile is materialized as PhysicalProbe. Neither
            // deployed nor stored carry-capable bodies belong to this slice.
            Without<crate::entities::craft::PhysicalProbe>,
            Without<crate::entities::craft::ProbeCustody>,
        ),
    >,
}
impl MarkingContacts<'_, '_> {
    pub(crate) fn hits(
        &self,
        rounds: &crate::weapons::torpedo::TorpedoSystem,
    ) -> Vec<MarkingContact> {
        let mut hulls: Vec<_> = self.hulls.iter().collect();
        hulls.sort_by(|a, b| a.0 .0.cmp(&b.0 .0));
        rounds
            .in_flight
            .iter()
            .filter_map(|round| {
                let payload = round.profile.marking.as_ref()?;
                let source = round.source_uuid.as_ref()?;
                let point = Vec3::new(round.x, round.y, round.z);
                hulls.iter().filter(|(id, _, _, scale, section)| {
                id.0 != *source
                    && matches!(scale, ColliderScale::Absolute(value) if *value == Vec3::ONE)
                    && matches!(section.0.shape, crate::entities::config::ColliderShape::Ball
                        | crate::entities::config::ColliderShape::Capsule
                        | crate::entities::config::ColliderShape::Cylinder)
            }).filter_map(|(id, pose, collider, _, _)| {
                surface(collider, pose, point).map(|(contact, offset)| {
                    let distance = point.distance_squared(Vec3::new(pose.x, pose.y, pose.z));
                    (distance, MarkingContact { round: round.uuid.clone(), subject: id.0.clone(),
                        source: source.clone(), point: contact, offset, payload: payload.clone(), parent: **pose })
                })
            }).min_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.subject.cmp(&b.1.subject)))
                .map(|(_, contact)| contact)
            })
            .collect()
    }
}

#[derive(Component, Clone, Debug)]
pub(crate) struct MarkingBeaconProfile(pub phoenix_model::marking::MarkingBeaconConfig);

pub(crate) fn install(
    config: &crate::entities::config::EntityConfig,
    point: Vec3,
    cmds: &mut EntityCommands,
) {
    if let Some(profile) = &config.marking_beacon {
        cmds.insert((
            MarkingBeaconProfile(profile.clone()),
            ShipPhysics {
                x: point.x,
                y: point.y,
                z: point.z,
                ..Default::default()
            },
        ));
    }
}

#[derive(bevy::ecs::system::SystemParam)]
pub(crate) struct MarkingRuntime<'w, 's> {
    pub contacts: MarkingContacts<'w, 's>,
    tick: Option<Res<'w, crate::sim_tick::SimTick>>,
    mint: crate::world_id::LiveMint<'w, { crate::world_id::IdNamespace::Entity as usize }>,
}
impl MarkingRuntime<'_, '_> {
    /// Existing Entity mint and ordinary spawner materialise real equipment.
    /// No runtime filesystem fallback is allowed after the content closure froze.
    pub fn spawn(&self, contact: MarkingContact, commands: &mut Commands) -> Option<Entity> {
        let tick = self.tick.as_ref()?.0;
        self.mint.as_deref()?;
        let template = &contact.payload.recipe.template;
        let mut config = crate::entities::config_cache::get_cached_entity_config(template)?;
        phoenix_sim_gameplay::weapons::marking::validate_payload_recipe(
            &contact.payload.recipe,
            &config,
        )
        .ok()?;
        config.faction = Some(contact.payload.faction);
        let uuid = crate::world_id::mint_live_id_with(
            self.mint.as_deref(),
            crate::world_id::IdNamespace::Entity,
        );
        let state = phoenix_model::marking::MarkingAttachment {
            round: contact.round,
            source: contact.source,
            payload: contact.payload.clone(),
            contact_tick: tick,
            location: phoenix_model::marking::MarkingLocation::Attached {
                subject: contact.subject,
                offset: contact.offset,
            },
            report: None,
        };
        let pose =
            crate::ship::physics_systems::marking_attached_pose(&contact.parent, contact.offset);
        let mut overrides = toml::map::Map::new();
        overrides.insert(
            "faction".into(),
            toml::Value::String(contact.payload.faction.to_string()),
        );
        let origin = crate::world::spawn_origin::SpawnOrigin {
            template_path: template.clone(),
            name: String::new(),
            position: contact.point.to_array(),
            overrides: Some(toml::Value::Table(overrides)),
            ..Default::default()
        };
        let entity =
            crate::entities::spawner::spawn_entity(commands, &config, contact.point, uuid, None);
        commands.entity(entity).insert((
            ShipMarkingAttachment(state),
            pose,
            Transform::from_translation(Vec3::new(pose.x, pose.y, pose.z))
                .with_rotation(crate::ship::physics_systems::physical_rotation(&pose)),
            crate::entities::spawner::EntitySpawnOrigin(origin),
            crate::entities::spawner::EntityTemplatePath::new(template),
            bevy_rapier3d::prelude::ActiveHooks::FILTER_CONTACT_PAIRS,
        ));
        Some(entity)
    }
}

/// One exact parent/body predicate shared by Rapier and ordinary collision
/// damage/de-overlap. Detachment immediately ends the pair exclusion.
#[derive(bevy::ecs::system::SystemParam)]
pub(crate) struct MechanicalPairs<'w, 's> {
    bodies: Query<'w, 's, (&'static EntityUuid, Option<&'static ShipMarkingAttachment>)>,
}
impl MechanicalPairs<'_, '_> {
    pub fn connected(&self, a: Entity, b: Entity) -> bool {
        let (Ok((a_id, a_marker)), Ok((b_id, b_marker))) = (self.bodies.get(a), self.bodies.get(b))
        else {
            return false;
        };
        let attached = |marker: Option<&ShipMarkingAttachment>, parent: &str| {
            marker.is_some_and(|marker|
            matches!(&marker.0.location, phoenix_model::marking::MarkingLocation::Attached { subject, .. } if subject == parent))
        };
        attached(a_marker, &b_id.0) || attached(b_marker, &a_id.0)
    }
}
#[derive(bevy::ecs::system::SystemParam)]
pub(crate) struct MarkingPhysicsHooks<'w, 's> {
    pairs: MechanicalPairs<'w, 's>,
}
impl bevy_rapier3d::prelude::BevyPhysicsHooks for MarkingPhysicsHooks<'_, '_> {
    fn filter_contact_pair(
        &self,
        context: bevy_rapier3d::prelude::PairFilterContextView,
    ) -> Option<bevy_rapier3d::rapier::geometry::SolverFlags> {
        (!self
            .pairs
            .connected(context.collider1(), context.collider2()))
        .then_some(bevy_rapier3d::rapier::geometry::SolverFlags::COMPUTE_IMPULSES)
    }
}

#[cfg(test)]
#[path = "marking_tests.rs"]
mod owner_tests;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn actual_shape_requires_endpoint_inside_and_projects_full_banked_rotation() {
        let pose = ShipPhysics {
            x: 12.0,
            y: 3.0,
            z: -7.0,
            yaw: 0.7,
            roll: 0.5,
            ..Default::default()
        };
        for collider in [
            Collider::ball(2.0),
            Collider::capsule_y(3.0, 1.0),
            Collider::cylinder(3.0, 1.0),
        ] {
            let rotation = crate::ship::physics_systems::physical_rotation(&pose);
            let origin = Vec3::new(pose.x, pose.y, pose.z);
            let endpoint = origin + rotation * Vec3::new(0.2, 1.0, 0.0);
            let (point, offset) = surface(&collider, &pose, endpoint).unwrap();
            assert!(point.distance(endpoint) > 0.01);
            let projected = crate::ship::physics_systems::marking_attached_pose(&pose, offset);
            assert!(Vec3::new(projected.x, projected.y, projected.z).distance(point) < 0.00001);
            assert!(surface(&collider, &pose, origin + Vec3::splat(10.0)).is_none());
        }
    }
}

pub(crate) fn register(app: &mut App) {
    use crate::authoritative::{DeclareState, StateClass};
    app.declare_state::<ShipMarkingAttachment>(StateClass::Folded, "marking-attachment");
    app.declare_state::<MarkingBeaconProfile>(StateClass::Derived, "marking-accepted-equipment");
}
