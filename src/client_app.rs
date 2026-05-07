//! Client-side Bevy app — lobby UI and (future) in-game UI.
//!
//! This plugin owns the `LobbyState` and `LocalPlayerToken` resources,
//! drains inbound `ServerMessage` events, re-renders the lobby UI on
//! state changes, and emits outbound `ClientMessage` events when buttons
//! are pressed. Outbound emission is the only side effect that escapes
//! the plugin; the bridge layer (`client_bridge`) is responsible for
//! marshalling those events to/from JS.
//!
//! The plugin is platform-agnostic; the bridge layer wires it together
//! with `DefaultPlugins` and the wasm-bindgen entry points.

use bevy::prelude::*;

use crate::client_lobby::{
    engage_message, message_for_slot_click, ConsoleSlot, LobbyState, LobbyView, LocalPlayerToken,
};
use crate::client_sim::{
    message_for_direction_press, red_alert_toggle_message, ClientSimState,
};
use crate::messages::{ClientMessage, Console, GamePhase, ServerMessage, ViewDirection};

// ── Events ─────────────────────────────────────────────────────────

/// Fired by the bridge each time JS hands the WASM client an inbound
/// `ServerMessage`. The plugin consumes these to update `LobbyState`.
#[derive(Message, Clone, Debug)]
pub struct InboundServerMessage(pub ServerMessage);

/// Fired by the plugin whenever a UI interaction needs to send a
/// `ClientMessage` to the host. The bridge layer drains these and
/// forwards JSON over the JS callback.
#[derive(Message, Clone, Debug)]
pub struct OutboundClientMessage(pub ClientMessage);

// ── Marker components ──────────────────────────────────────────────

/// Marks the root node of the lobby UI so it can be shown/hidden when
/// the phase changes.
#[derive(Component)]
struct LobbyRoot;

/// Marks the container of the per-console buttons so it can be cleared
/// and rebuilt on every `LobbyState` change.
#[derive(Component)]
struct ConsoleListRoot;

/// Marks the container of the player list lines.
#[derive(Component)]
struct PlayerListRoot;

/// Marks the Engage button so we can toggle its visibility per captaincy.
#[derive(Component)]
struct EngageButton;

/// Marks one console-row button and remembers which `Console` it acts on.
#[derive(Component)]
struct ConsoleButton(Console);

/// Marks the root of the captain console UI (view selector + Red Alert);
/// shown only when the local player holds CaptainChair and the phase is
/// InProgress.
#[derive(Component)]
struct CaptainPanel;

/// Marks one direction button in the view-selector cross.
#[derive(Component)]
struct ViewDirButton(ViewDirection);

/// Marks the Red Alert toggle button so its background and label can
/// reflect the current `ClientSimState.red_alert`.
#[derive(Component)]
struct RedAlertButton;

/// Marks the text node *inside* the Red Alert button so we can update
/// the "ON"/"OFF" label without rebuilding the button entity.
#[derive(Component)]
struct RedAlertLabel;

// ── Plugin ─────────────────────────────────────────────────────────

pub struct ClientAppPlugin;

impl Plugin for ClientAppPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LobbyState>()
            .init_resource::<ClientSimState>()
            .init_resource::<LocalPlayerToken>()
            .add_message::<InboundServerMessage>()
            .add_message::<OutboundClientMessage>()
            .add_systems(Startup, (setup_lobby_ui, setup_captain_ui))
            .add_systems(
                Update,
                (
                    apply_inbound_messages,
                    rebuild_lobby_ui_on_change,
                    toggle_lobby_visibility_on_phase,
                    toggle_captain_panel_visibility,
                    refresh_view_dir_highlights,
                    refresh_red_alert_button,
                    handle_console_button_press,
                    handle_engage_button_press,
                    handle_view_dir_button_press,
                    handle_red_alert_button_press,
                ),
            );
    }
}

// ── Setup ──────────────────────────────────────────────────────────

fn setup_lobby_ui(mut commands: Commands) {
    // 2D camera for UI rendering.
    commands.spawn(Camera2d);

    commands
        .spawn((
            LobbyRoot,
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::FlexStart,
                padding: UiRect::all(Val::Px(16.0)),
                row_gap: Val::Px(8.0),
                ..default()
            },
        ))
        .with_children(|root| {
            root.spawn((
                Text::new("Lobby"),
                TextFont { font_size: 22.0, ..default() },
                TextColor(Color::srgb(0.53, 0.67, 1.0)),
            ));

            root.spawn((
                ConsoleListRoot,
                Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(4.0),
                    ..default()
                },
            ));

            root.spawn((
                EngageButton,
                Button,
                Node {
                    padding: UiRect::all(Val::Px(10.0)),
                    margin: UiRect::top(Val::Px(8.0)),
                    ..default()
                },
                BackgroundColor(Color::srgb(0.13, 0.13, 0.27)),
                Visibility::Hidden,
            ))
            .with_children(|btn| {
                btn.spawn((
                    Text::new("Engage"),
                    TextFont { font_size: 16.0, ..default() },
                    TextColor(Color::srgb(0.93, 0.93, 1.0)),
                ));
            });

            root.spawn((
                PlayerListRoot,
                Node {
                    flex_direction: FlexDirection::Column,
                    margin: UiRect::top(Val::Px(12.0)),
                    row_gap: Val::Px(2.0),
                    ..default()
                },
            ));
        });
}

// ── Systems ────────────────────────────────────────────────────────

fn apply_inbound_messages(
    mut reader: MessageReader<InboundServerMessage>,
    mut lobby: ResMut<LobbyState>,
    mut sim: ResMut<ClientSimState>,
) {
    for ev in reader.read() {
        lobby.apply(&ev.0);
        sim.apply(&ev.0);
    }
}

