use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use rand::Rng;
use std::collections::{HashMap, HashSet, VecDeque};

use crate::beam_render;
use crate::entity_config::{EnginePfxConfig, PhaserBankConfig};
use crate::entity_spawner::{EntityUuid, HelmConsoleSection};
use crate::messages::GamePhase;
use crate::model_rig::ModelMarkers;
use crate::ship_state::ShipPhysics;
use crate::simulation::{
    ActiveBeam, Asteroid, AsteroidUuid, LocalShip, PhaserRenderConfig, TorpedoSystemResource,
};
use crate::weapons_plugin::PhaserCombatConfigResource;

const BEAM_RADIUS: f32 = 0.02;
const BEAM_Y_OFFSET: f32 = 0.0;
const CONTACT_GLOW_RADIUS: f32 = 0.225;

const TORPEDO_RADIUS: f32 = 0.45;
const TORPEDO_TRAIL_RADIUS: f32 = 0.18;
const TORPEDO_TRAIL_LIFETIME_SECS: f32 = 0.32;
const TORPEDO_TRAIL_MIN_DISTANCE: f32 = 0.35;
const TORPEDO_BURST_LIFETIME_SECS: f32 = 0.35;

const ENGINE_DEFAULT_COLOR: [f32; 4] = [0.25, 0.75, 1.0, 0.72];
const ENGINE_TRAIL_RADIUS: f32 = 1.5;
const ENGINE_TRAIL_CRUMB_LIFETIME_SECS: f32 = 1.5;
const ENGINE_TRAIL_MAX_CRUMBS: usize = 200;
const ENGINE_TRAIL_MIN_CRUMB_DIST: f32 = 0.08;

pub struct PfxPlugin;

impl Plugin for PfxPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<BeamPfxState>()
            .init_resource::<TorpedoPfxState>()
            .init_resource::<EngineTrailState>()
            .add_systems(
                Update,
                (
                    sync_phaser_beams.run_if(in_state(GamePhase::InProgress)),
                    sync_torpedo_pfx.run_if(in_state(GamePhase::InProgress)),
                    spawn_engine_trails.run_if(in_state(GamePhase::InProgress)),
                    tick_lifetime_pfx.run_if(in_state(GamePhase::InProgress)),
                    tick_bursts.run_if(in_state(GamePhase::InProgress)),
                )
                    // These read ship `Transform`/`ShipPhysics`, which
                    // `sync_ship_position` (SimSet::Physics) writes each tick.
                    // Without this, the two systems have a genuine read/write
                    // conflict on `Transform` with no ordering constraint
                    // between them, so PFX can read a stale pre-physics
                    // transform depending on scheduler tie-breaking.
                    .after(crate::sim_sets::SimSet::Physics),
            )
            .add_systems(OnExit(GamePhase::InProgress), cleanup_pfx);
    }
}

#[derive(Component)]
struct PfxEntity;

#[derive(Component)]
struct BeamBody;

#[derive(Component)]
struct BeamContactGlow;

#[derive(Component)]
struct TorpedoBody;

#[derive(Component)]
struct PfxLifetime {
    age: f32,
    lifetime: f32,
}

#[derive(Component)]
struct PfxBurst {
    start_scale: f32,
    end_scale: f32,
}

#[derive(Component)]
struct PfxFadingMaterial {
    handle: Handle<StandardMaterial>,
    color: [f32; 4],
    emissive_strength: f32,
}

struct BeamEntities {
    body: Entity,
    glow: Entity,
}

#[derive(Resource, Default)]
struct BeamPfxState {
    active: HashMap<String, BeamEntities>,
    target_point_choices: HashMap<String, usize>,
}

struct TorpedoEntities {
    body: Entity,
    last_pos: Vec3,
}

#[derive(Resource, Default)]
struct TorpedoPfxState {
    active: HashMap<String, TorpedoEntities>,
}

#[derive(Clone, Debug)]
struct TrailCrumb {
    pos: Vec3,
    width: f32,
    age: f32,
    lifetime: f32,
}

struct EmitterTrail {
    crumbs: VecDeque<TrailCrumb>,
    mesh_handle: Handle<Mesh>,
    #[allow(dead_code)]
    entity: Entity,
}

#[derive(Resource, Default)]
struct EngineTrailState {
    emitters: HashMap<String, EmitterTrail>,
}

