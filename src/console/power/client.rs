//! Client-side Power Panel plugin.
//!
//! Owns all Power console UI: per-console power allocation rows
//! (Helm/Weapons/Sensors), battery bar, lock state indicator, and overflow
//! allocation controls (hidden in Low complexity).
//!
//! Extracted from `client/app.rs` as part of the "Client split" series.
//! Compiled only when the `client` Cargo feature is enabled.

use bevy::prelude::*;

use crate::client_app::{OutboundClientMessage, HideableElement};
use crate::client_lobby::{ActiveConsole, LobbyState, LobbyView, LocalPlayerToken};
use crate::client_sim::ClientSimState;
use crate::messages::{Console, GamePhase};
use crate::ship_view::ShipView;

// ── Pure visibility helper ────────────────────────────────────────────

/// Decide whether the power panel should be visible.
///
/// Rules:
/// 1. Game phase must be `InProgress`.
/// 2. The local player must hold `Console::Power`.
/// 3. If holding **one console only**, show automatically.
/// 4. If holding **multiple consoles**, show only when `ActiveConsole`
///    is explicitly set to `Power`.
pub fn power_panel_visible(
    lobby: &LobbyState,
    token: &str,
    active: &ActiveConsole,
) -> bool {
    if lobby.phase != GamePhase::InProgress {
        return false;
    }
    let view = LobbyView::new(lobby, token);
    let consoles = view.my_consoles();
    if !consoles.contains(&Console::Power) {
        return false;
    }
    let count = consoles.len();
    match &active.0 {
        Some(c) => *c == Console::Power,
        None => count == 1,
    }
}

// ── Colour constants ─────────────────────────────────────────────────

const POWER_COL_INACTIVE: Color = Color::srgb(0.08, 0.08, 0.12);
const POWER_COL_LOCKED: Color = Color::srgb(0.06, 0.06, 0.08);
const POWER_INC_COLOR: Color = Color::srgb(0.10, 0.50, 0.30);
const POWER_INC_LOCKED: Color = Color::srgb(0.06, 0.06, 0.10);
const POWER_DEC_COLOR: Color = Color::srgb(0.50, 0.20, 0.10);
const POWER_DEC_LOCKED: Color = Color::srgb(0.06, 0.06, 0.10);
const POWER_BATTERY_BG: Color = Color::srgb(0.06, 0.06, 0.15);
const POWER_BATTERY_FILL: Color = Color::srgb(0.10, 0.60, 0.80);

// ── Marker components ────────────────────────────────────────────────

/// Marks the root of the power console UI; shown only when the local
/// player holds Power and the phase is InProgress.
#[derive(Component)]
pub struct PowerPanel;

/// Marks one power allocation row container, carrying the console it controls.
#[derive(Component)]
#[allow(dead_code)]
struct PowerRow(Console);

/// Marks the label showing current level for a row (inside that row).
/// Carries the console it represents for refresh matching.
#[derive(Component)]
struct PowerRowLevel(Console);

/// Marks the increment button for a power row. Carries the target console.
#[derive(Component)]
struct PowerIncButton(Console);

/// Marks the decrement button for a power row. Carries the target console.
#[derive(Component)]
struct PowerDecButton(Console);

/// Marks the battery bar fill node.
#[derive(Component)]
struct BatteryBar;

/// Marks the battery percentage text label.
#[derive(Component)]
struct BatteryLabel;

// ── Plugin ───────────────────────────────────────────────────────────

/// Plugin that owns all Power console UI and systems.
pub struct PowerPanelPlugin;

impl Plugin for PowerPanelPlugin {
    fn build(&self, app: &mut App) {
        app
            .add_systems(Startup, setup_power_ui)
            .add_systems(Update, (
                toggle_power_panel_visibility,
                refresh_power_panel,
                handle_increase_power,
                handle_decrease_power,
            ));
    }
}

// ── Setup ────────────────────────────────────────────────────────────

