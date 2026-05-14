use bevy::prelude::*;
use std::collections::{HashMap, HashSet};

use crate::lobby::{CurrentPhase, GameStateCache, WorldResource};
use crate::messages::{GamePhase, PhaserBank, ViewDirection, ViewMode};
use crate::radar;
use crate::ship_state::ShipState;
use crate::simulation::{ActiveBeam, AsteroidDestroyedVfx, PhaserRenderConfig, TorpedoSystemResource};
use crate::beam_render;

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

// ── Torpedo rendering ─────────────────────────────────────────────

/// Marks a 3D sphere entity that represents an in-flight torpedo on the viewscreen.
#[derive(Component)]
pub struct TorpedoSphere;

/// Maps torpedo UUID → the `Entity` of its sphere mesh, so we can update
/// positions each frame and despawn when the torpedo is removed.
#[derive(Resource, Default)]
pub struct TorpedoEntityMap(pub HashMap<String, Entity>);

/// Given the set of UUIDs currently in-flight and the set already tracked
/// in the entity map, returns which UUIDs need to be spawned and which
/// entities need to be despawned.
///
/// This pure function is the testable core of the sync logic.
pub fn diff_torpedo_sets(
    in_flight_uuids: &HashSet<String>,
    tracked: &HashSet<String>,
) -> (Vec<String>, Vec<String>) {
    let to_spawn: Vec<String> = in_flight_uuids.difference(tracked).cloned().collect();
    let to_despawn: Vec<String> = tracked.difference(in_flight_uuids).cloned().collect();
    (to_spawn, to_despawn)
}

// ── Marker Components ─────────────────────────────────────────────

#[derive(Component)]
struct LobbyCamera;

#[derive(Component)]
struct GameCamera;

/// 2D camera used to render the radar overlay during InProgress + ViewMode::Radar.
#[derive(Component)]
struct RadarCamera;

/// Marks entities that belong to the lobby scene (panel root).
#[derive(Component)]
struct LobbyItem;

/// Marks the text node whose content is the live player list.
#[derive(Component)]
struct PlayerListText;

/// FPS counter text — rendered in the Bevy UI overlay.
#[derive(Component)]
struct FpsText;

/// In-game crew roster shown on the view screen during InProgress phase.
#[derive(Component)]
struct ViewScreenText;

/// Top-centre label showing the current camera facing direction during InProgress phase.
#[derive(Component)]
struct ViewDirectionLabel;

// ── Plugin ────────────────────────────────────────────────────────

pub struct RendererPlugin;

impl Plugin for RendererPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TorpedoEntityMap>()
            .add_systems(Startup, setup)
            .add_systems(Update, (
                update_fps_counter,
                update_camera_aspect,
                toggle_cameras,
                toggle_lobby_items,
                update_player_list,
                update_view_screen_text,
                update_view_direction_label,
                hull_camera,
                draw_radar_overlay,
            ))
            .add_systems(Update, (
                draw_beam_vfx,
                spawn_ripples,
                tick_ripples,
                sync_torpedo_entities,
            ));
    }
}

// ── Setup ─────────────────────────────────────────────────────────

