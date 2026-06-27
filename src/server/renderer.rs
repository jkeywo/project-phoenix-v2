use bevy::core_pipeline::core_2d::graph::Core2d;
use bevy::core_pipeline::core_3d::graph::Core3d;
use bevy::core_pipeline::Skybox;
use bevy::pbr::{DistanceFog, FogFalloff};
use bevy::post_process::bloom::Bloom;
use bevy::prelude::*;
use bevy::camera::ClearColorConfig;
use bevy::render::camera::CameraRenderGraph;
use bevy::render::render_resource::{TextureViewDescriptor, TextureViewDimension};
use rand::Rng;
use rand::SeedableRng;
use std::collections::{HashMap, HashSet};

use crate::ai_plugin::WarpOutMarker;
use crate::entity_spawner::{RegionEffectsSection, RegionShapeSection};
use crate::lobby::GameStateCache;
use crate::messages::{GamePhase, ViewDirection, ViewMode};
use crate::region_effects::RegionEffectKind;
use crate::region_plugin::RegionMembership;
use crate::region_shape::RegionShape;
use crate::server::pfx::PfxPlugin;
use crate::ship_state::ShipState;
use crate::simulation::AsteroidDestroyedVfx;
use crate::world::server::OnScreenMessage;

// ── VFX Components ────────────────────────────────────────────────

/// A ripple ring expanding outward at an asteroid destruction site.
/// `elapsed` tracks how far through the animation we are; the effect
/// lasts `RIPPLE_DURATION` seconds.
#[derive(Component)]
struct RippleEffect {
    x: f32,
    z: f32,
    elapsed: f32,
}

const RIPPLE_DURATION: f32 = 1.2;
const RIPPLE_MAX_RADIUS: f32 = 30.0;
const SPACE_SKYBOX_PATH: &str = "skybox/phoenix_space_cubemap.png";
const SPACE_SKYBOX_BRIGHTNESS: f32 = 450.0;

// ── Marker Components ─────────────────────────────────────────────

#[derive(Component)]
struct LobbyCamera;

#[derive(Component)]
pub struct GameCamera;

#[derive(Resource)]
struct SpaceSkyboxAsset {
    image: Handle<Image>,
    is_loaded: bool,
}

/// FPS counter text — rendered in the Bevy UI overlay.
#[derive(Component)]
struct FpsText;

/// In-game crew roster shown on the view screen during InProgress phase.
#[derive(Component)]
struct ViewScreenText;

/// Top-centre label showing the current camera facing direction during InProgress phase.
#[derive(Component)]
struct ViewDirectionLabel;

/// Root node of the comms overlay panel on the viewscreen.
/// Spawned/despawned by `sync_comms_overlay` based on `OnScreenMessage`.
#[derive(Component)]
struct CommsOverlay;

// ── Plugin ────────────────────────────────────────────────────────

pub struct RendererPlugin;

impl Plugin for RendererPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(PfxPlugin)
            .init_resource::<NebulaFogState>()
            .init_resource::<NebulaCloudState>()
            .add_systems(Startup, setup)
            .add_systems(
                PostStartup,
                // Explicit `.after` for documentation: PostStartup naturally
                // runs after Startup (where `insert_world_config_resource`
                // lives), but the annotation makes the ordering contract
                // visible at the registration site.
                spawn_world_ambient_light.after(crate::world::server::insert_world_config_resource),
            )
            .add_systems(
                Update,
                (
                    update_fps_counter,
                    update_camera_aspect,
                    prepare_space_skybox_cubemap,
                    toggle_cameras,
                    update_view_screen_text,
                    update_view_direction_label,
                    hull_camera.run_if(in_state(GamePhase::InProgress)),
                    sync_comms_overlay.run_if(in_state(GamePhase::InProgress)),
                ),
            )
            .add_systems(
                Update,
                (
                    spawn_ripples,
                    tick_ripples.run_if(in_state(GamePhase::InProgress)),
                    draw_warp_exit_markers.run_if(in_state(GamePhase::InProgress)),
                    nebula_fog_system.run_if(in_state(GamePhase::InProgress)),
                    spawn_nebula_cloud_particles,
                ),
            )
            .add_systems(OnExit(GamePhase::InProgress), cleanup_nebula_fog);
    }
}

