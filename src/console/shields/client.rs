//! Client-side Shields Panel plugin — migrated to HTML console (issue #424).
//!
//! The Bevy widget tree has been removed. The server now pushes
//! `ConsoleStateChanged { name: "Shields", json }` which the JS bridge
//! delivers to `window.__updateConsole("Shields", json)`. The HTML panel
//! at `gui/shield-console.html` renders the state and sends
//! `window.__sendAction({ action: "set_shield_focus", facing })`
//! back through the `UiAction` bridge.
//!
//! This module keeps:
//! - `shields_panel_visible` — pure visibility predicate (unit tested)
//! - Pure helper fns for HP bar rendering (used only in unit tests now)
//! - `ShieldsPanelPlugin` as an empty no-op so `client/app.rs` keeps compiling

use bevy::prelude::*;

use crate::client_lobby::{ActiveConsole, LobbyState, LobbyView};
use crate::gui::StateVisuals;
use crate::messages::{Console, GamePhase, ViewDirection};

// ── Pure visibility helper ────────────────────────────────────────────

/// Decide whether the shields panel should be visible.
pub fn shields_panel_visible(
    lobby: &LobbyState,
    token: &str,
    active: &ActiveConsole,
) -> bool {
    if lobby.phase != GamePhase::InProgress {
        return false;
    }
    let view = LobbyView::new(lobby, token);
    let consoles = view.my_consoles();
    if !consoles.contains(&Console::Shields) {
        return false;
    }
    let count = consoles.len();
    match &active.0 {
        Some(c) => *c == Console::Shields,
        None => count == 1,
    }
}

// ── Pure helpers (kept for unit test coverage) ────────────────────────

pub fn focus_button_visuals() -> StateVisuals {
    StateVisuals::from_colors(
        Color::srgb(0.10, 0.20, 0.35),
        Color::srgb(0.14, 0.28, 0.48),
        Color::srgb(0.10, 0.55, 0.75),
        Color::srgb(0.15, 0.35, 0.55),
        Color::srgb(0.05, 0.08, 0.14),
    )
}

pub fn clear_button_visuals() -> StateVisuals {
    StateVisuals::from_colors(
        Color::srgb(0.35, 0.25, 0.10),
        Color::srgb(0.50, 0.36, 0.16),
        Color::srgb(0.70, 0.50, 0.20),
        Color::srgb(0.55, 0.40, 0.18),
        Color::srgb(0.14, 0.10, 0.05),
    )
}

pub fn hp_bar_visuals() -> StateVisuals {
    StateVisuals::from_colors(
        Color::srgb(0.20, 0.40, 0.80),
        Color::srgb(0.20, 0.40, 0.80),
        Color::srgb(0.20, 0.80, 0.40),
        Color::srgb(0.20, 0.40, 0.80),
        Color::srgb(0.30, 0.10, 0.10),
    )
}

pub fn hp_readout_visuals() -> StateVisuals {
    StateVisuals::from_colors(
        Color::srgb(0.60, 0.80, 1.00),
        Color::srgb(0.60, 0.80, 1.00),
        Color::srgb(0.30, 1.00, 0.60),
        Color::srgb(0.60, 0.80, 1.00),
        Color::srgb(0.35, 0.35, 0.45),
    )
}

pub fn hp_fraction(hp: i32, max_hp: i32) -> f32 {
    if max_hp <= 0 {
        0.0
    } else {
        (hp as f32 / max_hp as f32).clamp(0.0, 1.0)
    }
}

pub fn hp_readout_text(hp: i32, max_hp: i32) -> String {
    format!("{}/{}", hp, max_hp)
}

pub fn direction_to_label(dir: &ViewDirection) -> &'static str {
    match dir {
        ViewDirection::Fore => "Fore",
        ViewDirection::Port => "Port",
        ViewDirection::Aft => "Aft",
        ViewDirection::Starboard => "Starboard",
    }
}

pub const FOCUS_DIRECTIONS: [ViewDirection; 4] = [
    ViewDirection::Fore,
    ViewDirection::Port,
    ViewDirection::Aft,
    ViewDirection::Starboard,
];

pub const FACING_LABELS: [&str; 4] = ["Fore", "Port", "Aft", "Starboard"];

// ── No-op plugin ─────────────────────────────────────────────────────