/// Unified beam-rendering system for every ship (player + NPC).
///
/// Iterates every ship with an `ActiveBeam` (`Query<..., With<Ship>>`) and
/// upserts a beam-body + contact-glow pair per active beam. The per-ship
/// `PhaserRenderConfig` component (color / range fallback) and
/// `PhaserCombatConfigResource` (per-bank color / range / marker) are read
/// from the shooter's own components — no separate player/NPC branches.
///
/// Beam origin: if the active bank has a `marker` name, use its transformed
/// world position; otherwise a bank-aware fallback centered on the ship's
/// [`Transform`] (bank facing → tangent offset around hull).
///
/// Beam end: target position resolved via [`target_position`] (asteroid, NPC,
/// player ship, or ship-target-point), then clamped to the bank/render range
/// via [`clamp_endpoint`] centered on the shooter's transform.
///
/// Key format: `"beam:<shooter_uuid>:<bank>:<target_uuid>"` — unique per
/// (shooter, bank, target) so simultaneous beams from different shooters or
/// different banks render as distinct entities.
fn sync_phaser_beams(
    // Every ship with an active beam. `EntityUuid` is `Option` because the
    // legacy player-ship spawn path assigned no UUID in some code paths; when
    // absent we synthesise `"local"` as the shooter identity.
    beam_ships_q: Query<
        (
            &Transform,
            Option<&ModelMarkers>,
            &ActiveBeam,
            Option<&EntityUuid>,
            Option<&PhaserRenderConfig>,
            Option<&PhaserCombatConfigResource>,
            bevy::ecs::query::Has<LocalShip>,
        ),
        (
            With<crate::server_app::Ship>,
            Without<BeamBody>,
            Without<BeamContactGlow>,
        ),
    >,
    // Resource-level fallbacks kept for legacy code paths that read only the
    // global resource (pre-PR-5 tests still work).
    render_cfg_res: Res<PhaserRenderConfig>,
    combat_cfg_res: Res<PhaserCombatConfigResource>,
    asteroid_q: Query<
        (&AsteroidUuid, &Transform),
        (With<Asteroid>, Without<BeamBody>, Without<BeamContactGlow>),
    >,
    entity_q: Query<
        (&EntityUuid, &Transform, Option<&ModelMarkers>),
        (
            Without<Asteroid>,
            Without<BeamBody>,
            Without<BeamContactGlow>,
        ),
    >,
    local_ship_q: Query<
        (&Transform, Option<&ModelMarkers>, Option<&EntityUuid>),
        (With<LocalShip>, Without<BeamBody>, Without<BeamContactGlow>),
    >,
    mut state: ResMut<BeamPfxState>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut body_q: Query<&mut Transform, (With<BeamBody>, Without<BeamContactGlow>)>,
    mut glow_q: Query<&mut Transform, (With<BeamContactGlow>, Without<BeamBody>)>,
) {
    let mut live_keys = HashSet::new();

    // LocalShip UUID is needed by `target_position` / `target_point_count` to
    // resolve beams that terminate on the player ship (NPC-fires-at-player).
    let local_ship_uuid = local_ship_q
        .single()
        .ok()
        .and_then(|(_, _, uuid)| uuid.map(|u| u.0.clone()));

    for (src_t, src_markers, beam, src_uuid_opt, render_cfg_opt, combat_cfg_opt, is_local) in
        beam_ships_q.iter()
    {
        let Some(target_uuid) = beam.target_uuid.clone() else {
            continue;
        };

        // Shooter identity: prefer the entity's own UUID; for the LocalShip in
        // legacy test harnesses without one, fall back to a stable string.
        // NPCs always have a UUID from `entities::spawner::spawn_entity`; skip
        // any that don't (nothing to key the render entity on).
        let src_key: String = match src_uuid_opt {
            Some(u) => u.0.clone(),
            None if is_local => "local".to_string(),
            None => continue,
        };

        let bank_id = beam.bank.as_deref().unwrap_or("default");
        let key = format!("beam:{}:{}:{}", src_key, bank_id, target_uuid);

        // Per-entity component paths (preferred). Fall back to the global
        // Resource so pre-PR-5 test paths still render.
        let render_cfg: &PhaserRenderConfig = render_cfg_opt.unwrap_or(&render_cfg_res);
        let combat_cfg: &PhaserCombatConfigResource = combat_cfg_opt.unwrap_or(&combat_cfg_res);
        let bank_cfg = beam
            .bank
            .as_deref()
            .and_then(|id| combat_cfg.0.bank_by_id(id));

        let target_point_index = choose_target_point_index(
            &key,
            target_point_count(
                &target_uuid,
                local_ship_uuid.as_deref(),
                &entity_q,
                &local_ship_q,
            ),
            &mut state,
        );
        let Some(target_pos) = target_position(
            &target_uuid,
            src_t,
            local_ship_uuid.as_deref(),
            target_point_index,
            &asteroid_q,
            &entity_q,
            &local_ship_q,
        ) else {
            continue;
        };

        let color = bank_cfg
            .map(|b| beam_render::resolve_beam_color(&b.beam_color))
            .unwrap_or(render_cfg.beam_color);
        let range = bank_cfg
            .map(|b| b.beam_range)
            .filter(|r| *r > 0.0)
            .unwrap_or(render_cfg.beam_range);

        // Origin: named marker takes priority; otherwise a bank-facing offset
        // around ship center (falls through to bare ship center when no bank
        // is defined). Uses the shooter's live Transform — position and yaw
        // both come from there, so this works for player and NPC alike.
        let origin = bank_cfg
            .and_then(|b| marker_origin(src_t, src_markers, b.marker.as_deref()))
            .unwrap_or_else(|| bank_fallback_origin(src_t, bank_cfg));
        let end = clamp_endpoint(origin, target_pos, src_t.translation, range);

        live_keys.insert(key.clone());
        upsert_beam(
            key,
            origin,
            end,
            color,
            &mut state,
            &mut commands,
            &mut meshes,
            &mut materials,
            &mut body_q,
            &mut glow_q,
        );
    }

    let dead: Vec<String> = state
        .active
        .keys()
        .filter(|key| !live_keys.contains(*key))
        .cloned()
        .collect();
    for key in dead {
        if let Some(entities) = state.active.remove(&key) {
            state.target_point_choices.remove(&key);
            commands.entity(entities.body).despawn();
            commands.entity(entities.glow).despawn();
        }
    }
}

fn upsert_beam(
    key: String,
    start: Vec3,
    end: Vec3,
    color: [f32; 4],
    state: &mut BeamPfxState,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    body_q: &mut Query<&mut Transform, (With<BeamBody>, Without<BeamContactGlow>)>,
    glow_q: &mut Query<&mut Transform, (With<BeamContactGlow>, Without<BeamBody>)>,
) {
    if let Some(existing) = state.active.get(&key) {
        if let Ok(mut body_transform) = body_q.get_mut(existing.body) {
            *body_transform = segment_transform(start, end, BEAM_RADIUS);
        }
        if let Ok(mut glow_transform) = glow_q.get_mut(existing.glow) {
            *glow_transform =
                Transform::from_translation(end).with_scale(Vec3::splat(CONTACT_GLOW_RADIUS));
        }
        return;
    }

    let body_mesh = meshes.add(Cylinder::new(1.0, 1.0));
    let glow_mesh = meshes.add(Sphere { radius: 1.0 });
    let body_mat = glow_material(materials, color, 6.0, AlphaMode::Blend);
    let glow_mat = glow_material(
        materials,
        [color[0], color[1], color[2], color[3] * 0.65],
        8.0,
        AlphaMode::Add,
    );

    let body = commands
        .spawn((
            PfxEntity,
            BeamBody,
            Mesh3d(body_mesh),
            MeshMaterial3d(body_mat),
            segment_transform(start, end, BEAM_RADIUS),
        ))
        .id();
    let glow = commands
        .spawn((
            PfxEntity,
            BeamContactGlow,
            Mesh3d(glow_mesh),
            MeshMaterial3d(glow_mat),
            Transform::from_translation(end).with_scale(Vec3::splat(CONTACT_GLOW_RADIUS)),
        ))
        .id();

    state.active.insert(key, BeamEntities { body, glow });
}

