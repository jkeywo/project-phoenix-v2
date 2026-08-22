//! Render half of the simulation app (issue #1195) -- the Bevy presentation
//! systems lifted out of `server_app` so the assembly module stays sim-only.
//!
//! Surface: the `render`-gated systems `render_spawned_entities`,
//! `update_mesh_lod`, and `face_player_lights`, plus the `ProceduralMeshCache`
//! resource and its `procedural_mesh_material` factory (the last two re-exported
//! from `server_app` for the model viewer). The LOD component, light spawners,
//! and retire/level helpers are module-private.
//!
//! Role: a render adapter. It reads simulation state (transforms, `[mesh]`/
//! `[light]` sections) and writes only Bevy visual components (`Mesh3d`,
//! `StandardMaterial`, `PointLight`, camera-facing rotations).
//!
//! Invariant: presentation only, never authoritative simulation state. These
//! systems run on `Update` under `SimPluginOptions::render`, never on
//! `FixedUpdate`, and touch no component the digest reads -- so `world_digest`
//! and replay are byte-identical whether or not the render half runs.

use bevy::prelude::*;

use crate::core::messages::GamePhase;
use crate::entities::glb_visual::{
    resolve_sidecar_rig, resolve_tier_parent_scale, spawn_glb_visual, tier_parent_scale_at,
    GlbSpawnOutcome, PendingSceneHandle,
};
use crate::server_app::{FacePlayerLight, LocalShip, LocalShipModel};
use std::collections::HashMap;

/// Marker: entity mesh has been rendered (GLB procedural).
/// Prevents re-processing by `render_spawned_entities`.
#[derive(Component)]
pub(crate) struct RenderProcessed;

/// Tag a freshly spawned GLB `SceneRoot` as the local ship's model: hidden by
/// default (shown only by the cinematic camera) and exempt from frustum culling
/// because it sits at the camera origin.
///
/// `spawn_glb_visual` is deliberately ignorant of the simulation, so this
/// decoration is applied by the caller to the child entity it returns.
fn decorate_local_ship_model(commands: &mut Commands, child: Entity) {
    commands.entity(child).insert((
        Visibility::Hidden,
        LocalShipModel,
        bevy::camera::visibility::NoFrustumCulling,
    ));
}

/// Rounded key for a cached procedural mesh (geometry only — colour/emissive do
/// not affect the mesh, so they are excluded to maximise sharing).
#[derive(Clone, PartialEq, Eq, Hash)]
struct ProcMeshKey {
    /// Shape discriminant: 0 = sphere, 1 = cuboid, 2 = torus.
    shape: u8,
    radius_q: i32,
    size_q: [i32; 3],
    minor_q: i32,
}

/// Rounded key for a cached procedural material (appearance only).
#[derive(Clone, PartialEq, Eq, Hash)]
struct ProcMatKey {
    colour_q: [i32; 3],
    emissive_q: i32,
}

/// Quantise a float to milli-units for use in a hashable cache key.
fn quantize_key(v: f32) -> i32 {
    (v * 1000.0).round() as i32
}

/// Deduplicates procedural meshes and materials by rounded key so that all
/// identical primitives (e.g. every distant asteroid's far-LOD sphere) share a
/// single mesh handle and a single material handle. Reusing handles lets the
/// renderer batch/instance the draws instead of issuing one per entity.
/// `pub(crate)` for the model viewer, which builds a ladder's procedural far
/// level through the same constructor rather than growing its own sphere.
#[derive(Resource, Default)]
pub(crate) struct ProceduralMeshCache {
    meshes: HashMap<ProcMeshKey, Handle<Mesh>>,
    materials: HashMap<ProcMatKey, Handle<StandardMaterial>>,
}

/// A procedural LOD level's own rotation, as a quaternion. Identity when the
/// level declares none.
fn level_rotation(level: &crate::entities::config::LodLevel) -> Quat {
    level
        .rotation
        .map(|r| Quat::from_euler(EulerRot::XYZ, r[0], r[1], r[2]))
        .unwrap_or(Quat::IDENTITY)
}