// ── Setup ─────────────────────────────────────────────────────────

fn setup(mut commands: Commands, asset_server: Res<AssetServer>) {
    let skybox_image = asset_server.load(SPACE_SKYBOX_PATH);
    commands.insert_resource(SpaceSkyboxAsset {
        image: skybox_image.clone(),
        is_loaded: false,
    });

    // 2D camera — active during lobby phase. `IsDefaultUiCamera` marks
    // this as the canonical UI target for all UI nodes. It stays active
    // throughout InProgress so the FPS counter, radar widgets, and viewscreen
    // border continue to render without an explicit UiTargetCamera.
    commands.spawn((
        LobbyCamera,
        Camera2d,
        Camera {
            order: 0,
            clear_color: ClearColorConfig::None,
            ..default()
        },
        CameraRenderGraph::new(Core2d),
        IsDefaultUiCamera,
    ));

    // 3D camera — active during in-game phase, positioned for ship view.
    // order: -1 so the 3D scene composites before the UI layer (LobbyCamera
    // order 0), keeping the viewscreen border in front of all 3D objects.
    commands.spawn((
        GameCamera,
        Camera3d::default(),
        Camera {
            is_active: false,
            order: -1,
            ..default()
        },
        CameraRenderGraph::new(Core3d),
        Projection::Perspective(PerspectiveProjection {
            far: 5000.0,
            ..default()
        }),
        Bloom::NATURAL,
        Skybox {
            image: skybox_image,
            brightness: SPACE_SKYBOX_BRIGHTNESS,
            ..default()
        },
        Transform::from_xyz(0.0, 2.0, -10.0),
    ));

    // Ambient light is now spawned by `spawn_world_ambient_light` in
    // `PostStartup`, which reads `WorldConfig.ambient_light` if present and
    // falls back to the default warm fill otherwise.

    // View-screen crew roster — visible only during InProgress phase.
    commands.spawn((
        ViewScreenText,
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(12.0),
            left: Val::Px(12.0),
            ..default()
        },
        Text::new(""),
        TextFont {
            font_size: 14.0,
            ..default()
        },
        TextColor(Color::srgba(0.7, 0.85, 1.0, 0.75)),
        Visibility::Hidden,
    ));

    // Direction label — top-centre, visible only during InProgress.
    commands
        .spawn((
            ViewDirectionLabel,
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(8.0),
                width: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                ..default()
            },
            Visibility::Hidden,
        ))
        .with_children(|parent| {
            parent.spawn((
                Text::new("FORE"),
                TextFont {
                    font_size: 20.0,
                    ..default()
                },
                TextColor(Color::srgba(0.9, 0.95, 1.0, 0.9)),
            ));
        });

    // Red Alert overlay is now handled in server.html via a CSS vignette,
    // toggled by SimState messages routed through JS.

    // ── FPS counter (top-right, Bevy UI) ─────────────────────────────
    commands.spawn((
        FpsText,
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(48.0),
            right: Val::Px(12.0),
            ..default()
        },
        Text::new("-- fps"),
        TextFont {
            font_size: 13.0,
            ..default()
        },
        TextColor(Color::srgb(0.8, 0.8, 0.95)),
    ));
}

// ── Systems ───────────────────────────────────────────────────────

fn prepare_space_skybox_cubemap(
    asset_server: Res<AssetServer>,
    mut images: ResMut<Assets<Image>>,
    skybox_asset: Option<ResMut<SpaceSkyboxAsset>>,
    mut skyboxes: Query<&mut Skybox, With<GameCamera>>,
) {
    let Some(mut skybox_asset) = skybox_asset else {
        return;
    };
    if skybox_asset.is_loaded || !asset_server.load_state(&skybox_asset.image).is_loaded() {
        return;
    }

    let Some(image) = images.get_mut(&skybox_asset.image) else {
        return;
    };
    if image.texture_descriptor.array_layer_count() == 1 {
        let layers = image.height() / image.width();
        if layers != 6 {
            bevy::log::error!(
                "space skybox expected a vertical 6-face cubemap, got {}x{}",
                image.width(),
                image.height()
            );
            skybox_asset.is_loaded = true;
            return;
        }
        if let Err(err) = image.reinterpret_stacked_2d_as_array(layers) {
            bevy::log::error!("space skybox cubemap conversion failed: {err}");
            skybox_asset.is_loaded = true;
            return;
        }
        image.texture_view_descriptor = Some(TextureViewDescriptor {
            dimension: Some(TextureViewDimension::Cube),
            ..default()
        });
    }

    for mut skybox in &mut skyboxes {
        skybox.image = skybox_asset.image.clone();
    }
    skybox_asset.is_loaded = true;
}

