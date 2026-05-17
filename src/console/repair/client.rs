//! Client-side Repair Panel plugin.
//!
//! Owns all Repair console UI: team status rows and console dispatch buttons.
//! Compiled only when the `client` Cargo feature is enabled.

use bevy::prelude::*;

use crate::client_app::OutboundClientMessage;
use crate::client_lobby::{ActiveConsole, LobbyState, LobbyView, LocalPlayerToken};
use crate::client_sim::ClientSimState;
use crate::messages::{ClientMessage, Console, GamePhase};

// ── Pure visibility helper ────────────────────────────────────────────

/// Decide whether the repair panel should be visible.
pub fn repair_panel_visible(
    lobby: &LobbyState,
    token: &str,
    active: &ActiveConsole,
) -> bool {
    if lobby.phase != GamePhase::InProgress {
        return false;
    }
    let view = LobbyView::new(lobby, token);
    let consoles = view.my_consoles();
    if !consoles.contains(&Console::Repair) {
        return false;
    }
    let count = consoles.len();
    match &active.0 {
        Some(c) => *c == Console::Repair,
        None => count == 1,
    }
}

// ── Marker components ────────────────────────────────────────────────

/// Marks the root of the repair console UI.
#[derive(Component)]
pub struct RepairPanel;

/// Marks a team row container (index 0, 1, or 2).
#[derive(Component)]
#[allow(dead_code)]
struct RepairTeamRow(usize);

/// Marks the progress-bar fill inside a team row.
#[derive(Component)]
struct RepairTeamFill(usize);

/// Marks the status text overlaid on a team row.
#[derive(Component)]
struct RepairTeamStatusText(usize);

/// Marks a console dispatch button. Carries the target console.
#[derive(Component)]
struct DispatchButton(Console);

// ── Plugin ───────────────────────────────────────────────────────────

/// Plugin that owns all Repair console UI and systems.
pub struct RepairPanelPlugin;

impl Plugin for RepairPanelPlugin {
    fn build(&self, app: &mut App) {
        app
            .add_systems(Startup, setup_repair_ui)
            .add_systems(Update, (
                toggle_repair_panel_visibility,
                refresh_repair_panel,
                handle_dispatch_button_press,
            ));
    }
}

// ── Setup ────────────────────────────────────────────────────────────

fn setup_repair_ui(mut commands: Commands) {
    commands
        .spawn((
            RepairPanel,
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
                Text::new("Repair Console"),
                TextFont { font_size: 24.0, ..default() },
                TextColor(Color::srgb(0.3, 1.0, 0.5)),
            ));

            // Console dispatch buttons
            panel.spawn((
                Node {
                    flex_direction: FlexDirection::Row,
                    column_gap: Val::Px(8.0),
                    ..default()
                },
            )).with_children(|row| {
                for (console, label) in [
                    (Console::Helm, "HELM"),
                    (Console::Tactical, "TACTICAL"),
                    (Console::Power, "POWER"),
                    (Console::Shields, "SHIELDS"),
                ] {
                    row.spawn((
                        DispatchButton(console),
                        Button,
                        Node {
                            padding: UiRect::axes(Val::Px(16.0), Val::Px(12.0)),
                            ..default()
                        },
                        BackgroundColor(Color::srgb(0.10, 0.25, 0.15)),
                    )).with_children(|btn| {
                        btn.spawn((
                            Text::new(label),
                            TextFont { font_size: 16.0, ..default() },
                            TextColor(Color::srgb(0.5, 1.0, 0.7)),
                        ));
                    });
                }
            });

            // Team rows
            for i in 0..3 {
                panel.spawn((
                    RepairTeamRow(i),
                    Node {
                        flex_direction: FlexDirection::Row,
                        align_items: AlignItems::Center,
                        width: Val::Percent(80.0),
                        height: Val::Px(36.0),
                        position_type: PositionType::Relative,
                        ..default()
                    },
                    BackgroundColor(Color::srgb(0.05, 0.10, 0.20)),
                )).with_children(|row| {
                    row.spawn((
                        RepairTeamFill(i),
                        Node {
                            position_type: PositionType::Absolute,
                            left: Val::Px(0.0),
                            top: Val::Px(0.0),
                            width: Val::Percent(0.0),
                            height: Val::Percent(100.0),
                            ..default()
                        },
                        BackgroundColor(Color::srgb(0.10, 0.20, 0.60)),
                    ));
                    row.spawn((
                        RepairTeamStatusText(i),
                        Text::new("Idle"),
                        TextFont { font_size: 14.0, ..default() },
                        TextColor(Color::srgb(0.6, 0.8, 1.0)),
                        Node {
                            position_type: PositionType::Absolute,
                            left: Val::Px(12.0),
                            top: Val::Px(0.0),
                            bottom: Val::Px(0.0),
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            ..default()
                        },
                    ));
                });
            }
        });
}