fn setup(
    mut commands: Commands,
) {
    // 2D camera — active during lobby phase. `IsDefaultUiCamera` marks
    // this as the canonical UI target; Bevy 0.18 requires exactly one such
    // camera for text glyph extraction to resolve when multiple Camera2d
    // entities exist (we also have `RadarCamera`).
    commands.spawn((LobbyCamera, Camera2d, Camera { order: 0, ..default() }, IsDefaultUiCamera));

    // 3D camera — active during in-game phase, positioned for ship view.
    // Far plane extended so the starfield skybox at radius ~2000 is visible.
    commands.spawn((
        GameCamera,
        Camera3d::default(),
        Camera { is_active: false, order: 0, ..default() },
        Projection::Perspective(PerspectiveProjection {
            far: 5000.0,
            ..default()
        }),
        Transform::from_xyz(0.0, 2.0, -10.0),
    ));

    // 2D camera — active during InProgress + ViewMode::Radar; renders gizmos overlay.
    commands.spawn((
        RadarCamera,
        Camera2d,
        Camera { is_active: false, order: 1, ..default() },
    ));

    // Directional light for the 3D scene
    commands.spawn((
        DirectionalLight { illuminance: 5_000.0, ..default() },
        Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, -0.9, 0.5, 0.0)),
    ));

    // Low ambient so cosmetic asteroids out of the directional light still register.
    commands.spawn(AmbientLight {
        color: Color::srgb(0.5, 0.55, 0.7),
        brightness: 80.0,
        ..default()
    });

    // Lobby: panel anchored top-left via node UI.
    // Padding accounts for the viewscreen border corners and edges so that
    // text doesn't overlap the border frame (which uses CORNER_W=240px and
    // CORNER_H=140px for each corner; EDGE_THICKNESS=44px).
    const CORNER_W: f32 = 240.0;
    const CORNER_H: f32 = 140.0;
    commands
        .spawn((
            LobbyItem,
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::FlexStart,
                align_items: AlignItems::FlexStart,
                padding: UiRect {
                    left: Val::Px(CORNER_W),
                    top: Val::Px(CORNER_H),
                    right: Val::Px(CORNER_W),
                    bottom: Val::Px(CORNER_H + 8.0),
                },
                ..default()
            },
        ))
        .with_children(|parent| {
            parent.spawn((
                Text::new("Bridge Crew"),
                TextFont { font_size: 22.0, ..default() },
                TextColor(Color::srgb(0.53, 0.67, 1.0)),
            ));
            parent.spawn((
                PlayerListText,
                Text::new("Players:\n—"),
                TextFont { font_size: 15.0, ..default() },
                TextColor(Color::srgb(0.6, 0.7, 0.73)),
            ));
        });

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
        TextFont { font_size: 14.0, ..default() },
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
                TextFont { font_size: 20.0, ..default() },
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
        TextFont { font_size: 13.0, ..default() },
        TextColor(Color::srgb(0.8, 0.8, 0.95)),
    ));
}

// ── Systems ───────────────────────────────────────────────────────

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
    phase: Res<CurrentPhase>,
    ship: Res<ShipState>,
    mut game: Query<&mut Camera, (With<GameCamera>, Without<RadarCamera>)>,
    mut radar_cam: Query<&mut Camera, (With<RadarCamera>, Without<GameCamera>)>,
) {
    if !phase.is_changed() && !ship.is_changed() {
        return;
    }
    let in_game = phase.0 == GamePhase::InProgress;
    let radar_active = in_game && matches!(ship.view_mode, ViewMode::Radar | ViewMode::ScienceRadar | ViewMode::SystemChart);
    let game_active  = in_game && !radar_active;

    // LobbyCamera (Camera2d, IsDefaultUiCamera) is intentionally kept active
    // in all phases so that UI nodes without an explicit UiTargetCamera
    // (e.g. the FPS counter) continue to render during InProgress. Lobby UI
    // nodes are hidden via Visibility by toggle_lobby_items instead.
    if let Ok(mut cam) = game.single_mut()     { cam.is_active = game_active; }
    if let Ok(mut cam) = radar_cam.single_mut(){ cam.is_active = radar_active; }
}

fn toggle_lobby_items(
    phase: Res<CurrentPhase>,
    mut query: Query<&mut Visibility, With<LobbyItem>>,
) {
    if !phase.is_changed() {
        return;
    }
    let hidden = phase.0 == GamePhase::InProgress;
    for mut vis in query.iter_mut() {
        *vis = if hidden { Visibility::Hidden } else { Visibility::Visible };
    }
}

fn update_player_list(
    cache: Res<GameStateCache>,
    ship_stations: Option<Res<crate::stations::ShipStations>>,
    mut query: Query<&mut Text, With<PlayerListText>>,
) {
    if !cache.is_changed() {
        return;
    }
    let Ok(mut text) = query.single_mut() else { return };
    let mut content = "Players:\n".to_string();

    // Determine player count (non-spectators) for station lookup.
    let player_count = cache.0.players.iter().filter(|p| !p.consoles.is_empty()).count() as u32;

    for p in &cache.0.players {
        if p.consoles.is_empty() {
            content.push_str(&format!("• {} — (spectating)\n", p.name));
        } else {
            // Find the station name whose consoles intersect this player's consoles.
            let station_name = ship_stations.as_ref().and_then(|ss| {
                ss.configs.get(&player_count).and_then(|defs| {
                    defs.iter().find(|d| d.consoles.iter().any(|c| p.consoles.contains(c))).map(|d| d.name.as_str())
                })
            }).unwrap_or("(unknown)");
            content.push_str(&format!("• {} — {}\n", p.name, station_name));
        }
    }
    **text = content;
}