/// PostStartup: spawn the scene's ambient light. Reads the optional
/// `[ambient_light]` block from `WorldConfig` if present; otherwise falls
/// back to a warm fill (`Color::srgb(0.6, 0.55, 0.5)`, `brightness = 300.0`).
/// Stars contribute per-system point lights on top of this fill.
fn spawn_world_ambient_light(
    mut commands: Commands,
    world_config: Option<Res<crate::world::config::WorldConfig>>,
) {
    let (color, brightness) = world_config
        .as_ref()
        .and_then(|wc| wc.ambient_light.as_ref())
        .map(|al| {
            let c = al.color.unwrap_or([0.6, 0.55, 0.5]);
            (
                Color::srgb(c[0], c[1], c[2]),
                al.brightness.unwrap_or(300.0),
            )
        })
        .unwrap_or_else(|| (Color::srgb(0.6, 0.55, 0.5), 300.0));

    commands.spawn(AmbientLight {
        color,
        brightness,
        ..default()
    });
}

/// Compute and display FPS using Bevy's Time + Local — works on native and WASM.
fn update_fps_counter(
    time: Res<Time>,
    mut fps_query: Query<&mut Text, With<FpsText>>,
    mut tracker: Local<(u32, f32)>, // (frame_count, accumulated_time)
) {
    tracker.0 += 1;
    tracker.1 += time.delta().as_secs_f32();

    if tracker.1 >= 0.5 {
        let fps = (tracker.0 as f32 / tracker.1).round() as u32;
        if let Ok(mut text) = fps_query.single_mut() {
            **text = format!("{} fps", fps);
        }
        tracker.0 = 0;
        tracker.1 = 0.0;
    }
}

/// Updates the GameCamera's perspective projection aspect ratio to match
/// the current window dimensions. Without this, the camera retains its
/// initial aspect ratio — the 3D view appears stretched on resize.
fn update_camera_aspect(
    window: Query<&Window>,
    mut game_cam: Query<&mut Projection, With<GameCamera>>,
) {
    let Ok(window) = window.single() else { return };
    let w = window.width();
    let h = window.height();
    if w <= 0.0 || h <= 0.0 {
        return;
    }
    let aspect = w / h;
    for mut proj in game_cam.iter_mut() {
        if let Projection::Perspective(ref mut p) = *proj {
            if (p.aspect_ratio - aspect).abs() > 0.001 {
                p.aspect_ratio = aspect;
            }
        }
    }
}

fn toggle_cameras(
    state: Res<State<GamePhase>>,
    ship: Res<ShipState>,
    mut game: Query<&mut Camera, With<GameCamera>>,
) {
    if !state.is_changed() && !ship.is_changed() {
        return;
    }
    let in_game = state.get() == &GamePhase::InProgress;
    // The 3D GameCamera is active only when we're in a Camera or Comms view.
    // Radar views are handled by the GenericRadarWidget UI pipeline
    // (ServerViewscreenRadarPlugin), which renders through the always-active
    // LobbyCamera (Camera2d, IsDefaultUiCamera).
    let game_active = in_game && matches!(ship.view_mode, ViewMode::Camera(_) | ViewMode::Comms);

    // LobbyCamera (Camera2d, IsDefaultUiCamera) is intentionally kept active
    // in all phases so that UI nodes (FPS counter, radar widgets) continue to
    // render during InProgress without an explicit UiTargetCamera.
    if let Ok(mut cam) = game.single_mut() {
        cam.is_active = game_active;
    }
}

fn update_view_screen_text(
    cache: Res<GameStateCache>,
    mut query: Query<(&mut Text, &mut Visibility), With<ViewScreenText>>,
) {
    if !cache.is_changed() {
        return;
    }
    let Ok((_text, mut vis)) = query.single_mut() else {
        return;
    };
    // Console roster is now hidden on the view screen — crew can see the
    // in-game HUD. Keep the entity but always hide it.
    *vis = Visibility::Hidden;
}