// ── Systems ──────────────────────────────────────────────────────────

fn toggle_repair_panel_visibility(
    lobby: Res<LobbyState>,
    token: Res<LocalPlayerToken>,
    active: Res<ActiveConsole>,
    mut panel: Query<&mut Visibility, With<RepairPanel>>,
) {
    let visible = repair_panel_visible(&lobby, &token.0, &active);
    for mut vis in panel.iter_mut() {
        *vis = if visible { Visibility::Visible } else { Visibility::Hidden };
    }
}

/// Refresh the repair panel: team status bars and text.
fn refresh_repair_panel(
    sim: Res<ClientSimState>,
    mut team_fill: Query<(&mut Node, &mut BackgroundColor, &RepairTeamFill), Without<DispatchButton>>,
    mut team_status: Query<(&mut Text, &mut TextColor, &RepairTeamStatusText)>,
) {
    if !sim.is_changed() {
        return;
    }

    // Determine if all teams are busy
    let all_busy = sim.repair_teams.iter().all(|t| !matches!(t, crate::messages::TeamSlot::Idle));
    let _ = all_busy; // Used for button dimming if needed in future

    // Update team progress bars and status text
    for (mut node, mut fill_bg, fill) in team_fill.iter_mut() {
        let idx = fill.0;
        let slot = sim.repair_teams.get(idx);
        let (pct, color) = match slot {
            None | Some(crate::messages::TeamSlot::Idle) => {
                (0.0, Color::srgb(0.10, 0.20, 0.60))
            }
            Some(crate::messages::TeamSlot::Travelling { elapsed, .. }) => {
                let pct = (elapsed / 5.0 * 100.0).clamp(0.0, 100.0);
                (pct, Color::srgb(0.60, 0.60, 0.10))
            }
            Some(crate::messages::TeamSlot::Repairing { elapsed, .. }) => {
                // Show elapsed time as a rough progress indicator (max 50s for full console)
                let pct = (elapsed / 50.0 * 100.0).clamp(0.0, 100.0);
                (pct, Color::srgb(0.10, 0.70, 0.20))
            }
            Some(crate::messages::TeamSlot::Returning { elapsed }) => {
                let pct = (elapsed / 5.0 * 100.0).clamp(0.0, 100.0);
                (pct, Color::srgb(0.30, 0.30, 0.80))
            }
        };
        node.width = Val::Percent(pct);
        fill_bg.0 = color;
    }

    for (mut text, mut color, status) in team_status.iter_mut() {
        let idx = status.0;
        let slot = sim.repair_teams.get(idx);
        match slot {
            None | Some(crate::messages::TeamSlot::Idle) => {
                **text = "Idle".to_string();
                *color = TextColor(Color::srgb(0.5, 0.7, 1.0));
            }
            Some(crate::messages::TeamSlot::Travelling { console, .. }) => {
                **text = format!("→ {}", console.display_name());
                *color = TextColor(Color::srgb(1.0, 0.9, 0.3));
            }
            Some(crate::messages::TeamSlot::Repairing { console, .. }) => {
                **text = format!("Repairing {}", console.display_name());
                *color = TextColor(Color::srgb(0.3, 1.0, 0.3));
            }
            Some(crate::messages::TeamSlot::Returning { .. }) => {
                **text = "Returning".to_string();
                *color = TextColor(Color::srgb(0.5, 0.5, 1.0));
            }
        }
    }
}