/// Build — or fetch from `cache` — the `Mesh3d`/material handles for a
/// procedural primitive. Mirrors PATH B of the flat renderer but routes through
/// the cache so identical primitives share handles. Shared by the flat renderer
/// and the LOD system.
pub(crate) fn procedural_mesh_material(
    cache: &mut ProceduralMeshCache,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    shape: crate::entities::config::MeshShape,
    radius: f32,
    size: Option<[f32; 3]>,
    minor_radius: f32,
    colour: &[f32],
    emissive_mul: f32,
) -> (Handle<Mesh>, Handle<StandardMaterial>) {
    use crate::entities::config::MeshShape;

    let (shape_id, size_for_key) = match shape {
        MeshShape::Sphere => (0u8, [0.0; 3]),
        MeshShape::Cuboid => (1u8, size.unwrap_or([2.0, 1.0, 3.0])),
        MeshShape::Torus => (2u8, [0.0; 3]),
    };
    let mesh_key = ProcMeshKey {
        shape: shape_id,
        radius_q: quantize_key(radius),
        size_q: [
            quantize_key(size_for_key[0]),
            quantize_key(size_for_key[1]),
            quantize_key(size_for_key[2]),
        ],
        minor_q: quantize_key(minor_radius),
    };
    let mesh_handle = cache
        .meshes
        .entry(mesh_key)
        .or_insert_with(|| match shape {
            MeshShape::Sphere => meshes.add(Sphere {
                radius: radius.max(0.1),
            }),
            MeshShape::Cuboid => {
                let [x, y, z] = size.unwrap_or([2.0, 1.0, 3.0]);
                meshes.add(Cuboid::new(x, y, z))
            }
            MeshShape::Torus => meshes.add(Torus {
                major_radius: radius.max(0.5),
                minor_radius: minor_radius.max(0.1),
            }),
        })
        .clone();

    let rgb = if colour.len() >= 3 {
        [colour[0], colour[1], colour[2]]
    } else {
        [0.6, 0.6, 0.6]
    };
    let mat_key = ProcMatKey {
        colour_q: [
            quantize_key(rgb[0]),
            quantize_key(rgb[1]),
            quantize_key(rgb[2]),
        ],
        emissive_q: quantize_key(emissive_mul),
    };
    let mat_handle = cache
        .materials
        .entry(mat_key)
        .or_insert_with(|| {
            let color = Color::srgb(rgb[0], rgb[1], rgb[2]);
            let emissive = LinearRgba::from(color) * emissive_mul;
            materials.add(StandardMaterial {
                base_color: color,
                emissive,
                ..default()
            })
        })
        .clone();

    (mesh_handle, mat_handle)
}

/// Distance-based mesh LOD state, attached to entities whose model rig sidecar
/// declares one or more `[[lod]]` levels. [`update_mesh_lod`] selects and swaps
/// the active level each frame based on camera distance;
/// [`render_spawned_entities`] skips rendering these entities directly.
#[derive(Component)]
pub(crate) struct MeshLods {
    /// Ordered near→far LOD levels copied from the model's rig sidecar
    /// ([`crate::entities::model_rig::ModelRig::lod`], issue #914).
    levels: Vec<crate::entities::config::LodLevel>,
    /// Flat mesh config supplying fallback fields (colour/radius/emissive/size/
    /// minor_radius) and the shared `variant` for levels that omit them.
    base: crate::entities::config::MeshConfig,
    /// The primary model sidecar's `[base].scale`. Every LOD tier of one model
    /// shares the SAME base sidecar (`ModelRig::lod`), but each tier's own GLB
    /// resolves its OWN sidecar in `spawn_glb_visual` — so only the near tier,
    /// whose model IS the primary GLB, is guaranteed to pick this up. Captured
    /// here so `update_mesh_lod` can make every tier (GLB and billboard) reach
    /// the same world size the near tier does. How much of it a given ladder's
    /// far tiers still need is [`tier_parent_scale`]'s question, not this one's.
    base_scale: [f32; 3],
    /// Cached [`resolve_tier_parent_scale`] for this ladder: the scale a
    /// non-near tier folds onto the PARENT transform. Resolved on the first
    /// switch away from the near tier and reused after, so the extra sidecar
    /// read costs once per entity rather than once per LOD crossing.
    tier_scale: Option<Vec3>,
    /// Active level index; `None` until the first evaluation establishes it.
    current: Option<usize>,
    /// The child carrying the active level's visual — a GLB level's
    /// `SceneRoot`, or a shape level's `Mesh3d`.
    scene_child: Option<Entity>,
    /// Whether this entity is the local player's ship (GLB starts hidden).
    is_local_ship: bool,
}

