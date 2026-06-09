//! Client-side Power Panel plugin — migrated to HTML console (issue #425).
//!
//! The Bevy widget tree has been removed. The server now pushes
//! `ConsoleStateChanged { name: "Power", json }` which the JS bridge
//! delivers to `window.__updateConsole("Power", json)`. The HTML panel
//! at `gui/power-console.html` renders the state and sends
//! `window.__sendAction({ action: "increase_power"|"decrease_power", ... })`
//! back through the `UiAction` bridge.
//!
//! This module keeps:
//! - `power_panel_visible` — pure visibility predicate (unit tested)
//! - Visual helper fns (`inc_visuals`, `dec_visuals`, `battery_visuals`,
//!   `level_readout_visuals`) — used only in unit tests now
//! - `PowerPanelPlugin` as an empty no-op so `client/app.rs` keeps compiling

use bevy::prelude::*;

use crate::client_lobby::{ActiveConsole, LobbyState, LobbyView};
use crate::gui::StateVisuals;
use crate::messages::{Console, GamePhase};

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

// ── Pure helpers ──────────────────────────────────────────────────────

/// Build `StateVisuals` for an increment button.
pub fn inc_visuals() -> StateVisuals {
    StateVisuals::from_colors(
        Color::srgb(0.10, 0.50, 0.30),
        Color::srgb(0.14, 0.60, 0.35),
        Color::srgb(0.10, 0.65, 0.35),
        Color::srgb(0.18, 0.70, 0.40),
        Color::srgb(0.06, 0.06, 0.10),
    )
}

/// Build `StateVisuals` for a decrement button.
pub fn dec_visuals() -> StateVisuals {
    StateVisuals::from_colors(
        Color::srgb(0.50, 0.20, 0.10),
        Color::srgb(0.60, 0.24, 0.12),
        Color::srgb(0.65, 0.20, 0.10),
        Color::srgb(0.75, 0.25, 0.12),
        Color::srgb(0.06, 0.06, 0.10),
    )
}

/// Build `StateVisuals` for the battery `ProgressBar`.
pub fn battery_visuals() -> StateVisuals {
    StateVisuals::from_colors(
        Color::srgb(0.10, 0.60, 0.80),
        Color::srgb(0.10, 0.60, 0.80),
        Color::srgb(0.10, 0.60, 0.80),
        Color::srgb(0.10, 0.60, 0.80),
        Color::srgb(0.15, 0.15, 0.25),
    )
}

/// Build `StateVisuals` for a power level `TextReadout`.
pub fn level_readout_visuals() -> StateVisuals {
    StateVisuals::from_colors(
        Color::srgb(0.9, 0.9, 1.0),
        Color::srgb(0.9, 0.9, 1.0),
        Color::srgb(0.3, 1.0, 0.8),
        Color::srgb(0.9, 0.9, 1.0),
        Color::srgb(0.4, 0.4, 0.5),
    )
}

// ── Plugin ───────────────────────────────────────────────────────────

/// Plugin stub kept for `client/app.rs` compile compatibility.
///
/// The power console UI is now rendered by `gui/power-console.html`.
pub struct PowerPanelPlugin;

impl Plugin for PowerPanelPlugin {
    fn build(&self, _app: &mut App) {
        // No-op: HTML panel replaces the Bevy widget tree.
    }
}

