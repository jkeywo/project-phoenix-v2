// Presentation-only particle/render effects: never feeds simulation state, so
// platform-varying std transcendentals are fine here (issue #908, simmath.rs),
// and so is the OS-entropy `rand::rng()` this module draws from for particle
// variation (issue #903) — nothing here is read back into sim state.
#![allow(clippy::disallowed_methods)]

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::reflect::TypePath;
use bevy::render::render_resource::AsBindGroup;
use bevy::shader::ShaderRef;
use rand::Rng;
use std::collections::{HashMap, HashSet, VecDeque};

use crate::console::weapons::PhaserCombatConfigResource;
use crate::console::weapons::{BlasterSystemResource, ShipDestroyedVfx};
use crate::core::messages::GamePhase;
use crate::entities::config::{EnginePfxConfig, PhaserBankConfig};
use crate::entities::model_rig::ModelMarkers;
use crate::entities::spawner::{EntityUuid, HelmConsoleSection};
use crate::render_setup::GameCamera;
use crate::server_app::{
    ActiveBeam, Asteroid, AsteroidUuid, LocalShip, PhaserRenderConfig, TorpedoSystemResource,
};
use crate::ship::state::ShipPhysics;
use crate::weapons::beam_render;

const BEAM_Y_OFFSET: f32 = 0.0;

// Textured phaser beam layers (issue: phaser-pfx-replacement). Widths are
// "radius" values fed into `segment_transform`, matching the old cylinder
// convention (final half-width = radius, since the unit ribbon mesh spans
// -1..1 in local X).
//
// Narrowed glow / tightened + brightened core (issue phaser-pfx-core-beam):
// the pre-existing core layer was wide enough and dim enough relative to the
// glow around it that the beam read as one soft diffused streak rather than
// a hot core inside a haze. The glow is now the halo, not the body.
const BEAM_GLOW_WIDTH: f32 = 0.22;
const BEAM_CORE_WIDTH: f32 = 0.045;
const CONTACT_GLOW_SIZE: f32 = 0.5;

// The single texture column the phaser beam samples along its whole length
// (issue #938, see `beam_ribbon_quad_mesh`). 0.5 is the horizontal centre of
// `beam_glow.png` / `beam_core.png` — the widest, brightest cross-section of
// the lens shape those textures are authored as. This is not a designer knob:
// it names the middle of a texture, the same way the ribbon quad's vertex
// positions name its corners.
const BEAM_PROFILE_U: f32 = 0.5;

// Contact-glow pulse at the target (issue phaser-pfx-core-beam): a static
// glow at the impact point reads as inert; a rhythmic brightness/size pulse
// sells continuous energy transfer into the target instead. Phase is
// randomized per beam at spawn (see `upsert_beam`) so simultaneous hits don't
// pulse in lockstep — presentation-only, so the OS-entropy `rand::rng()` this
// file already draws from (issue #903) is fine here too.
const CONTACT_PULSE_HZ: f32 = 3.4;
const CONTACT_PULSE_SCALE_MIN: f32 = 0.7;
const CONTACT_PULSE_SCALE_MAX: f32 = 1.45;
const CONTACT_PULSE_EMISSIVE_MIN: f32 = 0.6;
const CONTACT_PULSE_EMISSIVE_MAX: f32 = 1.6;
const CONTACT_GLOW_EMISSIVE_STRENGTH: f32 = 6.0;

const MUZZLE_FLASH_LIFETIME_SECS: f32 = 0.12;
const MUZZLE_FLASH_START_SIZE: f32 = 0.15;
const MUZZLE_FLASH_END_SIZE: f32 = 0.5;

const IMPACT_RING_LIFETIME_SECS: f32 = 0.35;
const IMPACT_RING_START_SIZE: f32 = 0.15;
const IMPACT_RING_END_SIZE: f32 = 0.9;

const IMPACT_SPARK_LIFETIME_SECS: f32 = 0.25;
const IMPACT_SPARK_SIZE: f32 = 0.35;
const IMPACT_SPARK_COUNT: usize = 4;
const IMPACT_SPARK_SPREAD: f32 = 0.6;

// Photon torpedo visuals (issue #826; textured core+shell+flare replacing
// the flat-sphere placeholder, matching the blaster/explosion PFX pattern).
const TORPEDO_TRAIL_RADIUS: f32 = 0.18;
const TORPEDO_TRAIL_LIFETIME_SECS: f32 = 0.32;
const TORPEDO_TRAIL_MIN_DISTANCE: f32 = 0.35;
const TORPEDO_COLOR: [f32; 4] = [1.0, 0.55, 0.12, 1.0];
const TORPEDO_CORE_COLOR: [f32; 4] = [1.0, 0.95, 0.85, 1.0];

const TORPEDO_CORE_SIZE: f32 = 0.3;
const TORPEDO_CORE_EMISSIVE: f32 = 9.0;
const TORPEDO_SHELL_SIZE: f32 = 0.7;
const TORPEDO_SHELL_EMISSIVE: f32 = 3.5;
const TORPEDO_FLARE_LENGTH: f32 = 1.4;
const TORPEDO_FLARE_WIDTH: f32 = 0.4;
const TORPEDO_FLARE_EMISSIVE: f32 = 3.0;

const TORPEDO_LAUNCH_FLASH_LIFETIME_SECS: f32 = 0.12;
const TORPEDO_LAUNCH_FLASH_START_SIZE: f32 = 0.25;
const TORPEDO_LAUNCH_FLASH_END_SIZE: f32 = 0.9;

const TORPEDO_IMPACT_FLASH_LIFETIME_SECS: f32 = 0.08;
const TORPEDO_IMPACT_FLASH_START_SIZE: f32 = 0.2;
const TORPEDO_IMPACT_FLASH_END_SIZE: f32 = 0.55;

const TORPEDO_IMPACT_PLASMA_LIFETIME_SECS: f32 = 0.6;
const TORPEDO_IMPACT_PLASMA_START_SCALE: f32 = 0.4;
const TORPEDO_IMPACT_PLASMA_END_SCALE: f32 = 1.6;

const TORPEDO_IMPACT_RING_LIFETIME_SECS: f32 = 0.4;
const TORPEDO_IMPACT_RING_START_SCALE: f32 = 0.2;
const TORPEDO_IMPACT_RING_END_SCALE: f32 = 2.2;

const TORPEDO_IMPACT_SPARK_COUNT: usize = 8;
const TORPEDO_IMPACT_SPARK_LIFETIME_SECS: f32 = 0.35;
const TORPEDO_IMPACT_SPARK_SCALE: f32 = 0.3;
const TORPEDO_IMPACT_SPARK_SPREAD: f32 = 0.6;

// Blaster projectile visuals (issue #638; textured crossed-quad bolt
// replacing the earlier flat sphere placeholder).
const BLASTER_SPHERE_VISUAL_SCALE_THRESHOLD: f32 = 1.5;
const BLASTER_BOLT_COLOR: [f32; 4] = [0.3, 0.8, 1.0, 1.0];
const BLASTER_SPHERE_COLOR: [f32; 4] = [1.0, 0.4, 0.05, 1.0];
const BLASTER_EMISSIVE: f32 = 5.0;

// Bolt mesh proportions: half-length feeds `segment_transform` directly as
// the distance from tail to front, so these are full visible lengths.
const BLASTER_BOLT_LENGTH: f32 = 0.9;
const BLASTER_BOLT_GLOW_WIDTH: f32 = 0.22;
const BLASTER_BOLT_CORE_WIDTH: f32 = 0.07;
// Heavy blaster (visual_scale >= threshold) gets a proportionally larger bolt.
const BLASTER_SPHERE_BOLT_LENGTH: f32 = 1.6;
const BLASTER_SPHERE_GLOW_WIDTH: f32 = 0.4;
const BLASTER_SPHERE_CORE_WIDTH: f32 = 0.14;

const BLASTER_TRAIL_MIN_DISTANCE: f32 = 0.12;
const BLASTER_TRAIL_LIFETIME_SECS: f32 = 0.08;
const BLASTER_TRAIL_WIDTH_SCALE: f32 = 0.6;

const BLASTER_MUZZLE_FLASH_LIFETIME_SECS: f32 = 0.05;
const BLASTER_MUZZLE_FLASH_START_SIZE: f32 = 0.12;
const BLASTER_MUZZLE_FLASH_END_SIZE: f32 = 0.4;

const BLASTER_IMPACT_RING_LIFETIME_SECS: f32 = 0.22;
const BLASTER_IMPACT_RING_START_SIZE: f32 = 0.12;
const BLASTER_IMPACT_RING_END_SIZE: f32 = 0.6;

const BLASTER_IMPACT_SPARK_LIFETIME_SECS: f32 = 0.18;
const BLASTER_IMPACT_SPARK_SIZE: f32 = 0.22;
const BLASTER_IMPACT_SPARK_COUNT: usize = 4;
const BLASTER_IMPACT_SPARK_SPREAD: f32 = 0.35;

const ENGINE_DEFAULT_COLOR: [f32; 4] = [0.25, 0.75, 1.0, 0.72];
const ENGINE_TRAIL_RADIUS: f32 = 1.5;
const ENGINE_TRAIL_CRUMB_LIFETIME_SECS: f32 = 1.5;
const ENGINE_TRAIL_MAX_CRUMBS: usize = 200;
const ENGINE_TRAIL_MIN_CRUMB_DIST: f32 = 0.08;
// Width tapers as a crumb ages, on top of the speed-based width set at spawn.
const ENGINE_TRAIL_AGE_WIDTH_FALLOFF: f32 = 0.5;

const ENGINE_TRAIL_SHADER: &str = "shaders/engine_trail.wgsl";
const ENGINE_TRAIL_NOISE_TEXTURE: &str = "pfx/engine_trail/wispy_noise.png";
const ENGINE_TRAIL_DISTORTION_TEXTURE: &str = "pfx/engine_trail/distortion_map.png";
const ENGINE_TRAIL_GRADIENT_TEXTURE: &str = "pfx/engine_trail/soft_gradient.png";
const ENGINE_TRAIL_DISSOLVE_TEXTURE: &str = "pfx/engine_trail/dissolve_mask.png";
const ENGINE_TRAIL_SCROLL_SPEED: f32 = 1.4;
const ENGINE_TRAIL_DISTORTION_STRENGTH: f32 = 0.06;

pub struct PfxPlugin;

