use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use std::collections::{HashMap, HashSet, VecDeque};

use crate::ai_plugin::{AiControllerComponent, EntityPhaserState};
use crate::beam_render;
use crate::entity_config::{EnginePfxConfig, PhaserBankConfig, PhaserCombatConfig};
use crate::entity_spawner::{EntityUuid, HelmConsoleSection, WeaponsConsoleSection};
use crate::messages::GamePhase;
use crate::model_rig::ModelMarkers;
use crate::ship_state::ShipState;
use crate::simulation::{
    ActiveBeam, Asteroid, AsteroidUuid, PhaserRenderConfig, Ship, TorpedoSystemResource,
};
use crate::weapons_plugin::PhaserCombatConfigResource;

const BEAM_RADIUS: f32 = 0.04;
const BEAM_Y_OFFSET: f32 = 0.0;
const CONTACT_GLOW_RADIUS: f32 = 0.45;

const TORPEDO_RADIUS: f32 = 0.45;
const TORPEDO_TRAIL_RADIUS: f32 = 0.18;
const TORPEDO_TRAIL_LIFETIME_SECS: f32 = 0.32;
const TORPEDO_TRAIL_MIN_DISTANCE: f32 = 0.35;
const TORPEDO_BURST_LIFETIME_SECS: f32 = 0.35;

const ENGINE_DEFAULT_COLOR: [f32; 4] = [0.25, 0.75, 1.0, 0.72];
const ENGINE_TRAIL_RADIUS: f32 = 0.22;
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
                ),
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
}

struct TorpedoEntities {
    body: Entity,
    last_pos: Vec3,
}

#[derive(Resource, Default)]
struct TorpedoPfxState {
    active: HashMap<String, TorpedoEntities>,
}

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

