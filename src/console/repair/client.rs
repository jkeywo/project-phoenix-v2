//! Client-side Repair Panel plugin — migrated to HTML console (issue #425).
//!
//! The Bevy widget tree has been removed. The server now pushes
//! `ConsoleStateChanged { name: "Repair", json }` which the JS bridge
//! delivers to `window.__updateConsole("Repair", json)`. The HTML panel
//! at `gui/repair-console.html` renders the state and sends
//! `window.__sendAction({ action: "dispatch_repair_team", ... })`
//! back through the `UiAction` bridge.
//!
//! This module keeps:
//! - `repair_panel_visible` — pure visibility predicate (unit tested)
//! - `row_data_for_slot` / `row_data_for_slot_with_timings` — pure row-data
//!   derivation helpers (unit tested)
//! - Visual helper fns (used only in unit tests now)
//! - `RepairPanelPlugin` as an empty no-op so `client/app.rs` keeps compiling

use bevy::prelude::*;

use crate::client_lobby::{ActiveConsole, LobbyState, LobbyView};
use crate::gui::StateVisuals;
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
    pub fills: bool,
    /// Human-readable status label.
    pub status: String,
    /// Whether this team slot is `Idle`.
    pub is_idle: bool,
    /// Console the team is currently heading toward or working at.
    pub active_console: Option<Console>,
}

const TRAVEL_DURATION_SECS: f32 = 5.0;
const REPAIR_DURATION_SECS: f32 = 50.0;

/// Derive [`RowData`] from a single `TeamSlot` using server-broadcast timings.
pub fn row_data_for_slot_with_timings(
    slot: &TeamSlot,
    travel_duration_secs: f32,
    repair_duration_secs: f32,
) -> RowData {
    let travel = if travel_duration_secs > 0.0 { travel_duration_secs } else { TRAVEL_DURATION_SECS };
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

/// Back-compat wrapper using baseline timings (5 s travel, 50 s repair).
pub fn row_data_for_slot(slot: &TeamSlot) -> RowData {
    row_data_for_slot_with_timings(slot, TRAVEL_DURATION_SECS, REPAIR_DURATION_SECS)
}

// ── Pure visual helpers ──────────────────────────────────────────────

pub fn team_bar_idle_visuals() -> StateVisuals {
    StateVisuals::from_colors(
        Color::srgb(0.10, 0.20, 0.60),
        Color::srgb(0.10, 0.20, 0.60),
        Color::srgb(0.10, 0.20, 0.60),
        Color::srgb(0.10, 0.20, 0.60),
        Color::srgb(0.05, 0.10, 0.20),
    )
}

pub fn team_bar_travelling_visuals() -> StateVisuals {
    StateVisuals::from_colors(
        Color::srgb(0.60, 0.60, 0.10),
        Color::srgb(0.60, 0.60, 0.10),
        Color::srgb(0.70, 0.70, 0.12),
        Color::srgb(0.60, 0.60, 0.10),
        Color::srgb(0.05, 0.10, 0.20),
    )
}

pub fn team_bar_repairing_visuals() -> StateVisuals {
    StateVisuals::from_colors(
        Color::srgb(0.10, 0.70, 0.20),
        Color::srgb(0.10, 0.70, 0.20),
        Color::srgb(0.12, 0.80, 0.22),
        Color::srgb(0.10, 0.70, 0.20),
        Color::srgb(0.05, 0.10, 0.20),
    )
}

pub fn team_bar_returning_visuals() -> StateVisuals {
    StateVisuals::from_colors(
        Color::srgb(0.30, 0.30, 0.80),
        Color::srgb(0.30, 0.30, 0.80),
        Color::srgb(0.35, 0.35, 0.90),
        Color::srgb(0.30, 0.30, 0.80),
        Color::srgb(0.05, 0.10, 0.20),
    )
}

pub fn team_status_readout_visuals() -> StateVisuals {
    StateVisuals::from_colors(
        Color::srgb(0.5, 0.7, 1.0),
        Color::srgb(0.5, 0.7, 1.0),
        Color::srgb(0.8, 0.9, 1.0),
        Color::srgb(0.5, 0.7, 1.0),
        Color::srgb(0.3, 0.3, 0.4),
    )
}

pub fn hull_readout_visuals() -> StateVisuals {
    StateVisuals::from_colors(
        Color::srgb(0.6, 0.8, 1.0),
        Color::srgb(0.6, 0.8, 1.0),
        Color::srgb(0.6, 0.8, 1.0),
        Color::srgb(0.6, 0.8, 1.0),
        Color::srgb(0.3, 0.3, 0.4),
    )
}

pub fn dispatch_button_visuals() -> StateVisuals {
    StateVisuals::from_colors(
        Color::srgb(0.10, 0.25, 0.15),
        Color::srgb(0.15, 0.35, 0.20),
        Color::srgb(0.20, 0.55, 0.30),
        Color::srgb(0.25, 0.45, 0.25),
        Color::srgb(0.05, 0.10, 0.08),
    )
}

// ── Plugin ───────────────────────────────────────────────────────────

/// Plugin stub kept for `client/app.rs` compile compatibility.
///
/// The repair console UI is now rendered by `gui/repair-console.html`.
pub struct RepairPanelPlugin;

impl Plugin for RepairPanelPlugin {
    fn build(&self, _app: &mut App) {
        // No-op: HTML panel replaces the Bevy widget tree.
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
        let active = ActiveConsole::default();
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
        let d = row_data_for_slot_with_timings(
            &TeamSlot::Travelling { console: Console::Helm, elapsed: 2.5 },
            10.0,
            50.0,
        );
        assert!((d.pct - 25.0).abs() < 0.01, "pct should be 25, got {}", d.pct);
        assert_eq!(d.active_console, Some(Console::Helm));
    }

    #[test]
    fn repairing_pct_is_always_full() {
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