impl Plugin for PfxPlugin {
    fn build(&self, app: &mut App) {
        // `MaterialPlugin` needs `Assets<Shader>`/`Assets<Image>` registered;
        // normally supplied by `RenderPlugin`/`ImagePlugin`, but the headless
        // server bootstrap (`server::bridge`) skips those, so register them
        // here when they are genuinely absent.
        //
        // The `contains_resource` guards are load-bearing: `init_asset` is not
        // idempotent. It installs a fresh `Assets<A>` backed by a new
        // `AssetIndexAllocator` and overwrites the `AssetServer`'s handle
        // provider for `A`. Every handle already minted from the old allocator
        // then indexes into storage that never allocated it, and the insert
        // that lands when its load finishes panics out of bounds. It also
        // discards the default and transparent images `ImagePlugin` seeds.
        if !app
            .world()
            .contains_resource::<Assets<bevy::shader::Shader>>()
        {
            app.init_asset::<bevy::shader::Shader>()
                .init_asset_loader::<bevy::shader::ShaderLoader>();
        }
        if !app.world().contains_resource::<Assets<Image>>() {
            app.init_asset::<Image>();
        }

        app.init_resource::<BeamPfxState>()
            .init_resource::<TorpedoPfxState>()
            .init_resource::<BlasterPfxState>()
            .init_resource::<EngineTrailState>()
            .init_resource::<PhaserPfxAssets>()
            .init_resource::<BlasterBoltPfxAssets>()
            .init_resource::<TorpedoPfxAssets>()
            .init_resource::<ShipExplosionPfxAssets>()
            // `spawn_ship_explosions` reads this message; registering it here
            // too (redundantly with `WeaponsPlugin`) means test apps that add
            // `PfxPlugin` without `WeaponsPlugin` still work — `add_message`
            // is idempotent (backed by `init_resource::<Messages<T>>`).
            .add_message::<ShipDestroyedVfx>()
            .add_plugins(MaterialPlugin::<EngineTrailMaterial>::default())
            .add_systems(Startup, load_engine_trail_textures)
            .add_systems(
                Update,
                (
                    sync_phaser_beams.run_if(in_state(GamePhase::InProgress)),
                    pulse_beam_contact_glow
                        .after(sync_phaser_beams)
                        .run_if(in_state(GamePhase::InProgress)),
                    sync_torpedo_pfx.run_if(in_state(GamePhase::InProgress)),
                    sync_blaster_pfx.run_if(in_state(GamePhase::InProgress)),
                    spawn_ship_explosions.run_if(in_state(GamePhase::InProgress)),
                    spawn_engine_trails
                        .after(crate::server::renderer::apply_local_ship_render_interpolation)
                        // Non-local hulls now interpolate too (thrust-burst
                        // fix), so their engine trails must read the same
                        // interpolated pose rather than the fixed-tick one.
                        .after(crate::server::renderer::apply_ship_render_interpolation)
                        .run_if(in_state(GamePhase::InProgress)),
                    tick_engine_trail_materials,
                    tick_lifetime_pfx.run_if(in_state(GamePhase::InProgress)),
                    tick_bursts.run_if(in_state(GamePhase::InProgress)),
                ),
                // These read ship `Transform`/`ShipPhysics`, which
                // `sync_ship_position` writes each sim tick. Since issue #895
                // that writer runs in `FixedUpdate`, which always completes
                // before `Update`, so the old `.after(SimSet::Physics)` edge
                // is provided by schedule order and no longer declared here.
            )
            // Runs after tick_bursts/sync_phaser_beams/pulse_beam_contact_glow
            // so its camera-facing rotation always wins for the frame (those
            // systems write Transform too, on the same textured-billboard
            // entities — pulse_beam_contact_glow only ever touches `scale`,
            // but Bevy's conflict detection is per-component, not per-field).
            .add_systems(
                Update,
                billboard_face_camera
                    .after(sync_phaser_beams)
                    .after(pulse_beam_contact_glow)
                    .after(tick_bursts)
                    .run_if(in_state(GamePhase::InProgress)),
            )
            .add_systems(OnExit(GamePhase::InProgress), cleanup_pfx);
    }
}

/// Layered "ion trail" ribbon material: scrolling wispy-noise flow, UV
/// distortion for wiggle, a soft cross-ribbon gradient profile, and a
/// dissolve mask that breaks up the tail fade. See `engine_trail.wgsl`.
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
struct EngineTrailMaterial {
    #[texture(0)]
    #[sampler(1)]
    noise_texture: Handle<Image>,
    #[texture(2)]
    #[sampler(3)]
    distortion_texture: Handle<Image>,
    #[texture(4)]
    #[sampler(5)]
    gradient_texture: Handle<Image>,
    #[texture(6)]
    #[sampler(7)]
    dissolve_texture: Handle<Image>,
    #[uniform(8)]
    color_r: f32,
    #[uniform(8)]
    color_g: f32,
    #[uniform(8)]
    color_b: f32,
    #[uniform(8)]
    color_a: f32,
    #[uniform(8)]
    time: f32,
    #[uniform(8)]
    scroll_speed: f32,
    #[uniform(8)]
    distortion_strength: f32,
    #[uniform(8)]
    _pad0: f32,
}

impl Material for EngineTrailMaterial {
    fn fragment_shader() -> ShaderRef {
        ENGINE_TRAIL_SHADER.into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Add
    }

    fn specialize(
        _pipeline: &bevy::pbr::MaterialPipeline,
        descriptor: &mut bevy::render::render_resource::RenderPipelineDescriptor,
        _layout: &bevy::mesh::MeshVertexBufferLayoutRef,
        _key: bevy::pbr::MaterialPipelineKey<Self>,
    ) -> Result<(), bevy::render::render_resource::SpecializedMeshPipelineError> {
        // The ribbon is a flat, camera-facing-ish strip; disable backface
        // culling so it stays visible from either side.
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

/// Preloaded texture handles shared by every engine trail material instance.
#[derive(Resource, Clone)]
struct EngineTrailTextures {
    noise: Handle<Image>,
    distortion: Handle<Image>,
    gradient: Handle<Image>,
    dissolve: Handle<Image>,
}

fn load_engine_trail_textures(mut commands: Commands, asset_server: Res<AssetServer>) {
    commands.insert_resource(EngineTrailTextures {
        noise: asset_server.load(ENGINE_TRAIL_NOISE_TEXTURE),
        distortion: asset_server.load(ENGINE_TRAIL_DISTORTION_TEXTURE),
        gradient: asset_server.load(ENGINE_TRAIL_GRADIENT_TEXTURE),
        dissolve: asset_server.load(ENGINE_TRAIL_DISSOLVE_TEXTURE),
    });
}

/// Advances the scroll-time uniform on every live engine trail material.
fn tick_engine_trail_materials(
    time: Res<Time>,
    mut materials: ResMut<Assets<EngineTrailMaterial>>,
) {
    let elapsed = time.elapsed_secs();
    for (_, material) in materials.iter_mut() {
        material.time = elapsed;
    }
}

/// Texture handles for the textured phaser beam/impact PFX layers, loaded
/// once at plugin build time from `assets/pfx/` (sourced from
/// `raw/pfx/phaser_pfx_assets/`).
#[derive(Resource)]
struct PhaserPfxAssets {
    beam_glow: Handle<Image>,
    beam_core: Handle<Image>,
    radial_glow: Handle<Image>,
    impact_ring: Handle<Image>,
    spark_streak: Handle<Image>,
}

impl FromWorld for PhaserPfxAssets {
    fn from_world(world: &mut World) -> Self {
        let asset_server = world.resource::<AssetServer>();
        Self {
            beam_glow: asset_server.load("pfx/beam_glow.png"),
            beam_core: asset_server.load("pfx/beam_core.png"),
            radial_glow: asset_server.load("pfx/radial_glow.png"),
            impact_ring: asset_server.load("pfx/impact_ring.png"),
            spark_streak: asset_server.load("pfx/spark_streak.png"),
        }
    }
}

/// Texture handles for the textured blaster-bolt PFX layers, loaded once at
/// plugin build time from `assets/pfx/blaster/`. Muzzle flash, impact ring
/// and impact sparks reuse the generic `PhaserPfxAssets` textures (same
/// radial-glow/ring/streak shapes work for any energy-weapon burst — only
/// the travelling bolt itself needs the asymmetric bolt-specific core/glow).
#[derive(Resource)]
struct BlasterBoltPfxAssets {
    bolt_core: Handle<Image>,
    bolt_glow: Handle<Image>,
}

impl FromWorld for BlasterBoltPfxAssets {
    fn from_world(world: &mut World) -> Self {
        let asset_server = world.resource::<AssetServer>();
        Self {
            bolt_core: asset_server.load("pfx/blaster/blaster_core.png"),
            bolt_glow: asset_server.load("pfx/blaster/blaster_glow.png"),
        }
    }
}

/// Texture handle for the one new photon-torpedo PFX asset — a hard, small,
/// bright energy core (harder falloff than the generic `radial_glow`, used
/// for the shell). The directional flare reuses `BlasterBoltPfxAssets::
/// bolt_glow` (same asymmetric trailing-streak shape); launch flash, impact
/// ring and sparks reuse `PhaserPfxAssets`; the impact plasma bloom reuses
/// `ShipExplosionPfxAssets::puff`.
#[derive(Resource)]
struct TorpedoPfxAssets {
    core: Handle<Image>,
}

impl FromWorld for TorpedoPfxAssets {
    fn from_world(world: &mut World) -> Self {
        let asset_server = world.resource::<AssetServer>();
        Self {
            core: asset_server.load("pfx/torpedo/torpedo_core.png"),
        }
    }
}

/// Marker for PFX entities that should always face the game camera —
/// textured quads (beam contact glow, muzzle flash, impact ring, sparks)
/// rendered as billboards rather than surfaces with fixed orientation.
#[derive(Component)]
struct Billboard;

/// Rotates every `Billboard` entity to face the camera each frame.
fn billboard_face_camera(
    cam_q: Query<&Transform, (With<GameCamera>, Without<Billboard>)>,
    mut q: Query<&mut Transform, With<Billboard>>,
) {
    let Ok(cam_t) = cam_q.single() else {
        return;
    };
    let cam_pos = cam_t.translation;
    for mut t in q.iter_mut() {
        if (cam_pos - t.translation).length_squared() > 1e-6 {
            t.look_at(cam_pos, Vec3::Y);
        }
    }
}

/// Pulses each live beam's target-contact glow in size and brightness (issue
/// phaser-pfx-core-beam). `sync_phaser_beams` only ever writes this entity's
/// `translation` once it exists (see the comment in `upsert_beam`'s "existing"
/// branch) — this system owns `scale` and the material's `emissive` for the
/// whole lifetime of the glow, so the two never fight over the same field.
///
/// Every beam's contact glow gets its own `StandardMaterial` instance (built
/// fresh per beam in `upsert_beam`), so mutating it here never bleeds into
/// another beam's glow.
fn pulse_beam_contact_glow(
    time: Res<Time>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut q: Query<
        (
            &BeamContactPulse,
            &MeshMaterial3d<StandardMaterial>,
            &mut Transform,
        ),
        With<BeamContactGlow>,
    >,
) {
    let t = time.elapsed_secs();
    for (pulse, mat_handle, mut transform) in &mut q {
        let wave = 0.5 + 0.5 * (t * CONTACT_PULSE_HZ * std::f32::consts::TAU + pulse.phase).sin();

        let scale =
            CONTACT_PULSE_SCALE_MIN + (CONTACT_PULSE_SCALE_MAX - CONTACT_PULSE_SCALE_MIN) * wave;
        transform.scale = Vec3::splat(CONTACT_GLOW_SIZE * scale);

        let emissive_mul = CONTACT_PULSE_EMISSIVE_MIN
            + (CONTACT_PULSE_EMISSIVE_MAX - CONTACT_PULSE_EMISSIVE_MIN) * wave;
        if let Some(mat) = materials.get_mut(&mat_handle.0) {
            mat.emissive = LinearRgba::new(
                pulse.base_emissive.red * emissive_mul,
                pulse.base_emissive.green * emissive_mul,
                pulse.base_emissive.blue * emissive_mul,
                pulse.base_emissive.alpha,
            );
        }
    }
}

#[derive(Component)]
struct PfxEntity;

#[derive(Component)]
struct BeamBody;

#[derive(Component)]
struct BeamContactGlow;

/// Drives `pulse_beam_contact_glow`'s rhythmic brightness/size pulse for one
/// beam's target-contact glow. `base_emissive` is the material's emissive at
/// spawn (`t=0`, pulse factor 1.0); the pulse system always sets an ABSOLUTE
/// value scaled off it rather than multiplying the current value, so per-frame
/// factors never compound into drift.
#[derive(Component)]
struct BeamContactPulse {
    phase: f32,
    base_emissive: LinearRgba,
}

#[derive(Component)]
struct TorpedoBody;

#[derive(Component)]
struct BlasterBolt;

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
    glow_a: Entity,
    glow_b: Entity,
    core_a: Entity,
    core_b: Entity,
    contact: Entity,
}

#[derive(Resource, Default)]
struct BeamPfxState {
    active: HashMap<String, BeamEntities>,
    target_point_choices: HashMap<String, usize>,
}

struct TorpedoEntities {
    core: Entity,
    shell: Entity,
    flare_a: Entity,
    flare_b: Entity,
    last_pos: Vec3,
}

#[derive(Resource, Default)]
struct TorpedoPfxState {
    active: HashMap<String, TorpedoEntities>,
}

struct BlasterPfxEntities {
    glow_a: Entity,
    glow_b: Entity,
    core_a: Entity,
    core_b: Entity,
    last_pos: Vec3,
    half_len: f32,
    glow_width: f32,
    core_width: f32,
    color: [f32; 4],
}

#[derive(Resource, Default)]
struct BlasterPfxState {
    active: HashMap<String, BlasterPfxEntities>,
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
            &PhaserRenderConfig,
            &PhaserCombatConfigResource,
            bevy::ecs::query::Has<LocalShip>,
        ),
        (
            With<crate::server_app::Ship>,
            Without<BeamBody>,
            Without<BeamContactGlow>,
        ),
    >,
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
    pfx_assets: Res<PhaserPfxAssets>,
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