fn toggle_lobby_visibility_on_phase(
    state: Res<LobbyState>,
    mut roots: Query<&mut Visibility, With<LobbyRoot>>,
) {
    if !state.is_changed() {
        return;
    }
    let in_lobby = state.phase == GamePhase::Lobby;
    for mut vis in roots.iter_mut() {
        *vis = if in_lobby { Visibility::Visible } else { Visibility::Hidden };
    }
}

fn rebuild_lobby_ui_on_change(
    mut commands: Commands,
    state: Res<LobbyState>,
    token: Res<LocalPlayerToken>,
    console_root: Query<Entity, With<ConsoleListRoot>>,
    player_root: Query<Entity, With<PlayerListRoot>>,
    children_q: Query<&Children>,
    mut engage: Query<&mut Visibility, With<EngageButton>>,
) {
    if !state.is_changed() && !token.is_changed() {
        return;
    }
    let view = LobbyView::new(&state, &token.0);

    // Console buttons.
    if let Ok(root) = console_root.single() {
        if let Ok(children) = children_q.get(root) {
            for child in children.iter() {
                commands.entity(child).despawn();
            }
        }
        commands.entity(root).with_children(|parent| {
            for slot in view.console_slots() {
                spawn_console_row(parent, &slot);
            }
        });
    }

    // Player list lines.
    if let Ok(root) = player_root.single() {
        if let Ok(children) = children_q.get(root) {
            for child in children.iter() {
                commands.entity(child).despawn();
            }
        }
        commands.entity(root).with_children(|parent| {
            for p in &state.players {
                let mark = if p.token == token.0 { "▶ " } else { "• " };
                let consoles = if p.consoles.is_empty() {
                    String::new()
                } else {
                    let names: Vec<&str> =
                        p.consoles.iter().map(|c| c.display_name()).collect();
                    format!(" — {}", names.join(", "))
                };
                parent.spawn((
                    Text::new(format!("{mark}{}{consoles}", p.name)),
                    TextFont { font_size: 13.0, ..default() },
                    TextColor(Color::srgb(0.6, 0.7, 0.73)),
                ));
            }
        });
    }

    // Engage visibility.
    if let Ok(mut vis) = engage.single_mut() {
        *vis = if view.is_captain() {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
    }
}

fn spawn_console_row(parent: &mut ChildSpawnerCommands, slot: &ConsoleSlot) {
    let (label, console_for_click, bg, fg) = match slot {
        ConsoleSlot::Available { console } => (
            format!("{}: available", console.display_name()),
            Some(console.clone()),
            Color::srgb(0.13, 0.13, 0.27),
            Color::srgb(0.93, 0.93, 1.0),
        ),
        ConsoleSlot::Occupied { console, holder_name } => (
            format!("{}: {}", console.display_name(), holder_name),
            None,
            Color::srgb(0.07, 0.07, 0.10),
            Color::srgb(0.42, 0.49, 0.55),
        ),
        ConsoleSlot::Mine { console } => (
            format!("{}: Mine — release", console.display_name()),
            Some(console.clone()),
            Color::srgb(0.20, 0.24, 0.40),
            Color::srgb(0.55, 0.70, 1.0),
        ),
    };

    let mut row = parent.spawn((
        Node {
            padding: UiRect::all(Val::Px(8.0)),
            ..default()
        },
        BackgroundColor(bg),
    ));
    if let Some(c) = console_for_click {
        row.insert((Button, ConsoleButton(c)));
    }
    row.with_children(|inner| {
        inner.spawn((
            Text::new(label),
            TextFont { font_size: 14.0, ..default() },
            TextColor(fg),
        ));
    });
}

fn handle_console_button_press(
    mut interactions: Query<
        (&Interaction, &ConsoleButton),
        (Changed<Interaction>, With<Button>),
    >,
    mut outbound: MessageWriter<OutboundClientMessage>,
) {
    for (interaction, ConsoleButton(c)) in interactions.iter_mut() {
        if *interaction != Interaction::Pressed {
            continue;
        }
        // Re-derive the message via the slot helper so the same rule
        // (ConsoleSlot → ClientMessage) governs both code paths.
        let slot = ConsoleSlot::Available { console: c.clone() };
        if let Some(msg) = message_for_slot_click(&slot) {
            outbound.write(OutboundClientMessage(msg));
        }
    }
}

fn handle_engage_button_press(
    mut interactions: Query<
        &Interaction,
        (Changed<Interaction>, With<Button>, With<EngageButton>),
    >,
    mut outbound: MessageWriter<OutboundClientMessage>,
) {
    for interaction in interactions.iter_mut() {
        if *interaction == Interaction::Pressed {
            outbound.write(OutboundClientMessage(engage_message()));
        }
    }
}

// ── Captain console UI ─────────────────────────────────────────────

/// Background colour for an inactive direction button in the cross.
const VIEW_BTN_BG_INACTIVE: Color = Color::srgb(0.13, 0.13, 0.27);
/// Background colour for the currently active direction button.
const VIEW_BTN_BG_ACTIVE:   Color = Color::srgb(0.20, 0.24, 0.40);
/// Background for the Red Alert toggle when alert is OFF.
const RED_ALERT_BG_OFF: Color = Color::srgb(0.13, 0.13, 0.27);
/// Background for the Red Alert toggle when alert is ON (deep red).
const RED_ALERT_BG_ON:  Color = Color::srgb(0.40, 0.0, 0.0);

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

fn spawn_view_dir_button(
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

fn spawn_view_label(grid: &mut ChildSpawnerCommands, column: i16, row: i16) {
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