/// Plugin stub kept for `client/app.rs` compile compatibility.
pub struct ShieldsPanelPlugin;

impl Plugin for ShieldsPanelPlugin {
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
    use crate::messages::{ClientMessage, Console, GamePhase, GameState, Player, ServerMessage, ShipClientConfig};
    use crate::stations_config::ShipStations;
    use std::collections::HashMap;

    fn player(token: &str, consoles: Vec<Console>) -> Player {
        Player {
            token: token.into(),
            name: "test".into(),
            consoles,
            connected: true,
        }
    }

    fn game_state(phase: GamePhase, players: Vec<Player>) -> GameState {
        GameState {
            phase,
            players,
            complexity: HashMap::new(),
            world: None,
        }
    }

    fn welcome(state: GameState) -> ServerMessage {
        ServerMessage::Welcome {
            state,
            ship_stations: ShipStations::default(),
            ship_config: ShipClientConfig::default(),
        }
    }

    fn in_progress_shields_lobby(token: &str) -> LobbyState {
        let mut s = LobbyState::default();
        s.apply(&welcome(game_state(
            GamePhase::InProgress,
            vec![player(token, vec![Console::Shields])],
        )));
        s
    }

    fn no_tab() -> ActiveConsole { ActiveConsole(None) }
    fn tab(c: Console) -> ActiveConsole { ActiveConsole(Some(c)) }

    // ── shields_panel_visible ────────────────────────────────────────

    #[test]
    fn shields_panel_hidden_in_lobby_phase() {
        let lobby = LobbyState::default();
        let active = no_tab();
        assert!(!shields_panel_visible(&lobby, "tok", &active));
    }

    #[test]
    fn shields_panel_visible_in_progress_holding_shields() {
        let lobby = in_progress_shields_lobby("tok");
        let active = no_tab();
        assert!(shields_panel_visible(&lobby, "tok", &active));
    }

    #[test]
    fn shields_panel_hidden_when_player_does_not_hold_shields() {
        let lobby = in_progress_shields_lobby("tok");
        let active = no_tab();
        assert!(!shields_panel_visible(&lobby, "other", &active));
    }

    #[test]
    fn shields_panel_visible_when_active_console_is_shields_multi_console() {
        let mut lobby = LobbyState::default();
        lobby.apply(&welcome(game_state(
            GamePhase::InProgress,
            vec![player("tok", vec![Console::Shields, Console::Tactical])],
        )));
        let active = tab(Console::Shields);
        assert!(shields_panel_visible(&lobby, "tok", &active));
    }

    #[test]
    fn shields_panel_hidden_when_active_console_is_other_multi_console() {
        let mut lobby = LobbyState::default();
        lobby.apply(&welcome(game_state(
            GamePhase::InProgress,
            vec![player("tok", vec![Console::Shields, Console::Tactical])],
        )));
        let active = tab(Console::Tactical);
        assert!(!shields_panel_visible(&lobby, "tok", &active));
    }

    #[test]
    fn shields_panel_hidden_when_no_active_console_and_holding_multiple() {
        let mut lobby = LobbyState::default();
        lobby.apply(&welcome(game_state(
            GamePhase::InProgress,
            vec![player("tok", vec![Console::Shields, Console::Tactical])],
        )));
        let active = no_tab();
        assert!(!shields_panel_visible(&lobby, "tok", &active));
    }

    // ── hp_fraction ─────────────────────────────────────────────────

    #[test]
    fn hp_fraction_full_hp_returns_one() {
        assert_eq!(hp_fraction(100, 100), 1.0);
    }

    #[test]
    fn hp_fraction_zero_hp_returns_zero() {
        assert_eq!(hp_fraction(0, 100), 0.0);
    }