    for (src_t, src_markers, beam, src_uuid_opt, render_cfg, combat_cfg, is_local) in
        beam_ships_q.iter()
    {
        if !beam.is_firing() {
            continue;
        }

        // Shooter identity: prefer the entity's own UUID; for the LocalShip in
        // legacy test harnesses without one, fall back to a stable string.
        // NPCs always have a UUID from `entities::spawner::spawn_entity`; skip
        // any that don't (nothing to key the render entity on).
        let src_key: String = match src_uuid_opt {
            Some(u) => u.0.clone(),
            None if is_local => "local".to_string(),
            None => continue,
        };

        // One render entity per LIVE BANK (issue #790). The key format already
        // carried the bank, and its doc already promised "simultaneous beams
        // from different banks render as distinct entities" — before #790 the
        // single-slot `ActiveBeam` made that unreachable. It is reachable now.
        for (bank_id, slot) in beam.live_banks() {
            let target_uuid = slot.target_uuid.as_str();
            let key = format!("beam:{}:{}:{}", src_key, bank_id, target_uuid);
            let bank_cfg = combat_cfg.0.bank_by_id(bank_id);

            let target_point_index = choose_target_point_index(
                &key,
                target_point_count(
                    target_uuid,
                    local_ship_uuid.as_deref(),
                    &entity_q,
                    &local_ship_q,
                ),
                &mut state,
            );
            let Some(target_pos) = target_position(
                target_uuid,
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
                &pfx_assets,
                &mut state,
                &mut commands,
                &mut meshes,
                &mut materials,
                &mut body_q,
                &mut glow_q,
            );
        }
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
            commands.entity(entities.glow_a).try_despawn();
            commands.entity(entities.glow_b).try_despawn();
            commands.entity(entities.core_a).try_despawn();
            commands.entity(entities.core_b).try_despawn();
            commands.entity(entities.contact).try_despawn();
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn upsert_beam(
    key: String,
    start: Vec3,
    end: Vec3,
    color: [f32; 4],
    pfx_assets: &PhaserPfxAssets,
    state: &mut BeamPfxState,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    body_q: &mut Query<&mut Transform, (With<BeamBody>, Without<BeamContactGlow>)>,
    glow_q: &mut Query<&mut Transform, (With<BeamContactGlow>, Without<BeamBody>)>,
) {
    let cross = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);

    if let Some(existing) = state.active.get(&key) {
        let glow_t = segment_transform(start, end, BEAM_GLOW_WIDTH);
        let core_t = segment_transform(start, end, BEAM_CORE_WIDTH);
        if let Ok(mut t) = body_q.get_mut(existing.glow_a) {
            *t = glow_t;
        }
        if let Ok(mut t) = body_q.get_mut(existing.glow_b) {
            *t = Transform {
                rotation: glow_t.rotation * cross,
                ..glow_t
            };
        }
        if let Ok(mut t) = body_q.get_mut(existing.core_a) {
            *t = core_t;
        }
        if let Ok(mut t) = body_q.get_mut(existing.core_b) {
            *t = Transform {
                rotation: core_t.rotation * cross,
                ..core_t
            };
        }
        // Translation only — `pulse_beam_contact_glow` owns this entity's
        // scale (and its material's emissive) every frame, `.after` this
        // system. Resetting scale here would fight that pulse back to a
        // constant size on every beam-position update.
        if let Ok(mut t) = glow_q.get_mut(existing.contact) {
            t.translation = end;
        }
        return;
    }

    // First tick this beam exists: build the crossed-ribbon body (broad
    // colour glow + narrow white-hot core, per-layer textured quads so the
    // beam never disappears when viewed edge-on), a camera-facing contact
    // glow at the endpoint, a brief muzzle flash at the origin, and an
    // impact burst (ring + sparks) at the endpoint.
    let ribbon_mesh = meshes.add(beam_ribbon_quad_mesh());
    let billboard_mesh = meshes.add(unit_billboard_mesh());

    // Glow is the halo, not the body (issue phaser-pfx-core-beam): lower
    // alpha and emissive than before so it reads as a soft haze around the
    // core rather than competing with it. Core pushed further toward
    // white-hot (was 0.4/0.6, now 0.2/0.8) and its emissive raised so the
    // narrowed `BEAM_CORE_WIDTH` above still reads as unmistakably brighter
    // than the glow around it, not just thinner.
    let glow_color = [color[0], color[1], color[2], color[3] * 0.55];
    let core_color = [
        color[0] * 0.2 + 0.8,
        color[1] * 0.2 + 0.8,
        color[2] * 0.2 + 0.8,
        color[3],
    ];

    let glow_mat =
        phaser_texture_material(materials, pfx_assets.beam_glow.clone(), glow_color, 3.0);
    let core_mat =
        phaser_texture_material(materials, pfx_assets.beam_core.clone(), core_color, 11.0);
    let contact_mat = phaser_texture_material(
        materials,
        pfx_assets.radial_glow.clone(),
        core_color,
        CONTACT_GLOW_EMISSIVE_STRENGTH,
    );

    let glow_t = segment_transform(start, end, BEAM_GLOW_WIDTH);
    let core_t = segment_transform(start, end, BEAM_CORE_WIDTH);

    let glow_a = commands
        .spawn((
            PfxEntity,
            BeamBody,
            Mesh3d(ribbon_mesh.clone()),
            MeshMaterial3d(glow_mat.clone()),
            glow_t,
        ))
        .id();
    let glow_b = commands
        .spawn((
            PfxEntity,
            BeamBody,
            Mesh3d(ribbon_mesh.clone()),
            MeshMaterial3d(glow_mat),
            Transform {
                rotation: glow_t.rotation * cross,
                ..glow_t
            },
        ))
        .id();
    let core_a = commands
        .spawn((
            PfxEntity,
            BeamBody,
            Mesh3d(ribbon_mesh.clone()),
            MeshMaterial3d(core_mat.clone()),
            core_t,
        ))
        .id();
    let core_b = commands
        .spawn((
            PfxEntity,
            BeamBody,
            Mesh3d(ribbon_mesh),
            MeshMaterial3d(core_mat),
            Transform {
                rotation: core_t.rotation * cross,
                ..core_t
            },
        ))
        .id();
    let contact = commands
        .spawn((
            PfxEntity,
            BeamContactGlow,
            Billboard,
            Mesh3d(billboard_mesh.clone()),
            MeshMaterial3d(contact_mat),
            Transform::from_translation(end).with_scale(Vec3::splat(CONTACT_GLOW_SIZE)),
            BeamContactPulse {
                phase: rand::rng().random_range(0.0..std::f32::consts::TAU),
                base_emissive: LinearRgba::new(
                    core_color[0] * CONTACT_GLOW_EMISSIVE_STRENGTH,
                    core_color[1] * CONTACT_GLOW_EMISSIVE_STRENGTH,
                    core_color[2] * CONTACT_GLOW_EMISSIVE_STRENGTH,
                    core_color[3],
                ),
            },
        ))
        .id();

    spawn_muzzle_flash(
        start,
        &billboard_mesh,
        pfx_assets,
        core_color,
        commands,
        materials,
    );
    spawn_impact_burst(end, &billboard_mesh, pfx_assets, color, commands, materials);

    state.active.insert(
        key,
        BeamEntities {
            glow_a,
            glow_b,
            core_a,
            core_b,
            contact,
        },
    );
}

/// Appearance and lifetime of one transient billboard. Random offsets stay with the effect.
struct BurstSprite {
    texture: Option<Handle<Image>>,
    color: [f32; 4],
    emissive_strength: f32,
    lifetime: f32,
    start_scale: f32,
    end_scale: Option<f32>,
}

fn spawn_burst_sprite(
    position: Vec3,
    mesh: Handle<Mesh>,
    sprite: BurstSprite,
    commands: &mut Commands,
    materials: &mut Assets<StandardMaterial>,
) {
    // Untextured blaster flashes retain their original single-sided material.
    let material = match sprite.texture {
        Some(texture) => {
            phaser_texture_material(materials, texture, sprite.color, sprite.emissive_strength)
        }
        None => glow_material(
            materials,
            sprite.color,
            sprite.emissive_strength,
            AlphaMode::Add,
        ),
    };
    let mut entity = commands.spawn((
        PfxEntity,
        Billboard,
        Mesh3d(mesh),
        MeshMaterial3d(material.clone()),
        Transform::from_translation(position).with_scale(Vec3::splat(sprite.start_scale)),
        PfxLifetime {
            age: 0.0,
            lifetime: sprite.lifetime,
        },
        PfxFadingMaterial {
            handle: material,
            color: sprite.color,
            emissive_strength: sprite.emissive_strength,
        },
    ));
    if let Some(end_scale) = sprite.end_scale {
        entity.insert(PfxBurst {
            start_scale: sprite.start_scale,
            end_scale,
        });
    }
}

/// Brief bright flash establishing the beam's origin point (per the "muzzle
/// effect" design: restrained, brief, tightly concentrated).
fn spawn_muzzle_flash(
    pos: Vec3,
    billboard_mesh: &Handle<Mesh>,
    pfx_assets: &PhaserPfxAssets,
    color: [f32; 4],
    commands: &mut Commands,
    materials: &mut Assets<StandardMaterial>,
) {
    spawn_burst_sprite(
        pos,
        billboard_mesh.clone(),
        BurstSprite {
            texture: Some(pfx_assets.radial_glow.clone()),
            color,
            emissive_strength: 8.0,
            lifetime: MUZZLE_FLASH_LIFETIME_SECS,
            start_scale: MUZZLE_FLASH_START_SIZE,
            end_scale: Some(MUZZLE_FLASH_END_SIZE),
        },
        commands,
        materials,
    );
}

/// One-shot impact burst at the beam endpoint: an expanding ring plus a
/// handful of outward sparks, layered on top of the persistent contact glow.
fn spawn_impact_burst(
    pos: Vec3,
    billboard_mesh: &Handle<Mesh>,
    pfx_assets: &PhaserPfxAssets,
    color: [f32; 4],
    commands: &mut Commands,
    materials: &mut Assets<StandardMaterial>,
) {
    let ring_color = [color[0], color[1], color[2], color[3] * 0.9];
    spawn_burst_sprite(
        pos,
        billboard_mesh.clone(),
        BurstSprite {
            texture: Some(pfx_assets.impact_ring.clone()),
            color: ring_color,
            emissive_strength: 5.0,
            lifetime: IMPACT_RING_LIFETIME_SECS,
            start_scale: IMPACT_RING_START_SIZE,
            end_scale: Some(IMPACT_RING_END_SIZE),
        },
        commands,
        materials,
    );

    let mut rng = rand::rng();
    for _ in 0..IMPACT_SPARK_COUNT {
        let offset = Vec3::new(
            rng.random_range(-1.0_f32..1.0),
            rng.random_range(-0.3_f32..0.3),
            rng.random_range(-1.0_f32..1.0),
        )
        .normalize_or_zero()
            * IMPACT_SPARK_SPREAD;
        let spark_color = [
            color[0] * 0.5 + 0.5,
            color[1] * 0.5 + 0.5,
            color[2] * 0.5 + 0.5,
            color[3],
        ];
        spawn_burst_sprite(
            pos + offset,
            billboard_mesh.clone(),
            BurstSprite {
                texture: Some(pfx_assets.spark_streak.clone()),
                color: spark_color,
                emissive_strength: 6.0,
                lifetime: IMPACT_SPARK_LIFETIME_SECS,
                start_scale: IMPACT_SPARK_SIZE,
                end_scale: None,
            },
            commands,
            materials,
        );
    }
}

/// Unit quad (local X width -1..1, local Y length -0.5..0.5) reused via
/// `segment_transform`'s (radius, length, radius) scale — matches the
/// convention the old unit `Cylinder` primitive used. UV maps local Y
/// (segment length) to U and local X (segment width) to V.
///
/// This is the *projectile* ribbon: U sweeps 0..1 from tail to head so a
/// texture authored with a bright rounded tip and a tapered fade reads as a
/// bolt travelling head-first (blaster bolt, torpedo flare). Sustained beams
/// want `beam_ribbon_quad_mesh` instead — see the note there.
fn unit_ribbon_quad_mesh() -> Mesh {
    ribbon_quad_mesh(0.0, 1.0)
}

/// The phaser beam's ribbon quad: identical geometry to
/// `unit_ribbon_quad_mesh`, but with U pinned to a single texture column
/// instead of sweeping along the beam.
///
/// Issue #938: `beam_glow.png` and `beam_core.png` are lens-shaped — opaque
/// at the centre and fading to transparent at *all four* edges, left and
/// right included, not just top and bottom. Sweeping U along the beam
/// therefore stretched that horizontal falloff over the entire
/// muzzle-to-target span, which is exactly the reported artefact: alpha
/// peaked at mid-U (the bulge) and fell to zero at U=0 and U=1 (the taper to
/// nothing at both ends). The geometry was never the problem —
/// `segment_transform` already scales width independently of length.
///
/// A phaser is a sustained beam, not a projectile: it has no head or tail, so
/// there is nothing for the length axis to encode. Sampling the texture's
/// widest cross-section at every point along the beam keeps the authored
/// across-width falloff — V still crosses the soft edge — while making the
/// profile constant from muzzle to target, at any length and from any camera
/// angle (the crossed `_a`/`_b` quads are unchanged).
fn beam_ribbon_quad_mesh() -> Mesh {
    ribbon_quad_mesh(BEAM_PROFILE_U, BEAM_PROFILE_U)
}

/// Shared builder for the crossed-ribbon quads. `u_at_tail`/`u_at_head` are
/// the texture columns sampled at the segment's start (local Y = -0.5) and
/// end (local Y = +0.5); passing the same value for both makes the sampled
/// profile constant along the segment.
fn ribbon_quad_mesh(u_at_tail: f32, u_at_head: f32) -> Mesh {
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    let positions: Vec<[f32; 3]> = vec![
        [-1.0, -0.5, 0.0],
        [1.0, -0.5, 0.0],
        [1.0, 0.5, 0.0],
        [-1.0, 0.5, 0.0],
    ];
    let normals: Vec<[f32; 3]> = vec![[0.0, 0.0, 1.0]; 4];
    let uvs: Vec<[f32; 2]> = vec![
        [u_at_tail, 0.0],
        [u_at_tail, 1.0],
        [u_at_head, 1.0],
        [u_at_head, 0.0],
    ];
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_indices(Indices::U32(vec![0, 1, 2, 0, 2, 3]));
    mesh
}

/// Unit quad (-0.5..0.5 both axes) for camera-facing billboards (muzzle
/// flash, contact glow, impact ring, sparks).
fn unit_billboard_mesh() -> Mesh {
    Mesh::from(Rectangle::new(1.0, 1.0))
}

/// Additive-blended, unlit, double-sided textured material for a phaser PFX
/// layer (beam glow/core, muzzle flash, contact glow, impact ring, sparks).
fn phaser_texture_material(
    materials: &mut Assets<StandardMaterial>,
    texture: Handle<Image>,
    color: [f32; 4],
    emissive_strength: f32,
) -> Handle<StandardMaterial> {
    materials.add(StandardMaterial {
        base_color: Color::srgba(color[0], color[1], color[2], color[3]),
        base_color_texture: Some(texture),
        emissive: LinearRgba::new(
            color[0] * emissive_strength,
            color[1] * emissive_strength,
            color[2] * emissive_strength,
            color[3],
        ),
        alpha_mode: AlphaMode::Add,
        unlit: true,
        double_sided: true,
        cull_mode: None,
        ..default()
    })
}

/// Renders every ship's in-flight torpedoes each frame.
///
/// Iterates `Query<..., With<Ship>>` so NPC torpedoes render alongside the
/// player's. Torpedo UUIDs are globally unique (minted via
/// `crate::world_id::mint_id_with`, issue #907 — a `(namespace, tick, seq)`
/// counter, not `Uuid::new_v4()`), so merging in-flight lists across ships
/// never collides on tracker keys.
///
/// Each torpedo is a hard core + soft shell billboard pair plus a
/// velocity-aligned directional flare (crossed ribbon, reusing the blaster
/// bolt's asymmetric glow texture), per the photon-torpedo PFX guide. A
/// launch flash fires when a torpedo first appears in `in_flight`; a
/// richer impact burst (contact flash, plasma bloom, ring, sparks) fires
/// on despawn — torpedoes get a more elaborate detonation than blaster
/// bolts, matching their heavier-weapon role.
#[allow(clippy::too_many_arguments)]
fn sync_torpedo_pfx(
    ships_q: Query<&TorpedoSystemResource, With<crate::server_app::Ship>>,
    torpedo_pfx_assets: Res<TorpedoPfxAssets>,
    bolt_pfx_assets: Res<BlasterBoltPfxAssets>,
    phaser_pfx_assets: Res<PhaserPfxAssets>,
    explosion_pfx_assets: Res<ShipExplosionPfxAssets>,
    mut state: ResMut<TorpedoPfxState>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut body_q: Query<&mut Transform, With<TorpedoBody>>,
) {
    // Collect (uuid, x, y, z, heading) tuples for every in-flight torpedo
    // across every ship. A single flat list makes the diff-against-tracker
    // trivial, and heading lets the flare orient correctly for homing
    // torpedoes that curve mid-flight. Y is the torpedo's real altitude
    // (issue #768), so a climbing/descending torpedo renders off the play
    // plane instead of being pinned to it; a Planar torpedo has y == 0.
    let mut all_in_flight: Vec<(String, f32, f32, f32, f32)> = Vec::new();
    for torpedo_sys in ships_q.iter() {
        for t in &torpedo_sys.0.in_flight {
            all_in_flight.push((t.uuid.clone(), t.x, t.y, t.z, t.heading));
        }
    }

    let live: HashSet<String> = all_in_flight.iter().map(|(u, ..)| u.clone()).collect();
    let tracked: HashSet<String> = state.active.keys().cloned().collect();
    let (to_spawn, to_despawn) = diff_torpedo_sets(&live, &tracked);

    for uuid in to_despawn {
        if let Some(entities) = state.active.remove(&uuid) {
            commands.entity(entities.core).try_despawn();
            commands.entity(entities.shell).try_despawn();
            commands.entity(entities.flare_a).try_despawn();
            commands.entity(entities.flare_b).try_despawn();
            spawn_torpedo_impact_burst(
                entities.last_pos,
                &phaser_pfx_assets,
                &explosion_pfx_assets,
                &mut commands,
                &mut meshes,
                &mut materials,
            );
        }
    }

    for uuid in to_spawn {
        if let Some((_, x, y, z, heading)) = all_in_flight.iter().find(|(u, ..)| u == &uuid) {
            let pos = Vec3::new(*x, *y, *z);
            spawn_torpedo_pfx(
                uuid,
                pos,
                *heading,
                &torpedo_pfx_assets,
                &bolt_pfx_assets,
                &phaser_pfx_assets,
                &mut state,
                &mut commands,
                &mut meshes,
                &mut materials,
            );
        }
    }

    for (uuid, x, y, z, heading) in &all_in_flight {
        let pos = Vec3::new(*x, *y, *z);
        update_torpedo_pfx(
            uuid,
            pos,
            *heading,
            &mut state,
            &mut commands,
            &mut meshes,
            &mut materials,
            &mut body_q,
        );
    }
}

/// Spawns the core+shell billboards and directional flare for a
/// newly-appeared torpedo, plus its launch flash. Called once per torpedo,
/// the tick it first shows up in `in_flight`.
#[allow(clippy::too_many_arguments)]
fn spawn_torpedo_pfx(
    uuid: String,
    pos: Vec3,
    heading: f32,
    torpedo_pfx_assets: &TorpedoPfxAssets,
    bolt_pfx_assets: &BlasterBoltPfxAssets,
    phaser_pfx_assets: &PhaserPfxAssets,
    state: &mut TorpedoPfxState,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
) {
    let billboard_mesh = meshes.add(unit_billboard_mesh());

    let core_mat = phaser_texture_material(
        materials,
        torpedo_pfx_assets.core.clone(),
        TORPEDO_CORE_COLOR,
        TORPEDO_CORE_EMISSIVE,
    );
    let core = commands
        .spawn((
            PfxEntity,
            TorpedoBody,
            Billboard,
            Mesh3d(billboard_mesh.clone()),
            MeshMaterial3d(core_mat),
            Transform::from_translation(pos).with_scale(Vec3::splat(TORPEDO_CORE_SIZE)),
        ))
        .id();

    let shell_mat = phaser_texture_material(
        materials,
        phaser_pfx_assets.radial_glow.clone(),
        TORPEDO_COLOR,
        TORPEDO_SHELL_EMISSIVE,
    );
    let shell = commands
        .spawn((
            PfxEntity,
            TorpedoBody,
            Billboard,
            Mesh3d(billboard_mesh),
            MeshMaterial3d(shell_mat),
            Transform::from_translation(pos).with_scale(Vec3::splat(TORPEDO_SHELL_SIZE)),
        ))
        .id();

    // Directional flare: a crossed ribbon trailing behind the torpedo,
    // reusing the blaster bolt's asymmetric glow texture (bright rounded
    // tip at the "front"/current position, tapered fade at the "tail").
    let ribbon_mesh = meshes.add(unit_ribbon_quad_mesh());
    let cross = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
    let forward = blaster_bolt_forward(heading);
    let front = pos;
    let tail = pos - forward * TORPEDO_FLARE_LENGTH;
    let flare_t = segment_transform(tail, front, TORPEDO_FLARE_WIDTH);
    let flare_mat = phaser_texture_material(
        materials,
        bolt_pfx_assets.bolt_glow.clone(),
        TORPEDO_COLOR,
        TORPEDO_FLARE_EMISSIVE,
    );
    let flare_a = commands
        .spawn((
            PfxEntity,
            TorpedoBody,
            Mesh3d(ribbon_mesh.clone()),
            MeshMaterial3d(flare_mat.clone()),
            flare_t,
        ))
        .id();
    let flare_b = commands
        .spawn((
            PfxEntity,
            TorpedoBody,
            Mesh3d(ribbon_mesh),
            MeshMaterial3d(flare_mat),
            Transform {
                rotation: flare_t.rotation * cross,
                ..flare_t
            },
        ))
        .id();

    spawn_torpedo_launch_flash(pos, phaser_pfx_assets, commands, meshes, materials);

    state.active.insert(
        uuid,
        TorpedoEntities {
            core,
            shell,
            flare_a,
            flare_b,
            last_pos: pos,
        },
    );
}

/// Updates an already-live torpedo's billboards/flare to its new
/// position/heading each frame, and lays down a trail segment behind it.
fn update_torpedo_pfx(
    uuid: &str,
    pos: Vec3,
    heading: f32,
    state: &mut TorpedoPfxState,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    body_q: &mut Query<&mut Transform, With<TorpedoBody>>,
) {
    let Some(entities) = state.active.get_mut(uuid) else {
        return;
    };

    if entities.last_pos.distance(pos) >= TORPEDO_TRAIL_MIN_DISTANCE {
        spawn_trail_segment(
            entities.last_pos,
            pos,
            TORPEDO_TRAIL_RADIUS,
            [1.0, 0.45, 0.08, 0.5],
            4.0,
            TORPEDO_TRAIL_LIFETIME_SECS,
            commands,
            meshes,
            materials,
        );
    }
    entities.last_pos = pos;

    if let Ok(mut t) = body_q.get_mut(entities.core) {
        t.translation = pos;
    }
    if let Ok(mut t) = body_q.get_mut(entities.shell) {
        t.translation = pos;
    }

    let cross = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
    let forward = blaster_bolt_forward(heading);
    let front = pos;
    let tail = pos - forward * TORPEDO_FLARE_LENGTH;
    let flare_t = segment_transform(tail, front, TORPEDO_FLARE_WIDTH);
    if let Ok(mut t) = body_q.get_mut(entities.flare_a) {
        *t = flare_t;
    }
    if let Ok(mut t) = body_q.get_mut(entities.flare_b) {
        *t = Transform {
            rotation: flare_t.rotation * cross,
            ..flare_t
        };
    }
}

/// Brief flash establishing the torpedo's launch point, reusing the
/// generic radial-glow texture (same shape as the phaser/blaster muzzle
/// flash — only color, size and lifetime differ per weapon).
fn spawn_torpedo_launch_flash(
    pos: Vec3,
    phaser_pfx_assets: &PhaserPfxAssets,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
) {
    spawn_burst_sprite(
        pos,
        meshes.add(unit_billboard_mesh()),
        BurstSprite {
            texture: Some(phaser_pfx_assets.radial_glow.clone()),
            color: TORPEDO_COLOR,
            emissive_strength: TORPEDO_CORE_EMISSIVE,
            lifetime: TORPEDO_LAUNCH_FLASH_LIFETIME_SECS,
            start_scale: TORPEDO_LAUNCH_FLASH_START_SIZE,
            end_scale: Some(TORPEDO_LAUNCH_FLASH_END_SIZE),
        },
        commands,
        materials,
    );
}

/// Detonation burst where a torpedo disappears (hit or expiry): a hard
/// contact flash, an irregular plasma bloom (reusing the ship-explosion
/// puff texture at torpedo scale), an expanding ring, and radial sparks —
/// a richer sequence than the blaster's ring+sparks, matching the
/// torpedo's heavier-weapon role in the design guide.
fn spawn_torpedo_impact_burst(
    pos: Vec3,
    phaser_pfx_assets: &PhaserPfxAssets,
    explosion_pfx_assets: &ShipExplosionPfxAssets,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
) {
    let billboard_mesh = meshes.add(unit_billboard_mesh());

    // Contact flash.
    spawn_burst_sprite(
        pos,
        billboard_mesh.clone(),
        BurstSprite {
            texture: Some(phaser_pfx_assets.radial_glow.clone()),
            color: TORPEDO_CORE_COLOR,
            emissive_strength: TORPEDO_CORE_EMISSIVE,
            lifetime: TORPEDO_IMPACT_FLASH_LIFETIME_SECS,
            start_scale: TORPEDO_IMPACT_FLASH_START_SIZE,
            end_scale: Some(TORPEDO_IMPACT_FLASH_END_SIZE),
        },
        commands,
        materials,
    );

    // Irregular plasma bloom.
    spawn_burst_sprite(
        pos,
        billboard_mesh.clone(),
        BurstSprite {
            texture: Some(explosion_pfx_assets.puff.clone()),
            color: TORPEDO_COLOR,
            emissive_strength: TORPEDO_SHELL_EMISSIVE,
            lifetime: TORPEDO_IMPACT_PLASMA_LIFETIME_SECS,
            start_scale: TORPEDO_IMPACT_PLASMA_START_SCALE,
            end_scale: Some(TORPEDO_IMPACT_PLASMA_END_SCALE),
        },
        commands,
        materials,
    );

    // Expanding ring.
    spawn_burst_sprite(
        pos,
        billboard_mesh.clone(),
        BurstSprite {
            texture: Some(phaser_pfx_assets.impact_ring.clone()),
            color: TORPEDO_COLOR,
            emissive_strength: TORPEDO_SHELL_EMISSIVE,
            lifetime: TORPEDO_IMPACT_RING_LIFETIME_SECS,
            start_scale: TORPEDO_IMPACT_RING_START_SCALE,
            end_scale: Some(TORPEDO_IMPACT_RING_END_SCALE),
        },
        commands,
        materials,
    );

    // Radial sparks.
    let mut rng = rand::rng();
    for _ in 0..TORPEDO_IMPACT_SPARK_COUNT {
        let offset = Vec3::new(
            rng.random_range(-1.0_f32..1.0),
            rng.random_range(-0.3_f32..0.3),
            rng.random_range(-1.0_f32..1.0),
        )
        .normalize_or_zero()
            * TORPEDO_IMPACT_SPARK_SPREAD;
        let spark_color = [
            TORPEDO_COLOR[0] * 0.5 + 0.5,
            TORPEDO_COLOR[1] * 0.5 + 0.5,
            TORPEDO_COLOR[2] * 0.5 + 0.5,
            TORPEDO_COLOR[3],
        ];
        spawn_burst_sprite(
            pos + offset,
            billboard_mesh.clone(),
            BurstSprite {
                texture: Some(phaser_pfx_assets.spark_streak.clone()),
                color: spark_color,
                emissive_strength: TORPEDO_SHELL_EMISSIVE,
                lifetime: TORPEDO_IMPACT_SPARK_LIFETIME_SECS,
                start_scale: TORPEDO_IMPACT_SPARK_SCALE,
                end_scale: None,
            },
            commands,
            materials,
        );
    }
}

/// Renders every ship's in-flight blaster projectiles each frame.
///
/// Iterates `Query<..., With<Ship>>` so NPC blasters render alongside the
/// player's. Uses `visual_scale` to switch between two visual variants:
///
///  - `visual_scale < BLASTER_SPHERE_VISUAL_SCALE_THRESHOLD`: standard bolt
///    (cyan, smaller) — used by Destroyer blasters.
///  - `visual_scale >= BLASTER_SPHERE_VISUAL_SCALE_THRESHOLD`: heavy bolt
///    (orange, larger) — used by Battleship heavy blaster.
///
/// Each bolt is a textured crossed-quad (glow + hot core layers, per the
/// phaser-beam pattern) oriented along the projectile's `heading` so it
/// reads correctly from any camera angle and never degenerates into an
/// invisible edge-on plane. A brief muzzle flash fires when a projectile
/// first appears in `in_flight`; an impact burst fires when it disappears
/// (hit or expiry — both look identical from here, matching the existing
/// torpedo-burst-on-despawn convention).
#[allow(clippy::too_many_arguments)]
fn sync_blaster_pfx(
    ships_q: Query<&BlasterSystemResource, With<crate::server_app::Ship>>,
    bolt_pfx_assets: Res<BlasterBoltPfxAssets>,
    phaser_pfx_assets: Res<PhaserPfxAssets>,
    mut state: ResMut<BlasterPfxState>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut body_q: Query<&mut Transform, With<BlasterBolt>>,
) {
    let mut all_in_flight: Vec<(String, f32, f32, f32, bool)> = Vec::new();
    for blaster_sys in ships_q.iter() {
        for bank in &blaster_sys.0 {
            let is_heavy = bank.config.visual_scale >= BLASTER_SPHERE_VISUAL_SCALE_THRESHOLD;
            for p in &bank.in_flight {
                all_in_flight.push((p.id.clone(), p.x, p.z, p.heading, is_heavy));
            }
        }
    }

    let live: HashSet<String> = all_in_flight.iter().map(|(u, ..)| u.clone()).collect();
    let tracked: HashSet<String> = state.active.keys().cloned().collect();
    let (to_spawn, to_despawn) = diff_torpedo_sets(&live, &tracked);

    for uuid in to_despawn {
        if let Some(entities) = state.active.remove(&uuid) {
            commands.entity(entities.glow_a).try_despawn();
            commands.entity(entities.glow_b).try_despawn();
            commands.entity(entities.core_a).try_despawn();
            commands.entity(entities.core_b).try_despawn();
            spawn_blaster_impact_burst(
                entities.last_pos,
                &phaser_pfx_assets,
                &mut commands,
                &mut meshes,
                &mut materials,
            );
        }
    }

    for uuid in to_spawn {
        if let Some((_, x, z, heading, is_heavy)) = all_in_flight.iter().find(|(u, ..)| u == &uuid)
        {
            let pos = Vec3::new(*x, 0.1, *z);
            spawn_blaster_bolt(
                uuid,
                pos,
                *heading,
                *is_heavy,
                &bolt_pfx_assets,
                &mut state,
                &mut commands,
                &mut meshes,
                &mut materials,
            );
        }
    }

    for (uuid, x, z, heading, _) in &all_in_flight {
        let pos = Vec3::new(*x, 0.1, *z);
        update_blaster_bolt(
            uuid,
            pos,
            *heading,
            &mut state,
            &mut commands,
            &mut meshes,
            &mut materials,
            &mut body_q,
        );
    }
}

/// The bolt's forward direction from its `heading` (radians, ship-forward
/// convention `atan2(dx, -dz)` — see `crates/phoenix-sim-gameplay/src/weapons/blaster.rs`).
fn blaster_bolt_forward(heading: f32) -> Vec3 {
    Vec3::new(heading.sin(), 0.0, -heading.cos())
}

/// Spawns the crossed-quad bolt body for a newly-appeared projectile, plus
/// its muzzle flash. Called once per projectile, the tick it first shows up
/// in `in_flight` — which is also the correct moment for the muzzle flash
/// (spec: "the muzzle flash should begin at the moment the projectile is
/// spawned, not one frame afterward").
#[allow(clippy::too_many_arguments)]
fn spawn_blaster_bolt(
    uuid: String,
    pos: Vec3,
    heading: f32,
    is_heavy: bool,
    pfx_assets: &BlasterBoltPfxAssets,
    state: &mut BlasterPfxState,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
) {
    let (half_len, glow_width, core_width, color) = if is_heavy {
        (
            BLASTER_SPHERE_BOLT_LENGTH * 0.5,
            BLASTER_SPHERE_GLOW_WIDTH,
            BLASTER_SPHERE_CORE_WIDTH,
            BLASTER_SPHERE_COLOR,
        )
    } else {
        (
            BLASTER_BOLT_LENGTH * 0.5,
            BLASTER_BOLT_GLOW_WIDTH,
            BLASTER_BOLT_CORE_WIDTH,
            BLASTER_BOLT_COLOR,
        )
    };
    let forward = blaster_bolt_forward(heading);
    let tail = pos - forward * half_len;
    let front = pos + forward * half_len;
    let cross = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);