/// Retire whichever visual the active LOD level installed, so a new level can be
/// built cleanly. Both kinds of level hang their visual off a child — a GLB's
/// `SceneRoot`, a shape's `Mesh3d`, a billboard's root — so this deals with
/// exactly one entity, via `try_despawn` (safe if it was already removed; Bevy
/// 0.18 `despawn` panics on an already-despawned entity).
///
/// With a cross-fade window authored (`[render] lod_fade_secs`, PRD #1023) the
/// outgoing child is NOT despawned here: it is handed to
/// [`crate::entities::visual_fade`], which fades it out over the window and
/// despawns it at the end, while the incoming tier fades in over the same
/// window. Both tiers are on screen for it, so the outgoing one is rescaled to
/// hold the world size it had — the entity's own transform is about to become
/// the INCOMING tier's, and that scale is the thing `tier_parent_scale` exists
/// to get right. `fade_secs = 0` is the same-frame cut this always was.
///
/// Note: this intentionally does NOT remove `ModelMarkers`. On a GLB→GLB switch
/// the new level's `spawn_glb_visual` re-inserts `ModelMarkers`, and because
/// commands apply in enqueue order, a blanket `remove` here (queued after that
/// insert) would clobber the new markers. `ModelMarkers` is instead cleared
/// explicitly in the procedural branch of [`update_mesh_lod`] when switching
/// away from a GLB level to a shape level.
fn retire_lod_visual(
    commands: &mut Commands,
    lods: &mut MeshLods,
    fade_secs: f32,
    outgoing_parent_scale: Vec3,
    incoming_parent_scale: Vec3,
) {
    let Some(child) = lods.scene_child.take() else {
        return;
    };
    if fade_secs <= 0.0 {
        commands.entity(child).try_despawn();
        return;
    }
    let correction = crate::entities::visual_fade::parent_scale_correction(
        outgoing_parent_scale,
        incoming_parent_scale,
    );
    commands
        .entity(child)
        .insert(crate::entities::visual_fade::VisualFade::fade_out(
            fade_secs,
        ))
        .entry::<Transform>()
        .and_modify(move |mut tf| tf.scale *= correction);
}