/// Handle console dispatch button presses.
fn handle_dispatch_button_press(
    interactions: Query<(&Interaction, &DispatchButton), Changed<Interaction>>,
    mut outbound: MessageWriter<OutboundClientMessage>,
) {
    for (interaction, btn) in interactions.iter() {
        if *interaction != Interaction::Pressed {
            continue;
        }
        outbound.write(OutboundClientMessage(ClientMessage::DispatchRepairTeam { console: btn.0.clone() }));
    }
}

// ── Unit tests ───────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client_lobby::{ActiveConsole, LobbyState};
    use crate::messages::{Console, GamePhase, Player};
    use std::collections::HashMap;

    fn lobby_with_repair_player(token: &str, phase: GamePhase) -> LobbyState {
        let mut lobby = LobbyState::default();
        use crate::messages::{GameState, WorldData};
        lobby.apply(&crate::messages::ServerMessage::Welcome {
            state: GameState {
                phase,
                players: vec![Player {
                    token: token.into(),
                    name: "Repairer".into(),
                    consoles: vec![Console::Repair],
                    connected: true,
                }],
                complexity: HashMap::new(),
                world: None,
            },
            ship_stations: crate::stations_config::ShipStations::default(),
        });
        lobby
    }

    // ── repair_panel_visible ─────────────────────────────────────────

    #[test]
    fn repair_panel_hidden_in_lobby_phase() {
        let lobby = lobby_with_repair_player("tok", GamePhase::Lobby);
        let active = ActiveConsole::default();
        assert!(!repair_panel_visible(&lobby, "tok", &active));
    }

    #[test]
    fn repair_panel_visible_in_progress_holding_repair() {
        let lobby = lobby_with_repair_player("tok", GamePhase::InProgress);
        let active = ActiveConsole::default();
        assert!(repair_panel_visible(&lobby, "tok", &active));
    }

    #[test]
    fn repair_panel_hidden_when_player_does_not_hold_repair() {
        let lobby = lobby_with_repair_player("tok", GamePhase::InProgress);
        let active = ActiveConsole::default();
        assert!(!repair_panel_visible(&lobby, "other", &active));
    }

    #[test]
    fn repair_panel_visible_when_active_console_is_repair_multi_console() {
        let mut lobby = LobbyState::default();
        use crate::messages::{GameState, WorldData};
        lobby.apply(&crate::messages::ServerMessage::Welcome {
            state: GameState {
                phase: GamePhase::InProgress,
                players: vec![Player {
                    token: "tok".into(),
                    name: "Multi".into(),
                    consoles: vec![Console::Repair, Console::Tactical],
                    connected: true,
                }],
                complexity: HashMap::new(),
                world: None,
            },
            ship_stations: crate::stations_config::ShipStations::default(),
        });
        let active = ActiveConsole(Some(Console::Repair));
        assert!(repair_panel_visible(&lobby, "tok", &active));
    }

    #[test]
    fn repair_panel_hidden_when_active_console_is_other_multi_console() {
        let mut lobby = LobbyState::default();
        use crate::messages::GameState;
        lobby.apply(&crate::messages::ServerMessage::Welcome {
            state: GameState {
                phase: GamePhase::InProgress,
                players: vec![Player {
                    token: "tok".into(),
                    name: "Multi".into(),
                    consoles: vec![Console::Repair, Console::Tactical],
                    connected: true,
                }],
                complexity: HashMap::new(),
                world: None,
            },
            ship_stations: crate::stations_config::ShipStations::default(),
        });
        let active = ActiveConsole(Some(Console::Tactical));
        assert!(!repair_panel_visible(&lobby, "tok", &active));
    }

    #[test]
    fn repair_panel_hidden_when_no_active_console_and_holding_multiple() {
        let mut lobby = LobbyState::default();
        use crate::messages::GameState;
        lobby.apply(&crate::messages::ServerMessage::Welcome {
            state: GameState {
                phase: GamePhase::InProgress,
                players: vec![Player {
                    token: "tok".into(),
                    name: "Multi".into(),
                    consoles: vec![Console::Repair, Console::Tactical],
                    connected: true,
                }],
                complexity: HashMap::new(),
                world: None,
            },
            ship_stations: crate::stations_config::ShipStations::default(),
        });
        let active = ActiveConsole::default(); // None → auto → count != 1
        assert!(!repair_panel_visible(&lobby, "tok", &active));
    }
}