/// Renders every ship's in-flight torpedoes each frame.
///
/// Iterates `Query<..., With<Ship>>` so NPC torpedoes render alongside the
/// player's. Torpedo UUIDs are globally unique (uuid::Uuid::new_v4), so
/// merging in-flight lists across ships never collides on tracker keys.
fn sync_torpedo_pfx(
    ships_q: Query<&TorpedoSystemResource, With<crate::simulation::Ship>>,
    mut state: ResMut<TorpedoPfxState>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut transforms: Query<&mut Transform, With<TorpedoBody>>,
) {
    // Collect (uuid, x, z) triples for every in-flight torpedo across every
    // ship. A single flat list makes the diff-against-tracker trivial.
    let mut all_in_flight: Vec<(String, f32, f32)> = Vec::new();
    for torpedo_sys in ships_q.iter() {
        for t in &torpedo_sys.0.in_flight {
            all_in_flight.push((t.uuid.clone(), t.x, t.z));
        }
    }

    let live: HashSet<String> = all_in_flight.iter().map(|(u, _, _)| u.clone()).collect();
    let tracked: HashSet<String> = state.active.keys().cloned().collect();
    let (to_spawn, to_despawn) = diff_torpedo_sets(&live, &tracked);

    for uuid in to_despawn {
        if let Some(entities) = state.active.remove(&uuid) {
            commands.entity(entities.body).despawn();
            spawn_torpedo_burst(
                entities.last_pos,
                &mut commands,
                &mut meshes,
                &mut materials,
            );
        }
    }

    for uuid in to_spawn {
        if let Some((_, x, z)) = all_in_flight.iter().find(|(u, _, _)| u == &uuid) {
            let pos = Vec3::new(*x, 0.1, *z);
            let body = commands
                .spawn((
                    PfxEntity,
                    TorpedoBody,
                    Mesh3d(meshes.add(Sphere {
                        radius: TORPEDO_RADIUS,
                    })),
                    MeshMaterial3d(glow_material(
                        &mut materials,
                        [1.0, 0.78, 0.18, 1.0],
                        9.0,
                        AlphaMode::Opaque,
                    )),
                    Transform::from_translation(pos),
                ))
                .id();
            state.active.insert(
                uuid,
                TorpedoEntities {
                    body,
                    last_pos: pos,
                },
            );
        }
    }

    for (uuid, x, z) in &all_in_flight {
        let pos = Vec3::new(*x, 0.1, *z);
        if let Some(entities) = state.active.get_mut(uuid) {
            if entities.last_pos.distance(pos) >= TORPEDO_TRAIL_MIN_DISTANCE {
                spawn_trail_segment(
                    entities.last_pos,
                    pos,
                    TORPEDO_TRAIL_RADIUS,
                    [1.0, 0.45, 0.08, 0.5],
                    4.0,
                    TORPEDO_TRAIL_LIFETIME_SECS,
                    &mut commands,
                    &mut meshes,
                    &mut materials,
                );
            }
            entities.last_pos = pos;
            if let Ok(mut transform) = transforms.get_mut(entities.body) {
                transform.translation = pos;
            }
        }
    }
}

/// Updates per-ship engine trail ribbons (mesh + material) each frame.
///
/// Iterates every ship (player + NPC) uniformly. The key-base string
/// distinguishes ships by UUID; the LocalShip falls back to "engine:player"
/// only if it somehow has no `EntityUuid` (defensive — normally it does).
fn spawn_engine_trails(
    time: Res<Time>,
    mut state: ResMut<EngineTrailState>,
    ships_q: Query<
        (
            &Transform,
            &ShipPhysics,
            Option<&ModelMarkers>,
            Option<&HelmConsoleSection>,
            Option<&EntityUuid>,
            Has<LocalShip>,
        ),
        With<crate::simulation::Ship>,
    >,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let dt = time.delta_secs();

    let mut live_key_bases: std::collections::HashSet<String> = std::collections::HashSet::new();

    for (transform, physics, markers, helm, uuid, is_local) in ships_q.iter() {
        let key_base = match uuid {
            Some(u) => format!("engine:{}", u.0),
            None if is_local => "engine:player".to_string(),
            None => continue, // NPC without a UUID has no stable trail key.
        };
        live_key_bases.insert(key_base.clone());
        let max_speed = helm.map(|h| h.0.max_speed).unwrap_or(12.5).max(0.1);
        let normalized = (physics.forward_speed / max_speed).clamp(0.0, 1.0);
        let cfg = helm.and_then(|h| h.0.engine_pfx.as_ref());
        let settings = EnginePfxSettings::from_config(cfg);
        update_engine_trail(
            &key_base,
            transform,
            markers,
            cfg,
            normalized,
            dt,
            &settings,
            &mut state,
            &mut commands,
            &mut meshes,
            &mut materials,
        );
    }

    // Prune trail ribbon entities for ships that no longer exist in the world.
    // Emitter keys have the form "<key_base>:<emitter_idx>"; extract the base
    // and drop every entry whose ship is no longer in the query.
    let dead_keys: Vec<String> = state
        .emitters
        .keys()
        .filter(|key| {
            // Strip the trailing ":<emitter_idx>" suffix to recover the key_base.
            let base = key.rsplit_once(':').map(|x| x.0).unwrap_or(key.as_str());
            !live_key_bases.contains(base)
        })
        .cloned()
        .collect();
    for key in dead_keys {
        if let Some(trail) = state.emitters.remove(&key) {
            commands.entity(trail.entity).despawn();
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn update_engine_trail(
    key_base: &str,
    transform: &Transform,
    markers: Option<&ModelMarkers>,
    cfg: Option<&EnginePfxConfig>,
    normalized_speed: f32,
    dt: f32,
    settings: &EnginePfxSettings,
    state: &mut EngineTrailState,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
) {
    let emitters = engine_emitters(transform, markers, cfg);
    for (emitter_idx, (origin, direction)) in emitters.iter().enumerate() {
        let key = format!("{}:{}", key_base, emitter_idx);

        // Lazily create the ribbon entity and mesh for this emitter.
        if !state.emitters.contains_key(&key) {
            let mesh_handle = meshes.add(empty_ribbon_mesh());
            let mat_handle = trail_ribbon_material(materials, settings.color);
            let entity = commands
                .spawn((
                    PfxEntity,
                    Mesh3d(mesh_handle.clone()),
                    MeshMaterial3d(mat_handle),
                    Transform::default(),
                    bevy::camera::visibility::NoFrustumCulling,
                ))
                .id();
            state.emitters.insert(
                key.clone(),
                EmitterTrail {
                    crumbs: VecDeque::new(),
                    mesh_handle,
                    entity,
                },
            );
        }

        let trail = state.emitters.get_mut(&key).unwrap();

        // Age crumbs and drop expired ones from the tail.
        for crumb in trail.crumbs.iter_mut() {
            crumb.age += dt;
        }
        while trail
            .crumbs
            .back()
            .map(|c| c.age >= c.lifetime)
            .unwrap_or(false)
        {
            trail.crumbs.pop_back();
        }

        // Pin the ribbon head to the emitter origin; older crumbs form the
        // trail behind it.
        if normalized_speed > 0.05 {
            upsert_engine_head_crumb(
                &mut trail.crumbs,
                *origin,
                ENGINE_TRAIL_RADIUS * normalized_speed.max(0.35),
                settings.lifetime_secs,
            );
        }

        // Rebuild the ribbon mesh in place.
        if let Some(mesh) = meshes.get_mut(&trail.mesh_handle) {
            let render_crumbs = if normalized_speed > 0.05 {
                render_crumbs_from_marker(
                    &trail.crumbs,
                    *origin,
                    *direction,
                    ENGINE_TRAIL_RADIUS * normalized_speed.max(0.35),
                    settings.lifetime_secs,
                )
            } else {
                trail.crumbs.clone()
            };
            build_ribbon_into_mesh(mesh, &render_crumbs);
        }
    }
}

fn upsert_engine_head_crumb(
    crumbs: &mut VecDeque<TrailCrumb>,
    origin: Vec3,
    width: f32,
    lifetime: f32,
) {
    let should_insert = crumbs
        .front()
        .map(|c| c.pos.distance(origin) >= ENGINE_TRAIL_MIN_CRUMB_DIST)
        .unwrap_or(true);

    if should_insert {
        crumbs.push_front(TrailCrumb {
            pos: origin,
            width,
            age: 0.0,
            lifetime,
        });
    } else if let Some(front) = crumbs.front_mut() {
        front.pos = origin;
        front.width = width;
        front.age = 0.0;
        front.lifetime = lifetime;
    }

    while crumbs.len() > ENGINE_TRAIL_MAX_CRUMBS {
        crumbs.pop_back();
    }
}

fn render_crumbs_from_marker(
    crumbs: &VecDeque<TrailCrumb>,
    origin: Vec3,
    direction: Vec3,
    width: f32,
    lifetime: f32,
) -> VecDeque<TrailCrumb> {
    let mut render_crumbs = crumbs.clone();
    if let Some(front) = render_crumbs.front_mut() {
        front.pos = origin;
        front.width = width;
        front.age = 0.0;
        front.lifetime = lifetime;
    } else {
        render_crumbs.push_front(TrailCrumb {
            pos: origin,
            width,
            age: 0.0,
            lifetime,
        });
    }

    if render_crumbs.len() == 1 {
        let tail_dir = direction.normalize_or_zero();
        if tail_dir.length_squared() > 1e-6 {
            render_crumbs.push_back(TrailCrumb {
                pos: origin + tail_dir * ENGINE_TRAIL_MIN_CRUMB_DIST,
                width,
                age: 0.0,
                lifetime,
            });
        }
    }

    render_crumbs
}

fn empty_ribbon_mesh() -> Mesh {
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    let empty_pos: Vec<[f32; 3]> = vec![];
    let empty_uv: Vec<[f32; 2]> = vec![];
    let empty_col: Vec<[f32; 4]> = vec![];
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, empty_pos.clone());
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, empty_pos);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, empty_uv);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, empty_col);
    mesh.insert_indices(Indices::U32(vec![]));
    mesh
}