/// First-person hull camera: positioned at the ship's hull edge in the view direction,
/// looking straight out. Ship is always behind the camera.
/// Hull offset = 6.0 units (matches the ship's collision capsule radius).
fn hull_camera(ship: Res<ShipState>, mut cam_query: Query<&mut Transform, With<GameCamera>>) {
    let Ok(mut transform) = cam_query.single_mut() else {
        return;
    };

    // Direction vectors relative to ship heading (yaw=0 → ship faces -Z).
    // fwd = (sin(yaw), 0, -cos(yaw)), port = left of heading, starboard = right.
    // For ViewMode::Radar we still keep the camera at Fore so the 3D scene
    // remains coherent; the radar overlay is drawn separately (#45).
    let direction = match &ship.view_mode {
        ViewMode::Camera(d) => d.clone(),
        ViewMode::Radar
        | ViewMode::ScienceRadar
        | ViewMode::SensorsRadar
        | ViewMode::SystemChart
        | ViewMode::NavigationChart
        | ViewMode::Comms => ViewDirection::Fore,
    };
    let offset_dir = match direction {
        ViewDirection::Fore => Vec3::new(ship.yaw.sin(), 0.0, -ship.yaw.cos()),
        ViewDirection::Aft => Vec3::new(-ship.yaw.sin(), 0.0, ship.yaw.cos()),
        ViewDirection::Port => Vec3::new(-ship.yaw.cos(), 0.0, -ship.yaw.sin()),
        ViewDirection::Starboard => Vec3::new(ship.yaw.cos(), 0.0, ship.yaw.sin()),
    };

    const HULL_RADIUS: f32 = 6.0;
    const LOOK_DIST: f32 = 100.0;

    transform.translation = Vec3::new(
        ship.x + offset_dir.x * HULL_RADIUS,
        0.0,
        ship.z + offset_dir.z * HULL_RADIUS,
    );

    let look_target = Vec3::new(
        ship.x + offset_dir.x * LOOK_DIST,
        0.0,
        ship.z + offset_dir.z * LOOK_DIST,
    );
    transform.look_at(look_target, Vec3::Y);
}

fn update_view_direction_label(
    ship: Res<ShipState>,
    state: Res<State<GamePhase>>,
    mut label_query: Query<(&Children, &mut Visibility), With<ViewDirectionLabel>>,
    mut text_query: Query<&mut Text>,
) {
    if !ship.is_changed() && !state.is_changed() {
        return;
    }
    let Ok((children, mut vis)) = label_query.single_mut() else {
        return;
    };
    if state.get() != &GamePhase::InProgress {
        *vis = Visibility::Hidden;
        return;
    }
    *vis = Visibility::Visible;
    let label = match &ship.view_mode {
        ViewMode::Camera(ViewDirection::Fore) => "FORE",
        ViewMode::Camera(ViewDirection::Aft) => "AFT",
        ViewMode::Camera(ViewDirection::Port) => "PORT",
        ViewMode::Camera(ViewDirection::Starboard) => "STARBOARD",
        ViewMode::Radar => "RADAR",
        ViewMode::ScienceRadar => "SCIENCE RADAR",
        ViewMode::SensorsRadar => "SENSORS",
        ViewMode::SystemChart => "SYSTEM CHART",
        ViewMode::NavigationChart => "NAV CHART",
        ViewMode::Comms => "COMMS",
    };
    for child in children.iter() {
        if let Ok(mut text) = text_query.get_mut(child) {
            **text = label.to_string();
        }
    }
}

// ── VFX Systems ───────────────────────────────────────────────────

