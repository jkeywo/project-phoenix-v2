//! Detached measured proxies; authoritative actor transforms remain untouched.
use crate::{
    core::messages::GamePhase,
    entities::spawner::{EntityTagsSection, EntityUuid},
    science::broad::SensingControls,
    server_app::{AsteroidUuid, LocalShip, ShipSystemBlackboards},
    ship::{components::ShipConfigComponent, state::ShipViewMode},
    world::config::WorldConfig,
};
use bevy::prelude::*;
use std::collections::BTreeMap;

#[derive(Component, Clone, Debug, PartialEq)]
pub(crate) struct ContactProxy {
    pub contact: String,
    pub radius: f32,
    pub colour: [f32; 3],
}

pub(crate) fn register(app: &mut App) {
    use crate::authoritative::{DeclareState, StateClass};
    app.declare_state::<ContactProxy>(StateClass::Presentation, "measured-contact-proxy")
        .add_systems(
            Update,
            (
                sync_contact_visuals
                    .after(crate::server_app_render::update_mesh_lod)
                    .before(crate::actor_visibility::sync),
                draw_contact_proxies,
            )
                .chain(),
        );
}

pub(crate) fn sync_contact_visuals(
    mut commands: Commands,
    phase: Option<Res<State<GamePhase>>>,
    world_config: Option<Res<WorldConfig>>,
    world_metadata: Option<Res<crate::lobby::WorldResource>>,
    accepted: Option<Res<crate::lobby::server::ShipClientConfigResource>>,
    runtime: Option<Res<crate::world::server::WorldContentRuntime>>,
    observer: Query<
        (
            Option<&SensingControls>,
            Option<&ShipConfigComponent>,
            Option<&ShipSystemBlackboards>,
            Option<&ShipViewMode>,
            Option<&EntityUuid>,
        ),
        With<LocalShip>,
    >,
    actors: Query<
        (
            Entity,
            Option<&EntityUuid>,
            Option<&AsteroidUuid>,
            Option<&EntityTagsSection>,
        ),
        (
            Or<(With<EntityUuid>, With<AsteroidUuid>)>,
            Without<LocalShip>,
            Without<ContactProxy>,
            Allow<bevy::ecs::entity_disabling::Disabled>,
        ),
    >,
    mut exclusions: ResMut<crate::actor_visibility::ObserverVisibilityRoots>,
    proxies: Query<(Entity, &ContactProxy), Without<EntityUuid>>,
) {
    let fitted = observer.single().ok();
    let restricted = super::measured_contacts::restricted(
        accepted.as_deref(),
        fitted.is_some_and(|(controls, ..)| controls.is_some()),
    );
    let own = fitted.and_then(|(_, _, _, _, id)| id.map(|id| id.0.as_str()));
    let active = restricted
        && phase
            .as_ref()
            .is_some_and(|phase| *phase.get() == GamePhase::InProgress);
    let mut hidden_roots = std::collections::HashSet::new();
    // Generated rocks carry their authored tags in the existing world roster,
    // rather than EntityTagsSection. Read permission only, never its geometry.
    let charted_metadata: std::collections::HashSet<_> = world_metadata
        .as_ref()
        .into_iter()
        .flat_map(|world| &world.0.entities)
        .filter(|row| {
            row.tags
                .iter()
                .any(|tag| tag == crate::console_delivery::CHARTED_TAG)
        })
        .map(|row| row.uuid.as_str())
        .collect();
    for (entity, source, rock, tags) in &actors {
        let source = source
            .map(|source| source.0.as_str())
            .or_else(|| rock.map(|source| source.0.as_str()))
            .unwrap_or_default();
        let charted = own.is_some()
            && tags.map_or_else(
                || charted_metadata.contains(source),
                |tags| {
                    tags.0
                        .iter()
                        .any(|tag| tag.as_str() == crate::console_delivery::CHARTED_TAG)
                },
            )
            && !super::measured_contacts::concealed(
                runtime.as_deref(),
                own.unwrap_or_default(),
                source,
            );
        if active && !charted {
            hidden_roots.insert(entity);
        }
    }
    exclusions.0 = hidden_roots;

    let style = world_config
        .as_ref()
        .and_then(|world| world.render.as_ref())
        .map(|render| render.contact_proxy.clone())
        .unwrap_or_default();
    let contacts = fitted
        .filter(|_| active)
        .and_then(|(_, config, boards, view, _)| {
            Some(super::measured_contacts::contacts(
                &config?.0, boards?, view?,
            ))
        })
        .unwrap_or_default();
    let mut desired: BTreeMap<_, _> = contacts
        .into_iter()
        .map(|track| {
            let proxy = ContactProxy {
                contact: track.contact_id.clone(),
                radius: track.position_error.max(style.minimum_radius),
                colour: style.colour,
            };
            (
                track.contact_id,
                (
                    proxy,
                    Transform::from_xyz(track.position[0], 0.0, track.position[1]),
                ),
            )
        })
        .collect();
    for (entity, proxy) in &proxies {
        if let Some(bundle) = desired.remove(&proxy.contact) {
            commands.entity(entity).insert(bundle);
        } else {
            commands.entity(entity).try_despawn();
        }
    }
    for bundle in desired.into_values() {
        commands.spawn(bundle);
    }
}

fn draw_contact_proxies(proxies: Query<(&ContactProxy, &Transform)>, mut gizmos: Gizmos) {
    for (proxy, transform) in &proxies {
        let colour = Color::srgb(proxy.colour[0], proxy.colour[1], proxy.colour[2]);
        // A directionless generic envelope represents recorded uncertainty.
        // Neither identity nor classification supplies the hidden hull's rig,
        // dimensions, live heading, current position or vertical placement.
        for rotation in [
            Quat::IDENTITY,
            Quat::from_rotation_x(std::f32::consts::FRAC_PI_2),
            Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
        ] {
            gizmos.circle(
                Isometry3d::new(transform.translation, rotation),
                proxy.radius,
                colour,
            );
        }
    }
}

#[cfg(test)]
#[path = "contact_visuals_tests.rs"]
mod tests;
