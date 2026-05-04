use bevy::prelude::*;

use crate::lobby::{CurrentPhase, Sessions};
use crate::messages::GamePhase;
use crate::ship_state::ShipState;

// ── Marker Components ─────────────────────────────

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

/// Marks the Red Alert border overlay UI element.
#[derive(Component)]
struct RedAlertOver;

// ── Plugin ─────�────��─�─────────────���────���─

pub struct RendererPlugin;

impl Plugin for RendererPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup).add_systems(
            Update,
            (
                toggle_cameras,
                toggle_lobby_items,
                update_player_list,
                sync_red_alert_border,
            ),
        );
    }
}

// ── Setup ────────�─────���────�────���────���────��─���────���──

fn setup(
    mut commands: Commands,
) {
    // 2D camera — active during lobby phase
    commands.spawn((LobbyCamera, Camera2d, Camera { order: 0, ..default() }));

    // 3D camera — active during in-game phase, positioned for ship view
    commands.spawn((
        GameCamera,
        Camera3d::default(),
        Camera { is_active: false, order: 0, ..default() },
        Transform::from_xyz(0.0, 2.0, -10.0),
    ));

    // Directional light for the 3D scene
    commands.spawn((
        DirectionalLight { illuminance: 5_000.0, ..default() },
        Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, -0.9, 0.5, 0.0)),
    ));

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

    // Red Alert overlay — thin red borders on all four edges.
    // Each edge is a separate UI rect for proper coloring.
    for pos in &[PositionType::Absolute] {
        // Border container
        let mut border_root = commands.spawn((
            Node {
                position_type: *pos,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                padding: UiRect::ZERO,
                ..default()
            },
            Visibility::Hidden,
        ));

        // Four border strips
        for (_side, size) in &[
            ("top", (Val::Percent(100.0), Val::Px(8.0))),
            ("bottom", (Val::Percent(100.0), Val::Px(8.0))),
            ("left", (Val::Px(8.0), Val::Percent(100.0))),
            ("right", (Val::Px(8.0), Val::Percent(100.0))),
        ] {
            border_root.with_children(|c| {
                c.spawn((
                    Node {
                        width: size.0,
                        height: size.1,
                        ..default()
                    },
                    BackgroundColor(Color::srgb(1.0, 0.0, 0.0)),
                ));
            });
        }

        border_root.insert(RedAlertOver);
    }
}

// ── Systems ─────���──���────���────���────�──���────���────���────

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
        let console = p.console.as_ref().map(|c| format!("{c:?}")).unwrap_or_default();
        if console.is_empty() {
            content.push_str(&format!("• {}\n", p.name));
        } else {
            content.push_str(&format!("• {} - {}\n", p.name, console));
        }
    }
    **text = content;
}

/// Sync Red Alert overlay visibility based on ShipState::red_alert.
fn sync_red_alert_border(
    ship: Res<ShipState>,
    mut query: Query<&mut Visibility, With<RedAlertOver>>,
    phase: Res<CurrentPhase>,
) {
    // Only show during game phase
    if phase.0 != GamePhase::InProgress {
        return;
    }

    let Ok(mut visibility) = query.single_mut() else { return };

    if ship.is_changed() {
        if ship.snapshot().red_alert {
            *visibility = Visibility::Inherited;
        } else {
            *visibility = Visibility::Hidden;
        }
    }
}