/// Synchronises the comms overlay panel with `OnScreenMessage`.
///
/// When a message is present the overlay is spawned (or rebuilt if stale).
/// When the message is cleared the overlay is despawned.
/// Responses are listed as A), B), C) … read-only labels.
fn sync_comms_overlay(
    on_screen: Res<OnScreenMessage>,
    overlay_q: Query<Entity, With<CommsOverlay>>,
    mut commands: Commands,
) {
    if !on_screen.is_changed() {
        return;
    }

    // Always despawn any existing overlay first.
    for entity in overlay_q.iter() {
        commands.entity(entity).despawn();
    }

    let Some(ref msg) = on_screen.0 else { return };

    // Semi-transparent dark panel, centred on screen, ~60% width.
    let overlay = commands
        .spawn((
            CommsOverlay,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Percent(10.0),
                right: Val::Percent(10.0),
                top: Val::Percent(15.0),
                bottom: Val::Percent(15.0),
                flex_direction: FlexDirection::Column,
                padding: UiRect::all(Val::Px(16.0)),
                row_gap: Val::Px(8.0),
                ..default()
            },
            BackgroundColor(Color::srgba(0.05, 0.05, 0.12, 0.88)),
        ))
        .id();

    // Sender name header
    commands.entity(overlay).with_children(|p| {
        p.spawn((
            Text::new(msg.sender_name.clone()),
            TextFont {
                font_size: 22.0,
                ..default()
            },
            TextColor(Color::srgb(0.7, 0.6, 1.0)),
        ));
    });

    // Divider label
    commands.entity(overlay).with_children(|p| {
        p.spawn((
            Text::new(
                "\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}",
            ),
            TextFont {
                font_size: 14.0,
                ..default()
            },
            TextColor(Color::srgb(0.3, 0.3, 0.5)),
        ));
    });

    // Message body
    commands.entity(overlay).with_children(|p| {
        p.spawn((
            Text::new(msg.body.clone()),
            TextFont {
                font_size: 16.0,
                ..default()
            },
            TextColor(Color::srgb(0.9, 0.9, 1.0)),
            Node {
                flex_grow: 1.0,
                ..default()
            },
        ));
    });

    // Responses (read-only, labelled A, B, C…)
    if !msg.responses.is_empty() {
        commands.entity(overlay).with_children(|p| {
            p.spawn((
                Text::new("POSSIBLE RESPONSES:"),
                TextFont {
                    font_size: 12.0,
                    ..default()
                },
                TextColor(Color::srgb(0.5, 0.5, 0.6)),
            ));
        });
        for (idx, response) in msg.responses.iter().enumerate() {
            let letter = (b'A' + idx as u8) as char;
            let label = format!("{})  {}", letter, response);
            commands.entity(overlay).with_children(|p| {
                p.spawn((
                    Text::new(label),
                    TextFont {
                        font_size: 15.0,
                        ..default()
                    },
                    TextColor(Color::srgb(0.7, 0.95, 0.85)),
                ));
            });
        }
    }
}

/// Spawns a `RippleEffect` entity for each `AsteroidDestroyedVfx` event received.
fn spawn_ripples(mut events: MessageReader<AsteroidDestroyedVfx>, mut commands: Commands) {
    for ev in events.read() {
        commands.spawn(RippleEffect {
            x: ev.x,
            z: ev.z,
            elapsed: 0.0,
        });
    }
}

/// Ticks all active `RippleEffect` entities: advances time, draws the expanding
/// ring via 3D gizmos, and despawns the entity once the animation completes.
fn tick_ripples(
    time: Res<Time>,
    mut query: Query<(Entity, &mut RippleEffect)>,
    mut gizmos: Gizmos,
    mut commands: Commands,
) {
    let dt = time.delta_secs();
    for (entity, mut ripple) in query.iter_mut() {
        ripple.elapsed += dt;
        if ripple.elapsed >= RIPPLE_DURATION {
            commands.entity(entity).despawn();
            continue;
        }

        let t = ripple.elapsed / RIPPLE_DURATION; // 0..1
        let radius = RIPPLE_MAX_RADIUS * t;
        let alpha = (1.0 - t) * 0.85; // fade out

        // Draw a horizontal circle (XZ plane) as the ripple ring.
        // `gizmos.circle` draws in a plane defined by a normal vector.
        let center = Vec3::new(ripple.x, 0.0, ripple.z);
        let color = Color::srgba(1.0, 0.55, 0.1, alpha);
        gizmos.circle(
            Isometry3d::new(center, Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)),
            radius,
            color,
        );

        // Second outer ring, slightly delayed, for a double-ripple feel.
        if t > 0.15 {
            let t2 = (t - 0.15) / (1.0 - 0.15);
            let r2 = RIPPLE_MAX_RADIUS * t2;
            let a2 = (1.0 - t2) * 0.45;
            let c2 = Color::srgba(1.0, 0.75, 0.3, a2);
            gizmos.circle(
                Isometry3d::new(center, Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)),
                r2,
                c2,
            );
        }
    }
}

