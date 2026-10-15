//! Visual projection of the sole canonical custody owner. Never simulation state.
use bevy::{ecs::entity_disabling::Disabled, prelude::*};
use phoenix_model::craft::CraftLocation;
use phoenix_simulation::entities::craft::ProbeCustody;

/// Derived observer exclusions; the same gate below combines custody reasons.
#[derive(Resource, Default)]
pub(crate) struct ObserverVisibilityRoots(pub std::collections::HashSet<Entity>);

/// One visibility owner remembers its prior value while any reason hides a root.
/// Deployed does not mean visible: observer privacy and prior authored visibility
/// remain authoritative presentation inputs, including in serverless previews.
pub(crate) fn sync(
    exclusions: Res<ObserverVisibilityRoots>,
    custody: Query<(Entity, &ProbeCustody), Allow<Disabled>>,
    parents: Query<&ChildOf, Allow<Disabled>>,
    mut actors: Query<(Entity, &mut Visibility), Allow<Disabled>>,
    mut previous: Local<std::collections::HashMap<Entity, Visibility>>,
) {
    let mut roots = exclusions.0.clone();
    for (entity, custody) in &custody {
        if matches!(custody.0, CraftLocation::Carried { .. }) {
            roots.insert(entity);
        }
    }
    let mut live = std::collections::HashSet::new();
    for (entity, mut visibility) in &mut actors {
        live.insert(entity);
        let mut current = entity;
        let mut hidden = roots.contains(&current);
        while !hidden {
            let Ok(parent) = parents.get(current) else {
                break;
            };
            current = parent.parent();
            hidden = roots.contains(&current);
        }
        if hidden {
            previous.entry(entity).or_insert(*visibility);
            visibility.set_if_neq(Visibility::Hidden);
        } else if let Some(original) = previous.remove(&entity) {
            visibility.set_if_neq(original);
        }
    }
    previous.retain(|entity, _| live.contains(entity));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server_app_render::{render_spawned_entities, ProceduralMeshCache};
    #[test]
    fn actual_procedural_probe_keeps_same_mesh_and_projects_custody_before_extraction() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default()))
            .init_asset::<Mesh>()
            .init_asset::<Image>()
            .init_asset::<StandardMaterial>()
            .init_asset::<bevy::scene::Scene>()
            .init_asset::<crate::entities::star::StarSurfaceMaterial>()
            .init_asset::<crate::entities::star::StarHaloMaterial>()
            .init_asset::<crate::entities::planet::PlanetSurfaceMaterial>()
            .init_asset::<crate::entities::planet::PlanetCloudMaterial>()
            .init_resource::<ProceduralMeshCache>()
            .init_resource::<ObserverVisibilityRoots>()
            .add_systems(Update, (render_spawned_entities, sync).chain());
        let config = phoenix_simulation::entities::include_resolve::load_entity_config(
            "assets/entities/physical_probe.toml",
        )
        .unwrap();
        let actor = phoenix_simulation::entities::spawner::spawn_entity(
            &mut app.world_mut().commands(),
            &config,
            Vec3::ZERO,
            "visual-probe".into(),
            None,
        );
        app.world_mut().flush();
        app.update();
        let mesh = app
            .world()
            .get::<Mesh3d>(actor)
            .expect("ordinary procedural renderer creates actual same-entity mesh")
            .0
            .id();
        assert!(app
            .world()
            .get::<Children>(actor)
            .is_none_or(|children| children.is_empty()));
        app.world_mut().entity_mut(actor).insert((
            ProbeCustody(CraftLocation::Carried {
                parent_uuid: "actual-parent".into(),
                bay: phoenix_model::identity::SystemId("actual-bay".into()),
            }),
            Disabled,
        ));
        let digest = phoenix_simulation::sim_digest::world_digest(app.world());
        app.update();
        assert_eq!(
            *app.world().get::<Visibility>(actor).unwrap(),
            Visibility::Hidden
        );
        assert_eq!(app.world().get::<Mesh3d>(actor).unwrap().0.id(), mesh);
        assert_eq!(
            phoenix_simulation::sim_digest::world_digest(app.world()),
            digest,
            "visual custody projection never mutates canonical state"
        );
        app.world_mut()
            .entity_mut(actor)
            .remove::<Disabled>()
            .insert(ProbeCustody(CraftLocation::Deployed {}));
        app.update();
        assert_eq!(
            *app.world().get::<Visibility>(actor).unwrap(),
            Visibility::Inherited
        );
        assert_eq!(app.world().get::<Mesh3d>(actor).unwrap().0.id(), mesh);
        assert!(app
            .world()
            .get::<Children>(actor)
            .is_none_or(|children| children.is_empty()));
    }
}
