//! Client-side Repair Panel plugin.
//!
//! Owns all Repair console UI: team status rows and console dispatch buttons.
//! Compiled only when the `client` Cargo feature is enabled.

use bevy::prelude::*;

use crate::client_app::OutboundClientMessage;
use crate::client_lobby::{ActiveConsole, LobbyState, LobbyView, LocalPlayerToken};
use crate::client_sim::ClientSimState;
use crate::messages::{Console, GamePhase, TeamSlot};

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

// ── Pure row-data derivation ─────────────────────────────────────────

/// Data derived from a single [`TeamSlot`] that drives one team-row in the UI.
///
/// Extracted as a pure function so it can be unit-tested without Bevy.
pub struct RowData {
    /// Fill percentage for the progress bar (0–100).
    pub pct: f32,
    /// Whether the bar fills (true) or drains (false).
    ///
    /// - Travelling / Repairing → fills.
    /// - Returning → drains.
    pub fills: bool,
    /// Human-readable status label.
    pub status: String,
    /// Whether this team slot is `Idle`.
    pub is_idle: bool,
    /// Console the team is currently heading toward or working at (used to
    /// highlight the matching dispatch button).
    pub active_console: Option<Console>,
}

const TRAVEL_DURATION_SECS: f32 = 5.0;
const REPAIR_DURATION_SECS: f32 = 50.0;

