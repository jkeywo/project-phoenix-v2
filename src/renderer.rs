use bevy::prelude::*;

use crate::lobby::{CurrentPhase, Sessions};
use crate::messages::GamePhase;
use crate::simulation::RedAlertState;

// ── Marker Components ──────────────────────────────────────────────────────

#[derive(Component)]
struct LobbyCamera;

#[derive(Component)]
struct GameCamera;

/// Marks entities that belong to the lobby scene (text, QR placeholder).
#[derive(Component)]
struct LobbyItem;

#[derive(Component)]
struct PlayerListText;

#[derive(Component)]
struct RotatingCube;

// ── Plugin ─────────────────────────────────────────────────────────────────

pub struct RendererPlugin;

impl Plugin for RendererPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup).add_systems(
            Update,
            (
                toggle_cameras,
                toggle_lobby_items,
                toggle_cube,
                update_player_list,
                rotate_cube,
                sync_cube_color,
            ),
        );
    }
}

// ── Setup ──────────────────────────────────────────────────────────────────

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    // 2D camera — active during lobby phase
    commands.spawn((LobbyCamera, Camera2d, Camera { order: 0, ..default() }));

    // 3D camera — active during in-game phase
    commands.spawn((
        GameCamera,
        Camera3d::default(),
        Camera { is_active: false, order: 0, ..default() },
        Transform::from_xyz(0.0, 2.0, 6.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));

    // Directional light for the 3D scene
    commands.spawn((
        DirectionalLight { illuminance: 5_000.0, ..default() },
        Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, -0.9, 0.5, 0.0)),
    ));

    // Lobby: player list
    commands.spawn((
        LobbyItem,
        PlayerListText,
        Text2d::new("Players:\n—"),
        Transform::from_xyz(-300.0, 150.0, 0.0),
    ));

    // Game: rotating cube (hidden until game starts)
    commands.spawn((
        RotatingCube,
        Mesh3d(meshes.add(Cuboid::default())),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::WHITE,
            ..default()
        })),
        Transform::default(),
        Visibility::Hidden,
    ));
}

// ── Systems ────────────────────────────────────────────────────────────────

fn toggle_cameras(
    phase: Res<CurrentPhase>,
    mut lobby: Query<&mut Camera, (With<LobbyCamera>, Without<GameCamera>)>,
    mut game: Query<&mut Camera, (With<GameCamera>, Without<LobbyCamera>)>,
) {
    if !phase.is_changed() {
        return;
    }
    let in_game = phase.0 == GamePhase::InProgress;
    if let Ok(mut cam) = lobby.single_mut() {
        cam.is_active = !in_game;
    }
    if let Ok(mut cam) = game.single_mut() {
        cam.is_active = in_game;
    }
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

fn toggle_cube(
    phase: Res<CurrentPhase>,
    mut query: Query<&mut Visibility, With<RotatingCube>>,
) {
    if !phase.is_changed() {
        return;
    }
    if let Ok(mut vis) = query.single_mut() {
        *vis = if phase.0 == GamePhase::InProgress {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
    }
}

fn update_player_list(
    sessions: Res<Sessions>,
    mut query: Query<&mut Text2d, With<PlayerListText>>,
) {
    if !sessions.is_changed() {
        return;
    }
    let Ok(mut text) = query.single_mut() else { return };
    let mut content = "Players:\n".to_string();
    for p in sessions.0.players() {
        let console = p.console.as_ref().map(|c| format!("{c:?}")).unwrap_or_default();
        content.push_str(&format!("• {} {}\n", p.name, console));
    }
    text.0 = content;
}

fn rotate_cube(
    time: Res<Time>,
    phase: Res<CurrentPhase>,
    mut query: Query<&mut Transform, With<RotatingCube>>,
) {
    if phase.0 != GamePhase::InProgress {
        return;
    }
    let dt = time.delta_secs();
    for mut transform in query.iter_mut() {
        transform.rotate_y(dt * 0.8);
        transform.rotate_x(dt * 0.5);
    }
}

fn sync_cube_color(
    red_alert: Res<RedAlertState>,
    cube_query: Query<&MeshMaterial3d<StandardMaterial>, With<RotatingCube>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    if !red_alert.is_changed() {
        return;
    }
    let Ok(handle) = cube_query.single() else { return };
    if let Some(mat) = materials.get_mut(handle.id()) {
        mat.base_color = if red_alert.active {
            Color::srgb(1.0, 0.0, 0.0)
        } else {
            Color::WHITE
        };
    }
}