// ── Nebula Fog & Cloud ──────────────────────────────────────────

/// Marker on nebula cloud particle meshes.
#[derive(Component)]
struct NebulaCloudParticle;

/// Tracks which region entities have had their cloud particles spawned.
#[derive(Resource, Default)]
struct NebulaCloudState {
    entities: HashMap<Entity, Vec<Entity>>,
}

/// Fog transition state — lerps intensity for a smooth fade-in/out.
/// Stores the last active fog parameters so the fade-out can complete
/// after the ship leaves the nebula region.
#[derive(Resource)]
struct NebulaFogState {
    intensity: f32,
    color: [f32; 3],
    density: f32,
}

impl Default for NebulaFogState {
    fn default() -> Self {
        Self {
            intensity: 0.0,
            color: [0.0; 3],
            density: 0.0,
        }
    }
}

/// How fast the fog intensity approaches its target (per second).
/// At this rate the transition completes in ~1.43 s.
const NEBULA_FOG_LERP_RATE: f32 = 0.7;

/// Number of small mesh spheres per nebula used to render the exterior cloud.
const NEBULA_CLOUD_PARTICLE_COUNT: usize = 60;

/// Each frame, checks whether the ship is inside a region with the `NebulaFog`
/// effect and smoothly transitions `DistanceFog` on the `GameCamera`.
fn nebula_fog_system(
    membership: Res<RegionMembership>,
    region_q: Query<&RegionEffectsSection>,
    ship_q: Query<Entity, With<crate::server_app::Ship>>,
    cam_q: Query<Entity, With<GameCamera>>,
    mut state: ResMut<NebulaFogState>,
    mut commands: Commands,
    time: Res<Time>,
) {
    let Ok(ship_entity) = ship_q.single() else {
        return;
    };
    let Ok(cam_entity) = cam_q.single() else {
        return;
    };
    let dt = time.delta_secs();

    let active_fog: Option<([f32; 3], f32)> =
        if let Some(inside) = membership.inside.get(&ship_entity) {
            inside.iter().find_map(|&region| {
                region_q.get(region).ok().and_then(|effects| {
                    effects.0.iter().find_map(|e| {
                        if let RegionEffectKind::NebulaFog { color, density } = e {
                            Some((*color, *density))
                        } else {
                            None
                        }
                    })
                })
            })
        } else {
            None
        };

    // Update stored fog params when inside a nebula, so the fade-out
    // still has valid values after leaving the region.
    if let Some((color, density)) = active_fog {
        state.color = color;
        state.density = density;
    }

    let target = if active_fog.is_some() { 1.0 } else { 0.0 };

    if state.intensity < target {
        state.intensity = (state.intensity + NEBULA_FOG_LERP_RATE * dt).min(target);
    } else if state.intensity > target {
        state.intensity = (state.intensity - NEBULA_FOG_LERP_RATE * dt).max(target);
    }

    if state.intensity <= 0.0 {
        commands.entity(cam_entity).remove::<DistanceFog>();
    } else {
        commands.entity(cam_entity).insert(DistanceFog {
            color: Color::srgb(state.color[0], state.color[1], state.color[2]),
            falloff: FogFalloff::Exponential {
                density: state.density * state.intensity,
            },
            ..default()
        });
    }
}

/// Remove fog when the game leaves `InProgress` so residual settings do not
/// carry over (the camera entity persists between phases).
fn cleanup_nebula_fog(mut commands: Commands, cam_q: Query<Entity, With<GameCamera>>) {
    if let Ok(cam) = cam_q.single() {
        commands.entity(cam).remove::<DistanceFog>();
    }
}