fn trail_ribbon_material(
    materials: &mut Assets<StandardMaterial>,
    color: [f32; 4],
) -> Handle<StandardMaterial> {
    materials.add(StandardMaterial {
        base_color: Color::srgba(color[0], color[1], color[2], color[3]),
        emissive: LinearRgba::new(color[0] * 3.0, color[1] * 3.0, color[2] * 3.0, 1.0),
        alpha_mode: AlphaMode::Blend,
        unlit: true,
        double_sided: true,
        cull_mode: None,
        ..default()
    })
}

/// Rebuilds the ribbon geometry in-place from the ordered breadcrumb deque.
/// crumbs[0] is the newest point (near the ship), crumbs[n-1] is the oldest.
fn build_ribbon_into_mesh(mesh: &mut Mesh, crumbs: &VecDeque<TrailCrumb>) {
    if crumbs.len() < 2 {
        let empty_pos: Vec<[f32; 3]> = vec![];
        let empty_uv: Vec<[f32; 2]> = vec![];
        let empty_col: Vec<[f32; 4]> = vec![];
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, empty_pos.clone());
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, empty_pos);
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, empty_uv);
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, empty_col);
        mesh.insert_indices(Indices::U32(vec![]));
        return;
    }

    let n = crumbs.len();
    let crumbs_slice: Vec<&TrailCrumb> = crumbs.iter().collect();

    let mut positions: Vec<[f32; 3]> = Vec::with_capacity(n * 2);
    let mut normals: Vec<[f32; 3]> = Vec::with_capacity(n * 2);
    let mut uvs: Vec<[f32; 2]> = Vec::with_capacity(n * 2);
    let mut colors: Vec<[f32; 4]> = Vec::with_capacity(n * 2);
    let mut indices: Vec<u32> = Vec::with_capacity((n - 1) * 6);

    for (i, crumb) in crumbs_slice.iter().enumerate() {
        // Central-difference tangent (direction toward newer crumb).
        let tangent = if i == 0 {
            (crumbs_slice[0].pos - crumbs_slice[1].pos).normalize_or_zero()
        } else if i == n - 1 {
            (crumbs_slice[n - 2].pos - crumbs_slice[n - 1].pos).normalize_or_zero()
        } else {
            (crumbs_slice[i - 1].pos - crumbs_slice[i + 1].pos).normalize_or_zero()
        };

        // Perpendicular with a strong Y component so the ribbon is vertical
        // (visible from the default behind/above camera angle).
        let perp = if tangent.length_squared() > 1e-6 {
            let tan = tangent.normalize();
            let h = tan.cross(Vec3::Y).normalize_or_zero();
            if h.length_squared() > 1e-6 {
                tan.cross(h).normalize_or_zero()
            } else {
                Vec3::X
            }
        } else {
            Vec3::X
        };

        let age_frac = (crumb.age / crumb.lifetime.max(0.001)).clamp(0.0, 1.0);
        let hw = crumb.width * 0.5;
        let base = Vec3::new(crumb.pos.x, crumb.pos.y + 0.05, crumb.pos.z);

        positions.push((base - perp * hw).to_array());
        positions.push((base + perp * hw).to_array());
        normals.push([0.0, 1.0, 0.0]);
        normals.push([0.0, 1.0, 0.0]);

        let u = i as f32 / (n - 1) as f32;
        uvs.push([u, 0.0]);
        uvs.push([u, 1.0]);

        let alpha = 1.0 - age_frac;
        colors.push([1.0, 1.0, 1.0, alpha]);
        colors.push([1.0, 1.0, 1.0, alpha]);

        // Two CCW triangles per quad (viewed from +Y).
        if i < n - 1 {
            let base_idx = (i * 2) as u32;
            indices.push(base_idx);
            indices.push(base_idx + 2);
            indices.push(base_idx + 3);
            indices.push(base_idx);
            indices.push(base_idx + 3);
            indices.push(base_idx + 1);
        }
    }

    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    mesh.insert_indices(Indices::U32(indices));
}

fn tick_lifetime_pfx(
    time: Res<Time>,
    mut commands: Commands,
    mut query: Query<(Entity, &mut PfxLifetime, Option<&PfxFadingMaterial>)>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let dt = time.delta_secs();
    for (entity, mut lifetime, fading) in query.iter_mut() {
        lifetime.age += dt;
        let remaining = 1.0 - (lifetime.age / lifetime.lifetime.max(0.001)).clamp(0.0, 1.0);
        if let Some(fading) = fading {
            if let Some(mat) = materials.get_mut(&fading.handle) {
                mat.base_color = Color::srgba(
                    fading.color[0],
                    fading.color[1],
                    fading.color[2],
                    fading.color[3] * remaining,
                );
                mat.emissive = LinearRgba::new(
                    fading.color[0] * fading.emissive_strength * remaining,
                    fading.color[1] * fading.emissive_strength * remaining,
                    fading.color[2] * fading.emissive_strength * remaining,
                    fading.color[3] * remaining,
                );
            }
        }
        if lifetime.age >= lifetime.lifetime {
            commands.entity(entity).despawn();
        }
    }
}

