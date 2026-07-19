//! Spawning the thing being looked at.
//!
//! Two sources, both routed through the game's own construction code:
//!
//! - `?model=` — a GLB, spawned via
//!   [`crate::entities::glb_visual::spawn_glb_visual`] so the `.model.toml`
//!   base rig is composed exactly as the game composes it.
//! - `?entity=` — an entity TOML, parsed with `EntityConfig::from_toml` and
//!   dispatched to the star, planet or mesh visual the game would build.
//!
//! Both are asynchronous on wasm (the GLB streams; the TOML is fetched by JS
//! and pushed back), so spawning is a poll loop rather than a one-shot.

use bevy::prelude::*;

use crate::entities::celestial_visual::{insert_planet_visual, insert_star_visual};
use crate::entities::glb_visual::{spawn_glb_visual, GlbSpawnOutcome, PendingSceneHandle};
use crate::entity_config::EntityConfig;
use crate::entity_planet::{PlanetCloudMaterial, PlanetSurfaceMaterial};
use crate::entity_star::{StarHaloMaterial, StarSurfaceMaterial};

use super::ViewerArgs;

/// The entity holding whatever is currently on display.
#[derive(Component)]
pub struct Subject;

/// Tracks the in-flight spawn so the poll loop knows when to stop.
#[derive(Resource, Default)]
pub struct SubjectState {
    /// Set once the subject has fully resolved (or permanently failed).
    pub settled: bool,
    /// Rig extents of the current model, used once by the camera to frame it.
    pub extents: Option<Vec3>,
}

impl SubjectState {
    /// Tear down the current subject so `spawn_subject` builds a fresh one.
    /// Used when the HTML panel switches models.
    pub fn respawn(&mut self, commands: &mut Commands) {
        self.settled = false;
        self.extents = None;
        commands.queue(|world: &mut World| {
            let existing: Vec<Entity> = world
                .query_filtered::<Entity, With<Subject>>()
                .iter(world)
                .collect();
            for entity in existing {
                world.entity_mut(entity).despawn();
            }
        });
    }
}

/// Startup: create the subject entity at the origin. The visual is attached
/// later by [`poll_pending_model`] once its assets resolve.
pub fn spawn_subject(mut commands: Commands) {
    commands.spawn((Subject, Transform::default(), Visibility::default()));
}

/// Each frame until settled: try to attach a visual to the subject entity.
pub fn poll_pending_model(
    mut commands: Commands,
    args: Res<ViewerArgs>,
    mut state: ResMut<SubjectState>,
    asset_server: Res<AssetServer>,
    scenes: Res<Assets<bevy::scene::Scene>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut star_surface: ResMut<Assets<StarSurfaceMaterial>>,
    mut star_halo: ResMut<Assets<StarHaloMaterial>>,
    mut planet_surface: ResMut<Assets<PlanetSurfaceMaterial>>,
    mut planet_cloud: ResMut<Assets<PlanetCloudMaterial>>,
    subjects: Query<(Entity, Option<&PendingSceneHandle>), With<Subject>>,
) {
    if state.settled {
        return;
    }
    let Ok((entity, pending)) = subjects.single() else {
        // Zero subjects during a respawn, or more than one mid-teardown.
        return;
    };

    if let Some(model_path) = &args.model {
        match spawn_glb_visual(
            &mut commands,
            &asset_server,
            &scenes,
            entity,
            model_path,
            args.variant.as_deref(),
            pending,
        ) {
            GlbSpawnOutcome::Pending => {}
            GlbSpawnOutcome::Failed => {
                bevy::log::error!("viewer: could not load {model_path}");
                state.settled = true;
            }
            GlbSpawnOutcome::Spawned(_) => {
                state.extents = crate::entities::glb_visual::resolve_sidecar_rig(
                    model_path,
                    args.variant.as_deref(),
                )
                .and_then(|rig| rig.extents.map(|e| Vec3::from_array(e.size)));
                state.settled = true;
            }
        }
        return;
    }

    let Some(entity_path) = &args.entity else {
        state.settled = true;
        return;
    };

    // Entity TOML: routed through the same JS fetch queue the rig sidecars use.
    let Some(toml_str) = fetch_toml(entity_path) else {
        return; // fetch in flight — retry next frame
    };
    state.settled = true;
    let cfg = match EntityConfig::from_toml(&toml_str) {
        Ok(cfg) => cfg,
        Err(e) => {
            bevy::log::error!("viewer: {entity_path} failed to parse: {e}");
            return;
        }
    };

    if let Some(star) = &cfg.star {
        state.extents = Some(Vec3::splat(star.radius * 2.0));
        insert_star_visual(
            &mut commands,
            &mut meshes,
            &mut star_surface,
            &mut star_halo,
            entity,
            star,
        );
    } else if let Some(planet) = &cfg.planet {
        state.extents = Some(Vec3::splat(planet.radius * 2.0));
        insert_planet_visual(
            &mut commands,
            &mut meshes,
            &mut planet_surface,
            &mut planet_cloud,
            &asset_server,
            entity,
            planet,
        );
    } else if let Some(mesh) = &cfg.mesh {
        // An entity whose visual is just a GLB: re-enter the model path next
        // frame by rewriting args, so the rig handling stays in one place.
        if let Some(model) = &mesh.model {
            bevy::log::info!("viewer: {entity_path} resolves to model {model}");
            commands.queue({
                let model = model.clone();
                let variant = mesh.variant.clone();
                move |world: &mut World| {
                    let mut args = world.resource_mut::<ViewerArgs>();
                    args.model = Some(model);
                    args.variant = variant;
                    args.entity = None;
                    world.resource_mut::<SubjectState>().settled = false;
                }
            });
        } else {
            bevy::log::error!("viewer: {entity_path} has a [mesh] with no model path");
        }
    } else {
        bevy::log::error!("viewer: {entity_path} has no [star], [planet] or [mesh] section");
    }
}

/// Read a TOML by path: filesystem on native, JS fetch queue on wasm.
/// Returns `None` while a wasm fetch is still in flight.
fn fetch_toml(path: &str) -> Option<String> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::fs::read_to_string(path).ok()
    }
    #[cfg(target_arch = "wasm32")]
    {
        crate::config_cache::take_pending_sidecar_toml(path).or_else(|| {
            crate::config_cache::request_sidecar_fetch(path.to_string());
            None
        })
    }
}