/// Derive [`RowData`] from a single `TeamSlot`.
pub fn row_data_for_slot(slot: &TeamSlot) -> RowData {
    match slot {
        TeamSlot::Idle => RowData {
            pct: 0.0,
            fills: true,
            status: "Idle".into(),
            is_idle: true,
            active_console: None,
        },
        TeamSlot::Travelling { console, elapsed } => RowData {
            pct: (elapsed / TRAVEL_DURATION_SECS * 100.0).clamp(0.0, 100.0),
            fills: true,
            status: format!("→ {}", console.display_name()),
            is_idle: false,
            active_console: Some(console.clone()),
        },
        TeamSlot::Repairing { console, elapsed } => RowData {
            pct: (elapsed / REPAIR_DURATION_SECS * 100.0).clamp(0.0, 100.0),
            fills: true,
            status: format!("Repairing {}", console.display_name()),
            is_idle: false,
            active_console: Some(console.clone()),
        },
        TeamSlot::Returning { remaining, queued } => RowData {
            pct: (remaining / TRAVEL_DURATION_SECS * 100.0).clamp(0.0, 100.0),
            fills: false,
            status: if let Some(c) = queued {
                format!("Returning → {}", c.display_name())
            } else {
                "Returning".into()
            },
            is_idle: false,
            active_console: queued.clone(),
        },
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
#[allow(dead_code)]
struct RepairTeamFill(usize);

/// Marks the status text overlaid on a team row.
#[derive(Component)]
#[allow(dead_code)]
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
    if let Ok(mut text) = hull_text.single_mut() {
        let total_current: f32 = sim.console_hull.iter().map(|h| h.current).sum();
        let total_max: f32 = sim.console_hull.iter().map(|h| h.max_hp).sum();
        **text = format!("Hull: {:.0}/{}", total_current, total_max as u32);
    }

    // Despawn all existing team rows (handles count changes cleanly)
    for entity in existing_rows.iter() {
        commands.entity(entity).despawn();
    }

    // Respawn team rows from current repair teams
    let Ok(container_entity) = container.single() else {
        return;
    };

    // Pre-compute team row data via the pure `row_data_for_slot` function.
    struct RenderRow {
        data: RowData,
        bar_color: Color,
        status_color: Color,
    }

    let rows: Vec<RenderRow> = sim.repair_teams.iter().map(|slot| {
        let data = row_data_for_slot(slot);
        let (bar_color, status_color) = if data.is_idle {
            (Color::srgb(0.10, 0.20, 0.60), Color::srgb(0.5, 0.7, 1.0))
        } else if data.fills {
            // Travelling or Repairing — use warm colours
            match slot {
                TeamSlot::Repairing { .. } => (Color::srgb(0.10, 0.70, 0.20), Color::srgb(0.3, 1.0, 0.3)),
                _ => (Color::srgb(0.60, 0.60, 0.10), Color::srgb(1.0, 0.9, 0.3)),
            }
        } else {
            // Returning
            (Color::srgb(0.30, 0.30, 0.80), Color::srgb(0.5, 0.5, 1.0))
        };
        RenderRow { data, bar_color, status_color }
    }).collect();

    let btn_bg = Color::srgb(0.10, 0.25, 0.15);
    let btn_active_bg = Color::srgb(0.20, 0.55, 0.30);
    let btn_text = Color::srgb(0.5, 1.0, 0.7);
    let btn_text_active = Color::srgb(1.0, 1.0, 1.0);

    commands.entity(container_entity).with_children(|parent| {
        for (i, row) in rows.iter().enumerate() {
            parent.spawn((
                RepairTeamRow(i),
                Node {
                    flex_direction: FlexDirection::Column,
                    width: Val::Percent(100.0),
                    padding: UiRect::all(Val::Px(4.0)),
                    row_gap: Val::Px(4.0),
                    ..default()
                },
                BackgroundColor(Color::srgb(0.05, 0.10, 0.20)),
            )).with_children(|rc| {
                // Top row: progress bar + status label
                rc.spawn(Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    width: Val::Percent(100.0),
                    height: Val::Px(32.0),
                    column_gap: Val::Px(8.0),
                    ..default()
                }).with_children(|top| {
                    // Progress bar
                    top.spawn((
                        Node {
                            width: Val::Percent(50.0),
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
                                width: Val::Percent(row.data.pct),
                                height: Val::Percent(100.0),
                                ..default()
                            },
                            BackgroundColor(row.bar_color),
                        ));
                    });

                    // Status label
                    top.spawn((
                        RepairTeamStatusText(i),
                        Text::new(row.data.status.as_str()),
                        TextFont { font_size: 14.0, ..default() },
                        TextColor(row.status_color),
                    ));
                });

                // Bottom row: dispatch buttons
                rc.spawn(Node {
                    flex_direction: FlexDirection::Row,
                    column_gap: Val::Px(6.0),
                    ..default()
                }).with_children(|bottom| {
                    for (console, label) in [
                        (Console::Helm, "HELM"),
                        (Console::Tactical, "TACTICAL"),
                        (Console::Power, "POWER"),
                        (Console::Shields, "SHIELDS"),
                    ] {
                        let is_active = row.data.active_console.as_ref() == Some(&console);
                        let bg = if is_active { btn_active_bg } else { btn_bg };
                        let fg = if is_active { btn_text_active } else { btn_text };
                        bottom.spawn((
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
            });
        }
    });
}

/// Handle console dispatch button presses.
/// Sends `DispatchRepairTeam` for any team state — the server handles redirect/recall logic.
fn handle_dispatch_button_press(
    interactions: Query<(&Interaction, &DispatchButton), Changed<Interaction>>,
    mut outbound: MessageWriter<OutboundClientMessage>,
) {
    for (interaction, btn) in interactions.iter() {
        if *interaction != Interaction::Pressed {
            continue;
        }
        outbound.write(OutboundClientMessage(crate::client_sim::dispatch_repair_team_message(btn.team_idx as u8, btn.console.clone())));
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

    // ── row_data_for_slot ────────────────────────────────────────────

    #[test]
    fn idle_slot_has_zero_pct_and_no_active_console() {
        let d = row_data_for_slot(&TeamSlot::Idle);
        assert_eq!(d.pct, 0.0);
        assert!(d.is_idle);
        assert!(d.active_console.is_none());
        assert!(d.fills);
        assert_eq!(d.status, "Idle");
    }

    #[test]
    fn travelling_fills_bar_and_reports_active_console() {
        let d = row_data_for_slot(&TeamSlot::Travelling {
            console: Console::Helm,
            elapsed: 2.5,
        });
        // 2.5 / 5.0 * 100 = 50 %
        assert!((d.pct - 50.0).abs() < 0.01, "pct should be 50, got {}", d.pct);
        assert!(d.fills, "Travelling should fill the bar");
        assert!(!d.is_idle);
        assert_eq!(d.active_console, Some(Console::Helm));
        assert!(d.status.contains("Helm"), "status should mention Helm, got '{}'", d.status);
    }

    #[test]
    fn travelling_pct_clamped_to_100() {
        let d = row_data_for_slot(&TeamSlot::Travelling {
            console: Console::Tactical,
            elapsed: 999.0,
        });
        assert_eq!(d.pct, 100.0);
    }

    #[test]
    fn repairing_fills_bar_and_reports_active_console() {
        let d = row_data_for_slot(&TeamSlot::Repairing {
            console: Console::Tactical,
            elapsed: 25.0,
        });
        // 25 / 50 * 100 = 50 %
        assert!((d.pct - 50.0).abs() < 0.01, "pct should be 50, got {}", d.pct);
        assert!(d.fills, "Repairing should fill the bar");
        assert!(!d.is_idle);
        assert_eq!(d.active_console, Some(Console::Tactical));
        assert!(d.status.contains("Tactical"), "status should mention Tactical");
    }

    #[test]
    fn returning_drains_bar_and_no_queue_means_no_active_console() {
        let d = row_data_for_slot(&TeamSlot::Returning {
            remaining: 2.5,
            queued: None,
        });
        // 2.5 / 5.0 * 100 = 50 % — but bar drains, so pct represents how much remains
        assert!((d.pct - 50.0).abs() < 0.01, "pct should be 50, got {}", d.pct);
        assert!(!d.fills, "Returning should drain the bar");
        assert!(!d.is_idle);
        assert!(d.active_console.is_none());
        assert_eq!(d.status, "Returning");
    }

    #[test]
    fn returning_with_queue_shows_destination_and_highlights_queued_console() {
        let d = row_data_for_slot(&TeamSlot::Returning {
            remaining: 1.0,
            queued: Some(Console::Power),
        });
        assert!(!d.fills);
        assert_eq!(d.active_console, Some(Console::Power));
        assert!(d.status.contains("Power"), "status should show queued destination");
    }

    #[test]
    fn returning_pct_clamped_at_zero_when_remaining_is_zero() {
        let d = row_data_for_slot(&TeamSlot::Returning {
            remaining: 0.0,
            queued: None,
        });
        assert_eq!(d.pct, 0.0);
    }

    // ── dispatch_repair_team_message builder ─────────────────────────

    #[test]
    fn dispatch_repair_team_message_encodes_team_and_console() {
        let msg = crate::client_sim::dispatch_repair_team_message(1, Console::Shields);
        assert!(
            matches!(msg, ClientMessage::DispatchRepairTeam { team_idx: 1, console: Console::Shields }),
            "unexpected message: {:?}", msg
        );
    }

    #[test]
    fn dispatch_repair_team_message_team_zero_helm() {
        let msg = crate::client_sim::dispatch_repair_team_message(0, Console::Helm);
        assert!(
            matches!(msg, ClientMessage::DispatchRepairTeam { team_idx: 0, console: Console::Helm }),
        );
    }
}
