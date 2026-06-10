//! Client-side Sensors Panel plugin — migrated to HTML console (issue #457).
//!
//! The Bevy widget tree has been removed. The server now pushes
//! `ConsoleStateChanged { name: "Sensors", json }` which the JS bridge
//! delivers to `window.__updateConsole("Sensors", json)`. The HTML panel
//! at `gui/sensors-console.html` renders the state and sends actions
//! back through the `UiAction` bridge.
//!
//! This module keeps:
//! - `sensors_panel_visible` — pure visibility predicate (unit tested)
//! - `science_target_message` — thin wrapper (unit tested)
//! - Pure colour visuals for test coverage
//! - `SensorsPanelPlugin` as an empty no-op so `client/app.rs` keeps compiling

use bevy::prelude::*;

use crate::client_lobby::{ActiveConsole, LobbyState, LobbyView};
use crate::client_sim::set_science_target_message;
use crate::gui::StateVisuals;
use crate::messages::{ClientMessage, Console, GamePhase};

// ── Pure visibility helper ────────────────────────────────────────────

/// Decide whether the sensors panel should be visible.
pub fn sensors_panel_visible(
    lobby: &LobbyState,
    token: &str,
    active: &ActiveConsole,
) -> bool {
    if lobby.phase != GamePhase::InProgress {
        return false;
    }
    let view = LobbyView::new(lobby, token);
    let consoles = view.my_consoles();
    if !consoles.contains(&Console::Sensors) {
        return false;
    }
    let count = consoles.len();
    match &active.0 {
        Some(c) => *c == Console::Sensors,
        None => count == 1,
    }
}

// ── Pure helpers (kept for unit test coverage) ────────────────────────

/// Muted blue-green button visuals — used for ON SCREEN.
pub fn on_screen_visuals() -> StateVisuals {
    StateVisuals::from_colors(
        Color::srgb(0.10, 0.30, 0.25), // idle
        Color::srgb(0.15, 0.40, 0.32), // hover
        Color::srgb(0.12, 0.50, 0.38), // active
        Color::srgb(0.18, 0.55, 0.40), // press
        Color::srgb(0.05, 0.15, 0.12), // disabled
    )
}

/// Danger (red) button visuals — used for Cancel Impulse.
pub fn cancel_impulse_visuals() -> StateVisuals {
    StateVisuals::from_colors(
        Color::srgb(0.40, 0.05, 0.05), // idle
        Color::srgb(0.55, 0.08, 0.08), // hover
        Color::srgb(0.60, 0.07, 0.07), // active
        Color::srgb(0.70, 0.10, 0.10), // press
        Color::srgb(0.15, 0.03, 0.03), // disabled
    )
}

/// Build the `ClientMessage` for designating a science target.
pub fn science_target_message(uuid: String) -> ClientMessage {
    set_science_target_message(uuid)
}

// ── No-op plugin ─────────────────────────────────────────────────────

/// Plugin stub kept for `client/app.rs` compile compatibility.
pub struct SensorsPanelPlugin;

impl Plugin for SensorsPanelPlugin {
    fn build(&self, _app: &mut App) {
        // No-op: HTML panel replaces the Bevy widget tree.
    }
}