fn setup_power_ui(mut commands: Commands) {
    commands
        .spawn((
            PowerPanel,
            Node {
                position_type: PositionType::Absolute,
                left:   Val::Px(0.0),
                top:    Val::Px(0.0),
                right:  Val::Px(0.0),
                bottom: Val::Px(0.0),
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                row_gap: Val::Px(12.0),
                padding: UiRect::all(Val::Px(24.0)),
                ..default()
            },
            Visibility::Hidden,
        ))
        .with_children(|panel| {
            panel.spawn((
                Text::new("Power Console"),
                TextFont { font_size: 24.0, ..default() },
                TextColor(Color::srgb(0.3, 1.0, 0.8)),
            ));

            // Three power rows: Helm, Weapons, Sensors
            for (console, label) in [
                (Console::Helm, "Helm"),
                (Console::Tactical, "Weapons"),
                (Console::Sensors, "Sensors"),
            ] {
                panel.spawn((
                    PowerRow(console.clone()),
                    Node {
                        flex_direction: FlexDirection::Row,
                        align_items: AlignItems::Center,
                        column_gap: Val::Px(12.0),
                        padding: UiRect::axes(Val::Px(16.0), Val::Px(8.0)),
                        ..default()
                    },
                    BackgroundColor(POWER_COL_INACTIVE),
                )).with_children(|row| {
                    // Console name
                    row.spawn((
                        Text::new(label),
                        TextFont { font_size: 16.0, ..default() },
                        TextColor(Color::srgb(0.7, 0.9, 1.0)),
                        Node { width: Val::Px(80.0), ..default() },
                    ));
                    // Decrement button
                    row.spawn((
                        PowerDecButton(console.clone()),
                        Button,
                        Node {
                            width: Val::Px(36.0), height: Val::Px(36.0),
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            ..default()
                        },
                        BackgroundColor(POWER_DEC_COLOR),
                    )).with_children(|btn| {
                        btn.spawn((
                            Text::new("-"),
                            TextFont { font_size: 22.0, ..default() },
                            TextColor(Color::srgb(0.9, 0.9, 1.0)),
                        ));
                    });
                    // Level text
                    row.spawn((
                        PowerRowLevel(console.clone()),
                        Text::new("2"),
                        TextFont { font_size: 18.0, ..default() },
                        TextColor(Color::srgb(0.9, 0.9, 1.0)),
                        Node { min_width: Val::Px(24.0), justify_content: JustifyContent::Center, ..default() },
                    ));
                    // Increment button
                    row.spawn((
                        PowerIncButton(console.clone()),
                        Button,
                        Node {
                            width: Val::Px(36.0), height: Val::Px(36.0),
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            ..default()
                        },
                        BackgroundColor(POWER_INC_COLOR),
                    )).with_children(|btn| {
                        btn.spawn((
                            Text::new("+"),
                            TextFont { font_size: 22.0, ..default() },
                            TextColor(Color::srgb(0.9, 0.9, 1.0)),
                        ));
                    });
                });
            }

            // Overflow allocation controls (hidden in Low complexity — AI manages points 7 & 8).
            panel.spawn((
                HideableElement("power_overflow_controls".into()),
                Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: Val::Px(8.0),
                    padding: UiRect::axes(Val::Px(16.0), Val::Px(4.0)),
                    ..default()
                },
            )).with_children(|overflow_row| {
                overflow_row.spawn((
                    Text::new("Overflow (pts 7-8): Manual"),
                    TextFont { font_size: 13.0, ..default() },
                    TextColor(Color::srgb(0.6, 0.7, 0.5)),
                ));
            });

            // Battery bar section
            panel.spawn((
                Node {
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    row_gap: Val::Px(4.0),
                    width: Val::Percent(80.0),
                    max_width: Val::Px(300.0),
                    ..default()
                },
            )).with_children(|battery_section| {
                // Battery bar background
                battery_section.spawn((
                    Node {
                        width: Val::Percent(100.0),
                        height: Val::Px(16.0),
                        position_type: PositionType::Relative,
                        ..default()
                    },
                    BackgroundColor(POWER_BATTERY_BG),
                )).with_children(|bar_bg| {
                    bar_bg.spawn((
                        BatteryBar,
                        Node {
                            position_type: PositionType::Absolute,
                            left: Val::Px(0.0),
                            top: Val::Px(0.0),
                            width: Val::Percent(0.0),
                            height: Val::Percent(100.0),
                            ..default()
                        },
                        BackgroundColor(POWER_BATTERY_FILL),
                    ));
                });
                // Battery percentage label
                battery_section.spawn((
                    BatteryLabel,
                    Text::new("Battery: 0%"),
                    TextFont { font_size: 14.0, ..default() },
                    TextColor(Color::srgb(0.5, 0.8, 1.0)),
                ));
            });
        });
}