    let ribbon_mesh = meshes.add(unit_ribbon_quad_mesh());
    let glow_color = [color[0], color[1], color[2], color[3] * 0.8];
    let core_color = [
        color[0] * 0.3 + 0.7,
        color[1] * 0.3 + 0.7,
        color[2] * 0.3 + 0.7,
        color[3],
    ];
    let glow_mat = phaser_texture_material(
        materials,
        pfx_assets.bolt_glow.clone(),
        glow_color,
        BLASTER_EMISSIVE * 0.7,
    );
    let core_mat = phaser_texture_material(
        materials,
        pfx_assets.bolt_core.clone(),
        core_color,
        BLASTER_EMISSIVE,
    );

    let glow_t = segment_transform(tail, front, glow_width);
    let core_t = segment_transform(tail, front, core_width);

    let glow_a = commands
        .spawn((
            PfxEntity,
            BlasterBolt,
            Mesh3d(ribbon_mesh.clone()),
            MeshMaterial3d(glow_mat.clone()),
            glow_t,
        ))
        .id();
    let glow_b = commands
        .spawn((
            PfxEntity,
            BlasterBolt,
            Mesh3d(ribbon_mesh.clone()),
            MeshMaterial3d(glow_mat),
            Transform {
                rotation: glow_t.rotation * cross,
                ..glow_t
            },
        ))
        .id();
    let core_a = commands
        .spawn((
            PfxEntity,
            BlasterBolt,
            Mesh3d(ribbon_mesh.clone()),
            MeshMaterial3d(core_mat.clone()),
            core_t,
        ))
        .id();
    let core_b = commands
        .spawn((
            PfxEntity,
            BlasterBolt,
            Mesh3d(ribbon_mesh),
            MeshMaterial3d(core_mat),
            Transform {
                rotation: core_t.rotation * cross,
                ..core_t
            },
        ))
        .id();

