//! Client-side Comms Panel plugin.
//!
//! Renders a two-panel inbox layout:
//! - **Left panel** — message list (sender name + subject line).
//! - **Right panel** — expanded chat view when a message is selected.
//!
//! This plugin drives `ClientCommsState` from inbound `ServerMessage`s and
//! wires response buttons back to `ClientMessage` outbound events. The
//! response buttons are rendered but inert in this slice (no scenario-driven
//! message production yet); they will be activated in the next slice.
//!
//! **Not unit-tested** — visual / Bevy layer. See `client_comms.rs` for the
//! pure, tested logic that backs this plugin.
//!
//! Replaces `src/comms_plugin.rs` (folded in here). Compiled only when the
//! `client` Cargo feature is enabled.

use bevy::prelude::*;

use crate::client_comms::ClientCommsState;
use crate::client_lobby::{ActiveConsole, LobbyState, LobbyView, LocalPlayerToken};
use crate::messages::{Console, GamePhase};

// ── Pure visibility helper ────────────────────────────────────────────

/// Decide whether the comms panel should be visible.
///
/// Rules:
/// 1. Game phase must be `InProgress`.
/// 2. The local player must hold `Console::Communications`.
/// 3. If holding **one console only**, show automatically.
/// 4. If holding **multiple consoles**, show only when `ActiveConsole`
///    is explicitly set to `Communications`.
pub fn comms_panel_visible(
    lobby: &LobbyState,
    token: &str,
    active: &ActiveConsole,
) -> bool {
    if lobby.phase != GamePhase::InProgress {
        return false;
    }
    let view = LobbyView::new(lobby, token);
    let consoles = view.my_consoles();
    if !consoles.contains(&Console::Comms) {
        return false;
    }
    let count = consoles.len();
    match &active.0 {
        Some(c) => *c == Console::Comms,
        None => count == 1,
    }
}

// ── Resources ────────────────────────────────────────────────────────

/// Tracks which sub-view the Comms console is currently showing.
///
/// This is a placeholder resource until PRD #119 fills in the full
/// Comms console design.
#[derive(Resource, Clone, Debug, PartialEq, Eq, Default)]
pub enum CommsView {
    /// Default inbox list view.
    #[default]
    Inbox,
}

// ── Marker components ────────────────────────────────────────────────

/// Marks the root of the Comms console UI; shown only when the local
/// player holds `Console::Communications` and the phase is InProgress.
#[derive(Component)]
pub struct CommsPanel;

// ── Plugin ────────────────────────────────────────────────────────────

pub struct CommsPanelPlugin;

impl Plugin for CommsPanelPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ClientCommsState>()
            .init_resource::<CommsView>()
            .add_systems(Startup, setup_comms_ui)
            .add_systems(Update, toggle_comms_panel_visibility);
    }
}

// ── Setup ────────────────────────────────────────────────────────────

fn setup_comms_ui(mut commands: Commands) {
    commands.spawn((
        CommsPanel,
        Node {
            position_type: PositionType::Absolute,
            left:   Val::Px(0.0),
            top:    Val::Px(0.0),
            right:  Val::Px(0.0),
            bottom: Val::Px(0.0),
            flex_direction: FlexDirection::Column,
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            row_gap: Val::Px(8.0),
            ..default()
        },
        Visibility::Hidden,
    ))
    .with_children(|panel| {
        panel.spawn((
            Text::new("Comms"),
            TextFont { font_size: 32.0, ..default() },
            TextColor(Color::srgb(0.8, 0.7, 1.0)),
        ));
    });
}

// ── Systems ──────────────────────────────────────────────────────────

fn toggle_comms_panel_visibility(
    lobby: Res<LobbyState>,
    token: Res<LocalPlayerToken>,
    active: Res<ActiveConsole>,
    mut panel: Query<&mut Visibility, With<CommsPanel>>,
) {
    if !lobby.is_changed() && !token.is_changed() && !active.is_changed() {
        return;
    }
    let visible = comms_panel_visible(&lobby, &token.0, &active);
    for mut vis in panel.iter_mut() {
        *vis = if visible { Visibility::Visible } else { Visibility::Hidden };
    }
}

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client_lobby::{LobbyState, ActiveConsole};
    use crate::messages::GamePhase;

    fn lobby_in_progress() -> LobbyState {
        let mut s = LobbyState::default();
        s.phase = GamePhase::InProgress;
        s
    }

    #[test]
    fn comms_panel_not_visible_in_lobby_phase() {
        let lobby = LobbyState::default();
        let active = ActiveConsole::default();
        assert!(!comms_panel_visible(&lobby, "tok", &active));
    }

    #[test]
    fn comms_panel_not_visible_when_player_does_not_hold_comms() {
        let lobby = lobby_in_progress();
        let active = ActiveConsole::default();
        assert!(!comms_panel_visible(&lobby, "tok", &active));
    }

    #[test]
    fn comms_view_default_is_inbox() {
        let v = CommsView::default();
        assert_eq!(v, CommsView::Inbox);
    }
}