// ── Unit tests ───────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client_lobby::{ActiveConsole, LobbyState};
    use crate::gui::resolve_visual;
    use crate::messages::{Console, GamePhase, Player, ShipClientConfig};
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
            ship_config: ShipClientConfig::default(),
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
            ship_config: ShipClientConfig::default(),
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
            ship_config: ShipClientConfig::default(),
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
            ship_config: ShipClientConfig::default(),
        });
        let active = ActiveConsole::default(); // None → auto → count != 1
        assert!(!power_panel_visible(&lobby, "tok", &active));
    }

    // ── inc_visuals / dec_visuals: five distinct states ───────────────

    #[test]
    fn inc_visuals_has_distinct_five_states() {
        let v = inc_visuals();
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
    fn dec_visuals_has_distinct_five_states() {
        let v = dec_visuals();
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
    fn battery_visuals_disabled_differs_from_idle() {
        let v = battery_visuals();
        let idle     = resolve_visual(&v, false, false, false, false).color;
        let disabled = resolve_visual(&v, true,  false, false, false).color;
        assert_ne!(idle, disabled);
    }

    // ── Battery formatting helpers ───────────────────────────────────

    #[test]
    fn battery_readout_formats_percentage() {
        let battery_raw = 75.0_f32;
        let text = format!("{:.0}%", battery_raw);
        assert_eq!(text, "75%");
    }

    #[test]
    fn battery_fraction_clamped_to_unit_interval() {
        let raw = 120.0_f32;
        let fraction = (raw / 100.0).clamp(0.0, 1.0);
        assert_eq!(fraction, 1.0);

        let raw_neg = -10.0_f32;
        let fraction_neg = (raw_neg / 100.0).clamp(0.0, 1.0);
        assert_eq!(fraction_neg, 0.0);
    }

    // ── can_increase / can_decrease integration with is_power_locked ─

    #[test]
    fn increase_blocked_when_locked() {
        use crate::client_sim::{can_increase_power, is_power_locked};
        let payload = Some((2u8, 2u8, 2u8, 50.0_f32, true));
        let locked = is_power_locked(&payload);
        assert!(!can_increase_power(&(2, 2, 2), &Console::Helm, locked));
    }

    #[test]
    fn decrease_blocked_when_locked() {
        use crate::client_sim::{can_decrease_power, is_power_locked};
        let payload = Some((2u8, 2u8, 2u8, 50.0_f32, true));
        let locked = is_power_locked(&payload);
        assert!(!can_decrease_power(&(2, 2, 2), &Console::Helm, locked));
    }

    #[test]
    fn increase_allowed_when_not_locked_and_under_cap() {
        use crate::client_sim::{can_increase_power, is_power_locked};
        let payload = Some((2u8, 2u8, 2u8, 50.0_f32, false));
        let locked = is_power_locked(&payload);
        assert!(can_increase_power(&(2, 2, 2), &Console::Helm, locked));
    }

    #[test]
    fn decrease_allowed_when_not_locked_and_above_min() {
        use crate::client_sim::{can_decrease_power, is_power_locked};
        let payload = Some((3u8, 2u8, 2u8, 50.0_f32, false));
        let locked = is_power_locked(&payload);
        assert!(can_decrease_power(&(3, 2, 2), &Console::Helm, locked));
    }

    // ── increase_power_message / decrease_power_message ─────────────

    #[test]
    fn increase_power_message_produces_correct_variant() {
        use crate::client_sim::increase_power_message;
        use crate::messages::ClientMessage;
        let msg = increase_power_message(Console::Helm);
        assert_eq!(msg, ClientMessage::IncreasePower { console: Console::Helm });
    }

    #[test]
    fn decrease_power_message_produces_correct_variant() {
        use crate::client_sim::decrease_power_message;
        use crate::messages::ClientMessage;
        let msg = decrease_power_message(Console::Sensors);
        assert_eq!(msg, ClientMessage::DecreasePower { console: Console::Sensors });
    }

    #[test]
    fn increase_power_message_tactical() {
        use crate::client_sim::increase_power_message;
        use crate::messages::ClientMessage;
        let msg = increase_power_message(Console::Tactical);
        assert_eq!(msg, ClientMessage::IncreasePower { console: Console::Tactical });
    }

    #[test]
    fn decrease_power_message_tactical() {
        use crate::client_sim::decrease_power_message;
        use crate::messages::ClientMessage;
        let msg = decrease_power_message(Console::Tactical);
        assert_eq!(msg, ClientMessage::DecreasePower { console: Console::Tactical });
    }

    #[test]
    fn increase_power_message_sensors() {
        use crate::client_sim::increase_power_message;
        use crate::messages::ClientMessage;
        let msg = increase_power_message(Console::Sensors);
        assert_eq!(msg, ClientMessage::IncreasePower { console: Console::Sensors });
    }
}