    spawn_blaster_muzzle_flash(tail, color, commands, meshes, materials);

    state.active.insert(
        uuid,
        BlasterPfxEntities {
            glow_a,
            glow_b,
            core_a,
            core_b,
            last_pos: pos,
            half_len,
            glow_width,
            core_width,
            color,
        },
    );
}

/// Updates an already-live bolt's transform to its new position/heading each
/// frame, and lays down a short fading trail segment behind it once it has
/// moved far enough (spec: "a blaster bolt does not usually need a long
/// continuous trail — a short afterimage is enough").
fn update_blaster_bolt(
    uuid: &str,
    pos: Vec3,
    heading: f32,
    state: &mut BlasterPfxState,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    body_q: &mut Query<&mut Transform, With<BlasterBolt>>,
) {
    let Some(entities) = state.active.get_mut(uuid) else {
        return;
    };

    if entities.last_pos.distance(pos) >= BLASTER_TRAIL_MIN_DISTANCE {
        spawn_trail_segment(
            entities.last_pos,
            pos,
            entities.glow_width * BLASTER_TRAIL_WIDTH_SCALE,
            [
                entities.color[0],
                entities.color[1],
                entities.color[2],
                entities.color[3] * 0.5,
            ],
            BLASTER_EMISSIVE * 0.5,
            BLASTER_TRAIL_LIFETIME_SECS,
            commands,
            meshes,
            materials,
        );
    }
    entities.last_pos = pos;

    let forward = blaster_bolt_forward(heading);
    let tail = pos - forward * entities.half_len;
    let front = pos + forward * entities.half_len;
    let glow_width = entities.glow_width;
    let core_width = entities.core_width;
    let cross = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
    let glow_t = segment_transform(tail, front, glow_width);
    let core_t = segment_transform(tail, front, core_width);

    if let Ok(mut t) = body_q.get_mut(entities.glow_a) {
        *t = glow_t;
    }
    if let Ok(mut t) = body_q.get_mut(entities.glow_b) {
        *t = Transform {
            rotation: glow_t.rotation * cross,
            ..glow_t
        };
    }
    if let Ok(mut t) = body_q.get_mut(entities.core_a) {
        *t = core_t;
    }
    if let Ok(mut t) = body_q.get_mut(entities.core_b) {
        *t = Transform {
            rotation: core_t.rotation * cross,
            ..core_t
        };
    }
}