// ── Systems ──────────────────────────────────────────────────────────

fn toggle_power_panel_visibility(
    lobby: Res<LobbyState>,
    token: Res<LocalPlayerToken>,
    active: Res<ActiveConsole>,
    mut panel: Query<&mut Visibility, With<PowerPanel>>,
) {
    if !lobby.is_changed() && !token.is_changed() && !active.is_changed() {
        return;
    }
    let visible = power_panel_visible(&lobby, &token.0, &active);
    for mut vis in panel.iter_mut() {
        *vis = if visible { Visibility::Visible } else { Visibility::Hidden };
    }
}

/// Refresh the power panel: power levels, button enable/disable, battery bar, lock state.
fn refresh_power_panel(
    sim: Res<ClientSimState>,
    ship_view: Res<ShipView>,
    mut row_bg: Query<(&mut BackgroundColor, &PowerRow), (Without<PowerIncButton>, Without<PowerDecButton>)>,
    mut level_labels: Query<(&mut Text, &PowerRowLevel), Without<BatteryLabel>>,
    mut inc_buttons: Query<(&mut BackgroundColor, &PowerIncButton), (Without<PowerRow>, Without<PowerDecButton>)>,
    mut dec_buttons: Query<(&mut BackgroundColor, &PowerDecButton), (Without<PowerRow>, Without<PowerIncButton>)>,
    mut battery_bar: Query<&mut Node, With<BatteryBar>>,
    mut battery_label: Query<&mut Text, (With<BatteryLabel>, Without<PowerRowLevel>)>,
) {
    if !sim.is_changed() && !ship_view.is_changed() {
        return;
    }

    let locked = crate::client_sim::is_power_locked(&sim.power_state_payload);
    let battery_pct = crate::client_sim::battery_percentage(&sim.power_state_payload);

    // Update battery bar width and label
    for mut node in battery_bar.iter_mut() {
        node.width = Val::Percent(battery_pct);
    }
    for mut text in battery_label.iter_mut() {
        **text = format!("Battery: {:.0}%", battery_pct);
    }

    // Update each power row background + level text
    for (mut bg, _row) in row_bg.iter_mut() {
        bg.0 = if locked { POWER_COL_LOCKED } else { POWER_COL_INACTIVE };
    }

    // Update level labels, matching by console
    for (mut text, level_component) in level_labels.iter_mut() {
        let lvl = match level_component.0 {
            Console::Helm => ship_view.power_levels.0,
            Console::Tactical => ship_view.power_levels.1,
            Console::Sensors => ship_view.power_levels.2,
            _ => 0,
        };
        **text = format!("{}", lvl);
    }

    // Update increment buttons by matching their console
    for (mut bg, inc) in inc_buttons.iter_mut() {
        let can_inc = crate::client_sim::can_increase_power(
            &ship_view.power_levels, &inc.0, locked,
        );
        bg.0 = if can_inc { POWER_INC_COLOR } else { POWER_INC_LOCKED };
    }

    // Update decrement buttons by matching their console
    for (mut bg, dec) in dec_buttons.iter_mut() {
        let can_dec = crate::client_sim::can_decrease_power(
            &ship_view.power_levels, &dec.0, locked,
        );
        bg.0 = if can_dec { POWER_DEC_COLOR } else { POWER_DEC_LOCKED };
    }
}

/// Handle increment button presses on the Power console.
fn handle_increase_power(
    interactions: Query<(&Interaction, &PowerIncButton), Changed<Interaction>>,
    sim: Res<ClientSimState>,
    ship_view: Res<ShipView>,
    mut outbound: MessageWriter<OutboundClientMessage>,
) {
    for (interaction, inc) in interactions.iter() {
        if *interaction != Interaction::Pressed {
            continue;
        }
        let locked = crate::client_sim::is_power_locked(&sim.power_state_payload);
        if !crate::client_sim::can_increase_power(&ship_view.power_levels, &inc.0, locked) {
            continue;
        }
        outbound.write(OutboundClientMessage(
            crate::client_sim::increase_power_message(inc.0.clone()),
        ));
    }
}