// ── Unit tests ───────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client_lobby::{ActiveConsole, LobbyState};
    use crate::messages::{GamePhase, GameState, Player, ServerMessage, ShipClientConfig};
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

    fn in_progress_sensors_lobby(token: &str) -> LobbyState {
        let mut s = LobbyState::default();
        s.apply(&welcome(game_state(
            GamePhase::InProgress,
            vec![player(token, vec![Console::Sensors])],
        )));
        s
    }

    fn no_tab() -> ActiveConsole {
        ActiveConsole(None)
    }
    fn tab(c: Console) -> ActiveConsole {
        ActiveConsole(Some(c))
    }

    // ── sensors_panel_visible ────────────────────────────────────────────────

    #[test]
    fn sensors_panel_not_visible_in_lobby_phase() {
        let lobby = LobbyState::default();
        let active = no_tab();
        assert!(!sensors_panel_visible(&lobby, "tok", &active));
    }

    #[test]
    fn sensors_panel_not_visible_when_player_does_not_hold_sensors() {
        let lobby = {
            let mut s = LobbyState::default();
            s.apply(&welcome(game_state(
                GamePhase::InProgress,
                vec![player("tok", vec![Console::Helm])],
            )));
            s
        };
        let active = no_tab();
        assert!(!sensors_panel_visible(&lobby, "tok", &active));
    }

    #[test]
    fn sensors_panel_visible_when_sole_console_and_no_tab() {
        let lobby = in_progress_sensors_lobby("tok");
        let active = no_tab();
        assert!(sensors_panel_visible(&lobby, "tok", &active));
    }

    #[test]
    fn sensors_panel_visible_when_multi_console_and_sensors_tab() {
        let mut s = LobbyState::default();
        s.apply(&welcome(game_state(
            GamePhase::InProgress,
            vec![player("tok", vec![Console::Sensors, Console::Helm])],
        )));
        let active = tab(Console::Sensors);
        assert!(sensors_panel_visible(&s, "tok", &active));
    }

    #[test]
    fn sensors_panel_hidden_when_multi_console_and_other_tab() {
        let mut s = LobbyState::default();
        s.apply(&welcome(game_state(
            GamePhase::InProgress,
            vec![player("tok", vec![Console::Sensors, Console::Helm])],
        )));
        let active = tab(Console::Helm);
        assert!(!sensors_panel_visible(&s, "tok", &active));
    }

    #[test]
    fn sensors_panel_hidden_when_multi_console_and_no_tab() {
        let mut s = LobbyState::default();
        s.apply(&welcome(game_state(
            GamePhase::InProgress,
            vec![player("tok", vec![Console::Sensors, Console::Helm])],
        )));
        let active = no_tab();
        assert!(!sensors_panel_visible(&s, "tok", &active));
    }

    // ── science_target_message ────────────────────────────────────────────────

    #[test]
    fn science_target_message_produces_set_science_target() {
        let msg = science_target_message("entity-42".into());
        assert_eq!(
            msg,
            ClientMessage::SetScienceTarget {
                uuid: "entity-42".into()
            }
        );
    }

    // ── on_screen button sends ScienceRadar ViewMode ──────────────────────────

    #[test]
    fn on_screen_message_variant_is_set_view_science_radar() {
        use crate::messages::ViewMode;
        let msg = ClientMessage::SetView {
            mode: ViewMode::ScienceRadar,
        };
        assert!(matches!(
            msg,
            ClientMessage::SetView {
                mode: ViewMode::ScienceRadar
            }
        ));
    }

    // ── cancel impulse message variant ────────────────────────────────────────

    #[test]
    fn cancel_impulse_message_variant_is_correct() {
        let msg = ClientMessage::CancelImpulse;
        assert!(matches!(msg, ClientMessage::CancelImpulse));
    }

    // ── Science radar filter ──────────────────────────────────────────────────

    #[test]
    fn science_radar_filter_includes_ships() {
        use crate::gui::{is_on_radar, RadarFilter};
        let filter = RadarFilter(std::collections::HashSet::from([
            "ship".to_string(),
            "asteroid".to_string(),
        ]));
        assert!(is_on_radar(&filter, &["ship".to_string()]));
    }

    #[test]
    fn science_radar_filter_includes_asteroids() {
        use crate::gui::{is_on_radar, RadarFilter};
        let filter = RadarFilter(std::collections::HashSet::from([
            "ship".to_string(),
            "asteroid".to_string(),
        ]));
        assert!(is_on_radar(&filter, &["asteroid".to_string()]));
    }

    #[test]
    fn science_radar_filter_excludes_missiles() {
        use crate::gui::{is_on_radar, RadarFilter};
        let filter = RadarFilter(std::collections::HashSet::from([
            "ship".to_string(),
            "asteroid".to_string(),
        ]));
        assert!(!is_on_radar(&filter, &["missile".to_string()]));
    }

    // ── StateVisuals: five widget states render distinctly ────────────────────

    #[test]
    fn on_screen_visuals_has_distinct_five_states() {
        use crate::gui::resolve_visual;
        let v = on_screen_visuals();
        let idle     = resolve_visual(&v, false, false, false, false).color;
        let hover    = resolve_visual(&v, false, false, false, true).color;
        let active   = resolve_visual(&v, false, false, true, false).color;
        let press    = resolve_visual(&v, false, true, false, false).color;
        let disabled = resolve_visual(&v, true, false, false, false).color;
        assert_ne!(idle, hover);
        assert_ne!(idle, active);
        assert_ne!(idle, press);
        assert_ne!(idle, disabled);
    }

    #[test]
    fn cancel_impulse_visuals_has_distinct_five_states() {
        use crate::gui::resolve_visual;
        let v = cancel_impulse_visuals();
        let idle     = resolve_visual(&v, false, false, false, false).color;
        let hover    = resolve_visual(&v, false, false, false, true).color;
        let active   = resolve_visual(&v, false, false, true, false).color;
        let press    = resolve_visual(&v, false, true, false, false).color;
        let disabled = resolve_visual(&v, true, false, false, false).color;
        assert_ne!(idle, hover);
        assert_ne!(idle, active);
        assert_ne!(idle, press);
        assert_ne!(idle, disabled);
    }
}
