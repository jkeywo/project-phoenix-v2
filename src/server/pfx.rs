use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::reflect::TypePath;
use bevy::render::render_resource::AsBindGroup;
use bevy::shader::ShaderRef;
use rand::Rng;
use std::collections::{HashMap, HashSet, VecDeque};

use crate::beam_render;
use crate::console::weapons::{BlasterSystemResource, ShipDestroyedVfx};
use crate::entity_config::{EnginePfxConfig, PhaserBankConfig};
use crate::entity_spawner::{EntityUuid, HelmConsoleSection};
use crate::messages::GamePhase;
use crate::model_rig::ModelMarkers;
use crate::server::renderer::GameCamera;
use crate::ship_state::ShipPhysics;
use crate::simulation::{
    ActiveBeam, Asteroid, AsteroidUuid, LocalShip, PhaserRenderConfig, TorpedoSystemResource,
};
use crate::weapons_plugin::PhaserCombatConfigResource;

const BEAM_Y_OFFSET: f32 = 0.0;

// Textured phaser beam layers (issue: phaser-pfx-replacement). Widths are
// "radius" values fed into `segment_transform`, matching the old cylinder
// convention (final half-width = radius, since the unit ribbon mesh spans
// -1..1 in local X).
const BEAM_GLOW_WIDTH: f32 = 0.28;
const BEAM_CORE_WIDTH: f32 = 0.08;
const CONTACT_GLOW_SIZE: f32 = 0.5;

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

// Dust mote defaults (overridden by the [dust] world config block).
const DUST_SHADER: &str = "shaders/dust_mote.wgsl";

const DUST_SPEED_CURVE_EXPONENT: f32 = 2.0;
const DUST_LOW_SPEED_TINT: [f32; 3] = [0.55, 0.65, 0.75];
const DUST_HIGH_SPEED_TINT: [f32; 3] = [0.95, 0.98, 1.0];
// Streak length leads, brightness follows, density lags — see spec §10.
const DUST_STREAK_RESPONSE_SECS: f32 = 0.10;
const DUST_BRIGHTNESS_RESPONSE_SECS: f32 = 0.22;
const DUST_SPAWN_RESPONSE_SECS: f32 = 0.50;
const DUST_CENTRE_FADE_INNER: f32 = 0.15;
const DUST_CENTRE_FADE_OUTER: f32 = 0.55;
const DUST_EDGE_FADE: f32 = 0.12;
const DUST_TURBULENCE: f32 = 0.05;
const DUST_MOTE_SPEED_MULTIPLIER: f32 = 2.0;
/// Fallback max speed when no `HelmConsoleSection` is present on the local ship.
/// Matches the typical player ship `max_speed` in TOML; never used in normal play.
const DUST_FALLBACK_MAX_SPEED: f32 = 12.5;

/// Below this fraction of max speed the field is fully idle. Prevents the
/// "snow in space" read when the ship is station-keeping (spec §4/§20).
const DUST_IDLE_SPEED_FRAC: f32 = 0.02;

/// How far behind the camera a mote travels before it is recycled. Also the
/// slack added to transit time so motes live until they are genuinely past.
const DUST_BEHIND_CAMERA_MARGIN: f32 = 5.0;

// Built-in near/mid/far layers, used when the world TOML declares no
// `[[dust.layer]]`. Ranged fields are [at_rest, at_full_speed]; figures follow
// the spec §13 emitter tables. `width` is a fraction of screen height, scaled
// by spawn depth at runtime — see `dust_view_height_at`.
const DUST_DEFAULT_LAYERS: [DustLayerDefaults; 3] = [
    DustLayerDefaults {
        texture: "pfx/space_mote_streak_head.png",
        max_motes: 24,
        spawn_rate: [0.0, 12.0],
        opacity: [0.2, 1.0],
        brightness: [0.8, 3.0],
        width: 0.03,
        length: [3.0, 20.0],
        max_lifetime_secs: 0.8,
        depth_band: [4.0, 25.0],
        edge_bias: 0.7,
        additive: true,
        glint_texture: Some("pfx/space_mote_glint_4point.png"),
        glint_chance: 0.02,
    },
    DustLayerDefaults {
        texture: "pfx/space_mote_streak_soft.png",
        max_motes: 160,
        spawn_rate: [5.0, 160.0],
        opacity: [0.1, 0.7],
        brightness: [0.3, 1.8],
        width: 0.012,
        length: [1.0, 12.0],
        max_lifetime_secs: 2.0,
        depth_band: [10.0, 70.0],
        edge_bias: 0.0,
        additive: true,
        glint_texture: None,
        glint_chance: 0.0,
    },
    DustLayerDefaults {
        texture: "pfx/space_mote_compact_core.png",
        max_motes: 220,
        spawn_rate: [10.0, 250.0],
        // Kept below the bloom threshold so distant motes stay subtle and the
        // screen doesn't white out at speed (spec §8/§20).
        opacity: [0.03, 0.25],
        brightness: [0.15, 0.8],
        width: 0.004,
        length: [1.0, 5.0],
        max_lifetime_secs: 4.0,
        depth_band: [40.0, 150.0],
        // Spec §13: uniform, with a slight central reduction.
        edge_bias: 0.1,
        // Alpha-blended rather than additive: far motes are numerous, and
        // additive stacking at this density is what fogs the scene.
        additive: false,
        glint_texture: None,
        glint_chance: 0.0,
    },
];

const DUST_WARP_TEXTURE: &str = "pfx/space_mote_streak_soft.png";
const DUST_WARP_MOTES: u32 = 40;
const DUST_WARP_WIDTH: f32 = 0.018;
const DUST_WARP_LENGTH_MULTIPLIER: f32 = 40.0;
const DUST_WARP_BRIGHTNESS: f32 = 1.6;
const DUST_WARP_ENTER_SECS: f32 = 0.4;
const DUST_WARP_EXIT_SECS: f32 = 0.6;

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
            .init_resource::<DustFieldState>()
            .init_resource::<PhaserPfxAssets>()
            .init_resource::<BlasterBoltPfxAssets>()
            .init_resource::<TorpedoPfxAssets>()
            .init_resource::<ShipExplosionPfxAssets>()
            // `spawn_ship_explosions` reads this message; registering it here
            // too (redundantly with `WeaponsPlugin`) means test apps that add
            // `PfxPlugin` without `WeaponsPlugin` still work — `add_message`
            // is idempotent (backed by `init_resource::<Messages<T>>`).
            .add_message::<ShipDestroyedVfx>()
            .add_plugins(MaterialPlugin::<DustMoteMaterial>::default())
            .add_plugins(MaterialPlugin::<EngineTrailMaterial>::default())
            .add_systems(Startup, load_engine_trail_textures)
            .add_systems(
                Update,
                (
                    sync_phaser_beams.run_if(in_state(GamePhase::InProgress)),
                    sync_torpedo_pfx.run_if(in_state(GamePhase::InProgress)),
                    sync_blaster_pfx.run_if(in_state(GamePhase::InProgress)),
                    spawn_ship_explosions.run_if(in_state(GamePhase::InProgress)),
                    spawn_engine_trails.run_if(in_state(GamePhase::InProgress)),
                    tick_engine_trail_materials,
                    tick_lifetime_pfx.run_if(in_state(GamePhase::InProgress)),
                    tick_bursts.run_if(in_state(GamePhase::InProgress)),
                    // Ordered: state feeds both the emitter and the materials.
                    tick_dust_state.run_if(in_state(GamePhase::InProgress)),
                    spawn_dust_motes
                        .after(tick_dust_state)
                        .run_if(in_state(GamePhase::InProgress)),
                    move_dust_motes
                        .after(tick_dust_state)
                        .run_if(in_state(GamePhase::InProgress)),
                    sync_dust_materials
                        .after(tick_dust_state)
                        .run_if(in_state(GamePhase::InProgress)),
                )
                    // These read ship `Transform`/`ShipPhysics`, which
                    // `sync_ship_position` (SimSet::Physics) writes each tick.
                    // Without this, the two systems have a genuine read/write
                    // conflict on `Transform` with no ordering constraint
                    // between them, so PFX can read a stale pre-physics
                    // transform depending on scheduler tie-breaking.
                    .after(crate::sim_sets::SimSet::Physics),
            )
            // Runs after tick_bursts/sync_phaser_beams so its camera-facing
            // rotation always wins for the frame (those systems write
            // Transform too, on the same textured-billboard entities).
            .add_systems(
                Update,
                billboard_face_camera
                    .after(sync_phaser_beams)
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

#[derive(Component)]
struct PfxEntity;

#[derive(Component)]
struct BeamBody;

#[derive(Component)]
struct BeamContactGlow;

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

/// Which emitter a live mote belongs to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum DustMoteKind {
    /// Index into `DustPfxSettings::layers`.
    Layer(usize),
    Warp,
}

/// Component distinguishing ambient dust motes from other PFX entities, and
/// carrying the per-mote variation that would otherwise need a per-mote
/// material (spec §4: uniform motes read as a flat screen overlay).
#[derive(Component)]
struct DustMote {
    kind: DustMoteKind,
    /// World-space width of this mote.
    width: f32,
    /// Per-mote multiplier on the layer's speed-driven streak length.
    length_scale: f32,
    /// Small constant lateral drift, breaking perfect velocity alignment.
    turbulence: Vec3,
}

/// Material for one dust layer. Textures are white-with-shape-in-alpha, so the
/// tint, brightness and opacity uniforms are what make a single material serve
/// every mote in its layer.
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
struct DustMoteMaterial {
    #[uniform(0)]
    tint_r: f32,
    #[uniform(0)]
    tint_g: f32,
    #[uniform(0)]
    tint_b: f32,
    #[uniform(0)]
    brightness: f32,
    #[uniform(0)]
    opacity: f32,
    #[uniform(0)]
    centre_fade_inner: f32,
    #[uniform(0)]
    centre_fade_outer: f32,
    #[uniform(0)]
    edge_fade: f32,
    #[texture(1)]
    #[sampler(2)]
    texture: Handle<Image>,
    /// Additive for near/mid, alpha-blended for far (spec §18).
    additive: bool,
}

impl Material for DustMoteMaterial {
    fn fragment_shader() -> ShaderRef {
        DUST_SHADER.into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        if self.additive {
            AlphaMode::Add
        } else {
            AlphaMode::Blend
        }
    }
}