/// Brief bright flash establishing the bolt's origin point, reusing the
/// generic radial-glow texture (same shape as the phaser muzzle flash —
/// only color, size and lifetime differ per weapon).
fn spawn_blaster_muzzle_flash(
    pos: Vec3,
    color: [f32; 4],
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
) {
    spawn_burst_sprite(
        pos,
        meshes.add(unit_billboard_mesh()),
        BurstSprite {
            texture: None,
            color,
            emissive_strength: BLASTER_EMISSIVE * 1.4,
            lifetime: BLASTER_MUZZLE_FLASH_LIFETIME_SECS,
            start_scale: BLASTER_MUZZLE_FLASH_START_SIZE,
            end_scale: Some(BLASTER_MUZZLE_FLASH_END_SIZE),
        },
        commands,
        materials,
    );
}

/// One-shot impact burst where a bolt disappears (hit or expiry): an
/// expanding ring plus a handful of outward sparks, reusing the phaser's
/// generic impact textures per the "separate weapon energy from surface
/// response" pattern in the design spec.
fn spawn_blaster_impact_burst(
    pos: Vec3,
    phaser_pfx_assets: &PhaserPfxAssets,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
) {
    let billboard_mesh = meshes.add(unit_billboard_mesh());
    let color = BLASTER_BOLT_COLOR;
    let ring_color = [color[0], color[1], color[2], color[3] * 0.9];
    spawn_burst_sprite(
        pos,
        billboard_mesh.clone(),
        BurstSprite {
            texture: Some(phaser_pfx_assets.impact_ring.clone()),
            color: ring_color,
            emissive_strength: BLASTER_EMISSIVE,
            lifetime: BLASTER_IMPACT_RING_LIFETIME_SECS,
            start_scale: BLASTER_IMPACT_RING_START_SIZE,
            end_scale: Some(BLASTER_IMPACT_RING_END_SIZE),
        },
        commands,
        materials,
    );

    let mut rng = rand::rng();
    for _ in 0..BLASTER_IMPACT_SPARK_COUNT {
        let offset = Vec3::new(
            rng.random_range(-1.0_f32..1.0),
            rng.random_range(-0.3_f32..0.3),
            rng.random_range(-1.0_f32..1.0),
        )
        .normalize_or_zero()
            * BLASTER_IMPACT_SPARK_SPREAD;
        let spark_color = [
            color[0] * 0.5 + 0.5,
            color[1] * 0.5 + 0.5,
            color[2] * 0.5 + 0.5,
            color[3],
        ];
        spawn_burst_sprite(
            pos + offset,
            billboard_mesh.clone(),
            BurstSprite {
                texture: Some(phaser_pfx_assets.spark_streak.clone()),
                color: spark_color,
                emissive_strength: BLASTER_EMISSIVE,
                lifetime: BLASTER_IMPACT_SPARK_LIFETIME_SECS,
                start_scale: BLASTER_IMPACT_SPARK_SIZE,
                end_scale: None,
            },
            commands,
            materials,
        );
    }
}