/// Add visual meshes and materials to spawned entities that have a `[mesh]`
/// section but no `RenderProcessed` yet. When `cfg.model` is set, loads a GLB
/// scene instead of creating a procedural shape — but defers insertion until
/// the asset is actually loaded (avoids attaching an unloaded handle that
/// would never retry). Applies `cfg.scale` and `cfg.rotation` to the entity's
/// transform in both paths. Additionally, if the entity carries a `Lights`
/// component (from one or more `[[light]]` TOML entries), attach the matching
/// `PointLight`/`DirectionalLight` components (single light → inline, multiple
/// → spawned as child entities).
///
/// Entities whose model rig sidecar declares a `[[lod]]` chain are NOT rendered
/// here: they receive a [`MeshLods`] component and are driven by
/// [`update_mesh_lod`].
pub(crate) fn render_spawned_entities(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut star_surface_materials: ResMut<Assets<crate::entities::star::StarSurfaceMaterial>>,
    mut star_halo_materials: ResMut<Assets<crate::entities::star::StarHaloMaterial>>,
    mut planet_surface_materials: ResMut<Assets<crate::entities::planet::PlanetSurfaceMaterial>>,
    mut planet_cloud_materials: ResMut<Assets<crate::entities::planet::PlanetCloudMaterial>>,
    mut proc_cache: ResMut<ProceduralMeshCache>,
    scenes: Res<Assets<bevy::scene::Scene>>,
    tuning: Option<Res<crate::render_setup::RenderTuning>>,
    phase: Option<Res<State<GamePhase>>>,
    entities: Query<
        (
            Entity,
            &Transform,
            Option<&crate::entities::spawner::MeshSection>,
            Option<&crate::entities::spawner::StarSection>,
            Option<&crate::entities::spawner::PlanetSection>,
            Option<&crate::entities::spawner::Lights>,
            Option<&PendingSceneHandle>,
            Option<&crate::server_app::LocalShip>,
        ),
        Without<RenderProcessed>,
    >,
) {
    // A mid-mission arrival materialises rather than popping (PRD #1023). Only
    // the GLB path takes it: a procedural entity's mesh hangs off the ENTITY,
    // not a child, and an entity's transform is simulation state — physics
    // rewrites it every tick and Rapier reads its scale — so a visual effect
    // has no business animating it. `Option<Res<_>>` per AGENTS.md, so a
    // bare-`App` fixture that registered neither resource still validates.
    let tuning = tuning.map(|t| *t).unwrap_or_default();
    // Every visual on this path is an entity's first and only one — a model
    // with no `[[lod]]` chain never swaps — so `first_visual` is unconditional.
    let materialise = tuning.arrival(
        true,
        phase.is_some_and(|p| *p.get() == GamePhase::InProgress),
    );

    for (entity, transform, mesh_sec, star_sec, planet_sec, lights_opt, pending, local_ship) in
        entities.iter()
    {
        let mesh_cfg_for_transform = mesh_sec.map(|mesh_sec| &mesh_sec.0);

        if let Some(star_sec) = star_sec {
            crate::entities::celestial_visual::insert_star_visual(
                &mut commands,
                &mut meshes,
                &mut star_surface_materials,
                &mut star_halo_materials,
                entity,
                &star_sec.0,
            );
        } else if let Some(planet_sec) = planet_sec {
            // Textured planet: UV sphere with the custom planet shader, plus
            // an optional alpha-blended cloud shell child. Checked before the
            // `[mesh]` branch — planet templates keep a procedural `[mesh]`
            // fallback for headless/editor contexts that must not win here.
            crate::entities::celestial_visual::insert_planet_visual(
                &mut commands,
                &mut meshes,
                &mut planet_surface_materials,
                &mut planet_cloud_materials,
                &asset_server,
                entity,
                &planet_sec.0,
            );
        } else if let Some(mesh_sec) = mesh_sec {
            let cfg = &mesh_sec.0;

            if let Some(model_path) = &cfg.model {
                // The LOD ladder is owned by the model, not the entity (issue
                // #914), so whether this is a LOD entity at all is a question
                // only the rig sidecar can answer — resolve it first. On wasm a
                // sidecar still in flight yields `None`; retry next frame, which
                // is the same wait the flat GLB path already takes. On native
                // the read is synchronous.
                let Some(rig) = resolve_sidecar_rig(model_path, cfg.variant.as_deref()) else {
                    continue;
                };
                if !rig.lod.is_empty() {
                    // LOD entity: defer the visual to `update_mesh_lod`, which
                    // selects a level by camera distance each frame. Attach the
                    // LOD state; the flat paths below are skipped for this
                    // entity. `base` stays the entity's own `[mesh]` so a shared
                    // ladder still renders each rock's authored colour/radius.
                    commands.entity(entity).insert(MeshLods {
                        levels: rig.lod.clone(),
                        base: cfg.clone(),
                        base_scale: rig.base.scale,
                        tier_scale: None,
                        current: None,
                        scene_child: None,
                        is_local_ship: local_ship.is_some(),
                    });
                } else {
                    // PATH A: GLB model (shared helper preserves the async logic).
                    // `rig` was already resolved above to answer "does this model
                    // have a [[lod]] chain" — hand it straight through instead of
                    // making spawn_glb_visual read/parse the same sidecar again.
                    match spawn_glb_visual(
                        &mut commands,
                        &asset_server,
                        &scenes,
                        entity,
                        model_path,
                        cfg.variant.as_deref(),
                        pending,
                        Some(&rig),
                    ) {
                        GlbSpawnOutcome::Spawned(child) => {
                            if local_ship.is_some() {
                                decorate_local_ship_model(&mut commands, child);
                            }
                            if let Some(fade) = materialise {
                                commands.entity(child).insert(fade);
                            }
                        }
                        // GLB / rig not loaded yet — try again next frame.
                        GlbSpawnOutcome::Pending => continue,
                        GlbSpawnOutcome::Failed => {
                            // Stop retrying an entity whose GLB will never load.
                            commands.entity(entity).insert(RenderProcessed);
                            continue;
                        }
                    }
                }
            } else {
                // PATH B: Procedural primitive (deduped via the shared cache).
                let emissive_mul = cfg.emissive.unwrap_or(0.4);
                let (mesh, mat) = procedural_mesh_material(
                    &mut proc_cache,
                    &mut meshes,
                    &mut materials,
                    cfg.shape,
                    cfg.radius,
                    cfg.size,
                    cfg.minor_radius,
                    &cfg.colour,
                    emissive_mul,
                );
                commands
                    .entity(entity)
                    .insert((Mesh3d(mesh), MeshMaterial3d(mat)));
            }
        } else {
            continue;
        }

        // Apply scale/rotation — preserves spawn position. `mesh_cfg_for_transform`
        // is `None` for stars, so this is a no-op on that path.
        if let Some(cfg) =
            mesh_cfg_for_transform.filter(|cfg| cfg.scale != 1.0 || cfg.rotation != [0.0, 0.0, 0.0])
        {
            commands.entity(entity).insert(Transform {
                translation: transform.translation,
                rotation: bevy::math::Quat::from_euler(
                    bevy::math::EulerRot::XYZ,
                    cfg.rotation[0],
                    cfg.rotation[1],
                    cfg.rotation[2],
                ),
                scale: Vec3::splat(cfg.scale),
            });
        }

        // Mark processed so we never visit this entity again.
        let mut ec = commands.entity(entity);
        ec.insert(RenderProcessed);

        // Attach lights, if any. A light that needs to face the player must
        // be its own child entity so rotating it doesn't rotate the parent's
        // visual mesh; otherwise a single light can live on the entity itself.
        if let Some(lights_comp) = lights_opt {
            let lights = &lights_comp.0;
            let needs_children = lights.len() > 1 || lights.iter().any(|l| l.face_player);
            match (lights.len(), needs_children) {
                (0, _) => {}
                (1, false) => insert_light(&mut ec, &lights[0]),
                _ => {
                    ec.with_children(|parent| {
                        for light in lights {
                            spawn_child_light(parent, light);
                        }
                    });
                }
            }
        }
    }
}