    #[test]
    fn hp_fraction_half_hp_returns_half() {
        assert!((hp_fraction(50, 100) - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn hp_fraction_zero_max_returns_zero() {
        assert_eq!(hp_fraction(10, 0), 0.0);
    }

    #[test]
    fn hp_fraction_clamped_above_one() {
        assert_eq!(hp_fraction(150, 100), 1.0);
    }

    #[test]
    fn hp_fraction_negative_hp_clamped_to_zero() {
        assert_eq!(hp_fraction(-5, 100), 0.0);
    }

    // ── hp_readout_text ─────────────────────────────────────────────

    #[test]
    fn hp_readout_text_formats_correctly() {
        assert_eq!(hp_readout_text(75, 100), "75/100");
    }

    #[test]
    fn hp_readout_text_zero_hp() {
        assert_eq!(hp_readout_text(0, 100), "0/100");
    }

    #[test]
    fn hp_readout_text_full_hp() {
        assert_eq!(hp_readout_text(100, 100), "100/100");
    }

    // ── direction_to_label ──────────────────────────────────────────

    #[test]
    fn direction_to_label_fore() {
        assert_eq!(direction_to_label(&ViewDirection::Fore), "Fore");
    }

    #[test]
    fn direction_to_label_port() {
        assert_eq!(direction_to_label(&ViewDirection::Port), "Port");
    }

    #[test]
    fn direction_to_label_aft() {
        assert_eq!(direction_to_label(&ViewDirection::Aft), "Aft");
    }

    #[test]
    fn direction_to_label_starboard() {
        assert_eq!(direction_to_label(&ViewDirection::Starboard), "Starboard");
    }

    // ── focus_button_visuals ───────────────────────────────────────

    #[test]
    fn focus_button_visuals_has_distinct_five_states() {
        let v = focus_button_visuals();
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

    // ── clear_button_visuals ────────────────────────────────────────

    #[test]
    fn clear_button_visuals_idle_differs_from_press() {
        let v = clear_button_visuals();
        let idle  = resolve_visual(&v, false, false, false, false).color;
        let press = resolve_visual(&v, false, true,  false, false).color;
        assert_ne!(idle, press);
    }

    // ── hp_bar_visuals ──────────────────────────────────────────────

    #[test]
    fn hp_bar_visuals_active_differs_from_idle() {
        let v = hp_bar_visuals();
        let idle   = resolve_visual(&v, false, false, false, false).color;
        let active = resolve_visual(&v, false, false, true,  false).color;
        assert_ne!(idle, active);
    }

    #[test]
    fn hp_bar_visuals_disabled_differs_from_idle() {
        let v = hp_bar_visuals();
        let idle     = resolve_visual(&v, false, false, false, false).color;
        let disabled = resolve_visual(&v, true,  false, false, false).color;
        assert_ne!(idle, disabled);
    }

    // ── hp_readout_visuals ──────────────────────────────────────────

    #[test]
    fn hp_readout_visuals_active_differs_from_idle() {
        let v = hp_readout_visuals();
        let idle   = resolve_visual(&v, false, false, false, false).color;
        let active = resolve_visual(&v, false, false, true,  false).color;
        assert_ne!(idle, active);
    }

    // ── FOCUS_DIRECTIONS order ──────────────────────────────────────

    #[test]
    fn focus_directions_order_matches_facing_labels() {
        assert_eq!(direction_to_label(&FOCUS_DIRECTIONS[0]), FACING_LABELS[0]);
        assert_eq!(direction_to_label(&FOCUS_DIRECTIONS[1]), FACING_LABELS[1]);
        assert_eq!(direction_to_label(&FOCUS_DIRECTIONS[2]), FACING_LABELS[2]);
        assert_eq!(direction_to_label(&FOCUS_DIRECTIONS[3]), FACING_LABELS[3]);
    }

    #[test]
    fn focus_directions_has_four_entries() {
        assert_eq!(FOCUS_DIRECTIONS.len(), 4);
    }

    // ── SetShieldFocus message construction ─────────────────────────

    #[test]
    fn set_shield_focus_fore_message() {
        let facing = Some(ViewDirection::Fore);
        let msg = ClientMessage::SetShieldFocus { facing: facing.clone() };
        assert_eq!(msg, ClientMessage::SetShieldFocus { facing });
    }

    #[test]
    fn set_shield_focus_none_clears_focus() {
        let msg = ClientMessage::SetShieldFocus { facing: None };
        assert_eq!(msg, ClientMessage::SetShieldFocus { facing: None });
    }

    #[test]
    fn set_shield_focus_all_directions() {
        for dir in &FOCUS_DIRECTIONS {
            let msg = ClientMessage::SetShieldFocus {
                facing: Some(dir.clone()),
            };
            assert!(matches!(msg, ClientMessage::SetShieldFocus { facing: Some(_) }));
        }
    }
}