/// Unit quad in the XY plane facing +Z, for velocity-aligned mote billboards.
///
/// The UVs deliberately run `u = 1` at `-X` to `u = 0` at `+X`, i.e. mirrored
/// versus the usual convention. `space_mote_streak_head.png` carries its bright
/// head at the low-U end, and the billboard aligns local +X with the mote's
/// direction of travel — so without this flip the head would trail the streak
/// instead of leading it, and the whole field would read as moving backwards.
fn dust_quad_mesh() -> Mesh {
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_POSITION,
        vec![
            [-0.5, -0.5, 0.0],
            [0.5, -0.5, 0.0],
            [0.5, 0.5, 0.0],
            [-0.5, 0.5, 0.0],
        ],
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 0.0, 1.0]; 4])
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_UV_0,
        vec![[1.0, 0.0], [0.0, 0.0], [0.0, 1.0], [1.0, 1.0]],
    )
    .with_inserted_indices(Indices::U32(vec![0, 1, 2, 0, 2, 3]))
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
            &pfx_assets,
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
        if let Ok(mut t) = glow_q.get_mut(existing.contact) {
            *t = Transform::from_translation(end).with_scale(Vec3::splat(CONTACT_GLOW_SIZE));
        }
        return;
    }

    // First tick this beam exists: build the crossed-ribbon body (broad
    // colour glow + narrow white-hot core, per-layer textured quads so the
    // beam never disappears when viewed edge-on), a camera-facing contact
    // glow at the endpoint, a brief muzzle flash at the origin, and an
    // impact burst (ring + sparks) at the endpoint.
    let ribbon_mesh = meshes.add(unit_ribbon_quad_mesh());
    let billboard_mesh = meshes.add(unit_billboard_mesh());

    let glow_color = [color[0], color[1], color[2], color[3] * 0.85];
    let core_color = [
        color[0] * 0.4 + 0.6,
        color[1] * 0.4 + 0.6,
        color[2] * 0.4 + 0.6,
        color[3],
    ];

    let glow_mat =
        phaser_texture_material(materials, pfx_assets.beam_glow.clone(), glow_color, 4.0);
    let core_mat =
        phaser_texture_material(materials, pfx_assets.beam_core.clone(), core_color, 7.0);
    let contact_mat =
        phaser_texture_material(materials, pfx_assets.radial_glow.clone(), core_color, 6.0);

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
    let mat = phaser_texture_material(materials, pfx_assets.radial_glow.clone(), color, 8.0);
    commands.spawn((
        PfxEntity,
        Billboard,
        Mesh3d(billboard_mesh.clone()),
        MeshMaterial3d(mat.clone()),
        Transform::from_translation(pos).with_scale(Vec3::splat(MUZZLE_FLASH_START_SIZE)),
        PfxLifetime {
            age: 0.0,
            lifetime: MUZZLE_FLASH_LIFETIME_SECS,
        },
        PfxBurst {
            start_scale: MUZZLE_FLASH_START_SIZE,
            end_scale: MUZZLE_FLASH_END_SIZE,
        },
        PfxFadingMaterial {
            handle: mat,
            color,
            emissive_strength: 8.0,
        },
    ));
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
    let ring_mat =
        phaser_texture_material(materials, pfx_assets.impact_ring.clone(), ring_color, 5.0);
    commands.spawn((
        PfxEntity,
        Billboard,
        Mesh3d(billboard_mesh.clone()),
        MeshMaterial3d(ring_mat.clone()),
        Transform::from_translation(pos).with_scale(Vec3::splat(IMPACT_RING_START_SIZE)),
        PfxLifetime {
            age: 0.0,
            lifetime: IMPACT_RING_LIFETIME_SECS,
        },
        PfxBurst {
            start_scale: IMPACT_RING_START_SIZE,
            end_scale: IMPACT_RING_END_SIZE,
        },
        PfxFadingMaterial {
            handle: ring_mat,
            color: ring_color,
            emissive_strength: 5.0,
        },
    ));

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
        let spark_mat =
            phaser_texture_material(materials, pfx_assets.spark_streak.clone(), spark_color, 6.0);
        commands.spawn((
            PfxEntity,
            Billboard,
            Mesh3d(billboard_mesh.clone()),
            MeshMaterial3d(spark_mat.clone()),
            Transform::from_translation(pos + offset).with_scale(Vec3::splat(IMPACT_SPARK_SIZE)),
            PfxLifetime {
                age: 0.0,
                lifetime: IMPACT_SPARK_LIFETIME_SECS,
            },
            PfxFadingMaterial {
                handle: spark_mat,
                color: spark_color,
                emissive_strength: 6.0,
            },
        ));
    }
}

/// Unit quad (local X width -1..1, local Y length -0.5..0.5) reused via
/// `segment_transform`'s (radius, length, radius) scale — matches the
/// convention the old unit `Cylinder` primitive used. UV maps local Y
/// (beam length) to U and local X (beam width) to V, since the source
/// beam-profile textures run horizontally (length) with the bright falloff
/// band across their vertical (width) axis.
fn unit_ribbon_quad_mesh() -> Mesh {
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
    let uvs: Vec<[f32; 2]> = vec![[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]];
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_indices(Indices::U32(vec![0, 1, 2, 0, 2, 3]));
    mesh
}