/// Distance-based LOD driver. For each entity carrying a [`MeshLods`] component,
/// computes the 3-D distance from the [`GameCamera`](crate::render_setup::GameCamera)
/// to the entity, selects the appropriate level via
/// [`crate::entities::config::select_lod`] (with hysteresis), and — when the chosen
/// level differs from the current one — tears down the old visual and builds the
/// new one through the same helpers the flat renderer uses.
///
/// GLB levels that are still async-loading keep the current visual and retry
/// next frame, so a switch never leaves the entity permanently invisible.
/// Runs after [`render_spawned_entities`] so newly-attached `MeshLods` are
/// established the same frame they are spawned.
///
/// Two presentation windows ride this swap since PRD #1023, both authored in the
/// world's `[render]` block and both no-ops at a duration of zero:
/// * a **cross-fade** whenever one tier replaces another — see
///   [`retire_lod_visual`];
/// * a **materialisation** on an entity's FIRST tier, when the mission is
///   already in progress. That is the mid-mission arrival the PRD asks for: a
///   reinforcement's visual appears the frame its GLB finishes streaming, and
///   this covers exactly that latency rather than pretending to be a warp-in.
pub(crate) fn update_mesh_lod(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut proc_cache: ResMut<ProceduralMeshCache>,
    scenes: Res<Assets<bevy::scene::Scene>>,
    tuning: Option<Res<crate::render_setup::RenderTuning>>,
    phase: Option<Res<State<GamePhase>>>,
    camera: Query<&GlobalTransform, With<crate::render_setup::GameCamera>>,
    mut lod_entities: Query<(
        Entity,
        &mut Transform,
        &mut MeshLods,
        Option<&PendingSceneHandle>,
    )>,
) {
    use crate::entities::config::select_lod;

    // No camera → nothing to measure distance against; try again next frame.
    let Some(cam_tf) = camera.iter().next() else {
        return;
    };
    let cam_pos = cam_tf.translation();

    // `Option<Res<_>>` for the reason `LogFilterConfig` is (AGENTS.md): a bare
    // `Res` fails Bevy's parameter validation in any bare-`App` fixture. The
    // fallback is the same authored calibration the resource is initialised to.
    let tuning = tuning.map(|t| *t).unwrap_or_default();
    let mid_mission = phase.is_some_and(|p| *p.get() == GamePhase::InProgress);

    for (entity, mut transform, mut lods, pending) in lod_entities.iter_mut() {
        // Use the entity's LOCAL transform, not its `GlobalTransform`: on the
        // frame an entity is first rendered its `MeshLods` is inserted this same
        // Update, but global transforms aren't propagated until PostUpdate, so a
        // `GlobalTransform` read here would still be the identity default and pick
        // the initial level from distance-to-origin (a one-frame wrong-LOD flash).
        // Asteroids are top-level/unparented, so local == world. If a parented
        // entity ever needs LOD, this must switch to a propagated world position.
        let distance = transform.translation.distance(cam_pos);
        let target = select_lod(&lods.levels, distance, lods.current);

        // Issue lod-preload-by-distance, part 3: always try to have the next
        // MORE detailed level (one index closer than `target`) warm in the
        // asset server's cache, so an approaching ship never triggers a
        // fresh async load the frame it actually crosses into that band —
        // it's already sitting in cache from here. Runs every frame
        // regardless of whether `target` just changed (a ship can sit near a
        // boundary for a while before crossing it), and never touches
        // `lods.current` or the displayed visual — only the block below does
        // that. `asset_server.load()` is idempotent: a path already
        // loading/loaded just returns the existing handle, so this is cheap
        // once warm.
        if target > 0 {
            if let Some(model_path) = lods
                .levels
                .get(target - 1)
                .and_then(|level| level.model.as_deref())
            {
                let rel = model_path.strip_prefix("assets/").unwrap_or(model_path);
                let _: Handle<bevy::scene::Scene> = asset_server.load(format!("{rel}#Scene0"));
            }
        }

        if lods.current == Some(target) {
            continue;
        }

        // Copy the target level out so the `lods` borrow is free for teardown.
        let Some(level) = lods.levels.get(target).cloned() else {
            continue;
        };

        // Recompute the entity's scale from the flat `[mesh] scale` and this
        // level's optional `[x, y, z]`. Recomputed rather than multiplied in,
        // so switching between levels that do and do not declare one is
        // symmetric and leaves nothing to unwind: a level with no `scale` puts
        // the entity back to exactly what it spawned with.
        //
        // `tier_base` folds in however much of the primary sidecar's
        // `[base].scale` this tier still needs — see `tier_parent_scale`, which
        // the viewer's own LOD path asks the same question of. Resolved once per
        // entity, then cached.
        let ladder_tier_scale = match lods.tier_scale {
            Some(scale) => scale,
            None => {
                match resolve_tier_parent_scale(
                    &lods.levels,
                    lods.base_scale,
                    lods.base.variant.as_deref(),
                ) {
                    Some(scale) => {
                        lods.tier_scale = Some(scale);
                        scale
                    }
                    // wasm only: the generated tier's sidecar is still in
                    // flight. Hold the current visual and retry next frame,
                    // exactly as a pending GLB does below.
                    None => continue,
                }
            }
        };
        let tier_base = tier_parent_scale_at(target, ladder_tier_scale);
        // NOT assigned yet. A GLB level can come back `Pending` for any number
        // of frames while its scene streams, and the OLD level's child is still
        // on screen for every one of them — so rescaling the parent here would
        // dress the outgoing model in the incoming tier's scale and hold it
        // there until the swap lands. Each branch below assigns it at the point
        // it actually commits to the new level.
        let next_scale = Vec3::splat(lods.base.scale)
            * tier_base
            * level.scale.map(Vec3::from_array).unwrap_or(Vec3::ONE);
        // The scale the OUTGOING tier is currently drawn at — read before any
        // branch overwrites it, because that is what a cross-fading outgoing
        // child has to be corrected back to once the entity takes the incoming
        // tier's scale.
        let outgoing_scale = transform.scale;

        // This entity's FIRST visual materialises (if the mission is already
        // running); every later one cross-fades with the tier it replaces.
        let arrival = tuning.arrival(lods.current.is_none(), mid_mission);
        // Nothing to fade out of on the first tier, whatever the window says.
        let fade_out_secs = if lods.scene_child.is_some() {
            tuning.lod_fade_secs
        } else {
            0.0
        };

        if let Some(model_path) = level.model.as_deref() {
            let variant = level.variant.clone().or_else(|| lods.base.variant.clone());
            // A tier whose ladder declares it ships NO sidecar is handed the
            // identity rig it would have resolved anyway, so nothing requests a
            // file that was deliberately never written — on wasm that request is
            // a fetch, and its 404 is the console error this path used to print
            // for every hull model in the scene. Any other tier's sidecar hasn't
            // been resolved yet this frame; let spawn_glb_visual resolve it.
            let declared = crate::entities::glb_visual::declared_tier_rig(&level);
            match spawn_glb_visual(
                &mut commands,
                &asset_server,
                &scenes,
                entity,
                model_path,
                variant.as_deref(),
                pending,
                declared.as_ref(),
            ) {
                // Keep the current visual until the new GLB resolves — avoids a
                // visible gap. `current` is left unchanged so we retry next frame.
                GlbSpawnOutcome::Pending => continue,
                GlbSpawnOutcome::Failed => {
                    // Give up on this level; drop the old visual and settle so we
                    // stop retrying it every frame. No cross-fade: there is
                    // nothing incoming to fade the outgoing tier against, and
                    // holding a dead tier on screen for the window would only
                    // delay the (already wrong) empty result.
                    transform.scale = next_scale;
                    retire_lod_visual(&mut commands, &mut lods, 0.0, outgoing_scale, next_scale);
                    lods.current = Some(target);
                }
                GlbSpawnOutcome::Spawned(child) => {
                    transform.scale = next_scale;
                    if lods.is_local_ship {
                        decorate_local_ship_model(&mut commands, child);
                    }
                    retire_lod_visual(
                        &mut commands,
                        &mut lods,
                        fade_out_secs,
                        outgoing_scale,
                        next_scale,
                    );
                    if let Some(fade) = arrival {
                        commands.entity(child).insert(fade);
                    }
                    lods.scene_child = Some(child);
                    lods.current = Some(target);
                }
            }
        } else if let Some(shape) = level.shape {
            // A procedural level builds its mesh from cached primitives and so
            // commits this same frame — nothing to wait for, so the scale lands
            // here rather than in a branch below.
            transform.scale = next_scale;
            // Procedural level — fields fall back to the flat `base` config.
            let radius = level.radius.unwrap_or(lods.base.radius);
            let minor = level.minor_radius.unwrap_or(lods.base.minor_radius);
            let size = level.size.or(lods.base.size);
            let emissive_mul = level.emissive.or(lods.base.emissive).unwrap_or(0.4);
            let colour = level
                .colour
                .clone()
                .unwrap_or_else(|| lods.base.colour.clone());
            let (mesh, mat) = procedural_mesh_material(
                &mut proc_cache,
                &mut meshes,
                &mut materials,
                shape,
                radius,
                size,
                minor,
                &colour,
                emissive_mul,
            );
            retire_lod_visual(
                &mut commands,
                &mut lods,
                fade_out_secs,
                outgoing_scale,
                next_scale,
            );
            // Switching to a shape level: drop any `ModelMarkers` left by a prior
            // GLB level (no-op if absent). Enqueued after teardown, so it never
            // races a freshly-inserted marker map.
            commands
                .entity(entity)
                .remove::<crate::entities::model_rig::ModelMarkers>();
            // The mesh goes on a CHILD, as a GLB level's `SceneRoot` does, so
            // the level can carry its own rotation. Rotating the entity itself
            // is not available: an entity's rotation is simulation state, and
            // physics rewrites it every tick on anything that moves.
            let child = commands
                .spawn((
                    Mesh3d(mesh),
                    MeshMaterial3d(mat),
                    Transform::from_rotation(level_rotation(&level)),
                ))
                .id();
            commands.entity(entity).add_child(child);
            if let Some(fade) = arrival {
                commands.entity(child).insert(fade);
            }
            lods.scene_child = Some(child);
            lods.current = Some(target);
        } else if let Some(atlas) = level.billboard.as_deref() {
            // Billboard level: a camera-facing quad textured from a yaw-ring
            // atlas. Width/height (world units) come from the level's `scale`;
            // the ENTITY takes a UNIFORM scale here rather than `next_scale` —
            // that would multiply in this level's `[w, h, 1]`, which on a quad
            // that rotates to face the camera would shear it. The width/height
            // instead ride the child's own scale (see `spawn_billboard_child`),
            // leaving the parent uniform. A billboard commits this frame too, so
            // this is its equivalent of the `next_scale` assignment above.
            let billboard_scale = Vec3::splat(lods.base.scale);
            transform.scale = billboard_scale;
            // Quad size and ring size are the billboard module's rules, asked
            // for here rather than restated — the viewer's own billboard
            // preview asks the same two questions of the same two functions.
            let [w, h] = crate::entities::billboard::billboard_quad_size(level.scale, tier_base);
            let views = crate::entities::billboard::billboard_yaw_views(&level);
            retire_lod_visual(
                &mut commands,
                &mut lods,
                fade_out_secs,
                outgoing_scale,
                billboard_scale,
            );
            commands
                .entity(entity)
                .remove::<crate::entities::model_rig::ModelMarkers>();
            let child = crate::entities::billboard::spawn_billboard_child(
                &mut commands,
                &mut meshes,
                &mut materials,
                &asset_server,
                atlas,
                w,
                h,
                views,
            );
            commands.entity(entity).add_child(child);
            if let Some(fade) = arrival {
                commands.entity(child).insert(fade);
            }
            lods.scene_child = Some(child);
            lods.current = Some(target);
        } else {
            // No model, shape, or billboard — invalid level. Settle so we don't spin.
            bevy::log::warn!(
                "update_mesh_lod: LOD level {target} on {entity:?} has no model, shape, or billboard — skipping"
            );
            lods.current = Some(target);
        }
    }
}