// ── Ship death explosion (issue #825) ───────────────────────────────────────
//
// One reusable explosion asset set, scaled per ship by `ShipDestroyedVfx::
// radius` (the destroyed entity's `[collider]` TOML radius). Layers, per the
// sci-fi explosion PFX guide: a bright primary flash, several irregular
// "plasma core" puffs, a few larger/dimmer/longer-lived "vapour cloud"
// puffs, an expanding shockwave ring, and a scatter of fading sparks —
// deliberately skipping the guide's mesh-debris/secondary-detonation/
// velocity-inheritance layers (no established convention for moving PFX
// particles in this codebase yet; every existing burst here is a
// static-position scale+fade, matching `spawn_blaster_impact_burst`).

const EXPLOSION_FLASH_LIFETIME_SECS: f32 = 0.1;
const EXPLOSION_FLASH_START_SCALE: f32 = 0.4;
const EXPLOSION_FLASH_END_SCALE: f32 = 1.4;
const EXPLOSION_FLASH_COLOR: [f32; 4] = [1.0, 0.98, 0.9, 1.0];
const EXPLOSION_FLASH_EMISSIVE: f32 = 9.0;

const EXPLOSION_CORE_PUFF_COUNT: usize = 6;
const EXPLOSION_CORE_PUFF_LIFETIME_SECS: f32 = 0.6;
const EXPLOSION_CORE_PUFF_START_SCALE: f32 = 0.5;
const EXPLOSION_CORE_PUFF_END_SCALE: f32 = 0.9;
const EXPLOSION_CORE_PUFF_SPREAD: f32 = 0.35;
const EXPLOSION_CORE_COLOR: [f32; 4] = [1.0, 0.65, 0.25, 0.95];
const EXPLOSION_CORE_EMISSIVE: f32 = 6.0;

const EXPLOSION_CLOUD_PUFF_COUNT: usize = 4;
const EXPLOSION_CLOUD_PUFF_LIFETIME_SECS: f32 = 2.0;
const EXPLOSION_CLOUD_PUFF_START_SCALE: f32 = 0.6;
const EXPLOSION_CLOUD_PUFF_END_SCALE: f32 = 1.8;
const EXPLOSION_CLOUD_PUFF_SPREAD: f32 = 0.5;
const EXPLOSION_CLOUD_COLOR: [f32; 4] = [0.9, 0.35, 0.12, 0.55];
const EXPLOSION_CLOUD_EMISSIVE: f32 = 2.5;

const EXPLOSION_RING_LIFETIME_SECS: f32 = 0.5;
const EXPLOSION_RING_START_SCALE: f32 = 0.3;
const EXPLOSION_RING_END_SCALE: f32 = 3.5;
const EXPLOSION_RING_COLOR: [f32; 4] = [1.0, 0.7, 0.35, 0.5];
const EXPLOSION_RING_EMISSIVE: f32 = 4.0;

const EXPLOSION_SPARK_COUNT: usize = 10;
const EXPLOSION_SPARK_LIFETIME_SECS: f32 = 0.5;
const EXPLOSION_SPARK_SCALE: f32 = 0.4;
const EXPLOSION_SPARK_SPREAD: f32 = 0.8;
const EXPLOSION_SPARK_COLOR: [f32; 4] = [1.0, 0.55, 0.2, 1.0];
const EXPLOSION_SPARK_EMISSIVE: f32 = 6.0;

/// Texture handle for the one new explosion-specific asset — an irregular
/// "plasma puff" blob reused (at different scale/colour/lifetime) for both
/// the hot core and the cooler outer cloud layers. The flash, shockwave
/// ring and sparks reuse `PhaserPfxAssets`' generic radial-glow/ring/streak
/// textures, same as the blaster and phaser impact bursts.
#[derive(Resource)]
struct ShipExplosionPfxAssets {
    puff: Handle<Image>,
}

impl FromWorld for ShipExplosionPfxAssets {
    fn from_world(world: &mut World) -> Self {
        let asset_server = world.resource::<AssetServer>();
        Self {
            puff: asset_server.load("pfx/explosion/explosion_puff.png"),
        }
    }
}

/// Spawns a death explosion for every `ShipDestroyedVfx` fired this tick
/// (phaser/blaster/torpedo kills alike — see `console::weapons::server`).
fn spawn_ship_explosions(
    mut events: MessageReader<ShipDestroyedVfx>,
    explosion_assets: Res<ShipExplosionPfxAssets>,
    phaser_pfx_assets: Res<PhaserPfxAssets>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    for ev in events.read() {
        let pos = Vec3::new(ev.x, 0.1, ev.z);
        let radius = ev.radius.max(0.5);
        let billboard_mesh = meshes.add(unit_billboard_mesh());
        let mut rng = rand::rng();

        // Primary flash — the brightest, briefest moment.
        spawn_burst_sprite(
            pos,
            billboard_mesh.clone(),
            BurstSprite {
                texture: Some(phaser_pfx_assets.radial_glow.clone()),
                color: EXPLOSION_FLASH_COLOR,
                emissive_strength: EXPLOSION_FLASH_EMISSIVE,
                lifetime: EXPLOSION_FLASH_LIFETIME_SECS,
                start_scale: EXPLOSION_FLASH_START_SCALE * radius,
                end_scale: Some(EXPLOSION_FLASH_END_SCALE * radius),
            },
            &mut commands,
            &mut materials,
        );

        // Hot plasma core — several irregular puffs, short-lived, bright.
        for _ in 0..EXPLOSION_CORE_PUFF_COUNT {
            let offset = random_horizontal_offset(&mut rng, EXPLOSION_CORE_PUFF_SPREAD * radius);
            spawn_burst_sprite(
                pos + offset,
                billboard_mesh.clone(),
                BurstSprite {
                    texture: Some(explosion_assets.puff.clone()),
                    color: EXPLOSION_CORE_COLOR,
                    emissive_strength: EXPLOSION_CORE_EMISSIVE,
                    lifetime: EXPLOSION_CORE_PUFF_LIFETIME_SECS,
                    start_scale: EXPLOSION_CORE_PUFF_START_SCALE * radius,
                    end_scale: Some(EXPLOSION_CORE_PUFF_END_SCALE * radius),
                },
                &mut commands,
                &mut materials,
            );
        }

        // Outer vapour cloud — fewer, larger, dimmer, longer-lived puffs.
        for _ in 0..EXPLOSION_CLOUD_PUFF_COUNT {
            let offset = random_horizontal_offset(&mut rng, EXPLOSION_CLOUD_PUFF_SPREAD * radius);
            spawn_burst_sprite(
                pos + offset,
                billboard_mesh.clone(),
                BurstSprite {
                    texture: Some(explosion_assets.puff.clone()),
                    color: EXPLOSION_CLOUD_COLOR,
                    emissive_strength: EXPLOSION_CLOUD_EMISSIVE,
                    lifetime: EXPLOSION_CLOUD_PUFF_LIFETIME_SECS,
                    start_scale: EXPLOSION_CLOUD_PUFF_START_SCALE * radius,
                    end_scale: Some(EXPLOSION_CLOUD_PUFF_END_SCALE * radius),
                },
                &mut commands,
                &mut materials,
            );
        }

        // Expanding shockwave ring.
        spawn_burst_sprite(
            pos,
            billboard_mesh.clone(),
            BurstSprite {
                texture: Some(phaser_pfx_assets.impact_ring.clone()),
                color: EXPLOSION_RING_COLOR,
                emissive_strength: EXPLOSION_RING_EMISSIVE,
                lifetime: EXPLOSION_RING_LIFETIME_SECS,
                start_scale: EXPLOSION_RING_START_SCALE * radius,
                end_scale: Some(EXPLOSION_RING_END_SCALE * radius),
            },
            &mut commands,
            &mut materials,
        );

        // Sparks scattered omnidirectionally (not a directional impact, so
        // no incoming-shot bias like `spawn_blaster_impact_burst`'s sparks).
        for _ in 0..EXPLOSION_SPARK_COUNT {
            let offset = random_horizontal_offset(&mut rng, EXPLOSION_SPARK_SPREAD * radius);
            spawn_burst_sprite(
                pos + offset,
                billboard_mesh.clone(),
                BurstSprite {
                    texture: Some(phaser_pfx_assets.spark_streak.clone()),
                    color: EXPLOSION_SPARK_COLOR,
                    emissive_strength: EXPLOSION_SPARK_EMISSIVE,
                    lifetime: EXPLOSION_SPARK_LIFETIME_SECS,
                    start_scale: EXPLOSION_SPARK_SCALE * radius,
                    end_scale: None,
                },
                &mut commands,
                &mut materials,
            );
        }
    }
}

/// A random offset within `spread` of the origin, mostly in the XZ plane
/// with a small vertical component — same convention as the impact-spark
/// offsets in `spawn_blaster_impact_burst` / `spawn_impact_burst`.
fn random_horizontal_offset(rng: &mut impl Rng, spread: f32) -> Vec3 {
    Vec3::new(
        rng.random_range(-1.0_f32..1.0),
        rng.random_range(-0.3_f32..0.3),
        rng.random_range(-1.0_f32..1.0),
    )
    .normalize_or_zero()
        * spread
        * rng.random_range(0.2_f32..1.0)
}