fn sync_phaser_beams(
    ship: Res<ShipState>,
    beam: Res<ActiveBeam>,
    render_cfg: Res<PhaserRenderConfig>,
    combat_cfg: Res<PhaserCombatConfigResource>,
    asteroid_q: Query<
        (&AsteroidUuid, &Transform),
        (With<Asteroid>, Without<BeamBody>, Without<BeamContactGlow>),
    >,
    entity_q: Query<
        (&EntityUuid, &Transform),
        (
            Without<Asteroid>,
            Without<BeamBody>,
            Without<BeamContactGlow>,
        ),
    >,
    player_ship_q: Query<
        (&Transform, Option<&ModelMarkers>, Option<&EntityUuid>),
        (With<Ship>, Without<BeamBody>, Without<BeamContactGlow>),
    >,
    npc_beam_q: Query<
        (
            &EntityUuid,
            &Transform,
            Option<&ModelMarkers>,
            &EntityPhaserState,
            Option<&WeaponsConsoleSection>,
        ),
        (Without<BeamBody>, Without<BeamContactGlow>),
    >,
    mut state: ResMut<BeamPfxState>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut body_q: Query<&mut Transform, (With<BeamBody>, Without<BeamContactGlow>)>,
    mut glow_q: Query<&mut Transform, (With<BeamContactGlow>, Without<BeamBody>)>,
) {
    let mut live_keys = HashSet::new();

    if let Some(target_uuid) = &beam.target_uuid {
        if let Some((start, end, color)) = resolve_player_beam(
            target_uuid,
            &ship,
            &beam,
            &render_cfg,
            &combat_cfg.0,
            &asteroid_q,
            &entity_q,
            &player_ship_q,
        ) {
            let key = format!(
                "player:{}:{}",
                beam.bank.as_ref().map(|b| b.as_str()).unwrap_or("default"),
                target_uuid
            );
            live_keys.insert(key.clone());
            upsert_beam(
                key,
                start,
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
    }

    let player_ship_uuid = player_ship_q
        .single()
        .ok()
        .and_then(|(_, _, uuid)| uuid.map(|u| u.0.clone()));

    for (src_uuid, src_t, src_markers, phaser, weapons) in npc_beam_q.iter() {
        if !phaser.beam_active {
            continue;
        }
        let Some(target_uuid) = phaser.beam_target else {
            continue;
        };
        let target_uuid = target_uuid.to_string();
        let Some(target_pos) = target_position(
            &target_uuid,
            &ship,
            player_ship_uuid.as_deref(),
            &asteroid_q,
            &entity_q,
        ) else {
            continue;
        };

        let bank = weapons.and_then(|w| w.0.phaser_banks.first());
        let color = bank
            .map(|b| beam_render::resolve_beam_color(&b.beam_color))
            .unwrap_or(beam_render::DEFAULT_BEAM_COLOR);
        let range = bank
            .map(|b| b.beam_range)
            .filter(|r| *r > 0.0)
            .unwrap_or(PhaserCombatConfig::DEFAULT_PHASER_RANGE);
        let origin = bank
            .and_then(|b| marker_origin(src_t, src_markers, b.marker.as_deref()))
            .unwrap_or(src_t.translation + Vec3::new(0.0, BEAM_Y_OFFSET, 0.0));
        let end = clamp_endpoint(origin, target_pos, src_t.translation, range);

        let key = format!("npc:{}:{}", src_uuid.0, target_uuid);
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
            commands.entity(entities.body).despawn();
            commands.entity(entities.glow).despawn();
        }
    }
}

fn resolve_player_beam(
    target_uuid: &str,
    ship: &ShipState,
    beam: &ActiveBeam,
    render_cfg: &PhaserRenderConfig,
    combat_cfg: &PhaserCombatConfig,
    asteroid_q: &Query<
        (&AsteroidUuid, &Transform),
        (With<Asteroid>, Without<BeamBody>, Without<BeamContactGlow>),
    >,
    entity_q: &Query<
        (&EntityUuid, &Transform),
        (
            Without<Asteroid>,
            Without<BeamBody>,
            Without<BeamContactGlow>,
        ),
    >,
    player_ship_q: &Query<
        (&Transform, Option<&ModelMarkers>, Option<&EntityUuid>),
        (With<Ship>, Without<BeamBody>, Without<BeamContactGlow>),
    >,
) -> Option<(Vec3, Vec3, [f32; 4])> {
    let target_pos = target_position(target_uuid, ship, None, asteroid_q, entity_q)?;
    let bank_id = beam.bank.as_deref();
    let bank_config = bank_id.and_then(|id| combat_cfg.bank_by_id(id));
    let color = bank_config
        .map(|b| beam_render::resolve_beam_color(&b.beam_color))
        .unwrap_or(render_cfg.beam_color);
    let range = beam
        .bank
        .as_deref()
        .and_then(|id| combat_cfg.bank_by_id(id))
        .map(|b| b.beam_range)
        .filter(|r| *r > 0.0)
        .unwrap_or(render_cfg.beam_range);

    let origin = if let Ok((transform, markers, _)) = player_ship_q.single() {
        bank_config
            .and_then(|b| marker_origin(transform, markers, b.marker.as_deref()))
            .unwrap_or_else(|| player_bank_fallback_origin(ship, bank_config))
    } else {
        player_bank_fallback_origin(ship, bank_config)
    };
    let range_origin = Vec3::new(ship.x, BEAM_Y_OFFSET, ship.z);
    let end = clamp_endpoint(origin, target_pos, range_origin, range);
    Some((origin, end, color))
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

fn sync_torpedo_pfx(
    torpedo_sys: Option<Res<TorpedoSystemResource>>,
    mut state: ResMut<TorpedoPfxState>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut transforms: Query<&mut Transform, With<TorpedoBody>>,
) {
    let Some(torpedo_sys) = torpedo_sys else {
        return;
    };

    let in_flight = &torpedo_sys.0.in_flight;
    let live: HashSet<String> = in_flight.iter().map(|t| t.uuid.clone()).collect();
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
        if let Some(t) = in_flight.iter().find(|t| t.uuid == uuid) {
            let pos = Vec3::new(t.x, 0.1, t.z);
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

    for t in in_flight {
        let pos = Vec3::new(t.x, 0.1, t.z);
        if let Some(entities) = state.active.get_mut(&t.uuid) {
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

fn spawn_engine_trails(
    time: Res<Time>,
    ship: Res<ShipState>,
    mut state: ResMut<EngineTrailState>,
    player_q: Query<
        (
            &Transform,
            Option<&ModelMarkers>,
            Option<&HelmConsoleSection>,
            Option<&EntityUuid>,
        ),
        With<Ship>,
    >,
    npc_q: Query<
        (
            &Transform,
            Option<&ModelMarkers>,
            Option<&HelmConsoleSection>,
            Option<&EntityUuid>,
            &AiControllerComponent,
        ),
        Without<Ship>,
    >,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let dt = time.delta_secs();

    if let Ok((transform, markers, helm, uuid)) = player_q.single() {
        let key_base = uuid
            .map(|u| format!("engine:{}", u.0))
            .unwrap_or_else(|| "engine:player".to_string());
        let max_speed = helm.map(|h| h.0.max_speed).unwrap_or(12.5).max(0.1);
        let normalized = (ship.forward_speed / max_speed).clamp(0.0, 1.0);
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

    for (transform, markers, helm, uuid, ai) in npc_q.iter() {
        let Some(uuid) = uuid else {
            continue;
        };
        let key_base = format!("engine:{}", uuid.0);
        let max_speed = helm.map(|h| h.0.max_speed).unwrap_or(12.5).max(0.1);
        let normalized = (ai.forward_speed / max_speed).clamp(0.0, 1.0);
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
    for (emitter_idx, (origin, _dir)) in emitters.iter().enumerate() {
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
        while trail.crumbs.back().map(|c| c.age >= c.lifetime).unwrap_or(false) {
            trail.crumbs.pop_back();
        }

        // Push a new crumb at the emitter origin if the ship has moved enough.
        if normalized_speed > 0.05 {
            let far_enough = trail
                .crumbs
                .front()
                .map(|c| c.pos.distance(*origin) >= ENGINE_TRAIL_MIN_CRUMB_DIST)
                .unwrap_or(true);
            if far_enough {
                trail.crumbs.push_front(TrailCrumb {
                    pos: *origin,
                    width: ENGINE_TRAIL_RADIUS * normalized_speed.max(0.35),
                    age: 0.0,
                    lifetime: settings.lifetime_secs,
                });
                if trail.crumbs.len() > ENGINE_TRAIL_MAX_CRUMBS {
                    trail.crumbs.pop_back();
                }
            }
        }

        // Rebuild the ribbon mesh in place.
        if let Some(mesh) = meshes.get_mut(&trail.mesh_handle) {
            build_ribbon_into_mesh(mesh, &trail.crumbs);
        }
    }
}

fn empty_ribbon_mesh() -> Mesh {
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD);
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

        // Perpendicular in the XZ plane for ribbon width.
        let perp = if tangent.length_squared() > 1e-6 {
            Vec3::new(-tangent.z, 0.0, tangent.x).normalize_or_zero()
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

        // Alpha fades with age.
        let alpha = (1.0 - age_frac) * crumb.age.min(0.05) / 0.05; // brief pop-in guard
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
    torpedo_state.active.clear();
    engine_state.emitters.clear();
}

fn target_position(
    uuid: &str,
    ship: &ShipState,
    player_ship_uuid: Option<&str>,
    asteroid_q: &Query<
        (&AsteroidUuid, &Transform),
        (With<Asteroid>, Without<BeamBody>, Without<BeamContactGlow>),
    >,
    entity_q: &Query<
        (&EntityUuid, &Transform),
        (
            Without<Asteroid>,
            Without<BeamBody>,
            Without<BeamContactGlow>,
        ),
    >,
) -> Option<Vec3> {
    if player_ship_uuid == Some(uuid) {
        return Some(Vec3::new(ship.x, 0.0, ship.z));
    }
    asteroid_q
        .iter()
        .find_map(|(u, t)| (u.0 == uuid).then_some(t.translation))
        .or_else(|| {
            entity_q
                .iter()
                .find_map(|(u, t)| (u.0 == uuid).then_some(t.translation))
        })
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

fn player_bank_fallback_origin(ship: &ShipState, bank: Option<&PhaserBankConfig>) -> Vec3 {
    let center = Vec3::new(ship.x, BEAM_Y_OFFSET, ship.z);
    let forward = Vec3::new(ship.yaw.sin(), 0.0, -ship.yaw.cos());
    let right = Vec3::new(ship.yaw.cos(), 0.0, ship.yaw.sin());
    let Some(bank) = bank else {
        return center;
    };
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
    fn segment_transform_places_midpoint_and_scales_height() {
        let transform = segment_transform(Vec3::ZERO, Vec3::new(0.0, 4.0, 0.0), 0.25);
        assert_eq!(transform.translation, Vec3::new(0.0, 2.0, 0.0));
        assert_eq!(transform.scale, Vec3::new(0.25, 4.0, 0.25));
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
        let markers = ModelMarkers(map);
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
        let markers = ModelMarkers(map);
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
}