fn tick_bursts(time: Res<Time>, mut query: Query<(&PfxLifetime, &PfxBurst, &mut Transform)>) {
    for (lifetime, burst, mut transform) in query.iter_mut() {
        let t = (lifetime.age / lifetime.lifetime.max(0.001)).clamp(0.0, 1.0);
        let scale = burst.start_scale.lerp(burst.end_scale, t);
        transform.scale = Vec3::splat(scale);
        transform.rotate_y(time.delta_secs() * 3.0);
    }
}

fn cleanup_pfx(
    mut commands: Commands,
    query: Query<Entity, With<PfxEntity>>,
    mut beam_state: ResMut<BeamPfxState>,
    mut torpedo_state: ResMut<TorpedoPfxState>,
    mut engine_state: ResMut<EngineTrailState>,
) {
    for entity in query.iter() {
        commands.entity(entity).despawn();
    }
    beam_state.active.clear();
    beam_state.target_point_choices.clear();
    torpedo_state.active.clear();
    engine_state.emitters.clear();
}

fn choose_target_point_index(
    key: &str,
    target_point_count: usize,
    state: &mut BeamPfxState,
) -> Option<usize> {
    if target_point_count == 0 {
        state.target_point_choices.remove(key);
        return None;
    }

    if let Some(index) = state.target_point_choices.get(key).copied() {
        if index < target_point_count {
            return Some(index);
        }
    }

    let mut rng = rand::rng();
    let index = rng.random_range(0..target_point_count);
    state.target_point_choices.insert(key.to_string(), index);
    Some(index)
}

fn target_point_count(
    uuid: &str,
    local_ship_uuid: Option<&str>,
    entity_q: &Query<
        (&EntityUuid, &Transform, Option<&ModelMarkers>),
        (
            Without<Asteroid>,
            Without<BeamBody>,
            Without<BeamContactGlow>,
        ),
    >,
    local_ship_q: &Query<
        (&Transform, Option<&ModelMarkers>, Option<&EntityUuid>),
        (With<LocalShip>, Without<BeamBody>, Without<BeamContactGlow>),
    >,
) -> usize {
    if local_ship_uuid == Some(uuid) {
        return local_ship_q
            .single()
            .ok()
            .and_then(|(_, markers, _)| markers.map(ModelMarkers::target_point_count))
            .unwrap_or(0);
    }

    entity_q
        .iter()
        .find_map(|(u, _, markers)| {
            (u.0 == uuid).then(|| markers.map(ModelMarkers::target_point_count).unwrap_or(0))
        })
        .unwrap_or(0)
}

fn target_position(
    uuid: &str,
    shooter_transform: &Transform,
    local_ship_uuid: Option<&str>,
    target_point_index: Option<usize>,
    asteroid_q: &Query<
        (&AsteroidUuid, &Transform),
        (With<Asteroid>, Without<BeamBody>, Without<BeamContactGlow>),
    >,
    entity_q: &Query<
        (&EntityUuid, &Transform, Option<&ModelMarkers>),
        (
            Without<Asteroid>,
            Without<BeamBody>,
            Without<BeamContactGlow>,
        ),
    >,
    local_ship_q: &Query<
        (&Transform, Option<&ModelMarkers>, Option<&EntityUuid>),
        (With<LocalShip>, Without<BeamBody>, Without<BeamContactGlow>),
    >,
) -> Option<Vec3> {
    if local_ship_uuid == Some(uuid) {
        if let Some((transform, markers, _)) = local_ship_q.iter().next() {
            // Prefer a configured target point, but fall back to the
            // LocalShip's own translation (not the shooter's) when it has
            // none — matches the entity_q branch below. Previously this
            // fell through to `shooter_transform.translation` whenever
            // `target_point_position` returned `None`, which is the common
            // case (no `ModelMarkers` configured), making the beam's
            // endpoint lock onto the shooter instead of tracking the
            // LocalShip as it moved.
            return Some(
                target_point_position(transform, markers, target_point_index)
                    .unwrap_or(transform.translation),
            );
        }
        // Degenerate: no LocalShip entity exists in the world at all —
        // fall back to the shooter's own position as a last resort.
        return Some(shooter_transform.translation);
    }
    asteroid_q
        .iter()
        .find_map(|(u, t)| (u.0 == uuid).then_some(t.translation))
        .or_else(|| {
            entity_q.iter().find_map(|(u, t, markers)| {
                if u.0 == uuid {
                    Some(
                        target_point_position(t, markers, target_point_index)
                            .unwrap_or(t.translation),
                    )
                } else {
                    None
                }
            })
        })
}

fn target_point_position(
    transform: &Transform,
    markers: Option<&ModelMarkers>,
    target_point_index: Option<usize>,
) -> Option<Vec3> {
    let marker = markers?.target_point(target_point_index?)?;
    Some(transform.transform_point(Vec3::from_array(marker.position)))
}

fn marker_origin(
    transform: &Transform,
    markers: Option<&ModelMarkers>,
    marker_name: Option<&str>,
) -> Option<Vec3> {
    let marker = markers?.get(marker_name?)?;
    Some(transform.transform_point(Vec3::from_array(marker.position)))
}

fn marker_emitter(
    transform: &Transform,
    markers: Option<&ModelMarkers>,
    marker_name: Option<&str>,
) -> Option<(Vec3, Vec3)> {
    let marker = markers?.get(marker_name?)?;
    let origin = transform.transform_point(Vec3::from_array(marker.position));
    let direction = transform.rotation * Vec3::from_array(marker.direction);
    (direction.length_squared() > 1e-6).then_some((origin, direction.normalize()))
}

/// Bank-aware fallback beam origin when the bank has no named marker.
/// Positions the emitter around the ship's transform based on the bank's
/// facing angle (forward for fore banks, right/left for beam banks, etc.),
/// producing visually distinct emitter positions per bank.
///
/// Falls through to bare ship center when no bank config is available.
fn bank_fallback_origin(src_t: &Transform, bank: Option<&PhaserBankConfig>) -> Vec3 {
    let center = Vec3::new(src_t.translation.x, BEAM_Y_OFFSET, src_t.translation.z);
    let Some(bank) = bank else {
        return center;
    };
    // Recover yaw from the transform's rotation. `Transform::rotation` is the
    // authoritative attitude for both player and NPC ships — matches ship
    // rendering and the physics-integrator output.
    let (yaw, _pitch, _roll) = src_t.rotation.to_euler(bevy::math::EulerRot::YXZ);
    let forward = Vec3::new(yaw.sin(), 0.0, -yaw.cos());
    let right = Vec3::new(yaw.cos(), 0.0, yaw.sin());
    let facing = bank.facing_deg.to_radians();
    center + forward * facing.cos() * 3.0 + right * facing.sin() * beam_render::BANK_HULL_OFFSET
}

