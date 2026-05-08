use bevy::prelude::*;

use crate::client::app::OutboundClientMessage;
use crate::client::lobby_state::{LobbyState, LobbyView, LocalPlayerToken};
use crate::client::sim_state::{
    message_for_direction_press, red_alert_toggle_message, ClientSimState,
};
use crate::shared::messages::{GamePhase, ViewDirection};

// ── Marker components ──────────────────────────────────────────────

/// Marks the root of the captain console UI (view selector + Red Alert);
/// shown only when the local player holds CaptainChair and the phase is
/// InProgress.
#[derive(Component)]
pub struct CaptainPanel;

/// Marks one direction button in the view-selector cross.
#[derive(Component)]
pub struct ViewDirButton(pub ViewDirection);

/// Marks the Red Alert toggle button so its background and label can
/// reflect the current `ClientSimState.red_alert`.
#[derive(Component)]
pub struct RedAlertButton;

/// Marks the text node *inside* the Red Alert button so we can update
/// the "ON"/"OFF" label without rebuilding the button entity.
#[derive(Component)]
pub struct RedAlertLabel;

// ── Constants ──────────────────────────────────────────────────────

/// Background colour for an inactive direction button in the cross.
pub const VIEW_BTN_BG_INACTIVE: Color = Color::srgb(0.13, 0.13, 0.27);
/// Background colour for the currently active direction button.
pub const VIEW_BTN_BG_ACTIVE:   Color = Color::srgb(0.20, 0.24, 0.40);
/// Background for the Red Alert toggle when alert is OFF.
pub const RED_ALERT_BG_OFF: Color = Color::srgb(0.13, 0.13, 0.27);
/// Background for the Red Alert toggle when alert is ON (deep red).
pub const RED_ALERT_BG_ON:  Color = Color::srgb(0.40, 0.0, 0.0);

// ── Plugin ─────────────────────────────────────────────────────────

pub struct CaptainConsolePlugin;

impl Plugin for CaptainConsolePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup_captain_ui)
            .add_systems(Update, (
                toggle_captain_panel_visibility,
                refresh_view_dir_highlights,
                refresh_red_alert_button,
                handle_view_dir_button_press,
                handle_red_alert_button_press,
            ));
    }
}

// ── Setup ──────────────────────────────────────────────────────────