fn insert_light(
    ec: &mut bevy::ecs::system::EntityCommands,
    light: &crate::entities::config::LightConfig,
) {
    use crate::entities::config::LightKind;
    let color = Color::srgb(light.colour[0], light.colour[1], light.colour[2]);
    match light.kind {
        LightKind::Point => {
            ec.insert(PointLight {
                color,
                intensity: light.intensity,
                range: light.range.unwrap_or(50.0),
                shadows_enabled: false,
                ..default()
            });
        }
        LightKind::Directional => {
            ec.insert(DirectionalLight {
                color,
                illuminance: light.intensity,
                shadows_enabled: false,
                ..default()
            });
        }
    }
}

fn spawn_child_light(
    parent: &mut bevy::ecs::relationship::RelatedSpawnerCommands<ChildOf>,
    light: &crate::entities::config::LightConfig,
) {
    use crate::entities::config::LightKind;
    let color = Color::srgb(light.colour[0], light.colour[1], light.colour[2]);
    match light.kind {
        LightKind::Point => {
            let mut child = parent.spawn(PointLight {
                color,
                intensity: light.intensity,
                range: light.range.unwrap_or(50.0),
                shadows_enabled: false,
                ..default()
            });
            if light.face_player {
                child.insert(FacePlayerLight);
            }
        }
        LightKind::Directional => {
            let mut child = parent.spawn(DirectionalLight {
                color,
                illuminance: light.intensity,
                shadows_enabled: false,
                ..default()
            });
            if light.face_player {
                child.insert(FacePlayerLight);
            }
        }
    }
}