/// Handle decrement button presses on the Power console.
fn handle_decrease_power(
    interactions: Query<(&Interaction, &PowerDecButton), Changed<Interaction>>,
    sim: Res<ClientSimState>,
    ship_view: Res<ShipView>,
    mut outbound: MessageWriter<OutboundClientMessage>,
) {
    for (interaction, dec) in interactions.iter() {
        if *interaction != Interaction::Pressed {
            continue;
        }
        let locked = crate::client_sim::is_power_locked(&sim.power_state_payload);
        if !crate::client_sim::can_decrease_power(&ship_view.power_levels, &dec.0, locked) {
            continue;
        }
        outbound.write(OutboundClientMessage(
            crate::client_sim::decrease_power_message(dec.0.clone()),
        ));
    }
}

// ── Unit tests ───────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client_lobby::{ActiveConsole, LobbyState};
    use crate::messages::{Console, GamePhase, Player};
    use std::collections::HashMap;

    fn lobby_with_power_player(token: &str, phase: GamePhase) -> LobbyState {
        let mut lobby = LobbyState::default();
        use crate::messages::GameState;
        lobby.apply(&crate::messages::ServerMessage::Welcome {
            state: GameState {
                phase,
                players: vec![Player {
                    token: token.into(),
                    name: "Powertrain".into(),
                    consoles: vec![Console::Power],
                    connected: true,
                }],
                complexity: HashMap::new(),
                world: None,
            },
            ship_stations: crate::stations_config::ShipStations::default(),
        });
        lobby
    }

    // ── power_panel_visible ──────────────────────────────────────────

    #[test]
    fn power_panel_hidden_in_lobby_phase() {
        let lobby = lobby_with_power_player("tok", GamePhase::Lobby);
        let active = ActiveConsole::default();
        assert!(!power_panel_visible(&lobby, "tok", &active));
    }

    #[test]
    fn power_panel_visible_in_progress_holding_power() {
        let lobby = lobby_with_power_player("tok", GamePhase::InProgress);
        let active = ActiveConsole::default();
        assert!(power_panel_visible(&lobby, "tok", &active));
    }

    #[test]
    fn power_panel_hidden_when_player_does_not_hold_power() {
        let lobby = lobby_with_power_player("tok", GamePhase::InProgress);
        let active = ActiveConsole::default();
        assert!(!power_panel_visible(&lobby, "other", &active));
    }

    #[test]
    fn power_panel_visible_when_active_console_is_power_multi_console() {
        let mut lobby = LobbyState::default();
        use crate::messages::GameState;
        lobby.apply(&crate::messages::ServerMessage::Welcome {
            state: GameState {
                phase: GamePhase::InProgress,
                players: vec![Player {
                    token: "tok".into(),
                    name: "Multi".into(),
                    consoles: vec![Console::Power, Console::Tactical],
                    connected: true,
                }],
                complexity: HashMap::new(),
                world: None,
            },
            ship_stations: crate::stations_config::ShipStations::default(),
        });
        let active = ActiveConsole(Some(Console::Power));
        assert!(power_panel_visible(&lobby, "tok", &active));
    }

    #[test]
    fn power_panel_hidden_when_active_console_is_other_multi_console() {
        let mut lobby = LobbyState::default();
        use crate::messages::GameState;
        lobby.apply(&crate::messages::ServerMessage::Welcome {
            state: GameState {
                phase: GamePhase::InProgress,
                players: vec![Player {
                    token: "tok".into(),
                    name: "Multi".into(),
                    consoles: vec![Console::Power, Console::Tactical],
                    connected: true,
                }],
                complexity: HashMap::new(),
                world: None,
            },
            ship_stations: crate::stations_config::ShipStations::default(),
        });
        let active = ActiveConsole(Some(Console::Tactical));
        assert!(!power_panel_visible(&lobby, "tok", &active));
    }

    #[test]
    fn power_panel_hidden_when_no_active_console_and_holding_multiple() {
        let mut lobby = LobbyState::default();
        use crate::messages::GameState;
        lobby.apply(&crate::messages::ServerMessage::Welcome {
            state: GameState {
                phase: GamePhase::InProgress,
                players: vec![Player {
                    token: "tok".into(),
                    name: "Multi".into(),
                    consoles: vec![Console::Power, Console::Tactical],
                    connected: true,
                }],
                complexity: HashMap::new(),
                world: None,
            },
            ship_stations: crate::stations_config::ShipStations::default(),
        });
        let active = ActiveConsole::default(); // None → auto → count != 1
        assert!(!power_panel_visible(&lobby, "tok", &active));
    }
}
