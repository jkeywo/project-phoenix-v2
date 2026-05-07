use bevy::prelude::*;

use crate::lobby::{CurrentPhase, Sessions};
use crate::messages::{Console, GamePhase};
use crate::ship_state::ShipState;

// ── Marker Components ─────────────────────────────────────────────

#[derive(Component)]
struct LobbyCamera;

#[derive(Component)]
struct GameCamera;

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

// ── Plugin ────────────────────────────────────────────────────────

pub struct RendererPlugin;

impl Plugin for RendererPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup)
            .add_systems(Update, (
                update_fps_counter,
                toggle_cameras,
                toggle_lobby_items,
                update_player_list,
                update_view_screen_text,
                follow_camera,
            ));
    }
}

// ── Setup ─────────────────────────────────────────────────────────

fn setup(
    mut commands: Commands,
) {
    // 2D camera — active during lobby phase
    commands.spawn((LobbyCamera, Camera2d, Camera { order: 0, ..default() }));

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

    // Lobby: panel anchored top-left via node UI
    commands
        .spawn((
            LobbyItem,
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::FlexStart,
                align_items: AlignItems::FlexStart,
                padding: UiRect::all(Val::Px(12.0)),
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

fn update_player_list(
    sessions: Res<Sessions>,
    mut query: Query<&mut Text, With<PlayerListText>>,
) {
    if !sessions.is_changed() {
        return;
    }
    let Ok(mut text) = query.single_mut() else { return };
    let mut content = "Players:\n".to_string();
    for p in sessions.0.players() {
        let consoles: String = {
            let names: Vec<String> = p.consoles.iter().map(|c| c.display_name().to_string()).collect();
            if names.is_empty() {
                String::new()
            } else {
                format!("({})", names.join(", "))
            }
        };
        if consoles.is_empty() {
            content.push_str(&format!("• {}\n", p.name));
        } else {
            content.push_str(&format!("• {} — {}\n", p.name, consoles));
        }
    }
    **text = content;
}

fn update_view_screen_text(
    sessions: Res<Sessions>,
    phase: Res<CurrentPhase>,
    mut query: Query<(&mut Text, &mut Visibility), With<ViewScreenText>>,
) {
    if !sessions.is_changed() && !phase.is_changed() {
        return;
    }
    let Ok((mut text, mut vis)) = query.single_mut() else { return };
    if phase.0 != GamePhase::InProgress {
        *vis = Visibility::Hidden;
        return;
    }
    *vis = Visibility::Visible;
    let players = sessions.0.players();
    let mut content = "VIEW SCREEN\n".to_string();
    for console in [Console::CaptainChair, Console::Helm] {
        let label = console.display_name();
        let holder_name = sessions.0.console_holder(console)
            .and_then(|token| players.iter().find(|p| p.token == token))
            .map(|p| p.name.as_str())
            .unwrap_or("—");
        content.push_str(&format!("{}: {}\n", label, holder_name));
    }
    **text = content;
}

/// Forward camera: 1 unit in front of the ship, 0 units above, looking ahead to the horizon.
///
/// Bevy is left-handed: camera local +Z = forward view direction.
/// Ship forward: (sin yaw, 0, -cos yaw).  yaw=0 → ship faces -Z.
/// Camera must sit behind the ship (+Z at yaw=0) and look further ahead (-Z).
fn follow_camera(
    ship: Res<ShipState>,
    mut cam_query: Query<&mut Transform, With<GameCamera>>,
    phase: Res<CurrentPhase>,
) {
    if phase.0 != GamePhase::InProgress {
        return;
    }
    let Ok(mut transform) = cam_query.single_mut() else { return };

    // Ship forward direction
    let fwd_x = ship.yaw.sin();
    let fwd_z = -ship.yaw.cos();

    // Camera: 1 units in front of the ship, 0 units above
    transform.translation = Vec3::new(
        ship.x + fwd_x * 1.0,
        0.0,
        ship.z + fwd_z * 1.0,
    );

    // Look point: 20 units ahead of the ship along its forward axis, at ship altitude
    let look_at = Vec3::new(
        ship.x + fwd_x * 20.0,
        0.0,
        ship.z + fwd_z * 20.0,
    );
    transform.look_at(look_at, Vec3::Y);
}