/// Unit quad (-0.5..0.5 both axes) for camera-facing billboards (muzzle
/// flash, contact glow, impact ring, sparks).
fn unit_billboard_mesh() -> Mesh {
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    let positions: Vec<[f32; 3]> = vec![
        [-0.5, -0.5, 0.0],
        [0.5, -0.5, 0.0],
        [0.5, 0.5, 0.0],
        [-0.5, 0.5, 0.0],
    ];
    let normals: Vec<[f32; 3]> = vec![[0.0, 0.0, 1.0]; 4];
    let uvs: Vec<[f32; 2]> = vec![[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]];
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_indices(Indices::U32(vec![0, 1, 2, 0, 2, 3]));
    mesh
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
/// player's. Torpedo UUIDs are globally unique (uuid::Uuid::new_v4), so
/// merging in-flight lists across ships never collides on tracker keys.
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
    ships_q: Query<&TorpedoSystemResource, With<crate::simulation::Ship>>,
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
    // Collect (uuid, x, z, heading) quadruples for every in-flight torpedo
    // across every ship. A single flat list makes the diff-against-tracker
    // trivial, and heading lets the flare orient correctly for homing
    // torpedoes that curve mid-flight.
    let mut all_in_flight: Vec<(String, f32, f32, f32)> = Vec::new();
    for torpedo_sys in ships_q.iter() {
        for t in &torpedo_sys.0.in_flight {
            all_in_flight.push((t.uuid.clone(), t.x, t.z, t.heading));
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
        if let Some((_, x, z, heading)) = all_in_flight.iter().find(|(u, ..)| u == &uuid) {
            let pos = Vec3::new(*x, 0.1, *z);
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

    for (uuid, x, z, heading) in &all_in_flight {
        let pos = Vec3::new(*x, 0.1, *z);
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
    let mat = phaser_texture_material(
        materials,
        phaser_pfx_assets.radial_glow.clone(),
        TORPEDO_COLOR,
        TORPEDO_CORE_EMISSIVE,
    );
    commands.spawn((
        PfxEntity,
        Billboard,
        Mesh3d(meshes.add(unit_billboard_mesh())),
        MeshMaterial3d(mat.clone()),
        Transform::from_translation(pos).with_scale(Vec3::splat(TORPEDO_LAUNCH_FLASH_START_SIZE)),
        PfxLifetime {
            age: 0.0,
            lifetime: TORPEDO_LAUNCH_FLASH_LIFETIME_SECS,
        },
        PfxBurst {
            start_scale: TORPEDO_LAUNCH_FLASH_START_SIZE,
            end_scale: TORPEDO_LAUNCH_FLASH_END_SIZE,
        },
        PfxFadingMaterial {
            handle: mat,
            color: TORPEDO_COLOR,
            emissive_strength: TORPEDO_CORE_EMISSIVE,
        },
    ));
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
    let flash_mat = phaser_texture_material(
        materials,
        phaser_pfx_assets.radial_glow.clone(),
        TORPEDO_CORE_COLOR,
        TORPEDO_CORE_EMISSIVE,
    );
    commands.spawn((
        PfxEntity,
        Billboard,
        Mesh3d(billboard_mesh.clone()),
        MeshMaterial3d(flash_mat.clone()),
        Transform::from_translation(pos)
            .with_scale(Vec3::splat(TORPEDO_IMPACT_FLASH_START_SIZE)),
        PfxLifetime {
            age: 0.0,
            lifetime: TORPEDO_IMPACT_FLASH_LIFETIME_SECS,
        },
        PfxBurst {
            start_scale: TORPEDO_IMPACT_FLASH_START_SIZE,
            end_scale: TORPEDO_IMPACT_FLASH_END_SIZE,
        },
        PfxFadingMaterial {
            handle: flash_mat,
            color: TORPEDO_CORE_COLOR,
            emissive_strength: TORPEDO_CORE_EMISSIVE,
        },
    ));

    // Irregular plasma bloom.
    let plasma_mat = phaser_texture_material(
        materials,
        explosion_pfx_assets.puff.clone(),
        TORPEDO_COLOR,
        TORPEDO_SHELL_EMISSIVE,
    );
    commands.spawn((
        PfxEntity,
        Billboard,
        Mesh3d(billboard_mesh.clone()),
        MeshMaterial3d(plasma_mat.clone()),
        Transform::from_translation(pos)
            .with_scale(Vec3::splat(TORPEDO_IMPACT_PLASMA_START_SCALE)),
        PfxLifetime {
            age: 0.0,
            lifetime: TORPEDO_IMPACT_PLASMA_LIFETIME_SECS,
        },
        PfxBurst {
            start_scale: TORPEDO_IMPACT_PLASMA_START_SCALE,
            end_scale: TORPEDO_IMPACT_PLASMA_END_SCALE,
        },
        PfxFadingMaterial {
            handle: plasma_mat,
            color: TORPEDO_COLOR,
            emissive_strength: TORPEDO_SHELL_EMISSIVE,
        },
    ));

    // Expanding ring.
    let ring_mat = phaser_texture_material(
        materials,
        phaser_pfx_assets.impact_ring.clone(),
        TORPEDO_COLOR,
        TORPEDO_SHELL_EMISSIVE,
    );
    commands.spawn((
        PfxEntity,
        Billboard,
        Mesh3d(billboard_mesh.clone()),
        MeshMaterial3d(ring_mat.clone()),
        Transform::from_translation(pos).with_scale(Vec3::splat(TORPEDO_IMPACT_RING_START_SCALE)),
        PfxLifetime {
            age: 0.0,
            lifetime: TORPEDO_IMPACT_RING_LIFETIME_SECS,
        },
        PfxBurst {
            start_scale: TORPEDO_IMPACT_RING_START_SCALE,
            end_scale: TORPEDO_IMPACT_RING_END_SCALE,
        },
        PfxFadingMaterial {
            handle: ring_mat,
            color: TORPEDO_COLOR,
            emissive_strength: TORPEDO_SHELL_EMISSIVE,
        },
    ));

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
        let spark_mat = phaser_texture_material(
            materials,
            phaser_pfx_assets.spark_streak.clone(),
            spark_color,
            TORPEDO_SHELL_EMISSIVE,
        );
        commands.spawn((
            PfxEntity,
            Billboard,
            Mesh3d(billboard_mesh.clone()),
            MeshMaterial3d(spark_mat.clone()),
            Transform::from_translation(pos + offset)
                .with_scale(Vec3::splat(TORPEDO_IMPACT_SPARK_SCALE)),
            PfxLifetime {
                age: 0.0,
                lifetime: TORPEDO_IMPACT_SPARK_LIFETIME_SECS,
            },
            PfxFadingMaterial {
                handle: spark_mat,
                color: spark_color,
                emissive_strength: TORPEDO_SHELL_EMISSIVE,
            },
        ));
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
        update_blaster_bolt(uuid, pos, *heading, &mut state, &mut commands, &mut meshes, &mut materials, &mut body_q);
    }
}

/// The bolt's forward direction from its `heading` (radians, ship-forward
/// convention `atan2(dx, -dz)` — see `src/weapons/blaster.rs`).
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
    let mat = glow_material(materials, color, BLASTER_EMISSIVE * 1.4, AlphaMode::Add);
    commands.spawn((
        PfxEntity,
        Billboard,
        Mesh3d(meshes.add(unit_billboard_mesh())),
        MeshMaterial3d(mat.clone()),
        Transform::from_translation(pos).with_scale(Vec3::splat(BLASTER_MUZZLE_FLASH_START_SIZE)),
        PfxLifetime {
            age: 0.0,
            lifetime: BLASTER_MUZZLE_FLASH_LIFETIME_SECS,
        },
        PfxBurst {
            start_scale: BLASTER_MUZZLE_FLASH_START_SIZE,
            end_scale: BLASTER_MUZZLE_FLASH_END_SIZE,
        },
        PfxFadingMaterial {
            handle: mat,
            color,
            emissive_strength: BLASTER_EMISSIVE * 1.4,
        },
    ));
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
    let ring_mat = phaser_texture_material(
        materials,
        phaser_pfx_assets.impact_ring.clone(),
        ring_color,
        BLASTER_EMISSIVE,
    );
    commands.spawn((
        PfxEntity,
        Billboard,
        Mesh3d(billboard_mesh.clone()),
        MeshMaterial3d(ring_mat.clone()),
        Transform::from_translation(pos).with_scale(Vec3::splat(BLASTER_IMPACT_RING_START_SIZE)),
        PfxLifetime {
            age: 0.0,
            lifetime: BLASTER_IMPACT_RING_LIFETIME_SECS,
        },
        PfxBurst {
            start_scale: BLASTER_IMPACT_RING_START_SIZE,
            end_scale: BLASTER_IMPACT_RING_END_SIZE,
        },
        PfxFadingMaterial {
            handle: ring_mat,
            color: ring_color,
            emissive_strength: BLASTER_EMISSIVE,
        },
    ));

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
        let spark_mat = phaser_texture_material(
            materials,
            phaser_pfx_assets.spark_streak.clone(),
            spark_color,
            BLASTER_EMISSIVE,
        );
        commands.spawn((
            PfxEntity,
            Billboard,
            Mesh3d(billboard_mesh.clone()),
            MeshMaterial3d(spark_mat.clone()),
            Transform::from_translation(pos + offset)
                .with_scale(Vec3::splat(BLASTER_IMPACT_SPARK_SIZE)),
            PfxLifetime {
                age: 0.0,
                lifetime: BLASTER_IMPACT_SPARK_LIFETIME_SECS,
            },
            PfxFadingMaterial {
                handle: spark_mat,
                color: spark_color,
                emissive_strength: BLASTER_EMISSIVE,
            },
        ));
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
        let flash_mat = phaser_texture_material(
            &mut materials,
            phaser_pfx_assets.radial_glow.clone(),
            EXPLOSION_FLASH_COLOR,
            EXPLOSION_FLASH_EMISSIVE,
        );
        commands.spawn((
            PfxEntity,
            Billboard,
            Mesh3d(billboard_mesh.clone()),
            MeshMaterial3d(flash_mat.clone()),
            Transform::from_translation(pos)
                .with_scale(Vec3::splat(EXPLOSION_FLASH_START_SCALE * radius)),
            PfxLifetime {
                age: 0.0,
                lifetime: EXPLOSION_FLASH_LIFETIME_SECS,
            },
            PfxBurst {
                start_scale: EXPLOSION_FLASH_START_SCALE * radius,
                end_scale: EXPLOSION_FLASH_END_SCALE * radius,
            },
            PfxFadingMaterial {
                handle: flash_mat,
                color: EXPLOSION_FLASH_COLOR,
                emissive_strength: EXPLOSION_FLASH_EMISSIVE,
            },
        ));

        // Hot plasma core — several irregular puffs, short-lived, bright.
        for _ in 0..EXPLOSION_CORE_PUFF_COUNT {
            let offset = random_horizontal_offset(&mut rng, EXPLOSION_CORE_PUFF_SPREAD * radius);
            let mat = phaser_texture_material(
                &mut materials,
                explosion_assets.puff.clone(),
                EXPLOSION_CORE_COLOR,
                EXPLOSION_CORE_EMISSIVE,
            );
            commands.spawn((
                PfxEntity,
                Billboard,
                Mesh3d(billboard_mesh.clone()),
                MeshMaterial3d(mat.clone()),
                Transform::from_translation(pos + offset)
                    .with_scale(Vec3::splat(EXPLOSION_CORE_PUFF_START_SCALE * radius)),
                PfxLifetime {
                    age: 0.0,
                    lifetime: EXPLOSION_CORE_PUFF_LIFETIME_SECS,
                },
                PfxBurst {
                    start_scale: EXPLOSION_CORE_PUFF_START_SCALE * radius,
                    end_scale: EXPLOSION_CORE_PUFF_END_SCALE * radius,
                },
                PfxFadingMaterial {
                    handle: mat,
                    color: EXPLOSION_CORE_COLOR,
                    emissive_strength: EXPLOSION_CORE_EMISSIVE,
                },
            ));
        }

        // Outer vapour cloud — fewer, larger, dimmer, longer-lived puffs.
        for _ in 0..EXPLOSION_CLOUD_PUFF_COUNT {
            let offset = random_horizontal_offset(&mut rng, EXPLOSION_CLOUD_PUFF_SPREAD * radius);
            let mat = phaser_texture_material(
                &mut materials,
                explosion_assets.puff.clone(),
                EXPLOSION_CLOUD_COLOR,
                EXPLOSION_CLOUD_EMISSIVE,
            );
            commands.spawn((
                PfxEntity,
                Billboard,
                Mesh3d(billboard_mesh.clone()),
                MeshMaterial3d(mat.clone()),
                Transform::from_translation(pos + offset)
                    .with_scale(Vec3::splat(EXPLOSION_CLOUD_PUFF_START_SCALE * radius)),
                PfxLifetime {
                    age: 0.0,
                    lifetime: EXPLOSION_CLOUD_PUFF_LIFETIME_SECS,
                },
                PfxBurst {
                    start_scale: EXPLOSION_CLOUD_PUFF_START_SCALE * radius,
                    end_scale: EXPLOSION_CLOUD_PUFF_END_SCALE * radius,
                },
                PfxFadingMaterial {
                    handle: mat,
                    color: EXPLOSION_CLOUD_COLOR,
                    emissive_strength: EXPLOSION_CLOUD_EMISSIVE,
                },
            ));
        }

        // Expanding shockwave ring.
        let ring_mat = phaser_texture_material(
            &mut materials,
            phaser_pfx_assets.impact_ring.clone(),
            EXPLOSION_RING_COLOR,
            EXPLOSION_RING_EMISSIVE,
        );
        commands.spawn((
            PfxEntity,
            Billboard,
            Mesh3d(billboard_mesh.clone()),
            MeshMaterial3d(ring_mat.clone()),
            Transform::from_translation(pos)
                .with_scale(Vec3::splat(EXPLOSION_RING_START_SCALE * radius)),
            PfxLifetime {
                age: 0.0,
                lifetime: EXPLOSION_RING_LIFETIME_SECS,
            },
            PfxBurst {
                start_scale: EXPLOSION_RING_START_SCALE * radius,
                end_scale: EXPLOSION_RING_END_SCALE * radius,
            },
            PfxFadingMaterial {
                handle: ring_mat,
                color: EXPLOSION_RING_COLOR,
                emissive_strength: EXPLOSION_RING_EMISSIVE,
            },
        ));

        // Sparks scattered omnidirectionally (not a directional impact, so
        // no incoming-shot bias like `spawn_blaster_impact_burst`'s sparks).
        for _ in 0..EXPLOSION_SPARK_COUNT {
            let offset = random_horizontal_offset(&mut rng, EXPLOSION_SPARK_SPREAD * radius);
            let mat = phaser_texture_material(
                &mut materials,
                phaser_pfx_assets.spark_streak.clone(),
                EXPLOSION_SPARK_COLOR,
                EXPLOSION_SPARK_EMISSIVE,
            );
            commands.spawn((
                PfxEntity,
                Billboard,
                Mesh3d(billboard_mesh.clone()),
                MeshMaterial3d(mat.clone()),
                Transform::from_translation(pos + offset)
                    .with_scale(Vec3::splat(EXPLOSION_SPARK_SCALE * radius)),
                PfxLifetime {
                    age: 0.0,
                    lifetime: EXPLOSION_SPARK_LIFETIME_SECS,
                },
                PfxFadingMaterial {
                    handle: mat,
                    color: EXPLOSION_SPARK_COLOR,
                    emissive_strength: EXPLOSION_SPARK_EMISSIVE,
                },
            ));
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
        With<crate::simulation::Ship>,
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
    normalized_speed: f32,
    dt: f32,
    settings: &EnginePfxSettings,
    textures: &EngineTrailTextures,
    state: &mut EngineTrailState,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<EngineTrailMaterial>,
) {
    let emitters = engine_emitters(transform, markers, cfg);
    for (emitter_idx, (origin, direction)) in emitters.iter().enumerate() {
        let key = format!("{}:{}", key_base, emitter_idx);

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
    mut dust_state: ResMut<DustFieldState>,
) {
    dust_state.reset();
    for entity in query.iter() {
        commands.entity(entity).try_despawn();
    }
    beam_state.active.clear();
    beam_state.target_point_choices.clear();
    torpedo_state.active.clear();
    blaster_state.active.clear();
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

/// The ship's true world-space velocity, combining forward and lateral motion.
///
/// At yaw 0 the ship faces `-Z`, so forward is `(sin y, 0, -cos y)` and
/// starboard is its right-hand perpendicular, `(cos y, 0, sin y)`. Folding in
/// `lateral_speed` is what makes the dust field react to strafing — deriving
/// drift from `forward_speed` alone leaves the field frozen while the ship
/// slides sideways.
///
/// `ShipPhysics` is an XZ-plane model with no vertical component, so Y is
/// always zero here by construction rather than by omission.
fn ship_velocity(physics: &ShipPhysics) -> Vec3 {
    let (sin_y, cos_y) = physics.yaw.sin_cos();
    let forward = Vec3::new(sin_y, 0.0, -cos_y);
    let starboard = Vec3::new(cos_y, 0.0, sin_y);
    forward * physics.forward_speed + starboard * physics.lateral_speed
}

/// Rotation aligning a mote billboard's local +X with `travel_dir` while
/// turning its face as close to the camera as possible (spec §6).
///
/// Gram-Schmidt: project `to_cam` onto the plane perpendicular to the streak
/// axis to get the quad normal, then complete the basis. When the mote travels
/// straight at or away from the camera the projection collapses and there is
/// no meaningful screen direction; we fall back to an arbitrary perpendicular
/// rather than emitting a NaN rotation. Those motes sit at the centre of the
/// screen with near-zero projected length, where the shader's centre fade
/// hides them anyway.
fn dust_billboard_rotation(travel_dir: Vec3, to_cam: Vec3) -> Quat {
    let x = travel_dir.normalize_or_zero();
    if x == Vec3::ZERO {
        return Quat::IDENTITY;
    }
    let projected = to_cam - x * to_cam.dot(x);
    let z = projected.normalize_or_zero();
    let z = if z == Vec3::ZERO {
        x.any_orthonormal_vector()
    } else {
        z
    };
    let y = z.cross(x);
    Quat::from_mat3(&Mat3::from_cols(x, y, z))
}

/// Reshape a uniform `[-1, 1]` sample toward the edges (`bias > 0`) or the
/// centre (`bias < 0`). Spec §13 wants near motes weighted to screen edges so
/// big close streaks stay peripheral.
fn dust_edge_shape(u: f32, bias: f32) -> f32 {
    let exponent = (1.0 - bias).clamp(0.05, 4.0);
    u.signum() * u.abs().powf(exponent)
}

/// Compile-time shape of a built-in layer, used when the world declares none.
///
/// `DustLayerConfig::name` has no counterpart here: it is an authoring label
/// that makes a `[[dust.layer]]` block readable, and the renderer identifies
/// layers by position.
struct DustLayerDefaults {
    texture: &'static str,
    max_motes: u32,
    spawn_rate: [f32; 2],
    opacity: [f32; 2],
    brightness: [f32; 2],
    width: f32,
    length: [f32; 2],
    max_lifetime_secs: f32,
    depth_band: [f32; 2],
    edge_bias: f32,
    additive: bool,
    glint_texture: Option<&'static str>,
    glint_chance: f32,
}

/// Interpolate an `[at_rest, at_full_speed]` pair by the curved speed `s`.
fn dust_ramp(range: [f32; 2], s: f32) -> f32 {
    range[0] + (range[1] - range[0]) * s
}

/// Exponential smoothing toward `target`, framerate-independent.
///
/// `response_secs` is the time constant: larger means laggier. Used to stagger
/// streak/brightness/density response so acceleration reads as immediate
/// without motes popping into existence (spec §10).
fn dust_smooth(current: f32, target: f32, response_secs: f32, dt: f32) -> f32 {
    if response_secs <= 0.0 {
        return target;
    }
    let alpha = 1.0 - (-dt / response_secs).exp();
    current + (target - current) * alpha
}

/// One resolved layer — defaults with world-config overrides applied.
#[derive(Clone, Debug)]
struct DustLayerSettings {
    texture: String,
    max_motes: u32,
    spawn_rate: [f32; 2],
    opacity: [f32; 2],
    brightness: [f32; 2],
    width: f32,
    length: [f32; 2],
    max_lifetime_secs: f32,
    depth_band: [f32; 2],
    edge_bias: f32,
    additive: bool,
    glint_texture: Option<String>,
    glint_chance: f32,
}

impl DustLayerSettings {
    fn from_config(
        defaults: &DustLayerDefaults,
        cfg: Option<&crate::world::config::DustLayerConfig>,
    ) -> Self {
        Self {
            texture: cfg
                .and_then(|c| c.texture.clone())
                .unwrap_or_else(|| defaults.texture.to_string()),
            max_motes: cfg.and_then(|c| c.max_motes).unwrap_or(defaults.max_motes),
            spawn_rate: cfg
                .and_then(|c| c.spawn_rate)
                .unwrap_or(defaults.spawn_rate),
            opacity: cfg.and_then(|c| c.opacity).unwrap_or(defaults.opacity),
            brightness: cfg
                .and_then(|c| c.brightness)
                .unwrap_or(defaults.brightness),
            width: cfg.and_then(|c| c.width).unwrap_or(defaults.width),
            length: cfg.and_then(|c| c.length).unwrap_or(defaults.length),
            max_lifetime_secs: cfg
                .and_then(|c| c.max_lifetime_secs)
                .unwrap_or(defaults.max_lifetime_secs),
            depth_band: cfg
                .and_then(|c| c.depth_band)
                .unwrap_or(defaults.depth_band),
            edge_bias: cfg.and_then(|c| c.edge_bias).unwrap_or(defaults.edge_bias),
            additive: cfg.and_then(|c| c.additive).unwrap_or(defaults.additive),
            glint_texture: cfg
                .and_then(|c| c.glint_texture.clone())
                .or_else(|| defaults.glint_texture.map(str::to_string)),
            glint_chance: cfg
                .and_then(|c| c.glint_chance)
                .unwrap_or(defaults.glint_chance),
        }
    }
}

/// Resolved warp-layer config (spec §14).
#[derive(Clone, Debug)]
struct DustWarpSettings {
    enabled: bool,
    texture: String,
    motes: u32,
    width: f32,
    length_multiplier: f32,
    brightness: f32,
    enter_secs: f32,
    exit_secs: f32,
}

/// Resolved dust config — constants with world-config overrides applied.
#[derive(Clone, Debug)]
struct DustPfxSettings {
    enabled: bool,
    speed_curve_exponent: f32,
    low_speed_tint: [f32; 3],
    high_speed_tint: [f32; 3],
    streak_response_secs: f32,
    brightness_response_secs: f32,
    spawn_response_secs: f32,
    centre_fade_inner: f32,
    centre_fade_outer: f32,
    edge_fade: f32,
    turbulence: f32,
    mote_speed_multiplier: f32,
    layers: Vec<DustLayerSettings>,
    warp: DustWarpSettings,
}

impl DustPfxSettings {
    fn from_world(world_config: Option<&crate::world::config::WorldConfig>) -> Self {
        let cfg = world_config.and_then(|wc| wc.dust.as_ref());

        // A world either declares its own layers or gets the built-in three.
        // Declared layers are matched to defaults positionally, so a world can
        // override just the near layer by declaring one `[[dust.layer]]`.
        let layers: Vec<DustLayerSettings> = match cfg {
            Some(c) if !c.layers.is_empty() => c
                .layers
                .iter()
                .enumerate()
                .map(|(i, layer)| {
                    let defaults = &DUST_DEFAULT_LAYERS[i.min(DUST_DEFAULT_LAYERS.len() - 1)];
                    DustLayerSettings::from_config(defaults, Some(layer))
                })
                .collect(),
            _ => DUST_DEFAULT_LAYERS
                .iter()
                .map(|d| DustLayerSettings::from_config(d, None))
                .collect(),
        };

        let warp_cfg = cfg.and_then(|c| c.warp.as_ref());
        let warp = DustWarpSettings {
            // Absent `[dust.warp]` means no warp field, matching the
            // "optional layer" framing in spec §14.
            enabled: warp_cfg.and_then(|w| w.enabled).unwrap_or(false),
            texture: warp_cfg
                .and_then(|w| w.texture.clone())
                .unwrap_or_else(|| DUST_WARP_TEXTURE.to_string()),
            motes: warp_cfg.and_then(|w| w.motes).unwrap_or(DUST_WARP_MOTES),
            width: warp_cfg.and_then(|w| w.width).unwrap_or(DUST_WARP_WIDTH),
            length_multiplier: warp_cfg
                .and_then(|w| w.length_multiplier)
                .unwrap_or(DUST_WARP_LENGTH_MULTIPLIER),
            brightness: warp_cfg
                .and_then(|w| w.brightness)
                .unwrap_or(DUST_WARP_BRIGHTNESS),
            enter_secs: warp_cfg
                .and_then(|w| w.enter_secs)
                .unwrap_or(DUST_WARP_ENTER_SECS),
            exit_secs: warp_cfg
                .and_then(|w| w.exit_secs)
                .unwrap_or(DUST_WARP_EXIT_SECS),
        };

        Self {
            enabled: cfg.and_then(|c| c.enabled).unwrap_or(true),
            speed_curve_exponent: cfg
                .and_then(|c| c.speed_curve_exponent)
                .unwrap_or(DUST_SPEED_CURVE_EXPONENT),
            low_speed_tint: cfg
                .and_then(|c| c.low_speed_tint)
                .unwrap_or(DUST_LOW_SPEED_TINT),
            high_speed_tint: cfg
                .and_then(|c| c.high_speed_tint)
                .unwrap_or(DUST_HIGH_SPEED_TINT),
            streak_response_secs: cfg
                .and_then(|c| c.streak_response_secs)
                .unwrap_or(DUST_STREAK_RESPONSE_SECS),
            brightness_response_secs: cfg
                .and_then(|c| c.brightness_response_secs)
                .unwrap_or(DUST_BRIGHTNESS_RESPONSE_SECS),
            spawn_response_secs: cfg
                .and_then(|c| c.spawn_response_secs)
                .unwrap_or(DUST_SPAWN_RESPONSE_SECS),
            centre_fade_inner: cfg
                .and_then(|c| c.centre_fade_inner)
                .unwrap_or(DUST_CENTRE_FADE_INNER),
            centre_fade_outer: cfg
                .and_then(|c| c.centre_fade_outer)
                .unwrap_or(DUST_CENTRE_FADE_OUTER),
            edge_fade: cfg.and_then(|c| c.edge_fade).unwrap_or(DUST_EDGE_FADE),
            turbulence: cfg.and_then(|c| c.turbulence).unwrap_or(DUST_TURBULENCE),
            mote_speed_multiplier: cfg
                .and_then(|c| c.mote_speed_multiplier)
                .unwrap_or(DUST_MOTE_SPEED_MULTIPLIER),
            layers,
            warp,
        }
    }
}

/// Every texture the dust field can reference for `world`, for asset preload.
///
/// The renderer's built-in layers are not spelled out in TOML, so walking the
/// world file alone would miss them and the textures would load lazily on the
/// first mote spawn — i.e. pop in mid-flight. Resolving the config here means
/// preload sees exactly what the emitter will ask for.
pub fn dust_texture_paths(world: Option<&crate::world::config::WorldConfig>) -> Vec<String> {
    let cfg = DustPfxSettings::from_world(world);
    if !cfg.enabled {
        return Vec::new();
    }
    let mut out: Vec<String> = Vec::new();
    for layer in &cfg.layers {
        out.push(layer.texture.clone());
        if let Some(glint) = &layer.glint_texture {
            out.push(glint.clone());
        }
    }
    if cfg.warp.enabled {
        out.push(cfg.warp.texture.clone());
    }
    out.sort();
    out.dedup();
    out
}

/// Per-layer material handles, built lazily once the textures are known.
struct DustLayerMaterials {
    main: Handle<DustMoteMaterial>,
    glint: Option<Handle<DustMoteMaterial>>,
}

/// Live state of the dust field: the smoothed speed channels, per-layer spawn
/// accumulators, and the lazily-built shared assets.
#[derive(Resource, Default)]
struct DustFieldState {
    /// Curved speed, smoothed at three different rates (spec §10). Streak
    /// length leads so acceleration reads immediately; density lags so motes
    /// don't visibly pop in.
    streak_s: f32,
    brightness_s: f32,
    spawn_s: f32,
    /// Warp field ramp, 0 = absent, 1 = full warp.
    warp_ramp: f32,
    spawn_acc: Vec<f32>,
    warp_acc: f32,
    quad: Option<Handle<Mesh>>,
    layers: Vec<DustLayerMaterials>,
    warp_material: Option<Handle<DustMoteMaterial>>,
}

impl DustFieldState {
    /// Drop every cached handle so the next world rebuilds against its own
    /// `[dust]` block rather than reusing the previous world's textures.
    fn reset(&mut self) {
        *self = Self::default();
    }
}

fn dust_material(
    texture: Handle<Image>,
    additive: bool,
    cfg: &DustPfxSettings,
) -> DustMoteMaterial {
    DustMoteMaterial {
        tint_r: cfg.low_speed_tint[0],
        tint_g: cfg.low_speed_tint[1],
        tint_b: cfg.low_speed_tint[2],
        brightness: 0.0,
        opacity: 0.0,
        centre_fade_inner: cfg.centre_fade_inner,
        centre_fade_outer: cfg.centre_fade_outer,
        edge_fade: cfg.edge_fade,
        texture,
        additive,
    }
}

/// Builds the shared quad and the per-layer materials on first use.
fn ensure_dust_assets(
    state: &mut DustFieldState,
    cfg: &DustPfxSettings,
    asset_server: &AssetServer,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<DustMoteMaterial>,
) {
    if state.quad.is_none() {
        state.quad = Some(meshes.add(dust_quad_mesh()));
    }
    if state.layers.len() != cfg.layers.len() {
        state.layers =
            cfg.layers
                .iter()
                .map(|layer| DustLayerMaterials {
                    main: materials.add(dust_material(
                        asset_server.load(&layer.texture),
                        layer.additive,
                        cfg,
                    )),
                    glint: layer.glint_texture.as_ref().map(|path| {
                        materials.add(dust_material(asset_server.load(path), true, cfg))
                    }),
                })
                .collect();
        state.spawn_acc = vec![0.0; cfg.layers.len()];
    }
    if state.warp_material.is_none() {
        state.warp_material = Some(materials.add(dust_material(
            asset_server.load(&cfg.warp.texture),
            true,
            cfg,
        )));
    }
}

/// How long a mote spawned at `depth` needs to live to transit the volume and
/// pass behind the camera, capped by `max_secs` (spec §9).
///
/// Deriving this from transit time rather than reading a fixed duration is what
/// makes a mote's whole visible life useful: a fixed lifetime kills fast motes
/// while they are still distant specks and never lets them stream past. The cap
/// only bites at low speed, where transit time would otherwise be unbounded and
/// motes would hang in space.
fn dust_lifetime(depth: f32, mote_speed: f32, max_secs: f32, jitter: f32) -> f32 {
    let transit = if mote_speed > 0.01 {
        (depth + DUST_BEHIND_CAMERA_MARGIN) / mote_speed
    } else {
        max_secs
    };
    (transit * jitter).min(max_secs).max(0.05)
}

/// World height spanned by the viewport at `depth` from the camera.
///
/// Mote widths are authored as a fraction of screen height, not in world units
/// (spec §13's "screen-relative units"). Converting through this keeps a mote's
/// apparent size independent of the depth band it spawned in — otherwise the
/// far layer, sitting 40–150 units out, is sub-pixel and invisible while the
/// near layer is whatever its band happens to make it.
fn dust_view_height_at(depth: f32, fov: f32) -> f32 {
    2.0 * depth * (fov * 0.5).tan()
}

/// Normalised speed for the local ship, already through the speed curve.
///
/// The ceiling is impulse-inclusive so that ordinary flight occupies the lower
/// part of the curve and impulse is what drives the field to full white.
fn dust_speed_fraction(
    physics: &ShipPhysics,
    helm: Option<&HelmConsoleSection>,
    exponent: f32,
) -> f32 {
    let max_speed = helm
        .map(|h| h.0.max_speed)
        .unwrap_or(DUST_FALLBACK_MAX_SPEED)
        .max(0.1);
    let speed = ship_velocity(physics).length();
    (speed / max_speed).clamp(0.0, 1.0).powf(exponent.max(0.01))
}

/// Advances the smoothed speed channels and the warp ramp.
fn tick_dust_state(
    time: Res<Time>,
    world_config: Option<Res<crate::world::config::WorldConfig>>,
    mut state: ResMut<DustFieldState>,
    ship_q: Query<
        (
            &ShipPhysics,
            Option<&HelmConsoleSection>,
            Option<&crate::simulation::ShipImpulse>,
        ),
        With<LocalShip>,
    >,
) {
    let Ok((physics, helm, impulse)) = ship_q.single() else {
        return;
    };
    let cfg = DustPfxSettings::from_world(world_config.as_deref());
    let dt = time.delta_secs();
    let target = dust_speed_fraction(physics, helm, cfg.speed_curve_exponent);

    state.streak_s = dust_smooth(state.streak_s, target, cfg.streak_response_secs, dt);
    state.brightness_s = dust_smooth(state.brightness_s, target, cfg.brightness_response_secs, dt);
    state.spawn_s = dust_smooth(state.spawn_s, target, cfg.spawn_response_secs, dt);

    // Warp ramp. Charging exposes a 0→1 progress value we can ride directly;
    // disengage has no engine-side ramp (`cancel_charge` snaps Active → Idle
    // in one frame), so the spin-down is timed here off `exit_secs`.
    let warp_target = match impulse.map(|i| i.0.phase) {
        Some(crate::impulse::ImpulsePhase::Active) => 1.0,
        Some(crate::impulse::ImpulsePhase::Charging) => {
            impulse.map(|i| i.0.charge_progress).unwrap_or(0.0)
        }
        _ => 0.0,
    };
    let warp_response = if warp_target > state.warp_ramp {
        cfg.warp.enter_secs
    } else {
        cfg.warp.exit_secs
    };
    state.warp_ramp = if cfg.warp.enabled {
        dust_smooth(state.warp_ramp, warp_target, warp_response, dt)
    } else {
        0.0
    };
}

/// Pushes the smoothed speed values into the shared per-layer materials.
///
/// This is the whole reason a layer can share one material: everything that
/// varies with speed is a uniform, so this is a handful of writes per frame
/// rather than an allocation per mote.
fn sync_dust_materials(
    world_config: Option<Res<crate::world::config::WorldConfig>>,
    state: Res<DustFieldState>,
    mut materials: ResMut<Assets<DustMoteMaterial>>,
) {
    let cfg = DustPfxSettings::from_world(world_config.as_deref());

    // Ordinary layers yield the screen to the warp field as it ramps in.
    let layer_scale = 1.0 - state.warp_ramp;

    for (i, layer) in cfg.layers.iter().enumerate() {
        let Some(handles) = state.layers.get(i) else {
            continue;
        };
        let tint = dust_tint(&cfg, state.brightness_s);
        let brightness = dust_ramp(layer.brightness, state.brightness_s);
        let opacity = dust_ramp(layer.opacity, state.brightness_s) * layer_scale;
        for handle in [Some(&handles.main), handles.glint.as_ref()]
            .into_iter()
            .flatten()
        {
            if let Some(mat) = materials.get_mut(handle) {
                mat.tint_r = tint[0];
                mat.tint_g = tint[1];
                mat.tint_b = tint[2];
                mat.brightness = brightness;
                mat.opacity = opacity;
                mat.centre_fade_inner = cfg.centre_fade_inner;
                mat.centre_fade_outer = cfg.centre_fade_outer;
                mat.edge_fade = cfg.edge_fade;
            }
        }
    }

    if let Some(mat) = state
        .warp_material
        .as_ref()
        .and_then(|handle| materials.get_mut(handle))
    {
        let tint = cfg.high_speed_tint;
        mat.tint_r = tint[0];
        mat.tint_g = tint[1];
        mat.tint_b = tint[2];
        mat.brightness = cfg.warp.brightness;
        mat.opacity = state.warp_ramp;
        // Warp streaks cross the whole screen, so the centre mask that keeps
        // ordinary motes out of the targeting area would erase them.
        mat.centre_fade_inner = 0.0;
        mat.centre_fade_outer = 0.0;
        mat.edge_fade = cfg.edge_fade;
    }
}

/// Mote tint at the given curved speed: cool grey-blue at rest, near-white at
/// full speed (spec §7).
fn dust_tint(cfg: &DustPfxSettings, s: f32) -> [f32; 3] {
    let lo = cfg.low_speed_tint;
    let hi = cfg.high_speed_tint;
    [
        lo[0] + (hi[0] - lo[0]) * s,
        lo[1] + (hi[1] - lo[1]) * s,
        lo[2] + (hi[2] - lo[2]) * s,
    ]
}

/// Spawns camera-relative motes for each layer at a rate driven by ship speed.
#[allow(clippy::too_many_arguments)]
fn spawn_dust_motes(
    time: Res<Time>,
    world_config: Option<Res<crate::world::config::WorldConfig>>,
    mut state: ResMut<DustFieldState>,
    asset_server: Res<AssetServer>,
    cam_q: Query<(&Transform, &Projection), With<GameCamera>>,
    ship_q: Query<(&ShipPhysics, Option<&HelmConsoleSection>), With<LocalShip>>,
    mote_q: Query<&DustMote>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<DustMoteMaterial>>,
) {
    let Ok((cam_transform, projection)) = cam_q.single() else {
        return;
    };
    let Ok((physics, helm)) = ship_q.single() else {
        return;
    };
    let cfg = DustPfxSettings::from_world(world_config.as_deref());
    if !cfg.enabled {
        return;
    }

    ensure_dust_assets(&mut state, &cfg, &asset_server, &mut meshes, &mut materials);

    let velocity = ship_velocity(physics);
    // Motes travel opposite the ship, so their leading direction is -V.
    let travel_dir = (-velocity).normalize_or_zero();
    if travel_dir == Vec3::ZERO {
        return;
    }

    // Station-keeping shows no motes at all. Motes hanging in space at a crawl
    // are what read as snow rather than as motion (spec §4/§20), and the field
    // exists to communicate velocity — with no velocity there is nothing to say.
    let max_speed = helm
        .map(|h| h.0.max_speed)
        .unwrap_or(DUST_FALLBACK_MAX_SPEED)
        .max(0.1);
    if velocity.length() / max_speed < DUST_IDLE_SPEED_FRAC {
        return;
    }
    // Apparent mote speed — what transit-time lifetimes are computed against.
    let mote_speed = velocity.length() * cfg.mote_speed_multiplier;

    let dt = time.delta_secs();
    let (fov, aspect) = match projection {
        Projection::Perspective(p) => (p.fov, p.aspect_ratio),
        _ => (std::f32::consts::FRAC_PI_4, 1.777),
    };

    let mut live: Vec<u32> = vec![0; cfg.layers.len()];
    let mut live_warp = 0u32;
    for mote in mote_q.iter() {
        match mote.kind {
            DustMoteKind::Layer(i) if i < live.len() => live[i] += 1,
            DustMoteKind::Warp => live_warp += 1,
            _ => {}
        }
    }

    let mut rng = rand::rng();
    let cam_pos = cam_transform.translation;

    // The ordinary layers stand down while the warp field owns the screen.
    if state.warp_ramp < 0.99 {
        for (i, layer) in cfg.layers.iter().enumerate() {
            let rate = dust_ramp(layer.spawn_rate, state.spawn_s);
            let to_spawn = dust_take_spawn_budget(
                &mut state.spawn_acc[i],
                rate,
                layer.max_motes.saturating_sub(live[i]),
                dt,
            );
            for _ in 0..to_spawn {
                let depth = rng.random_range(
                    layer.depth_band[0]..layer.depth_band[1].max(layer.depth_band[0] + 0.001),
                );
                let pos = dust_spawn_position(
                    cam_pos,
                    cam_transform,
                    travel_dir,
                    depth,
                    fov,
                    aspect,
                    layer.edge_bias,
                    &mut rng,
                );
                let use_glint = layer.glint_chance > 0.0
                    && state.layers[i].glint.is_some()
                    && rng.random::<f32>() < layer.glint_chance;
                let material = if use_glint {
                    state.layers[i].glint.clone().expect("glint checked above")
                } else {
                    state.layers[i].main.clone()
                };
                let lifetime = dust_lifetime(
                    depth,
                    mote_speed,
                    layer.max_lifetime_secs,
                    rng.random_range(0.8..1.2),
                );
                commands.spawn((
                    PfxEntity,
                    DustMote {
                        kind: DustMoteKind::Layer(i),
                        width: layer.width
                            * dust_view_height_at(depth, fov)
                            * rng.random_range(0.7..1.3),
                        length_scale: rng.random_range(0.75..1.25),
                        turbulence: dust_turbulence(cfg.turbulence, &mut rng),
                    },
                    Mesh3d(state.quad.clone().expect("quad built above")),
                    MeshMaterial3d(material),
                    Transform::from_translation(pos),
                    PfxLifetime { age: 0.0, lifetime },
                ));
            }
        }
    }

    // Warp field (spec §14): a dedicated high-speed layer rather than the
    // ordinary motes stretched indefinitely.
    if cfg.warp.enabled && state.warp_ramp > 0.01 {
        let budget = cfg.warp.motes.saturating_sub(live_warp);
        let rate = cfg.warp.motes as f32 * 2.0 * state.warp_ramp;
        let to_spawn = dust_take_spawn_budget(&mut state.warp_acc, rate, budget, dt);
        let warp_material = state
            .warp_material
            .clone()
            .expect("warp material built above");
        for _ in 0..to_spawn {
            let depth = rng.random_range(10.0..90.0);
            let pos = dust_spawn_position(
                cam_pos,
                cam_transform,
                travel_dir,
                depth,
                fov,
                aspect,
                0.35,
                &mut rng,
            );
            commands.spawn((
                PfxEntity,
                DustMote {
                    kind: DustMoteKind::Warp,
                    width: cfg.warp.width
                        * dust_view_height_at(depth, fov)
                        * rng.random_range(0.6..1.4),
                    length_scale: rng.random_range(0.6..1.4),
                    turbulence: Vec3::ZERO,
                },
                Mesh3d(state.quad.clone().expect("quad built above")),
                MeshMaterial3d(warp_material.clone()),
                Transform::from_translation(pos),
                PfxLifetime {
                    age: 0.0,
                    lifetime: dust_lifetime(depth, mote_speed, 1.0, rng.random_range(0.8..1.2)),
                },
            ));
        }
    }
}

/// Draws from a fractional spawn accumulator, returning whole motes to spawn.
///
/// Clamping the accumulator at the cap is deliberate: without it the leftover
/// grows while every slot is occupied and then discharges as a burst the moment
/// slots free up, which defeats rate-based spawning entirely.
fn dust_take_spawn_budget(acc: &mut f32, rate: f32, budget: u32, dt: f32) -> u32 {
    *acc += rate * dt;
    let to_spawn = (*acc as u32).min(budget);
    *acc -= to_spawn as f32;
    if budget == 0 {
        *acc = acc.min(rate * dt);
    }
    to_spawn
}

/// Small constant lateral drift for one mote (spec §4).
fn dust_turbulence(strength: f32, rng: &mut impl Rng) -> Vec3 {
    if strength <= 0.0 {
        return Vec3::ZERO;
    }
    Vec3::new(
        rng.random_range(-1.0_f32..1.0),
        rng.random_range(-1.0_f32..1.0),
        rng.random_range(-1.0_f32..1.0),
    ) * strength
}

/// Picks a spawn point inside the view volume at `depth`, offset toward the
/// side the motes stream in from.
///
/// The lateral extent is derived from the camera frustum rather than a fixed
/// radius so motes cover the view at any depth, and the whole distribution is
/// pushed along `travel_dir` so motes have room to cross the screen before
/// expiring. That offset is what makes strafing work: when the ship slides
/// sideways `travel_dir` is lateral, so motes enter from the beam rather than
/// from ahead.
#[allow(clippy::too_many_arguments)]
fn dust_spawn_position(
    cam_pos: Vec3,
    cam_transform: &Transform,
    travel_dir: Vec3,
    depth: f32,
    fov: f32,
    aspect: f32,
    edge_bias: f32,
    rng: &mut impl Rng,
) -> Vec3 {
    // Cover appreciably more than the frustum so motes are already alive by the
    // time they enter view (spec §3).
    const FRUSTUM_MARGIN: f32 = 1.15;
    // How far along the incoming direction to bias spawns, as a fraction of the
    // lateral half-extent.
    const AHEAD_BIAS: f32 = 0.5;

    let half_h = depth * (fov * 0.5).tan() * FRUSTUM_MARGIN;
    let half_w = half_h * aspect;

    let u = dust_edge_shape(rng.random_range(-1.0_f32..1.0), edge_bias);
    let v = dust_edge_shape(rng.random_range(-1.0_f32..1.0), edge_bias);

    let base = cam_pos
        + cam_transform.forward() * depth
        + cam_transform.right() * (u * half_w)
        + cam_transform.up() * (v * half_h);

    base - travel_dir * (half_w * AHEAD_BIAS)
}

/// Drifts motes opposite the ship's true velocity, aligns each billboard to its
/// direction of travel, and recycles motes that fall behind the camera.
fn move_dust_motes(
    time: Res<Time>,
    world_config: Option<Res<crate::world::config::WorldConfig>>,
    state: Res<DustFieldState>,
    mut commands: Commands,
    cam_q: Query<&Transform, (With<GameCamera>, Without<DustMote>)>,
    ship_q: Query<&ShipPhysics, With<LocalShip>>,
    mut mote_q: Query<(Entity, &DustMote, &mut Transform)>,
) {
    let Ok(cam_transform) = cam_q.single() else {
        return;
    };
    let Ok(physics) = ship_q.single() else {
        return;
    };
    let cfg = DustPfxSettings::from_world(world_config.as_deref());

    let dt = time.delta_secs();
    let velocity = ship_velocity(physics);
    let speed = velocity.length();
    // Motes move opposite the ship — this is the entire velocity field.
    let mote_velocity = -velocity * cfg.mote_speed_multiplier;
    let travel_dir = mote_velocity.normalize_or_zero();

    let cam_pos = cam_transform.translation;
    let cam_forward = *cam_transform.forward();

    for (entity, mote, mut transform) in mote_q.iter_mut() {
        let drift = mote_velocity + mote.turbulence * speed;
        transform.translation += drift * dt;

        // Recycle once a mote falls behind the camera; it can no longer
        // contribute and holds a slot the emitter wants back.
        let behind = (transform.translation - cam_pos).dot(cam_forward);
        if behind < -DUST_BEHIND_CAMERA_MARGIN {
            commands.entity(entity).try_despawn();
            continue;
        }

        if travel_dir == Vec3::ZERO {
            continue;
        }
        let to_cam = (cam_pos - transform.translation).normalize_or_zero();
        transform.rotation = dust_billboard_rotation(travel_dir, to_cam);

        let (length_range, s) = match mote.kind {
            DustMoteKind::Layer(i) => match cfg.layers.get(i) {
                Some(layer) => (layer.length, state.streak_s),
                None => continue,
            },
            // The warp field only exists at speed, so it stretches with the
            // ramp rather than with ordinary streak response.
            DustMoteKind::Warp => ([1.0, cfg.warp.length_multiplier], state.warp_ramp),
        };
        let length = mote.width * dust_ramp(length_range, s) * mote.length_scale;
        transform.scale = Vec3::new(length, mote.width, 1.0);
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
mod dust_tests {
    use super::*;

    fn physics(yaw: f32, forward: f32, lateral: f32) -> ShipPhysics {
        ShipPhysics {
            x: 0.0,
            z: 0.0,
            yaw,
            forward_speed: forward,
            roll: 0.0,
            lateral_speed: lateral,
        }
    }

    fn settings() -> DustPfxSettings {
        DustPfxSettings::from_world(None)
    }

    // --- ship_velocity -----------------------------------------------------

    #[test]
    fn ship_velocity_at_zero_yaw_points_down_negative_z() {
        let v = ship_velocity(&physics(0.0, 10.0, 0.0));
        assert!(
            (v - Vec3::new(0.0, 0.0, -10.0)).length() < 1e-4,
            "got {v:?}"
        );
    }

    #[test]
    fn ship_velocity_follows_yaw() {
        // Yawed 90° to starboard, forward should be +X.
        let v = ship_velocity(&physics(std::f32::consts::FRAC_PI_2, 10.0, 0.0));
        assert!((v - Vec3::new(10.0, 0.0, 0.0)).length() < 1e-3, "got {v:?}");
    }

    /// Regression: dust ignored `lateral_speed` entirely, so the field froze
    /// while the ship strafed. Pure lateral motion must produce pure lateral
    /// velocity — at yaw 0, starboard is +X.
    #[test]
    fn ship_velocity_reacts_to_pure_strafe() {
        let v = ship_velocity(&physics(0.0, 0.0, 7.0));
        assert!(
            v.length() > 0.0,
            "strafing with no forward speed must still yield velocity"
        );
        assert!((v - Vec3::new(7.0, 0.0, 0.0)).length() < 1e-4, "got {v:?}");
    }

    #[test]
    fn ship_velocity_combines_forward_and_lateral() {
        let v = ship_velocity(&physics(0.0, 3.0, 4.0));
        // Forward -Z and starboard +X are perpendicular, so the magnitude is
        // the hypotenuse rather than either component.
        assert!((v.length() - 5.0).abs() < 1e-4, "got {}", v.length());
        assert!((v - Vec3::new(4.0, 0.0, -3.0)).length() < 1e-4, "got {v:?}");
    }

    #[test]
    fn ship_velocity_reverse_flips_direction() {
        let v = ship_velocity(&physics(0.0, -6.0, 0.0));
        assert!((v - Vec3::new(0.0, 0.0, 6.0)).length() < 1e-4, "got {v:?}");
    }

    #[test]
    fn ship_velocity_has_no_vertical_component() {
        // ShipPhysics is an XZ model; a Y drift would be motion the ship is
        // not making.
        let v = ship_velocity(&physics(0.9, 8.0, -3.0));
        assert_eq!(v.y, 0.0);
    }

    // --- billboard orientation --------------------------------------------

    #[test]
    fn billboard_aligns_local_x_with_direction_of_travel() {
        let travel = Vec3::new(0.0, 0.0, -1.0);
        let to_cam = Vec3::new(0.0, 1.0, 0.0);
        let rot = dust_billboard_rotation(travel, to_cam);
        let local_x = rot * Vec3::X;
        assert!(
            (local_x - travel).length() < 1e-4,
            "quad's long axis must follow travel, got {local_x:?}"
        );
    }

    #[test]
    fn billboard_faces_camera_as_closely_as_possible() {
        let travel = Vec3::new(0.0, 0.0, -1.0);
        let to_cam = Vec3::new(0.0, 1.0, 0.0);
        let rot = dust_billboard_rotation(travel, to_cam);
        let normal = rot * Vec3::Z;
        // to_cam is already perpendicular to travel, so the quad can face it
        // exactly.
        assert!(
            (normal - to_cam).length() < 1e-4,
            "quad normal should point at the camera, got {normal:?}"
        );
    }

    #[test]
    fn billboard_basis_stays_orthonormal() {
        let rot = dust_billboard_rotation(Vec3::new(1.0, 0.0, -2.0), Vec3::new(0.3, 0.9, 0.1));
        let (x, y, z) = (rot * Vec3::X, rot * Vec3::Y, rot * Vec3::Z);
        assert!(x.dot(y).abs() < 1e-4);
        assert!(x.dot(z).abs() < 1e-4);
        assert!(y.dot(z).abs() < 1e-4);
        assert!((x.length() - 1.0).abs() < 1e-4);
    }

    /// A mote heading straight at the camera has no projected direction. The
    /// Gram-Schmidt projection collapses to zero there, so this must fall back
    /// rather than produce a NaN rotation — and it is the common case when
    /// flying forward, not an edge case.
    #[test]
    fn billboard_degenerate_head_on_case_is_finite() {
        let travel = Vec3::new(0.0, 0.0, -1.0);
        let rot = dust_billboard_rotation(travel, travel);
        assert!(
            rot.is_finite(),
            "head-on mote produced a non-finite rotation"
        );
        let local_x = rot * Vec3::X;
        assert!(
            (local_x - travel).length() < 1e-4,
            "fallback must still align with travel, got {local_x:?}"
        );
    }

    #[test]
    fn billboard_zero_velocity_is_identity_not_nan() {
        let rot = dust_billboard_rotation(Vec3::ZERO, Vec3::Y);
        assert!(rot.is_finite());
    }

    // --- speed curves and smoothing ---------------------------------------

    #[test]
    fn dust_ramp_interpolates_between_rest_and_full_speed() {
        assert_eq!(dust_ramp([2.0, 10.0], 0.0), 2.0);
        assert_eq!(dust_ramp([2.0, 10.0], 1.0), 10.0);
        assert_eq!(dust_ramp([2.0, 10.0], 0.5), 6.0);
    }

    #[test]
    fn speed_curve_keeps_the_effect_restrained_at_low_speed() {
        // Half speed through an S² curve should land well under half strength,
        // which is the whole point of the exponent (spec §2).
        let half = dust_speed_fraction(&physics(0.0, 6.25, 0.0), None, 2.0);
        assert!((half - 0.25).abs() < 1e-3, "got {half}");
    }

    #[test]
    fn speed_curve_uses_true_velocity_not_just_forward_speed() {
        let forward_only = dust_speed_fraction(&physics(0.0, 3.0, 0.0), None, 1.0);
        let with_strafe = dust_speed_fraction(&physics(0.0, 3.0, 4.0), None, 1.0);
        assert!(
            with_strafe > forward_only,
            "strafing must raise the speed fraction ({with_strafe} vs {forward_only})"
        );
    }

    #[test]
    fn speed_fraction_clamps_at_full_speed() {
        let s = dust_speed_fraction(&physics(0.0, 999.0, 0.0), None, 2.0);
        assert!((s - 1.0).abs() < 1e-6, "got {s}");
    }

    #[test]
    fn dust_smooth_converges_toward_target() {
        let mut v = 0.0;
        for _ in 0..200 {
            v = dust_smooth(v, 1.0, 0.1, 0.016);
        }
        assert!((v - 1.0).abs() < 1e-3, "got {v}");
    }

    #[test]
    fn dust_smooth_zero_response_snaps() {
        assert_eq!(dust_smooth(0.0, 1.0, 0.0, 0.016), 1.0);
    }

    /// Spec §10: streak length should lead, brightness follow, density lag.
    /// That ordering is what makes acceleration feel immediate without motes
    /// visibly popping into existence.
    #[test]
    fn response_rates_stagger_streak_then_brightness_then_density() {
        let cfg = settings();
        let dt = 0.1;
        let streak = dust_smooth(0.0, 1.0, cfg.streak_response_secs, dt);
        let brightness = dust_smooth(0.0, 1.0, cfg.brightness_response_secs, dt);
        let spawn = dust_smooth(0.0, 1.0, cfg.spawn_response_secs, dt);
        assert!(
            streak > brightness && brightness > spawn,
            "expected streak > brightness > spawn, got {streak} / {brightness} / {spawn}"
        );
    }

    // --- tint --------------------------------------------------------------

    #[test]
    fn tint_runs_cool_grey_blue_to_near_white() {
        let cfg = settings();
        let at_rest = dust_tint(&cfg, 0.0);
        let at_speed = dust_tint(&cfg, 1.0);
        assert_eq!(at_rest, cfg.low_speed_tint);
        assert_eq!(at_speed, cfg.high_speed_tint);
        // "Whiter when fast" means the channels converge, not just brighten.
        let spread = |c: [f32; 3]| {
            c.iter().cloned().fold(f32::MIN, f32::max) - c.iter().cloned().fold(f32::MAX, f32::min)
        };
        assert!(
            spread(at_speed) < spread(at_rest),
            "high-speed tint should be less saturated than the low-speed tint"
        );
    }

    // --- spawn budget ------------------------------------------------------

    #[test]
    fn spawn_budget_accumulates_fractional_motes() {
        let mut acc = 0.0;
        // 10/sec for 0.05s = 0.5 motes — nothing yet.
        assert_eq!(dust_take_spawn_budget(&mut acc, 10.0, 100, 0.05), 0);
        // Another 0.05s tips it over 1.0.
        assert_eq!(dust_take_spawn_budget(&mut acc, 10.0, 100, 0.05), 1);
    }

    /// Without the at-cap clamp the accumulator grows while every slot is
    /// occupied, then discharges as a burst the moment slots free up.
    #[test]
    fn spawn_budget_does_not_bank_motes_while_at_cap() {
        let mut acc = 0.0;
        for _ in 0..100 {
            assert_eq!(dust_take_spawn_budget(&mut acc, 200.0, 0, 0.016), 0);
        }
        assert!(
            acc <= 200.0 * 0.016 + 1e-6,
            "accumulator banked up to {acc}"
        );
    }

    // --- edge bias ---------------------------------------------------------

    #[test]
    fn edge_bias_pushes_samples_toward_the_screen_edges() {
        // Near layer weights spawns to the edges so big close streaks stay
        // peripheral (spec §13).
        let biased = dust_edge_shape(0.5, 0.7);
        assert!(biased > 0.5, "expected outward push, got {biased}");
        assert!(biased <= 1.0);
    }

    #[test]
    fn edge_bias_zero_is_uniform_and_preserves_sign() {
        assert!((dust_edge_shape(0.5, 0.0) - 0.5).abs() < 1e-5);
        assert!((dust_edge_shape(-0.5, 0.0) + 0.5).abs() < 1e-5);
    }

    #[test]
    fn edge_bias_negative_pulls_samples_toward_the_centre() {
        assert!(dust_edge_shape(0.5, -1.0) < 0.5);
    }

    // --- screen-relative sizing --------------------------------------------

    /// Layer widths are fractions of screen height, not world units. Treating
    /// them as world units makes the far layer (40–150 units out, width 0.006)
    /// sub-pixel and invisible, which is exactly what happened first time.
    #[test]
    fn view_height_grows_with_depth() {
        let fov = std::f32::consts::FRAC_PI_4;
        let near = dust_view_height_at(10.0, fov);
        let far = dust_view_height_at(100.0, fov);
        assert!(
            (far / near - 10.0).abs() < 1e-3,
            "should scale linearly with depth"
        );
    }

    #[test]
    fn screen_relative_width_holds_apparent_size_across_depth_bands() {
        let fov = std::f32::consts::FRAC_PI_4;
        let frac = 0.02;
        // Two motes of the same authored width at very different depths must
        // subtend the same fraction of the view.
        let near_world = frac * dust_view_height_at(10.0, fov);
        let far_world = frac * dust_view_height_at(120.0, fov);
        assert!(
            far_world > near_world,
            "a deeper mote needs more world width"
        );
        let apparent = |w: f32, d: f32| w / dust_view_height_at(d, fov);
        assert!(
            (apparent(near_world, 10.0) - apparent(far_world, 120.0)).abs() < 1e-6,
            "apparent size must not depend on depth"
        );
    }

    #[test]
    fn builtin_widths_are_screen_fractions_not_world_units() {
        let cfg = settings();
        for layer in &cfg.layers {
            assert!(
                layer.width > 0.0 && layer.width < 0.5,
                "width {} does not read as a screen fraction",
                layer.width
            );
        }
    }

    // --- lifetime ----------------------------------------------------------

    /// A mote must live long enough to actually transit the volume and pass the
    /// camera. A fixed lifetime kills fast motes while they are still distant
    /// specks, so the field never reads as streaming past you.
    #[test]
    fn lifetime_covers_transit_to_behind_the_camera() {
        // 100 units out, closing at 50/s → ~2.1s to reach 5 units behind.
        let life = dust_lifetime(100.0, 50.0, 10.0, 1.0);
        assert!((life - 2.1).abs() < 1e-3, "got {life}");
    }

    #[test]
    fn lifetime_shortens_as_speed_rises() {
        let slow = dust_lifetime(100.0, 10.0, 100.0, 1.0);
        let fast = dust_lifetime(100.0, 100.0, 100.0, 1.0);
        assert!(fast < slow, "faster motes should transit sooner");
    }

    #[test]
    fn lifetime_is_capped_so_slow_motes_do_not_hang() {
        // Crawling: transit would be ~1000s. The cap is what stops motes
        // hanging in space looking like snow.
        let life = dust_lifetime(100.0, 0.1, 2.0, 1.0);
        assert_eq!(life, 2.0);
    }

    #[test]
    fn lifetime_at_zero_speed_falls_back_to_the_cap() {
        assert_eq!(dust_lifetime(50.0, 0.0, 3.0, 1.0), 3.0);
    }

    #[test]
    fn lifetime_never_returns_zero() {
        assert!(dust_lifetime(0.0, 1e6, 5.0, 1.0) >= 0.05);
    }

    // --- config resolution -------------------------------------------------

    #[test]
    fn settings_without_world_config_use_builtin_layers() {
        let cfg = settings();
        assert!(cfg.enabled);
        assert_eq!(cfg.layers.len(), 3);
        assert_eq!(cfg.speed_curve_exponent, DUST_SPEED_CURVE_EXPONENT);
        // Warp is opt-in: absent [dust.warp] means no warp field.
        assert!(!cfg.warp.enabled);
    }

    #[test]
    fn builtin_layers_run_near_to_far() {
        let cfg = settings();
        let depths: Vec<f32> = cfg.layers.iter().map(|l| l.depth_band[0]).collect();
        assert!(
            depths.windows(2).all(|w| w[0] < w[1]),
            "layers should be ordered near→far, got {depths:?}"
        );
        // Far motes must stay below the bloom threshold or the scene fogs.
        let far = cfg.layers.last().expect("three layers");
        let near = cfg.layers.first().expect("three layers");
        assert!(far.brightness[1] < near.brightness[1]);
        assert!(!far.additive, "far layer should alpha-blend, not add");
        assert!(near.additive, "near layer should be additive");
    }

    #[test]
    fn world_config_overrides_layers_positionally() {
        use crate::world::config::{DustLayerConfig, DustPfxConfig};
        let world = crate::world::config::WorldConfig {
            dust: Some(DustPfxConfig {
                layers: vec![DustLayerConfig {
                    max_motes: Some(7),
                    ..Default::default()
                }],
                ..Default::default()
            }),
            ..Default::default()
        };
        let cfg = DustPfxSettings::from_world(Some(&world));
        assert_eq!(cfg.layers.len(), 1);
        assert_eq!(cfg.layers[0].max_motes, 7);
        // Unset fields fall back to the matching built-in layer.
        assert_eq!(cfg.layers[0].texture, DUST_DEFAULT_LAYERS[0].texture);
        assert_eq!(cfg.layers[0].width, DUST_DEFAULT_LAYERS[0].width);
    }

    #[test]
    fn world_config_overrides_scalars() {
        use crate::world::config::DustPfxConfig;
        let world = crate::world::config::WorldConfig {
            dust: Some(DustPfxConfig {
                enabled: Some(false),
                turbulence: Some(0.5),
                low_speed_tint: Some([0.1, 0.2, 0.3]),
                ..Default::default()
            }),
            ..Default::default()
        };
        let cfg = DustPfxSettings::from_world(Some(&world));
        assert!(!cfg.enabled);
        assert_eq!(cfg.turbulence, 0.5);
        assert_eq!(cfg.low_speed_tint, [0.1, 0.2, 0.3]);
        // Untouched fields keep their defaults.
        assert_eq!(cfg.high_speed_tint, DUST_HIGH_SPEED_TINT);
    }

    #[test]
    fn empty_dust_block_keeps_builtin_layers() {
        let world = crate::world::config::WorldConfig {
            dust: Some(Default::default()),
            ..Default::default()
        };
        let cfg = DustPfxSettings::from_world(Some(&world));
        assert_eq!(cfg.layers.len(), 3);
    }

    // --- quad geometry -----------------------------------------------------

    /// `space_mote_streak_head.png` carries its bright head at the low-U end,
    /// and the billboard aligns local +X with travel. The quad's UVs are
    /// therefore mirrored so the head leads; if this flips, every near streak
    /// trails head-first and the field reads as moving backwards.
    #[test]
    fn quad_uvs_put_low_u_at_positive_x_so_streak_heads_lead() {
        let mesh = dust_quad_mesh();
        let positions = match mesh.attribute(Mesh::ATTRIBUTE_POSITION) {
            Some(bevy::mesh::VertexAttributeValues::Float32x3(p)) => p.clone(),
            _ => panic!("quad must have Float32x3 positions"),
        };
        let uvs = match mesh.attribute(Mesh::ATTRIBUTE_UV_0) {
            Some(bevy::mesh::VertexAttributeValues::Float32x2(u)) => u.clone(),
            _ => panic!("quad must have Float32x2 UVs"),
        };
        assert_eq!(positions.len(), 4);

        let u_at_max_x = positions
            .iter()
            .zip(&uvs)
            .filter(|(p, _)| p[0] > 0.0)
            .map(|(_, uv)| uv[0])
            .collect::<Vec<_>>();
        let u_at_min_x = positions
            .iter()
            .zip(&uvs)
            .filter(|(p, _)| p[0] < 0.0)
            .map(|(_, uv)| uv[0])
            .collect::<Vec<_>>();

        assert!(
            u_at_max_x.iter().all(|&u| u == 0.0),
            "leading (+X) edge must sample u=0 where the streak head lives, got {u_at_max_x:?}"
        );
        assert!(
            u_at_min_x.iter().all(|&u| u == 1.0),
            "trailing (-X) edge must sample u=1 (the tail), got {u_at_min_x:?}"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `PfxPlugin::build` must not re-register an asset type the bootstrap has
    /// already registered.
    ///
    /// `init_asset` is not idempotent: it swaps in a fresh `Assets<A>` backed by
    /// a new `AssetIndexAllocator` and overwrites the `AssetServer`'s handle
    /// provider, orphaning every handle minted before it ran. Those handles
    /// index past the end of the new storage, so the insert that lands when
    /// their load finishes panics out of bounds — which is what crashed the
    /// deployed server on load. Only the real bootstrap hit it: the automation
    /// bootstrap skips `RendererPlugin`, and so never builds `PfxPlugin`.
    #[test]
    fn pfx_plugin_preserves_already_registered_image_assets() {
        let mut app = App::new();
        app.add_plugins(bevy::app::TaskPoolPlugin::default())
            .add_plugins(bevy::asset::AssetPlugin::default());

        // Mirror `ImagePlugin::build`: register `Image`, then seed the default
        // image that the rest of the engine expects to find.
        app.init_asset::<Image>();
        app.world_mut()
            .resource_mut::<Assets<Image>>()
            .insert(&Handle::<Image>::default(), Image::default())
            .unwrap();

        // A handle minted from the original allocator, standing in for the ones
        // the render and UI plugins mint during `DefaultPlugins`.
        let minted: Handle<Image> = app.world().resource::<Assets<Image>>().reserve_handle();

        app.add_plugins(super::PfxPlugin);

        assert!(
            app.world()
                .resource::<Assets<Image>>()
                .get(&Handle::<Image>::default())
                .is_some(),
            "PfxPlugin discarded the default image seeded by ImagePlugin"
        );

        // Completing that load must still land in this collection. Before the
        // guard, this insert panicked with an out-of-bounds index.
        app.world_mut()
            .resource_mut::<Assets<Image>>()
            .insert(minted.id(), Image::default())
            .expect("a handle minted before PfxPlugin must still resolve after it");
    }

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
            target: crate::system_registry::helm_thrust_system_id(),
            payload: crate::messages::SystemControlPayload::SetThrust { value: 1.0 },
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
        .add_plugins(bevy::app::TaskPoolPlugin::default())
        .add_plugins(bevy::time::TimePlugin)
        .add_plugins(bevy::asset::AssetPlugin::default())
        .init_asset::<Mesh>()
        .init_asset::<StandardMaterial>()
        .init_asset::<Image>()
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
        // Multiple BeamBody entities exist now (crossed glow + core ribbon
        // layers), but they all share the same start/end and therefore the
        // same segment midpoint — any one of them is representative.
        let mut q = app
            .world_mut()
            .query_filtered::<&Transform, With<BeamBody>>();
        q.iter(app.world())
            .next()
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
                // `integrate_ship_physics` (issue #695) is scoped to
                // `AiHighFidelity` — add the marker + helm intent
                // components so `process_helm_inputs` -> physics still
                // moves this ship, matching pre-#695 behavior.
                crate::ai_plugin::AiHighFidelity,
                crate::ship::helm::ThrustInput::default(),
                crate::ship::helm::SteeringInput::default(),
                crate::ship::helm::LateralThrustInput::default(),
                crate::ship::helm::ImpulseCommand::default(),
                crate::ship::helm::BoostCommand::default(),
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
                // `integrate_ship_physics` (issue #695) is scoped to
                // `AiHighFidelity` — add the marker + helm intent
                // components so `process_helm_inputs` -> physics still
                // moves this ship, matching pre-#695 behavior.
                crate::ai_plugin::AiHighFidelity,
                crate::ship::helm::ThrustInput::default(),
                crate::ship::helm::SteeringInput::default(),
                crate::ship::helm::LateralThrustInput::default(),
                crate::ship::helm::ImpulseCommand::default(),
                crate::ship::helm::BoostCommand::default(),
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