fn update_view_screen_text(
    cache: Res<GameStateCache>,
    mut query: Query<(&mut Text, &mut Visibility), With<ViewScreenText>>,
) {
    if !cache.is_changed() {
        return;
    }
    let Ok((_text, mut vis)) = query.single_mut() else { return };
    // Console roster is now hidden on the view screen — crew can see the
    // in-game HUD. Keep the entity but always hide it.
    *vis = Visibility::Hidden;
}

/// First-person hull camera: positioned at the ship's hull edge in the view direction,
/// looking straight out. Ship is always behind the camera.
/// Hull offset = 6.0 units (matches the ship's collision capsule radius).
fn hull_camera(
    ship: Res<ShipState>,
    mut cam_query: Query<&mut Transform, With<GameCamera>>,
    phase: Res<CurrentPhase>,
) {
    if phase.0 != GamePhase::InProgress {
        return;
    }
    let Ok(mut transform) = cam_query.single_mut() else { return };

    // Direction vectors relative to ship heading (yaw=0 → ship faces -Z).
    // fwd = (sin(yaw), 0, -cos(yaw)), port = left of heading, starboard = right.
    // For ViewMode::Radar we still keep the camera at Fore so the 3D scene
    // remains coherent; the radar overlay is drawn separately (#45).
    let direction = match &ship.view_mode {
        ViewMode::Camera(d) => d.clone(),
        ViewMode::Radar | ViewMode::ScienceRadar | ViewMode::SystemChart => ViewDirection::Fore,
    };
    let offset_dir = match direction {
        ViewDirection::Fore      => Vec3::new( ship.yaw.sin(), 0.0, -ship.yaw.cos()),
        ViewDirection::Aft       => Vec3::new(-ship.yaw.sin(), 0.0,  ship.yaw.cos()),
        ViewDirection::Port      => Vec3::new(-ship.yaw.cos(), 0.0, -ship.yaw.sin()),
        ViewDirection::Starboard => Vec3::new( ship.yaw.cos(), 0.0,  ship.yaw.sin()),
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
    phase: Res<CurrentPhase>,
    mut label_query: Query<(&Children, &mut Visibility), With<ViewDirectionLabel>>,
    mut text_query: Query<&mut Text>,
) {
    if !ship.is_changed() && !phase.is_changed() {
        return;
    }
    let Ok((children, mut vis)) = label_query.single_mut() else { return };
    if phase.0 != GamePhase::InProgress {
        *vis = Visibility::Hidden;
        return;
    }
    *vis = Visibility::Visible;
    let label = match &ship.view_mode {
        ViewMode::Camera(ViewDirection::Fore)      => "FORE",
        ViewMode::Camera(ViewDirection::Aft)       => "AFT",
        ViewMode::Camera(ViewDirection::Port)      => "PORT",
        ViewMode::Camera(ViewDirection::Starboard) => "STARBOARD",
        ViewMode::Radar                            => "RADAR",
        ViewMode::ScienceRadar                     => "SCIENCE RADAR",
        ViewMode::SystemChart                      => "SYSTEM CHART",
    };
    for child in children.iter() {
        if let Ok(mut text) = text_query.get_mut(child) {
            **text = label.to_string();
        }
    }
}

/// Pixel radius of the radar disc on screen.
const RADAR_PIXEL_RADIUS: f32 = 220.0;

/// Draws the radar overlay (outer ring, mid ring, ship triangle, asteroid pips)
/// using gizmos in the RadarCamera's 2D space. Only emits when InProgress and
/// the ship's view mode is Radar; otherwise it does nothing (so the gizmo
/// buffer is empty and nothing is rendered).
fn draw_radar_overlay(
    phase: Res<CurrentPhase>,
    ship: Res<ShipState>,
    world: Option<Res<WorldResource>>,
    mut gizmos: Gizmos,
) {
    if phase.0 != GamePhase::InProgress {
        return;
    }
    if !matches!(ship.view_mode, ViewMode::Radar) {
        return;
    }
    let centre = Vec2::ZERO;
    let outer  = Color::srgba(0.4, 0.9, 0.5, 0.9);
    let mid    = Color::srgba(0.4, 0.9, 0.5, 0.45);
    let ship_c = Color::srgba(0.6, 1.0, 0.7, 1.0);
    let aster  = Color::srgba(0.85, 0.7, 0.55, 0.95);

    // Outer ring (full radar range) and mid ring (half range).
    gizmos.circle_2d(centre, RADAR_PIXEL_RADIUS, outer);
    let mid_ratio = radar::RADAR_MID_RING / radar::RADAR_RANGE;
    gizmos.circle_2d(centre, RADAR_PIXEL_RADIUS * mid_ratio, mid);

    // Ship triangle at centre, pointing up (forward = +y on the radar).
    let tip   = Vec2::new(0.0,  10.0);
    let left  = Vec2::new(-7.0, -8.0);
    let right = Vec2::new( 7.0, -8.0);
    gizmos.line_2d(tip,  left,  ship_c);
    gizmos.line_2d(left, right, ship_c);
    gizmos.line_2d(right, tip,  ship_c);

    // Asteroid pips, projected through pure radar math.
    if let Some(world) = world {
        for (rx, ry, rr) in radar::radar_dots(&world.0.entities, ship.x, ship.z, ship.yaw) {
            let pos = Vec2::new(rx * RADAR_PIXEL_RADIUS, ry * RADAR_PIXEL_RADIUS);
            let pix_radius = (rr * RADAR_PIXEL_RADIUS).max(2.0);
            gizmos.circle_2d(pos, pix_radius, aster);
        }
    }
}

// ── VFX Systems ───────────────────────────────────────────────────

/// Draws phaser beams from port and starboard banks to the target asteroid
/// while `ActiveBeam` has a live target. Uses 3D gizmo lines in world space.
///
/// The origin of each beam is offset laterally from the ship centre to the
/// appropriate hull side via `beam_render::bank_origin`.  The endpoint is
/// the asteroid position, clamped to max range via `beam_render::beam_endpoint`.
/// The beam colour is taken from `PhaserRenderConfig` (configurable via ship TOML).
fn draw_beam_vfx(
    phase: Res<CurrentPhase>,
    ship: Res<ShipState>,
    beam: Res<ActiveBeam>,
    render_cfg: Res<PhaserRenderConfig>,
    world: Option<Res<WorldResource>>,
    mut gizmos: Gizmos,
) {
    if phase.0 != GamePhase::InProgress {
        return;
    }
    let Some(target_uuid) = &beam.target_uuid else { return };
    let Some(world) = world else { return };

    let Some(asteroid) = world.0.entities.iter().find(|a| &a.uuid == target_uuid) else {
        return;
    };

    // Endpoint clamped to configured max range.
    let (end_x, end_z) = beam_render::beam_endpoint(
        ship.x, ship.z,
        asteroid.x(), asteroid.z(),
        render_cfg.beam_range,
    );

    // Resolve beam colour from config.
    let [r, g, b, a] = render_cfg.beam_color;
    let beam_color = Color::srgba(r, g, b, a);
    let glow_color = Color::srgba(r, g * 1.5, b * 2.0, a * 0.35);

    // Draw a beam for each bank, originating from the correct hull side.
    for bank in [PhaserBank::Port, PhaserBank::Starboard] {
        let (ox, oz) = beam_render::bank_origin(
            ship.x, ship.z, ship.yaw, bank, beam_render::BANK_HULL_OFFSET,
        );
        let origin = Vec3::new(ox, -1.5, oz);
        let target = Vec3::new(end_x, 0.0, end_z);

        // Core bright beam line
        gizmos.line(origin, target, beam_color);

        // Slightly wider glow by drawing two offset parallel lines
        let perp = {
            let dx = target.x - origin.x;
            let dz = target.z - origin.z;
            let len = (dx * dx + dz * dz).sqrt().max(0.001);
            Vec3::new(-dz / len * 0.5, 0.0, dx / len * 0.5)
        };
        gizmos.line(origin + perp, target + perp, glow_color);
        gizmos.line(origin - perp, target - perp, glow_color);
    }
}

/// Spawns a `RippleEffect` entity for each `AsteroidDestroyedVfx` event received.
fn spawn_ripples(
    mut events: MessageReader<AsteroidDestroyedVfx>,
    mut commands: Commands,
) {
    for ev in events.read() {
        commands.spawn(RippleEffect { x: ev.x, z: ev.z, elapsed: 0.0 });
    }
}

/// Ticks all active `RippleEffect` entities: advances time, draws the expanding
/// ring via 3D gizmos, and despawns the entity once the animation completes.
fn tick_ripples(
    time: Res<Time>,
    phase: Res<CurrentPhase>,
    mut query: Query<(Entity, &mut RippleEffect)>,
    mut gizmos: Gizmos,
    mut commands: Commands,
) {
    if phase.0 != GamePhase::InProgress {
        return;
    }
    let dt = time.delta_secs();
    for (entity, mut ripple) in query.iter_mut() {
        ripple.elapsed += dt;
        if ripple.elapsed >= RIPPLE_DURATION {
            commands.entity(entity).despawn();
            continue;
        }

        let t = ripple.elapsed / RIPPLE_DURATION;           // 0..1
        let radius = RIPPLE_MAX_RADIUS * t;
        let alpha = (1.0 - t) * 0.85;                       // fade out

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

// ── Torpedo entity sync ───────────────────────────────────────────

/// Synchronises the set of torpedo sphere entities with the live torpedoes in
/// `TorpedoSystemResource`.
///
/// - Spawns a bright-yellow sphere for each torpedo that entered `in_flight`.
/// - Updates the `Transform` of every existing torpedo sphere each frame.
/// - Despawns sphere entities for torpedoes that have left `in_flight`.
///
/// Only runs during `InProgress` phase; despawns all remaining torpedo spheres
/// when the game is not in progress.
fn sync_torpedo_entities(
    phase: Res<CurrentPhase>,
    torpedo_sys: Option<Res<TorpedoSystemResource>>,
    mut entity_map: ResMut<TorpedoEntityMap>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut transforms: Query<&mut Transform, With<TorpedoSphere>>,
) {
    let Some(torpedo_sys) = torpedo_sys else { return };

    if phase.0 != GamePhase::InProgress {
        for (_, entity) in entity_map.0.drain() {
            commands.entity(entity).despawn();
        }
        return;
    }

    let in_flight = &torpedo_sys.0.in_flight;
    let in_flight_uuids: HashSet<String> = in_flight.iter().map(|t| t.uuid.clone()).collect();
    let tracked_uuids: HashSet<String> = entity_map.0.keys().cloned().collect();
    let (to_spawn, to_despawn) = diff_torpedo_sets(&in_flight_uuids, &tracked_uuids);

    for uuid in to_despawn {
        if let Some(entity) = entity_map.0.remove(&uuid) {
            commands.entity(entity).despawn();
        }
    }

    let torpedo_mesh = meshes.add(Sphere { radius: 1.0 });
    let torpedo_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(1.0, 1.0, 0.0),
        emissive: LinearRgba::new(1.0, 1.0, 0.0, 1.0),
        ..default()
    });

    for uuid in &to_spawn {
        if let Some(t) = in_flight.iter().find(|t| &t.uuid == uuid) {
            let entity = commands.spawn((
                TorpedoSphere,
                Mesh3d(torpedo_mesh.clone()),
                MeshMaterial3d(torpedo_mat.clone()),
                Transform::from_xyz(t.x, 0.0, t.z),
            )).id();
            entity_map.0.insert(uuid.clone(), entity);
        }
    }

    for t in in_flight {
        if let Some(&entity) = entity_map.0.get(&t.uuid) {
            if let Ok(mut transform) = transforms.get_mut(entity) {
                transform.translation.x = t.x;
                transform.translation.z = t.z;
            }
        }
    }
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
}
