//! Client-side Repair Panel plugin.
//!
//! Owns all Repair console UI: team status rows and console dispatch buttons.
//! Compiled only when the `client` Cargo feature is enabled.

use bevy::prelude::*;

use crate::client_app::OutboundClientMessage;
use crate::client_lobby::{ActiveConsole, LobbyState, LobbyView, LocalPlayerToken};
use crate::client_sim::ClientSimState;
use crate::messages::{ClientMessage, Console, GamePhase, TeamSlot};

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

/// Marks a console dispatch button. Carries the target console and team index.
#[derive(Component)]
struct DispatchButton {
    console: Console,
    team_idx: usize,
}

/// Marks the aggregate hull display text node.
#[derive(Component)]
struct RepairHullText;

/// Marks the container for dynamic team rows.
#[derive(Component)]
struct RepairTeamContainer;

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
            panel.spawn(Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..default()
            }).with_children(|title_row| {
                title_row.spawn((
                    Text::new("Repair Console"),
                    TextFont { font_size: 24.0, ..default() },
                    TextColor(Color::srgb(0.3, 1.0, 0.5)),
                ));
                crate::client_elements::spawn_help_button(title_row, crate::client_elements::HelpPanel::Repair, 16.0);
            });
            crate::client_elements::spawn_help_overlay(panel, crate::client_elements::HelpPanel::Repair);

            // Aggregate hull display
            panel.spawn((
                RepairHullText,
                Text::new("Hull: --/--"),
                TextFont { font_size: 18.0, ..default() },
                TextColor(Color::srgb(0.6, 0.8, 1.0)),
            ));

            // Container for dynamic team rows (populated by refresh system)
            panel.spawn((
                RepairTeamContainer,
                Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(8.0),
                    width: Val::Percent(100.0),
                    ..default()
                },
            ));
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