fn clamp_endpoint(start: Vec3, target: Vec3, range_origin: Vec3, max_range: f32) -> Vec3 {
    if (target - range_origin).length() <= max_range {
        target
    } else if max_range <= 0.0 {
        start
    } else {
        let delta = target - start;
        let dist = delta.length();
        if dist < 1e-6 {
            return target;
        }

        let dir = delta / dist;
        let from_center = start - range_origin;
        let b = from_center.dot(dir);
        let c = from_center.length_squared() - max_range * max_range;
        let discriminant = b * b - c;
        if discriminant < 0.0 {
            start
        } else {
            start + dir * (-b + discriminant.sqrt()).clamp(0.0, dist)
        }
    }
}

fn segment_transform(start: Vec3, end: Vec3, radius: f32) -> Transform {
    let delta = end - start;
    let length = delta.length().max(0.001);
    let dir = delta / length;
    Transform {
        translation: start + delta * 0.5,
        rotation: Quat::from_rotation_arc(Vec3::Y, dir),
        scale: Vec3::new(radius, length, radius),
    }
}

fn glow_material(
    materials: &mut Assets<StandardMaterial>,
    color: [f32; 4],
    emissive_strength: f32,
    alpha_mode: AlphaMode,
) -> Handle<StandardMaterial> {
    materials.add(StandardMaterial {
        base_color: Color::srgba(color[0], color[1], color[2], color[3]),
        emissive: LinearRgba::new(
            color[0] * emissive_strength,
            color[1] * emissive_strength,
            color[2] * emissive_strength,
            color[3],
        ),
        alpha_mode,
        unlit: true,
        ..default()
    })
}

fn spawn_trail_segment(
    start: Vec3,
    end: Vec3,
    radius: f32,
    color: [f32; 4],
    emissive_strength: f32,
    lifetime_secs: f32,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
) -> Entity {
    let mat = glow_material(materials, color, emissive_strength, AlphaMode::Blend);
    commands
        .spawn((
            PfxEntity,
            Mesh3d(meshes.add(Cylinder::new(1.0, 1.0))),
            MeshMaterial3d(mat.clone()),
            segment_transform(start, end, radius),
            PfxLifetime {
                age: 0.0,
                lifetime: lifetime_secs.max(0.05),
            },
            PfxFadingMaterial {
                handle: mat,
                color,
                emissive_strength,
            },
        ))
        .id()
}

fn spawn_torpedo_burst(
    pos: Vec3,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
) {
    let color = [1.0, 0.72, 0.16, 0.75];
    let mat = glow_material(materials, color, 10.0, AlphaMode::Add);
    commands.spawn((
        PfxEntity,
        Mesh3d(meshes.add(Sphere { radius: 1.0 })),
        MeshMaterial3d(mat.clone()),
        Transform::from_translation(pos).with_scale(Vec3::splat(0.25)),
        PfxLifetime {
            age: 0.0,
            lifetime: TORPEDO_BURST_LIFETIME_SECS,
        },
        PfxBurst {
            start_scale: 0.25,
            end_scale: 2.2,
        },
        PfxFadingMaterial {
            handle: mat,
            color,
            emissive_strength: 10.0,
        },
    ));
}

fn engine_emitters(
    transform: &Transform,
    markers: Option<&ModelMarkers>,
    cfg: Option<&EnginePfxConfig>,
) -> Vec<(Vec3, Vec3)> {
    let marker_emitters: Vec<(Vec3, Vec3)> = cfg
        .into_iter()
        .flat_map(|cfg| cfg.markers.iter())
        .filter_map(|name| marker_emitter(transform, markers, Some(name.as_str())))
        .collect();
    if !marker_emitters.is_empty() {
        return marker_emitters;
    }

    let forward = transform.rotation * Vec3::NEG_Z;
    let aft = -forward.normalize_or_zero();
    vec![(transform.translation + aft * 3.0, aft)]
}

struct EnginePfxSettings {
    color: [f32; 4],
    lifetime_secs: f32,
}

impl EnginePfxSettings {
    fn from_config(cfg: Option<&EnginePfxConfig>) -> Self {
        Self {
            color: cfg.and_then(|c| c.color).unwrap_or(ENGINE_DEFAULT_COLOR),
            lifetime_secs: cfg
                .and_then(|c| c.trail_lifetime_secs)
                .unwrap_or(ENGINE_TRAIL_CRUMB_LIFETIME_SECS)
                .max(0.05),
        }
    }
}

