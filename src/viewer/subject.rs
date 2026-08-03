//! Spawning the thing being looked at.
//!
//! Three sources, all routed through the game's own construction code:
//!
//! - `?model=` — a GLB, spawned via
//!   [`crate::entities::glb_visual::spawn_glb_visual`] so the `.model.toml`
//!   base rig is composed exactly as the game composes it.
//! - `?entity=` — an entity TOML, parsed with `EntityConfig::from_toml` and
//!   dispatched to the star, planet or mesh visual the game would build.
//! - a level of the selected model's LOD ladder ([`super::lod`]) — either
//!   another GLB or, for a far level, the procedural primitive the game builds
//!   through the same `procedural_mesh_material` its own LOD swap uses.
//!
//! All are asynchronous on wasm (the GLB streams; the TOML is fetched by JS
//! and pushed back), so spawning is a poll loop rather than a one-shot.
//! [`Showing`] is the one field that says which of them is on screen; the LOD
//! module writes it, this module renders it.

use bevy::prelude::*;

use crate::entities::celestial_visual::{insert_planet_visual, insert_star_visual};
use crate::entities::glb_visual::{spawn_glb_visual, GlbSpawnOutcome, PendingSceneHandle};
use crate::entity_config::{EntityConfig, MeshShape};
use crate::entity_planet::{PlanetCloudMaterial, PlanetSurfaceMaterial};
use crate::entity_star::{StarHaloMaterial, StarSurfaceMaterial};

use super::ViewerArgs;

/// The entity holding whatever is currently on display.
#[derive(Component)]
pub struct Subject;

/// A procedural LOD level, resolved against the renderer's fallbacks.
#[derive(Debug, Clone, PartialEq)]
pub struct ProceduralLevel {
    pub shape: MeshShape,
    pub radius: f32,
    pub size: Option<[f32; 3]>,
    pub minor_radius: f32,
    pub colour: Vec<f32>,
    pub emissive: f32,
    /// The level's `[x, y, z]` scale, applied to the subject's transform the
    /// way `update_mesh_lod` applies it to the entity's.
    pub scale: Vec3,
    /// The level's own rotation, applied to a child of the subject — the same
    /// arrangement the game uses. It has to be the same one: a non-uniform
    /// scale and a rotation do not commute, so scale-on-parent ×
    /// rotation-on-child and both-on-one-transform are different orientations,
    /// and a viewer that picked the other would be showing something the game
    /// never renders.
    pub rotation: Quat,
}

/// What the subject is rendering right now.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Showing {
    /// The `?model=`/`?entity=` subject itself — the ladder is not in play.
    #[default]
    Base,
    /// A GLB LOD level.
    Glb {
        path: String,
        variant: Option<String>,
        /// The level's `[x, y, z]` scale — `Vec3::ONE` unless it declares one.
        scale: Vec3,
    },
    /// A procedural LOD level.
    Shape(ProceduralLevel),
}

/// Tracks the in-flight spawn so the poll loop knows when to stop.
#[derive(Resource, Default)]
pub struct SubjectState {
    /// Set once the subject has fully resolved (or permanently failed).
    pub settled: bool,
    /// Rig extents of the current model, used once by the camera to frame it.
    pub extents: Option<Vec3>,
    /// Which of the three sources is on screen.
    pub showing: Showing,
    /// Cleared on every respawn; the stats pass sets it once it has counted
    /// what the new visual is made of.
    pub measured: bool,
    /// Set while an explicit asset reload is in flight, so the subject is
    /// rebuilt when the new bytes land rather than from the cached ones.
    pub reloading: bool,
}

impl SubjectState {
    /// Tear down the current subject and put a fresh, empty one in its place,
    /// for [`poll_pending_model`] to attach the next visual to. Used when the
    /// HTML panel switches models and when the LOD level changes.
    ///
    /// The replacement is spawned inside the same queued closure as the
    /// teardown, not left to `spawn_subject` — that runs at `Startup` only, so
    /// a bare despawn left the world with no subject at all and the poll loop
    /// (which needs exactly one) returned early forever, blanking the viewer on
    /// the first model switch.
    pub fn respawn(&mut self, commands: &mut Commands) {
        self.settled = false;
        self.extents = None;
        self.measured = false;
        commands.queue(|world: &mut World| {
            let existing: Vec<Entity> = world
                .query_filtered::<Entity, With<Subject>>()
                .iter(world)
                .collect();
            for entity in existing {
                world.entity_mut(entity).despawn();
            }
            world.spawn(subject_bundle());
        });
    }
}