/// Refresh the repair panel: hull display and dynamic team rows with dispatch buttons.
fn refresh_repair_panel(
    sim: Res<ClientSimState>,
    mut hull_text: Query<&mut Text, With<RepairHullText>>,
    container: Query<Entity, With<RepairTeamContainer>>,
    mut commands: Commands,
    existing_rows: Query<Entity, With<RepairTeamRow>>,
) {
    if !sim.is_changed() {
        return;
    }

    // Update aggregate hull display
    if let Ok(mut text) = hull_text.get_single_mut() {
        let total_current: f32 = sim.console_hull.iter().map(|h| h.current).sum();
        let total_max: f32 = sim.console_hull.iter().map(|h| h.max_hp).sum();
        **text = format!("Hull: {:.0}/{}", total_current, total_max as u32);
    }

    // Despawn all existing team rows (handles count changes cleanly)
    for entity in existing_rows.iter() {
        commands.entity(entity).despawn_recursive();
    }

    // Respawn team rows from current repair teams
    let Ok(container_entity) = container.get_single() else {
        return;
    };

    // Pre-compute team row data to avoid borrowing sim inside with_children
    struct RowData {
        pct: f32,
        bar_color: Color,
        status: String,
        status_color: Color,
        is_idle: bool,
    }

    let rows: Vec<RowData> = sim.repair_teams.iter().map(|slot| {
        match slot {
            TeamSlot::Idle => RowData {
                pct: 0.0,
                bar_color: Color::srgb(0.10, 0.20, 0.60),
                status: "Idle".into(),
                status_color: Color::srgb(0.5, 0.7, 1.0),
                is_idle: true,
            },
            TeamSlot::Travelling { console, elapsed } => RowData {
                pct: (elapsed / 5.0 * 100.0).clamp(0.0, 100.0),
                bar_color: Color::srgb(0.60, 0.60, 0.10),
                status: format!("→ {}", console.display_name()),
                status_color: Color::srgb(1.0, 0.9, 0.3),
                is_idle: false,
            },
            TeamSlot::Repairing { console, elapsed } => RowData {
                pct: (elapsed / 50.0 * 100.0).clamp(0.0, 100.0),
                bar_color: Color::srgb(0.10, 0.70, 0.20),
                status: format!("Repairing {}", console.display_name()),
                status_color: Color::srgb(0.3, 1.0, 0.3),
                is_idle: false,
            },
            TeamSlot::Returning { elapsed } => RowData {
                pct: (elapsed / 5.0 * 100.0).clamp(0.0, 100.0),
                bar_color: Color::srgb(0.30, 0.30, 0.80),
                status: "Returning".into(),
                status_color: Color::srgb(0.5, 0.5, 1.0),
                is_idle: false,
            },
        }
    }).collect();

    let btn_bg = Color::srgb(0.10, 0.25, 0.15);
    let btn_disabled_bg = Color::srgb(0.05, 0.12, 0.08);
    let btn_text = Color::srgb(0.5, 1.0, 0.7);
    let btn_text_disabled = Color::srgb(0.3, 0.5, 0.3);

    commands.entity(container_entity).with_children(|parent| {
        for (i, row) in rows.iter().enumerate() {
            parent.spawn((
                RepairTeamRow(i),
                Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    width: Val::Percent(100.0),
                    height: Val::Px(64.0),
                    column_gap: Val::Px(8.0),
                    padding: UiRect::all(Val::Px(4.0)),
                    ..default()
                },
                BackgroundColor(Color::srgb(0.05, 0.10, 0.20)),
            )).with_children(|row| {
                // Progress bar
                row.spawn((
                    Node {
                        width: Val::Percent(30.0),
                        height: Val::Percent(100.0),
                        position_type: PositionType::Relative,
                        ..default()
                    },
                    BackgroundColor(Color::srgb(0.02, 0.05, 0.10)),
                )).with_children(|bar_container| {
                    bar_container.spawn((
                        RepairTeamFill(i),
                        Node {
                            position_type: PositionType::Absolute,
                            left: Val::Px(0.0),
                            top: Val::Px(0.0),
                            width: Val::Percent(row.pct),
                            height: Val::Percent(100.0),
                            ..default()
                        },
                        BackgroundColor(row.bar_color),
                    ));
                });

                // Status label
                row.spawn((
                    RepairTeamStatusText(i),
                    Text::new(row.status.as_str()),
                    TextFont { font_size: 14.0, ..default() },
                    TextColor(row.status_color),
                    Node {
                        width: Val::Percent(18.0),
                        ..default()
                    },
                ));

                // Four targeting buttons (disabled when team is not Idle)
                for (console, label) in [
                    (Console::Helm, "HELM"),
                    (Console::Tactical, "TACTICAL"),
                    (Console::Power, "POWER"),
                    (Console::Shields, "SHIELDS"),
                ] {
                    let bg = if row.is_idle { btn_bg } else { btn_disabled_bg };
                    let fg = if row.is_idle { btn_text } else { btn_text_disabled };
                    row.spawn((
                        DispatchButton { console, team_idx: i },
                        Button,
                        Node {
                            padding: UiRect::axes(Val::Px(8.0), Val::Px(6.0)),
                            ..default()
                        },
                        BackgroundColor(bg),
                    )).with_children(|btn| {
                        btn.spawn((
                            Text::new(label),
                            TextFont { font_size: 12.0, ..default() },
                            TextColor(fg),
                        ));
                    });
                }
            });
        }
    });
}

/// Handle console dispatch button presses.
/// Only dispatches when the associated team is Idle.
fn handle_dispatch_button_press(
    interactions: Query<(&Interaction, &DispatchButton), Changed<Interaction>>,
    sim: Res<ClientSimState>,
    mut outbound: MessageWriter<OutboundClientMessage>,
) {
    for (interaction, btn) in interactions.iter() {
        if *interaction != Interaction::Pressed {
            continue;
        }
        // Only dispatch if the team is idle
        if !sim.repair_teams.get(btn.team_idx).map_or(false, |t| matches!(t, TeamSlot::Idle)) {
            continue;
        }
        outbound.write(OutboundClientMessage(ClientMessage::DispatchRepairTeam { console: btn.console.clone() }));
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