fn setup_captain_ui(mut commands: Commands) {
    commands
        .spawn((
            CaptainPanel,
            Node {
                position_type: PositionType::Absolute,
                right: Val::Px(16.0),
                top:   Val::Px(16.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::FlexEnd,
                row_gap: Val::Px(8.0),
                ..default()
            },
            Visibility::Hidden,
        ))
        .with_children(|panel| {
            // ── View selector cross — 3×3 grid ──────────────────────
            panel
                .spawn((
                    Node {
                        display: Display::Grid,
                        grid_template_columns: vec![
                            GridTrack::px(48.0), GridTrack::px(48.0), GridTrack::px(48.0),
                        ],
                        grid_template_rows: vec![
                            GridTrack::px(40.0), GridTrack::px(40.0), GridTrack::px(40.0),
                        ],
                        column_gap: Val::Px(4.0),
                        row_gap:    Val::Px(4.0),
                        ..default()
                    },
                ))
                .with_children(|grid| {
                    spawn_view_dir_button(grid, ViewDirection::Fore,      "▲", 2, 1);
                    spawn_view_dir_button(grid, ViewDirection::Port,      "◄", 1, 2);
                    spawn_view_label(grid, 2, 2);
                    spawn_view_dir_button(grid, ViewDirection::Starboard, "►", 3, 2);
                    spawn_view_dir_button(grid, ViewDirection::Aft,       "▼", 2, 3);
                });

            // ── Red Alert toggle ────────────────────────────────────
            panel
                .spawn((
                    RedAlertButton,
                    Button,
                    Node {
                        padding: UiRect::all(Val::Px(10.0)),
                        ..default()
                    },
                    BackgroundColor(RED_ALERT_BG_OFF),
                ))
                .with_children(|btn| {
                    btn.spawn((
                        RedAlertLabel,
                        Text::new("Red Alert: OFF"),
                        TextFont { font_size: 16.0, ..default() },
                        TextColor(Color::srgb(0.93, 0.93, 1.0)),
                    ));
                });
        });
}

pub fn spawn_view_dir_button(
    grid: &mut ChildSpawnerCommands,
    direction: ViewDirection,
    glyph: &str,
    column: i16,
    row: i16,
) {
    grid.spawn((
        ViewDirButton(direction),
        Button,
        Node {
            grid_column: GridPlacement::start(column),
            grid_row:    GridPlacement::start(row),
            justify_content: JustifyContent::Center,
            align_items:     AlignItems::Center,
            ..default()
        },
        BackgroundColor(VIEW_BTN_BG_INACTIVE),
    ))
    .with_children(|inner| {
        inner.spawn((
            Text::new(glyph),
            TextFont { font_size: 22.0, ..default() },
            TextColor(Color::srgb(0.93, 0.93, 1.0)),
        ));
    });
}

pub fn spawn_view_label(grid: &mut ChildSpawnerCommands, column: i16, row: i16) {
    grid.spawn((
        Node {
            grid_column: GridPlacement::start(column),
            grid_row:    GridPlacement::start(row),
            justify_content: JustifyContent::Center,
            align_items:     AlignItems::Center,
            ..default()
        },
    ))
    .with_children(|inner| {
        inner.spawn((
            Text::new("View"),
            TextFont { font_size: 12.0, ..default() },
            TextColor(Color::srgb(0.6, 0.7, 0.73)),
        ));
    });
}

// ── Systems ────────────────────────────────────────────────────────

fn toggle_captain_panel_visibility(
    lobby: Res<LobbyState>,
    token: Res<LocalPlayerToken>,
    mut panel: Query<&mut Visibility, With<CaptainPanel>>,
) {
    if !lobby.is_changed() && !token.is_changed() {
        return;
    }
    let view = LobbyView::new(&lobby, &token.0);
    let visible = lobby.phase == GamePhase::InProgress && view.is_captain();
    for mut vis in panel.iter_mut() {
        *vis = if visible { Visibility::Visible } else { Visibility::Hidden };
    }
}

fn refresh_view_dir_highlights(
    sim: Res<ClientSimState>,
    mut buttons: Query<(&ViewDirButton, &mut BackgroundColor)>,
) {
    if !sim.is_changed() {
        return;
    }
    for (ViewDirButton(direction), mut bg) in buttons.iter_mut() {
        let active = sim.is_active_camera_direction(direction);
        bg.0 = if active { VIEW_BTN_BG_ACTIVE } else { VIEW_BTN_BG_INACTIVE };
    }
}

fn refresh_red_alert_button(
    sim: Res<ClientSimState>,
    mut button: Query<(Entity, &mut BackgroundColor), With<RedAlertButton>>,
    children_q: Query<&Children>,
    mut labels: Query<&mut Text, With<RedAlertLabel>>,
) {
    if !sim.is_changed() {
        return;
    }
    let Ok((button_entity, mut bg)) = button.single_mut() else { return };
    bg.0 = if sim.red_alert { RED_ALERT_BG_ON } else { RED_ALERT_BG_OFF };
    if let Ok(children) = children_q.get(button_entity) {
        for child in children.iter() {
            if let Ok(mut text) = labels.get_mut(child) {
                **text = if sim.red_alert {
                    "Red Alert: ON".to_string()
                } else {
                    "Red Alert: OFF".to_string()
                };
            }
        }
    }
}

fn handle_view_dir_button_press(
    mut interactions: Query<
        (&Interaction, &ViewDirButton),
        (Changed<Interaction>, With<Button>),
    >,
    mut outbound: MessageWriter<OutboundClientMessage>,
) {
    for (interaction, ViewDirButton(direction)) in interactions.iter_mut() {
        if *interaction == Interaction::Pressed {
            outbound.write(OutboundClientMessage(message_for_direction_press(direction.clone())));
        }
    }
}

fn handle_red_alert_button_press(
    mut interactions: Query<
        &Interaction,
        (Changed<Interaction>, With<Button>, With<RedAlertButton>),
    >,
    mut outbound: MessageWriter<OutboundClientMessage>,
) {
    for interaction in interactions.iter_mut() {
        if *interaction == Interaction::Pressed {
            outbound.write(OutboundClientMessage(red_alert_toggle_message()));
        }
    }
}