/// The empty subject entity, at the origin. Its visual is attached later by
/// [`poll_pending_model`] once the assets resolve.
fn subject_bundle() -> (Subject, Transform, Visibility) {
    (Subject, Transform::default(), Visibility::default())
}

/// Startup: create the subject entity.
pub fn spawn_subject(mut commands: Commands) {
    commands.spawn(subject_bundle());
}

/// Each frame until settled: try to attach a visual to the subject entity.
pub fn poll_pending_model(
    mut commands: Commands,
    args: Res<ViewerArgs>,
    mut state: ResMut<SubjectState>,
    asset_server: Res<AssetServer>,
    scenes: Res<Assets<bevy::scene::Scene>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut standard_materials: ResMut<Assets<StandardMaterial>>,
    mut proc_cache: ResMut<crate::server_app::ProceduralMeshCache>,
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

    // A procedural LOD level: built through the game's own cache-backed
    // constructor, so the far-level sphere the viewer draws is the same mesh
    // and material the game would draw.
    if let Showing::Shape(level) = state.showing.clone() {
        let (mesh, material) = crate::server_app::procedural_mesh_material(
            &mut proc_cache,
            &mut meshes,
            &mut standard_materials,
            level.shape,
            level.radius,
            level.size,
            level.minor_radius,
            &level.colour,
            level.emissive,
        );
        state.extents = Some(procedural_extents(&level));
        let child = commands
            .spawn((
                Mesh3d(mesh),
                MeshMaterial3d(material),
                Transform::from_rotation(level.rotation),
            ))
            .id();
        commands
            .entity(entity)
            .insert(Transform::from_scale(level.scale))
            .add_child(child);
        state.settled = true;
        return;
    }

    // The base model, or a GLB LOD level of it. Both are the same spawn.
    let level_glb = match &state.showing {
        Showing::Glb {
            path,
            variant,
            scale,
        } => Some((path.clone(), variant.clone(), *scale)),
        _ => None,
    };
    let model = level_glb.clone().or_else(|| {
        args.model
            .clone()
            .map(|m| (m, args.variant.clone(), Vec3::ONE))
    });

    if let Some((model_path, variant, scale)) = model {
        // Every respawn starts from a fresh subject at identity, so this is
        // the level's scale rather than a correction of a previous one.
        commands.entity(entity).insert(Transform::from_scale(scale));
        match spawn_glb_visual(
            &mut commands,
            &asset_server,
            &scenes,
            entity,
            &model_path,
            variant.as_deref(),
            pending,
            None,
        ) {
            GlbSpawnOutcome::Pending => {}
            GlbSpawnOutcome::Failed => {
                bevy::log::error!("viewer: could not load {model_path}");
                state.settled = true;
            }
            GlbSpawnOutcome::Spawned(_) => {
                state.extents = crate::entities::glb_visual::resolve_sidecar_rig(
                    &model_path,
                    variant.as_deref(),
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

/// Rebuild the subject once an explicitly reloaded asset has actually landed.
///
/// `AssetServer::reload` is asynchronous, and the old value stays in
/// `Assets<Scene>` until the new one arrives — so respawning at the moment the
/// reload is *requested* would rebuild from exactly the bytes being replaced.
/// `Modified` is the event that says the value has been swapped; a first load
/// emits `Added` and `LoadedWithDependencies` instead, so this cannot fire on
/// one.
pub fn respawn_on_asset_reload(
    mut scenes: MessageReader<AssetEvent<Scene>>,
    mut state: ResMut<SubjectState>,
    mut commands: Commands,
) {
    let modified = scenes
        .read()
        .any(|event| matches!(event, AssetEvent::Modified { .. }));
    if state.reloading && modified {
        state.reloading = false;
        state.respawn(&mut commands);
    }
}

/// The bounding size of a procedural level, so the camera can frame it the way
/// it frames a model's rig extents.
fn procedural_extents(level: &ProceduralLevel) -> Vec3 {
    level.scale * unscaled_extents(level)
}

fn unscaled_extents(level: &ProceduralLevel) -> Vec3 {
    match level.shape {
        MeshShape::Sphere => Vec3::splat(level.radius * 2.0),
        MeshShape::Cuboid => level.size.map(Vec3::from_array).unwrap_or(Vec3::ONE),
        // A torus is as wide as its major diameter plus a tube either side, and
        // only as tall as the tube.
        MeshShape::Torus => Vec3::new(
            (level.radius + level.minor_radius) * 2.0,
            level.minor_radius * 2.0,
            (level.radius + level.minor_radius) * 2.0,
        ),
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