/// Rotates every [`FacePlayerLight`] entity so it points toward the
/// player's ship, independent of its parent entity's orientation.
pub(crate) fn face_player_lights(
    ship_query: Query<&GlobalTransform, With<LocalShip>>,
    mut light_query: Query<(&GlobalTransform, &mut Transform), With<FacePlayerLight>>,
) {
    let Some(ship_transform) = ship_query.iter().next() else {
        return;
    };
    let player_pos = ship_transform.translation();
    for (global, mut transform) in &mut light_query {
        let light_pos = global.translation();
        if (player_pos - light_pos).length_squared() > f32::EPSILON {
            transform.rotation = Transform::from_translation(light_pos)
                .looking_at(player_pos, Vec3::Y)
                .rotation;
        }
    }
}

// -- Tests --
#[cfg(test)]
mod tests {
    use super::*;

    // ── LOD tier retirement (PRD #1023, module 5) ────────────────────────

    mod lod_cross_fade {
        use super::*;
        use crate::entities::visual_fade::{FadeDirection, VisualFade};

        /// The outgoing tier's own local scale before retirement. Any non-unit
        /// value does; what the test reads is what happened to it.
        const OUTGOING_CHILD_SCALE: f32 = 2.0;

        /// The flat `[mesh]` a `MeshLods` falls back to. Retirement reads none
        /// of it, so the values only have to be legal.
        fn bare_mesh_config() -> crate::entities::config::MeshConfig {
            crate::entities::config::MeshConfig {
                model: None,
                variant: None,
                shape: crate::entities::config::MeshShape::Sphere,
                colour: vec![0.5, 0.5, 0.5],
                radius: 1.0,
                size: None,
                minor_radius: 0.0,
                emissive: None,
                scale: 1.0,
                rotation: [0.0, 0.0, 0.0],
            }
        }

