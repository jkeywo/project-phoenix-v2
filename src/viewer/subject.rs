//! Spawning the thing being looked at.
//!
//! Three sources, all routed through the game's own construction code:
//!
//! - `?model=` — a GLB, spawned via
//!   [`crate::entities::glb_visual::spawn_glb_visual`] so the `.model.toml`
//!   base rig is composed exactly as the game composes it.
//! - `?entity=` — an entity TOML, parsed with `EntityConfig::from_toml` and
//!   dispatched to the star, planet or mesh visual the game would build.
//! - a level of the selected model's LOD ladder ([`super::lod`]) — another GLB,
//!   the imposter billboard the game builds through the same
//!   `spawn_billboard_child` its own LOD swap uses, or the procedural primitive
//!   it builds through the same `procedural_mesh_material`.
//!
//! All are asynchronous on wasm (the GLB streams; the TOML is fetched by JS
//! and pushed back), so spawning is a poll loop rather than a one-shot.
//! [`Showing`] is the one field that says which of them is on screen; the LOD
//! module writes it, this module renders it.

use bevy::prelude::*;

use crate::entities::celestial_visual::{insert_planet_visual, insert_star_visual};
use crate::entities::config::{EntityConfig, MeshShape};
use crate::entities::glb_visual::{spawn_glb_visual, GlbSpawnOutcome, PendingSceneHandle};
use crate::entities::planet::{PlanetCloudMaterial, PlanetSurfaceMaterial};
use crate::entities::star::{StarHaloMaterial, StarSurfaceMaterial};

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
    /// A billboard LOD level: the imposter atlas quad the game draws in the
    /// farthest band, built through the game's own
    /// [`crate::entities::billboard::spawn_billboard_child`] and turned by the
    /// game's own `orient_lod_billboards`.
    ///
    /// Until PRD #1023 the viewer had no way to build one, so `showing_for`
    /// returned [`Showing::Base`] for a billboard level and the panel's "fixed
    /// 3" showed the FULL-DETAIL model where the game draws an imposter. That
    /// is the tooling gap the PRD names: billboard pose snapping and per-level
    /// atlas quality shipped because the one tool for reviewing far LODs could
    /// not display the thing being reviewed.
    Billboard(BillboardLevel),
}

/// A billboard LOD level, resolved to what the renderer needs to build it.
#[derive(Debug, Clone, PartialEq)]
pub struct BillboardLevel {
    /// Atlas PNG path, as authored (`assets/…`).
    pub atlas: String,
    /// Quad width and height in world units —
    /// [`crate::entities::billboard::billboard_quad_size`]'s answer for this
    /// tier, so the preview is the size the game draws.
    pub size: [f32; 2],
    /// How many yaw tiles the atlas packs.
    pub views: u32,
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
    ladder: Option<Res<crate::viewer::lod::LadderState>>,
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

    // A billboard LOD level: the game's own imposter quad, built through the
    // game's own spawn so the preview IS what ships. The subject transform takes
    // a UNIFORM scale for the same reason `update_mesh_lod` gives the entity
    // one — the quad's world width and height ride the billboard's own root, so
    // rotating it to face the camera never shears it — and unity, because the
    // viewer has no entity `[mesh] scale` to fold in.
    if let Showing::Billboard(level) = state.showing.clone() {
        let child = crate::entities::billboard::spawn_billboard_child(
            &mut commands,
            &mut meshes,
            &mut standard_materials,
            &asset_server,
            &level.atlas,
            level.size[0],
            level.size[1],
            level.views,
        );
        state.extents = Some(Vec3::new(level.size[0], level.size[1], level.size[0]));
        commands
            .entity(entity)
            .insert(Transform::from_scale(Vec3::ONE))
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
        // When the thing on screen is a LADDER TIER that declares it ships no
        // sidecar, hand over the identity rig it would resolve rather than
        // asking for a file the pipeline never wrote. The base model (no level
        // showing) always has its own sidecar, so it resolves normally.
        let declared = level_glb
            .as_ref()
            .and(ladder.as_deref())
            .and_then(|l| l.current.and_then(|i| l.levels.get(i)))
            .and_then(crate::entities::glb_visual::declared_tier_rig);
        match spawn_glb_visual(
            &mut commands,
            &asset_server,
            &scenes,
            entity,
            &model_path,
            variant.as_deref(),
            pending,
            declared.as_ref(),
        ) {
            GlbSpawnOutcome::Pending => {}
            GlbSpawnOutcome::Failed => {
                bevy::log::error!("viewer: could not load {model_path}");
                state.settled = true;
            }
            GlbSpawnOutcome::Spawned(_) => {
                // Re-reading the sidecar `spawn_glb_visual` just resolved is a
                // cache hit — EXCEPT on a tier that declared it has none, where
                // it would be the very fetch the declaration exists to avoid.
                // That tier's rig is the identity one, which carries no extents,
                // so the answer is `None` either way; take it without asking.
                state.extents = match &declared {
                    Some(rig) => rig.extents.as_ref().map(|e| Vec3::from_array(e.size)),
                    None => crate::entities::glb_visual::resolve_sidecar_rig(
                        &model_path,
                        variant.as_deref(),
                    )
                    .and_then(|rig| rig.extents.map(|e| Vec3::from_array(e.size))),
                };
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
        crate::entities::config_cache::take_pending_sidecar_toml(path).or_else(|| {
            // Not optional: this reads the entity TOML `?entity=` names, which
            // the caller asked for by name and which therefore ought to exist.
            crate::entities::config_cache::request_sidecar_fetch(path.to_string(), false);
            None
        })
    }
}