/// Scans for region entities that have a `NebulaFog` effect and spawns a set
/// of small semi-transparent spheres (the exterior cloud) at deterministic
/// positions within the region volume.  Also cleans up particles for regions
/// that have been despawned (e.g. during world reload).
fn spawn_nebula_cloud_particles(
    mut state: ResMut<NebulaCloudState>,
    region_q: Query<(
        Entity,
        &RegionEffectsSection,
        &Transform,
        &RegionShapeSection,
    )>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let active: HashSet<Entity> = region_q.iter().map(|(e, ..)| e).collect();

    // Despawn particles for regions that no longer exist.
    let dead: Vec<Entity> = state
        .entities
        .keys()
        .copied()
        .filter(|e| !active.contains(e))
        .collect();
    for region in dead {
        if let Some(particles) = state.entities.remove(&region) {
            for p in particles {
                commands.entity(p).despawn();
            }
        }
    }

    // Spawn particles for new nebula regions.
    for (entity, effects, transform, shape) in region_q.iter() {
        if state.entities.contains_key(&entity) {
            continue;
        }

        let nebula_color = match effects
            .0
            .iter()
            .find(|e| matches!(e, RegionEffectKind::NebulaFog { .. }))
        {
            Some(RegionEffectKind::NebulaFog { color, .. }) => *color,
            _ => continue,
        };

        let radius = match &shape.0 {
            RegionShape::Sphere { radius } => *radius,
            RegionShape::Box { half_extents, .. } => half_extents[0].max(half_extents[2]),
            RegionShape::Torus { outer_radius, .. } => *outer_radius,
        };

        let center = transform.translation;

        let particle_mesh = meshes.add(Sphere { radius: 1.0 });
        let particle_mat = materials.add(StandardMaterial {
            base_color: Color::srgba(nebula_color[0], nebula_color[1], nebula_color[2], 0.15),
            emissive: LinearRgba::new(
                nebula_color[0] * 1.2,
                nebula_color[1] * 1.2,
                nebula_color[2] * 1.2,
                1.0,
            ),
            alpha_mode: AlphaMode::Blend,
            ..default()
        });

        // Deterministic seed from region centre so the cloud looks the same
        // every time the world is loaded.
        let seed = (center.x.to_bits() as u64)
            .wrapping_mul(0x9e3779b97f4a7c15)
            .wrapping_add(center.z.to_bits() as u64);
        let mut rng = rand::rngs::SmallRng::seed_from_u64(seed);

        let mut particles = Vec::with_capacity(NEBULA_CLOUD_PARTICLE_COUNT);
        for _ in 0..NEBULA_CLOUD_PARTICLE_COUNT {
            let theta = rng.random::<f32>() * std::f32::consts::TAU;
            let phi = (rng.random::<f32>() * 2.0 - 1.0).acos();
            let r = rng.random::<f32>().powf(0.333) * radius * 0.85;
            let y_scale = 0.15;
            let scale = rng.random::<f32>() * 17.0 + 8.0;

            let pos = Vec3::new(
                center.x + r * phi.sin() * theta.cos(),
                center.y + (rng.random::<f32>() - 0.5) * radius * y_scale,
                center.z + r * phi.sin() * theta.sin(),
            );

            let p = commands
                .spawn((
                    NebulaCloudParticle,
                    Mesh3d(particle_mesh.clone()),
                    MeshMaterial3d(particle_mat.clone()),
                    Transform::from_translation(pos).with_scale(Vec3::splat(scale)),
                ))
                .id();
            particles.push(p);
        }
        state.entities.insert(entity, particles);
    }
}

// ── Warp-exit marker renderer ─────────────────────────────────────────────────

/// Draws a vertical (XZ-plane) ring at the world position of each NPC entity
/// that is currently in the `WarpingOut` AI state.
///
/// The ring glows cyan and pulses in opacity with the remaining time so crews
/// can anticipate where the entity will disappear.
fn draw_warp_exit_markers(query: Query<(&WarpOutMarker, &Transform)>, mut gizmos: Gizmos) {
    for (marker, transform) in query.iter() {
        let center = transform.translation;
        // Pulse: brightest at full time remaining, dims as time runs out.
        let pulse = (marker.remaining_secs * 2.0).sin().abs().max(0.2);
        let color = Color::srgba(0.2, 0.9, 1.0, pulse * 0.8);

        // Vertical ring: identity rotation → XY plane (normal = Z).
        // Radius scales with target speed so faster entities show a larger ring.
        let radius = (marker.target_speed * 0.4).clamp(5.0, 40.0);
        gizmos.circle(Isometry3d::new(center, Quat::IDENTITY), radius, color);
        // Second inner ring for visual depth.
        gizmos.circle(
            Isometry3d::new(center, Quat::IDENTITY),
            radius * 0.6,
            Color::srgba(0.5, 1.0, 1.0, pulse * 0.4),
        );
    }
}