pub fn diff_torpedo_sets(
    in_flight_uuids: &HashSet<String>,
    tracked: &HashSet<String>,
) -> (Vec<String>, Vec<String>) {
    let to_spawn: Vec<String> = in_flight_uuids.difference(tracked).cloned().collect();
    let to_despawn: Vec<String> = tracked.difference(in_flight_uuids).cloned().collect();
    (to_spawn, to_despawn)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diff_torpedo_sets_spawns_new_uuids() {
        let in_flight: HashSet<String> = ["a".into(), "b".into()].into();
        let tracked: HashSet<String> = HashSet::new();
        let (to_spawn, to_despawn) = diff_torpedo_sets(&in_flight, &tracked);
        let mut to_spawn_sorted = to_spawn.clone();
        to_spawn_sorted.sort();
        assert_eq!(to_spawn_sorted, vec!["a".to_string(), "b".to_string()]);
        assert!(to_despawn.is_empty());
    }

    #[test]
    fn diff_torpedo_sets_despawns_removed_uuids() {
        let in_flight: HashSet<String> = HashSet::new();
        let tracked: HashSet<String> = ["a".into()].into();
        let (to_spawn, to_despawn) = diff_torpedo_sets(&in_flight, &tracked);
        assert!(to_spawn.is_empty());
        assert_eq!(to_despawn, vec!["a".to_string()]);
    }

    #[test]
    fn diff_torpedo_sets_no_change_when_same() {
        let in_flight: HashSet<String> = ["a".into()].into();
        let tracked: HashSet<String> = ["a".into()].into();
        let (to_spawn, to_despawn) = diff_torpedo_sets(&in_flight, &tracked);
        assert!(to_spawn.is_empty());
        assert!(to_despawn.is_empty());
    }

    #[test]
    fn diff_torpedo_sets_mixed_spawn_and_despawn() {
        let in_flight: HashSet<String> = ["b".into(), "c".into()].into();
        let tracked: HashSet<String> = ["a".into(), "b".into()].into();
        let (to_spawn, to_despawn) = diff_torpedo_sets(&in_flight, &tracked);
        assert_eq!(to_spawn, vec!["c".to_string()]);
        assert_eq!(to_despawn, vec!["a".to_string()]);
    }

    #[test]
    fn engine_pfx_settings_uses_renderer_defaults_for_sparse_config() {
        let cfg = EnginePfxConfig::default();
        let settings = EnginePfxSettings::from_config(Some(&cfg));
        assert_eq!(settings.color, ENGINE_DEFAULT_COLOR);
        assert_eq!(settings.lifetime_secs, ENGINE_TRAIL_CRUMB_LIFETIME_SECS);
    }

    #[test]
    fn engine_pfx_settings_uses_configured_values() {
        let cfg = EnginePfxConfig {
            color: Some([0.1, 0.2, 0.3, 0.4]),
            markers: vec![],
            trail_lifetime_secs: Some(0.8),
            trail_spawn_interval_secs: Some(0.03),
        };
        let settings = EnginePfxSettings::from_config(Some(&cfg));
        assert_eq!(settings.color, [0.1, 0.2, 0.3, 0.4]);
        assert_eq!(settings.lifetime_secs, 0.8);
    }

    #[test]
    fn clamp_endpoint_returns_target_inside_center_range() {
        let start = Vec3::new(4.0, 0.0, 0.0);
        let target = Vec3::new(0.0, 0.0, -40.0);
        let range_origin = Vec3::ZERO;

        assert_eq!(clamp_endpoint(start, target, range_origin, 50.0), target);
    }

    #[test]
    fn clamp_endpoint_limits_ray_to_centered_range_sphere() {
        let start = Vec3::new(4.0, 0.0, 0.0);
        let target = Vec3::new(4.0, 0.0, -80.0);
        let range_origin = Vec3::ZERO;
        let endpoint = clamp_endpoint(start, target, range_origin, 50.0);

        assert!((endpoint.x - 4.0).abs() < 1e-5);
        assert!((endpoint.z - -49.839745).abs() < 1e-4);
        assert!((endpoint.distance(range_origin) - 50.0).abs() < 1e-4);
    }

    #[test]
    fn target_point_position_transforms_model_point() {
        let rig = crate::model_rig::ModelRig::from_toml(
            r#"
[[target_points]]
position = [0.5, -0.1, 0.25]
"#,
        )
        .unwrap();
        let markers = ModelMarkers::from_rig(&rig);
        let transform = Transform::from_translation(Vec3::new(10.0, 2.0, -3.0));

        let point = target_point_position(&transform, Some(&markers), Some(0)).unwrap();

        assert_eq!(point, Vec3::new(10.5, 1.9, -2.75));
    }

    #[test]
    fn target_point_choice_stays_stable_for_live_beam_key() {
        let mut state = BeamPfxState::default();
        let first = choose_target_point_index("beam:a", 3, &mut state).unwrap();
        let second = choose_target_point_index("beam:a", 3, &mut state).unwrap();

        assert_eq!(first, second);
        assert!(first < 3);

        assert_eq!(choose_target_point_index("beam:a", 0, &mut state), None);
        assert!(state.target_point_choices.is_empty());
    }

    #[test]
    fn segment_transform_places_midpoint_and_scales_height() {
        let transform = segment_transform(Vec3::ZERO, Vec3::new(0.0, 4.0, 0.0), 0.25);
        assert_eq!(transform.translation, Vec3::new(0.0, 2.0, 0.0));
        assert_eq!(transform.scale, Vec3::new(0.25, 4.0, 0.25));
    }

    #[test]
    fn upsert_engine_head_crumb_pins_existing_head_to_marker() {
        let mut crumbs = VecDeque::from([
            TrailCrumb {
                pos: Vec3::new(0.02, 0.0, 0.0),
                width: 0.2,
                age: 0.2,
                lifetime: 1.0,
            },
            TrailCrumb {
                pos: Vec3::new(0.0, 0.0, 0.5),
                width: 0.2,
                age: 0.4,
                lifetime: 1.0,
            },
        ]);

        upsert_engine_head_crumb(&mut crumbs, Vec3::ZERO, 0.5, 1.5);

        assert_eq!(crumbs.len(), 2);
        assert_eq!(crumbs[0].pos, Vec3::ZERO);
        assert_eq!(crumbs[0].width, 0.5);
        assert_eq!(crumbs[0].age, 0.0);
        assert_eq!(crumbs[1].pos, Vec3::new(0.0, 0.0, 0.5));
    }

    #[test]
    fn render_crumbs_from_marker_adds_backward_tail_for_new_trail() {
        let crumbs = VecDeque::from([TrailCrumb {
            pos: Vec3::ZERO,
            width: 0.5,
            age: 0.0,
            lifetime: 1.5,
        }]);

        let render = render_crumbs_from_marker(&crumbs, Vec3::ZERO, Vec3::Z, 0.5, 1.5);

        assert_eq!(render.len(), 2);
        assert_eq!(render[0].pos, Vec3::ZERO);
        assert_eq!(render[1].pos, Vec3::Z * ENGINE_TRAIL_MIN_CRUMB_DIST);
    }

    #[test]
    fn engine_emitters_use_marker_direction() {
        let mut map = HashMap::new();
        map.insert(
            "aft_exhaust".to_string(),
            crate::model_rig::Marker {
                position: [1.0, 0.5, -2.0],
                direction: [0.0, 0.0, -1.0],
            },
        );
        let markers = ModelMarkers::from_markers(map);
        let cfg = EnginePfxConfig {
            color: None,
            markers: vec!["aft_exhaust".to_string()],
            trail_lifetime_secs: None,
            trail_spawn_interval_secs: None,
        };

        let emitters = engine_emitters(
            &Transform::from_translation(Vec3::new(10.0, 0.0, 20.0)),
            Some(&markers),
            Some(&cfg),
        );

        assert_eq!(emitters.len(), 1);
        assert_eq!(emitters[0].0, Vec3::new(11.0, 0.5, 18.0));
        assert_eq!(emitters[0].1, Vec3::NEG_Z);
    }

    #[test]
    fn engine_emitters_rotate_marker_direction_with_ship() {
        let mut map = HashMap::new();
        map.insert(
            "aft_exhaust".to_string(),
            crate::model_rig::Marker {
                position: [0.0, 0.0, 0.0],
                direction: [0.0, 0.0, -1.0],
            },
        );
        let markers = ModelMarkers::from_markers(map);
        let cfg = EnginePfxConfig {
            color: None,
            markers: vec!["aft_exhaust".to_string()],
            trail_lifetime_secs: None,
            trail_spawn_interval_secs: None,
        };
        let transform =
            Transform::from_rotation(Quat::from_rotation_y(std::f32::consts::FRAC_PI_2));

        let emitters = engine_emitters(&transform, Some(&markers), Some(&cfg));

        let expected = transform.rotation * Vec3::NEG_Z;
        assert!((emitters[0].1 - expected).length() < 1e-6);
    }

    #[test]
    fn engine_emitters_fallback_points_aft_from_ship_forward() {
        let emitters = engine_emitters(
            &Transform::from_translation(Vec3::new(1.0, 0.0, 2.0)),
            None,
            None,
        );

        assert_eq!(emitters.len(), 1);
        assert_eq!(emitters[0].0, Vec3::new(1.0, 0.0, 5.0));
        assert_eq!(emitters[0].1, Vec3::Z);
    }

    // ── Integration: does the beam PFX actually track ship movement? ──────
    //
    // The pure-function tests above all pass in isolation, but the reported
    // bug ("beam frozen when attacker/target move") is a whole-schedule
    // ordering hazard: `sync_phaser_beams` reads ship `Transform`, which is
    // written by `sync_ship_position` (in `ShipPlugin`), which in turn must
    // run *after* whatever system computes this tick's `ShipPhysics`.
    //
    // These tests drive movement through the REAL production pipeline
    // (`process_helm_inputs`, via a constant `HelmInput` admitted command on
    // a `LocalShip`) rather than a synthetic mover system. A synthetic mover
    // can only be pinned to a shared *label* (e.g. `AiTickLabel`) from
    // outside `ship_plugin.rs`, since the real writer/reader systems are
    // private to that module — and two systems that both merely reference
    // the same label, without an edge *between* them, have no defined
    // relative order (this was tried and silently passed for the wrong
    // reason). Driving `process_helm_inputs` for real gets the exact,
    // explicit `.after(process_helm_inputs)` edge on `sync_ship_position`
    // for free, with no privacy workarounds needed.
    fn thrust_command() -> crate::messages::AdmittedCommands {
        crate::messages::AdmittedCommands(vec![crate::messages::AdmittedCommand {
            target: crate::system_registry::helm_system_id(),
            payload: crate::messages::SystemControlPayload::HelmInput {
                thrust: 1.0,
                steering: 0.0,
            },
            response_token: None,
        }])
    }

    fn beam_test_app() -> App {
        let mut app = App::new();
        app.configure_sets(
            Update,
            (
                crate::sim_sets::SimSet::Input,
                crate::sim_sets::SimSet::Physics,
                crate::sim_sets::SimSet::Damage,
                crate::sim_sets::SimSet::Modifiers,
                crate::sim_sets::SimSet::Publish,
                crate::sim_sets::SimSet::PublishAggregate,
                crate::sim_sets::SimSet::Broadcast,
            )
                .chain(),
        )
        .add_plugins(crate::lobby::LobbyPlugin)
        .add_plugins(bevy::time::TimePlugin)
        .add_plugins(bevy::asset::AssetPlugin::default())
        .init_asset::<Mesh>()
        .init_asset::<StandardMaterial>()
        .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_millis(34),
        ))
        .init_resource::<crate::messages::InterSystemQueue>()
        .insert_resource(PhaserRenderConfig {
            beam_range: 1000.0,
            ..Default::default()
        })
        .insert_resource(crate::weapons_plugin::PhaserCombatConfigResource(
            crate::entity_config::PhaserCombatConfig { banks: vec![] },
        ))
        .add_plugins(crate::ship_plugin::ShipPlugin)
        .add_plugins(super::PfxPlugin);

        app.world_mut()
            .insert_resource(State::new(GamePhase::InProgress));
        app
    }

    fn beam_body_translation(app: &mut App) -> Vec3 {
        let mut q = app
            .world_mut()
            .query_filtered::<&Transform, With<BeamBody>>();
        q.single(app.world())
            .expect("BeamBody entity must exist")
            .translation
    }

    #[test]
    fn beam_transform_tracks_target_ship_physics_movement_across_ticks() {
        let mut app = beam_test_app();

        let target_uuid = "target-uuid-1".to_string();

        app.world_mut().spawn((
            crate::server_app::Ship,
            EntityUuid("shooter-uuid-1".to_string()),
            Transform::from_xyz(0.0, 0.0, 0.0),
            ShipPhysics::default(),
            ActiveBeam {
                target_uuid: Some(target_uuid.clone()),
                remaining_secs: 10.0,
                damage_accumulator: 0.0,
                bank: None,
            },
        ));

        let target = app
            .world_mut()
            .spawn((
                LocalShip,
                EntityUuid(target_uuid),
                Transform::from_xyz(0.0, 0.0, -10.0),
                ShipPhysics {
                    z: -10.0,
                    ..Default::default()
                },
                thrust_command(),
                crate::ship_plugin::ShipSystemControlSources::default(),
                crate::ship_plugin::LastHelmInput::default(),
            ))
            .id();

        // Precise check (catches a same-tick-stale ordering bug, not just a
        // hard freeze): after every tick, the beam midpoint must reflect
        // THIS tick's `ShipPhysics.z` (ground truth, read directly), not the
        // previous tick's. A `sync_ship_position` ordered before the system
        // that writes `ShipPhysics` this tick would make the beam trail by
        // exactly one tick's movement -- a loose "did it move at all?" check
        // would not catch that, since a laggy beam still moves every tick.
        for _ in 0..5 {
            app.update();
            let ground_truth_z = app.world().get::<ShipPhysics>(target).unwrap().z;
            let expected_mid_z = ground_truth_z / 2.0; // shooter stays at z=0
            let actual_mid_z = beam_body_translation(&mut app).z;
            assert!(
                (actual_mid_z - expected_mid_z).abs() < 0.01,
                "beam midpoint.z={actual_mid_z} should match this tick's target \
                 position (expected {expected_mid_z}, ground-truth target.z={ground_truth_z}) \
                 -- beam is reading a stale (last-tick) Transform"
            );
        }
    }

    #[test]
    fn beam_transform_tracks_shooter_ship_physics_movement_across_ticks() {
        let mut app = beam_test_app();

        let target_uuid = "target-uuid-2".to_string();

        let shooter = app
            .world_mut()
            .spawn((
                crate::server_app::Ship,
                LocalShip,
                EntityUuid("shooter-uuid-2".to_string()),
                Transform::from_xyz(0.0, 0.0, 0.0),
                ShipPhysics::default(),
                ActiveBeam {
                    target_uuid: Some(target_uuid.clone()),
                    remaining_secs: 10.0,
                    damage_accumulator: 0.0,
                    bank: None,
                },
                thrust_command(),
                crate::ship_plugin::ShipSystemControlSources::default(),
                crate::ship_plugin::LastHelmInput::default(),
            ))
            .id();

        app.world_mut().spawn((
            EntityUuid(target_uuid),
            Transform::from_xyz(0.0, 0.0, -10.0),
            ShipPhysics {
                z: -10.0,
                ..Default::default()
            },
        ));

        // Same precise per-tick check as the target-movement test above, but
        // with the roles reversed: the shooter is the one being moved.
        for _ in 0..5 {
            app.update();
            let ground_truth_z = app.world().get::<ShipPhysics>(shooter).unwrap().z;
            let expected_mid_z = (ground_truth_z + (-10.0)) / 2.0; // target stays at z=-10
            let actual_mid_z = beam_body_translation(&mut app).z;
            assert!(
                (actual_mid_z - expected_mid_z).abs() < 0.01,
                "beam midpoint.z={actual_mid_z} should match this tick's shooter \
                 position (expected {expected_mid_z}, ground-truth shooter.z={ground_truth_z}) \
                 -- beam is reading a stale (last-tick) Transform"
            );
        }
    }
}
