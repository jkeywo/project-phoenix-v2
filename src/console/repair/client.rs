//! Client-side Repair Panel plugin — migrated to `src/gui/` library widgets.
//!
//! Owns all Repair console UI: team status rows and console dispatch buttons.
//! Uses `GuiButton` for dispatch buttons (observer callbacks), `ProgressBar`
//! (continuous) with `ProgressValue` for team progress, and `TextReadout` with
//! `ReadoutValue` for team status and hull status text.
//!
//! No per-button marker-component query systems remain.
//! Compiled only when the `client` Cargo feature is enabled.

use bevy::prelude::*;

use crate::client::console_shell::ConsoleShell;
use crate::client_app::{ClientSet, OutboundClientMessage};
use crate::client_lobby::{ActiveConsole, LobbyState, LobbyView, LocalPlayerToken};
use crate::client_sim::ClientSimState;
use crate::gui::{
    spawn_gui_button, ButtonPressed, ButtonSize, ProgressBar, ProgressBarVariant, ProgressValue,
    ReadoutValue, StateVisuals, TextReadout, WidgetState,
};
use crate::messages::{Console, GamePhase, TeamSlot};
use crate::phone_border::framing::{DeviceOrientation, PhoneAssets};

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

/// Derive [`RowData`] from a single `TeamSlot` using server-broadcast timings.
///
/// `travel_duration_secs` and `repair_duration_secs` are sourced from
/// `LobbyState.ship_config` (which carries the `[repair]` block from the
/// ship TOML). `repair_duration_secs` is the time to fully repair the
/// target console at the broadcast rate (i.e. `console_max_hp / rate`).
///
/// Callers that don't have the server timings yet (very early frames, or
/// pure tests) can use [`row_data_for_slot`] which falls back to the
/// historical baseline (5s travel / 50s repair for a 25-HP console).
pub fn row_data_for_slot_with_timings(
    slot: &TeamSlot,
    travel_duration_secs: f32,
    repair_duration_secs: f32,
) -> RowData {
    // Guard against divide-by-zero / negative timings from a misconfigured
    // ship TOML — fall back to the baseline so the UI never produces NaN.
    let travel = if travel_duration_secs > 0.0 { travel_duration_secs } else { TRAVEL_DURATION_SECS };
    // NOTE: `repair_duration_secs` is accepted for symmetry with `travel`, but the
    // `Repairing` branch currently shows a flat 100% bar (progress is implied by
    // the per-team status broadcast). Kept as a parameter so callers don't have
    // to change when we surface per-tick repair progress.
    let _ = repair_duration_secs;
    match slot {
        TeamSlot::Idle => RowData {
            pct: 0.0,
            fills: true,
            status: "Idle".into(),
            is_idle: true,
            active_console: None,
        },
        TeamSlot::Travelling { console, elapsed } => RowData {
            pct: (elapsed / travel * 100.0).clamp(0.0, 100.0),
            fills: true,
            status: format!("→ {}", console.display_name()),
            is_idle: false,
            active_console: Some(console.clone()),
        },
        TeamSlot::Repairing { console } => RowData {
            pct: 100.0,
            fills: true,
            status: format!("Repairing {}", console.display_name()),
            is_idle: false,
            active_console: Some(console.clone()),
        },
        TeamSlot::Returning { remaining, queued } => RowData {
            pct: (remaining / travel * 100.0).clamp(0.0, 100.0),
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

/// Back-compat wrapper that uses the historical baseline timings
/// (5 s travel, 50 s repair for a 25-HP console). Prefer
/// [`row_data_for_slot_with_timings`] in live code paths so the panel
/// honours the `[repair]` block in the ship TOML.
pub fn row_data_for_slot(slot: &TeamSlot) -> RowData {
    row_data_for_slot_with_timings(slot, TRAVEL_DURATION_SECS, REPAIR_DURATION_SECS)
}

// ── Pure visual helpers ──────────────────────────────────────────────

/// `StateVisuals` for a team progress bar in idle state.
pub fn team_bar_idle_visuals() -> StateVisuals {
    StateVisuals::from_colors(
        Color::srgb(0.10, 0.20, 0.60), // idle
        Color::srgb(0.10, 0.20, 0.60), // hover (non-interactive)
        Color::srgb(0.10, 0.20, 0.60), // active
        Color::srgb(0.10, 0.20, 0.60), // press
        Color::srgb(0.05, 0.10, 0.20), // disabled
    )
}

/// `StateVisuals` for a team progress bar in travelling state.
pub fn team_bar_travelling_visuals() -> StateVisuals {
    StateVisuals::from_colors(
        Color::srgb(0.60, 0.60, 0.10), // idle
        Color::srgb(0.60, 0.60, 0.10), // hover
        Color::srgb(0.70, 0.70, 0.12), // active
        Color::srgb(0.60, 0.60, 0.10), // press
        Color::srgb(0.05, 0.10, 0.20), // disabled
    )
}

/// `StateVisuals` for a team progress bar in repairing state.
pub fn team_bar_repairing_visuals() -> StateVisuals {
    StateVisuals::from_colors(
        Color::srgb(0.10, 0.70, 0.20), // idle
        Color::srgb(0.10, 0.70, 0.20), // hover
        Color::srgb(0.12, 0.80, 0.22), // active
        Color::srgb(0.10, 0.70, 0.20), // press
        Color::srgb(0.05, 0.10, 0.20), // disabled
    )
}

/// `StateVisuals` for a team progress bar in returning state.
pub fn team_bar_returning_visuals() -> StateVisuals {
    StateVisuals::from_colors(
        Color::srgb(0.30, 0.30, 0.80), // idle
        Color::srgb(0.30, 0.30, 0.80), // hover
        Color::srgb(0.35, 0.35, 0.90), // active
        Color::srgb(0.30, 0.30, 0.80), // press
        Color::srgb(0.05, 0.10, 0.20), // disabled
    )
}

/// `StateVisuals` for a team status `TextReadout`.
pub fn team_status_readout_visuals() -> StateVisuals {
    StateVisuals::from_colors(
        Color::srgb(0.5, 0.7, 1.0),  // idle
        Color::srgb(0.5, 0.7, 1.0),  // hover
        Color::srgb(0.8, 0.9, 1.0),  // active
        Color::srgb(0.5, 0.7, 1.0),  // press
        Color::srgb(0.3, 0.3, 0.4),  // disabled
    )
}

/// `StateVisuals` for the hull `TextReadout`.
pub fn hull_readout_visuals() -> StateVisuals {
    StateVisuals::from_colors(
        Color::srgb(0.6, 0.8, 1.0), // idle
        Color::srgb(0.6, 0.8, 1.0), // hover
        Color::srgb(0.6, 0.8, 1.0), // active
        Color::srgb(0.6, 0.8, 1.0), // press
        Color::srgb(0.3, 0.3, 0.4), // disabled
    )
}

/// `StateVisuals` for a dispatch button (idle = inactive, active = console highlighted).
pub fn dispatch_button_visuals() -> StateVisuals {
    StateVisuals::from_colors(
        Color::srgb(0.10, 0.25, 0.15), // idle
        Color::srgb(0.15, 0.35, 0.20), // hover
        Color::srgb(0.20, 0.55, 0.30), // active (team heading here)
        Color::srgb(0.25, 0.45, 0.25), // press
        Color::srgb(0.05, 0.10, 0.08), // disabled
    )
}

// ── Marker components ────────────────────────────────────────────────

/// Marks the root of the repair console UI.
#[derive(Component)]
pub struct RepairPanel;

/// Marks a `ProgressBar` root for a specific team row (index 0, 1, or 2).
#[derive(Component, Clone)]
struct RepairTeamBar(usize);

/// Marks a `TextReadout` root for the team status (index 0, 1, or 2).
#[derive(Component, Clone)]
struct RepairTeamStatusReadout(usize);

/// Marks a dispatch `GuiButton`. Carries the target console and team index.
#[derive(Component, Clone)]
struct DispatchButton {
    console: Console,
    team_idx: usize,
}

/// Marks the hull `TextReadout` root.
#[derive(Component)]
struct RepairHullReadout;

// ── Plugin ───────────────────────────────────────────────────────────

/// Marker resource set once the repair UI has been spawned.
/// Carries the team count used at spawn time so we can detect changes.
#[derive(Resource)]
pub struct RepairPanelSpawned {
    team_count: usize,
}

// ── Plugin ───────────────────────────────────────────────────────────

/// Plugin that owns all Repair console UI and systems.
pub struct RepairPanelPlugin;

impl Plugin for RepairPanelPlugin {
    fn build(&self, app: &mut App) {
        app
            .add_systems(Update, (
                spawn_repair_ui.run_if(not(resource_exists::<RepairPanelSpawned>)),
                toggle_repair_panel_visibility.in_set(ClientSet::ConsoleUpdate),
                refresh_repair_panel,
                respawn_repair_on_orientation_change,
                respawn_repair_on_team_count_change,
            ));
    }
}

// ── Spawn (ConsoleShell) ─────────────────────────────────────────────

fn spawn_repair_ui(
    mut commands: Commands,
    assets: Option<Res<PhoneAssets>>,
    old_panel: Query<Entity, With<RepairPanel>>,
    old_help: Query<(Entity, &crate::client::elements::HelpOverlay)>,
    orientation: Option<Res<DeviceOrientation>>,
    sim: Res<ClientSimState>,
) {
    let Some(assets) = assets else { return };

    let team_count = sim.repair_teams.len();
    if team_count == 0 {
        return; // wait until the server broadcasts the team roster
    }

    let is_landscape = crate::phone_border::framing::is_landscape(orientation.as_deref());

    for entity in old_panel.iter() {
        commands.entity(entity).despawn();
    }
    for (entity, overlay) in old_help.iter() {
        if overlay.0 == crate::client::elements::HelpPanel::Repair {
            commands.entity(entity).despawn();
        }
    }

    commands.insert_resource(RepairPanelSpawned { team_count });

    let shell = ConsoleShell::spawn(
        &mut commands,
        assets.helm_panel_bg.clone(),
        is_landscape,
        crate::client::elements::HelpPanel::Repair,
        move |commands: &mut Commands, primary: Entity| {
            fill_repair_primary(commands, primary, team_count);
        },
        |_commands: &mut Commands, _secondary: Entity| {
            // Repair console uses a single column; secondary slot left empty.
        },
        &assets,
    );

    commands.entity(shell.root).insert((RepairPanel, Visibility::Hidden));
}

/// Primary slot: title, hull readout, one row per repair team.
fn fill_repair_primary(commands: &mut Commands, container: Entity, team_count: usize) {
    let col = commands
        .spawn(Node {
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            row_gap: Val::Px(12.0),
            padding: UiRect::all(Val::Px(8.0)),
            ..default()
        })
        .id();
    commands.entity(container).add_child(col);

    let title = commands
        .spawn((
            Text::new("Repair Console"),
            TextFont { font_size: 24.0, ..default() },
            TextColor(Color::srgb(0.3, 1.0, 0.5)),
        ))
        .id();
    commands.entity(col).add_child(title);

    let hull_readout = TextReadout::spawn(commands, "Hull", hull_readout_visuals());
    commands.entity(hull_readout).insert(RepairHullReadout);
    commands.entity(col).add_child(hull_readout);

    for i in 0..team_count {
        let row = spawn_repair_team_row(commands, i);
        commands.entity(col).add_child(row);
    }
}

// ── Orientation respawn ──────────────────────────────────────────────

fn respawn_repair_on_team_count_change(
    sim: Res<ClientSimState>,
    spawned: Option<Res<RepairPanelSpawned>>,
    panel: Query<Entity, With<RepairPanel>>,
    mut commands: Commands,
) {
    let Some(spawned) = spawned else { return };
    if !sim.is_changed() {
        return;
    }
    if sim.repair_teams.len() != spawned.team_count {
        for entity in panel.iter() {
            commands.entity(entity).despawn();
        }
        commands.remove_resource::<RepairPanelSpawned>();
    }
}

fn respawn_repair_on_orientation_change(
    orientation: Option<Res<DeviceOrientation>>,
    panel: Query<Entity, With<RepairPanel>>,
    mut commands: Commands,
) {
    let Some(orientation) = orientation else { return };
    if !orientation.is_changed() || orientation.is_added() {
        return;
    }
    for entity in panel.iter() {
        commands.entity(entity).despawn();
    }
    commands.remove_resource::<RepairPanelSpawned>();
}

/// Spawn a single static team row (progress bar, status readout, dispatch buttons).
///
/// Returns the row root entity.
fn spawn_repair_team_row(commands: &mut Commands, team_idx: usize) -> Entity {
    let row = commands
        .spawn((
            Node {
                flex_direction: FlexDirection::Column,
                width: Val::Percent(100.0),
                padding: UiRect::all(Val::Px(4.0)),
                row_gap: Val::Px(4.0),
                ..default()
            },
            BackgroundColor(Color::srgb(0.05, 0.10, 0.20)),
        ))
        .id();

    // ── Top sub-row: progress bar + status readout ────────────────────
    let top_row = commands
        .spawn(Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            width: Val::Percent(100.0),
            height: Val::Px(32.0),
            column_gap: Val::Px(8.0),
            ..default()
        })
        .id();

    // Progress bar (continuous)
    let bar = ProgressBar::spawn(
        commands,
        Vec2::new(150.0, 28.0),
        ProgressBarVariant::Continuous,
        team_bar_idle_visuals(),
        None,
    );
    commands.entity(bar).insert(RepairTeamBar(team_idx));

    // Status TextReadout
    let status = TextReadout::spawn(commands, "", team_status_readout_visuals());
    commands.entity(status).insert(RepairTeamStatusReadout(team_idx));

    commands.entity(top_row).add_child(bar);
    commands.entity(top_row).add_child(status);
    commands.entity(row).add_child(top_row);

    // ── Bottom sub-row: dispatch buttons ──────────────────────────────
    let btn_row = commands
        .spawn(Node {
            flex_direction: FlexDirection::Row,
            justify_content: JustifyContent::Center,
            column_gap: Val::Px(6.0),
            ..default()
        })
        .id();

    for (console, label) in [
        (Console::Helm,     "HELM"),
        (Console::Tactical, "TACTICAL"),
        (Console::Power,    "POWER"),
        (Console::Shields,  "SHIELDS"),
    ] {
        let btn = spawn_gui_button(
            commands,
            ButtonSize::Rect { width: 72.0, height: 28.0 },
            dispatch_button_visuals(),
        );
        let dispatch_info = DispatchButton { console: console.clone(), team_idx };
        commands.entity(btn)
            .insert(dispatch_info)
            .with_children(|b| {
                b.spawn((
                    Text::new(label),
                    TextFont { font_size: 11.0, ..default() },
                    TextColor(Color::srgb(0.5, 1.0, 0.7)),
                ));
            })
            .observe(move |_trigger: On<ButtonPressed>,
                           mut outbound: MessageWriter<OutboundClientMessage>| {
                outbound.write(OutboundClientMessage(
                    crate::client_sim::dispatch_repair_team_message(team_idx as u8, console.clone()),
                ));
            });
        commands.entity(btn_row).add_child(btn);
    }

    commands.entity(row).add_child(btn_row);

    row
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

/// Refresh the repair panel each frame when `ClientSimState` or `LobbyState` changes:
/// - Update `ReadoutValue` on the hull readout.
/// - Update `ProgressValue` + `StateVisuals` on each team progress bar.
/// - Update `ReadoutValue` on each team status readout.
/// - Update `WidgetState` on each dispatch button to highlight the active target.
fn refresh_repair_panel(
    sim: Res<ClientSimState>,
    lobby: Res<LobbyState>,
    mut hull_readout: Query<&mut ReadoutValue, (With<RepairHullReadout>, Without<RepairTeamStatusReadout>)>,
    mut team_bars: Query<(Entity, &mut ProgressValue, &mut StateVisuals, &RepairTeamBar)>,
    mut team_statuses: Query<(&mut ReadoutValue, &RepairTeamStatusReadout), Without<RepairHullReadout>>,
    mut dispatch_buttons: Query<(&mut WidgetState, &DispatchButton)>,
) {
    if !sim.is_changed() && !lobby.is_changed() {
        return;
    }

    // Update hull readout
    if let Ok(mut rv) = hull_readout.single_mut() {
        let total_current: f32 = sim.console_hull.iter().map(|h| h.current).sum();
        let total_max: f32 = sim.console_hull.iter().map(|h| h.max_hp).sum();
        rv.0 = format!("{:.0}/{}", total_current, total_max as u32);
    }

    // Server-broadcast repair pacing (sourced from [repair] in the ship TOML).
    let travel_secs = lobby.ship_config.repair_travel_secs;
    let repair_rate = lobby.ship_config.repair_rate_hp_per_sec;

    for (i, slot) in sim.repair_teams.iter().enumerate() {
        // Derive "time to fully repair the target console" from the broadcast
        // rate and the target console's max_hp.
        let repair_secs = match slot {
            TeamSlot::Repairing { console, .. } | TeamSlot::Travelling { console, .. } => {
                let max_hp = sim.console_hull
                    .iter()
                    .find(|h| &h.console == console)
                    .map(|h| h.max_hp)
                    .unwrap_or(25.0);
                if repair_rate > 0.0 { max_hp / repair_rate } else { 50.0 }
            }
            _ => {
                let max_hp = sim.console_hull.first().map(|h| h.max_hp).unwrap_or(25.0);
                if repair_rate > 0.0 { max_hp / repair_rate } else { 50.0 }
            }
        };
        let data = row_data_for_slot_with_timings(slot, travel_secs, repair_secs);

        // Update progress bar value + visuals
        for (_, mut pv, mut visuals, bar_marker) in team_bars.iter_mut() {
            if bar_marker.0 != i {
                continue;
            }
            // ProgressValue uses [0,1]
            pv.0 = (data.pct / 100.0).clamp(0.0, 1.0);

            let new_visuals = if data.is_idle {
                team_bar_idle_visuals()
            } else if data.fills {
                match slot {
                    TeamSlot::Repairing { .. } => team_bar_repairing_visuals(),
                    _ => team_bar_travelling_visuals(),
                }
            } else {
                team_bar_returning_visuals()
            };
            *visuals = new_visuals;
        }

        // Update team status readout
        for (mut rv, status_marker) in team_statuses.iter_mut() {
            if status_marker.0 != i {
                continue;
            }
            rv.0 = data.status.clone();
        }

        // Update dispatch button WidgetState (highlight active target)
        for (mut ws, btn) in dispatch_buttons.iter_mut() {
            if btn.team_idx != i {
                continue;
            }
            let should_be_active = data.active_console.as_ref() == Some(&btn.console);
            if ws.active != should_be_active {
                ws.active = should_be_active;
            }
        }
    }
}

// ── Unit tests ───────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client_lobby::{ActiveConsole, LobbyState};
    use crate::messages::{ClientMessage, Console, GamePhase, Player, ShipClientConfig};
    use std::collections::HashMap;

    fn lobby_with_repair_player(token: &str, phase: GamePhase) -> LobbyState {
        let mut lobby = LobbyState::default();
        use crate::messages::GameState;
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
            ship_config: ShipClientConfig::default(),
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
            ship_config: ShipClientConfig::default(),
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
            ship_config: ShipClientConfig::default(),
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
            ship_config: ShipClientConfig::default(),
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
        });
        assert!((d.pct - 100.0).abs() < 0.01, "pct should be 100, got {}", d.pct);
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

    // ── row_data_for_slot_with_timings (broadcast-driven) ────────────

    #[test]
    fn travelling_pct_scales_with_broadcast_travel_duration() {
        // With travel=10s, elapsed=2.5s should yield 25 % (vs 50 % at travel=5s).
        let d = row_data_for_slot_with_timings(
            &TeamSlot::Travelling { console: Console::Helm, elapsed: 2.5 },
            10.0, // broadcast travel duration
            50.0, // repair duration unused for Travelling
        );
        assert!((d.pct - 25.0).abs() < 0.01, "pct should be 25, got {}", d.pct);
        assert_eq!(d.active_console, Some(Console::Helm));
    }

    #[test]
    fn repairing_pct_is_always_full() {
        // Repairing shows a full bar regardless of how long the team has been there.
        let d = row_data_for_slot_with_timings(
            &TeamSlot::Repairing { console: Console::Tactical },
            5.0,
            100.0,
        );
        assert!((d.pct - 100.0).abs() < 0.01, "pct should be 100, got {}", d.pct);
    }

    #[test]
    fn returning_pct_scales_with_broadcast_travel_duration() {
        let d = row_data_for_slot_with_timings(
            &TeamSlot::Returning { remaining: 2.5, queued: None },
            10.0,
            50.0,
        );
        assert!((d.pct - 25.0).abs() < 0.01, "drain pct should be 25, got {}", d.pct);
        assert!(!d.fills);
    }

    #[test]
    fn zero_broadcast_timings_fall_back_to_baseline() {
        // If the broadcast hasn't arrived yet or carries a zero (misconfigured
        // TOML), the helper falls back to baseline so the UI never produces NaN.
        let d = row_data_for_slot_with_timings(
            &TeamSlot::Travelling { console: Console::Helm, elapsed: 2.5 },
            0.0, 0.0,
        );
        assert!((d.pct - 50.0).abs() < 0.01, "should fall back to 5s travel, got {}", d.pct);
    }

    // ── Visual helpers ───────────────────────────────────────────────

    #[test]
    fn dispatch_button_visuals_active_differs_from_idle() {
        use crate::gui::resolve_visual;
        let v = dispatch_button_visuals();
        let idle   = resolve_visual(&v, false, false, false, false).color;
        let active = resolve_visual(&v, false, false, true, false).color;
        assert_ne!(idle, active, "active dispatch button should look different from idle");
    }

    #[test]
    fn dispatch_button_visuals_has_five_distinct_states() {
        use crate::gui::resolve_visual;
        let v = dispatch_button_visuals();
        let idle     = resolve_visual(&v, false, false, false, false).color;
        let hover    = resolve_visual(&v, false, false, false, true ).color;
        let active   = resolve_visual(&v, false, false, true,  false).color;
        let press    = resolve_visual(&v, false, true,  false, false).color;
        let disabled = resolve_visual(&v, true,  false, false, false).color;
        assert_ne!(idle, hover);
        assert_ne!(idle, active);
        assert_ne!(idle, press);
        assert_ne!(idle, disabled);
    }

    #[test]
    fn team_bar_idle_visuals_disabled_differs_from_idle() {
        use crate::gui::resolve_visual;
        let v = team_bar_idle_visuals();
        let idle     = resolve_visual(&v, false, false, false, false).color;
        let disabled = resolve_visual(&v, true,  false, false, false).color;
        assert_ne!(idle, disabled);
    }

    #[test]
    fn team_bar_repairing_visuals_differs_from_travelling() {
        use crate::gui::resolve_visual;
        let repairing  = team_bar_repairing_visuals();
        let travelling = team_bar_travelling_visuals();
        let rep_idle = resolve_visual(&repairing,  false, false, false, false).color;
        let tra_idle = resolve_visual(&travelling, false, false, false, false).color;
        assert_ne!(rep_idle, tra_idle, "repairing and travelling bar colours should differ");
    }

    #[test]
    fn team_bar_returning_visuals_differs_from_repairing() {
        use crate::gui::resolve_visual;
        let returning  = team_bar_returning_visuals();
        let repairing  = team_bar_repairing_visuals();
        let ret_idle = resolve_visual(&returning, false, false, false, false).color;
        let rep_idle = resolve_visual(&repairing, false, false, false, false).color;
        assert_ne!(ret_idle, rep_idle);
    }

    // ── ProgressValue conversion ─────────────────────────────────────

    #[test]
    fn pct_to_progress_value_half() {
        // row_data pct is 0–100; ProgressValue expects 0–1
        let d = row_data_for_slot(&TeamSlot::Travelling {
            console: Console::Helm,
            elapsed: 2.5,
        });
        let progress_value = (d.pct / 100.0).clamp(0.0, 1.0);
        assert!((progress_value - 0.5).abs() < 0.01);
    }

    #[test]
    fn pct_to_progress_value_full() {
        let d = row_data_for_slot(&TeamSlot::Repairing {
            console: Console::Tactical,
        });
        let progress_value = (d.pct / 100.0).clamp(0.0, 1.0);
        assert_eq!(progress_value, 1.0);
    }

    #[test]
    fn pct_to_progress_value_zero() {
        let d = row_data_for_slot(&TeamSlot::Idle);
        let progress_value = (d.pct / 100.0).clamp(0.0, 1.0);
        assert_eq!(progress_value, 0.0);
    }
}