        /// Retire one LOD visual through a real `Commands`, and hand back the
        /// world plus the child that was retired.
        fn retire(fade_secs: f32, outgoing: Vec3, incoming: Vec3) -> (World, Entity) {
            let mut world = World::new();
            let entity = world.spawn(Transform::default()).id();
            let child = world
                .spawn(Transform::from_scale(Vec3::splat(OUTGOING_CHILD_SCALE)))
                .id();
            world.entity_mut(entity).add_child(child);

            let mut lods = MeshLods {
                levels: Vec::new(),
                base: bare_mesh_config(),
                base_scale: [1.0, 1.0, 1.0],
                tier_scale: None,
                current: Some(0),
                scene_child: Some(child),
                is_local_ship: false,
            };
            {
                let mut commands = world.commands();
                retire_lod_visual(&mut commands, &mut lods, fade_secs, outgoing, incoming);
            }
            world.flush();
            assert_eq!(
                lods.scene_child, None,
                "a retired visual is no longer the LOD's own child either way"
            );
            (world, child)
        }

        /// No authored window is the same-frame cut this always was — the
        /// behaviour every LOD test written before the cross-fade assumes.
        #[test]
        fn a_zero_window_despawns_the_outgoing_tier_immediately() {
            let (world, child) = retire(0.0, Vec3::ONE, Vec3::splat(0.75));
            assert!(
                world.get_entity(child).is_err(),
                "with no window the outgoing tier goes this frame"
            );
        }

        /// With a window, the outgoing tier stays on screen and is handed to
        /// the fade driver, which owns its despawn.
        #[test]
        fn a_window_keeps_the_outgoing_tier_and_fades_it() {
            let (world, child) = retire(0.25, Vec3::ONE, Vec3::splat(0.75));
            let fade = world
                .get::<VisualFade>(child)
                .expect("the outgoing tier is handed to the fade driver");
            assert_eq!(fade.direction, FadeDirection::Out);
            assert_eq!(fade.duration, 0.25);
            assert_eq!(fade.alpha(), 1.0, "the fade starts from fully visible");
        }

        /// The invariant the flash fix (9135d400) established, extended across
        /// the window: the new scale lands with the new tier, so the OUTGOING
        /// tier has to be corrected off it or it changes size while it fades.
        /// A hull ladder's near tier folds in nothing and its far tiers the
        /// whole `[base].scale`, so an uncorrected outgoing near tier would
        /// visibly shrink to 75% over the quarter-second it is dying.
        #[test]
        fn the_outgoing_tier_holds_its_world_size_while_the_entity_takes_the_new_one() {
            let outgoing_parent = Vec3::ONE;
            let incoming_parent = Vec3::splat(0.75);
            let (world, child) = retire(0.25, outgoing_parent, incoming_parent);
            let corrected = world.get::<Transform>(child).unwrap().scale;
            let world_size = corrected * incoming_parent;
            let was = Vec3::splat(OUTGOING_CHILD_SCALE) * outgoing_parent;
            assert!(
                (world_size - was).length() < 1e-5,
                "the fading tier must stay at {was:?} in world units, got {world_size:?}"
            );
        }

        /// Tiers whose parent scale does not change across the switch — every
        /// pipeline ladder, where the base scale rides the child — must not be
        /// touched at all.
        #[test]
        fn an_unchanged_parent_scale_leaves_the_outgoing_tier_alone() {
            let (world, child) = retire(0.25, Vec3::ONE, Vec3::ONE);
            assert_eq!(
                world.get::<Transform>(child).unwrap().scale,
                Vec3::splat(OUTGOING_CHILD_SCALE)
            );
        }

        /// Nothing to retire is not an error: the first tier an entity ever
        /// shows has no predecessor, and the swap path must be able to say so
        /// without spawning a fade over an entity that does not exist.
        #[test]
        fn retiring_nothing_does_nothing() {
            let mut world = World::new();
            let mut lods = MeshLods {
                levels: Vec::new(),
                base: bare_mesh_config(),
                base_scale: [1.0, 1.0, 1.0],
                tier_scale: None,
                current: None,
                scene_child: None,
                is_local_ship: false,
            };
            {
                let mut commands = world.commands();
                retire_lod_visual(&mut commands, &mut lods, 0.25, Vec3::ONE, Vec3::ONE);
            }
            world.flush();
            assert_eq!(lods.scene_child, None);
        }
    }
}