/// Updates per-ship engine trail ribbons (mesh + material) each frame.
///
/// Iterates every ship (player + NPC) uniformly. The key-base string
/// distinguishes ships by UUID; the LocalShip falls back to "engine:player"
/// only if it somehow has no `EntityUuid` (defensive — normally it does).
fn spawn_engine_trails(
    time: Res<Time>,
    mut state: ResMut<EngineTrailState>,
    textures: Option<Res<EngineTrailTextures>>,
    ships_q: Query<
        (
            &Transform,
            &ShipPhysics,
            Option<&ModelMarkers>,
            Option<&HelmConsoleSection>,
            Option<&EntityUuid>,
            Has<LocalShip>,
        ),
        With<crate::server_app::Ship>,
    >,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<EngineTrailMaterial>>,
) {
    // `load_engine_trail_textures` (Startup) always runs before the first
    // Update, but guard anyway so a plugin ordering change fails soft
    // instead of panicking mid-frame.
    let Some(textures) = textures else {
        return;
    };

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
        let cfg = helm.and_then(|h| h.0.engine_pfx.as_ref());
        let settings = EnginePfxSettings::from_config(cfg);
        update_engine_trail(
            &key_base,
            transform,
            markers,
            cfg,
            physics.forward_speed,
            max_speed,
            dt,
            &settings,
            &textures,
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
            commands.entity(trail.entity).try_despawn();
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn update_engine_trail(
    key_base: &str,
    transform: &Transform,
    markers: Option<&ModelMarkers>,
    cfg: Option<&EnginePfxConfig>,
    forward_speed: f32,
    max_speed: f32,
    dt: f32,
    settings: &EnginePfxSettings,
    textures: &EngineTrailTextures,
    state: &mut EngineTrailState,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<EngineTrailMaterial>,
) {
    let motion = classify_engine_trail_motion(forward_speed, max_speed);
    let emitters = engine_emitters(transform, markers, cfg);
    for (emitter_idx, emitter) in emitters.iter().enumerate() {
        let key = format!("{}:{}", key_base, emitter_idx);

        // An astern-moving ship has no forward exhaust trail. Keep an idle
        // ship's existing fade behaviour, but clear a previously forward trail
        // immediately as soon as its signed forward speed goes negative.
        if motion == EngineTrailMotion::Reverse {
            if let Some(trail) = state.emitters.get_mut(&key) {
                clear_engine_trail_crumbs(&mut trail.crumbs);
                if let Some(mesh) = meshes.get_mut(&trail.mesh_handle) {
                    build_ribbon_into_mesh(mesh, &trail.crumbs, 0.0);
                }
            }
            continue;
        }

        let normalized_speed = match motion {
            EngineTrailMotion::Forward(normalized_speed) => normalized_speed,
            EngineTrailMotion::Idle | EngineTrailMotion::Reverse => 0.0,
        };

        let geometry = settings.geometry_for(emitter.is_marker_attached);
        let width = ENGINE_TRAIL_RADIUS * normalized_speed.max(0.35) * geometry.scale;

        // Lazily create the ribbon entity and mesh for this emitter.
        if !state.emitters.contains_key(&key) {
            let mesh_handle = meshes.add(empty_ribbon_mesh());
            let mat_handle = trail_ribbon_material(materials, textures, settings.color);
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
                emitter.origin,
                width,
                settings.lifetime_secs,
            );
        }

        // Rebuild the ribbon mesh in place.
        if let Some(mesh) = meshes.get_mut(&trail.mesh_handle) {
            let render_crumbs = if normalized_speed > 0.05 {
                render_crumbs_from_marker(
                    &trail.crumbs,
                    emitter.origin,
                    emitter.direction,
                    width,
                    settings.lifetime_secs,
                )
            } else {
                trail.crumbs.clone()
            };
            build_ribbon_into_mesh(mesh, &render_crumbs, geometry.roll_radians);
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

fn clear_engine_trail_crumbs(crumbs: &mut VecDeque<TrailCrumb>) {
    crumbs.clear();
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
    materials: &mut Assets<EngineTrailMaterial>,
    textures: &EngineTrailTextures,
    color: [f32; 4],
) -> Handle<EngineTrailMaterial> {
    materials.add(EngineTrailMaterial {
        noise_texture: textures.noise.clone(),
        distortion_texture: textures.distortion.clone(),
        gradient_texture: textures.gradient.clone(),
        dissolve_texture: textures.dissolve.clone(),
        color_r: color[0],
        color_g: color[1],
        color_b: color[2],
        color_a: color[3],
        time: 0.0,
        scroll_speed: ENGINE_TRAIL_SCROLL_SPEED,
        distortion_strength: ENGINE_TRAIL_DISTORTION_STRENGTH,
        _pad0: 0.0,
    })
}

/// Rebuilds the ribbon geometry in-place from the ordered breadcrumb deque.
/// crumbs[0] is the newest point (near the ship), crumbs[n-1] is the oldest.
fn build_ribbon_into_mesh(mesh: &mut Mesh, crumbs: &VecDeque<TrailCrumb>, roll_radians: f32) {
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
        let perp = if tangent.length_squared() > 1e-6 {
            Quat::from_axis_angle(tangent.normalize(), roll_radians) * perp
        } else {
            perp
        };

        let age_frac = (crumb.age / crumb.lifetime.max(0.001)).clamp(0.0, 1.0);
        let hw = crumb.width * (1.0 - age_frac * ENGINE_TRAIL_AGE_WIDTH_FALLOFF) * 0.5;
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
            commands.entity(entity).try_despawn();
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
    mut blaster_state: ResMut<BlasterPfxState>,
    mut engine_state: ResMut<EngineTrailState>,
) {
    for entity in query.iter() {
        commands.entity(entity).try_despawn();
    }
    beam_state.active.clear();
    beam_state.target_point_choices.clear();
    torpedo_state.active.clear();
    blaster_state.active.clear();
    engine_state.emitters.clear();
}

/// Retire the active platform mote pool on accepted asset-stack replacement.
pub(crate) fn reset_pack_textures(world: &mut World) {
    super::native_visuals::reset_pack_textures(world);
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
    markers?.resolve_target_point_world_position(transform, target_point_index?)
}

fn marker_origin(
    transform: &Transform,
    markers: Option<&ModelMarkers>,
    marker_name: Option<&str>,
) -> Option<Vec3> {
    // Composes `entityTransform ∘ baseRig ∘ marker`: marker positions are
    // authored in the raw-GLB frame, so the base rig must be applied to place
    // the emitter on the correct (fore) end of the ship rather than the rear.
    markers?.resolve_world_position(transform, marker_name?)
}

fn marker_emitter(
    transform: &Transform,
    markers: Option<&ModelMarkers>,
    marker_name: Option<&str>,
) -> Option<(Vec3, Vec3)> {
    let markers = markers?;
    let name = marker_name?;
    Some((
        markers.resolve_world_position(transform, name)?,
        markers.resolve_world_direction(transform, name)?,
    ))
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

/// Every texture the dust field can reference for `world`, for asset preload.
///
/// The renderer's default mote textures are not spelled out in TOML, so walking the
/// world file alone would miss them and the textures would load lazily on the
/// first mote spawn — i.e. pop in mid-flight. Resolving the config here means
/// preload sees exactly what the emitter will ask for.
pub fn dust_texture_paths(world: Option<&crate::world::config::WorldConfig>) -> Vec<String> {
    let defaults = crate::world::native_render_config::PlatformRenderConfig::default();
    let cfg = world
        .and_then(|w| w.render.as_ref())
        .map(crate::world::config::RenderConfig::visuals)
        .unwrap_or(&defaults);
    if !cfg.motes || world.and_then(|w| w.dust.as_ref()).and_then(|d| d.enabled) == Some(false) {
        return Vec::new();
    }
    let mut paths = cfg.mote_textures.to_vec();
    paths.sort();
    paths.dedup();
    paths
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

#[derive(Clone, Copy, Debug)]
struct EngineEmitter {
    origin: Vec3,
    direction: Vec3,
    is_marker_attached: bool,
}

fn engine_emitters(
    transform: &Transform,
    markers: Option<&ModelMarkers>,
    cfg: Option<&EnginePfxConfig>,
) -> Vec<EngineEmitter> {
    let marker_emitters: Vec<EngineEmitter> = cfg
        .into_iter()
        .flat_map(|cfg| cfg.markers.iter())
        .filter_map(|name| {
            marker_emitter(transform, markers, Some(name.as_str())).map(|(origin, direction)| {
                EngineEmitter {
                    origin,
                    direction,
                    is_marker_attached: true,
                }
            })
        })
        .collect();
    if !marker_emitters.is_empty() {
        return marker_emitters;
    }

    let forward = transform.rotation * Vec3::NEG_Z;
    let aft = -forward.normalize_or_zero();
    vec![EngineEmitter {
        origin: transform.translation + aft * 3.0,
        direction: aft,
        is_marker_attached: false,
    }]
}

struct EnginePfxSettings {
    color: [f32; 4],
    lifetime_secs: f32,
    roll_radians: f32,
    scale: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct EngineTrailGeometry {
    roll_radians: f32,
    scale: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum EngineTrailMotion {
    Reverse,
    Idle,
    Forward(f32),
}

fn classify_engine_trail_motion(forward_speed: f32, max_speed: f32) -> EngineTrailMotion {
    if forward_speed < 0.0 {
        EngineTrailMotion::Reverse
    } else {
        let normalized_speed = (forward_speed / max_speed.max(0.1)).clamp(0.0, 1.0);
        if normalized_speed > 0.05 {
            EngineTrailMotion::Forward(normalized_speed)
        } else {
            EngineTrailMotion::Idle
        }
    }
}

impl EnginePfxSettings {
    fn from_config(cfg: Option<&EnginePfxConfig>) -> Self {
        Self {
            color: cfg.and_then(|c| c.color).unwrap_or(ENGINE_DEFAULT_COLOR),
            lifetime_secs: cfg
                .and_then(|c| c.trail_lifetime_secs)
                .unwrap_or(ENGINE_TRAIL_CRUMB_LIFETIME_SECS)
                .max(0.05),
            roll_radians: cfg.and_then(|c| c.roll_degrees).unwrap_or(0.0).to_radians(),
            scale: cfg.and_then(|c| c.scale).unwrap_or(1.0).max(0.0),
        }
    }

    fn geometry_for(&self, is_marker_attached: bool) -> EngineTrailGeometry {
        if is_marker_attached {
            EngineTrailGeometry {
                roll_radians: self.roll_radians,
                scale: self.scale,
            }
        } else {
            EngineTrailGeometry {
                roll_radians: 0.0,
                scale: 1.0,
            }
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
#[path = "pfx_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "pfx_burst_tests.rs"]
mod burst_tests;
